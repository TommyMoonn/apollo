use std::{
    io,
    path::Path,
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};

use egui::{CentralPanel, Context, Ui};

use crate::file_dialog::{
    DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogOwner, FileDialogRequest,
    FileDialogStartError, OutputFormat, SaveDialogOptions, WindowOwner,
};

use apollo::{
    audio_mix::{
        execution::{AudioMixExecutionState, AudioMixExecutor, AudioMixStartError},
        model::OutputContainer,
    },
    media::{
        error::{MediaError, MediaErrorKind},
        intake::{MediaIntake, MediaIntakeRequest, MediaLoadStart, MediaProbeDisposition},
        model::MediaFile,
        probe::{MediaProbeError, probe_media},
        tools::{MediaTools, MediaToolsError, discover_media_tools},
    },
    ui::{
        audio_mix as audio_mix_ui,
        audio_mix::{AudioMixPresentation, AudioMixSelectionAction, AudioMixSelectionState},
        audio_mix_progress::{self, AudioMixLifecycleAction, AudioMixLifecycleUiState},
        file_intake::{self, FileIntakeAction, FileIntakeTool},
        fonts, layout, media_error, theme, upscale as upscale_ui,
        upscale::{UpscaleSelectionAction, UpscaleSelectionState},
        upscale_progress::{self, UpscaleLifecycleAction, UpscaleLifecycleUiState},
        widgets,
    },
    upscale::execution::{UpscaleExecutionState, UpscaleExecutor, UpscaleStartError},
};

const EXECUTION_REPAINT_INTERVAL: Duration = Duration::from_millis(50);

fn file_intake_enabled(
    audio_state: &AudioMixExecutionState,
    upscale_state: &UpscaleExecutionState,
) -> bool {
    // Done/Back owns terminal settlement. Intake resumes only after both executors return to Idle.
    matches!(audio_state, AudioMixExecutionState::Idle)
        && matches!(upscale_state, UpscaleExecutionState::Idle)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    AudioMix,
    VideoUpscale,
}

impl Tool {
    fn intake_tool(self) -> FileIntakeTool {
        match self {
            Self::AudioMix => FileIntakeTool::AudioMix,
            Self::VideoUpscale => FileIntakeTool::Upscale,
        }
    }

    fn from_intake(tool: FileIntakeTool) -> Self {
        match tool {
            FileIntakeTool::AudioMix => Self::AudioMix,
            FileIntakeTool::Upscale => Self::VideoUpscale,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioMixToolState {
    session_id: u64,
    media: MediaFile,
    presentation: AudioMixPresentation,
    selection: AudioMixSelectionState,
    lifecycle_ui: AudioMixLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpscaleToolState {
    session_id: u64,
    media: MediaFile,
    selection: UpscaleSelectionState,
    lifecycle_ui: UpscaleLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppState {
    current_tool: Tool,
    intake_active: bool,
    intake_error: Option<MediaError>,
    intake_error_details_open: bool,
    media_intake: MediaIntake,
    audio_mix: Option<AudioMixToolState>,
    upscale: Option<UpscaleToolState>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            current_tool: Tool::AudioMix,
            intake_active: true,
            intake_error: None,
            intake_error_details_open: false,
            media_intake: MediaIntake::default(),
            audio_mix: None,
            upscale: None,
        }
    }
}

struct MediaInspection {
    request: MediaIntakeRequest,
    receiver: Receiver<Result<MediaFile, MediaProbeError>>,
}

impl MediaInspection {
    fn spawn(tools: MediaTools, request: MediaIntakeRequest, context: Context) -> io::Result<Self> {
        let path = request.path().to_path_buf();
        Self::spawn_with(request, context, move || probe_media(&tools, &path))
    }

    fn spawn_with<F>(request: MediaIntakeRequest, context: Context, inspect: F) -> io::Result<Self>
    where
        F: FnOnce() -> Result<MediaFile, MediaProbeError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-media-inspection".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(inspect));
                if let Ok(result) = result {
                    let _ = sender.send(result);
                }
                drop(sender);
                context.request_repaint();
            })?;

        Ok(Self { request, receiver })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MediaToolsState {
    Checking { attempt_id: u64 },
    Ready(MediaTools),
    Unavailable(MediaToolsError),
}

struct MediaToolsDiscovery {
    attempt_id: u64,
    receiver: Receiver<Result<MediaTools, MediaToolsError>>,
}

impl MediaToolsDiscovery {
    fn spawn<F>(attempt_id: u64, context: Context, discover: F) -> io::Result<Self>
    where
        F: FnOnce() -> Result<MediaTools, MediaToolsError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-media-tools".to_owned())
            .spawn(move || {
                let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(discover))
                {
                    Ok(result) => result,
                    Err(_) => Err(MediaToolsError::DiscoveryFailed(
                        "The media tool discovery worker stopped unexpectedly.".to_owned(),
                    )),
                };
                let _ = sender.send(result);
                context.request_repaint();
            })?;

        Ok(Self {
            attempt_id,
            receiver,
        })
    }
}

impl AppState {
    fn select_tool(&mut self, tool: Tool) {
        self.current_tool = tool;
    }

    fn return_to_file_intake(&mut self, tool: Tool) {
        self.current_tool = tool;
        self.intake_active = true;
        self.intake_error = None;
        self.intake_error_details_open = false;
    }

    fn retire_tool_sessions(&mut self) {
        self.audio_mix = None;
        self.upscale = None;
    }
}

pub(crate) struct ApolloApp {
    state: AppState,
    media_tools: MediaToolsState,
    media_tools_discovery: Option<MediaToolsDiscovery>,
    audio_mix_executor: AudioMixExecutor,
    upscale_executor: UpscaleExecutor,
    media_inspection: Option<MediaInspection>,
    file_dialog: FileDialogOwner,
    native_window_owner: Result<WindowOwner, String>,
    egui_context: Context,
    next_media_tools_attempt_id: u64,
    next_session_id: u64,
    tools_error_details_open: bool,
}

impl ApolloApp {
    pub(crate) fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        fonts::install(&creation_context.egui_ctx);
        theme::install(&creation_context.egui_ctx);

