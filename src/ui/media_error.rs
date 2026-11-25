use egui::{Response, Ui};

use crate::media::error::MediaError;

use super::{
    media_lifecycle::{diagnostic_viewport, heading, right_aligned_actions, secondary_copy},
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
        },
        ui::{fonts, theme},
    };

    use super::show_inline;

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
        fonts::install(&context);
        theme::install(&context);
        fn contains_text(shape: &egui::Shape, needle: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.job.text.contains(needle),
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| contains_text(shape, needle)),
                _ => false,
            }
        }
        for expanded in [false, true] {
            let mut details_open = expanded;
            let mut output = context.run_ui(Default::default(), |ui| {
                show_inline(ui, &error, &mut details_open);
            });
            output.textures_delta.clear();
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| contains_text(&shape.shape, "details"))
            );
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| contains_text(&shape.shape, error.message()))
            );
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .any(|shape| contains_text(&shape.shape, "RAW-STDERR-MARKER")),
                expanded
            );
            output.drop_without_applying_deltas();
        }
    }
}
