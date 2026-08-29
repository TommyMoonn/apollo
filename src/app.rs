use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};

use egui::{CentralPanel, Context, Ui};

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
        audio_mix::{AudioMixSelectionAction, AudioMixSelectionState},
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
const INSPECTION_REPAINT_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputPickerFormat {
    Mp4,
    Mkv,
}

impl OutputPickerFormat {
    #[cfg(any(target_os = "windows", test))]
    const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
        }
    }

    #[cfg(any(target_os = "windows", test))]
    const fn filter(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4 video (*.mp4)|*.mp4|All files (*.*)|*.*",
            Self::Mkv => "Matroska video (*.mkv)|*.mkv|All files (*.*)|*.*",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputPickerRequest {
    initial_directory: PathBuf,
    file_name: OsString,
    format: OutputPickerFormat,
}

impl OutputPickerRequest {
    fn new(output: &Path, format: OutputPickerFormat) -> Self {
        let initial_directory = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let file_name = output.file_name().unwrap_or_default().to_os_string();

        Self {
            initial_directory,
            file_name,
            format,
        }
    }
}

fn choose_output_destination<F>(
    request: &OutputPickerRequest,
    picker: F,
) -> Result<Option<PathBuf>, MediaError>
where
    F: FnOnce(&OutputPickerRequest) -> io::Result<Option<PathBuf>>,
{
    picker(request).map_err(|error| MediaError::output_picker_unavailable(error.to_string()))
}

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
    media: MediaFile,
    selection: AudioMixSelectionState,
    lifecycle_ui: AudioMixLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpscaleToolState {
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
    fn spawn(tools: MediaTools, request: MediaIntakeRequest) -> io::Result<Self> {
        let path = request.path().to_path_buf();
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-media-inspection".to_owned())
            .spawn(move || {
                let _ = sender.send(probe_media(&tools, &path));
            })?;

        Ok(Self { request, receiver })
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
    media_tools: Result<MediaTools, MediaToolsError>,
    audio_mix_executor: AudioMixExecutor,
    upscale_executor: UpscaleExecutor,
    media_inspection: Option<MediaInspection>,
    tools_error_details_open: bool,
}

impl ApolloApp {
    pub(crate) fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        fonts::install(&creation_context.egui_ctx);
        theme::install(&creation_context.egui_ctx);

        Self {
            state: AppState::default(),
            media_tools: discover_media_tools(),
            audio_mix_executor: AudioMixExecutor::new(),
            upscale_executor: UpscaleExecutor::new(),
            media_inspection: None,
            tools_error_details_open: false,
        }
    }

    fn file_intake_enabled(&self) -> bool {
        self.media_inspection.is_none()
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
                    self.state.audio_mix = Some(AudioMixToolState {
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
                    self.state.upscale = Some(UpscaleToolState {
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

    fn show_media_tools_unavailable(&mut self, ui: &mut Ui) {
        let error = self
            .media_tools
            .as_ref()
            .err()
            .map(MediaError::from_tools_error)
            .unwrap_or_else(|| MediaError::invalid_request("media tools are unavailable"));

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

    fn retry_media_tools(&mut self) {
        self.retry_media_tools_with(discover_media_tools);
    }

    fn retry_media_tools_with(
        &mut self,
        discover: impl FnOnce() -> Result<MediaTools, MediaToolsError>,
    ) {
        self.media_tools = discover();
        self.tools_error_details_open = false;
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
        let action = file_intake::show(
            ui,
            tool.intake_tool(),
            self.media_inspection.is_some(),
            self.state.intake_error.as_ref(),
            &mut self.state.intake_error_details_open,
        );
        match action {
            FileIntakeAction::OpenFile => match open_media_file_dialog() {
                Ok(Some(path)) => self.load_media_path(&path),
                Ok(None) => {}
                Err(error) => {
                    self.state.intake_error = Some(MediaError::invalid_request(error.to_string()));
                    self.state.intake_error_details_open = false;
                }
            },
            FileIntakeAction::SelectTool(selected) if selected != tool.intake_tool() => {
                self.select_tool(Tool::from_intake(selected));
            }
            FileIntakeAction::None | FileIntakeAction::SelectTool(_) => {}
        }
    }

    fn load_media_path(&mut self, path: &Path) {
        let Ok(tools) = self.media_tools.as_ref() else {
            return;
        };
        let tools = tools.clone();
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        let request_for_failure = request.clone();

        match MediaInspection::spawn(tools, request) {
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

        let session = self
            .state
            .audio_mix
            .as_mut()
            .expect("audio mix source exists while ready surface is shown");
        let action = audio_mix_ui::show(ui, &session.media, &mut session.selection);
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
            media_error::show_failure_content(
                ui,
                title,
                error,
                &mut session.start_error_details_open,
                "Back",
            )
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
        self.handle_audio_mix_action_with_picker(action, open_output_file_dialog)
    }

    fn handle_audio_mix_action_with_picker<F>(
        &mut self,
        action: AudioMixSelectionAction,
        picker: F,
    ) -> Result<(), AudioMixStartError>
    where
        F: FnOnce(&OutputPickerRequest) -> io::Result<Option<PathBuf>>,
    {
        match action {
            AudioMixSelectionAction::Back if self.file_intake_enabled() => {
                self.state.return_to_file_intake(Tool::AudioMix);
            }
            AudioMixSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let format = match session.selection.container() {
                    OutputContainer::Mp4 => OutputPickerFormat::Mp4,
                    OutputContainer::Mkv => OutputPickerFormat::Mkv,
                };
                let output = session.selection.output_path();
                let request = OutputPickerRequest::new(&output, format);

                match choose_output_destination(&request, picker) {
                    Ok(Some(path)) => {
                        let session = self
                            .state
                            .audio_mix
                            .as_mut()
                            .expect("audio mix session still exists after output browse");
                        session.selection.set_custom_output_path(path);
                        session.start_error = None;
                        session.start_error_details_open = false;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let session = self
                            .state
                            .audio_mix
                            .as_mut()
                            .expect("audio mix session still exists after output browse");
                        session.start_error = Some(error);
                        session.start_error_details_open = false;
                    }
                }
            }
            AudioMixSelectionAction::MixRequested => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let Ok(tools) = self.media_tools.as_ref() else {
                    return Ok(());
                };
                let request = session.selection.request(&session.media);
                self.audio_mix_executor
                    .start(tools, &session.media, &request)?;
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

        let upscale = self
            .state
            .upscale
            .as_mut()
            .expect("upscale source exists while ready surface is shown");
        let action = upscale_ui::show(ui, &upscale.media, &mut upscale.selection);
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
            media_error::show_failure_content(
                ui,
                title,
                error,
                &mut session.start_error_details_open,
                "Back",
            )
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
        self.handle_upscale_action_with_picker(action, open_output_file_dialog)
    }

    fn handle_upscale_action_with_picker<F>(
        &mut self,
        action: UpscaleSelectionAction,
        picker: F,
    ) -> Result<(), UpscaleStartError>
    where
        F: FnOnce(&OutputPickerRequest) -> io::Result<Option<PathBuf>>,
    {
        match action {
            UpscaleSelectionAction::Back if self.file_intake_enabled() => {
                self.state.return_to_file_intake(Tool::VideoUpscale);
            }
            UpscaleSelectionAction::BrowseOutput => {
                let Some(session) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let output = session.selection.output_path();
                let request = OutputPickerRequest::new(&output, OutputPickerFormat::Mp4);

                match choose_output_destination(&request, picker) {
                    Ok(Some(path)) => {
                        let session = self
                            .state
                            .upscale
                            .as_mut()
                            .expect("upscale session still exists after output browse");
                        session.selection.set_custom_output_path(path);
                        session.start_error = None;
                        session.start_error_details_open = false;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let session = self
                            .state
                            .upscale
                            .as_mut()
                            .expect("upscale session still exists after output browse");
                        session.start_error = Some(error);
                        session.start_error_details_open = false;
                    }
                }
            }
            UpscaleSelectionAction::UpscaleRequested => {
                let Some(upscale) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let Ok(tools) = self.media_tools.as_ref() else {
                    return Ok(());
                };
                let request = upscale.selection.request(&upscale.media);
                self.upscale_executor
                    .start(tools, &upscale.media, &request)?;
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
}

fn shows_audio_mix_lifecycle(state: &AudioMixExecutionState) -> bool {
    !matches!(state, AudioMixExecutionState::Idle)
}

fn shows_upscale_lifecycle(state: &UpscaleExecutionState) -> bool {
    !matches!(state, UpscaleExecutionState::Idle)
}

#[cfg(target_os = "windows")]
fn open_output_file_dialog(request: &OutputPickerRequest) -> io::Result<Option<PathBuf>> {
    const SCRIPT: &str = r#"Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.SaveFileDialog
$dialog.Title = 'Choose output destination'
$dialog.InitialDirectory = $env:APOLLO_OUTPUT_DIRECTORY
$dialog.FileName = $env:APOLLO_OUTPUT_FILENAME
$dialog.Filter = $env:APOLLO_OUTPUT_FILTER
$dialog.DefaultExt = $env:APOLLO_OUTPUT_EXTENSION
$dialog.AddExtension = $true
$dialog.OverwritePrompt = $true
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    [Console]::Write($dialog.FileName)
}"#;

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-Command", SCRIPT])
        .env("APOLLO_OUTPUT_DIRECTORY", &request.initial_directory)
        .env("APOLLO_OUTPUT_FILENAME", &request.file_name)
        .env("APOLLO_OUTPUT_FILTER", request.format.filter())
        .env("APOLLO_OUTPUT_EXTENSION", request.format.extension())
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let diagnostic = if stderr.is_empty() {
            format!("Windows output picker exited with {}", output.status)
        } else {
            stderr
        };
        return Err(io::Error::other(diagnostic));
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

#[cfg(not(target_os = "windows"))]
fn open_output_file_dialog(_request: &OutputPickerRequest) -> io::Result<Option<PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Output Browse is currently available on Windows.",
    ))
}

#[cfg(target_os = "windows")]
fn open_media_file_dialog() -> io::Result<Option<std::path::PathBuf>> {
    const SCRIPT: &str = r#"Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = 'Open media file'
$dialog.Filter = 'Video files|*.mp4;*.mkv;*.mov;*.avi;*.webm;*.m4v|All files|*.*'
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    [Console]::Write($dialog.FileName)
}"#;

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-Command", SCRIPT])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("Windows file picker failed"));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!path.is_empty()).then(|| std::path::PathBuf::from(path)))
}

#[cfg(not(target_os = "windows"))]
fn open_media_file_dialog() -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Open File is currently available on Windows; drag and drop is supported here.",
    ))
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
                if self.media_tools.is_err() {
                    self.show_media_tools_unavailable(ui);
                    return;
                }

                self.poll_media_inspection();
                if self.media_inspection.is_some() {
                    ui.ctx().request_repaint_after(INSPECTION_REPAINT_INTERVAL);
                }

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
        fs, io,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
    };

    use super::{
        ApolloApp, AppState, MediaInspection, OutputPickerFormat, Tool, file_intake_enabled,
        shows_audio_mix_lifecycle, shows_upscale_lifecycle,
    };
    use apollo::{
        audio_mix::{execution::AudioMixExecutionState, model::OutputContainer},
        media::{
            error::{MediaError, MediaErrorKind},
            execution::MediaExecutionFailure,
            model::{AudioStream, FrameRate, MediaFile, VideoStream},
            probe::MediaProbeError,
            process::MediaProcessFailure,
            tools::MediaTools,
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
        ApolloApp {
            state: AppState::default(),
            media_tools: Ok(MediaTools {
                ffmpeg: PathBuf::from("missing-ffmpeg"),
                ffprobe: PathBuf::from("missing-ffprobe"),
            }),
            audio_mix_executor: apollo::audio_mix::execution::AudioMixExecutor::new(),
            upscale_executor: apollo::upscale::execution::UpscaleExecutor::new(),
            media_inspection: None,
            tools_error_details_open: false,
        }
    }

    #[test]
    fn media_tools_retry_uses_the_shared_discovery_result() {
        let mut app = app();
        app.tools_error_details_open = true;
        let expected = MediaTools {
            ffmpeg: PathBuf::from(r"C:\Media Tools\ffmpeg.exe"),
            ffprobe: PathBuf::from(r"C:\Media Tools\ffprobe.exe"),
        };

        app.retry_media_tools_with(|| Ok(expected.clone()));

        assert_eq!(app.media_tools, Ok(expected));
        assert!(!app.tools_error_details_open);
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
        assert!(output_contains_text(&output, "Mix selected tracks"));
        assert!(output_contains_text(&output, "← Back"));
        assert!(!output_contains_text(&output, "Change"));
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
    fn audio_mix_browse_updates_destination_and_follows_container_format() {
        let (directory, path) = temp_file("audio-browse");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        let mp4_destination = directory.join("chosen mix.mp4");
        app.handle_audio_mix_action_with_picker(AudioMixSelectionAction::BrowseOutput, |request| {
            assert_eq!(request.initial_directory, directory);
            assert_eq!(request.file_name, "recording-mixed.mp4");
            assert_eq!(request.format, OutputPickerFormat::Mp4);
            assert_eq!(request.format.extension(), "mp4");
            assert!(request.format.filter().contains("*.mp4"));
            Ok(Some(mp4_destination.clone()))
        })
        .unwrap();
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            mp4_destination
        );

        let session = app.state.audio_mix.as_mut().unwrap();
        let media = session.media.clone();
        session
            .selection
            .set_container(&media, OutputContainer::Mkv);
        let mkv_destination = directory.join("chosen mix.mkv");
        app.handle_audio_mix_action_with_picker(AudioMixSelectionAction::BrowseOutput, |request| {
            assert_eq!(request.format, OutputPickerFormat::Mkv);
            assert_eq!(request.format.extension(), "mkv");
            assert!(request.format.filter().contains("*.mkv"));
            Ok(Some(mkv_destination.clone()))
        })
        .unwrap();
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            mkv_destination
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_browse_updates_destination_with_mp4_picker() {
        let (directory, path) = temp_file("upscale-browse");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let destination = directory.join("chosen upscale.mp4");

        app.handle_upscale_action_with_picker(UpscaleSelectionAction::BrowseOutput, |request| {
            assert_eq!(request.initial_directory, directory);
            assert_eq!(request.file_name, "recording-4k.mp4");
            assert_eq!(request.format, OutputPickerFormat::Mp4);
            assert_eq!(request.format.extension(), "mp4");
            assert!(request.format.filter().contains("*.mp4"));
            Ok(Some(destination.clone()))
        })
        .unwrap();

        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            destination
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

        app.handle_audio_mix_action_with_picker(AudioMixSelectionAction::BrowseOutput, |_| {
            Ok(None)
        })
        .unwrap();

        assert_eq!(app.state, before);
        fs::remove_dir_all(directory).unwrap();
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

        app.handle_upscale_action_with_picker(UpscaleSelectionAction::BrowseOutput, |_| {
            Err(io::Error::other("native picker diagnostic marker"))
        })
        .unwrap();

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
        session
            .selection
            .set_container(&session.media, OutputContainer::Mkv);
        let custom_output = directory.join("custom-mix.mkv");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.handle_audio_mix_action(AudioMixSelectionAction::Back)
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
        assert_eq!(session.selection.container(), OutputContainer::Mkv);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
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

        app.handle_upscale_action(UpscaleSelectionAction::Back)
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
        app.select_tool(Tool::VideoUpscale);
        let old_output = directory.join("old-custom.mp4");
        app.state
            .upscale
            .as_mut()
            .expect("upscale session should exist")
            .selection
            .set_custom_output_path(old_output);
        app.handle_upscale_action(UpscaleSelectionAction::Back)
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
