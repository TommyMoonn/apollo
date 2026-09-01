use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

use egui::{CentralPanel, Context, Ui};

use crate::file_dialog::{
    DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogOwner, FileDialogRequest,
    FileDialogStartError, OutputFormat, SaveDialogOptions, WindowOwner,
};
use crate::progress_repaint::MediaProgressRepaintOwner;
use crate::settings::AppSettings;

use apollo::{
    audio_extract::{
        execution::{AudioExtractExecutionState, AudioExtractExecutor, AudioExtractStartError},
        model::AudioExtractFormat,
    },
    audio_mix::{
        execution::{AudioMixExecutionState, AudioMixExecutor, AudioMixStartError},
        model::OutputContainer,
    },
    frame_capture::{
        execution::{FrameCaptureExecutionState, FrameCaptureExecutor, FrameCaptureStartError},
        model::FrameCaptureFormat,
        thumbnail::{FrameCaptureThumbnail, decode_frame_capture_thumbnail},
    },
    media::{
        container::MediaContainer,
        error::{MediaError, MediaErrorKind},
        intake::{MediaIntake, MediaIntakeRequest, MediaLoadStart, MediaProbeDisposition},
        model::MediaFile,
        probe::{MediaProbeError, probe_media},
        tools::{MediaTools, MediaToolsError, discover_media_tools},
    },
    remux::execution::{RemuxExecutionState, RemuxExecutor, RemuxStartError},
    ui::{
        SourceAction, audio_extract as audio_extract_ui,
        audio_extract::{AudioExtractSelectionAction, AudioExtractSelectionState},
        audio_extract_progress::{self, AudioExtractLifecycleAction, AudioExtractLifecycleUiState},
        audio_mix as audio_mix_ui,
        audio_mix::{AudioMixSelectionAction, AudioMixSelectionState},
        audio_mix_progress::{self, AudioMixLifecycleAction, AudioMixLifecycleUiState},
        file_intake::{self, FileIntakeAction},
        fonts, frame_capture as frame_capture_ui,
        frame_capture::{FrameCaptureSelectionAction, FrameCaptureSelectionState},
        frame_capture_progress::{
            self, FrameCaptureLifecycleAction, FrameCaptureLifecycleUiState,
            FrameCaptureThumbnailRequest,
        },
        layout, media_error, remux as remux_ui,
        remux::{RemuxSelectionAction, RemuxSelectionState},
        remux_progress::{self, RemuxLifecycleAction, RemuxLifecycleUiState},
        source_presentation::SourcePresentation,
        theme, upscale as upscale_ui,
        upscale::{UpscaleSelectionAction, UpscaleSelectionState},
        upscale_progress::{self, UpscaleLifecycleAction, UpscaleLifecycleUiState},
        widgets,
        workspace::{self, Operation as Tool, WorkspaceAction},
    },
    upscale::execution::{UpscaleExecutionState, UpscaleExecutor, UpscaleStartError},
};

fn file_intake_enabled(
    audio_state: &AudioMixExecutionState,
    upscale_state: &UpscaleExecutionState,
    remux_state: &RemuxExecutionState,
    audio_extract_state: &AudioExtractExecutionState,
    frame_capture_state: &FrameCaptureExecutionState,
) -> bool {
    // Done/Back owns terminal settlement. Intake resumes only after every executor returns to Idle.
    matches!(audio_state, AudioMixExecutionState::Idle)
        && matches!(upscale_state, UpscaleExecutionState::Idle)
        && matches!(remux_state, RemuxExecutionState::Idle)
        && matches!(audio_extract_state, AudioExtractExecutionState::Idle)
        && matches!(frame_capture_state, FrameCaptureExecutionState::Idle)
}

