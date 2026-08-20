use std::{io, path::Path, process::Command, time::Duration};

use egui::{CentralPanel, Context, RichText, TextStyle, Ui};

use apollo::{
    audio_mix::execution::{AudioMixExecutionState, AudioMixExecutor, AudioMixStartError},
    media::{
        intake::{MediaIntake, MediaLoadDisposition},
        model::MediaFile,
        probe::{MediaProbeError, probe_media},
        tools::{MediaTools, MediaToolsError, discover_media_tools},
    },
    ui::{
        audio_mix as audio_mix_ui,
        audio_mix::{AudioMixSelectionAction, AudioMixSelectionState},
        audio_mix_progress::{self, AudioMixLifecycleAction, AudioMixLifecycleUiState},
        file_intake::{self, FileIntakeAction, FileIntakeTool},
        fonts, layout, theme, upscale as upscale_ui,
        upscale::{UpscaleSelectionAction, UpscaleSelectionState},
        upscale_progress::{self, UpscaleLifecycleAction, UpscaleLifecycleUiState},
        widgets,
    },
    upscale::execution::{UpscaleExecutionState, UpscaleExecutor, UpscaleStartError},
};

const HOME_WORDMARK: &str = "apollo";
const HOME_PROMPT: &str = "Choose a media operation";
const AUDIO_MIX_LABEL: &str = "Audio Mix";
const UPSCALE_LABEL: &str = "Upscale";
const EXECUTION_REPAINT_INTERVAL: Duration = Duration::from_millis(50);

fn home_action_layout(available_width: f32, action_widths: [f32; 2]) -> layout::ActionLayout {
    layout::action_layout(available_width, &action_widths)
}

