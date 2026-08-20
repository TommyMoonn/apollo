use std::time::Duration;

use egui::{CentralPanel, RichText, TextStyle, Ui};

use apollo::{
    media::{
        model::MediaFile,
        tools::{MediaTools, MediaToolsError, discover_media_tools},
    },
    ui::{
        fonts, layout, theme, upscale as upscale_ui,
        upscale::{UpscaleSelectionAction, UpscaleSelectionState},
        widgets,
    },
    upscale::execution::{UpscaleExecutor, UpscaleStartError},
};

const HOME_WORDMARK: &str = "apollo";
const HOME_PROMPT: &str = "Choose a media operation";
const AUDIO_MIX_LABEL: &str = "Audio Mix";
const UPSCALE_LABEL: &str = "Upscale";
const EXECUTION_REPAINT_INTERVAL: Duration = Duration::from_millis(50);

fn home_action_layout(available_width: f32, action_widths: [f32; 2]) -> layout::ActionLayout {
    layout::action_layout(available_width, &action_widths)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    AudioMix,
    VideoUpscale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpscaleToolState {
    media: MediaFile,
    selection: UpscaleSelectionState,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct AppState {
    current_tool: Option<Tool>,
    upscale: Option<UpscaleToolState>,
}

impl AppState {
    fn select_tool(&mut self, tool: Tool) {
        self.current_tool = Some(tool);
    }

    fn open_upscale(&mut self) {
        self.select_tool(Tool::VideoUpscale);
    }

    fn show_home(&mut self) {
        self.current_tool = None;
    }

    fn change_upscale_source(&mut self) {
        self.upscale = None;
        self.show_home();
    }
}

#[derive(Debug)]
pub(crate) struct ApolloApp {
    state: AppState,
    media_tools: Result<MediaTools, MediaToolsError>,
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
            upscale_executor: UpscaleExecutor::new(),
        })
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
            |ui| self.show_home_tool_buttons(ui),
        );
    }

    fn show_home_tool_buttons(&mut self, ui: &mut Ui) {
        if widgets::mode_button(ui, AUDIO_MIX_LABEL, false).clicked() {
            self.state.select_tool(Tool::AudioMix);
        }

        if widgets::mode_button(ui, UPSCALE_LABEL, false).clicked() {
            self.state.open_upscale();
        }
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
        match tool {
            Tool::AudioMix => self.show_tool_placeholder(ui, "Audio Mix"),
            Tool::VideoUpscale => self.show_upscale_tool(ui),
        }
    }

    fn show_tool_placeholder(&mut self, ui: &mut Ui, heading: &str) {
        layout::centered_content(ui, |ui| {
            ui.heading(heading);
            ui.add_space(theme::SPACING.compact);
            ui.label("Tool controls are added in a later phase.");
            ui.add_space(theme::SPACING.section);

            if widgets::secondary_button(ui, "Back").clicked() {
                self.state.show_home();
            }
        });
    }

    fn show_upscale_tool(&mut self, ui: &mut Ui) {
        self.poll_upscale_execution(ui);

        let Some(upscale) = self.state.upscale.as_mut() else {
            self.show_tool_placeholder(ui, "Video Upscale");
            return;
        };

        let action = upscale_ui::show(ui, &upscale.media, &mut upscale.selection);
        if let Err(error) = self.handle_upscale_action(action) {
            eprintln!("Upscale could not start: {error}");
        }
    }

    fn poll_upscale_execution(&mut self, ui: &Ui) {
        self.upscale_executor.poll();
        if self.upscale_executor.is_running() {
            ui.ctx().request_repaint_after(EXECUTION_REPAINT_INTERVAL);
        }
    }

    fn handle_upscale_action(
        &mut self,
        action: UpscaleSelectionAction,
    ) -> Result<(), UpscaleStartError> {
        match action {
            UpscaleSelectionAction::ChangeSource if !self.upscale_executor.is_running() => {
                self.state.change_upscale_source();
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

                match self.state.current_tool {
                    Some(tool) => self.show_tool(ui, tool),
                    None => self.show_home(ui),
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        AUDIO_MIX_LABEL, ApolloApp, AppState, HOME_PROMPT, HOME_WORDMARK, Tool, UPSCALE_LABEL,
        UpscaleToolState, home_action_layout,
    };
    use apollo::{
        media::{
            model::{FrameRate, MediaFile, VideoStream},
            tools::MediaTools,
        },
        ui::{
            fonts,
            layout::{self, ActionLayout},
            theme,
            upscale::UpscaleSelectionAction,
            widgets,
        },
        upscale::model::{UpscaleTarget, VideoCodec},
    };

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("recording.mkv"),
            duration: None,
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: FrameRate::new(60, 1),
            }],
            audio_streams: Vec::new(),
        }
    }

    fn app() -> ApolloApp {
        ApolloApp {
            state: AppState::default(),
            media_tools: Ok(MediaTools {
                ffmpeg: std::env::temp_dir().join("apollo-phase33-missing-ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            }),
            upscale_executor: apollo::upscale::execution::UpscaleExecutor::new(),
        }
    }

    fn open_ready_upscale(app: &mut ApolloApp) {
        let media = media();
        app.state.upscale = Some(UpscaleToolState {
            selection: apollo::ui::upscale::UpscaleSelectionState::new(&media),
            media,
        });
        app.state.open_upscale();
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

    #[test]
    fn default_application_state_is_tool_chooser() {
        let state = AppState::default();

        assert!(state.current_tool.is_none());
    }

    #[test]
    fn tool_transitions_are_explicit_and_reversible() {
        let mut state = AppState::default();

        state.select_tool(Tool::AudioMix);
        assert_eq!(state.current_tool, Some(Tool::AudioMix));

        state.show_home();
        assert!(state.current_tool.is_none());

        state.open_upscale();
        assert_eq!(state.current_tool, Some(Tool::VideoUpscale));
    }

    #[test]
    fn opening_upscale_source_routes_to_phase_32_ready_surface() {
        let mut app = app();
        open_ready_upscale(&mut app);
        let context = egui::Context::default();
        fonts::install(&context).expect("test typography should install");
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::VideoUpscale);
        });

        assert_eq!(app.state.current_tool, Some(Tool::VideoUpscale));
        assert!(app.state.upscale.is_some());
        assert!(output_contains_text(&output, "Upscale to"));
        assert!(output_contains_text(&output, "Video codec"));
        assert!(output_contains_text(&output, "Actual output"));
        output.textures_delta.clear();
    }

    #[test]
    fn upscale_ready_selection_survives_home_and_tool_navigation() {
        let mut app = app();
        open_ready_upscale(&mut app);
        let ready = app
            .state
            .upscale
            .as_mut()
            .expect("upscale source should exist");
        ready
            .selection
            .set_target(&ready.media, UpscaleTarget::Uhd8k);
        ready.selection.set_codec(VideoCodec::H265);

        app.state.show_home();
        app.state.select_tool(Tool::VideoUpscale);

        let ready = app
            .state
            .upscale
            .as_ref()
            .expect("upscale source should persist");
        assert_eq!(ready.selection.target(), UpscaleTarget::Uhd8k);
        assert_eq!(ready.selection.codec(), VideoCodec::H265);
    }

    #[test]
    fn change_source_returns_to_existing_home_boundary_and_clears_ready_state() {
        let mut app = app();
        open_ready_upscale(&mut app);

        app.handle_upscale_action(UpscaleSelectionAction::ChangeSource)
            .unwrap();

        assert!(app.state.current_tool.is_none());
        assert!(app.state.upscale.is_none());
    }

    #[test]
    fn browse_intent_does_not_replace_ready_state() {
        let mut app = app();
        open_ready_upscale(&mut app);
        let before = app.state.clone();

        app.handle_upscale_action(UpscaleSelectionAction::BrowseOutput)
            .unwrap();

        assert_eq!(app.state, before);
        assert_eq!(
            app.upscale_executor.state(),
            &apollo::upscale::execution::UpscaleExecutionState::Idle
        );
    }

    #[test]
    fn upscale_intent_reaches_phase_33_executor() {
        let mut app = app();
        open_ready_upscale(&mut app);

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
        assert_eq!(
            app.upscale_executor.state(),
            &apollo::upscale::execution::UpscaleExecutionState::Idle
        );
    }

    #[test]
    fn initial_state_is_deterministic() {
        assert_eq!(AppState::default(), AppState::default());
    }

    #[test]
    fn initialization_does_not_select_or_start_a_media_tool() {
        let state = AppState::default();

        assert!(state.current_tool.is_none());
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
            assert_eq!(theme::SPACING.compact, 7.0);
        });
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
