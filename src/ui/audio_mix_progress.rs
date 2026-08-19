use std::{path::PathBuf, time::Duration};

use egui::{Align, Label, Layout, Response, RichText, ScrollArea, Sides, TextStyle, Ui};

use crate::{
    audio_mix::{
        execution::{AudioMixExecutionFailure, AudioMixExecutionState, AudioMixExecutor},
        model::{AudioMixRequest, OutputContainer},
    },
    media::{model::MediaFile, process::MediaProcessFailure, progress::MediaProgress},
};

use super::{fonts, layout, theme, widgets};

const RUNNING_TITLE: &str = "Mixing audio";
const PROCESSING_LABEL: &str = "Processing";
const CANCEL_LABEL: &str = "Cancel";
const COMPLETE_TITLE: &str = "Audio mix complete";
const OPEN_FOLDER_LABEL: &str = "Open folder";
const DONE_LABEL: &str = "Done";
const FAILED_TITLE: &str = "Audio mix failed";
const FAILURE_COPY: &str =
    "The selected audio streams could not be combined into the requested output.";
const SHOW_DETAILS_LABEL: &str = "Show details";
const HIDE_DETAILS_LABEL: &str = "Hide details";
const BACK_LABEL: &str = "Back";
const CANCELLED_TITLE: &str = "Audio mix cancelled";
const CANCELLED_COPY: &str = "No final output was published.";
const CANCELLED_CLEANUP: &str = "temporary output cleaned";
const NO_DIAGNOSTIC: &str = "No diagnostic details were reported.";

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

#[derive(Debug, Clone, PartialEq)]
struct ProgressPresentation {
    fraction: f32,
    percent: Option<String>,
    time: Option<String>,
    frame: Option<String>,
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
        AudioMixExecutionState::Failed(failure) => show_failed(ui, &failure, executor, ui_state),
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

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, RUNNING_TITLE);
        ui.add_space(theme::STATUS_COPY_GAP);
        secondary_copy(ui, &source_to_output(request));

        ui.add_space(theme::SPACING.spacious);
        progress_header(ui, &presentation);
        ui.add_space(theme::SPACING.compact);
        let _ = widgets::progress_bar(ui, presentation.fraction);
        ui.add_space(theme::SPACING.compact);
        progress_metadata(ui, &presentation);

        ui.add_space(theme::RUNNING_FOOTER_GAP);
        let response =
            right_aligned_actions(ui, |ui| widgets::destructive_button(ui, CANCEL_LABEL));
        if response.clicked() && executor.cancel() {
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

    layout::focus_content(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        heading(ui, COMPLETE_TITLE);
        ui.add_space(theme::SPACING.focal);
        file_name(ui, output);

        if let Some(summary) = video_copy_summary(media) {
            ui.add_space(theme::SPACING.tight);
            technical_faint(ui, &summary);
        }
        ui.add_space(theme::SPACING.tight);
        technical_faint(ui, &mixed_audio_summary(request.container));

        ui.add_space(theme::RESULT_FOOTER_GAP);
        let (done, open_folder) = right_aligned_actions(ui, |ui| {
            ui.spacing_mut().item_spacing.x = theme::FOOTER_ACTION_GAP;
            let done = widgets::primary_button(ui, DONE_LABEL);
            let open_folder = widgets::secondary_button(ui, OPEN_FOLDER_LABEL);
            (done, open_folder)
        });

        if done.clicked() {
            let reset = executor.reset();
            debug_assert!(reset, "completed executor must be resettable");
            ui_state.reset();
            action = AudioMixLifecycleAction::Done;
        } else if open_folder.clicked() {
            action = AudioMixLifecycleAction::OpenFolder(output_parent(output));
        }
    });

    action
}

struct FailureContentResponse {
    back: Response,
}

