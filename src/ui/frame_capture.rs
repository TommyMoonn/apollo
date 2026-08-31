use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use egui::{Align, Label, Layout, RichText, Sides, TextEdit, TextStyle, Ui};

use crate::{
    frame_capture::{
        model::{
            FrameCaptureFormat, FrameCaptureRequest, FrameTimestamp, FrameTimestampParseError,
        },
        validation::{
            FrameCaptureValidationError, validate_frame_capture_request, validate_frame_timestamp,
        },
    },
    media::{model::MediaFile, output::MediaOutputError},
};

use super::{fonts, layout, output_name::OutputNameState, theme, widgets};

const TIMESTAMP_LABEL: &str = "Timestamp";
const FORMAT_LABEL: &str = "Format";
const OUTPUT_LABEL: &str = "Output";
const BROWSE_LABEL: &str = "Browse";
const CAPTURE_LABEL: &str = "Capture frame";
const FORMAT_OPTIONS: [&str; 2] = ["PNG", "JPEG"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureSelectionState {
    timestamp_text: String,
    format: FrameCaptureFormat,
    output_name: OutputNameState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameCaptureSelectionAction {
    #[default]
    None,
    BrowseOutput,
    CaptureRequested,
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

    pub fn timestamp_text(&self) -> &str {
        &self.timestamp_text
    }

    pub fn set_timestamp_text(&mut self, media: &MediaFile, timestamp: String) {
        self.timestamp_text = timestamp;
        if let Ok(timestamp) = FrameTimestamp::from_str(&self.timestamp_text)
            && validate_frame_timestamp(timestamp, media.duration).is_ok()
        {
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

pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut FrameCaptureSelectionState,
) -> FrameCaptureSelectionAction {
    let mut action = FrameCaptureSelectionAction::None;
    let validation = state.validation(media);
    let feedback = validation.as_ref().err().and_then(ready_feedback);
    let plan = layout::task_surface_plan(
        ui.available_height(),
        fixed_flow_height(ui, media, feedback),
        0.0,
        0.0,
    );

    layout::task_surface(ui, plan.overflow, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;

        show_source_header(ui, media);
        ui.add_space(theme::SPACING.section);

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
        if let Some(duration) = media.duration {
            ui.add_space(theme::SPACING.tight);
            technical_context(ui, &format!("Duration {}", format_duration(duration)));
        }

        ui.add_space(theme::SPACING.focal);
        let _ = widgets::section_label(ui, FORMAT_LABEL);
        ui.add_space(theme::SPACING.compact);
        show_format_selector(ui, state);
        ui.add_space(theme::SPACING.tight);
        technical_context(ui, format_detail(state.format()));

        ui.add_space(theme::SPACING.focal);
        let _ = widgets::section_label(ui, OUTPUT_LABEL);
        ui.add_space(theme::SPACING.compact);
        let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
        let row = widgets::output_filename_row(
            ui,
            &mut output_stem,
            state.format().extension(),
            BROWSE_LABEL,
        );
        if row.filename.changed() {
            state.set_output_filename_stem(OsString::from(output_stem));
        }
        if row.browse.clicked() {
            action = FrameCaptureSelectionAction::BrowseOutput;
        }

        if let Some(feedback) = feedback {
            ui.add_space(theme::SPACING.tight);
            let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
        }

        ui.add_space(theme::SPACING.related);
        let response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::primary_button_enabled(ui, CAPTURE_LABEL, validation.is_ok())
            })
            .inner;
        if response.clicked() {
            action = FrameCaptureSelectionAction::CaptureRequested;
        }
    });

    action
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

fn show_source_header(ui: &mut Ui, media: &MediaFile) {
    let metadata = video_summary(media);
    let header_height = source_header_height(ui, metadata.is_some());
    Sides::new()
        .height(header_height.max(theme::COMPACT_CONTROL_HEIGHT))
        .spacing(theme::SOURCE_HEADER_GAP)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = theme::SPACING.tight;
                    ui.add(
                        Label::new(
                            RichText::new(source_file_name(&media.path))
                                .text_style(TextStyle::Body),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    if let Some(metadata) = metadata {
                        ui.add(
                            Label::new(
                                RichText::new(metadata)
                                    .text_style(fonts::technical_text_style())
                                    .color(theme::SECONDARY_FAINT),
                            )
                            .truncate()
                            .selectable(false),
                        );
                    }
                });
            },
            |_| {},
        );
}

