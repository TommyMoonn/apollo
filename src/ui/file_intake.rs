use std::cell::Cell;

use egui::{CornerRadius, RichText, Stroke, StrokeKind, TextStyle, Ui};

use crate::media::error::MediaError;

use super::{fonts, layout, media_error, theme, widgets};

const WORDMARK: &str = "apollo";
const OPEN_LABEL: &str = "Open files";
const DROP_HINT: &str = "or drop a video anywhere";
const INSPECTING_LABEL: &str = "Inspecting media…";
const AUDIO_MIX_LABEL: &str = "Audio Mix";
const UPSCALE_LABEL: &str = "Upscale";
const REMUX_LABEL: &str = "Remux";
const AUDIO_EXTRACT_LABEL: &str = "Audio Extract";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIntakeTool {
    AudioMix,
    Upscale,
    Remux,
    AudioExtract,
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
        ui.max_rect().shrink(theme::SPACING.compact),
        CornerRadius::same(theme::CONTAINER_RADIUS),
        Stroke::new(theme::SEPARATOR_WIDTH, theme::BORDER_STRONG),
        StrokeKind::Inside,
    );
}

pub fn show(
    ui: &mut Ui,
    selected_tool: FileIntakeTool,
    inspecting: bool,
    error: Option<&MediaError>,
    error_details_open: &mut bool,
) -> FileIntakeAction {
    show_with_layout_plan(ui, selected_tool, inspecting, error, error_details_open).0
}

fn show_with_layout_plan(
    ui: &mut Ui,
    selected_tool: FileIntakeTool,
    inspecting: bool,
    error: Option<&MediaError>,
    error_details_open: &mut bool,
) -> (FileIntakeAction, layout::IntakeLayoutPlan) {
    if error.is_none() {
        *error_details_open = false;
    }

    let action = Cell::new(FileIntakeAction::None);
    let content_width = layout::bounded_content_width(ui.available_width());
    let region_heights =
        intake_region_heights(ui, content_width, inspecting, error, *error_details_open);
    let plan = layout::intake_content(
        ui,
        region_heights,
        show_wordmark,
        |ui| {
            let next = show_intake_area(ui, inspecting, error, error_details_open);
            if next != FileIntakeAction::None {
                action.set(next);
            }
        },
        |ui| {
            let next = show_mode_selector(ui, selected_tool);
            if next != FileIntakeAction::None {
                action.set(next);
            }
        },
    );

    (action.get(), plan)
}

fn intake_region_heights(
    ui: &Ui,
    content_width: f32,
    inspecting: bool,
    error: Option<&MediaError>,
    error_details_open: bool,
) -> [f32; 3] {
    let top = ui.text_style_height(&fonts::wordmark_text_style());
    let bottom = theme::COMPACT_CONTROL_HEIGHT;
    let middle = if inspecting {
        ui.text_style_height(&TextStyle::Body)
    } else {
        let mut height = theme::PRIMARY_CONTROL_HEIGHT
            + theme::SPACING.compact
            + ui.text_style_height(&TextStyle::Small);

        if let Some(error) = error {
            height += theme::SPACING.compact
                + media_error::inline_height(ui, error, error_details_open, content_width);
        }
        height
    };

    [top, middle, bottom]
}

fn show_wordmark(ui: &mut Ui) {
    ui.label(
        RichText::new(WORDMARK)
            .text_style(fonts::wordmark_text_style())
            .extra_letter_spacing(fonts::WORDMARK_LETTER_SPACING),
    );
}

fn show_intake_area(
    ui: &mut Ui,
    inspecting: bool,
    error: Option<&MediaError>,
    error_details_open: &mut bool,
) -> FileIntakeAction {
    let mut action = FileIntakeAction::None;

    if inspecting {
        ui.label(
            RichText::new(INSPECTING_LABEL)
                .text_style(TextStyle::Body)
                .color(theme::SECONDARY),
        );
        return action;
    }

    if widgets::secondary_button(ui, OPEN_LABEL).clicked() {
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
    }

    action
}

fn show_mode_selector(ui: &mut Ui, selected_tool: FileIntakeTool) -> FileIntakeAction {
    let sizes = [
        widgets::mode_button_intrinsic_size(ui, AUDIO_MIX_LABEL),
        widgets::mode_button_intrinsic_size(ui, UPSCALE_LABEL),
        widgets::mode_button_intrinsic_size(ui, REMUX_LABEL),
        widgets::mode_button_intrinsic_size(ui, AUDIO_EXTRACT_LABEL),
    ];
    let widths = sizes.map(|size| size.x);
    let arrangement = layout::action_layout(ui.available_width(), &widths);
    let mut action = FileIntakeAction::None;

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
        if widgets::mode_button(ui, REMUX_LABEL, selected_tool == FileIntakeTool::Remux).clicked() {
            action = FileIntakeAction::SelectTool(FileIntakeTool::Remux);
        }
        if widgets::mode_button(
            ui,
            AUDIO_EXTRACT_LABEL,
            selected_tool == FileIntakeTool::AudioExtract,
        )
        .clicked()
        {
            action = FileIntakeAction::SelectTool(FileIntakeTool::AudioExtract);
        }
    });

    action
}