fn show_failed(
    ui: &mut Ui,
    failure: &AudioMixExecutionFailure,
    executor: &mut AudioMixExecutor,
    ui_state: &mut AudioMixLifecycleUiState,
) -> AudioMixLifecycleAction {
    let response = layout::focus_content(ui, |ui| failure_content(ui, failure, ui_state));

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
    failure: &AudioMixExecutionFailure,
    ui_state: &mut AudioMixLifecycleUiState,
) -> FailureContentResponse {
    let diagnostic = failure_diagnostic(failure);

    ui.spacing_mut().item_spacing.y = 0.0;
    heading(ui, FAILED_TITLE);
    ui.add_space(theme::STATUS_COPY_GAP);
    secondary_copy(ui, FAILURE_COPY);

    ui.add_space(theme::DIAGNOSTIC_TOP_GAP);
    widgets::separator(ui);
    ui.add_space(theme::DETAILS_SEPARATOR_GAP);
    technical_faint(ui, &failure_summary(failure));

    ui.add_space(theme::DETAILS_TOP_GAP);
    let details_label = if ui_state.details_open {
        HIDE_DETAILS_LABEL
    } else {
        SHOW_DETAILS_LABEL
    };
    if widgets::plain_button(ui, details_label).clicked() {
        ui_state.details_open = !ui_state.details_open;
    }
    if ui_state.details_open {
        ui.add_space(theme::DETAILS_TOP_GAP);
        widgets::separator(ui);
        ui.add_space(theme::DETAILS_SEPARATOR_GAP);
        diagnostic_viewport(ui, diagnostic);
    }

    ui.add_space(theme::RESULT_FOOTER_GAP);
    let back = right_aligned_actions(ui, |ui| widgets::secondary_button(ui, BACK_LABEL));

    FailureContentResponse { back }
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
        if response.clicked() {
            let reset = executor.reset();
            debug_assert!(reset, "cancelled executor must be resettable");
            ui_state.reset();
            action = AudioMixLifecycleAction::Done;
        }
    });

    action
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(RichText::new(text).text_style(TextStyle::Heading))
            .selectable(false)
            .wrap(),
    );
}

fn secondary_copy(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(TextStyle::Small)
                .color(theme::SECONDARY),
        )
        .selectable(false)
        .wrap(),
    );
}

fn technical_faint(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
        )
        .selectable(false)
        .wrap(),
    );
}

fn diagnostic_viewport(ui: &mut Ui, text: &str) {
    ScrollArea::vertical()
        .max_height(layout::DIAGNOSTIC_MAX_HEIGHT)
        .min_scrolled_height(layout::DIAGNOSTIC_MAX_HEIGHT)
        .auto_shrink([false, false])
        .show(ui, |ui| diagnostic_text(ui, text));
}

fn diagnostic_text(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
        )
        .selectable(true)
        .wrap(),
    );
}

fn file_name(ui: &mut Ui, output: &std::path::Path) {
    let name = output
        .file_name()
        .unwrap_or(output.as_os_str())
        .to_string_lossy();
    ui.add(
        Label::new(RichText::new(name).text_style(TextStyle::Body))
            .truncate()
            .selectable(false),
    );
}

fn progress_header(ui: &mut Ui, presentation: &ProgressPresentation) {
    Sides::new()
        .spacing(theme::INLINE_ACTION_GAP)
        .truncate()
        .show(
            ui,
            |ui| secondary_copy(ui, PROCESSING_LABEL),
            |ui| {
                if let Some(percent) = &presentation.percent {
                    technical_faint(ui, percent);
                }
            },
        );
}

fn progress_metadata(ui: &mut Ui, presentation: &ProgressPresentation) {
    Sides::new()
        .spacing(theme::INLINE_ACTION_GAP)
        .truncate()
        .show(
            ui,
            |ui| {
                if let Some(time) = &presentation.time {
                    technical_faint(ui, time);
                }
            },
            |ui| {
                if let Some(frame) = &presentation.frame {
                    technical_faint(ui, frame);
                }
            },
        );
}

fn right_aligned_actions<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), theme::PRIMARY_CONTROL_HEIGHT),
        Layout::right_to_left(Align::Center),
        add_contents,
    )
    .inner
}

