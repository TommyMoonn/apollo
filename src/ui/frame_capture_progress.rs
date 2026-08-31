use std::path::{Path, PathBuf};

use egui::Ui;

use crate::{
    frame_capture::{
        execution::{
            FrameCaptureExecutionFailure, FrameCaptureExecutionState, FrameCaptureExecutor,
        },
        model::FrameCaptureRequest,
    },
    media::{
        error::{MediaError, MediaOutputContainer},
        progress::MediaProgress,
    },
};

use super::{
    frame_capture::format_label,
    layout, media_error,
    media_lifecycle::{
        file_name, heading, output_parent, progress_presentation, right_aligned_actions,
        secondary_copy, show_progress, source_to_output, technical_faint,
    },
    theme, widgets,
};

const CANCEL_LABEL: &str = "Cancel";
const RUNNING_TITLE: &str = "Capturing frame";
const COMPLETE_TITLE: &str = "Frame capture complete";
const FAILED_TITLE: &str = "Frame capture failed";
const CANCELLED_TITLE: &str = "Frame capture cancelled";
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DONE_LABEL: &str = "Done";
const BACK_LABEL: &str = "Back";
const CANCELLED_COPY: &str = "No final output was published.";
const CANCELLED_CLEANUP: &str = "temporary output cleaned";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameCaptureLifecycleUiState {
    details_open: bool,
}

impl FrameCaptureLifecycleUiState {
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    fn reset(&mut self) {
        self.details_open = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FrameCaptureLifecycleAction {
    #[default]
    None,
    CancelRequested,
    OpenFolder(PathBuf),
    Done,
    Back,
}

pub fn show(
    ui: &mut Ui,
    request: &FrameCaptureRequest,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let execution_state = executor.poll().clone();
    let progress = executor.progress().cloned();

    match execution_state {
        FrameCaptureExecutionState::Idle => FrameCaptureLifecycleAction::None,
        FrameCaptureExecutionState::Running => {
            show_running(ui, request, progress.as_ref(), executor)
        }
        FrameCaptureExecutionState::Completed { output } => {
            show_completed(ui, request, &output, executor, ui_state)
        }
        FrameCaptureExecutionState::Failed(failure) => {
            show_failed(ui, &failure, executor, ui_state)
        }
        FrameCaptureExecutionState::Cancelled => show_cancelled(ui, executor, ui_state),
    }
}

fn show_running(
    ui: &mut Ui,
    request: &FrameCaptureRequest,
    progress: Option<&MediaProgress>,
    executor: &FrameCaptureExecutor,
) -> FrameCaptureLifecycleAction {
    let mut action = FrameCaptureLifecycleAction::None;
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
            action = FrameCaptureLifecycleAction::CancelRequested;
        }
    });
    action
}

fn show_completed(
    ui: &mut Ui,
    request: &FrameCaptureRequest,
    output: &Path,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let mut action = FrameCaptureLifecycleAction::None;
    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, COMPLETE_TITLE);
        ui.add_space(theme::SPACING.focal);
        file_name(ui, output);
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, format_label(request.format));
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, &request.timestamp.to_string());
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
                action = FrameCaptureLifecycleAction::Done;
            } else if open_folder.clicked() {
                action = FrameCaptureLifecycleAction::OpenFolder(output_parent(output));
            }
        }
    });
    action
}

fn show_failed(
    ui: &mut Ui,
    failure: &FrameCaptureExecutionFailure,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let error = MediaError::from_execution_failure(failure, MediaOutputContainer::Other);
    let response = layout::focus_content(ui, |ui| {
        let mut sizing_details_open = ui_state.details_open;
        let details_open = if ui.is_sizing_pass() {
            &mut sizing_details_open
        } else {
            &mut ui_state.details_open
        };
        media_error::show_failure_content(ui, FAILED_TITLE, &error, details_open, BACK_LABEL)
    });
    if response.inner.clicked() {
        reset_terminal(executor, ui_state, "failed");
        FrameCaptureLifecycleAction::Back
    } else {
        FrameCaptureLifecycleAction::None
    }
}

fn show_cancelled(
    ui: &mut Ui,
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
) -> FrameCaptureLifecycleAction {
    let mut action = FrameCaptureLifecycleAction::None;
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
            action = FrameCaptureLifecycleAction::Done;
        }
    });
    action
}

fn reset_terminal(
    executor: &mut FrameCaptureExecutor,
    ui_state: &mut FrameCaptureLifecycleUiState,
    terminal_name: &str,
) {
    let reset = executor.reset();
    debug_assert!(reset, "{terminal_name} executor must be resettable");
    ui_state.reset();
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::frame_capture::{
        execution::FrameCaptureExecutor,
        model::{FrameCaptureFormat, FrameCaptureRequest, FrameTimestamp},
    };

    use super::{
        CANCELLED_CLEANUP, CANCELLED_COPY, CANCELLED_TITLE, COMPLETE_TITLE, FAILED_TITLE,
        FrameCaptureLifecycleUiState, RUNNING_TITLE, show_cancelled, show_completed, show_running,
    };

    #[test]
    fn lifecycle_copy_matches_frame_capture_surfaces() {
        assert_eq!(RUNNING_TITLE, "Capturing frame");
        assert_eq!(COMPLETE_TITLE, "Frame capture complete");
        assert_eq!(FAILED_TITLE, "Frame capture failed");
        assert_eq!(CANCELLED_TITLE, "Frame capture cancelled");
    }

    #[test]
    fn running_completed_and_cancelled_views_render_expected_content() {
        let request = request();
        let executor = FrameCaptureExecutor::new();
        let running = render(|ui| {
            let _ = show_running(ui, &request, None, &executor);
        });
        for text in [
            "Capturing frame",
            "recording.mkv → recording-frame.png",
            "Processing",
            "Cancel",
        ] {
            assert!(contains(&running, text), "missing {text:?}");
        }
        running.drop_without_applying_deltas();

        let mut executor = FrameCaptureExecutor::new();
        let mut state = FrameCaptureLifecycleUiState::default();
        let completed = render(|ui| {
            let _ = show_completed(ui, &request, &request.output, &mut executor, &mut state);
        });
        for text in [
            "Frame capture complete",
            "recording-frame.png",
            "PNG",
            "00:00:01.250",
            "Open folder",
            "Done",
        ] {
            assert!(contains(&completed, text), "missing {text:?}");
        }
        completed.drop_without_applying_deltas();

        let cancelled = render(|ui| {
            let _ = show_cancelled(ui, &mut executor, &mut state);
        });
        for text in [CANCELLED_TITLE, CANCELLED_COPY, CANCELLED_CLEANUP, "Done"] {
            assert!(contains(&cancelled, text), "missing {text:?}");
        }
        cancelled.drop_without_applying_deltas();
    }

    fn request() -> FrameCaptureRequest {
        FrameCaptureRequest {
            input: PathBuf::from("C:/media/recording.mkv"),
            output: PathBuf::from("C:/media/recording-frame.png"),
            timestamp: FrameTimestamp::from_milliseconds(1_250),
            format: FrameCaptureFormat::Png,
        }
    }

    fn render(mut add: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
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
            add(ui);
        })
    }

    fn contains(output: &egui::FullOutput, expected: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|clipped| shape_contains(&clipped.shape, expected))
    }

    fn shape_contains(shape: &egui::Shape, expected: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text.contains(expected),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| shape_contains(shape, expected)),
            _ => false,
        }
    }
}