        let egui_context = creation_context.egui_ctx.clone();
        let mut app = Self {
            state: AppState::default(),
            media_tools: MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                "Media tool discovery has not started.".to_owned(),
            )),
            media_tools_discovery: None,
            audio_mix_executor: AudioMixExecutor::new(),
            upscale_executor: UpscaleExecutor::new(),
            media_inspection: None,
            file_dialog: FileDialogOwner::default(),
            native_window_owner: WindowOwner::from_window(creation_context),
            egui_context: egui_context.clone(),
            next_media_tools_attempt_id: 0,
            next_session_id: 1,
            tools_error_details_open: false,
        };
        app.start_media_tools_discovery_with(egui_context, discover_media_tools);
        app
    }

    fn ready_media_tools(&self) -> Option<&MediaTools> {
        match &self.media_tools {
            MediaToolsState::Ready(tools) => Some(tools),
            MediaToolsState::Checking { .. } | MediaToolsState::Unavailable(_) => None,
        }
    }

    fn file_intake_enabled(&self) -> bool {
        self.media_inspection.is_none()
            && !self.file_dialog.is_pending()
            && file_intake_enabled(
                self.audio_mix_executor.state(),
                self.upscale_executor.state(),
            )
    }

    fn select_tool(&mut self, tool: Tool) {
        self.state.select_tool(tool);
        self.state.intake_error = None;
        self.state.intake_error_details_open = false;
        if !self.activate_cached_source(tool) {
            self.state.intake_active = true;
        }
    }

    fn activate_cached_source(&mut self, tool: Tool) -> bool {
        let Some(media) = self.state.media_intake.current_if_unchanged().cloned() else {
            self.state.retire_tool_sessions();
            return false;
        };

        match tool {
            Tool::AudioMix => {
                if !self
                    .state
                    .audio_mix
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.audio_mix = Some(AudioMixToolState {
                        session_id,
                        presentation: AudioMixPresentation::new(&media),
                        selection: AudioMixSelectionState::new(&media),
                        lifecycle_ui: AudioMixLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
            Tool::VideoUpscale => {
                if !self
                    .state
                    .upscale
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.upscale = Some(UpscaleToolState {
                        session_id,
                        selection: UpscaleSelectionState::new(&media),
                        lifecycle_ui: UpscaleLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
        }
        self.state.intake_active = false;
        true
    }

    fn take_session_id(&mut self) -> u64 {
        let session_id = self.next_session_id;
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .expect("Apollo tool session identifier space is exhausted");
        session_id
    }

    fn show_media_tools_unavailable(&mut self, ui: &mut Ui) {
        let error = match &self.media_tools {
            MediaToolsState::Unavailable(error) => MediaError::from_tools_error(error),
            MediaToolsState::Checking { .. } | MediaToolsState::Ready(_) => {
                MediaError::invalid_request("media tools are unavailable")
            }
        };

        layout::centered_content(ui, |ui| {
            ui.heading("Media tools unavailable");
            ui.add_space(theme::SPACING.compact);
            media_error::show_inline(ui, &error, &mut self.tools_error_details_open);
            ui.add_space(theme::SPACING.section);
            if widgets::secondary_button(ui, "Retry").clicked() {
                self.retry_media_tools();
            }
        });
    }

    fn show_media_tools_checking(&self, ui: &mut Ui) {
        layout::centered_content(ui, |ui| {
            ui.label("Checking media tools…");
        });
    }

    fn retry_media_tools(&mut self) {
        self.start_media_tools_discovery_with(self.egui_context.clone(), discover_media_tools);
    }

    fn start_media_tools_discovery_with<F>(&mut self, context: Context, discover: F) -> bool
    where
        F: FnOnce() -> Result<MediaTools, MediaToolsError> + Send + 'static,
    {
        if matches!(&self.media_tools, MediaToolsState::Checking { .. })
            || self.media_tools_discovery.is_some()
        {
            return false;
        }

        let attempt_id = self.next_media_tools_attempt_id;
        self.next_media_tools_attempt_id = self.next_media_tools_attempt_id.wrapping_add(1);
        self.tools_error_details_open = false;

        match MediaToolsDiscovery::spawn(attempt_id, context, discover) {
            Ok(discovery) => {
                self.media_tools = MediaToolsState::Checking { attempt_id };
                self.media_tools_discovery = Some(discovery);
                true
            }
            Err(error) => {
                self.media_tools = MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                    format!("Media tool discovery worker could not start: {error}"),
                ));
                false
            }
        }
    }

    fn poll_media_tools_discovery(&mut self) {
        let result = match self.media_tools_discovery.as_ref() {
            Some(discovery) => discovery.receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(result) => {
                let discovery = self
                    .media_tools_discovery
                    .take()
                    .expect("media tool discovery exists while its result is settled");
                self.complete_media_tools_discovery(discovery.attempt_id, result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let discovery = self
                    .media_tools_discovery
                    .take()
                    .expect("media tool discovery exists while disconnect is settled");
                self.complete_media_tools_discovery(
                    discovery.attempt_id,
                    Err(MediaToolsError::DiscoveryFailed(
                        "The media tool discovery worker stopped before returning a result."
                            .to_owned(),
                    )),
                );
            }
        }
    }

    fn complete_media_tools_discovery(
        &mut self,
        attempt_id: u64,
        result: Result<MediaTools, MediaToolsError>,
    ) -> bool {
        if !matches!(
            &self.media_tools,
            MediaToolsState::Checking {
                attempt_id: current
            } if *current == attempt_id
        ) {
            return false;
        }

        self.media_tools = match result {
            Ok(tools) => MediaToolsState::Ready(tools),
            Err(error) => MediaToolsState::Unavailable(error),
        };
        self.tools_error_details_open = false;
        true
    }

    fn show_tool(&mut self, ui: &mut Ui, tool: Tool) {
        if self.state.intake_active {
            self.show_file_intake(ui, tool);
            return;
        }

        match tool {
            Tool::AudioMix if self.state.audio_mix.is_some() => self.show_audio_mix_tool(ui),
            Tool::VideoUpscale if self.state.upscale.is_some() => self.show_upscale_tool(ui),
            _ => self.show_file_intake(ui, tool),
        }
    }

    fn show_file_intake(&mut self, ui: &mut Ui, tool: Tool) {
        let action = ui
            .add_enabled_ui(!self.file_dialog.is_pending(), |ui| {
                file_intake::show(
                    ui,
                    tool.intake_tool(),
                    self.media_inspection.is_some(),
                    self.state.intake_error.as_ref(),
                    &mut self.state.intake_error_details_open,
                )
            })
            .inner;
        match action {
            FileIntakeAction::OpenFile => {
                self.start_file_dialog(FileDialogRequest::OpenMedia, ui.ctx().clone())
            }
            FileIntakeAction::SelectTool(selected) if selected != tool.intake_tool() => {
                self.select_tool(Tool::from_intake(selected));
            }
            FileIntakeAction::None | FileIntakeAction::SelectTool(_) => {}
        }
    }

    fn load_media_path(&mut self, path: &Path) {
        let Some(tools) = self.ready_media_tools() else {
            return;
        };
        let tools = tools.clone();
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        let request_for_failure = request.clone();

        match MediaInspection::spawn(tools, request, self.egui_context.clone()) {
            Ok(inspection) => self.media_inspection = Some(inspection),
            Err(error) => {
                self.state.media_intake.retire_probe(&request_for_failure);
                self.state.intake_error = Some(MediaError::probe_failure(format!(
                    "Media inspection worker could not start: {error}"
                )));
            }
        }
    }

    fn begin_media_path(&mut self, path: &Path) -> Option<MediaIntakeRequest> {
        if !self.file_intake_enabled() {
            return None;
        }

        self.state.intake_active = true;
        self.state.intake_error = None;
        self.state.intake_error_details_open = false;
        match self.state.media_intake.begin_load(path) {
            Ok(MediaLoadStart::Reused) => {
                let _ = self.activate_cached_source(self.state.current_tool);
                None
            }
            Ok(MediaLoadStart::Probe(request)) => {
                self.state.retire_tool_sessions();
                Some(request)
            }
            Err(error) => {
                self.state.retire_tool_sessions();
                self.state.intake_error = Some(MediaError::from_intake_error(&error));
                None
            }
        }
    }

    fn poll_media_inspection(&mut self) {
        let result = match self.media_inspection.as_ref() {
            Some(inspection) => inspection.receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(result) => {
                let inspection = self
                    .media_inspection
                    .take()
                    .expect("inspection exists while its result is settled");
                self.complete_media_probe(inspection.request, result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let inspection = self
                    .media_inspection
                    .take()
                    .expect("inspection exists while its worker disconnect is settled");
                self.state.media_intake.retire_probe(&inspection.request);
                self.state.intake_active = true;
                self.state.retire_tool_sessions();
                self.state.intake_error = Some(MediaError::probe_failure(
                    "Media inspection worker stopped before returning a result.",
                ));
                self.state.intake_error_details_open = false;
            }
        }
    }

    fn complete_media_probe(
        &mut self,
        request: MediaIntakeRequest,
        result: Result<MediaFile, MediaProbeError>,
    ) {
        match self.state.media_intake.complete_probe(request, result) {
            Ok(MediaProbeDisposition::Published) => {
                let _ = self.activate_cached_source(self.state.current_tool);
            }
            Ok(MediaProbeDisposition::Retired) => {}
            Err(error) => {
                self.state.intake_active = true;
                self.state.retire_tool_sessions();
                self.state.intake_error = Some(MediaError::from_intake_error(&error));
                self.state.intake_error_details_open = false;
            }
        }
    }

    #[cfg(test)]
    fn load_media_path_with_probe<F>(&mut self, path: &Path, probe: F)
    where
        F: FnOnce(&Path) -> Result<MediaFile, MediaProbeError>,
    {
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        let result = probe(request.path());
        self.complete_media_probe(request, result);
    }

    fn handle_window_file_drop(&mut self, context: &Context) {
        if !self.file_intake_enabled() {
            return;
        }
        let dropped = context.input(|input| match input.raw.dropped_files.as_slice() {
            [] => None,
            [file] => Some(Ok(file.path().to_path_buf())),
            _ => Some(Err(MediaError::multiple_files())),
        });
        match dropped {
            Some(Ok(path)) => self.load_media_path(&path),
            Some(Err(error)) => {
                self.state.intake_active = true;
                self.state.intake_error = Some(error);
                self.state.intake_error_details_open = false;
            }
            None => {}
        }
    }

    fn drag_hovered(&self, context: &Context) -> bool {
        self.file_intake_enabled() && context.input(|input| input.raw.hovered_files.len() == 1)
    }

    fn show_audio_mix_tool(&mut self, ui: &mut Ui) {
        if shows_audio_mix_lifecycle(self.audio_mix_executor.state()) {
            let action = {
                let session = self
                    .state
                    .audio_mix
                    .as_mut()
                    .expect("audio mix source exists while lifecycle surface is shown");
                let request = session.selection.request(&session.media);
                audio_mix_progress::show(
                    ui,
                    &session.media,
                    &request,
                    &mut self.audio_mix_executor,
                    &mut session.lifecycle_ui,
                )
            };
            if self.audio_mix_executor.is_running() {
                ui.ctx().request_repaint_after(EXECUTION_REPAINT_INTERVAL);
            }
            self.handle_audio_mix_lifecycle_action(action);
            return;
        }

        if self.show_audio_mix_start_error(ui) {
            return;
        }

        let action = ui
            .add_enabled_ui(!self.file_dialog.is_pending(), |ui| {
                let session = self
                    .state
                    .audio_mix
                    .as_mut()
                    .expect("audio mix source exists while ready surface is shown");
                audio_mix_ui::show(
                    ui,
                    &session.media,
                    &session.presentation,
                    &mut session.selection,
                )
            })
            .inner;
        if let Err(error) = self.handle_audio_mix_action(action, ui.ctx().clone())
            && let Some(session) = self.state.audio_mix.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_audio_mix_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.audio_mix.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Audio mix could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    fn handle_audio_mix_action(
        &mut self,
        action: AudioMixSelectionAction,
        context: Context,
    ) -> Result<(), AudioMixStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        match action {
            AudioMixSelectionAction::Back if self.file_intake_enabled() => {
                self.state.return_to_file_intake(Tool::AudioMix);
            }
            AudioMixSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let request = audio_mix_output_dialog_request(session);
                self.start_file_dialog(request, context);
            }
            AudioMixSelectionAction::MixRequested => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = session.selection.request(&session.media);
                self.audio_mix_executor
                    .start(&tools, &session.media, &request)?;
            }
            AudioMixSelectionAction::None | AudioMixSelectionAction::Back => {}
        }
        Ok(())
    }

    fn handle_audio_mix_lifecycle_action(&self, action: AudioMixLifecycleAction) {
        if let AudioMixLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Mix output folder could not be opened: {error}");
        }
    }

    fn show_upscale_tool(&mut self, ui: &mut Ui) {
        if shows_upscale_lifecycle(self.upscale_executor.state()) {
            let action = {
                let upscale = self
                    .state
                    .upscale
                    .as_mut()
                    .expect("upscale source exists while lifecycle surface is shown");
                let request = upscale.selection.request(&upscale.media);
                upscale_progress::show(
                    ui,
                    &upscale.media,
                    &request,
                    &mut self.upscale_executor,
                    &mut upscale.lifecycle_ui,
                )
            };
            if self.upscale_executor.is_running() {
                ui.ctx().request_repaint_after(EXECUTION_REPAINT_INTERVAL);
            }
            self.handle_upscale_lifecycle_action(action);
            return;
        }

        if self.show_upscale_start_error(ui) {
            return;
        }

        let action = ui
            .add_enabled_ui(!self.file_dialog.is_pending(), |ui| {
                let upscale = self
                    .state
                    .upscale
                    .as_mut()
                    .expect("upscale source exists while ready surface is shown");
                upscale_ui::show(ui, &upscale.media, &mut upscale.selection)
            })
            .inner;
        if let Err(error) = self.handle_upscale_action(action, ui.ctx().clone())
            && let Some(session) = self.state.upscale.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_upscale_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.upscale.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Upscale could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    fn handle_upscale_action(
        &mut self,
        action: UpscaleSelectionAction,
        context: Context,
    ) -> Result<(), UpscaleStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        match action {
            UpscaleSelectionAction::Back if self.file_intake_enabled() => {
                self.state.return_to_file_intake(Tool::VideoUpscale);
            }
            UpscaleSelectionAction::BrowseOutput => {
                let Some(session) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let request = upscale_output_dialog_request(session);
                self.start_file_dialog(request, context);
            }
            UpscaleSelectionAction::UpscaleRequested => {
                let Some(upscale) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = upscale.selection.request(&upscale.media);
                self.upscale_executor
                    .start(&tools, &upscale.media, &request)?;
            }
            UpscaleSelectionAction::None | UpscaleSelectionAction::Back => {}
        }
        Ok(())
    }

    fn handle_upscale_lifecycle_action(&self, action: UpscaleLifecycleAction) {
        if let UpscaleLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Upscale output folder could not be opened: {error}");
        }
    }

    fn start_file_dialog(&mut self, request: FileDialogRequest, context: Context) {
        let target = request.target();
        if let Err(error) =
            self.file_dialog
                .request(request, context, self.native_window_owner.clone())
            && !matches!(error, FileDialogStartError::AlreadyPending)
        {
            self.settle_file_dialog(FileDialogCompletion {
                target,
                outcome: DialogOutcome::Failed(error.to_string()),
            });
        }
    }

    fn poll_file_dialog(&mut self) {
        if let Some(completion) = self.file_dialog.poll() {
            self.settle_file_dialog(completion);
        }
    }

    fn settle_file_dialog(&mut self, completion: FileDialogCompletion) {
        match (completion.target, completion.outcome) {
            (_, DialogOutcome::Cancelled) => {}
            (DialogTarget::MediaInput, DialogOutcome::Selected(path)) => {
                self.load_media_path(&path);
            }
            (DialogTarget::MediaInput, DialogOutcome::Failed(diagnostic)) => {
                self.state.intake_error = Some(MediaError::file_picker_unavailable(diagnostic));
                self.state.intake_error_details_open = false;
            }
            (target, outcome) => settle_output_file_dialog(
                &mut self.state,
                self.audio_mix_executor.state(),
                self.upscale_executor.state(),
                target,
                outcome,
            ),
        }
    }
}

fn settle_output_file_dialog(
    state: &mut AppState,
    audio_execution: &AudioMixExecutionState,
    upscale_execution: &UpscaleExecutionState,
    target: DialogTarget,
    outcome: DialogOutcome,
) {
    match (target, outcome) {
        (DialogTarget::AudioMixOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(audio_execution, AudioMixExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_mix
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::AudioMixOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(audio_execution, AudioMixExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_mix
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::UpscaleOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(upscale_execution, UpscaleExecutionState::Idle) =>
        {
            if let Some(session) = state
                .upscale
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::UpscaleOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(upscale_execution, UpscaleExecutionState::Idle) =>
        {
            if let Some(session) = state
                .upscale
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        _ => {}
    }
}

fn save_dialog_options(output: &Path, format: OutputFormat) -> SaveDialogOptions {
    let initial_directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let file_name = output.file_name().unwrap_or_default().to_os_string();
    SaveDialogOptions {
        initial_directory,
        file_name,
        format,
    }
}

fn output_dialog_request(
    target: DialogTarget,
    output: &Path,
    format: OutputFormat,
) -> FileDialogRequest {
    FileDialogRequest::SaveOutput {
        target,
        options: save_dialog_options(output, format),
    }
}

fn audio_mix_output_dialog_request(session: &AudioMixToolState) -> FileDialogRequest {
    let format = match session.selection.container() {
        OutputContainer::Mp4 => OutputFormat::Mp4,
        OutputContainer::Mkv => OutputFormat::Mkv,
    };
    output_dialog_request(
        DialogTarget::AudioMixOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn upscale_output_dialog_request(session: &UpscaleToolState) -> FileDialogRequest {
    output_dialog_request(
        DialogTarget::UpscaleOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        OutputFormat::Mp4,
    )
}

fn shows_audio_mix_lifecycle(state: &AudioMixExecutionState) -> bool {
    !matches!(state, AudioMixExecutionState::Idle)
}

fn shows_upscale_lifecycle(state: &UpscaleExecutionState) -> bool {
    !matches!(state, UpscaleExecutionState::Idle)
}

fn open_folder(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    #[cfg(any(target_os = "windows", target_os = "macos", unix))]
    {
        command.arg(path).spawn().map(|_| ())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "opening folders is not supported on this platform",
        ))
    }
}

impl eframe::App for ApolloApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        CentralPanel::default()
            .frame(layout::content_frame(ui.style()))
            .show(ui, |ui| {
                self.poll_media_tools_discovery();
                if matches!(&self.media_tools, MediaToolsState::Checking { .. }) {
                    self.show_media_tools_checking(ui);
                    return;
                }
                if matches!(&self.media_tools, MediaToolsState::Unavailable(_)) {
                    self.show_media_tools_unavailable(ui);
                    return;
                }

                self.poll_file_dialog();
                self.poll_media_inspection();

                self.handle_window_file_drop(ui.ctx());
                if self.drag_hovered(ui.ctx()) {
                    file_intake::show_drag_hover(ui);
                }

                self.show_tool(ui, self.state.current_tool);
            });
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    use super::{
        ApolloApp, AppState, MediaInspection, MediaToolsState, Tool,
        audio_mix_output_dialog_request, file_intake_enabled, settle_output_file_dialog,
        shows_audio_mix_lifecycle, shows_upscale_lifecycle,
    };
    use crate::file_dialog::{
        DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogOwner, FileDialogRequest,
        FileDialogStartError, OutputFormat, WindowOwner,
    };
    use apollo::{
        audio_mix::{
            execution::AudioMixExecutionState,
            model::{LoudnessNormalization, LoudnessTargets, OutputContainer, TrackGain},
        },
        media::{
            error::{MediaError, MediaErrorKind},
            execution::MediaExecutionFailure,
            intake::MediaLoadStart,
            model::{AudioStream, FrameRate, MediaFile, VideoStream},
            probe::MediaProbeError,
            process::MediaProcessFailure,
            tools::{MediaTools, MediaToolsError},
        },
        ui::{audio_mix::AudioMixSelectionAction, fonts, theme, upscale::UpscaleSelectionAction},
        upscale::{
            execution::UpscaleExecutionState,
            model::{UpscaleTarget, VideoCodec},
        },
    };

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct TestDroppedFile {
        path: PathBuf,
    }

    impl egui::DroppedFile for TestDroppedFile {
        fn path(&self) -> &Path {
            &self.path
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            fs::read(&self.path).map_err(|error| error.to_string())
        }
    }

    fn temp_file(label: &str) -> (PathBuf, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "apollo-phase35-app-{label}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("recording.mkv");
        fs::write(&path, b"media").unwrap();
        (directory, path)
    }

    fn media(path: &Path) -> MediaFile {
        MediaFile {
            path: path.to_path_buf(),
            duration: None,
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: FrameRate::new(60, 1),
            }],
            audio_streams: vec![AudioStream {
                stream_index: 1,
                codec: Some("aac".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: Some("Desktop Audio".to_owned()),
                language: None,
            }],
        }
    }

    fn app() -> ApolloApp {
        let egui_context = egui::Context::default();
        ApolloApp {
            state: AppState::default(),
            media_tools: MediaToolsState::Ready(MediaTools {
                ffmpeg: PathBuf::from("missing-ffmpeg"),
                ffprobe: PathBuf::from("missing-ffprobe"),
            }),
            media_tools_discovery: None,
            audio_mix_executor: apollo::audio_mix::execution::AudioMixExecutor::new(),
            upscale_executor: apollo::upscale::execution::UpscaleExecutor::new(),
            media_inspection: None,
            file_dialog: FileDialogOwner::default(),
            native_window_owner: Ok(WindowOwner::test_value()),
            egui_context,
            next_media_tools_attempt_id: 0,
            next_session_id: 1,
            tools_error_details_open: false,
        }
    }

    #[test]
    fn media_tool_discovery_starts_in_checking_without_blocking() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        let (release_sender, release_receiver) = mpsc::channel();

        assert!(
            app.start_media_tools_discovery_with(egui::Context::default(), move || {
                release_receiver.recv().unwrap();
                Err(MediaToolsError::MissingBoth)
            },)
        );

        assert!(matches!(app.media_tools, MediaToolsState::Checking { .. }));
        assert!(app.media_tools_discovery.is_some());
        release_sender.send(()).unwrap();
    }

    #[test]
    fn media_tool_discovery_completion_requests_repaint_and_transitions_to_ready() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        app.tools_error_details_open = true;
        let expected = MediaTools {
            ffmpeg: PathBuf::from(r"C:\Media Tools\ffmpeg.exe"),
            ffprobe: PathBuf::from(r"C:\Media Tools\ffprobe.exe"),
        };
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
        });

        assert!(app.start_media_tools_discovery_with(context, {
            let expected = expected.clone();
            move || Ok(expected)
        }));
        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("discovery completion should request repaint");
        app.poll_media_tools_discovery();

        assert_eq!(app.media_tools, MediaToolsState::Ready(expected));
        assert!(app.media_tools_discovery.is_none());
        assert!(!app.tools_error_details_open);
    }

    #[test]
    fn media_tool_discovery_failure_transitions_to_unavailable() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
        });

        assert!(app.start_media_tools_discovery_with(context, || {
            Err(MediaToolsError::MissingFfprobe)
        }));
        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("failed discovery completion should request repaint");
        app.poll_media_tools_discovery();

        assert_eq!(
            app.media_tools,
            MediaToolsState::Unavailable(MediaToolsError::MissingFfprobe)
        );
    }

    #[test]
    fn media_tool_retry_is_single_flight() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        let (release_sender, release_receiver) = mpsc::channel();

        assert!(
            app.start_media_tools_discovery_with(egui::Context::default(), move || {
                release_receiver.recv().unwrap();
                Err(MediaToolsError::MissingBoth)
            },)
        );
        assert!(
            !app.start_media_tools_discovery_with(egui::Context::default(), || {
                panic!("overlapping retry discovery must not start")
            })
        );

        release_sender.send(()).unwrap();
    }

    #[test]
    fn stale_media_tool_discovery_completion_is_rejected() {
        let mut app = app();
        app.media_tools = MediaToolsState::Checking { attempt_id: 7 };
        let stale = MediaTools {
            ffmpeg: PathBuf::from("stale-ffmpeg"),
            ffprobe: PathBuf::from("stale-ffprobe"),
        };

        assert!(!app.complete_media_tools_discovery(6, Ok(stale)));
        assert_eq!(app.media_tools, MediaToolsState::Checking { attempt_id: 7 });
    }

    fn dropped_file(path: &Path) -> egui::DroppedFileHandle {
        Arc::new(TestDroppedFile {
            path: path.to_path_buf(),
        })
    }

    fn handle_drop(app: &mut ApolloApp, paths: &[&Path]) {
        let context = egui::Context::default();
        let input = egui::RawInput {
            dropped_files: paths.iter().map(|path| dropped_file(path)).collect(),
            ..Default::default()
        };
        let output = context.run_ui(input, |ui| app.handle_window_file_drop(ui.ctx()));
        output.drop_without_applying_deltas();
    }

    fn drag_hovered(app: &ApolloApp, paths: &[&Path]) -> bool {
        let context = egui::Context::default();
        let input = egui::RawInput {
            hovered_files: paths
                .iter()
                .map(|path| egui::HoveredFile {
                    path: Some((*path).to_path_buf()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut hovered = false;
        let output = context.run_ui(input, |ui| hovered = app.drag_hovered(ui.ctx()));
        output.drop_without_applying_deltas();
        hovered
    }

    fn begin_pending_inspection(
        app: &mut ApolloApp,
        path: &Path,
    ) -> mpsc::SyncSender<Result<MediaFile, MediaProbeError>> {
        let request = app
            .begin_media_path(path)
            .expect("new test source should require inspection");
        let (sender, receiver) = mpsc::sync_channel(1);
        app.media_inspection = Some(MediaInspection { request, receiver });
        sender
    }

    fn shape_contains_text(shape: &egui::Shape, needle: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text.contains(needle),
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .any(|shape| shape_contains_text(shape, needle)),
            _ => false,
        }
    }

    fn output_contains_text(output: &egui::FullOutput, needle: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|clipped| shape_contains_text(&clipped.shape, needle))
    }

    fn terminal_execution_states() -> [AudioMixExecutionState; 3] {
        [
            AudioMixExecutionState::Completed {
                output: PathBuf::from("finished.mp4"),
            },
            AudioMixExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: "encoding failed".to_owned(),
            }),
            AudioMixExecutionState::Cancelled,
        ]
    }

    #[test]
    fn fresh_application_state_starts_on_audio_mix_file_intake() {
        let state = AppState::default();

        assert_eq!(state.current_tool, Tool::AudioMix);
        assert!(state.intake_active);
        assert!(state.audio_mix.is_none());
        assert!(state.upscale.is_none());
    }

    #[test]
    fn initialization_selects_audio_mix_without_starting_an_executor() {
        let app = app();

        assert_eq!(app.state.current_tool, Tool::AudioMix);
        assert!(app.state.intake_active);
        assert!(!app.audio_mix_executor.is_running());
        assert!(!app.upscale_executor.is_running());
    }

    #[test]
    fn fresh_application_state_renders_unified_file_intake() {
        let mut app = app();
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let tool = app.state.current_tool;
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, tool);
        });

        assert!(output_contains_text(&output, "Open files"));
        assert!(output_contains_text(&output, "Audio Mix"));
        assert!(output_contains_text(&output, "Upscale"));
        output.textures_delta.clear();
    }

    #[test]
    fn inspecting_state_is_sparse_and_blocks_source_replacement() {
        let (directory, path) = temp_file("inspecting-state");
        let mut app = app();
        let _sender = begin_pending_inspection(&mut app, &path);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        assert!(app.state.intake_active);
        assert!(app.media_inspection.is_some());
        assert!(!app.file_intake_enabled());

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioMix);
        });

        assert!(output_contains_text(&output, "Inspecting media…"));
        assert!(output_contains_text(&output, "Audio Mix"));
        assert!(output_contains_text(&output, "Upscale"));
        assert!(!output_contains_text(&output, "Open files"));
        assert!(!output_contains_text(&output, "or drop a video anywhere"));
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_completion_activates_currently_selected_mode() {
        let (directory, path) = temp_file("inspection-mode-switch");
        let mut app = app();
        let sender = begin_pending_inspection(&mut app, &path);

        app.select_tool(Tool::VideoUpscale);
        sender.send(Ok(media(&path))).unwrap();
        app.poll_media_inspection();

        assert!(app.media_inspection.is_none());
        assert!(!app.state.intake_active);
        assert!(app.state.audio_mix.is_none());
        assert_eq!(
            app.state
                .upscale
                .as_ref()
                .map(|session| &session.media.path),
            Some(&path)
        );
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_completion_requests_repaint_without_fixed_polling() {
        let (directory, path) = temp_file("inspection-repaint");
        let mut app = app();
        let request = app
            .begin_media_path(&path)
            .expect("new source should require inspection");
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
        });
        let media = media(&path);
        app.media_inspection = Some(
            MediaInspection::spawn_with(request, context, move || Ok(media))
                .expect("inspection worker should start"),
        );

        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("inspection completion should request repaint");
        app.poll_media_inspection();

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_worker_panic_requests_repaint_and_surfaces_probe_failure() {
        let (directory, path) = temp_file("inspection-worker-panic");
        let mut app = app();
        let request = app
            .begin_media_path(&path)
            .expect("new source should require inspection");
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        let repaint_barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&repaint_barrier);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
            worker_barrier.wait();
        });
        app.media_inspection = Some(
            MediaInspection::spawn_with(request, context, || {
                panic!("simulated inspection worker failure")
            })
            .expect("inspection worker should start"),
        );

        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("failed inspection worker should request repaint");
        app.poll_media_inspection();
        repaint_barrier.wait();

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_inspection_completion_cannot_replace_newer_source() {
        let (directory, first) = temp_file("stale-inspection");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        let first_request = app
            .begin_media_path(&first)
            .expect("first source should require inspection");
        let second_request = match app.state.media_intake.begin_load(&second).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("replacement should require inspection"),
        };

        app.complete_media_probe(first_request, Ok(media(&first)));
        assert!(app.state.media_intake.current().is_none());
        app.complete_media_probe(second_request, Ok(media(&second)));

        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&second)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_inspection_returns_to_intake_without_current_metadata() {
        let (directory, path) = temp_file("inspection-failure");
        let mut app = app();
        let sender = begin_pending_inspection(&mut app, &path);

        sender
            .send(Err(MediaProbeError::FfprobeFailed {
                code: Some(1),
                diagnostic: "probe failed".to_owned(),
            }))
            .unwrap();
        app.poll_media_inspection();

        assert!(app.media_inspection.is_none());
        assert!(app.state.intake_active);
        assert!(app.state.media_intake.current().is_none());
        assert!(app.state.audio_mix.is_none());
        assert!(app.state.upscale.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switching_mode_on_intake_routes_next_source_to_selected_tool() {
        let (directory, path) = temp_file("mode-switch");
        let mut app = app();

        app.select_tool(Tool::VideoUpscale);
        assert_eq!(app.state.current_tool, Tool::VideoUpscale);
        assert!(app.state.intake_active);

        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        assert!(!app.state.intake_active);
        assert!(app.state.audio_mix.is_none());
        assert_eq!(
            app.state
                .upscale
                .as_ref()
                .map(|session| &session.media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_valid_file_routes_audio_mix_to_ready_state() {
        let (directory, path) = temp_file("open");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        assert!(!app.state.intake_active);
        assert_eq!(
            app.state.audio_mix.as_ref().map(|s| &s.media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dropped_valid_file_routes_upscale_to_ready_state() {
        let (directory, path) = temp_file("drop");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);

        assert!(file_intake_enabled(
            app.audio_mix_executor.state(),
            app.upscale_executor.state(),
        ));
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        assert!(!app.state.intake_active);
        assert_eq!(
            app.state.upscale.as_ref().map(|s| &s.media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn one_file_drop_starts_normal_media_inspection() {
        let (directory, path) = temp_file("single-drop");
        let mut app = app();

        handle_drop(&mut app, &[&path]);

        assert_eq!(
            app.media_inspection
                .as_ref()
                .map(|inspection| inspection.request.path()),
            Some(path.as_path())
        );
        assert!(app.state.intake_error.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn multiple_file_drop_is_rejected_before_any_probe_starts() {
        let (first_directory, first) = temp_file("multi-drop-first");
        let (second_directory, second) = temp_file("multi-drop-second");
        let mut app = app();

        handle_drop(&mut app, &[&first, &second]);

        assert!(app.media_inspection.is_none());
        assert!(app.state.media_intake.current().is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::MultipleFiles)
        );
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::message),
            Some("Drop one file at a time.")
        );
        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn directory_drop_uses_shared_intake_error_path() {
        let directory = std::env::temp_dir().join(format!(
            "apollo-phase49-directory-drop-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let mut app = app();

        handle_drop(&mut app, &[&directory]);

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::SourceUnavailable)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_path_drop_uses_shared_intake_error_path() {
        let directory = std::env::temp_dir().join(format!(
            "apollo-phase49-missing-drop-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        let missing = directory.join("missing.mkv");
        let mut app = app();

        handle_drop(&mut app, &[&missing]);

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::SourceUnavailable)
        );
    }

    #[test]
    fn opening_upscale_source_renders_ready_surface() {
        let (directory, path) = temp_file("upscale-ready-render");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::VideoUpscale);
        });

        assert!(output_contains_text(&output, "Upscale to"));
        assert!(output_contains_text(&output, "Video codec"));
        assert!(output_contains_text(&output, "Actual output"));
        assert!(output_contains_text(&output, "← Back"));
        assert!(!output_contains_text(&output, "Change"));
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_audio_mix_source_renders_ready_surface() {
        let (directory, path) = temp_file("audio-ready-render");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioMix);
        });

        assert!(output_contains_text(&output, "Audio tracks"));
        assert!(output_contains_text(&output, "Normalize output loudness"));
        assert!(!output_contains_text(&output, "Integrated loudness"));
        assert!(output_contains_text(&output, "Mix selected tracks"));
        assert!(output_contains_text(&output, "← Back"));
        assert!(!output_contains_text(&output, "Change"));
        output.textures_delta.clear();

        app.state
            .audio_mix
            .as_mut()
            .unwrap()
            .selection
            .set_normalization_enabled(true);
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioMix);
        });
        for label in [
            "Integrated loudness",
            "Maximum true peak",
            "Loudness range",
            "Reset normalization targets",
        ] {
            assert!(output_contains_text(&output, label), "missing {label}");
        }
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_ready_selection_survives_same_source_navigation() {
        let (directory, path) = temp_file("selection-preserve");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let ready = app
            .state
            .upscale
            .as_mut()
            .expect("upscale session should exist");
        ready
            .selection
            .set_target(&ready.media, UpscaleTarget::Uhd8k);
        ready.selection.set_codec(VideoCodec::H265);

        app.select_tool(Tool::VideoUpscale);

        let ready = app
            .state
            .upscale
            .as_ref()
            .expect("upscale session should persist");
        assert_eq!(ready.selection.target(), UpscaleTarget::Uhd8k);
        assert_eq!(ready.selection.codec(), VideoCodec::H265);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_mix_output_dialog_request_follows_current_container() {
        let (directory, path) = temp_file("audio-browse");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        let session = app.state.audio_mix.as_ref().unwrap();
        let request = audio_mix_output_dialog_request(session);
        let FileDialogRequest::SaveOutput { target, options } = request else {
            panic!("audio browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::AudioMixOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, directory);
        assert_eq!(options.file_name, "recording-mixed.mp4");
        assert_eq!(options.format, OutputFormat::Mp4);

        let session = app.state.audio_mix.as_mut().unwrap();
        let media = session.media.clone();
        session
            .selection
            .set_container(&media, OutputContainer::Mkv);
        let request = audio_mix_output_dialog_request(session);
        let FileDialogRequest::SaveOutput { options, .. } = request else {
            unreachable!();
        };
        assert_eq!(options.file_name, "recording-mixed.mkv");
        assert_eq!(options.format, OutputFormat::Mkv);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn selected_output_paths_route_to_the_owning_tool() {
        let (directory, path) = temp_file("upscale-browse");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let upscale_session_id = app.state.upscale.as_ref().unwrap().session_id;
        let destination = directory.join("chosen upscale.mp4");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::UpscaleOutput {
                session_id: upscale_session_id,
            },
            outcome: DialogOutcome::Selected(destination.clone()),
        });

        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            destination
        );

        app.select_tool(Tool::AudioMix);
        let audio_session_id = app.state.audio_mix.as_ref().unwrap().session_id;
        let audio_destination = directory.join("chosen mix.mp4");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput {
                session_id: audio_session_id,
            },
            outcome: DialogOutcome::Selected(audio_destination.clone()),
        });
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            audio_destination
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cancelled_output_browse_preserves_ready_state() {
        let (directory, path) = temp_file("browse-cancel");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let before = app.state.clone();
        let session_id = app.state.audio_mix.as_ref().unwrap().session_id;

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput { session_id },
            outcome: DialogOutcome::Cancelled,
        });

        assert_eq!(app.state, before);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pending_dialog_blocks_conflicting_app_actions_and_a_second_request() {
        let (directory, path) = temp_file("pending-dialog-boundary");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let (release_sender, release_receiver) = mpsc::channel();
        app.file_dialog
            .request_for_test(
                FileDialogRequest::OpenMedia,
                egui::Context::default(),
                move |_| {
                    release_receiver.recv().unwrap();
                    Ok(None)
                },
            )
            .unwrap();

        assert!(!app.file_intake_enabled());
        app.handle_audio_mix_action(AudioMixSelectionAction::Back, egui::Context::default())
            .unwrap();
        assert!(!app.state.intake_active);
        assert_eq!(
            app.file_dialog.request(
                FileDialogRequest::OpenMedia,
                egui::Context::default(),
                Ok(WindowOwner::test_value()),
            ),
            Err(FileDialogStartError::AlreadyPending)
        );

        release_sender.send(()).unwrap();
        for _ in 0..100 {
            if app.file_dialog.poll().is_some() {
                fs::remove_dir_all(directory).unwrap();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("test dialog worker did not complete");
    }

    #[test]
    fn stale_output_completions_cannot_mutate_replacement_sessions() {
        let (directory, first) = temp_file("stale-dialog-session");
        let second = directory.join("replacement.mkv");
        let third = directory.join("third.mkv");
        fs::write(&second, b"replacement").unwrap();
        fs::write(&third, b"third").unwrap();
        let mut app = app();

        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        let old_audio_id = app.state.audio_mix.as_ref().unwrap().session_id;
        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_audio = app.state.audio_mix.as_ref().unwrap();
        let replacement_audio_id = replacement_audio.session_id;
        let replacement_audio_output = replacement_audio.selection.output_path();
        assert_ne!(old_audio_id, replacement_audio_id);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput {
                session_id: old_audio_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale-mix.mp4")),
        });
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            replacement_audio_output
        );

        app.select_tool(Tool::VideoUpscale);
        let old_upscale_id = app.state.upscale.as_ref().unwrap().session_id;
        app.load_media_path_with_probe(&third, |path| Ok(media(path)));
        let replacement_upscale = app.state.upscale.as_ref().unwrap();
        let replacement_upscale_id = replacement_upscale.session_id;
        let replacement_upscale_output = replacement_upscale.selection.output_path();
        assert_ne!(old_upscale_id, replacement_upscale_id);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::UpscaleOutput {
                session_id: old_upscale_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale-upscale.mp4")),
        });
        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            replacement_upscale_output
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn output_completion_cannot_mutate_selection_after_execution_starts() {
        let (directory, path) = temp_file("dialog-after-execution");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let audio_id = app.state.audio_mix.as_ref().unwrap().session_id;
        let audio_output = app
            .state
            .audio_mix
            .as_ref()
            .unwrap()
            .selection
            .output_path();

        settle_output_file_dialog(
            &mut app.state,
            &AudioMixExecutionState::Running,
            &UpscaleExecutionState::Idle,
            DialogTarget::AudioMixOutput {
                session_id: audio_id,
            },
            DialogOutcome::Selected(directory.join("late-mix.mp4")),
        );
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            audio_output
        );

        app.select_tool(Tool::VideoUpscale);
        let upscale_id = app.state.upscale.as_ref().unwrap().session_id;
        let upscale_output = app.state.upscale.as_ref().unwrap().selection.output_path();
        settle_output_file_dialog(
            &mut app.state,
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Running,
            DialogTarget::UpscaleOutput {
                session_id: upscale_id,
            },
            DialogOutcome::Selected(directory.join("late-upscale.mp4")),
        );
        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            upscale_output
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_dialog_selected_path_starts_the_shared_intake_route() {
        let (directory, path) = temp_file("dialog-open-route");
        let mut app = app();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Selected(path.clone()),
        });

        let inspection = app
            .media_inspection
            .as_ref()
            .expect("selected Open path should start media inspection");
        assert_eq!(inspection.request.path(), path);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_dialog_failure_surfaces_bounded_intake_feedback() {
        let mut app = app();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Failed("native open diagnostic marker".to_owned()),
        });

        let error = app
            .state
            .intake_error
            .as_ref()
            .expect("open dialog failure should become intake state");
        assert_eq!(error.kind(), MediaErrorKind::FilePickerUnavailable);
        assert_eq!(error.diagnostic(), Some("native open diagnostic marker"));
        assert!(!app.state.intake_error_details_open);
    }

    #[test]
    fn output_picker_failure_surfaces_bounded_user_feedback() {
        let (directory, path) = temp_file("browse-failure");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let output_before = app
            .state
            .upscale
            .as_ref()
            .unwrap()
            .selection
            .output_path()
            .to_path_buf();
        let session_id = app.state.upscale.as_ref().unwrap().session_id;

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::UpscaleOutput { session_id },
            outcome: DialogOutcome::Failed("native picker diagnostic marker".to_owned()),
        });

        let error = app
            .state
            .upscale
            .as_ref()
            .unwrap()
            .start_error
            .as_ref()
            .expect("picker failure should become user-facing state");
        assert_eq!(error.kind(), MediaErrorKind::OutputPickerUnavailable);
        assert_eq!(error.diagnostic(), Some("native picker diagnostic marker"));
        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            output_before
        );

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut rendered = context.run_ui(Default::default(), |ui| {
            assert!(app.show_upscale_start_error(ui));
        });
        assert!(output_contains_text(&rendered, "Output picker unavailable"));
        assert!(output_contains_text(
            &rendered,
            "The output picker could not be opened."
        ));
        assert!(!output_contains_text(
            &rendered,
            "native picker diagnostic marker"
        ));
        rendered.textures_delta.clear();

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_intent_reaches_executor() {
        let (directory, path) = temp_file("execute");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        let error = app
            .handle_upscale_action(
                UpscaleSelectionAction::UpscaleRequested,
                egui::Context::default(),
            )
            .expect_err("missing test FFmpeg should prove the execution path was attempted");

        assert!(matches!(
            error,
            apollo::upscale::execution::UpscaleStartError::Process(
                apollo::media::process::MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound
                )
            )
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        use apollo::media::{execution::MediaExecutionFailure, process::MediaProcessFailure};

        assert!(!shows_upscale_lifecycle(&UpscaleExecutionState::Idle));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Running));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Completed {
            output: PathBuf::from("recording-4k.mp4"),
        }));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Failed(
            MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            }
        )));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Cancelled));
    }

    #[test]
    fn probe_failure_leaves_no_previous_ready_metadata() {
        let (directory, first) = temp_file("stale-first");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.load_media_path_with_probe(&second, |_| {
            Err(apollo::media::probe::MediaProbeError::FfprobeFailed {
                code: Some(1),
                diagnostic: "probe failed".to_owned(),
            })
        });

        assert!(app.state.intake_active);
        assert!(app.state.upscale.is_none());
        assert!(app.state.media_intake.current().is_none());
        assert!(app.state.intake_error.is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switching_tools_reuses_successful_probe_without_probing_again() {
        let (directory, path) = temp_file("reuse");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.state.return_to_file_intake(Tool::AudioMix);

        app.select_tool(Tool::VideoUpscale);

        assert!(!app.state.intake_active);
        assert_eq!(
            app.state.upscale.as_ref().map(|s| &s.media.path),
            Some(&path)
        );
        assert_eq!(
            app.state.media_intake.current().map(|m| &m.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_mix_back_returns_to_intake_and_same_source_restores_ready_state() {
        let (directory, path) = temp_file("audio-back");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let session = app
            .state
            .audio_mix
            .as_mut()
            .expect("audio mix session should exist");
        session.selection.toggle_stream(&session.media, 1);
        assert!(session.selection.set_track_gain(
            &session.media,
            1,
            TrackGain::from_tenths_db(-55),
        ));
        session
            .selection
            .set_container(&session.media, OutputContainer::Mkv);
        let custom_output = directory.join("custom-mix.mkv");
        session
            .selection
            .set_custom_output_path(custom_output.clone());
        let normalization_targets = LoudnessTargets::from_tenths(-205, -8, 145);
        assert!(
            session
                .selection
                .set_normalization_targets(normalization_targets)
        );
        session.selection.set_normalization_enabled(true);

        app.handle_audio_mix_action(AudioMixSelectionAction::Back, egui::Context::default())
            .unwrap();

        assert!(app.state.intake_active);
        assert_eq!(app.state.current_tool, Tool::AudioMix);
        assert!(app.state.audio_mix.is_some());

        app.load_media_path_with_probe(&path, |_| -> Result<MediaFile, MediaProbeError> {
            panic!("unchanged cached source must not be probed again")
        });

        let session = app
            .state
            .audio_mix
            .as_ref()
            .expect("same-source Audio Mix session should be restored");
        assert!(!app.state.intake_active);
        assert_eq!(session.selection.selected_count(), 0);
        assert_eq!(
            session.selection.track_gain(1),
            Some(TrackGain::from_tenths_db(-55))
        );
        assert_eq!(session.selection.container(), OutputContainer::Mkv);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        assert!(session.selection.normalization_enabled());
        assert_eq!(
            session.selection.normalization_targets(),
            normalization_targets
        );
        assert_eq!(
            session.selection.request(&session.media).normalization,
            LoudnessNormalization::Enabled(normalization_targets)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_back_returns_to_intake_and_same_source_restores_ready_state() {
        let (directory, path) = temp_file("upscale-back");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let session = app
            .state
            .upscale
            .as_mut()
            .expect("upscale session should exist");
        session
            .selection
            .set_target(&session.media, UpscaleTarget::Uhd8k);
        session.selection.set_codec(VideoCodec::H265);
        let custom_output = directory.join("custom-upscale.mp4");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.handle_upscale_action(UpscaleSelectionAction::Back, egui::Context::default())
            .unwrap();

        assert!(app.state.intake_active);
        assert_eq!(app.state.current_tool, Tool::VideoUpscale);
        assert!(app.state.upscale.is_some());

        app.load_media_path_with_probe(&path, |_| -> Result<MediaFile, MediaProbeError> {
            panic!("unchanged cached source must not be probed again")
        });

        let session = app
            .state
            .upscale
            .as_ref()
            .expect("same-source Upscale session should be restored");
        assert!(!app.state.intake_active);
        assert_eq!(session.selection.target(), UpscaleTarget::Uhd8k);
        assert_eq!(session.selection.codec(), VideoCodec::H265);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replacement_source_retires_preserved_ready_sessions() {
        let (directory, first) = temp_file("replace-after-back");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::AudioMix);
        assert!(app.state.audio_mix.is_some());
        let old_audio_mix = app.state.audio_mix.as_mut().unwrap();
        old_audio_mix.selection.set_normalization_enabled(true);
        assert!(
            old_audio_mix
                .selection
                .set_normalization_targets(LoudnessTargets::from_tenths(-220, -9, 135))
        );
        app.select_tool(Tool::VideoUpscale);
        let old_output = directory.join("old-custom.mp4");
        app.state
            .upscale
            .as_mut()
            .expect("upscale session should exist")
            .selection
            .set_custom_output_path(old_output);
        app.handle_upscale_action(UpscaleSelectionAction::Back, egui::Context::default())
            .unwrap();

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));

        assert!(app.state.audio_mix.is_none());
        let session = app
            .state
            .upscale
            .as_ref()
            .expect("replacement should create a new Upscale session");
        assert!(!app.state.intake_active);
        assert_eq!(session.media.path, second);
        assert!(!session.selection.output_is_customized());
        assert_eq!(session.selection.target(), UpscaleTarget::Uhd4k);
        assert_eq!(session.selection.codec(), VideoCodec::H264);
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&second)
        );

        app.select_tool(Tool::AudioMix);
        let audio_mix = app
            .state
            .audio_mix
            .as_ref()
            .expect("replacement should create a new Audio Mix session");
        assert!(
            audio_mix
                .selection
                .request(&audio_mix.media)
                .selected_tracks
                .iter()
                .all(|track| track.gain == TrackGain::ZERO)
        );
        assert!(!audio_mix.selection.normalization_enabled());
        assert_eq!(
            audio_mix.selection.normalization_targets(),
            LoudnessTargets::default()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changed_cached_file_is_not_reused_when_switching_tools() {
        let (directory, path) = temp_file("changed-switch");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.state.return_to_file_intake(Tool::AudioMix);
        fs::write(&path, b"changed-size").unwrap();

        app.select_tool(Tool::VideoUpscale);

        assert!(app.state.intake_active);
        assert!(app.state.upscale.is_none());
        assert!(app.state.media_intake.current().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lifecycle_routing_remains_operation_specific() {
        assert!(!shows_audio_mix_lifecycle(&AudioMixExecutionState::Idle));
        assert!(!shows_upscale_lifecycle(&UpscaleExecutionState::Idle));
    }

    #[test]
    fn file_drop_policy_disables_replacement_while_either_operation_runs() {
        assert!(file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Running,
            &UpscaleExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Running,
        ));
    }

    #[test]
    fn drag_hover_marks_only_one_available_file_as_a_valid_target() {
        let (first_directory, first) = temp_file("hover-first");
        let (second_directory, second) = temp_file("hover-second");
        let mut app = app();

        assert!(drag_hovered(&app, &[&first]));
        assert!(!drag_hovered(&app, &[&first, &second]));

        let _sender = begin_pending_inspection(&mut app, &first);
        assert!(!drag_hovered(&app, &[&second]));

        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn audio_mix_terminal_states_reject_source_replacement() {
        for state in terminal_execution_states() {
            assert!(!file_intake_enabled(&state, &UpscaleExecutionState::Idle,));
        }
    }

    #[test]
    fn upscale_terminal_states_reject_source_replacement() {
        for state in terminal_execution_states() {
            assert!(!file_intake_enabled(&AudioMixExecutionState::Idle, &state,));
        }
    }

    #[test]
    fn terminal_reset_to_idle_reenables_file_intake() {
        let completed = AudioMixExecutionState::Completed {
            output: PathBuf::from("finished.mp4"),
        };
        assert!(!file_intake_enabled(
            &completed,
            &UpscaleExecutionState::Idle,
        ));

        assert!(file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
        ));
    }
}
