use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use egui::{Align, Label, Layout, RichText, TextEdit, TextureId, Ui};

use crate::{
    frame_capture::{
        model::{
            FrameCaptureFormat, FrameCaptureRequest, FrameTimestamp, FrameTimestampParseError,
        },
        preview::{PreviewImage, PreviewState},
        validation::{
            FrameCaptureValidationError, validate_frame_capture_request, validate_frame_timestamp,
        },
    },
    media::{model::MediaFile, output::MediaOutputError},
};

use super::{
    ADD_TO_QUEUE_LABEL, OperationSurfaceMode, fonts, layout, media_error,
    output_name::OutputNameState, show_queue_confirmation, theme, widgets,
};

const TIMESTAMP_LABEL: &str = "Timestamp";
const OUTPUT_LABEL: &str = "Output";
const FORMAT_LABEL: &str = "Format";
const BROWSE_LABEL: &str = "Browse";
const CAPTURE_LABEL: &str = "Capture frame";
const FORMAT_OPTIONS: [&str; 2] = ["PNG", "JPEG"];
const PREVIEW_ALT_TEXT: &str = "Frame preview at the selected timestamp";
const PREVIEW_UPDATING_TEXT: &str = "Updating preview…";
const PREVIEW_FAILED_TEXT: &str = "Preview unavailable";
const PREVIEW_FALLBACK_SIZE: [usize; 2] = [640, 360];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivePreviewStatus {
    Idle,
    Updating,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameCaptureLivePreview {
    status: LivePreviewStatus,
    texture: Option<(TextureId, [usize; 2])>,
}

impl FrameCaptureLivePreview {
    pub fn from_preview(
        state: &PreviewState<PreviewImage>,
        retained_texture: Option<(TextureId, [usize; 2])>,
    ) -> Self {
        let status = match state {
            PreviewState::Idle => LivePreviewStatus::Idle,
            PreviewState::Debouncing { .. } | PreviewState::Loading { .. } => {
                LivePreviewStatus::Updating
            }
            PreviewState::Ready { .. } => LivePreviewStatus::Ready,
            PreviewState::Failed { .. } => LivePreviewStatus::Failed,
        };
        let texture = matches!(
            status,
            LivePreviewStatus::Idle | LivePreviewStatus::Updating | LivePreviewStatus::Ready
        )
        .then_some(retained_texture)
        .flatten();
        Self { status, texture }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureSelectionState {
    timestamp_text: String,
    format: FrameCaptureFormat,
    output_name: OutputNameState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCapturePresentation {
    duration_context: Option<String>,
}

impl FrameCapturePresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            duration_context: media
                .duration
                .map(|duration| format!("Duration {}", format_duration(duration))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameCaptureSelectionAction {
    #[default]
    None,
    BrowseOutput,
    QueueRequested,
    CaptureRequested,
    SaveQueueChanges,
    CancelQueueEdit,
}

impl FrameCaptureSelectionState {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            timestamp_text: FrameTimestamp::ZERO.to_string(),
            format: FrameCaptureFormat::Png,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(&media.path, FrameTimestamp::ZERO),
            ),
        }
    }

    pub fn from_request(media: &MediaFile, request: &FrameCaptureRequest) -> Self {
        let mut state = Self {
            timestamp_text: request.timestamp.to_string(),
            format: request.format,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(&media.path, request.timestamp),
            ),
        };
        state.output_name.set_custom_path(request.output.clone());
        state
    }

    pub fn timestamp_text(&self) -> &str {
        &self.timestamp_text
    }

    pub fn valid_timestamp(&self, media: &MediaFile) -> Option<FrameTimestamp> {
        let timestamp = FrameTimestamp::from_str(&self.timestamp_text).ok()?;
        validate_frame_timestamp(timestamp, media.duration)
            .is_ok()
            .then_some(timestamp)
    }

    pub fn set_timestamp_text(&mut self, media: &MediaFile, timestamp: String) {
        self.timestamp_text = timestamp;
        if let Some(timestamp) = self.valid_timestamp(media) {
            self.output_name
                .set_generated_stem(default_output_stem(&media.path, timestamp));
        }
    }

    pub fn format(&self) -> FrameCaptureFormat {
        self.format
    }

    pub fn set_format(&mut self, format: FrameCaptureFormat) {
        self.format = format;
    }

    pub fn output_path(&self) -> PathBuf {
        self.output_name.path(self.format.extension())
    }

    pub fn output_filename_stem(&self) -> &OsStr {
        self.output_name.stem()
    }

    pub fn output_is_customized(&self) -> bool {
        self.output_name.is_customized()
    }

    pub fn set_output_filename_stem(&mut self, stem: OsString) {
        self.output_name.set_stem(stem);
    }

    pub fn set_custom_output_path(&mut self, output: PathBuf) {
        self.output_name.set_custom_path(output);
    }

    pub fn request(&self, media: &MediaFile) -> Option<FrameCaptureRequest> {
        Some(FrameCaptureRequest {
            input: media.path.clone(),
            output: self.output_path(),
            timestamp: FrameTimestamp::from_str(&self.timestamp_text).ok()?,
            format: self.format,
        })
    }

    pub fn can_capture(&self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    fn validation(&self, media: &MediaFile) -> Result<(), FrameCaptureReadyError> {
        let timestamp = FrameTimestamp::from_str(&self.timestamp_text)
            .map_err(FrameCaptureReadyError::Timestamp)?;
        let request = FrameCaptureRequest {
            input: media.path.clone(),
            output: self.output_path(),
            timestamp,
            format: self.format,
        };
        validate_frame_capture_request(&request, media).map_err(FrameCaptureReadyError::Request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameCaptureReadyError {
    Timestamp(FrameTimestampParseError),
    Request(FrameCaptureValidationError),
}

pub fn show<'a>(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &FrameCapturePresentation,
    state: &mut FrameCaptureSelectionState,
    live_preview: Option<FrameCaptureLivePreview>,
    mode: impl Into<OperationSurfaceMode<'a>>,
) -> FrameCaptureSelectionAction {
    let mode = mode.into();
    let shared_defaults = matches!(mode, OperationSurfaceMode::BatchDefaults { .. });
    let action = Cell::new(FrameCaptureSelectionAction::None);
    let validation = state.validation(media);
    let feedback = validation.as_ref().err().and_then(ready_feedback);

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            let _ = widgets::section_label(ui, TIMESTAMP_LABEL);
            ui.add_space(theme::SPACING.compact);
            let mut timestamp_text = state.timestamp_text().to_owned();
            let timestamp = ui.add_sized(
                [ui.available_width(), theme::COMPACT_CONTROL_HEIGHT],
                TextEdit::singleline(&mut timestamp_text)
                    .hint_text("HH:MM:SS.mmm")
                    .vertical_align(Align::Center),
            );
            if timestamp.changed() {
                state.set_timestamp_text(media, timestamp_text);
            }
            if timestamp.lost_focus() {
                ui.ctx().request_repaint();
            }
            if let Some(duration_context) = presentation.duration_context.as_deref() {
                ui.add_space(theme::SPACING.tight);
                technical_context(ui, duration_context);
            }

            if let Some(live_preview) = live_preview {
                ui.add_space(theme::SPACING.related);
                show_live_preview(ui, media, live_preview);
            }

            ui.add_space(theme::SPACING.section);
            let _ = widgets::section_label(
                ui,
                if shared_defaults {
                    FORMAT_LABEL
                } else {
                    OUTPUT_LABEL
                },
            );
            ui.add_space(theme::SPACING.compact);
            if shared_defaults {
                show_format_selector(ui, state);
            } else {
                let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
                let selector_width = widgets::select_menu_intrinsic_width(ui, &FORMAT_OPTIONS);
                let row = widgets::operation_output_row(
                    ui,
                    &mut output_stem,
                    state.format().extension(),
                    BROWSE_LABEL,
                    selector_width,
                    |ui| show_format_selector(ui, state),
                );
                if row.filename.changed() {
                    state.set_output_filename_stem(OsString::from(output_stem));
                }
                if row.browse.clicked() {
                    action.set(FrameCaptureSelectionAction::BrowseOutput);
                }
            }
            ui.add_space(theme::SPACING.tight);
            technical_context(ui, format_detail(state.format()));

            if let Some(feedback) = feedback {
                ui.add_space(theme::SPACING.tight);
                let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
            }
            if let Some(error) = mode.inline_action_error() {
                ui.add_space(theme::SPACING.related);
                media_error::show_operation_action(ui, error);
            }
        },
        |ui| match mode {
            OperationSurfaceMode::Immediate {
                start_allowed,
                queue_confirmation,
                ..
            } => {
                if widgets::primary_button_enabled(
                    ui,
                    CAPTURE_LABEL,
                    validation.is_ok() && start_allowed,
                )
                .clicked()
                {
                    action.set(FrameCaptureSelectionAction::CaptureRequested);
                }
                if ui
                    .add_enabled_ui(validation.is_ok(), |ui| {
                        widgets::secondary_button(ui, ADD_TO_QUEUE_LABEL)
                    })
                    .inner
                    .clicked()
                {
                    action.set(FrameCaptureSelectionAction::QueueRequested);
                }
                show_queue_confirmation(ui, queue_confirmation);
            }
            edit_mode => {
                let (save_label, cancel_label) = edit_mode
                    .edit_labels()
                    .expect("non-immediate operation mode has edit labels");
                let save_allowed = match edit_mode {
                    OperationSurfaceMode::BatchDefaults { save_allowed } => save_allowed,
                    _ => validation.is_ok(),
                };
                if widgets::primary_button_enabled(ui, save_label, save_allowed).clicked() {
                    action.set(FrameCaptureSelectionAction::SaveQueueChanges);
                }
                if widgets::secondary_button(ui, cancel_label).clicked() {
                    action.set(FrameCaptureSelectionAction::CancelQueueEdit);
                }
            }
        },
    );

    action.get()
}

fn show_live_preview(ui: &mut Ui, media: &MediaFile, preview: FrameCaptureLivePreview) {
    let source_size = media
        .video_streams
        .first()
        .and_then(|video| Some([video.width? as usize, video.height? as usize]))
        .unwrap_or(PREVIEW_FALLBACK_SIZE);
    let image_size = preview
        .texture
        .map_or(source_size, |(_, texture_size)| texture_size);
    let display_size = layout::live_preview_size(ui.available_width(), image_size);

    match preview.status {
        LivePreviewStatus::Updating => technical_context(ui, PREVIEW_UPDATING_TEXT),
        LivePreviewStatus::Failed => technical_context(ui, PREVIEW_FAILED_TEXT),
        LivePreviewStatus::Idle | LivePreviewStatus::Ready => {
            ui.add_space(ui.text_style_height(&fonts::technical_text_style()));
        }
    }
    ui.add_space(theme::SPACING.tight);

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        let texture = preview.texture.map(|(texture, _)| texture);
        let _ = widgets::framed_preview(ui, texture, display_size, PREVIEW_ALT_TEXT);
    });
}

