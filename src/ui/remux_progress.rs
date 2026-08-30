use std::path::{Path, PathBuf};

use egui::{Response, Ui};

use crate::{
    media::{
        container::MediaContainer,
        error::{MediaError, MediaOutputContainer},
        progress::MediaProgress,
    },
    remux::{
        execution::{RemuxExecutionFailure, RemuxExecutionState, RemuxExecutor},
        model::RemuxRequest,
    },
};

use super::{
    layout, media_error,
    media_lifecycle::{
        file_name, heading, output_parent, progress_presentation, right_aligned_actions,
        secondary_copy, show_progress, source_to_output, technical_faint,
    },
    remux::container_label,
    theme, widgets,
};

const CANCEL_LABEL: &str = "Cancel";
const RUNNING_TITLE: &str = "Remuxing media";
const COMPLETE_TITLE: &str = "Remux complete";
const FAILED_TITLE: &str = "Remux failed";
const CANCELLED_TITLE: &str = "Remux cancelled";
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DONE_LABEL: &str = "Done";
const BACK_LABEL: &str = "Back";
const CANCELLED_COPY: &str = "No final output was published.";
const CANCELLED_CLEANUP: &str = "temporary output cleaned";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemuxLifecycleUiState {
    details_open: bool,
}

impl RemuxLifecycleUiState {
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    fn reset(&mut self) {
        self.details_open = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RemuxLifecycleAction {
    #[default]
    None,
    CancelRequested,
    OpenFolder(PathBuf),
    Done,
    Back,
}

pub fn show(
    ui: &mut Ui,
    request: &RemuxRequest,
    executor: &mut RemuxExecutor,
    ui_state: &mut RemuxLifecycleUiState,
) -> RemuxLifecycleAction {
    let execution_state = executor.poll().clone();
    let progress = executor.progress().cloned();

    match execution_state {
        RemuxExecutionState::Idle => RemuxLifecycleAction::None,
        RemuxExecutionState::Running => show_running(ui, request, progress.as_ref(), executor),
        RemuxExecutionState::Completed { output } => {
            show_completed(ui, request, &output, executor, ui_state)
        }
        RemuxExecutionState::Failed(failure) => {
            show_failed(ui, request, &failure, executor, ui_state)
        }
        RemuxExecutionState::Cancelled => show_cancelled(ui, executor, ui_state),
    }
}

fn show_running(
    ui: &mut Ui,
    request: &RemuxRequest,
    progress: Option<&MediaProgress>,
    executor: &RemuxExecutor,
) -> RemuxLifecycleAction {
    let mut action = RemuxLifecycleAction::None;
    let presentation = progress_presentation(progress);
    let path_summary = source_to_output(&request.input, &request.output);

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, RUNNING_TITLE);
        ui.add_space(theme::STATUS_COPY_GAP);
        secondary_copy(ui, &path_summary);
        ui.add_space(theme::SPACING.spacious);
        show_progress(ui, &presentation);
        ui.add_space(theme::RUNNING_FOOTER_GAP);
        let response =
            right_aligned_actions(ui, |ui| widgets::destructive_button(ui, CANCEL_LABEL));
        if !ui.is_sizing_pass() && response.clicked() && executor.cancel() {
            action = RemuxLifecycleAction::CancelRequested;
        }
    });

    action
}

fn show_completed(
    ui: &mut Ui,
    request: &RemuxRequest,
    output: &Path,
    executor: &mut RemuxExecutor,
    ui_state: &mut RemuxLifecycleUiState,
) -> RemuxLifecycleAction {
    let mut action = RemuxLifecycleAction::None;

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, COMPLETE_TITLE);
        ui.add_space(theme::SPACING.focal);
        file_name(ui, output);
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, container_label(request.container));
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, "all streams copied");
        ui.add_space(theme::RESULT_FOOTER_GAP);
        let (done, open_folder) = right_aligned_actions(ui, |ui| {
            ui.spacing_mut().item_spacing.x = theme::FOOTER_ACTION_GAP;
            let done = widgets::primary_button(ui, DONE_LABEL);
            let open_folder = widgets::secondary_button(ui, OPEN_FOLDER_LABEL);
            (done, open_folder)
        });
        if !ui.is_sizing_pass() {
            if done.clicked() {
                reset_terminal(executor, ui_state, "completed");
                action = RemuxLifecycleAction::Done;
            } else if open_folder.clicked() {
                action = RemuxLifecycleAction::OpenFolder(output_parent(output));
            }
        }
    });

    action
}

fn show_failed(
    ui: &mut Ui,
    request: &RemuxRequest,
    failure: &RemuxExecutionFailure,
    executor: &mut RemuxExecutor,
    ui_state: &mut RemuxLifecycleUiState,
) -> RemuxLifecycleAction {
    let error = failure_error(request.container, failure);
    let response = layout::focus_content(ui, |ui| {
        let mut sizing_details_open = ui_state.details_open;
        let details_open = if ui.is_sizing_pass() {
            &mut sizing_details_open
        } else {
            &mut ui_state.details_open
        };
        failure_content(ui, &error, details_open)
    });

    if response.inner.back.clicked() {
        reset_terminal(executor, ui_state, "failed");
        RemuxLifecycleAction::Back
    } else {
        RemuxLifecycleAction::None
    }
}

struct FailureContentResponse {
    back: Response,
}

fn failure_content(
    ui: &mut Ui,
    error: &MediaError,
    details_open: &mut bool,
) -> FailureContentResponse {
    let back = media_error::show_failure_content(ui, FAILED_TITLE, error, details_open, BACK_LABEL);
    FailureContentResponse { back }
}