#[cfg(test)]
mod tests {
    use crate::{
        media::{error::MediaError, tools::MediaToolsError},
        ui::{
            fonts,
            layout::{self, IntakeLayout},
            theme,
        },
    };

    use super::{
        AUDIO_EXTRACT_LABEL, AUDIO_MIX_LABEL, DROP_HINT, FileIntakeTool, INSPECTING_LABEL,
        OPEN_LABEL, REMUX_LABEL, UPSCALE_LABEL, WORDMARK, intake_region_heights,
        show_with_layout_plan,
    };

    #[test]
    fn intake_copy_matches_quiet_focus_surface() {
        assert_eq!(WORDMARK, "apollo");
        assert_eq!(OPEN_LABEL, "Open files");
        assert_eq!(DROP_HINT, "or drop a video anywhere");
        assert_eq!(INSPECTING_LABEL, "Inspecting media…");
        assert_eq!(AUDIO_MIX_LABEL, "Audio Mix");
        assert_eq!(UPSCALE_LABEL, "Upscale");
        assert_eq!(REMUX_LABEL, "Remux");
        assert_eq!(AUDIO_EXTRACT_LABEL, "Audio Extract");
    }

    #[test]
    fn wrapped_error_changes_real_intake_layout_with_width() {
        let error = MediaError::from_tools_error(&MediaToolsError::MissingBoth);
        let wide_width = layout::MAX_CONTENT_WIDTH;
        let narrow_width =
            layout::MINIMUM_CONTENT_SIZE.x - f32::from(layout::CONTENT_PADDING) * 2.0;
        let wide_heights = region_heights(wide_width, Some(&error), false);
        let narrow_heights = region_heights(narrow_width, Some(&error), false);

        assert!(narrow_heights[1] > wide_heights[1]);
        let available_height =
            (required_height(wide_heights) + required_height(narrow_heights)) * 0.5;

        let wide = render_intake(wide_width, available_height, Some(&error), false);
        let narrow = render_intake(narrow_width, available_height, Some(&error), false);

        assert_eq!(wide, IntakeLayout::Distributed);
        assert_eq!(narrow, IntakeLayout::Compact);
    }

    #[test]
    fn expanded_diagnostics_participate_in_real_intake_layout_decision() {
        let error = MediaError::from_tools_error(&MediaToolsError::MissingBoth);
        let width = layout::MINIMUM_CONTENT_SIZE.x - f32::from(layout::CONTENT_PADDING) * 2.0;
        let collapsed_heights = region_heights(width, Some(&error), false);
        let expanded_heights = region_heights(width, Some(&error), true);

        assert!(expanded_heights[1] > collapsed_heights[1]);
        let available_height =
            (required_height(collapsed_heights) + required_height(expanded_heights)) * 0.5;

        let collapsed = render_intake(width, available_height, Some(&error), false);
        let expanded = render_intake(width, available_height, Some(&error), true);

        assert_eq!(collapsed, IntakeLayout::Distributed);
        assert_eq!(expanded, IntakeLayout::Compact);
    }

    #[test]
    fn short_intake_remains_distributed_when_it_fits() {
        let width = layout::MINIMUM_CONTENT_SIZE.x - f32::from(layout::CONTENT_PADDING) * 2.0;
        let heights = region_heights(width, None, false);
        let available_height = required_height(heights) + 1.0;

        assert_eq!(
            render_intake(width, available_height, None, false),
            IntakeLayout::Distributed
        );
    }

    fn required_height(region_heights: [f32; 3]) -> f32 {
        region_heights.into_iter().sum::<f32>() + theme::SPACING.section * 2.0
    }

    fn region_heights(width: f32, error: Option<&MediaError>, details_open: bool) -> [f32; 3] {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut result = [0.0; 3];
        let output = context.run_ui(Default::default(), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 500.0), egui::Sense::hover());
            let child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            let content_width = super::layout::bounded_content_width(child.available_width());
            result = intake_region_heights(&child, content_width, false, error, details_open);
        });
        output.drop_without_applying_deltas();
        result
    }

    fn render_intake(
        width: f32,
        height: f32,
        error: Option<&MediaError>,
        details_open: bool,
    ) -> IntakeLayout {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut layout = None;
        let mut details_open = details_open;
        let output = context.run_ui(Default::default(), |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
            layout = Some(
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::Center)),
                    |ui| {
                        show_with_layout_plan(
                            ui,
                            FileIntakeTool::AudioMix,
                            false,
                            error,
                            &mut details_open,
                        )
                        .1
                        .layout
                    },
                )
                .inner,
            );
        });
        output.drop_without_applying_deltas();
        layout.expect("intake layout should render")
    }
}