fn show_format_selector(ui: &mut Ui, state: &mut FrameCaptureSelectionState) {
    let mut requested = None;
    widgets::select_menu(ui, format_label(state.format()), &FORMAT_OPTIONS, |ui| {
        for format in [FrameCaptureFormat::Png, FrameCaptureFormat::Jpeg] {
            if widgets::select_menu_option(ui, format_label(format), state.format() == format)
                .clicked()
            {
                requested = Some(format);
                ui.close();
            }
        }
    });
    if let Some(format) = requested {
        state.set_format(format);
    }
}

fn ready_feedback(error: &FrameCaptureReadyError) -> Option<&'static str> {
    match error {
        FrameCaptureReadyError::Timestamp(_) => Some("Enter a timestamp using HH:MM:SS.mmm."),
        FrameCaptureReadyError::Request(FrameCaptureValidationError::MissingVideoStream) => {
            Some("This source has no video stream to capture.")
        }
        FrameCaptureReadyError::Request(FrameCaptureValidationError::NegativeTimestamp) => {
            Some("Timestamp cannot be negative.")
        }
        FrameCaptureReadyError::Request(
            FrameCaptureValidationError::TimestampAtOrBeyondDuration { .. },
        ) => Some("Timestamp must be before the source duration."),
        FrameCaptureReadyError::Request(
            FrameCaptureValidationError::OutputExtensionMismatch { actual: None, .. }
            | FrameCaptureValidationError::Output(MediaOutputError::InvalidOutputPath(_)),
        ) => Some("Enter a usable output filename."),
        FrameCaptureReadyError::Request(FrameCaptureValidationError::OutputExtensionMismatch {
            ..
        }) => Some("Output filename must match the selected format."),
        FrameCaptureReadyError::Request(FrameCaptureValidationError::Output(_)) => None,
    }
}