fn operation_workspace_visible(
    execution_state: &AudioMixExecutionState,
    has_ready_session: bool,
) -> bool {
    matches!(execution_state, AudioMixExecutionState::Idle) && has_ready_session
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum NavigationState {
    #[default]
    NoSource,
    SourceWorkspace {
        active_operation: Tool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioMixToolState {
    session_id: u64,
    media: MediaFile,
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
struct RemuxToolState {
    session_id: u64,
    media: MediaFile,
    selection: RemuxSelectionState,
    lifecycle_ui: RemuxLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioExtractToolState {
    session_id: u64,
    media: MediaFile,
    selection: AudioExtractSelectionState,
    lifecycle_ui: AudioExtractLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameCaptureToolState {
    session_id: u64,
    media: MediaFile,
    selection: FrameCaptureSelectionState,
    lifecycle_ui: FrameCaptureLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppState {
    navigation: NavigationState,
    intake_error: Option<MediaError>,
    intake_error_details_open: bool,
    media_intake: MediaIntake,
    source_presentation: Option<Arc<SourcePresentation>>,
    audio_mix: Option<AudioMixToolState>,
    upscale: Option<UpscaleToolState>,
    remux: Option<RemuxToolState>,
    audio_extract: Option<AudioExtractToolState>,
    frame_capture: Option<FrameCaptureToolState>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            navigation: NavigationState::NoSource,
            intake_error: None,
            intake_error_details_open: false,
            media_intake: MediaIntake::default(),
            source_presentation: None,
            audio_mix: None,
            upscale: None,
            remux: None,
            audio_extract: None,
            frame_capture: None,
        }
    }
}

struct MediaInspection {
    request: MediaIntakeRequest,
    receiver: Receiver<Result<MediaFile, MediaProbeError>>,
}

struct FrameCaptureThumbnailLoad {
    session_id: u64,
    request_id: u64,
    receiver: Receiver<Result<FrameCaptureThumbnail, String>>,
}

impl FrameCaptureThumbnailLoad {
    fn spawn(
        session_id: u64,
        request: &FrameCaptureThumbnailRequest,
        context: Context,
    ) -> io::Result<Self> {
        let output = request.output().to_path_buf();
        Self::spawn_with(session_id, request.request_id(), context, move || {
            decode_frame_capture_thumbnail(&output)
        })
    }

    fn spawn_with<F>(
        session_id: u64,
        request_id: u64,
        context: Context,
        decode: F,
    ) -> io::Result<Self>
    where
        F: FnOnce() -> Result<FrameCaptureThumbnail, String> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-frame-thumbnail".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode));
                if let Ok(result) = result {
                    let _ = sender.send(result);
                }
                drop(sender);
                context.request_repaint();
            })?;
        Ok(Self {
            session_id,
            request_id,
            receiver,
        })
    }
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
    fn active_operation(&self) -> Option<Tool> {
        match self.navigation {
            NavigationState::NoSource => None,
            NavigationState::SourceWorkspace { active_operation } => Some(active_operation),
        }
    }

    fn enter_workspace(&mut self, active_operation: Tool) {
        self.navigation = NavigationState::SourceWorkspace { active_operation };
        self.intake_error = None;
        self.intake_error_details_open = false;
    }

    fn close_source(&mut self) {
        self.navigation = NavigationState::NoSource;
        self.media_intake.close_source();
        self.source_presentation = None;
        self.retire_tool_sessions();
        self.intake_error = None;
        self.intake_error_details_open = false;
    }

    fn retire_tool_sessions(&mut self) {
        self.audio_mix = None;
        self.upscale = None;
        self.remux = None;
        self.audio_extract = None;
        self.frame_capture = None;
    }
}

pub(crate) struct ApolloApp {
    state: AppState,
    media_tools: MediaToolsState,
    media_tools_discovery: Option<MediaToolsDiscovery>,
    audio_mix_executor: AudioMixExecutor,
    upscale_executor: UpscaleExecutor,
    remux_executor: RemuxExecutor,
    audio_extract_executor: AudioExtractExecutor,
    frame_capture_executor: FrameCaptureExecutor,
    frame_capture_thumbnail_load: Option<FrameCaptureThumbnailLoad>,
    progress_repaint: MediaProgressRepaintOwner,
    media_inspection: Option<MediaInspection>,
    file_dialog: FileDialogOwner,
    settings: AppSettings,
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
        let native_window_owner = WindowOwner::from_window(creation_context);
        let mut app = Self {
            state: AppState::default(),
            media_tools: MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                "Media tool discovery has not started.".to_owned(),
            )),
            media_tools_discovery: None,
            audio_mix_executor: AudioMixExecutor::new(),
            upscale_executor: UpscaleExecutor::new(),
            remux_executor: RemuxExecutor::new(),
            audio_extract_executor: AudioExtractExecutor::new(),
            frame_capture_executor: FrameCaptureExecutor::new(),
            frame_capture_thumbnail_load: None,
            progress_repaint: MediaProgressRepaintOwner::new(egui_context.clone()),
            media_inspection: None,
            file_dialog: FileDialogOwner::new(egui_context.clone(), native_window_owner),
            settings: AppSettings::load(creation_context.storage),
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
                self.remux_executor.state(),
                self.audio_extract_executor.state(),
                self.frame_capture_executor.state(),
            )
    }

    fn workspace_interaction_enabled(&self) -> bool {
        self.media_inspection.is_none()
    }

    fn handle_source_action(&mut self, action: SourceAction) {
        if !self.file_intake_enabled() {
            return;
        }
        match action {
            SourceAction::ChangeFile => self.start_file_dialog(FileDialogRequest::OpenSource {
                initial_directory: None,
            }),
            SourceAction::CloseSource => self.close_source(),
        }
    }

    fn close_source(&mut self) {
        self.frame_capture_thumbnail_load = None;
        self.state.close_source();
    }

    fn select_tool(&mut self, tool: Tool) {
        if !self.file_intake_enabled()
            || !matches!(
                self.state.navigation,
                NavigationState::SourceWorkspace { .. }
            )
        {
            return;
        }
        if self.activate_cached_source(tool) {
            self.state.navigation = NavigationState::SourceWorkspace {
                active_operation: tool,
            };
        }
    }

    fn activate_cached_source(&mut self, tool: Tool) -> bool {
        let Some(media) = self.state.media_intake.current().cloned() else {
            self.state.navigation = NavigationState::NoSource;
            self.state.retire_tool_sessions();
            return false;
        };
        self.state
            .source_presentation
            .get_or_insert_with(|| Arc::new(SourcePresentation::new(&media)));

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
            Tool::Remux => {
                if !self
                    .state
                    .remux
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.remux = Some(RemuxToolState {
                        session_id,
                        selection: RemuxSelectionState::new(&media),
                        lifecycle_ui: RemuxLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
            Tool::AudioExtract => {
                if !self
                    .state
                    .audio_extract
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.audio_extract = Some(AudioExtractToolState {
                        session_id,
                        selection: AudioExtractSelectionState::new(&media),
                        lifecycle_ui: AudioExtractLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
            Tool::FrameCapture => {
                if !self
                    .state
                    .frame_capture
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.frame_capture = Some(FrameCaptureToolState {
                        session_id,
                        selection: FrameCaptureSelectionState::new(&media),
                        lifecycle_ui: FrameCaptureLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
        }
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

    fn operation_ready_surface(&self, active_operation: Tool) -> bool {
        match active_operation {
            Tool::AudioMix => operation_workspace_visible(
                self.audio_mix_executor.state(),
                self.state
                    .audio_mix
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::VideoUpscale => operation_workspace_visible(
                self.upscale_executor.state(),
                self.state
                    .upscale
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::Remux => operation_workspace_visible(
                self.remux_executor.state(),
                self.state
                    .remux
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::AudioExtract => operation_workspace_visible(
                self.audio_extract_executor.state(),
                self.state
                    .audio_extract
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::FrameCapture => operation_workspace_visible(
                self.frame_capture_executor.state(),
                self.state
                    .frame_capture
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
        }
    }

    fn show_navigation(&mut self, ui: &mut Ui) {
        let NavigationState::SourceWorkspace { active_operation } = self.state.navigation else {
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };

        let Some(media) = self.state.media_intake.current() else {
            self.close_source();
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };
        let source_surface_key = egui::Id::new(media.path.as_path());
        let Some(source_presentation) = self.state.source_presentation.clone() else {
            self.close_source();
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };

        if self.operation_ready_surface(active_operation) {
            let workspace_action = layout::workspace_chrome(ui, |ui| {
                workspace::show(
                    ui,
                    &source_presentation.workspace,
                    active_operation,
                    self.media_inspection.is_none()
                        && file_intake_enabled(
                            self.audio_mix_executor.state(),
                            self.upscale_executor.state(),
                            self.remux_executor.state(),
                            self.audio_extract_executor.state(),
                            self.frame_capture_executor.state(),
                        ),
                )
            })
            .inner;
            match workspace_action {
                WorkspaceAction::Source(action) => self.handle_source_action(action),
                WorkspaceAction::SelectOperation(operation) => self.select_tool(operation),
                WorkspaceAction::None => {}
            }
        }

        let Some(active_operation) = self.state.active_operation() else {
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };
        let surface_key = egui::Id::new((active_operation.label(), source_surface_key));
        let ready_surface = self.operation_ready_surface(active_operation);
        if ready_surface && let Some(error) = self.state.intake_error.as_ref() {
            layout::content_gutter(ui, |ui| {
                media_error::show_inline(ui, error, &mut self.state.intake_error_details_open);
                ui.add_space(theme::SPACING.section);
            });
        }
        let show_operation = |app: &mut Self, ui: &mut Ui| match active_operation {
            Tool::AudioMix if app.state.audio_mix.is_some() => app.show_audio_mix_tool(ui),
            Tool::VideoUpscale if app.state.upscale.is_some() => app.show_upscale_tool(ui),
            Tool::Remux if app.state.remux.is_some() => app.show_remux_tool(ui),
            Tool::AudioExtract if app.state.audio_extract.is_some() => {
                app.show_audio_extract_tool(ui)
            }
            Tool::FrameCapture if app.state.frame_capture.is_some() => {
                app.show_frame_capture_tool(ui)
            }
            _ => {
                layout::padded_content(ui, |ui| app.show_file_intake(ui));
            }
        };
        layout::surface_fade(ui, surface_key, |ui| {
            if ready_surface {
                show_operation(self, ui);
            } else {
                layout::padded_content(ui, |ui| show_operation(self, ui));
            }
        });
    }

    #[cfg(test)]
    fn show_tool(&mut self, ui: &mut Ui, tool: Tool) {
        self.select_tool(tool);
        self.show_navigation(ui);
    }

    fn show_file_intake(&mut self, ui: &mut Ui) {
        let action = file_intake::show(
            ui,
            self.media_inspection.is_some(),
            self.state.intake_error.as_ref(),
            &mut self.state.intake_error_details_open,
        );
        match action {
            FileIntakeAction::OpenFile => self.handle_source_action(SourceAction::ChangeFile),
            FileIntakeAction::None => {}
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

        self.state.intake_error = None;
        self.state.intake_error_details_open = false;
        match self.state.media_intake.begin_load(path) {
            Ok(MediaLoadStart::Reused) => {
                let operation = self.state.active_operation().unwrap_or(Tool::AudioMix);
                if self.activate_cached_source(operation) {
                    self.state.enter_workspace(operation);
                }
                None
            }
            Ok(MediaLoadStart::Probe(request)) => Some(request),
            Err(error) => {
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
                self.state.intake_error = Some(MediaError::probe_failure(
                    "Media inspection worker stopped before returning a result.",
                ));
                self.state.intake_error_details_open = false;
            }
        }
    }

    fn poll_frame_capture_thumbnail(&mut self) {
        let result = match self.frame_capture_thumbnail_load.as_ref() {
            Some(load) => load.receiver.try_recv(),
            None => return,
        };

        let result = match result {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Frame Capture preview worker stopped before returning a result.".to_owned())
            }
        };
        let load = self
            .frame_capture_thumbnail_load
            .take()
            .expect("thumbnail load exists while its result is settled");
        let Some(session) = self
            .state
            .frame_capture
            .as_mut()
            .filter(|session| session.session_id == load.session_id)
        else {
            return;
        };
        session
            .lifecycle_ui
            .settle_thumbnail(load.request_id, result);
    }

    fn complete_media_probe(
        &mut self,
        request: MediaIntakeRequest,
        result: Result<MediaFile, MediaProbeError>,
    ) {
        match self.state.media_intake.complete_probe(request, result) {
            Ok(MediaProbeDisposition::Published) => {
                let operation = self.state.active_operation().unwrap_or(Tool::AudioMix);
                self.state.source_presentation = self
                    .state
                    .media_intake
                    .current()
                    .map(|media| Arc::new(SourcePresentation::new(media)));
                self.frame_capture_thumbnail_load = None;
                self.state.retire_tool_sessions();
                if self.activate_cached_source(operation) {
                    self.state.enter_workspace(operation);
                }
            }
            Ok(MediaProbeDisposition::Retired) => {}
            Err(error) => {
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
            self.handle_audio_mix_lifecycle_action(action);
            return;
        }

        if self.show_audio_mix_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .audio_mix
                    .as_mut()
                    .expect("audio mix source exists while ready surface is shown");
                audio_mix_ui::show(
                    ui,
                    &session.media,
                    &presentation.audio_mix,
                    &mut session.selection,
                )
            })
            .inner;
        if let Err(error) = self.handle_audio_mix_action(action)
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
    ) -> Result<(), AudioMixStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        match action {
            AudioMixSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let request = audio_mix_output_dialog_request(session);
                self.start_file_dialog(request);
            }
            AudioMixSelectionAction::MixRequested => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = session.selection.request(&session.media);
                let wake = self.progress_repaint.begin();
                self.audio_mix_executor
                    .start_with_wake(&tools, &session.media, &request, wake)?;
            }
            AudioMixSelectionAction::None => {}
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
            self.handle_upscale_lifecycle_action(action);
            return;
        }

        if self.show_upscale_start_error(ui) {
            return;
        }

        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let upscale = self
                    .state
                    .upscale
                    .as_mut()
                    .expect("upscale source exists while ready surface is shown");
                upscale_ui::show(ui, &upscale.media, &mut upscale.selection)
            })
            .inner;
        if let Err(error) = self.handle_upscale_action(action)
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
    ) -> Result<(), UpscaleStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        match action {
            UpscaleSelectionAction::BrowseOutput => {
                let Some(session) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let request = upscale_output_dialog_request(session);
                self.start_file_dialog(request);
            }
            UpscaleSelectionAction::UpscaleRequested => {
                let Some(upscale) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = upscale.selection.request(&upscale.media);
                let wake = self.progress_repaint.begin();
                self.upscale_executor
                    .start_with_wake(&tools, &upscale.media, &request, wake)?;
            }
            UpscaleSelectionAction::None => {}
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

    fn show_remux_tool(&mut self, ui: &mut Ui) {
        if shows_remux_lifecycle(self.remux_executor.state()) {
            let action = {
                let remux = self
                    .state
                    .remux
                    .as_mut()
                    .expect("remux source exists while lifecycle surface is shown");
                let request = remux.selection.request(&remux.media);
                remux_progress::show(
                    ui,
                    &request,
                    &mut self.remux_executor,
                    &mut remux.lifecycle_ui,
                )
            };
            self.handle_remux_lifecycle_action(action);
            return;
        }

        if self.show_remux_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let remux = self
                    .state
                    .remux
                    .as_mut()
                    .expect("remux source exists while ready surface is shown");
                remux_ui::show(ui, &remux.media, &presentation.remux, &mut remux.selection)
            })
            .inner;
        if let Err(error) = self.handle_remux_action(action)
            && let Some(session) = self.state.remux.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_remux_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.remux.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Remux could not start"
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

    fn handle_remux_action(&mut self, action: RemuxSelectionAction) -> Result<(), RemuxStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        match action {
            RemuxSelectionAction::BrowseOutput => {
                let Some(session) = self.state.remux.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(remux_output_dialog_request(session));
            }
            RemuxSelectionAction::RemuxRequested => {
                let Some(remux) = self.state.remux.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = remux.selection.request(&remux.media);
                let wake = self.progress_repaint.begin();
                self.remux_executor
                    .start_with_wake(&tools, &remux.media, &request, wake)?;
            }
            RemuxSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_remux_lifecycle_action(&self, action: RemuxLifecycleAction) {
        if let RemuxLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Remux output folder could not be opened: {error}");
        }
    }

    fn show_audio_extract_tool(&mut self, ui: &mut Ui) {
        if shows_audio_extract_lifecycle(self.audio_extract_executor.state()) {
            let action = {
                let session = self
                    .state
                    .audio_extract
                    .as_mut()
                    .expect("audio extract source exists while lifecycle surface is shown");
                let request = session
                    .selection
                    .request(&session.media)
                    .expect("running audio extract session owns a selected audio stream");
                audio_extract_progress::show(
                    ui,
                    &request,
                    &mut self.audio_extract_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_audio_extract_lifecycle_action(action);
            return;
        }

        if self.show_audio_extract_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .audio_extract
                    .as_mut()
                    .expect("audio extract source exists while ready surface is shown");
                audio_extract_ui::show(
                    ui,
                    &session.media,
                    &presentation.audio_extract,
                    &mut session.selection,
                )
            })
            .inner;
        if let Err(error) = self.handle_audio_extract_action(action)
            && let Some(session) = self.state.audio_extract.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_audio_extract_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.audio_extract.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Audio extraction could not start"
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

    fn handle_audio_extract_action(
        &mut self,
        action: AudioExtractSelectionAction,
    ) -> Result<(), AudioExtractStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        match action {
            AudioExtractSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(audio_extract_output_dialog_request(session));
            }
            AudioExtractSelectionAction::ExtractRequested => {
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return Ok(());
                };
                let Some(request) = session.selection.request(&session.media) else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let wake = self.progress_repaint.begin();
                self.audio_extract_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
            }
            AudioExtractSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_audio_extract_lifecycle_action(&self, action: AudioExtractLifecycleAction) {
        if let AudioExtractLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Extract output folder could not be opened: {error}");
        }
    }

    fn show_frame_capture_tool(&mut self, ui: &mut Ui) {
        if shows_frame_capture_lifecycle(self.frame_capture_executor.state()) {
            let action = {
                let session = self
                    .state
                    .frame_capture
                    .as_mut()
                    .expect("frame capture source exists while lifecycle surface is shown");
                let request = session
                    .selection
                    .request(&session.media)
                    .expect("running frame capture session owns a valid timestamp");
                frame_capture_progress::show(
                    ui,
                    &request,
                    &mut self.frame_capture_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_frame_capture_lifecycle_action(action);
            return;
        }

        if self.show_frame_capture_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .frame_capture
                    .as_mut()
                    .expect("frame capture source exists while ready surface is shown");
                frame_capture_ui::show(
                    ui,
                    &session.media,
                    &presentation.frame_capture,
                    &mut session.selection,
                )
            })
            .inner;
        if let Err(error) = self.handle_frame_capture_action(action)
            && let Some(session) = self.state.frame_capture.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_frame_capture_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.frame_capture.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Frame capture could not start"
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

    fn handle_frame_capture_action(
        &mut self,
        action: FrameCaptureSelectionAction,
    ) -> Result<(), FrameCaptureStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        match action {
            FrameCaptureSelectionAction::BrowseOutput => {
                let Some(session) = self.state.frame_capture.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(frame_capture_output_dialog_request(session));
            }
            FrameCaptureSelectionAction::CaptureRequested => {
                let Some(session) = self.state.frame_capture.as_ref() else {
                    return Ok(());
                };
                let Some(request) = session.selection.request(&session.media) else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let wake = self.progress_repaint.begin();
                self.frame_capture_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
                self.frame_capture_thumbnail_load = None;
            }
            FrameCaptureSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_frame_capture_lifecycle_action(&mut self, action: FrameCaptureLifecycleAction) {
        match action {
            FrameCaptureLifecycleAction::LoadThumbnail(request) => {
                self.start_frame_capture_thumbnail_load(request);
            }
            FrameCaptureLifecycleAction::OpenFolder(path) => {
                if let Err(error) = open_folder(&path) {
                    eprintln!("Frame Capture output folder could not be opened: {error}");
                }
            }
            FrameCaptureLifecycleAction::None
            | FrameCaptureLifecycleAction::CancelRequested
            | FrameCaptureLifecycleAction::Done
            | FrameCaptureLifecycleAction::Back => {}
        }
    }

    fn start_frame_capture_thumbnail_load(&mut self, request: FrameCaptureThumbnailRequest) {
        let Some(session_id) = self
            .state
            .frame_capture
            .as_ref()
            .map(|session| session.session_id)
        else {
            return;
        };
        match FrameCaptureThumbnailLoad::spawn(session_id, &request, self.egui_context.clone()) {
            Ok(load) => self.frame_capture_thumbnail_load = Some(load),
            Err(error) => {
                if let Some(session) = self
                    .state
                    .frame_capture
                    .as_mut()
                    .filter(|session| session.session_id == session_id)
                {
                    session.lifecycle_ui.settle_thumbnail(
                        request.request_id(),
                        Err(format!(
                            "Frame Capture preview worker could not start: {error}"
                        )),
                    );
                }
            }
        }
    }

    fn prepare_file_dialog_request(&self, mut request: FileDialogRequest) -> FileDialogRequest {
        match &mut request {
            FileDialogRequest::OpenSource { initial_directory } => {
                *initial_directory = self.settings.source_directory();
            }
            FileDialogRequest::SaveOutput { options, .. } => {
                let source_fallback = options
                    .initial_directory
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("."));
                options.initial_directory = self.settings.output_directory(&source_fallback);
            }
        }
        request
    }

    fn start_file_dialog(&mut self, request: FileDialogRequest) {
        let request = self.prepare_file_dialog_request(request);
        let target = request.target();
        if let Err(error) = self.file_dialog.request(request)
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
        if let DialogOutcome::Selected(path) = &completion.outcome {
            match completion.target {
                DialogTarget::MediaInput => self.settings.remember_source(path),
                _ => self.settings.remember_output(path),
            }
        }
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
                ToolExecutionStates {
                    audio_mix: self.audio_mix_executor.state(),
                    upscale: self.upscale_executor.state(),
                    remux: self.remux_executor.state(),
                    audio_extract: self.audio_extract_executor.state(),
                    frame_capture: self.frame_capture_executor.state(),
                },
                target,
                outcome,
            ),
        }
    }
}

struct ToolExecutionStates<'a> {
    audio_mix: &'a AudioMixExecutionState,
    upscale: &'a UpscaleExecutionState,
    remux: &'a RemuxExecutionState,
    audio_extract: &'a AudioExtractExecutionState,
    frame_capture: &'a FrameCaptureExecutionState,
}

fn settle_output_file_dialog(
    state: &mut AppState,
    execution: ToolExecutionStates<'_>,
    target: DialogTarget,
    outcome: DialogOutcome,
) {
    match (target, outcome) {
        (DialogTarget::AudioMixOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.audio_mix, AudioMixExecutionState::Idle) =>
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
            if matches!(execution.audio_mix, AudioMixExecutionState::Idle) =>
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
            if matches!(execution.upscale, UpscaleExecutionState::Idle) =>
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
            if matches!(execution.upscale, UpscaleExecutionState::Idle) =>
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
        (DialogTarget::RemuxOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.remux, RemuxExecutionState::Idle) =>
        {
            if let Some(session) = state
                .remux
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::RemuxOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.remux, RemuxExecutionState::Idle) =>
        {
            if let Some(session) = state
                .remux
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::AudioExtractOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.audio_extract, AudioExtractExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_extract
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::AudioExtractOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.audio_extract, AudioExtractExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_extract
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::FrameCaptureOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.frame_capture, FrameCaptureExecutionState::Idle) =>
        {
            if let Some(session) = state
                .frame_capture
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::FrameCaptureOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.frame_capture, FrameCaptureExecutionState::Idle) =>
        {
            if let Some(session) = state
                .frame_capture
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
        initial_directory: Some(initial_directory),
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

fn remux_output_dialog_request(session: &RemuxToolState) -> FileDialogRequest {
    let format = match session.selection.container() {
        MediaContainer::Mp4 => OutputFormat::Mp4,
        MediaContainer::Matroska => OutputFormat::Mkv,
    };
    output_dialog_request(
        DialogTarget::RemuxOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn audio_extract_output_dialog_request(session: &AudioExtractToolState) -> FileDialogRequest {
    let format = match session.selection.format() {
        AudioExtractFormat::Original => OutputFormat::Mka,
        AudioExtractFormat::Aac => OutputFormat::M4a,
        AudioExtractFormat::Flac => OutputFormat::Flac,
    };
    output_dialog_request(
        DialogTarget::AudioExtractOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn frame_capture_output_dialog_request(session: &FrameCaptureToolState) -> FileDialogRequest {
    let format = match session.selection.format() {
        FrameCaptureFormat::Png => OutputFormat::Png,
        FrameCaptureFormat::Jpeg => OutputFormat::Jpeg,
    };
    output_dialog_request(
        DialogTarget::FrameCaptureOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn shows_audio_mix_lifecycle(state: &AudioMixExecutionState) -> bool {
    !matches!(state, AudioMixExecutionState::Idle)
}

fn shows_upscale_lifecycle(state: &UpscaleExecutionState) -> bool {
    !matches!(state, UpscaleExecutionState::Idle)
}

fn shows_remux_lifecycle(state: &RemuxExecutionState) -> bool {
    !matches!(state, RemuxExecutionState::Idle)
}

fn shows_audio_extract_lifecycle(state: &AudioExtractExecutionState) -> bool {
    !matches!(state, AudioExtractExecutionState::Idle)
}

fn shows_frame_capture_lifecycle(state: &FrameCaptureExecutionState) -> bool {
    !matches!(state, FrameCaptureExecutionState::Idle)
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
            .frame(layout::root_frame(ui.style()))
            .show(ui, |ui| {
                self.poll_media_tools_discovery();
                if matches!(&self.media_tools, MediaToolsState::Checking { .. }) {
                    layout::padded_content(ui, |ui| self.show_media_tools_checking(ui));
                    return;
                }
                if matches!(&self.media_tools, MediaToolsState::Unavailable(_)) {
                    layout::padded_content(ui, |ui| self.show_media_tools_unavailable(ui));
                    return;
                }

                self.poll_file_dialog();
                self.poll_media_inspection();
                self.poll_frame_capture_thumbnail();

                self.handle_window_file_drop(ui.ctx());
                if self.drag_hovered(ui.ctx()) {
                    file_intake::show_drag_hover(ui);
                }

                self.show_navigation(ui);
            });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.save(storage);
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
        ApolloApp, AppSettings, AppState, FrameCaptureThumbnailLoad, MediaInspection,
        MediaToolsState, NavigationState, Tool, ToolExecutionStates,
        audio_extract_output_dialog_request, audio_mix_output_dialog_request, file_intake_enabled,
        frame_capture_output_dialog_request, operation_workspace_visible,
        remux_output_dialog_request, settle_output_file_dialog, shows_audio_extract_lifecycle,
        shows_audio_mix_lifecycle, shows_frame_capture_lifecycle, shows_remux_lifecycle,
        shows_upscale_lifecycle,
    };
    use crate::file_dialog::{
        DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogOwner, FileDialogRequest,
        FileDialogStartError, OutputFormat,
    };
    use crate::progress_repaint::MediaProgressRepaintOwner;
    use apollo::{
        audio_extract::{execution::AudioExtractExecutionState, model::AudioExtractFormat},
        audio_mix::{
            execution::AudioMixExecutionState,
            model::{LoudnessNormalization, LoudnessTargets, OutputContainer, TrackGain},
        },
        frame_capture::{
            execution::FrameCaptureExecutionState, model::FrameCaptureFormat,
            thumbnail::FrameCaptureThumbnail,
        },
        media::{
            container::MediaContainer,
            error::{MediaError, MediaErrorKind},
            execution::MediaExecutionFailure,
            intake::MediaLoadStart,
            model::{AudioStream, FrameRate, MediaFile, MediaStream, StreamKind, VideoStream},
            probe::MediaProbeError,
            process::MediaProcessFailure,
            tools::{MediaTools, MediaToolsError},
        },
        remux::execution::RemuxExecutionState,
        ui::{
            SourceAction, audio_extract::AudioExtractSelectionAction, fonts,
            frame_capture::FrameCaptureSelectionAction,
            frame_capture_progress::FrameCaptureThumbnailState, layout,
            remux::RemuxSelectionAction, theme, upscale::UpscaleSelectionAction,
        },
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
            streams: vec![
                MediaStream {
                    stream_index: 0,
                    kind: StreamKind::Video,
                    codec: Some("h264".to_owned()),
                },
                MediaStream {
                    stream_index: 1,
                    kind: StreamKind::Audio,
                    codec: Some("aac".to_owned()),
                },
            ],
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

    fn thumbnail() -> FrameCaptureThumbnail {
        FrameCaptureThumbnail::from_rgba([2, 1], vec![255; 8]).unwrap()
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
            remux_executor: apollo::remux::execution::RemuxExecutor::new(),
            audio_extract_executor: apollo::audio_extract::execution::AudioExtractExecutor::new(),
            frame_capture_executor: apollo::frame_capture::execution::FrameCaptureExecutor::new(),
            frame_capture_thumbnail_load: None,
            progress_repaint: MediaProgressRepaintOwner::new(egui_context.clone()),
            media_inspection: None,
            file_dialog: FileDialogOwner::new_for_test(egui_context.clone(), || {
                Ok(|_: &FileDialogRequest| Ok(None))
            }),
            settings: AppSettings::default(),
            egui_context,
            next_media_tools_attempt_id: 0,
            next_session_id: 1,
            tools_error_details_open: false,
        }
    }

    fn assert_no_source(state: &AppState) {
        assert_eq!(state.navigation, NavigationState::NoSource);
        assert!(state.media_intake.current().is_none());
    }

    fn assert_workspace(state: &AppState, operation: Tool) {
        assert_eq!(
            state.navigation,
            NavigationState::SourceWorkspace {
                active_operation: operation,
            }
        );
        assert!(state.media_intake.current().is_some());
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

    fn settled_navigation_repaint_delay(context: &egui::Context, app: &mut ApolloApp) -> Duration {
        for _ in 0..8 {
            let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
            let repaint_delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
            if repaint_delay == Duration::MAX {
                return repaint_delay;
            }
        }
        Duration::ZERO
    }

    fn shape_text_count(shape: &egui::Shape, needle: &str) -> usize {
        match shape {
            egui::Shape::Text(text) => usize::from(text.galley.job.text == needle),
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .map(|shape| shape_text_count(shape, needle))
                .sum(),
            _ => 0,
        }
    }

    fn output_text_count(output: &egui::FullOutput, needle: &str) -> usize {
        output
            .shapes
            .iter()
            .map(|shape| shape_text_count(&shape.shape, needle))
            .sum()
    }

    fn shape_text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.job.text == text => {
                Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
            }
            egui::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| shape_text_rect(shape, text))
            }
            _ => None,
        }
    }

    fn output_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|clipped| shape_text_rect(&clipped.shape, text))
            .unwrap_or_else(|| panic!("missing rendered text {text:?}"))
    }

    fn output_bottom_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
        fn collect(shape: &egui::Shape, text: &str, matches: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => {
                    matches.push(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, text, matches);
                    }
                }
                _ => {}
            }
        }

        let mut matches = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, text, &mut matches);
        }
        matches
            .into_iter()
            .max_by(|left, right| left.bottom().total_cmp(&right.bottom()))
            .unwrap_or_else(|| panic!("missing rendered text {text:?}"))
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
    fn fresh_application_state_starts_without_a_source() {
        let state = AppState::default();

        assert_no_source(&state);
        assert!(state.audio_mix.is_none());
        assert!(state.upscale.is_none());
        assert!(state.remux.is_none());
        assert!(state.audio_extract.is_none());
    }

    #[test]
    fn initialization_has_no_source_without_starting_an_executor() {
        let app = app();

        assert_no_source(&app.state);
        assert!(!app.audio_mix_executor.is_running());
        assert!(!app.upscale_executor.is_running());
        assert!(!app.remux_executor.is_running());
        assert!(!app.audio_extract_executor.is_running());
    }

    #[test]
    fn fresh_application_state_renders_unified_file_intake() {
        let mut app = app();
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });

        assert!(output_contains_text(&output, "Open files"));
        assert!(!output_contains_text(&output, "Audio Mix"));
        assert!(!output_contains_text(&output, "Upscale"));
        output.textures_delta.clear();
    }

    #[test]
    fn idle_no_source_surface_schedules_no_application_repaint() {
        let mut app = app();
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        theme::set_motion_durations(&context, theme::MotionDurations::DISABLED);

        let repaint_delay = settled_navigation_repaint_delay(&context, &mut app);

        assert_eq!(repaint_delay, Duration::MAX);
    }

    #[test]
    fn idle_source_workspace_schedules_no_application_repaint() {
        let (directory, path) = temp_file("idle-workspace-repaint");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        theme::set_motion_durations(&context, theme::MotionDurations::DISABLED);

        let repaint_delay = settled_navigation_repaint_delay(&context, &mut app);

        assert_eq!(repaint_delay, Duration::MAX);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspecting_state_is_sparse_and_blocks_source_replacement() {
        let (directory, path) = temp_file("inspecting-state");
        let mut app = app();
        let _sender = begin_pending_inspection(&mut app, &path);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        assert_eq!(app.state.navigation, NavigationState::NoSource);
        assert!(app.media_inspection.is_some());
        assert!(!app.file_intake_enabled());

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });

        assert!(output_contains_text(&output, "Inspecting media…"));
        assert!(!output_contains_text(&output, "Audio Mix"));
        assert!(!output_contains_text(&output, "Open files"));
        assert!(!output_contains_text(&output, "or drop a video anywhere"));
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_completion_enters_default_source_workspace() {
        let (directory, path) = temp_file("inspection-mode-switch");
        let mut app = app();
        let sender = begin_pending_inspection(&mut app, &path);

        sender.send(Ok(media(&path))).unwrap();
        app.poll_media_inspection();

        assert!(app.media_inspection.is_none());
        assert_workspace(&app.state, Tool::AudioMix);
        assert!(app.state.audio_mix.is_some());
        assert_eq!(
            app.state
                .audio_mix
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
    fn frame_thumbnail_decode_runs_on_the_dedicated_worker() {
        let (thread_sender, thread_receiver) = mpsc::sync_channel(1);
        let load =
            FrameCaptureThumbnailLoad::spawn_with(7, 11, egui::Context::default(), move || {
                thread_sender
                    .send(std::thread::current().name().map(str::to_owned))
                    .unwrap();
                Ok(thumbnail())
            })
            .expect("thumbnail worker should start");

        assert_eq!(
            thread_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Some("apollo-frame-thumbnail".to_owned())
        );
        assert!(
            load.receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_ok()
        );
    }

    #[test]
    fn thumbnail_completion_routes_to_its_originating_frame_capture_session() {
        let (directory, path) = temp_file("thumbnail-routing");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let session_id = session.session_id;
        let request = session
            .lifecycle_ui
            .begin_thumbnail(Path::new("capture.png"))
            .unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        app.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
            session_id,
            request_id: request.request_id(),
            receiver,
        });

        sender.send(Ok(thumbnail())).unwrap();
        app.poll_frame_capture_thumbnail();

        assert!(app.frame_capture_thumbnail_load.is_none());
        assert!(matches!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .lifecycle_ui
                .thumbnail(),
            FrameCaptureThumbnailState::Ready { request_id, .. }
                if *request_id == request.request_id()
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_replacement_clears_thumbnail_and_rejects_stale_completion() {
        let (first_directory, first_path) = temp_file("thumbnail-first-source");
        let (second_directory, second_path) = temp_file("thumbnail-second-source");
        let mut app = app();
        app.load_media_path_with_probe(&first_path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let old_session = app.state.frame_capture.as_mut().unwrap();
        let old_session_id = old_session.session_id;
        let old_request = old_session
            .lifecycle_ui
            .begin_thumbnail(Path::new("old-capture.png"))
            .unwrap();

        app.load_media_path_with_probe(&second_path, |path| Ok(media(path)));
        assert!(app.frame_capture_thumbnail_load.is_none());
        let replacement = app.state.frame_capture.as_ref().unwrap();
        assert_ne!(replacement.session_id, old_session_id);
        assert!(matches!(
            replacement.lifecycle_ui.thumbnail(),
            FrameCaptureThumbnailState::Empty
        ));

        let (sender, receiver) = mpsc::sync_channel(1);
        app.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
            session_id: old_session_id,
            request_id: old_request.request_id(),
            receiver,
        });
        sender.send(Ok(thumbnail())).unwrap();
        app.poll_frame_capture_thumbnail();
        assert!(matches!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .lifecycle_ui
                .thumbnail(),
            FrameCaptureThumbnailState::Empty
        ));

        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn close_source_releases_thumbnail_worker_and_session_state() {
        let (directory, path) = temp_file("thumbnail-close-source");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let request = session
            .lifecycle_ui
            .begin_thumbnail(Path::new("capture.png"))
            .unwrap();
        let (_sender, receiver) = mpsc::sync_channel(1);
        app.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
            session_id: session.session_id,
            request_id: request.request_id(),
            receiver,
        });

        app.handle_source_action(SourceAction::CloseSource);

        assert!(app.frame_capture_thumbnail_load.is_none());
        assert!(app.state.frame_capture.is_none());
        assert_no_source(&app.state);
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
        assert_no_source(&app.state);
        assert!(app.state.media_intake.current().is_none());
        assert!(app.state.audio_mix.is_none());
        assert!(app.state.upscale.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });
        output.textures_delta.clear();
        assert!(output_contains_text(
            &output,
            "This file could not be inspected."
        ));
        assert!(output_contains_text(&output, "Open files"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn operation_selection_is_navigation_inside_a_loaded_workspace() {
        let (directory, path) = temp_file("mode-switch");
        let mut app = app();

        app.select_tool(Tool::VideoUpscale);
        assert_no_source(&app.state);

        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        assert_workspace(&app.state, Tool::AudioMix);
        let source_path = app.state.media_intake.current().unwrap().path.clone();

        for operation in [
            Tool::VideoUpscale,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
            Tool::AudioMix,
        ] {
            app.select_tool(operation);
            assert_workspace(&app.state, operation);
            assert_eq!(app.state.media_intake.current().unwrap().path, source_path);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn accepted_source_presentation_is_reused_until_source_replacement() {
        let (directory, first) = temp_file("source-presentation");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        let original_presentation = app.state.source_presentation.as_ref().unwrap().clone();

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        for operation in [
            Tool::AudioMix,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
        ] {
            app.select_tool(operation);
            let output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
            output.drop_without_applying_deltas();
            assert!(Arc::ptr_eq(
                app.state.source_presentation.as_ref().unwrap(),
                &original_presentation
            ));
        }

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        assert!(!Arc::ptr_eq(
            app.state.source_presentation.as_ref().unwrap(),
            &original_presentation
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn change_file_cancel_preserves_complete_workspace() {
        let (directory, path) = temp_file("change-cancel");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        app.state
            .frame_capture
            .as_mut()
            .unwrap()
            .selection
            .set_format(FrameCaptureFormat::Jpeg);
        let before = app.state.clone();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Cancelled,
        });

        assert_eq!(app.state, before);
        assert!(app.state.intake_error.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn change_file_keeps_workspace_until_replacement_probe_publishes() {
        let (directory, first) = temp_file("change-transaction");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_container(MediaContainer::Mp4);
        app.state.intake_error = Some(MediaError::probe_failure("previous replacement failed"));
        let old_session = app.state.remux.clone().unwrap();

        let request = app
            .begin_media_path(&second)
            .expect("replacement should require inspection");

        assert_workspace(&app.state, Tool::Remux);
        assert!(app.state.intake_error.is_none());
        assert_eq!(app.state.remux.as_ref(), Some(&old_session));
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&first)
        );

        app.complete_media_probe(request, Ok(media(&second)));

        assert_workspace(&app.state, Tool::Remux);
        let replacement = app.state.remux.as_ref().unwrap();
        assert_ne!(replacement.session_id, old_session.session_id);
        assert_eq!(replacement.media.path, second);
        assert_eq!(replacement.selection.container(), MediaContainer::Matroska);
        assert!(app.state.intake_error.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn close_source_returns_to_true_empty_intake() {
        let (directory, path) = temp_file("close-source");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        for operation in [
            Tool::VideoUpscale,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
        ] {
            app.select_tool(operation);
        }

        app.handle_source_action(SourceAction::CloseSource);

        assert_no_source(&app.state);
        assert!(app.state.audio_mix.is_none());
        assert!(app.state.upscale.is_none());
        assert!(app.state.remux.is_none());
        assert!(app.state.audio_extract.is_none());
        assert!(app.state.frame_capture.is_none());
        assert!(app.state.source_presentation.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_valid_file_routes_audio_mix_to_ready_state() {
        let (directory, path) = temp_file("open");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        assert_workspace(&app.state, Tool::AudioMix);
        assert_eq!(
            app.state.audio_mix.as_ref().map(|s| &s.media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dropped_valid_file_enters_default_workspace() {
        let (directory, path) = temp_file("drop");
        let mut app = app();
        assert!(file_intake_enabled(
            app.audio_mix_executor.state(),
            app.upscale_executor.state(),
            app.remux_executor.state(),
            app.audio_extract_executor.state(),
            app.frame_capture_executor.state(),
        ));
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        assert_workspace(&app.state, Tool::AudioMix);
        assert_eq!(
            app.state.audio_mix.as_ref().map(|s| &s.media.path),
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
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::VideoUpscale);
        });
        output.textures_delta.clear();

        assert!(output_contains_text(&output, "Upscale to"));
        assert!(output_contains_text(&output, "Output"));
        assert!(output_contains_text(&output, "H.264"));
        assert!(output_contains_text(&output, "Browse"));
        assert!(!output_contains_text(&output, "Video codec"));
        assert!(!output_contains_text(&output, "Actual output"));
        assert!(!output_contains_text(&output, "← Back"));
        assert!(output_contains_text(&output, "Change file"));
        assert!(output_contains_text(&output, "Close source"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_remux_source_renders_ready_surface() {
        let (directory, path) = temp_file("remux-ready-render");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::Remux);
        });
        output.textures_delta.clear();

        for text in [
            "recording.mkv",
            "1920×1080",
            "MKV",
            "Output",
            "Browse",
            "Remux",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        assert!(!output_contains_text(&output, "Container"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_audio_extract_source_renders_ready_surface() {
        let (directory, path) = temp_file("audio-extract-ready-render");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioExtract);
        });
        output.textures_delta.clear();
        for text in [
            "recording.mkv",
            "1920×1080",
            "Audio track",
            "Desktop Audio",
            "stream 1",
            "Original (copy)",
            "no re-encode · MKA",
            "Output",
            "Browse",
            "Extract",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        assert!(!output_contains_text(&output, "Format"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_frame_capture_source_renders_ready_surface() {
        let (directory, path) = temp_file("frame-capture-ready-render");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::FrameCapture);
        });
        output.textures_delta.clear();
        for text in [
            "recording.mkv",
            "1920×1080",
            "Timestamp",
            "00:00:00.000",
            "PNG",
            "lossless",
            "Output",
            "Browse",
            "Capture frame",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        assert!(!output_contains_text(&output, "Format"));
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
        output.textures_delta.clear();

        assert!(output_contains_text(&output, "Audio tracks"));
        assert!(output_contains_text(&output, "Loudness normalization"));
        assert!(output_contains_text(&output, "Output"));
        assert!(output_contains_text(&output, "MP4"));
        assert!(output_contains_text(&output, "Browse"));
        assert!(!output_contains_text(&output, "Integrated loudness"));
        assert!(output_contains_text(&output, "Mix selected tracks"));
        assert!(!output_contains_text(&output, "← Back"));
        assert!(output_contains_text(&output, "Change file"));
        let audio_tracks = output_text_rect(&output, "Audio tracks");
        let select_all = output_text_rect(&output, "Select all");
        let clear = output_text_rect(&output, "Clear");
        let selected = output_text_rect(&output, "1 selected");
        let reset_gains = output_text_rect(&output, "Reset gains");
        let loudness = output_text_rect(&output, "Loudness normalization");
        assert!((audio_tracks.center().y - select_all.center().y).abs() <= 1.0);
        assert!((audio_tracks.center().y - clear.center().y).abs() <= 1.0);
        assert!((selected.center().y - reset_gains.center().y).abs() <= 1.0);
        assert!(selected.top() > audio_tracks.bottom());
        assert!(loudness.top() > selected.bottom());

        app.state
            .audio_mix
            .as_mut()
            .unwrap()
            .selection
            .set_normalization_enabled(true);
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioMix);
        });
        output.textures_delta.clear();
        for label in [
            "Integrated loudness",
            "Maximum true peak",
            "Loudness range",
            "Reset normalization targets",
        ] {
            assert!(output_contains_text(&output, label), "missing {label}");
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shared_workspace_chrome_is_identical_across_all_ready_operations() {
        let (directory, path) = temp_file("shared-workspace-chrome");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        for operation in Tool::ALL {
            let mut output = context.run_ui(Default::default(), |ui| {
                app.show_tool(ui, operation);
            });
            output.textures_delta.clear();

            assert_eq!(output_text_count(&output, "recording.mkv"), 1);
            for label in [
                "1920×1080",
                "Change file",
                "Close source",
                "Audio Mix",
                "Upscale",
                "Remux",
                "Audio Extract",
                "Frame Capture",
            ] {
                assert!(
                    output_contains_text(&output, label),
                    "{operation:?} missing workspace label {label:?}"
                );
            }
            assert!(!output_contains_text(&output, "← Back"));
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn running_and_terminal_operation_states_hide_workspace_chrome() {
        let non_ready_states = [
            AudioMixExecutionState::Running,
            AudioMixExecutionState::Completed {
                output: PathBuf::from("finished.mp4"),
            },
            AudioMixExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: "failed".to_owned(),
            }),
            AudioMixExecutionState::Cancelled,
        ];

        assert!(operation_workspace_visible(
            &AudioMixExecutionState::Idle,
            true
        ));
        assert!(!operation_workspace_visible(
            &AudioMixExecutionState::Idle,
            false
        ));
        for state in non_ready_states {
            assert!(!operation_workspace_visible(&state, true));
        }
    }

    #[test]
    fn standalone_operation_feedback_does_not_render_workspace_components() {
        let (directory, path) = temp_file("standalone-operation-feedback");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        app.state.frame_capture.as_mut().unwrap().start_error =
            Some(MediaError::invalid_request("simulated start failure"));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        output.textures_delta.clear();

        assert!(output_contains_text(
            &output,
            "Frame capture could not start"
        ));
        for workspace_label in [
            "Change file",
            "Close source",
            "Audio Mix",
            "Upscale",
            "Remux",
            "Audio Extract",
            "Frame Capture",
        ] {
            assert!(!output_contains_text(&output, workspace_label));
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_ready_operation_keeps_its_primary_action_bottom_owned_at_all_heights() {
        let (directory, path) = temp_file("shared-operation-shell");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let action_height = theme::PRIMARY_CONTROL_HEIGHT
            + f32::from(layout::ACTION_BAR_VERTICAL_PADDING) * 2.0
            + theme::SEPARATOR_WIDTH;

        for (operation, label) in [
            (Tool::AudioMix, "Mix selected tracks"),
            (Tool::VideoUpscale, "Upscale"),
            (Tool::Remux, "Remux"),
            (Tool::AudioExtract, "Extract"),
            (Tool::FrameCapture, "Capture frame"),
        ] {
            for (width, height) in [
                (400.0, 260.0),
                (620.0, 379.0),
                (1_200.0, 700.0),
                (620.0, 900.0),
            ] {
                let mut viewport = egui::Rect::NOTHING;
                let mut output = context.run_ui(Default::default(), |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                    viewport = rect;
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    );
                    child.set_clip_rect(rect);
                    app.show_tool(&mut child, operation);
                });
                output.textures_delta.clear();
                let action = output_bottom_text_rect(&output, label);
                let source = output_text_rect(&output, "recording.mkv");

                assert!(
                    (action.center().y - (viewport.bottom() - action_height * 0.5)).abs() <= 1.0,
                    "{operation:?} at {width}x{height}: action={action:?}, viewport={viewport:?}"
                );
                assert!(source.top() >= viewport.top() + f32::from(layout::CONTENT_PADDING));
                assert!(source.bottom() < action.top());
            }
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_ready_selection_survives_same_source_navigation() {
        let (directory, path) = temp_file("selection-preserve");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
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
        assert_eq!(options.initial_directory, Some(directory.clone()));
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
    fn remux_output_dialog_request_follows_current_container() {
        let (directory, path) = temp_file("remux-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);

        let session = app.state.remux.as_ref().unwrap();
        let FileDialogRequest::SaveOutput { target, options } =
            remux_output_dialog_request(session)
        else {
            panic!("remux browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::RemuxOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, Some(directory.clone()));
        assert_eq!(options.file_name, "recording-remuxed.mkv");
        assert_eq!(options.format, OutputFormat::Mkv);

        let session = app.state.remux.as_mut().unwrap();
        session.selection.set_container(MediaContainer::Mp4);
        let FileDialogRequest::SaveOutput { options, .. } = remux_output_dialog_request(session)
        else {
            unreachable!();
        };
        assert_eq!(options.file_name, "recording-remuxed.mp4");
        assert_eq!(options.format, OutputFormat::Mp4);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_extract_output_dialog_request_follows_current_format() {
        let (directory, path) = temp_file("audio-extract-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);

        let session = app.state.audio_extract.as_ref().unwrap();
        let FileDialogRequest::SaveOutput { target, options } =
            audio_extract_output_dialog_request(session)
        else {
            panic!("audio extract Browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::AudioExtractOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, Some(directory.clone()));
        assert_eq!(options.file_name, "recording-audio.mka");
        assert_eq!(options.format, OutputFormat::Mka);

        let session = app.state.audio_extract.as_mut().unwrap();
        for (format, name, dialog_format) in [
            (
                AudioExtractFormat::Aac,
                "recording-audio.m4a",
                OutputFormat::M4a,
            ),
            (
                AudioExtractFormat::Flac,
                "recording-audio.flac",
                OutputFormat::Flac,
            ),
        ] {
            session.selection.set_format(format);
            let FileDialogRequest::SaveOutput { options, .. } =
                audio_extract_output_dialog_request(session)
            else {
                unreachable!();
            };
            assert_eq!(options.file_name, name);
            assert_eq!(options.format, dialog_format);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn frame_capture_output_dialog_request_follows_current_format() {
        let (directory, path) = temp_file("frame-capture-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);

        let session = app.state.frame_capture.as_ref().unwrap();
        let FileDialogRequest::SaveOutput { target, options } =
            frame_capture_output_dialog_request(session)
        else {
            panic!("frame capture Browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::FrameCaptureOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, Some(directory.clone()));
        assert_eq!(options.file_name, "recording-frame-00-00-00-000.png");
        assert_eq!(options.format, OutputFormat::Png);

        let session = app.state.frame_capture.as_mut().unwrap();
        session.selection.set_format(FrameCaptureFormat::Jpeg);
        let FileDialogRequest::SaveOutput { options, .. } =
            frame_capture_output_dialog_request(session)
        else {
            unreachable!();
        };
        assert_eq!(options.file_name, "recording-frame-00-00-00-000.jpg");
        assert_eq!(options.format, OutputFormat::Jpeg);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dialog_requests_prefer_independent_remembered_directories() {
        let (source_directory, source_path) = temp_file("remember-source");
        let output_directory = source_directory.join("remembered-output");
        fs::create_dir_all(&output_directory).unwrap();
        let mut app = app();
        app.settings
            .remember_source(&source_directory.join("previous-source.mp4"));
        app.settings
            .remember_output(&output_directory.join("previous-output.mkv"));

        let open = app.prepare_file_dialog_request(FileDialogRequest::OpenSource {
            initial_directory: None,
        });
        assert_eq!(
            open,
            FileDialogRequest::OpenSource {
                initial_directory: Some(source_directory.clone()),
            }
        );

        app.load_media_path_with_probe(&source_path, |path| Ok(media(path)));
        let save = app.prepare_file_dialog_request(audio_mix_output_dialog_request(
            app.state.audio_mix.as_ref().unwrap(),
        ));
        let FileDialogRequest::SaveOutput { options, .. } = save else {
            unreachable!();
        };
        assert_eq!(options.initial_directory, Some(output_directory));
        fs::remove_dir_all(source_directory).unwrap();
    }

    #[test]
    fn dialog_cancel_does_not_change_directory_memory() {
        let (directory, path) = temp_file("remember-cancel");
        let mut app = app();
        app.settings.remember_source(&path);
        app.settings
            .remember_output(&directory.join("existing.mkv"));
        let before = app.settings.clone();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Cancelled,
        });
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput { session_id: 1 },
            outcome: DialogOutcome::Cancelled,
        });

        assert_eq!(app.settings, before);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn successful_dialog_selections_update_only_their_directory_history() {
        let (directory, source_path) = temp_file("remember-success");
        let output_directory = directory.join("output");
        fs::create_dir_all(&output_directory).unwrap();
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Selected(source_path),
        });
        assert_eq!(app.settings.source_directory(), Some(directory.clone()));
        assert_eq!(app.settings.output_directory(Path::new("missing")), None);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput { session_id: 999 },
            outcome: DialogOutcome::Selected(output_directory.join("chosen.mkv")),
        });
        assert_eq!(app.settings.source_directory(), Some(directory.clone()));
        assert_eq!(
            app.settings.output_directory(Path::new("missing")),
            Some(output_directory)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn selected_output_paths_route_to_the_owning_tool() {
        let (directory, path) = temp_file("upscale-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
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

        app.select_tool(Tool::Remux);
        let remux_session_id = app.state.remux.as_ref().unwrap().session_id;
        let remux_destination = directory.join("chosen remux.mkv");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::RemuxOutput {
                session_id: remux_session_id,
            },
            outcome: DialogOutcome::Selected(remux_destination.clone()),
        });
        assert_eq!(
            app.state.remux.as_ref().unwrap().selection.output_path(),
            remux_destination
        );

        app.select_tool(Tool::AudioExtract);
        let extract_session_id = app.state.audio_extract.as_ref().unwrap().session_id;
        let extract_destination = directory.join("chosen audio.mka");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioExtractOutput {
                session_id: extract_session_id,
            },
            outcome: DialogOutcome::Selected(extract_destination.clone()),
        });
        assert_eq!(
            app.state
                .audio_extract
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            extract_destination
        );

        app.select_tool(Tool::FrameCapture);
        let capture_session_id = app.state.frame_capture.as_ref().unwrap().session_id;
        let capture_destination = directory.join("chosen frame.png");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::FrameCaptureOutput {
                session_id: capture_session_id,
            },
            outcome: DialogOutcome::Selected(capture_destination.clone()),
        });
        assert_eq!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            capture_destination
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_audio_extract_completion_cannot_mutate_replacement_session() {
        let (directory, first) = temp_file("stale-audio-extract");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);
        let stale_session_id = app.state.audio_extract.as_ref().unwrap().session_id;

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_before = app
            .state
            .audio_extract
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioExtractOutput {
                session_id: stale_session_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale.mka")),
        });
        assert_eq!(
            app.state
                .audio_extract
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            replacement_before
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_frame_capture_completion_cannot_mutate_replacement_session() {
        let (directory, first) = temp_file("stale-frame-capture");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let stale_session_id = app.state.frame_capture.as_ref().unwrap().session_id;

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_before = app
            .state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::FrameCaptureOutput {
                session_id: stale_session_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale.png")),
        });
        assert_eq!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            replacement_before
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_remux_output_completion_cannot_mutate_replacement_session() {
        let (directory, first) = temp_file("stale-remux");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let stale_session_id = app.state.remux.as_ref().unwrap().session_id;

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_before = app.state.remux.as_ref().unwrap().selection.output_path();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::RemuxOutput {
                session_id: stale_session_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale-result.mkv")),
        });

        let replacement = app.state.remux.as_ref().unwrap();
        assert_ne!(replacement.session_id, stale_session_id);
        assert_eq!(replacement.selection.output_path(), replacement_before);
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
        let mut release_receiver = Some(release_receiver);
        app.file_dialog = FileDialogOwner::new_for_test(egui::Context::default(), move || {
            Ok(move |_: &FileDialogRequest| {
                if let Some(receiver) = release_receiver.take() {
                    receiver.recv().unwrap();
                }
                Ok(None)
            })
        });
        app.file_dialog
            .request(FileDialogRequest::OpenSource {
                initial_directory: None,
            })
            .unwrap();

        assert!(!app.file_intake_enabled());
        assert!(app.workspace_interaction_enabled());
        assert_workspace(&app.state, Tool::AudioMix);
        assert_eq!(
            app.file_dialog.request(FileDialogRequest::OpenSource {
                initial_directory: None,
            }),
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
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Running,
                upscale: &UpscaleExecutionState::Idle,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Idle,
                frame_capture: &FrameCaptureExecutionState::Idle,
            },
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
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Idle,
                upscale: &UpscaleExecutionState::Running,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Idle,
                frame_capture: &FrameCaptureExecutionState::Idle,
            },
            DialogTarget::UpscaleOutput {
                session_id: upscale_id,
            },
            DialogOutcome::Selected(directory.join("late-upscale.mp4")),
        );
        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            upscale_output
        );

        app.select_tool(Tool::AudioExtract);
        let extract_id = app.state.audio_extract.as_ref().unwrap().session_id;
        let extract_output = app
            .state
            .audio_extract
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        settle_output_file_dialog(
            &mut app.state,
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Idle,
                upscale: &UpscaleExecutionState::Idle,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Running,
                frame_capture: &FrameCaptureExecutionState::Idle,
            },
            DialogTarget::AudioExtractOutput {
                session_id: extract_id,
            },
            DialogOutcome::Selected(directory.join("late-audio.mka")),
        );
        assert_eq!(
            app.state
                .audio_extract
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            extract_output
        );

        app.select_tool(Tool::FrameCapture);
        let capture_id = app.state.frame_capture.as_ref().unwrap().session_id;
        let capture_output = app
            .state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        settle_output_file_dialog(
            &mut app.state,
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Idle,
                upscale: &UpscaleExecutionState::Idle,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Idle,
                frame_capture: &FrameCaptureExecutionState::Running,
            },
            DialogTarget::FrameCaptureOutput {
                session_id: capture_id,
            },
            DialogOutcome::Selected(directory.join("late-frame.png")),
        );
        assert_eq!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            capture_output
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
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
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
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);

        let error = app
            .handle_upscale_action(UpscaleSelectionAction::UpscaleRequested)
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
    fn remux_intent_reaches_executor() {
        let (directory, path) = temp_file("remux-execute");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);

        let error = app
            .handle_remux_action(RemuxSelectionAction::RemuxRequested)
            .expect_err("missing test FFmpeg should prove the execution path was attempted");

        assert!(matches!(
            error,
            apollo::remux::execution::RemuxStartError::Process(
                apollo::media::process::MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound
                )
            )
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_extract_intent_reaches_executor() {
        let (directory, path) = temp_file("audio-extract-execute");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);

        let error = app
            .handle_audio_extract_action(AudioExtractSelectionAction::ExtractRequested)
            .expect_err("missing test FFmpeg should prove the execution path was attempted");
        assert!(matches!(
            error,
            apollo::audio_extract::execution::AudioExtractStartError::Process(
                apollo::media::process::MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound
                )
            )
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn frame_capture_intent_reaches_executor() {
        let (directory, path) = temp_file("frame-capture-execute");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);

        let error = app
            .handle_frame_capture_action(FrameCaptureSelectionAction::CaptureRequested)
            .expect_err("missing test FFmpeg should prove the execution path was attempted");
        assert!(matches!(
            error,
            apollo::frame_capture::execution::FrameCaptureStartError::Process(
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
    fn remux_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        assert!(!shows_remux_lifecycle(&RemuxExecutionState::Idle));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Running));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Completed {
            output: PathBuf::from("recording-remuxed.mkv"),
        }));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Failed(
            MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            }
        )));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Cancelled));
    }

    #[test]
    fn audio_extract_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        assert!(!shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Idle
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Running
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Completed {
                output: PathBuf::from("recording-audio.mka"),
            }
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            })
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Cancelled
        ));
    }

    #[test]
    fn frame_capture_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        assert!(!shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Idle
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Running
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Completed {
                output: PathBuf::from("recording-frame.png"),
            }
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            })
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Cancelled
        ));
    }

    #[test]
    fn change_file_probe_failure_is_rendered_and_survives_operation_switching() {
        let (directory, first) = temp_file("stale-first");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let before = app.state.clone();
        app.load_media_path_with_probe(&second, |_| {
            Err(apollo::media::probe::MediaProbeError::FfprobeFailed {
                code: Some(1),
                diagnostic: "probe failed".to_owned(),
            })
        });

        assert_workspace(&app.state, Tool::VideoUpscale);
        assert_eq!(app.state.upscale, before.upscale);
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&first)
        );
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });
        output.textures_delta.clear();
        assert!(output_contains_text(
            &output,
            "This file could not be inspected."
        ));
        assert!(output_contains_text(
            &output,
            "Check that the source is a readable media file and try again."
        ));
        assert!(output_contains_text(&output, "Upscale to"));
        assert!(output_contains_text(&output, "Change file"));
        assert!(output_contains_text(&output, "Audio Mix"));

        app.select_tool(Tool::Remux);

        assert_workspace(&app.state, Tool::Remux);
        assert_eq!(app.state.upscale, before.upscale);
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });
        output.textures_delta.clear();
        assert!(output_contains_text(
            &output,
            "This file could not be inspected."
        ));
        assert!(output_contains_text(&output, "Remux"));
        assert!(output_contains_text(&output, "Change file"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switching_tools_reuses_successful_probe_without_probing_again() {
        let (directory, path) = temp_file("reuse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        app.select_tool(Tool::VideoUpscale);

        assert_workspace(&app.state, Tool::VideoUpscale);
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
    fn audio_mix_state_survives_operation_switching() {
        let (directory, path) = temp_file("audio-back");
        let mut app = app();
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

        app.select_tool(Tool::VideoUpscale);
        app.select_tool(Tool::AudioMix);

        let session = app
            .state
            .audio_mix
            .as_ref()
            .expect("same-source Audio Mix session should be restored");
        assert_workspace(&app.state, Tool::AudioMix);
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
    fn upscale_state_survives_operation_switching() {
        let (directory, path) = temp_file("upscale-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
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

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::VideoUpscale);

        let session = app
            .state
            .upscale
            .as_ref()
            .expect("same-source Upscale session should be restored");
        assert_workspace(&app.state, Tool::VideoUpscale);
        assert_eq!(session.selection.target(), UpscaleTarget::Uhd8k);
        assert_eq!(session.selection.codec(), VideoCodec::H265);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remux_state_survives_operation_switching() {
        let (directory, path) = temp_file("remux-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let session = app
            .state
            .remux
            .as_mut()
            .expect("remux session should exist");
        session.selection.set_container(MediaContainer::Mp4);
        let custom_output = directory.join("custom-remux.mp4");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::Remux);

        let session = app
            .state
            .remux
            .as_ref()
            .expect("same-source Remux session should be restored");
        assert_workspace(&app.state, Tool::Remux);
        assert_eq!(session.selection.container(), MediaContainer::Mp4);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_extract_state_survives_operation_switching() {
        let (directory, path) = temp_file("audio-extract-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);
        let session = app
            .state
            .audio_extract
            .as_mut()
            .expect("audio extract session should exist");
        session.selection.set_format(AudioExtractFormat::Flac);
        let custom_output = directory.join("dialogue.flac");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::AudioExtract);
        let session = app
            .state
            .audio_extract
            .as_ref()
            .expect("same-source Audio Extract session should be restored");
        assert_workspace(&app.state, Tool::AudioExtract);
        assert_eq!(session.selection.selected_stream_index(), Some(1));
        assert_eq!(session.selection.format(), AudioExtractFormat::Flac);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn frame_capture_state_survives_operation_switching() {
        let (directory, path) = temp_file("frame-capture-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app
            .state
            .frame_capture
            .as_mut()
            .expect("frame capture session should exist");
        let inspected_media = media(&path);
        session
            .selection
            .set_timestamp_text(&inspected_media, "00:00:03.250".into());
        session.selection.set_format(FrameCaptureFormat::Jpeg);
        let custom_output = directory.join("chosen-frame.jpg");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::FrameCapture);
        let session = app
            .state
            .frame_capture
            .as_ref()
            .expect("same-source Frame Capture session should be restored");
        assert_workspace(&app.state, Tool::FrameCapture);
        assert_eq!(session.selection.timestamp_text(), "00:00:03.250");
        assert_eq!(session.selection.format(), FrameCaptureFormat::Jpeg);
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
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
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
        app.load_media_path_with_probe(&second, |path| Ok(media(path)));

        assert!(app.state.audio_mix.is_none());
        let session = app
            .state
            .upscale
            .as_ref()
            .expect("replacement should create a new Upscale session");
        assert_workspace(&app.state, Tool::VideoUpscale);
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
    fn switching_operations_does_not_revalidate_or_clear_the_source() {
        let (directory, path) = temp_file("changed-switch");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        fs::write(&path, b"changed-size").unwrap();

        app.select_tool(Tool::VideoUpscale);

        assert_workspace(&app.state, Tool::VideoUpscale);
        assert!(app.state.upscale.is_some());
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lifecycle_routing_remains_operation_specific() {
        assert!(!shows_audio_mix_lifecycle(&AudioMixExecutionState::Idle));
        assert!(!shows_upscale_lifecycle(&UpscaleExecutionState::Idle));
        assert!(!shows_remux_lifecycle(&RemuxExecutionState::Idle));
    }

    #[test]
    fn file_drop_policy_disables_replacement_while_any_operation_runs() {
        assert!(file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Running,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Running,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Running,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Running,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Running,
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
            assert!(!file_intake_enabled(
                &state,
                &UpscaleExecutionState::Idle,
                &RemuxExecutionState::Idle,
                &AudioExtractExecutionState::Idle,
                &FrameCaptureExecutionState::Idle,
            ));
        }
    }

    #[test]
    fn upscale_terminal_states_reject_source_replacement() {
        for state in terminal_execution_states() {
            assert!(!file_intake_enabled(
                &AudioMixExecutionState::Idle,
                &state,
                &RemuxExecutionState::Idle,
                &AudioExtractExecutionState::Idle,
                &FrameCaptureExecutionState::Idle,
            ));
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
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));

        assert!(file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
    }
}
