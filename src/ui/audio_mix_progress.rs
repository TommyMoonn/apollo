use std::path::PathBuf;

use egui::{Response, Ui};

use crate::{
    audio_mix::{
        execution::{AudioMixExecutionFailure, AudioMixExecutionState, AudioMixExecutor},
        model::{AudioMixRequest, OutputContainer},
    },
    media::{
        error::{MediaError, MediaOutputContainer},
        model::MediaFile,
        progress::MediaProgress,
    },
};

use super::{
    layout, media_error,
    media_lifecycle::{
        file_name, heading, output_parent, progress_presentation, right_aligned_actions,
        secondary_copy, show_progress, source_to_output as source_to_output_paths, technical_faint,
    },
    theme, widgets,
};

const RUNNING_TITLE: &str = "Mixing audio";
const CANCEL_LABEL: &str = "Cancel";
const COMPLETE_TITLE: &str = "Audio mix complete";
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DONE_LABEL: &str = "Done";
const FAILED_TITLE: &str = "Audio mix failed";
const BACK_LABEL: &str = "Back";
const CANCELLED_TITLE: &str = "Audio mix cancelled";
const CANCELLED_COPY: &str = "No final output was published.";
const CANCELLED_CLEANUP: &str = "temporary output cleaned";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioMixLifecycleUiState {
    details_open: bool,
}

impl AudioMixLifecycleUiState {
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    fn reset(&mut self) {
        self.details_open = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AudioMixLifecycleAction {
    #[default]
    None,
    CancelRequested,
    OpenFolder(PathBuf),
    Done,
    Back,
}

/// Render the running or terminal Audio Mix surface for the current executor state.
///
/// The shared executor remains authoritative for success, failure, cancellation, progress parsing,
/// and output settlement. This view polls it once per UI frame, performs only direct user actions
/// (cancel/reset), and returns external intent such as opening the published output folder.
pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    request: &AudioMixRequest,
    executor: &mut AudioMixExecutor,
    ui_state: &mut AudioMixLifecycleUiState,
) -> AudioMixLifecycleAction {
    let execution_state = executor.poll().clone();
    let progress = executor.progress().cloned();

    match execution_state {
        AudioMixExecutionState::Idle => AudioMixLifecycleAction::None,
        AudioMixExecutionState::Running => show_running(ui, request, progress.as_ref(), executor),
        AudioMixExecutionState::Completed { output } => {
            show_completed(ui, media, request, &output, executor, ui_state)
        }
        AudioMixExecutionState::Failed(failure) => {
            show_failed(ui, request, &failure, executor, ui_state)
        }
        AudioMixExecutionState::Cancelled => show_cancelled(ui, executor, ui_state),
    }
}

fn show_running(
    ui: &mut Ui,
    request: &AudioMixRequest,
    progress: Option<&MediaProgress>,
    executor: &AudioMixExecutor,
) -> AudioMixLifecycleAction {
    let mut action = AudioMixLifecycleAction::None;
    let presentation = progress_presentation(progress);
    let path_summary = source_to_output(request);

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
            action = AudioMixLifecycleAction::CancelRequested;
        }
    });

    action
}

fn show_completed(
    ui: &mut Ui,
    media: &MediaFile,
    request: &AudioMixRequest,
    output: &std::path::Path,
    executor: &mut AudioMixExecutor,
    ui_state: &mut AudioMixLifecycleUiState,
) -> AudioMixLifecycleAction {
    let mut action = AudioMixLifecycleAction::None;
    let video_summary = video_copy_summary(media);
    let audio_summary = mixed_audio_summary(request.container);

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, COMPLETE_TITLE);
        ui.add_space(theme::SPACING.focal);
        file_name(ui, output);

        if let Some(summary) = &video_summary {
            ui.add_space(theme::SPACING.tight);
            technical_faint(ui, summary);
        }
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, &audio_summary);

        ui.add_space(theme::RESULT_FOOTER_GAP);
        let (done, open_folder) = right_aligned_actions(ui, |ui| {
            ui.spacing_mut().item_spacing.x = theme::FOOTER_ACTION_GAP;
            let done = widgets::primary_button(ui, DONE_LABEL);
            let open_folder = widgets::secondary_button(ui, OPEN_FOLDER_LABEL);
            (done, open_folder)
        });

        if !ui.is_sizing_pass() {
            if done.clicked() {
                let reset = executor.reset();
                debug_assert!(reset, "completed executor must be resettable");
                ui_state.reset();
                action = AudioMixLifecycleAction::Done;
            } else if open_folder.clicked() {
                action = AudioMixLifecycleAction::OpenFolder(output_parent(output));
            }
        }
    });

    action
}

