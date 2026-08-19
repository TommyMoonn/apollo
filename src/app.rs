use egui::{CentralPanel, RichText, TextStyle, Ui};

use apollo::{
    media::tools::{MediaTools, MediaToolsError, discover_media_tools},
    ui::{fonts, layout, theme, widgets},
};

const HOME_WORDMARK: &str = "apollo";
const HOME_PROMPT: &str = "Choose a media operation";
const AUDIO_MIX_LABEL: &str = "Audio Mix";
const UPSCALE_LABEL: &str = "Upscale";

fn home_action_layout(available_width: f32, action_widths: [f32; 2]) -> layout::ActionLayout {
    layout::action_layout(available_width, &action_widths)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    AudioMix,
    VideoUpscale,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct AppState {
    current_tool: Option<Tool>,
}

impl AppState {
    fn select_tool(&mut self, tool: Tool) {
        self.current_tool = Some(tool);
    }

    fn show_home(&mut self) {
        self.current_tool = None;
    }
}

#[derive(Debug)]
pub(crate) struct ApolloApp {
    state: AppState,
    media_tools: Result<MediaTools, MediaToolsError>,
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
            self.state.select_tool(Tool::VideoUpscale);
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
        layout::centered_content(ui, |ui| {
            ui.heading(match tool {
                Tool::AudioMix => "Audio Mix",
                Tool::VideoUpscale => "Video Upscale",
            });
            ui.add_space(theme::SPACING.compact);
            ui.label("Tool controls are added in a later phase.");
            ui.add_space(theme::SPACING.section);

            if widgets::secondary_button(ui, "Back").clicked() {
                self.state.show_home();
            }
        });
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
    use super::{
        AUDIO_MIX_LABEL, AppState, HOME_PROMPT, HOME_WORDMARK, Tool, UPSCALE_LABEL,
        home_action_layout,
    };
    use apollo::ui::{
        layout::{self, ActionLayout},
        theme, widgets,
    };

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

        state.select_tool(Tool::VideoUpscale);
        assert_eq!(state.current_tool, Some(Tool::VideoUpscale));
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
