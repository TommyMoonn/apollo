use std::path::PathBuf;

use egui::{Response, Ui};

use crate::{
    media::{
        error::{MediaError, MediaOutputContainer},
        model::MediaFile,
        progress::MediaProgress,
    },
    upscale::{
        execution::{UpscaleExecutionFailure, UpscaleExecutionState, UpscaleExecutor},
        model::UpscaleRequest,
        sizing::calculate_output_dimensions,
    },
};

use super::{
    layout, media_error,
    media_lifecycle::{
        file_name, heading, output_parent, progress_presentation, right_aligned_actions,
        secondary_copy, show_progress, source_to_output, technical_faint,
    },
    theme,
    upscale::{codec_label, target_label},
    widgets,
};

const CANCEL_LABEL: &str = "Cancel";
const COMPLETE_TITLE: &str = "Upscale complete";
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DONE_LABEL: &str = "Done";
const FAILED_TITLE: &str = "Upscale failed";
const BACK_LABEL: &str = "Back";
const CANCELLED_TITLE: &str = "Upscale cancelled";
const CANCELLED_COPY: &str = "No final output was published.";
const CANCELLED_CLEANUP: &str = "temporary output cleaned";
const UNAVAILABLE_DIMENSIONS: &str = "Dimensions unavailable";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpscaleLifecycleUiState {
    details_open: bool,
}

impl UpscaleLifecycleUiState {
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    fn reset(&mut self) {
        self.details_open = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpscaleLifecycleAction {
    #[default]
    None,
    CancelRequested,
    OpenFolder(PathBuf),
    Done,
    Back,
}

/// Render the running or terminal Upscale surface for the current executor state.
///
/// The executor remains authoritative for process completion, cancellation, progress, output
/// publication, and cleanup. This view only presents that state, performs direct executor actions,
/// and returns external intent such as opening the published output folder.
pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    request: &UpscaleRequest,
    executor: &mut UpscaleExecutor,
    ui_state: &mut UpscaleLifecycleUiState,
) -> UpscaleLifecycleAction {
    let execution_state = executor.poll().clone();
    let progress = executor.progress().cloned();

    match execution_state {
        UpscaleExecutionState::Idle => UpscaleLifecycleAction::None,
        UpscaleExecutionState::Running => show_running(ui, request, progress.as_ref(), executor),
        UpscaleExecutionState::Completed { output } => {
            show_completed(ui, media, request, &output, executor, ui_state)
        }
        UpscaleExecutionState::Failed(failure) => show_failed(ui, &failure, executor, ui_state),
        UpscaleExecutionState::Cancelled => show_cancelled(ui, executor, ui_state),
    }
}

fn show_running(
    ui: &mut Ui,
    request: &UpscaleRequest,
    progress: Option<&MediaProgress>,
    executor: &UpscaleExecutor,
) -> UpscaleLifecycleAction {
    let mut action = UpscaleLifecycleAction::None;
    let presentation = progress_presentation(progress);

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, &running_title(request));
        ui.add_space(theme::STATUS_COPY_GAP);
        secondary_copy(ui, &source_to_output(&request.input, &request.output));

        ui.add_space(theme::SPACING.spacious);
        show_progress(ui, &presentation);

        ui.add_space(theme::RUNNING_FOOTER_GAP);
        let response =
            right_aligned_actions(ui, |ui| widgets::destructive_button(ui, CANCEL_LABEL));
        if response.clicked() && executor.cancel() {
            action = UpscaleLifecycleAction::CancelRequested;
        }
    });

    action
}

fn show_completed(
    ui: &mut Ui,
    media: &MediaFile,
    request: &UpscaleRequest,
    output: &std::path::Path,
    executor: &mut UpscaleExecutor,
    ui_state: &mut UpscaleLifecycleUiState,
) -> UpscaleLifecycleAction {
    let mut action = UpscaleLifecycleAction::None;

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, COMPLETE_TITLE);
        ui.add_space(theme::SPACING.focal);
        file_name(ui, output);

        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, &completion_dimensions(media, request));
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, codec_label(request.codec));

        ui.add_space(theme::RESULT_FOOTER_GAP);
        let (done, open_folder) = right_aligned_actions(ui, |ui| {
            ui.spacing_mut().item_spacing.x = theme::FOOTER_ACTION_GAP;
            let done = widgets::primary_button(ui, DONE_LABEL);
            let open_folder = widgets::secondary_button(ui, OPEN_FOLDER_LABEL);
            (done, open_folder)
        });

        if done.clicked() {
            reset_terminal(executor, ui_state, "completed");
            action = UpscaleLifecycleAction::Done;
        } else if open_folder.clicked() {
            action = UpscaleLifecycleAction::OpenFolder(output_parent(output));
        }
    });

    action
}

struct FailureContentResponse {
    back: Response,
}