fn fixed_flow_height(ui: &Ui, media: &MediaFile, feedback: Option<&str>) -> f32 {
    let section_label_height = ui.text_style_height(&TextStyle::Small);
    let output_row_height = widgets::output_filename_row_plan(ui, None, BROWSE_LABEL).flow_height();
    let feedback_height = feedback.map_or(0.0, |feedback| {
        theme::SPACING.tight + widgets::inline_status_height(ui, feedback, ui.available_width())
    });
    let duration_height = media.duration.map_or(0.0, |_| {
        theme::SPACING.tight + ui.text_style_height(&fonts::technical_text_style())
    });
    let format_detail_height =
        theme::SPACING.tight + ui.text_style_height(&fonts::technical_text_style());

    source_header_height(ui, video_summary(media).is_some())
        + theme::SPACING.section
        + section_label_height
        + theme::SPACING.compact
        + theme::COMPACT_CONTROL_HEIGHT
        + duration_height
        + theme::SPACING.focal
        + section_label_height
        + theme::SPACING.compact
        + theme::COMPACT_CONTROL_HEIGHT
        + format_detail_height
        + theme::SPACING.focal
        + section_label_height
        + theme::SPACING.compact
        + output_row_height
        + feedback_height
        + theme::SPACING.related
        + theme::PRIMARY_CONTROL_HEIGHT
}

fn source_header_height(ui: &Ui, has_metadata: bool) -> f32 {
    let metadata_height = if has_metadata {
        theme::SPACING.tight + ui.text_style_height(&fonts::technical_text_style())
    } else {
        0.0
    };
    (ui.text_style_height(&TextStyle::Body) + metadata_height).max(theme::COMPACT_CONTROL_HEIGHT)
}

fn source_file_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn video_summary(media: &MediaFile) -> Option<String> {
    let video = media.video_streams.first()?;
    match (video.width, video.height) {
        (Some(width), Some(height)) => Some(format!("{width}×{height}")),
        _ => Some("video stream".to_owned()),
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
        ui::{fonts, layout, theme},
    };

    use super::{
        FrameCaptureSelectionState, fixed_flow_height, format_detail, format_duration,
        ready_feedback,
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
        assert_eq!(
            state.request(&media).unwrap().timestamp.milliseconds(),
            1_250
        );
        assert!(state.can_capture(&media));
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-frame-00-00-01-250.png")
        );

        state.set_timestamp_text(&media, "one second".into());
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

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(ui, &media, &mut state);
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
        let mut state = FrameCaptureSelectionState::new(&media);

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(ui, &media, &mut state);
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

    #[test]
    fn constrained_ready_surface_scrolls_and_feedback_height_is_width_aware() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let output = context.run_ui(Default::default(), |ui| {
            let media = media(None);
            let fixed = fixed_flow_height(ui, &media, None);
            assert_eq!(
                layout::task_surface_plan(fixed - 1.0, fixed, 0.0, 0.0).overflow,
                layout::TaskOverflow::Scroll
            );
            assert_eq!(
                layout::task_surface_plan(fixed, fixed, 0.0, 0.0).overflow,
                layout::TaskOverflow::Flow
            );

            let feedback = "Enter a timestamp using HH:MM:SS.mmm.";
            let with_feedback = fixed_flow_height(ui, &media, Some(feedback));
            assert!(with_feedback > fixed);
        });
        output.drop_without_applying_deltas();
    }

    fn shape_contains(shape: &egui::Shape, expected: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text.contains(expected),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| shape_contains(shape, expected)),
            _ => false,
        }
    }
}
