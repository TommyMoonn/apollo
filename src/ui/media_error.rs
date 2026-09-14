use egui::{Response, Ui};

use crate::media::error::MediaError;

use super::{
    media_lifecycle::{
        diagnostic_height, diagnostic_viewport, heading, right_aligned_actions, secondary_copy,
        secondary_copy_height,
    },
    theme, widgets,
};

const SHOW_DETAILS_LABEL: &str = "Show details";
const HIDE_DETAILS_LABEL: &str = "Hide details";

pub fn show_inline(ui: &mut Ui, error: &MediaError, details_open: &mut bool) {
    ui.spacing_mut().item_spacing.y = 0.0;
    show_copy(ui, error);
    show_details(ui, error, details_open);
}

pub fn show_operation_action(ui: &mut Ui, error: &MediaError) {
    let id = ui.make_persistent_id(("operation-action-error", error.kind(), error.diagnostic()));
    let mut details_open = ui.data_mut(|data| data.get_temp::<bool>(id).unwrap_or(false));
    show_inline(ui, error, &mut details_open);
    ui.data_mut(|data| data.insert_temp(id, details_open));
}

pub(super) fn inline_height(
    ui: &Ui,
    error: &MediaError,
    details_open: bool,
    wrap_width: f32,
) -> f32 {
    let mut height = secondary_copy_height(ui, error.message(), wrap_width);

    if let Some(guidance) = error.guidance() {
        height += theme::SPACING.tight + secondary_copy_height(ui, guidance, wrap_width);
    }

    if error.diagnostic().is_some() {
        height += theme::DETAILS_TOP_GAP + theme::COMPACT_CONTROL_HEIGHT;
        if details_open {
            height += theme::DETAILS_TOP_GAP
                + theme::SEPARATOR_WIDTH
                + theme::DETAILS_SEPARATOR_GAP
                + diagnostic_height(ui, error.diagnostic().unwrap_or_default(), wrap_width);
        }
    }

    height
}

pub fn show_failure_content(
    ui: &mut Ui,
    title: &str,
    error: &MediaError,
    details_open: &mut bool,
    back_label: &str,
) -> Response {
    ui.spacing_mut().item_spacing.y = 0.0;
    heading(ui, title);
    ui.add_space(theme::STATUS_COPY_GAP);
    show_copy(ui, error);
    show_details(ui, error, details_open);

    ui.add_space(theme::RESULT_FOOTER_GAP);
    right_aligned_actions(ui, |ui| widgets::secondary_button(ui, back_label))
}

fn show_copy(ui: &mut Ui, error: &MediaError) {
    secondary_copy(ui, error.message());
    if let Some(guidance) = error.guidance() {
        ui.add_space(theme::SPACING.tight);
        secondary_copy(ui, guidance);
    }
}

fn show_details(ui: &mut Ui, error: &MediaError, details_open: &mut bool) {
    let Some(diagnostic) = error.diagnostic() else {
        *details_open = false;
        return;
    };

    ui.add_space(theme::DETAILS_TOP_GAP);
    let label = if *details_open {
        HIDE_DETAILS_LABEL
    } else {
        SHOW_DETAILS_LABEL
    };
    if widgets::plain_button(ui, label).clicked() {
        *details_open = !*details_open;
    }

    if *details_open {
        ui.add_space(theme::DETAILS_TOP_GAP);
        widgets::separator(ui);
        ui.add_space(theme::DETAILS_SEPARATOR_GAP);
        diagnostic_viewport(ui, diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        media::{
            error::{MediaError, MediaOutputContainer},
            execution::MediaExecutionFailure,
            process::MediaProcessFailure,
            tools::MediaToolsError,
        },
        ui::{fonts, media_lifecycle, theme},
    };

    use super::{inline_height, show_inline};

    #[test]
    fn inline_error_shows_concise_copy_without_raw_diagnostic_until_expanded() {
        let error = MediaError::from_execution_failure(
            &MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: "RAW-STDERR-MARKER".to_owned(),
            },
            MediaOutputContainer::Mp4,
        );
        let context = egui::Context::default();
        let mut details_open = false;
        let output = context.run_ui(Default::default(), |ui| {
            show_inline(ui, &error, &mut details_open);
        });
        let debug = format!("{:?}", output.shapes);

        assert!(!debug.contains("RAW-STDERR-MARKER"));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn inline_height_matches_wrapped_error_presentation_at_current_width() {
        let error = MediaError::from_tools_error(&MediaToolsError::MissingBoth);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        for (width, details_open) in [(344.0, false), (520.0, false), (344.0, true)] {
            let mut measured = 0.0;
            let mut rendered = 0.0;
            let output = context.run_ui(Default::default(), |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 500.0), egui::Sense::hover());
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );
                let wrap_width = child.available_width();
                measured = inline_height(&child, &error, details_open, wrap_width);

                let mut rendered_details_open = details_open;
                let start_y = child.cursor().top();
                show_inline(&mut child, &error, &mut rendered_details_open);
                rendered = child.cursor().top() - start_y;
            });

            assert!(
                (measured - rendered).abs() <= 0.5,
                "width={width}, details_open={details_open}, measured={measured}, rendered={rendered}"
            );
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn narrower_width_increases_wrapped_inline_error_height() {
        let error = MediaError::from_tools_error(&MediaToolsError::MissingBoth);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut narrow = 0.0;
        let mut wide = 0.0;

        let output = context.run_ui(Default::default(), |ui| {
            narrow = inline_height(ui, &error, false, 344.0);
            wide = inline_height(ui, &error, false, 520.0);
        });

        assert!(narrow > wide);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn short_diagnostic_uses_content_height_before_the_scroll_cap() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut short = 0.0;
        let mut long = 0.0;
        let output = context.run_ui(Default::default(), |ui| {
            short = media_lifecycle::diagnostic_height(ui, "one diagnostic line", 400.0);
            long = media_lifecycle::diagnostic_height(ui, &"diagnostic line ".repeat(200), 200.0);
        });
        assert!(short > 0.0 && short < crate::ui::layout::DIAGNOSTIC_MAX_HEIGHT);
        assert_eq!(long, crate::ui::layout::DIAGNOSTIC_MAX_HEIGHT);
        output.drop_without_applying_deltas();
    }
}