fn show_failed(
    ui: &mut Ui,
    failure: &UpscaleExecutionFailure,
    executor: &mut UpscaleExecutor,
    ui_state: &mut UpscaleLifecycleUiState,
) -> UpscaleLifecycleAction {
    let response = layout::focus_content(ui, |ui| failure_content(ui, failure, ui_state));

    if response.inner.back.clicked() {
        reset_terminal(executor, ui_state, "failed");
        UpscaleLifecycleAction::Back
    } else {
        UpscaleLifecycleAction::None
    }
}

fn failure_content(
    ui: &mut Ui,
    failure: &UpscaleExecutionFailure,
    ui_state: &mut UpscaleLifecycleUiState,
) -> FailureContentResponse {
    let error = failure_error(failure);
    let back = media_error::show_failure_content(
        ui,
        FAILED_TITLE,
        &error,
        &mut ui_state.details_open,
        BACK_LABEL,
    );
    FailureContentResponse { back }
}

fn failure_error(failure: &UpscaleExecutionFailure) -> MediaError {
    MediaError::from_execution_failure(failure, MediaOutputContainer::Mp4)
}

fn show_cancelled(
    ui: &mut Ui,
    executor: &mut UpscaleExecutor,
    ui_state: &mut UpscaleLifecycleUiState,
) -> UpscaleLifecycleAction {
    let mut action = UpscaleLifecycleAction::None;

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, CANCELLED_TITLE);
        ui.add_space(theme::STATUS_COPY_GAP);
        secondary_copy(ui, CANCELLED_COPY);
        ui.add_space(theme::DIAGNOSTIC_TOP_GAP);
        technical_faint(ui, CANCELLED_CLEANUP);

        ui.add_space(theme::RESULT_FOOTER_GAP);
        let response = right_aligned_actions(ui, |ui| widgets::primary_button(ui, DONE_LABEL));
        if response.clicked() {
            reset_terminal(executor, ui_state, "cancelled");
            action = UpscaleLifecycleAction::Done;
        }
    });

    action
}

fn running_title(request: &UpscaleRequest) -> String {
    format!("Upscaling to {}", target_label(request.target))
}

fn completion_dimensions(media: &MediaFile, request: &UpscaleRequest) -> String {
    media
        .video_streams
        .first()
        .and_then(|video| Some((video.width?, video.height?)))
        .and_then(|(width, height)| calculate_output_dimensions(width, height, request.target).ok())
        .map(|dimensions| format!("{} × {}", dimensions.width, dimensions.height))
        .unwrap_or_else(|| UNAVAILABLE_DIMENSIONS.to_owned())
}