fn failure_error(container: MediaContainer, failure: &RemuxExecutionFailure) -> MediaError {
    let output_container = match container {
        MediaContainer::Mp4 => MediaOutputContainer::Mp4,
        MediaContainer::Matroska => MediaOutputContainer::Mkv,
    };
    MediaError::from_execution_failure(failure, output_container)
}

fn show_cancelled(
    ui: &mut Ui,
    executor: &mut RemuxExecutor,
    ui_state: &mut RemuxLifecycleUiState,
) -> RemuxLifecycleAction {
    let mut action = RemuxLifecycleAction::None;
    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, CANCELLED_TITLE);
        ui.add_space(theme::STATUS_COPY_GAP);
        secondary_copy(ui, CANCELLED_COPY);
        ui.add_space(theme::DIAGNOSTIC_TOP_GAP);
        technical_faint(ui, CANCELLED_CLEANUP);
        ui.add_space(theme::RESULT_FOOTER_GAP);
        let response = right_aligned_actions(ui, |ui| widgets::primary_button(ui, DONE_LABEL));
        if !ui.is_sizing_pass() && response.clicked() {
            reset_terminal(executor, ui_state, "cancelled");
            action = RemuxLifecycleAction::Done;
        }
    });
    action
}

fn reset_terminal(
    executor: &mut RemuxExecutor,
    ui_state: &mut RemuxLifecycleUiState,
    terminal_name: &str,
) {
    let reset = executor.reset();
    debug_assert!(reset, "{terminal_name} executor must be resettable");
    ui_state.reset();
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        media::{container::MediaContainer, error::MediaErrorKind, process::MediaProcessFailure},
        remux::{
            execution::{RemuxExecutionFailure, RemuxExecutor},
            model::RemuxRequest,
        },
    };

    use super::{
        CANCELLED_CLEANUP, CANCELLED_COPY, CANCELLED_TITLE, COMPLETE_TITLE, FAILED_TITLE,
        RUNNING_TITLE, RemuxLifecycleUiState, failure_error, show_cancelled, show_completed,
        show_running,
    };

    #[test]
    fn lifecycle_copy_matches_remux_surfaces() {
        assert_eq!(RUNNING_TITLE, "Remuxing media");
        assert_eq!(COMPLETE_TITLE, "Remux complete");
        assert_eq!(FAILED_TITLE, "Remux failed");
        assert_eq!(CANCELLED_TITLE, "Remux cancelled");
        assert_eq!(CANCELLED_COPY, "No final output was published.");
        assert_eq!(CANCELLED_CLEANUP, "temporary output cleaned");
    }

    #[test]
    fn running_view_renders_progress_paths_and_cancel() {
        let request = request(MediaContainer::Matroska);
        let executor = RemuxExecutor::new();
        let output = render_test_ui_with_fonts(|ui| {
            let _ = show_running(ui, &request, None, &executor);
        });
        for text in [
            "Remuxing media",
            "recording.mov → recording-remuxed.mkv",
            "Processing",
            "Cancel",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn completed_view_renders_container_and_actions() {
        let request = request(MediaContainer::Matroska);
        let mut executor = RemuxExecutor::new();
        let mut state = RemuxLifecycleUiState::default();
        let output_path = request.output.clone();
        let output = render_test_ui_with_fonts(|ui| {
            let _ = show_completed(ui, &request, &output_path, &mut executor, &mut state);
        });
        for text in [
            "Remux complete",
            "recording-remuxed.mkv",
            "MKV",
            "all streams copied",
            "Open folder",
            "Done",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn failure_classification_uses_selected_container() {
        let failure = RemuxExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: "codec not currently supported in container could not find tag for codec"
                .to_owned(),
        };
        assert_eq!(
            failure_error(MediaContainer::Mp4, &failure).kind(),
            MediaErrorKind::Mp4Incompatibility
        );
        assert_eq!(
            failure_error(MediaContainer::Matroska, &failure).kind(),
            MediaErrorKind::UnknownFfmpegFailure
        );
    }

    #[test]
    fn cancelled_view_keeps_nonpublication_and_cleanup_copy() {
        let mut executor = RemuxExecutor::new();
        let mut state = RemuxLifecycleUiState::default();
        let output = render_test_ui_with_fonts(|ui| {
            let _ = show_cancelled(ui, &mut executor, &mut state);
        });
        for text in [
            "Remux cancelled",
            "No final output was published.",
            "temporary output cleaned",
            "Done",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        output.drop_without_applying_deltas();
    }

    fn request(container: MediaContainer) -> RemuxRequest {
        RemuxRequest {
            input: PathBuf::from("C:/media/recording.mov"),
            output: PathBuf::from(match container {
                MediaContainer::Mp4 => "C:/media/recording-remuxed.mp4",
                MediaContainer::Matroska => "C:/media/recording-remuxed.mkv",
            }),
            container,
        }
    }

    fn render_test_ui_with_fonts(mut add_contents: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let context = egui::Context::default();
        context.set_fonts(egui::FontDefinitions::default());
        context.run_ui(Default::default(), |ui| {
            let mut style = (**ui.style()).clone();
            style.text_styles.insert(
                crate::ui::fonts::technical_text_style(),
                egui::FontId::new(12.0, egui::FontFamily::Monospace),
            );
            style.text_styles.insert(
                crate::ui::fonts::wordmark_text_style(),
                egui::FontId::new(26.0, egui::FontFamily::Monospace),
            );
            ui.set_style(style);
            add_contents(ui);
        })
    }

    fn output_contains_text(output: &egui::FullOutput, expected: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|clipped| shape_contains_text(&clipped.shape, expected))
    }

    fn shape_contains_text(shape: &egui::Shape, expected: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text.contains(expected),
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .any(|shape| shape_contains_text(shape, expected)),
            _ => false,
        }
    }
}