struct FailureContentResponse {
    back: Response,
}

fn show_failed(
    ui: &mut Ui,
    request: &AudioMixRequest,
    failure: &AudioMixExecutionFailure,
    executor: &mut AudioMixExecutor,
    ui_state: &mut AudioMixLifecycleUiState,
) -> AudioMixLifecycleAction {
    let error = failure_error(failure, request.container);
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
        let reset = executor.reset();
        debug_assert!(reset, "failed executor must be resettable");
        ui_state.reset();
        AudioMixLifecycleAction::Back
    } else {
        AudioMixLifecycleAction::None
    }
}

fn failure_content(
    ui: &mut Ui,
    error: &MediaError,
    details_open: &mut bool,
) -> FailureContentResponse {
    let back = media_error::show_failure_content(ui, FAILED_TITLE, error, details_open, BACK_LABEL);
    FailureContentResponse { back }
}

fn failure_error(failure: &AudioMixExecutionFailure, container: OutputContainer) -> MediaError {
    let output_container = match container {
        OutputContainer::Mp4 => MediaOutputContainer::Mp4,
        OutputContainer::Mkv => MediaOutputContainer::Mkv,
    };
    MediaError::from_execution_failure(failure, output_container)
}

fn show_cancelled(
    ui: &mut Ui,
    executor: &mut AudioMixExecutor,
    ui_state: &mut AudioMixLifecycleUiState,
) -> AudioMixLifecycleAction {
    let mut action = AudioMixLifecycleAction::None;

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
            let reset = executor.reset();
            debug_assert!(reset, "cancelled executor must be resettable");
            ui_state.reset();
            action = AudioMixLifecycleAction::Done;
        }
    });

    action
}

fn source_to_output(request: &AudioMixRequest) -> String {
    source_to_output_paths(&request.input, &request.output)
}

fn video_copy_summary(media: &MediaFile) -> Option<String> {
    let video = media.video_streams.first()?;
    match (video.width, video.height) {
        (Some(width), Some(height)) => Some(format!("{width} × {height} · video copied")),
        _ => Some("video copied".to_owned()),
    }
}

fn mixed_audio_summary(container: OutputContainer) -> String {
    format!("1 mixed audio stream · {}", container_label(container))
}

