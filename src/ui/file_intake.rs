use egui::{RichText, Stroke, StrokeKind, TextStyle, Ui};

use crate::media::error::MediaError;

use super::{fonts, layout, media_error, theme, widgets};

const WORDMARK: &str = "apollo";
const OPEN_LABEL: &str = "Open file";
const DROP_HINT: &str = "or drop a video anywhere";
const AUDIO_MIX_LABEL: &str = "Audio Mix";
const UPSCALE_LABEL: &str = "Upscale";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIntakeTool {
    AudioMix,
    Upscale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileIntakeAction {
    #[default]
    None,
    OpenFile,
    SelectTool(FileIntakeTool),
}

pub fn show_drag_hover(ui: &Ui) {
    ui.painter().rect_stroke(
        ui.max_rect().shrink(8.0),
        0.0,
        Stroke::new(1.0, theme::BORDER_STRONG),
        StrokeKind::Inside,
    );
}

pub fn show(
    ui: &mut Ui,
    selected_tool: FileIntakeTool,
    error: Option<&MediaError>,
    error_details_open: &mut bool,
) -> FileIntakeAction {
    let mut action = FileIntakeAction::None;
    layout::centered_content(ui, |ui| {
        ui.label(
            RichText::new(WORDMARK)
                .text_style(fonts::wordmark_text_style())
                .extra_letter_spacing(fonts::WORDMARK_LETTER_SPACING),
        );
        ui.add_space(theme::SPACING.focal);

        if widgets::primary_button(ui, OPEN_LABEL).clicked() {
            action = FileIntakeAction::OpenFile;
        }
        ui.add_space(theme::SPACING.compact);
        ui.label(
            RichText::new(DROP_HINT)
                .text_style(TextStyle::Small)
                .color(theme::SECONDARY_FAINT),
        );

        if let Some(error) = error {
            ui.add_space(theme::SPACING.compact);
            media_error::show_inline(ui, error, error_details_open);
        } else {
            *error_details_open = false;
        }

        ui.add_space(theme::SPACING.focal);
        let sizes = [
            widgets::mode_button_intrinsic_size(ui, AUDIO_MIX_LABEL),
            widgets::mode_button_intrinsic_size(ui, UPSCALE_LABEL),
        ];
        let widths = sizes.map(|size| size.x);
        let arrangement = layout::action_layout(ui.available_width(), &widths);
        layout::centered_action_group(ui, &sizes, arrangement, theme::SPACING.compact, |ui| {
            if widgets::mode_button(
                ui,
                AUDIO_MIX_LABEL,
                selected_tool == FileIntakeTool::AudioMix,
            )
            .clicked()
            {
                action = FileIntakeAction::SelectTool(FileIntakeTool::AudioMix);
            }
            if widgets::mode_button(ui, UPSCALE_LABEL, selected_tool == FileIntakeTool::Upscale)
                .clicked()
            {
                action = FileIntakeAction::SelectTool(FileIntakeTool::Upscale);
            }
        });
    });

    action
}

#[cfg(test)]
mod tests {
    use super::{AUDIO_MIX_LABEL, DROP_HINT, OPEN_LABEL, UPSCALE_LABEL, WORDMARK};

    #[test]
    fn intake_copy_matches_quiet_focus_surface() {
        assert_eq!(WORDMARK, "apollo");
        assert_eq!(OPEN_LABEL, "Open file");
        assert_eq!(DROP_HINT, "or drop a video anywhere");
        assert_eq!(AUDIO_MIX_LABEL, "Audio Mix");
        assert_eq!(UPSCALE_LABEL, "Upscale");
    }
}