fn progress_presentation(progress: Option<&MediaProgress>) -> ProgressPresentation {
    let fraction = progress
        .and_then(|progress| progress.fraction)
        .filter(|fraction| fraction.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let percent = progress
        .and_then(|progress| progress.fraction)
        .filter(|fraction| fraction.is_finite())
        .map(|fraction| format!("{:.0}%", fraction.clamp(0.0, 1.0) * 100.0));

    let time = progress.and_then(|progress| match (progress.processed, progress.total) {
        (Some(processed), Some(total)) => Some(format!(
            "{} / {}",
            format_duration(processed),
            format_duration(total)
        )),
        (Some(processed), None) => Some(format!("{} elapsed", format_duration(processed))),
        (None, Some(total)) => Some(format!("-- / {}", format_duration(total))),
        (None, None) => None,
    });

    let frame = progress
        .and_then(|progress| progress.frame)
        .map(|frame| format!("Frame {}", format_integer(frame)));

    ProgressPresentation {
        fraction,
        percent,
        time,
        frame,
    }
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn format_integer(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    let first_group = digits.len() % 3;

    for (index, character) in digits.chars().enumerate() {
        if index > 0
            && (index == first_group
                || (index > first_group && (index - first_group).is_multiple_of(3)))
        {
            formatted.push(',');
        }
        formatted.push(character);
    }

    formatted
}

fn source_to_output(request: &AudioMixRequest) -> String {
    format!(
        "{} → {}",
        file_display_name(&request.input),
        file_display_name(&request.output)
    )
}

fn file_display_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
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

fn failure_summary(failure: &AudioMixExecutionFailure) -> String {
    match failure {
        AudioMixExecutionFailure::Process { reason, .. } => match reason {
            MediaProcessFailure::Exit(Some(code)) => format!("FFmpeg exited with code {code}."),
            MediaProcessFailure::Exit(None) => "FFmpeg exited unsuccessfully.".to_owned(),
            MediaProcessFailure::Io(kind) => format!("FFmpeg process failed: {kind:?}."),
        },
        AudioMixExecutionFailure::Output { reason, .. } => reason.to_string(),
    }
}

fn failure_diagnostic(failure: &AudioMixExecutionFailure) -> &str {
    let diagnostic = match failure {
        AudioMixExecutionFailure::Process { diagnostic, .. }
        | AudioMixExecutionFailure::Output { diagnostic, .. } => diagnostic.as_str(),
    };

    if diagnostic.trim().is_empty() {
        NO_DIAGNOSTIC
    } else {
        diagnostic
    }
}

fn output_parent(output: &std::path::Path) -> PathBuf {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use crate::{
        audio_mix::{
            execution::{AudioMixExecutionFailure, AudioMixExecutionState, AudioMixExecutor},
            model::{AudioMixRequest, OutputContainer},
        },
        media::{
            model::{AudioStream, MediaFile, VideoStream},
            process::MediaProcessFailure,
            progress::MediaProgress,
        },
    };

    use super::{
        AudioMixLifecycleUiState, CANCELLED_TITLE, COMPLETE_TITLE, FAILED_TITLE, RUNNING_TITLE,
        failure_content, failure_diagnostic, format_duration, format_integer, mixed_audio_summary,
        progress_presentation, source_to_output, video_copy_summary,
    };

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
            let mut state = AudioMixLifecycleUiState { details_open: true };

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

            let (expanded_rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let mut expanded_state = AudioMixLifecycleUiState { details_open: true };
            let expanded = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(expanded_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        crate::ui::layout::focus_content(ui, |ui| {
                            failure_content(ui, &failure, &mut expanded_state)
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
                            failure_content(ui, &failure, &mut compact_state)
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
            let mut state = AudioMixLifecycleUiState::default();

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
    fn diagnostic_details_are_retained_verbatim() {
        let failure = AudioMixExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: "stderr tail\nConversion failed!".to_owned(),
        };

        assert_eq!(
            failure_diagnostic(&failure),
            "stderr tail\nConversion failed!"
        );
    }

    #[test]
    fn missing_diagnostics_use_a_concise_fallback() {
        let failure = AudioMixExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: String::new(),
        };

        assert_eq!(
            failure_diagnostic(&failure),
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
            selected_streams: vec![1, 2],
            container: OutputContainer::Mp4,
        }
    }

    fn media() -> MediaFile {
        MediaFile {
            path: PathBuf::from("C:/media/recording.mkv"),
            duration: Some(Duration::from_secs(58)),
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
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
