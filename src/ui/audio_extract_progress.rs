use std::path::{Path, PathBuf};

use egui::Ui;

use crate::{
    audio_extract::{
        execution::{
            AudioExtractExecutionFailure, AudioExtractExecutionState, AudioExtractExecutor,
        },
        model::{AudioExtractFormat, AudioExtractRequest},
    },
    media::{
        error::{MediaError, MediaOutputContainer},
        progress::MediaProgress,
    },
};

use super::{
    audio_extract::format_label,
    layout, media_error,
    media_lifecycle::{
        file_name, heading, output_parent, progress_presentation, right_aligned_actions,
        secondary_copy, show_progress, source_to_output, technical_faint,
    },
    theme, widgets,
};

const CANCEL_LABEL: &str = "Cancel";
const RUNNING_TITLE: &str = "Extracting audio";
const COMPLETE_TITLE: &str = "Audio extraction complete";
const FAILED_TITLE: &str = "Audio extraction failed";
const CANCELLED_TITLE: &str = "Audio extraction cancelled";
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DONE_LABEL: &str = "Done";
const BACK_LABEL: &str = "Back";
const CANCELLED_COPY: &str = "No final output was published.";
const CANCELLED_CLEANUP: &str = "temporary output cleaned";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioExtractLifecycleUiState {
    details_open: bool,
}

impl AudioExtractLifecycleUiState {
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    fn reset(&mut self) {
        self.details_open = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AudioExtractLifecycleAction {
    #[default]
    None,
    CancelRequested,
    OpenFolder(PathBuf),
    Done,
    Back,
}

pub fn show(
    ui: &mut Ui,
    request: &AudioExtractRequest,
    executor: &mut AudioExtractExecutor,
    ui_state: &mut AudioExtractLifecycleUiState,
) -> AudioExtractLifecycleAction {
    let execution_state = executor.poll().clone();
    let progress = executor.progress().cloned();

    match execution_state {
        AudioExtractExecutionState::Idle => AudioExtractLifecycleAction::None,
        AudioExtractExecutionState::Running => {
            show_running(ui, request, progress.as_ref(), executor)
        }
        AudioExtractExecutionState::Completed { output } => {
            show_completed(ui, request, &output, executor, ui_state)
        }
        AudioExtractExecutionState::Failed(failure) => {
            show_failed(ui, request, &failure, executor, ui_state)
        }
        AudioExtractExecutionState::Cancelled => show_cancelled(ui, executor, ui_state),
    }
}

fn show_running(
    ui: &mut Ui,
    request: &AudioExtractRequest,
    progress: Option<&MediaProgress>,
    executor: &AudioExtractExecutor,
) -> AudioExtractLifecycleAction {
    let mut action = AudioExtractLifecycleAction::None;
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
            action = AudioExtractLifecycleAction::CancelRequested;
        }
    });
    action
}

fn show_completed(
    ui: &mut Ui,
    request: &AudioExtractRequest,
    output: &Path,
    executor: &mut AudioExtractExecutor,
    ui_state: &mut AudioExtractLifecycleUiState,
) -> AudioExtractLifecycleAction {
    let mut action = AudioExtractLifecycleAction::None;
    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, COMPLETE_TITLE);
        ui.add_space(theme::SPACING.focal);
        file_name(ui, output);
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, format_label(request.format));
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, completion_detail(request.format));
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
                action = AudioExtractLifecycleAction::Done;
            } else if open_folder.clicked() {
                action = AudioExtractLifecycleAction::OpenFolder(output_parent(output));
            }
        }
    });
    action
}

fn show_failed(
    ui: &mut Ui,
    request: &AudioExtractRequest,
    failure: &AudioExtractExecutionFailure,
    executor: &mut AudioExtractExecutor,
    ui_state: &mut AudioExtractLifecycleUiState,
) -> AudioExtractLifecycleAction {
    let error = failure_error(request.format, failure);
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
        AudioExtractLifecycleAction::Back
    } else {
        AudioExtractLifecycleAction::None
    }
}