fn container_label(container: OutputContainer) -> &'static str {
    match container {
        OutputContainer::Mp4 => "MP4",
        OutputContainer::Mkv => "MKV",
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use crate::{
        audio_mix::{
            execution::{AudioMixExecutionFailure, AudioMixExecutionState, AudioMixExecutor},
            model::{AudioMixRequest, LoudnessNormalization, OutputContainer, SelectedAudioTrack},
        },
        media::{
            model::{AudioStream, MediaFile, VideoStream},
            process::MediaProcessFailure,
            progress::MediaProgress,
        },
    };

    use super::{
        AudioMixLifecycleUiState, CANCELLED_TITLE, COMPLETE_TITLE, FAILED_TITLE, RUNNING_TITLE,
        failure_content, failure_error, mixed_audio_summary, progress_presentation,
        source_to_output, video_copy_summary,
    };
    use crate::ui::media_lifecycle::{format_duration, format_integer};

    #[test]
    fn lifecycle_copy_covers_running_and_terminal_states() {
        assert_eq!(RUNNING_TITLE, "Mixing audio");
        assert_eq!(COMPLETE_TITLE, "Audio mix complete");
        assert_eq!(FAILED_TITLE, "Audio mix failed");
        assert_eq!(CANCELLED_TITLE, "Audio mix cancelled");
    }

    #[test]
    fn bounded_progress_display_clamps_fraction_and_formats_metadata() {
        let progress = MediaProgress {
            processed: Some(Duration::from_secs(48)),
            total: Some(Duration::from_secs(58)),
            fraction: Some(1.25),
            frame: Some(67_482),
        };

        let presentation = progress_presentation(Some(&progress));

        assert_eq!(presentation.fraction, 1.0);
        assert_eq!(presentation.percent.as_deref(), Some("100%"));
        assert_eq!(presentation.time.as_deref(), Some("00:48 / 00:58"));
        assert_eq!(presentation.frame.as_deref(), Some("Frame 67,482"));
    }

    #[test]
    fn unknown_duration_shows_elapsed_time_without_inventing_fraction() {
        let progress = MediaProgress {
            processed: Some(Duration::from_secs(37)),
            total: None,
            fraction: None,
            frame: None,
        };

        let presentation = progress_presentation(Some(&progress));

        assert_eq!(presentation.fraction, 0.0);
        assert_eq!(presentation.percent, None);
        assert_eq!(presentation.time.as_deref(), Some("00:37 elapsed"));
    }

    #[test]
    fn no_progress_snapshot_is_a_neutral_empty_rail() {
        let presentation = progress_presentation(None);

        assert_eq!(presentation.fraction, 0.0);
        assert_eq!(presentation.percent, None);
        assert_eq!(presentation.time, None);
        assert_eq!(presentation.frame, None);
    }

    #[test]
    fn expanded_failure_above_230_px_scrolls_and_keeps_back_reachable() {
        run_test_ui_with_fonts(|ui| {
            let available = egui::Vec2::new(470.0, 260.0);
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let failure = process_failure(long_diagnostic());
            let error = failure_error(&failure, OutputContainer::Mp4);
            let mut state = AudioMixLifecycleUiState { details_open: true };

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        crate::ui::layout::focus_content(ui, |ui| {
                            failure_content(ui, &error, &mut state.details_open)
                        })
                    },
                )
                .inner;
            assert!(available.y > crate::ui::layout::FOCUS_CONTENT_MIN_HEIGHT);
            assert_eq!(crate::ui::layout::DIAGNOSTIC_MAX_HEIGHT, 80.0);
            assert!(response.is_scrollable());
            assert!(response.inner.back.rect.bottom() > parent_rect.bottom());
        });
    }

    #[test]
    fn collapsing_failure_details_restores_compact_non_overflow_layout() {
        run_test_ui_with_fonts(|ui| {
            let available = egui::Vec2::new(470.0, 260.0);
            let failure = process_failure(long_diagnostic());
            let error = failure_error(&failure, OutputContainer::Mp4);

            let (expanded_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let mut expanded_state = AudioMixLifecycleUiState { details_open: true };
            let expanded = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(expanded_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        crate::ui::layout::focus_content(ui, |ui| {
                            failure_content(ui, &error, &mut expanded_state.details_open)
                        })
                    },
                )
                .inner;

            let (compact_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let mut compact_state = AudioMixLifecycleUiState::default();
            let compact = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(compact_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        crate::ui::layout::focus_content(ui, |ui| {
                            failure_content(ui, &error, &mut compact_state.details_open)
                        })
                    },
                )
                .inner;

            assert!(expanded.is_scrollable());
            assert!(!compact.is_scrollable());
            assert!(compact.inner.back.rect.bottom() <= compact_rect.bottom());
        });
    }

    #[test]
    fn wrapped_failure_copy_at_narrow_width_cannot_make_back_unreachable() {
        run_test_ui_with_fonts(|ui| {
            let available = egui::Vec2::new(220.0, 240.0);
            let (parent_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let failure = process_failure("short diagnostic".to_owned());
            let error = failure_error(&failure, OutputContainer::Mp4);
            let mut state = AudioMixLifecycleUiState::default();

            let response = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(parent_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        crate::ui::layout::focus_content(ui, |ui| {
                            failure_content(ui, &error, &mut state.details_open)
                        })
                    },
                )
                .inner;

            let back_visible = response.inner.back.rect.bottom() <= parent_rect.bottom();
            assert!(back_visible || response.is_scrollable());
            if !back_visible {
                assert!(response.content_size.y > response.viewport_size.y);
            }
        });
    }

    fn run_test_ui_with_fonts(mut add_contents: impl FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::default());
        let output = ctx.run_ui(Default::default(), |ui| {
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
        });
        output.drop_without_applying_deltas();
    }

    fn process_failure(diagnostic: String) -> AudioMixExecutionFailure {
        AudioMixExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic,
        }
    }

    fn long_diagnostic() -> String {
        (0..40)
            .map(|index| format!("diagnostic line {index}: Conversion failed while filtering"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn terminal_failure_classification_uses_selected_output_container() {
        let failure = process_failure(
            "Could not find tag for codec pcm_s16le, codec not currently supported in container"
                .to_owned(),
        );

        let mp4 = failure_error(&failure, OutputContainer::Mp4);
        assert_eq!(
            mp4.kind(),
            crate::media::error::MediaErrorKind::Mp4Incompatibility
        );

        let mkv = failure_error(&failure, OutputContainer::Mkv);
        assert_eq!(
            mkv.kind(),
            crate::media::error::MediaErrorKind::UnknownFfmpegFailure
        );
        assert!(!mkv.message().contains("MP4"));
        assert!(
            !mkv.guidance()
                .is_some_and(|guidance| guidance.contains("Use MKV"))
        );
    }

    #[test]
    fn diagnostic_details_are_retained_verbatim() {
        let failure = AudioMixExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: "stderr tail\nConversion failed!".to_owned(),
        };

        assert_eq!(
            failure_error(&failure, OutputContainer::Mp4).diagnostic(),
            Some("stderr tail\nConversion failed!")
        );
    }

    #[test]
    fn missing_diagnostics_use_a_concise_fallback() {
        let failure = AudioMixExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: String::new(),
        };

        assert_eq!(
            failure_error(&failure, OutputContainer::Mp4).diagnostic_or_fallback(),
            "No diagnostic details were reported."
        );
    }

    #[test]
    fn completion_summaries_keep_video_copy_and_single_mixed_audio_visible() {
        let media = media();

        assert_eq!(
            video_copy_summary(&media).as_deref(),
            Some("1920 × 1080 · video copied")
        );
        assert_eq!(
            mixed_audio_summary(OutputContainer::Mp4),
            "1 mixed audio stream · MP4"
        );
        assert_eq!(
            mixed_audio_summary(OutputContainer::Mkv),
            "1 mixed audio stream · MKV"
        );
    }

    #[test]
    fn running_source_to_output_uses_file_names_only() {
        let request = request();

        assert_eq!(
            source_to_output(&request),
            "recording.mkv → recording-mixed.mp4"
        );
    }

    #[test]
    fn duration_and_frame_formatting_handle_long_values() {
        assert_eq!(format_duration(Duration::from_secs(3_725)), "01:02:05");
        assert_eq!(format_integer(7), "7");
        assert_eq!(format_integer(999), "999");
        assert_eq!(format_integer(1_000), "1,000");
        assert_eq!(format_integer(12_345_678), "12,345,678");
    }

    #[test]
    fn details_state_resets_for_reusable_tool_state() {
        let mut state = AudioMixLifecycleUiState { details_open: true };
        state.reset();

        assert!(!state.details_open());
    }

    #[test]
    fn executor_idle_state_is_the_reusable_selection_boundary() {
        let executor = AudioMixExecutor::new();

        assert_eq!(executor.state(), &AudioMixExecutionState::Idle);
    }

    fn request() -> AudioMixRequest {
        AudioMixRequest {
            input: PathBuf::from("C:/media/recording.mkv"),
            output: PathBuf::from("C:/media/recording-mixed.mp4"),
            selected_tracks: vec![
                SelectedAudioTrack::with_default_gain(1),
                SelectedAudioTrack::with_default_gain(2),
            ],
            normalization: LoudnessNormalization::Off,
            container: OutputContainer::Mp4,
        }
    }

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("C:/media/recording.mkv"),
            duration: Some(Duration::from_secs(58)),
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: vec![AudioStream {
                stream_index: 1,
                codec: Some("aac".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: Some("Desktop".to_owned()),
                language: None,
            }],
        }
    }
}