fn reset_terminal(
    executor: &mut UpscaleExecutor,
    ui_state: &mut UpscaleLifecycleUiState,
    terminal_name: &str,
) {
    let reset = executor.reset();
    debug_assert!(reset, "{terminal_name} executor must be resettable");
    ui_state.reset();
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use crate::{
        media::{
            model::{FrameRate, MediaFile, VideoStream},
            process::MediaProcessFailure,
            progress::MediaProgress,
        },
        upscale::{
            execution::{UpscaleExecutionFailure, UpscaleExecutionState, UpscaleExecutor},
            model::{UpscaleRequest, UpscaleTarget, VideoCodec},
        },
    };

    use super::{
        CANCELLED_CLEANUP, CANCELLED_COPY, CANCELLED_TITLE, COMPLETE_TITLE, FAILED_TITLE,
        UpscaleLifecycleUiState, completion_dimensions, failure_content, failure_error,
        reset_terminal, running_title, show_cancelled, show_completed, show_running,
    };
    use crate::{media::error::MediaErrorKind, ui::media_lifecycle::progress_presentation};

    #[test]
    fn lifecycle_copy_matches_quiet_focus_upscale_surfaces() {
        let request = request();

        assert_eq!(running_title(&request), "Upscaling to 4K");
        assert_eq!(COMPLETE_TITLE, "Upscale complete");
        assert_eq!(FAILED_TITLE, "Upscale failed");
        assert_eq!(CANCELLED_TITLE, "Upscale cancelled");
        assert_eq!(CANCELLED_COPY, "No final output was published.");
        assert_eq!(CANCELLED_CLEANUP, "temporary output cleaned");
    }

    #[test]
    fn running_view_renders_target_progress_and_source_to_output() {
        let request = request();
        let progress = MediaProgress {
            processed: Some(Duration::from_secs(37)),
            total: Some(Duration::from_secs(61)),
            fraction: Some(0.61),
            frame: Some(67_482),
        };
        let executor = UpscaleExecutor::new();

        let output = render_test_ui_with_fonts(|ui| {
            let _ = show_running(ui, &request, Some(&progress), &executor);
        });

        for text in [
            "Upscaling to 4K",
            "recording.mkv → recording-4k.mp4",
            "Processing",
            "61%",
            "00:37 / 01:01",
            "Frame 67,482",
            "Cancel",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn frame_and_progress_updates_use_shared_progress_presentation() {
        let progress = MediaProgress {
            processed: Some(Duration::from_secs(37)),
            total: Some(Duration::from_secs(61)),
            fraction: Some(0.61),
            frame: Some(67_482),
        };

        let presentation = progress_presentation(Some(&progress));

        assert_eq!(presentation.fraction, 0.61);
        assert_eq!(presentation.percent.as_deref(), Some("61%"));
        assert_eq!(presentation.time.as_deref(), Some("00:37 / 01:01"));
        assert_eq!(presentation.frame.as_deref(), Some("Frame 67,482"));
    }

    #[test]
    fn unknown_duration_keeps_elapsed_and_frame_without_inventing_percent() {
        let progress = MediaProgress {
            processed: Some(Duration::from_secs(37)),
            total: None,
            fraction: None,
            frame: Some(500),
        };

        let presentation = progress_presentation(Some(&progress));

        assert_eq!(presentation.fraction, 0.0);
        assert_eq!(presentation.percent, None);
        assert_eq!(presentation.time.as_deref(), Some("00:37 elapsed"));
        assert_eq!(presentation.frame.as_deref(), Some("Frame 500"));
    }

    #[test]
    fn completion_details_use_actual_sizing_and_selected_codec() {
        let media = media();
        let request = request();

        assert_eq!(completion_dimensions(&media, &request), "3840 × 2160");
        assert_eq!(crate::ui::upscale::codec_label(request.codec), "H.264");
    }

    #[test]
    fn completed_view_renders_output_resolution_codec_and_actions() {
        let media = media();
        let request = request();
        let mut executor = UpscaleExecutor::new();
        let mut state = UpscaleLifecycleUiState::default();
        let output_path = PathBuf::from("C:/media/recording-4k.mp4");

        let output = render_test_ui_with_fonts(|ui| {
            let _ = show_completed(
                ui,
                &media,
                &request,
                &output_path,
                &mut executor,
                &mut state,
            );
        });

        for text in [
            "Upscale complete",
            "recording-4k.mp4",
            "3840 × 2160",
            "H.264",
            "Open folder",
            "Done",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn terminal_failure_classification_keeps_upscale_mp4_context() {
        let failure = UpscaleExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic:
                "Could not find tag for codec pcm_s16le, codec not currently supported in container"
                    .to_owned(),
        };

        let error = failure_error(&failure);
        assert_eq!(error.kind(), MediaErrorKind::Mp4Incompatibility);
        assert!(error.message().contains("MP4"));
    }

    #[test]
    fn failure_details_retain_process_reason_and_diagnostic() {
        let failure = UpscaleExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: "Error initializing output stream\nConversion failed!".to_owned(),
        };

        let error = failure_error(&failure);
        assert_eq!(error.kind(), MediaErrorKind::UnknownFfmpegFailure);
        assert_eq!(
            error.diagnostic(),
            Some("Error initializing output stream\nConversion failed!")
        );
    }

    #[test]
    fn cancelled_view_states_nonpublication_and_cleanup() {
        let mut executor = UpscaleExecutor::new();
        let mut state = UpscaleLifecycleUiState::default();

        let output = render_test_ui_with_fonts(|ui| {
            let _ = show_cancelled(ui, &mut executor, &mut state);
        });

        for text in [
            "Upscale cancelled",
            "No final output was published.",
            "temporary output cleaned",
            "Done",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        output.drop_without_applying_deltas();
    }

    #[test]
    fn expanded_failure_keeps_back_reachable_or_scrollable() {
        run_test_ui_with_fonts(|ui| {
            let available = egui::Vec2::new(220.0, 240.0);
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let failure = UpscaleExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: (0..40)
                    .map(|index| format!("diagnostic line {index}: Conversion failed"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            let mut state = UpscaleLifecycleUiState { details_open: true };

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        crate::ui::layout::focus_content(ui, |ui| {
                            failure_content(ui, &failure, &mut state)
                        })
                    },
                )
                .inner;

            let back_visible = response.inner.back.rect.bottom() <= parent_rect.bottom();
            assert!(back_visible || response.is_scrollable());
        });
    }

    #[test]
    fn terminal_reset_returns_executor_and_lifecycle_state_to_idle_defaults() {
        let mut executor = UpscaleExecutor::new();
        let mut state = UpscaleLifecycleUiState { details_open: true };

        reset_terminal(&mut executor, &mut state, "test");

        assert_eq!(executor.state(), &UpscaleExecutionState::Idle);
        assert!(!state.details_open());
    }

    fn request() -> UpscaleRequest {
        UpscaleRequest {
            input: PathBuf::from("C:/media/recording.mkv"),
            output: PathBuf::from("C:/media/recording-4k.mp4"),
            target: UpscaleTarget::Uhd4k,
            codec: VideoCodec::H264,
        }
    }

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("C:/media/recording.mkv"),
            duration: Some(Duration::from_secs(61)),
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

    fn run_test_ui_with_fonts(mut add_contents: impl FnMut(&mut egui::Ui)) {
        render_test_ui_with_fonts(|ui| add_contents(ui)).drop_without_applying_deltas();
    }

    fn render_test_ui_with_fonts(mut add_contents: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::default());
        ctx.run_ui(Default::default(), |ui| {
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

    fn output_contains_text(output: &egui::FullOutput, needle: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|clipped| shape_contains_text(&clipped.shape, needle))
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
}