fn failure_error(format: AudioExtractFormat, failure: &AudioExtractExecutionFailure) -> MediaError {
    let output_container = match format {
        AudioExtractFormat::Original => MediaOutputContainer::Mkv,
        AudioExtractFormat::Aac => MediaOutputContainer::Mp4,
        AudioExtractFormat::Flac => MediaOutputContainer::Other,
    };
    MediaError::from_execution_failure(failure, output_container)
}

fn show_cancelled(
    ui: &mut Ui,
    executor: &mut AudioExtractExecutor,
    ui_state: &mut AudioExtractLifecycleUiState,
) -> AudioExtractLifecycleAction {
    let mut action = AudioExtractLifecycleAction::None;
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
            action = AudioExtractLifecycleAction::Done;
        }
    });
    action
}

fn completion_detail(format: AudioExtractFormat) -> &'static str {
    match format {
        AudioExtractFormat::Original => "selected stream copied",
        AudioExtractFormat::Aac => "AAC · 256 kbps",
        AudioExtractFormat::Flac => "lossless audio",
    }
}

fn reset_terminal(
    executor: &mut AudioExtractExecutor,
    ui_state: &mut AudioExtractLifecycleUiState,
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
        audio_extract::{
            execution::{AudioExtractExecutionFailure, AudioExtractExecutor},
            model::{AudioExtractFormat, AudioExtractRequest},
        },
        media::{error::MediaErrorKind, process::MediaProcessFailure},
    };

    use super::{
        AudioExtractLifecycleUiState, CANCELLED_CLEANUP, CANCELLED_COPY, CANCELLED_TITLE,
        COMPLETE_TITLE, FAILED_TITLE, RUNNING_TITLE, failure_error, show_cancelled, show_completed,
        show_running,
    };

    #[test]
    fn lifecycle_copy_matches_audio_extract_surfaces() {
        assert_eq!(RUNNING_TITLE, "Extracting audio");
        assert_eq!(COMPLETE_TITLE, "Audio extraction complete");
        assert_eq!(FAILED_TITLE, "Audio extraction failed");
        assert_eq!(CANCELLED_TITLE, "Audio extraction cancelled");
    }

    #[test]
    fn running_completed_and_cancelled_views_render_expected_content() {
        let request = request(AudioExtractFormat::Aac);
        let executor = AudioExtractExecutor::new();
        let running = render(|ui| {
            let _ = show_running(ui, &request, None, &executor);
        });
        for text in [
            "Extracting audio",
            "recording.mkv → recording-audio.m4a",
            "Processing",
            "Cancel",
        ] {
            assert!(contains(&running, text), "missing {text:?}");
        }
        running.drop_without_applying_deltas();

        let mut executor = AudioExtractExecutor::new();
        let mut state = AudioExtractLifecycleUiState::default();
        let completed = render(|ui| {
            let _ = show_completed(ui, &request, &request.output, &mut executor, &mut state);
        });
        for text in [
            "Audio extraction complete",
            "recording-audio.m4a",
            "AAC",
            "256 kbps",
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

    #[test]
    fn only_aac_uses_mp4_failure_classification() {
        let failure = AudioExtractExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: "codec not currently supported in container could not find tag for codec"
                .into(),
        };
        assert_eq!(
            failure_error(AudioExtractFormat::Aac, &failure).kind(),
            MediaErrorKind::Mp4Incompatibility
        );
        assert_eq!(
            failure_error(AudioExtractFormat::Original, &failure).kind(),
            MediaErrorKind::UnknownFfmpegFailure
        );
        assert_eq!(
            failure_error(AudioExtractFormat::Flac, &failure).kind(),
            MediaErrorKind::UnknownFfmpegFailure
        );
    }

    fn request(format: AudioExtractFormat) -> AudioExtractRequest {
        AudioExtractRequest {
            input: PathBuf::from("C:/media/recording.mkv"),
            output: PathBuf::from(format!("C:/media/recording-audio.{}", format.extension())),
            audio_stream_index: 3,
            format,
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