fn file_intake_enabled(
    current_tool: Option<Tool>,
    audio_state: &AudioMixExecutionState,
    upscale_state: &UpscaleExecutionState,
) -> bool {
    // Done/Back owns terminal settlement. Intake resumes only after both executors return to Idle.
    current_tool.is_some()
        && matches!(audio_state, AudioMixExecutionState::Idle)
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpscaleToolState {
    media: MediaFile,
    selection: UpscaleSelectionState,
    lifecycle_ui: UpscaleLifecycleUiState,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct AppState {
    current_tool: Option<Tool>,
    intake_active: bool,
    intake_error: Option<String>,
    media_intake: MediaIntake,
    audio_mix: Option<AudioMixToolState>,
    upscale: Option<UpscaleToolState>,
}

impl AppState {
    fn select_tool(&mut self, tool: Tool) {
        self.current_tool = Some(tool);
    }

    fn begin_source_change(&mut self, tool: Tool) {
        self.current_tool = Some(tool);
        self.intake_active = true;
        self.intake_error = None;
        match tool {
            Tool::AudioMix => self.audio_mix = None,
            Tool::VideoUpscale => self.upscale = None,
        }
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
}

impl ApolloApp {
    pub(crate) fn new(
        creation_context: &eframe::CreationContext<'_>,
    ) -> Result<Self, fonts::FontSetupError> {
        fonts::install(&creation_context.egui_ctx)?;
        theme::install(&creation_context.egui_ctx);

        Ok(Self {
            state: AppState::default(),
            media_tools: discover_media_tools(),
            audio_mix_executor: AudioMixExecutor::new(),
            upscale_executor: UpscaleExecutor::new(),
        })
    }

    fn file_intake_enabled(&self) -> bool {
        file_intake_enabled(
            self.state.current_tool,
            self.audio_mix_executor.state(),
            self.upscale_executor.state(),
        )
    }

    fn show_home(&mut self, ui: &mut Ui) {
        layout::centered_content(ui, |ui| {
            ui.label(
                RichText::new(HOME_WORDMARK)
                    .text_style(fonts::wordmark_text_style())
                    .extra_letter_spacing(fonts::WORDMARK_LETTER_SPACING),
            );
            ui.add_space(theme::SPACING.focal);
            ui.label(
                RichText::new(HOME_PROMPT)
                    .text_style(TextStyle::Small)
                    .color(theme::SECONDARY),
            );
            ui.add_space(theme::SPACING.focal);
            self.show_home_actions(ui);
        });
    }

    fn show_home_actions(&mut self, ui: &mut Ui) {
        let action_sizes = [
            widgets::mode_button_intrinsic_size(ui, AUDIO_MIX_LABEL),
            widgets::mode_button_intrinsic_size(ui, UPSCALE_LABEL),
        ];
        let action_widths = action_sizes.map(|size| size.x);
        let arrangement = home_action_layout(ui.available_width(), action_widths);

        layout::centered_action_group(
            ui,
            &action_sizes,
            arrangement,
            theme::SPACING.compact,
            |ui| {
                if widgets::mode_button(ui, AUDIO_MIX_LABEL, false).clicked() {
                    self.select_tool(Tool::AudioMix);
                }
                if widgets::mode_button(ui, UPSCALE_LABEL, false).clicked() {
                    self.select_tool(Tool::VideoUpscale);
                }
            },
        );
    }

    fn select_tool(&mut self, tool: Tool) {
        self.state.select_tool(tool);
        self.state.intake_error = None;
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
                        media,
                    });
                }
            }
        }
        self.state.intake_active = false;
        true
    }

    fn show_media_tools_unavailable(&mut self, ui: &mut Ui) {
        let message = self
            .media_tools
            .as_ref()
            .err()
            .map(MediaToolsError::user_message)
            .unwrap_or("FFmpeg and FFprobe could not be validated.");

        layout::centered_content(ui, |ui| {
            ui.heading("FFmpeg is required");
            ui.add_space(theme::SPACING.compact);
            ui.label(message);
            ui.add_space(theme::SPACING.section);
            if widgets::secondary_button(ui, "Retry").clicked() {
                self.media_tools = discover_media_tools();
            }
        });
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
        let action = file_intake::show(ui, tool.intake_tool(), self.state.intake_error.as_deref());
        match action {
            FileIntakeAction::OpenFile => match open_media_file_dialog() {
                Ok(Some(path)) => self.load_media_path(&path),
                Ok(None) => {}
                Err(error) => self.state.intake_error = Some(error.to_string()),
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
        self.load_media_path_with_probe(path, |path| probe_media(&tools, path));
    }

    fn load_media_path_with_probe<F>(&mut self, path: &Path, probe: F)
    where
        F: FnOnce(&Path) -> Result<MediaFile, MediaProbeError>,
    {
        if !self.file_intake_enabled() {
            return;
        }

        self.state.retire_tool_sessions();
        self.state.intake_active = true;
        self.state.intake_error = None;
        match self.state.media_intake.load_with_probe(path, probe) {
            Ok(MediaLoadDisposition::Probed | MediaLoadDisposition::Reused) => {
                let tool = self
                    .state
                    .current_tool
                    .expect("selected tool exists while accepting media");
                let _ = self.activate_cached_source(tool);
            }
            Err(error) => self.state.intake_error = Some(error.to_string()),
        }
    }

    fn handle_window_file_drop(&mut self, context: &Context) {
        if !self.file_intake_enabled() {
            return;
        }
        let dropped = context.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        });
        if let Some(path) = dropped {
            self.load_media_path(&path);
        }
    }

    fn drag_hovered(&self, context: &Context) -> bool {
        self.file_intake_enabled() && context.input(|input| !input.raw.hovered_files.is_empty())
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

        let session = self
            .state
            .audio_mix
            .as_mut()
            .expect("audio mix source exists while ready surface is shown");
        let action = audio_mix_ui::show(ui, &session.media, &mut session.selection);
        if let Err(error) = self.handle_audio_mix_action(action) {
            eprintln!("Audio Mix could not start: {error}");
        }
    }

    fn handle_audio_mix_action(
        &mut self,
        action: AudioMixSelectionAction,
    ) -> Result<(), AudioMixStartError> {
        match action {
            AudioMixSelectionAction::ChangeSource if self.file_intake_enabled() => {
                self.state.begin_source_change(Tool::AudioMix);
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
            AudioMixSelectionAction::None
            | AudioMixSelectionAction::BrowseOutput
            | AudioMixSelectionAction::ChangeSource => {}
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

        let upscale = self
            .state
            .upscale
            .as_mut()
            .expect("upscale source exists while ready surface is shown");
        let action = upscale_ui::show(ui, &upscale.media, &mut upscale.selection);
        if let Err(error) = self.handle_upscale_action(action) {
            eprintln!("Upscale could not start: {error}");
        }
    }

    fn handle_upscale_action(
        &mut self,
        action: UpscaleSelectionAction,
    ) -> Result<(), UpscaleStartError> {
        match action {
            UpscaleSelectionAction::ChangeSource if self.file_intake_enabled() => {
                self.state.begin_source_change(Tool::VideoUpscale);
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
            UpscaleSelectionAction::None
            | UpscaleSelectionAction::BrowseOutput
            | UpscaleSelectionAction::ChangeSource => {}
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

                self.handle_window_file_drop(ui.ctx());
                if self.drag_hovered(ui.ctx()) {
                    file_intake::show_drag_hover(ui);
                }

                match self.state.current_tool {
                    Some(tool) => self.show_tool(ui, tool),
                    None => self.show_home(ui),
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::{
        AUDIO_MIX_LABEL, ApolloApp, AppState, HOME_PROMPT, HOME_WORDMARK, Tool, UPSCALE_LABEL,
        file_intake_enabled, home_action_layout, shows_audio_mix_lifecycle,
        shows_upscale_lifecycle,
    };
    use apollo::{
        audio_mix::execution::AudioMixExecutionState,
        media::{
            execution::MediaExecutionFailure,
            model::{AudioStream, FrameRate, MediaFile, VideoStream},
            process::MediaProcessFailure,
            tools::MediaTools,
        },
        ui::{
            fonts,
            layout::{self, ActionLayout},
            theme,
            upscale::UpscaleSelectionAction,
            widgets,
        },
        upscale::{
            execution::UpscaleExecutionState,
            model::{UpscaleTarget, VideoCodec},
        },
    };

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

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
        }
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
    fn default_application_state_is_tool_chooser() {
        assert!(AppState::default().current_tool.is_none());
    }

    #[test]
    fn initial_state_is_deterministic() {
        assert_eq!(AppState::default(), AppState::default());
    }

    #[test]
    fn initialization_does_not_select_or_start_a_media_tool() {
        let app = app();
        assert!(app.state.current_tool.is_none());
        assert!(!app.audio_mix_executor.is_running());
        assert!(!app.upscale_executor.is_running());
    }

    #[test]
    fn home_uses_quiet_focus_copy_without_card_descriptions() {
        assert_eq!(HOME_WORDMARK, "apollo");
        assert_eq!(HOME_PROMPT, "Choose a media operation");
        assert_eq!(AUDIO_MIX_LABEL, "Audio Mix");
        assert_eq!(UPSCALE_LABEL, "Upscale");
    }

    #[test]
    fn home_horizontal_group_uses_intrinsic_mode_sizes_and_shared_gap() {
        egui::__run_test_ui(|ui| {
            let action_sizes = [
                widgets::mode_button_intrinsic_size(ui, AUDIO_MIX_LABEL),
                widgets::mode_button_intrinsic_size(ui, UPSCALE_LABEL),
            ];
            let group_size = layout::action_group_size(
                &action_sizes,
                ActionLayout::Horizontal,
                theme::SPACING.compact,
            );

            assert_eq!(
                group_size.x,
                action_sizes[0].x + theme::SPACING.compact + action_sizes[1].x
            );
            assert_eq!(group_size.y, theme::PRIMARY_CONTROL_HEIGHT);
        });
    }

    #[test]
    fn selecting_each_tool_enters_unified_intake_without_a_source() {
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        assert_eq!(app.state.current_tool, Some(Tool::AudioMix));
        assert!(app.state.intake_active);
        app.state.current_tool = None;
        app.select_tool(Tool::VideoUpscale);
        assert_eq!(app.state.current_tool, Some(Tool::VideoUpscale));
        assert!(app.state.intake_active);
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
            app.state.current_tool,
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
    fn opening_upscale_source_renders_ready_surface() {
        let (directory, path) = temp_file("upscale-ready-render");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context).expect("test typography should install");
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::VideoUpscale);
        });

        assert!(output_contains_text(&output, "Upscale to"));
        assert!(output_contains_text(&output, "Video codec"));
        assert!(output_contains_text(&output, "Actual output"));
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
        fonts::install(&context).expect("test typography should install");
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioMix);
        });

        assert!(output_contains_text(&output, "Audio tracks"));
        assert!(output_contains_text(&output, "Mix selected tracks"));
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

        app.state.current_tool = None;
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
    fn browse_intent_does_not_replace_upscale_ready_state() {
        let (directory, path) = temp_file("browse-preserve");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let before = app.state.clone();

        app.handle_upscale_action(UpscaleSelectionAction::BrowseOutput)
            .unwrap();

        assert_eq!(app.state, before);
        assert_eq!(app.upscale_executor.state(), &UpscaleExecutionState::Idle);
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
            Err(apollo::media::probe::MediaProbeError::FfprobeFailed(Some(
                1,
            )))
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
        app.state.begin_source_change(Tool::AudioMix);

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
    fn change_source_returns_to_intake_and_keeps_cache_for_tool_reuse() {
        let (directory, path) = temp_file("change");
        let mut app = app();
        app.select_tool(Tool::VideoUpscale);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.handle_upscale_action(UpscaleSelectionAction::ChangeSource)
            .unwrap();

        assert!(app.state.intake_active);
        assert!(app.state.upscale.is_none());
        assert_eq!(
            app.state.media_intake.current().map(|m| &m.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changed_cached_file_is_not_reused_when_switching_tools() {
        let (directory, path) = temp_file("changed-switch");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.state.begin_source_change(Tool::AudioMix);
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
            Some(Tool::AudioMix),
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            Some(Tool::AudioMix),
            &AudioMixExecutionState::Running,
            &UpscaleExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            Some(Tool::VideoUpscale),
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Running,
        ));
        assert!(!file_intake_enabled(
            None,
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
        ));
    }

    #[test]
    fn audio_mix_terminal_states_reject_source_replacement() {
        for state in terminal_execution_states() {
            assert!(!file_intake_enabled(
                Some(Tool::AudioMix),
                &state,
                &UpscaleExecutionState::Idle,
            ));
        }
    }

    #[test]
    fn upscale_terminal_states_reject_source_replacement() {
        for state in terminal_execution_states() {
            assert!(!file_intake_enabled(
                Some(Tool::VideoUpscale),
                &AudioMixExecutionState::Idle,
                &state,
            ));
        }
    }

    #[test]
    fn terminal_reset_to_idle_reenables_file_intake() {
        let completed = AudioMixExecutionState::Completed {
            output: PathBuf::from("finished.mp4"),
        };
        assert!(!file_intake_enabled(
            Some(Tool::AudioMix),
            &completed,
            &UpscaleExecutionState::Idle,
        ));

        assert!(file_intake_enabled(
            Some(Tool::AudioMix),
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
        ));
    }

    #[test]
    fn narrow_home_layout_stacks_tool_actions() {
        assert_eq!(
            home_action_layout(150.0, [80.0, 70.0]),
            ActionLayout::Stacked
        );
    }

    #[test]
    fn wider_home_layout_keeps_tool_actions_horizontal() {
        assert_eq!(
            home_action_layout(161.0, [80.0, 70.0]),
            ActionLayout::Horizontal
        );
    }
}