fn default_output_stem(input: &Path, timestamp: FrameTimestamp) -> OsString {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));
    let mut output = OsString::from(stem);
    output.push("-frame");
    output.push("-");
    output.push(timestamp.filename_component());
    output
}

pub(super) fn format_label(format: FrameCaptureFormat) -> &'static str {
    match format {
        FrameCaptureFormat::Png => "PNG",
        FrameCaptureFormat::Jpeg => "JPEG",
    }
}

fn format_detail(format: FrameCaptureFormat) -> &'static str {
    match format {
        FrameCaptureFormat::Png => "lossless",
        FrameCaptureFormat::Jpeg => "high quality",
    }
}

fn technical_context(ui: &mut Ui, text: &str) {
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(fonts::technical_text_style())
                .color(theme::SECONDARY_FAINT),
        )
        .selectable(false),
    );
}

fn format_duration(duration: Duration) -> String {
    let total_milliseconds = duration.as_millis();
    let hours = total_milliseconds / 3_600_000;
    let minutes = total_milliseconds % 3_600_000 / 60_000;
    let seconds = total_milliseconds % 60_000 / 1_000;
    let milliseconds = total_milliseconds % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}")
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf, time::Duration};

    use crate::{
        frame_capture::model::FrameCaptureFormat,
        media::model::{MediaFile, MediaStream, StreamKind, VideoStream},
        ui::{fonts, theme},
    };

    use super::{
        FrameCaptureLivePreview, FrameCapturePresentation, FrameCaptureSelectionState,
        LivePreviewStatus, format_detail, format_duration, ready_feedback,
    };

    fn media(duration: Option<Duration>) -> MediaFile {
        MediaFile {
            path: PathBuf::from("captures/recording.mkv"),
            duration,
            streams: vec![MediaStream {
                stream_index: 7,
                kind: StreamKind::Video,
                codec: Some("h264".into()),
            }],
            video_streams: vec![VideoStream {
                stream_index: 7,
                codec: Some("h264".into()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn defaults_to_zero_png_and_timestamped_frame_suffix() {
        let media = media(Some(Duration::from_secs(10)));
        let state = FrameCaptureSelectionState::new(&media);
        assert_eq!(state.timestamp_text(), "00:00:00.000");
        assert_eq!(state.format(), FrameCaptureFormat::Png);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-frame-00-00-00-000.png")
        );
        assert!(state.can_capture(&media));
        assert_eq!(state.request(&media).unwrap().timestamp.milliseconds(), 0);
    }

    #[test]
    fn timestamp_edits_drive_request_and_invalid_feedback() {
        let media = media(Some(Duration::from_secs(10)));
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_timestamp_text(&media, "00:00:01.250".into());
        let preview_timestamp = state.valid_timestamp(&media).unwrap();
        assert_eq!(
            state.request(&media).unwrap().timestamp.milliseconds(),
            1_250
        );
        assert_eq!(state.request(&media).unwrap().timestamp, preview_timestamp);
        assert!(state.can_capture(&media));
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-frame-00-00-01-250.png")
        );

        state.set_timestamp_text(&media, "one second".into());
        assert!(state.valid_timestamp(&media).is_none());
        assert!(state.request(&media).is_none());
        assert!(!state.can_capture(&media));
        assert_eq!(
            ready_feedback(&state.validation(&media).unwrap_err()),
            Some("Enter a timestamp using HH:MM:SS.mmm.")
        );
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-frame-00-00-01-250.png")
        );
    }

    #[test]
    fn invalid_timestamp_feedback_is_rendered_inline() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut state = FrameCaptureSelectionState::new(&media(None));
        let media = media(None);
        state.set_timestamp_text(&media, "not a timestamp".into());
        let presentation = FrameCapturePresentation::new(&media);

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(ui, &media, &presentation, &mut state, None, true);
        });
        assert!(output.shapes.iter().any(|shape| {
            shape_contains(&shape.shape, "Enter a timestamp using HH:MM:SS.mmm.")
        }));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn format_changes_extension_and_preserves_custom_stem() {
        let media = media(None);
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_output_filename_stem(OsString::from("chosen frame"));
        state.set_timestamp_text(&media, "00:00:04.250".into());
        state.set_format(FrameCaptureFormat::Jpeg);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/chosen frame.jpg")
        );
        state.set_format(FrameCaptureFormat::Png);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/chosen frame.png")
        );
        assert!(state.output_is_customized());
    }

    #[test]
    fn out_of_duration_timestamp_preserves_the_last_valid_generated_name() {
        let media = media(Some(Duration::from_secs(2)));
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_timestamp_text(&media, "00:00:01.500".into());
        let valid_output = state.output_path();

        state.set_timestamp_text(&media, "00:00:02.000".into());

        assert!(!state.can_capture(&media));
        assert_eq!(state.output_path(), valid_output);
    }

    #[test]
    fn known_duration_and_format_detail_render_as_compact_context() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let media = media(Some(Duration::from_millis(754_567)));
        let presentation = FrameCapturePresentation::new(&media);
        let mut state = FrameCaptureSelectionState::new(&media);

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(ui, &media, &presentation, &mut state, None, true);
        });

        assert!(
            output
                .shapes
                .iter()
                .any(|shape| { shape_contains(&shape.shape, "Duration 00:12:34.567") })
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| shape_contains(&shape.shape, "lossless"))
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn updating_preview_uses_restrained_status() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let media = media(Some(Duration::from_secs(10)));
        let presentation = FrameCapturePresentation::new(&media);
        let mut state = FrameCaptureSelectionState::new(&media);
        let preview = FrameCaptureLivePreview {
            status: LivePreviewStatus::Updating,
            texture: None,
        };

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(ui, &media, &presentation, &mut state, Some(preview), true);
        });

        assert!(
            output
                .shapes
                .iter()
                .any(|shape| { shape_contains(&shape.shape, super::PREVIEW_UPDATING_TEXT) })
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn idle_validation_state_keeps_the_last_valid_preview_texture() {
        let retained = (egui::TextureId::Managed(7), [640, 360]);
        let preview = FrameCaptureLivePreview::from_preview(
            &crate::frame_capture::preview::PreviewState::Idle,
            Some(retained),
        );

        assert_eq!(preview.status, LivePreviewStatus::Idle);
        assert_eq!(preview.texture, Some(retained));
    }

    #[test]
    fn preview_failure_is_feedback_only_and_does_not_change_capture_request() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let media = media(Some(Duration::from_secs(10)));
        let presentation = FrameCapturePresentation::new(&media);
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_timestamp_text(&media, "00:00:01.250".into());
        let request_before = state.request(&media).unwrap();
        let preview = FrameCaptureLivePreview {
            status: LivePreviewStatus::Failed,
            texture: None,
        };

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(ui, &media, &presentation, &mut state, Some(preview), true);
        });

        assert!(
            output
                .shapes
                .iter()
                .any(|shape| { shape_contains(&shape.shape, super::PREVIEW_FAILED_TEXT) })
        );
        assert!(state.can_capture(&media));
        assert_eq!(state.request(&media).unwrap(), request_before);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn duration_and_format_copy_match_fixed_policies() {
        assert_eq!(
            format_duration(Duration::from_millis(3_723_004)),
            "01:02:03.004"
        );
        assert_eq!(format_detail(FrameCaptureFormat::Png), "lossless");
        assert_eq!(format_detail(FrameCaptureFormat::Jpeg), "high quality");
    }

    #[test]
    fn browse_path_preserves_directory_while_format_owns_extension() {
        let media = media(None);
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/still.jpg"));
        assert_eq!(state.output_path(), PathBuf::from("exports/still.png"));
        state.set_format(FrameCaptureFormat::Jpeg);
        assert_eq!(state.output_path(), PathBuf::from("exports/still.jpg"));
    }

    #[test]
    fn duration_and_missing_video_validation_control_primary_action() {
        let mut media = media(Some(Duration::from_millis(500)));
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_timestamp_text(&media, "00:00:00.501".into());
        assert!(!state.can_capture(&media));

        state.set_timestamp_text(&media, "00:00:00.000".into());
        media.video_streams.clear();
        assert!(!state.can_capture(&media));
    }

    fn shape_contains(shape: &egui::Shape, expected: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text.contains(expected),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| shape_contains(shape, expected)),
            _ => false,
        }
    }
}
