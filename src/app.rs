use egui::{Button, CentralPanel, RichText, Ui};

use apollo::media::tools::{MediaTools, MediaToolsError, discover_media_tools};

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
    pub(crate) fn new(_creation_context: &eframe::CreationContext<'_>) -> Self {
        Self {
            state: AppState::default(),
            media_tools: discover_media_tools(),
        }
    }

    fn show_home(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(18.0);
            ui.heading("Apollo");
            ui.add_space(18.0);

            if tool_button(ui, "Audio Mix", "Mix selected audio tracks").clicked() {
                self.state.select_tool(Tool::AudioMix);
            }

            ui.add_space(8.0);

            if tool_button(ui, "Video Upscale", "Resize video to a larger output").clicked() {
                self.state.select_tool(Tool::VideoUpscale);
            }
        });
    }

    fn show_media_tools_unavailable(&mut self, ui: &mut Ui) {
        let message = self
            .media_tools
            .as_ref()
            .err()
            .map(MediaToolsError::user_message)
            .unwrap_or("FFmpeg and FFprobe could not be validated.");

        ui.vertical_centered(|ui| {
            ui.add_space(32.0);
            ui.heading("FFmpeg is required");
            ui.add_space(8.0);
            ui.label(message);
            ui.add_space(18.0);

            if ui.button("Retry").clicked() {
                self.media_tools = discover_media_tools();
            }
        });
    }

    fn show_tool(&mut self, ui: &mut Ui, tool: Tool) {
        ui.vertical_centered(|ui| {
            ui.add_space(18.0);
            ui.heading(match tool {
                Tool::AudioMix => "Audio Mix",
                Tool::VideoUpscale => "Video Upscale",
            });
            ui.add_space(8.0);
            ui.label("Tool controls are added in a later phase.");
            ui.add_space(18.0);

            if ui.button("Back").clicked() {
                self.state.show_home();
            }
        });
    }
}

impl eframe::App for ApolloApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        CentralPanel::default().show(ui, |ui| {
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

fn tool_button(ui: &mut Ui, title: &str, description: &str) -> egui::Response {
    ui.add_sized(
        [360.0, 72.0],
        Button::new(RichText::new(format!("{title}\n{description}"))).wrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::{AppState, Tool};

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
}
