use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use egui::{Align, Label, Layout, RichText, TextEdit, TextureId, Ui, WidgetInfo, WidgetType};

use crate::{
    frame_capture::{
        frame_index::{FrameDirection, FrameStep},
        model::{
            FrameCaptureFormat, FrameCaptureRequest, FramePosition, FrameTimestamp,
            FrameTimestampParseError,
        },
        preview::{PreviewImage, PreviewState},
        validation::{
            FrameCaptureValidationError, validate_frame_capture_request, validate_frame_position,
            validate_frame_timestamp,
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
const FRAME_INDEX_LOADING_TEXT: &str = "Finding nearby frames…";
const FRAME_INDEX_FAILED_TEXT: &str = "Frame navigation unavailable";
const PREVIEW_FALLBACK_SIZE: [usize; 2] = [640, 360];
const STEP_OPTIONS: [&str; 4] = ["1 frame", "2 frames", "5 frames", "10 frames"];

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
    texture: Option<(TextureId, [usize; 2], FramePosition)>,
}

impl FrameCaptureLivePreview {
    pub fn from_preview(
        state: &PreviewState<PreviewImage>,
        retained_texture: Option<(TextureId, [usize; 2], FramePosition)>,
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
    position: FramePosition,
    step: FrameStep,
    format: FrameCaptureFormat,
    output_name: OutputNameState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCapturePresentation {
    duration: Option<String>,
}

impl FrameCapturePresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            duration: media.duration.map(format_duration),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameNavigationAvailability {
    pub previous: bool,
    pub next: bool,
    pub loading: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameCapturePositionSurface {
    Simple,
    Precision {
        preview: FrameCaptureLivePreview,
        navigation: FrameNavigationAvailability,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionSettle {
    Debounced,
    Immediate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameCaptureSelectionAction {
    #[default]
    None,
    BrowseOutput,
    QueueRequested,
    CaptureRequested,
    PositionEdited(PositionSettle),
    StepRequested(FrameDirection),
    SaveQueueChanges,
    CancelQueueEdit,
}

impl FrameCaptureSelectionState {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            timestamp_text: FrameTimestamp::ZERO.to_string(),
            position: FrameTimestamp::ZERO.into(),
            step: FrameStep::One,
            format: FrameCaptureFormat::Png,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(&media.path, FrameTimestamp::ZERO),
            ),
        }
    }

    pub fn from_request(media: &MediaFile, request: &FrameCaptureRequest) -> Self {
        let mut state = Self {
            timestamp_text: request.position.to_string(),
            position: request.position,
            step: FrameStep::One,
            format: request.format,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(&media.path, request.position.display_timestamp()),
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

    pub fn valid_position(&self, media: &MediaFile) -> Option<FramePosition> {
        let timestamp = FrameTimestamp::from_str(&self.timestamp_text).ok()?;
        validate_frame_timestamp(timestamp, media.duration).ok()?;
        validate_frame_position(self.position, media.duration)
            .is_ok()
            .then_some(self.position)
    }

    pub fn set_timestamp_text(&mut self, media: &MediaFile, timestamp: String) {
        self.timestamp_text = timestamp;
        if let Some(timestamp) = self.valid_timestamp(media) {
            self.position = timestamp.into();
            self.output_name
                .set_generated_stem(default_output_stem(&media.path, timestamp));
        }
    }

    pub fn set_position(&mut self, media: &MediaFile, position: FramePosition) {
        self.position = position;
        let timestamp = position.display_timestamp();
        self.timestamp_text = timestamp.to_string();
        self.output_name
            .set_generated_stem(default_output_stem(&media.path, timestamp));
    }

    pub const fn step(&self) -> FrameStep {
        self.step
    }

    pub fn set_step(&mut self, step: FrameStep) {
        self.step = step;
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
        let position = self.valid_position(media)?;
        Some(FrameCaptureRequest {
            input: media.path.clone(),
            output: self.output_path(),
            position,
            format: self.format,
        })
    }

    pub fn can_capture(&self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    fn validation(&self, media: &MediaFile) -> Result<(), FrameCaptureReadyError> {
        let timestamp = FrameTimestamp::from_str(&self.timestamp_text)
            .map_err(FrameCaptureReadyError::Timestamp)?;
        validate_frame_timestamp(timestamp, media.duration)
            .map_err(FrameCaptureReadyError::Request)?;
        let request = FrameCaptureRequest {
            input: media.path.clone(),
            output: self.output_path(),
            position: self.position,
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
    position_surface: FrameCapturePositionSurface,
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

            match position_surface {
                FrameCapturePositionSurface::Simple => {
                    show_legacy_timestamp(ui, media, presentation, state);
                }
                FrameCapturePositionSurface::Precision {
                    preview,
                    navigation,
                } => {
                    show_live_preview(ui, media, preview, state.valid_position(media));
                    ui.add_space(theme::SPACING.related);
                    show_precision_position(ui, media, presentation, state, navigation, &action);
                }
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

fn show_live_preview(
    ui: &mut Ui,
    media: &MediaFile,
    preview: FrameCaptureLivePreview,
    current: Option<FramePosition>,
) -> egui::Response {
    let source_size = media
        .video_streams
        .first()
        .and_then(|video| Some([video.width? as usize, video.height? as usize]))
        .unwrap_or(PREVIEW_FALLBACK_SIZE);
    let image_size = preview
        .texture
        .map_or(source_size, |(_, texture_size, _)| texture_size);
    let display_size = layout::live_preview_size(ui.available_width(), image_size);
    let retained_position = preview.texture.map(|(_, _, position)| position);
    let retained_is_current = retained_position.is_some_and(|position| Some(position) == current);

    let status_height = ui.text_style_height(&fonts::technical_text_style());
    let (status_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), status_height),
        egui::Sense::hover(),
    );
    match preview.status {
        LivePreviewStatus::Updating => {
            paint_technical_context(ui, status_rect, PREVIEW_UPDATING_TEXT);
        }
        LivePreviewStatus::Failed => {
            paint_technical_context(ui, status_rect, PREVIEW_FAILED_TEXT);
        }
        LivePreviewStatus::Idle if retained_position.is_some() && !retained_is_current => {
            let text = format!(
                "Last preview: {}",
                retained_position.expect("retained preview position checked above")
            );
            paint_technical_context(ui, status_rect, &text);
        }
        LivePreviewStatus::Idle | LivePreviewStatus::Ready => {}
    }
    ui.add_space(theme::SPACING.tight);

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        let texture = preview.texture.map(|(texture, _, _)| texture);
        widgets::framed_preview(ui, texture, display_size, PREVIEW_ALT_TEXT)
    })
    .inner
}

fn show_precision_position(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &FrameCapturePresentation,
    state: &mut FrameCaptureSelectionState,
    navigation: FrameNavigationAvailability,
    action: &Cell<FrameCaptureSelectionAction>,
) {
    let mut timestamp_text = state.timestamp_text().to_owned();
    let (timestamp, _) = layout::frame_position_row(
        ui,
        |ui| {
            let response = ui.add_sized(
                [
                    layout::FRAME_POSITION_EDITOR_WIDTH.min(ui.available_width()),
                    theme::COMPACT_CONTROL_HEIGHT,
                ],
                TextEdit::singleline(&mut timestamp_text)
                    .font(fonts::technical_text_style())
                    .hint_text("HH:MM:SS.mmm")
                    .vertical_align(Align::Center),
            );
            response.widget_info(|| {
                WidgetInfo::labeled(
                    WidgetType::TextEdit,
                    response.enabled(),
                    "Current timestamp",
                )
            });
            response
        },
        |ui| {
            if let Some(duration) = presentation.duration.as_deref() {
                technical_context(ui, duration);
            }
        },
    );
    if timestamp.changed() {
        state.set_timestamp_text(media, timestamp_text);
        action.set(FrameCaptureSelectionAction::PositionEdited(
            PositionSettle::Debounced,
        ));
    }

    if let Some(maximum) = media.duration.and_then(scrubber_maximum) {
        ui.add_space(theme::SPACING.compact);
        let mut milliseconds = state
            .valid_position(media)
            .map_or(0, |position| position.display_timestamp().milliseconds())
            .clamp(0, maximum);
        let scrubber = widgets::frame_position_scrubber(ui, &mut milliseconds, maximum);
        if scrubber.changed() {
            state.set_position(
                media,
                FrameTimestamp::from_milliseconds(milliseconds).into(),
            );
        }
        if let Some(settle) = scrubber_settle(
            scrubber.changed(),
            scrubber.dragged(),
            scrubber.drag_stopped(),
        ) {
            action.set(FrameCaptureSelectionAction::PositionEdited(settle));
        }
    }

    ui.add_space(theme::SPACING.related);
    let action_sizes = [
        widgets::compact_secondary_button_intrinsic_size(ui, "−"),
        egui::vec2(
            widgets::select_menu_intrinsic_width(ui, &STEP_OPTIONS),
            theme::COMPACT_CONTROL_HEIGHT,
        ),
        widgets::compact_secondary_button_intrinsic_size(ui, "+"),
    ];
    layout::centered_action_group(
        ui,
        &action_sizes,
        layout::ActionLayout::Horizontal,
        theme::SPACING.compact,
        |ui| {
            let previous = widgets::compact_secondary_button_enabled(
                ui,
                "−",
                navigation.previous && !navigation.loading,
            );
            show_step_selector(ui, state);
            let next = widgets::compact_secondary_button_enabled(
                ui,
                "+",
                navigation.next && !navigation.loading,
            );
            if previous.clicked() {
                action.set(FrameCaptureSelectionAction::StepRequested(
                    FrameDirection::Previous,
                ));
            }
            if next.clicked() {
                action.set(FrameCaptureSelectionAction::StepRequested(
                    FrameDirection::Next,
                ));
            }
        },
    );
    if navigation.loading || navigation.failed {
        ui.add_space(theme::SPACING.tight);
        technical_context(
            ui,
            if navigation.loading {
                FRAME_INDEX_LOADING_TEXT
            } else {
                FRAME_INDEX_FAILED_TEXT
            },
        );
    }
}

fn show_legacy_timestamp(
    ui: &mut Ui,
    media: &MediaFile,
    presentation: &FrameCapturePresentation,
    state: &mut FrameCaptureSelectionState,
) {
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
    if let Some(duration) = presentation.duration.as_deref() {
        ui.add_space(theme::SPACING.tight);
        technical_context(ui, &format!("Duration {duration}"));
    }
}

fn show_step_selector(ui: &mut Ui, state: &mut FrameCaptureSelectionState) {
    let mut requested = None;
    widgets::select_menu(ui, step_label(state.step()), &STEP_OPTIONS, |ui| {
        for step in [
            FrameStep::One,
            FrameStep::Two,
            FrameStep::Five,
            FrameStep::Ten,
        ] {
            if widgets::select_menu_option(ui, step_label(step), state.step() == step).clicked() {
                requested = Some(step);
                ui.close();
            }
        }
    });
    if let Some(step) = requested {
        state.set_step(step);
    }
}

fn step_label(step: FrameStep) -> &'static str {
    match step {
        FrameStep::One => "1 frame",
        FrameStep::Two => "2 frames",
        FrameStep::Five => "5 frames",
        FrameStep::Ten => "10 frames",
    }
}

fn scrubber_maximum(duration: Duration) -> Option<i64> {
    let last_nanosecond = duration.as_nanos().checked_sub(1)?;
    i64::try_from(last_nanosecond / 1_000_000).ok()
}

fn scrubber_settle(
    changed: bool,
    pointer_held: bool,
    pointer_released: bool,
) -> Option<PositionSettle> {
    if pointer_released || (changed && !pointer_held) {
        Some(PositionSettle::Immediate)
    } else if changed {
        Some(PositionSettle::Debounced)
    } else {
        None
    }
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
    ui.add(technical_context_label(text));
}

fn technical_context_label(text: &str) -> Label {
    Label::new(
        RichText::new(text)
            .text_style(fonts::technical_text_style())
            .color(theme::SECONDARY_FAINT),
    )
    .selectable(false)
}

fn paint_technical_context(ui: &Ui, rect: egui::Rect, text: &str) {
    let font_id = ui.style().text_styles[&fonts::technical_text_style()].clone();
    ui.painter().text(
        rect.left_top(),
        egui::Align2::LEFT_TOP,
        text,
        font_id,
        theme::SECONDARY_FAINT,
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
        frame_capture::{
            frame_index::FrameStep,
            model::{
                FrameCaptureFormat, FramePosition, FrameTimeBase, FrameTimestamp,
                ResolvedFramePosition,
            },
        },
        media::model::{MediaFile, MediaStream, StreamKind, VideoStream},
        ui::{fonts, theme},
    };

    use super::{
        FrameCaptureLivePreview, FrameCapturePositionSurface, FrameCapturePresentation,
        FrameCaptureSelectionState, FrameNavigationAvailability, LivePreviewStatus, PositionSettle,
        format_detail, format_duration, ready_feedback, scrubber_maximum, scrubber_settle,
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
        assert_eq!(
            state
                .request(&media)
                .unwrap()
                .position
                .display_timestamp()
                .milliseconds(),
            0
        );
    }

    #[test]
    fn timestamp_edits_drive_request_and_invalid_feedback() {
        let media = media(Some(Duration::from_secs(10)));
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_timestamp_text(&media, "00:00:01.250".into());
        let preview_timestamp = state.valid_timestamp(&media).unwrap();
        assert_eq!(
            state
                .request(&media)
                .unwrap()
                .position
                .display_timestamp()
                .milliseconds(),
            1_250
        );
        assert_eq!(
            state.request(&media).unwrap().position.display_timestamp(),
            preview_timestamp
        );
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
    fn resolved_frame_identity_survives_presentation_and_capture_request() {
        let media = media(Some(Duration::from_secs(10)));
        let position = ResolvedFramePosition::new(
            1_001,
            FrameTimeBase::new(1, 30_000).expect("valid time base"),
        )
        .expect("displayable frame position");
        let request = crate::frame_capture::model::FrameCaptureRequest {
            input: media.path.clone(),
            output: PathBuf::from("captures/frame.png"),
            position: position.into(),
            format: FrameCaptureFormat::Png,
        };

        let state = FrameCaptureSelectionState::from_request(&media, &request);

        assert_eq!(state.timestamp_text(), "00:00:00.033");
        assert_eq!(state.request(&media).unwrap().position, position.into());
        assert_eq!(state.step(), FrameStep::One);
    }

    #[test]
    fn scrubber_range_uses_the_last_valid_millisecond_before_duration() {
        assert_eq!(scrubber_maximum(Duration::from_secs(10)), Some(9_999));
        assert_eq!(
            scrubber_maximum(Duration::from_nanos(10_000_500_000)),
            Some(10_000)
        );
        assert_eq!(scrubber_maximum(Duration::ZERO), None);
    }

    #[test]
    fn scrubber_movement_debounces_but_click_and_release_settle_immediately() {
        assert_eq!(
            scrubber_settle(true, true, false),
            Some(PositionSettle::Debounced)
        );
        assert_eq!(
            scrubber_settle(true, false, false),
            Some(PositionSettle::Immediate)
        );
        assert_eq!(
            scrubber_settle(false, false, true),
            Some(PositionSettle::Immediate)
        );
        assert_eq!(scrubber_settle(false, true, false), None);
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
            let _ = super::show(
                ui,
                &media,
                &presentation,
                &mut state,
                FrameCapturePositionSurface::Simple,
                true,
            );
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
            let _ = super::show(
                ui,
                &media,
                &presentation,
                &mut state,
                FrameCapturePositionSurface::Simple,
                true,
            );
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
            let _ = super::show(
                ui,
                &media,
                &presentation,
                &mut state,
                FrameCapturePositionSurface::Precision {
                    preview,
                    navigation: FrameNavigationAvailability {
                        previous: false,
                        next: false,
                        loading: true,
                        failed: false,
                    },
                },
                true,
            );
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
    fn updating_a_retained_preview_keeps_the_preview_layout_stable() {
        fn preview_top(status: LivePreviewStatus, current: FramePosition) -> f32 {
            let context = egui::Context::default();
            fonts::install(&context);
            theme::install(&context);
            let media = media(Some(Duration::from_secs(10)));
            let retained_position = FrameTimestamp::from_milliseconds(1_250).into();
            let preview = FrameCaptureLivePreview {
                status,
                texture: Some((egui::TextureId::Managed(9), [640, 360], retained_position)),
            };
            let top = std::cell::Cell::new(0.0);

            let output = context.run_ui(Default::default(), |ui| {
                let response = super::show_live_preview(ui, &media, preview, Some(current));
                top.set(response.rect.top());
            });
            output.drop_without_applying_deltas();
            top.get()
        }

        let retained_position = FrameTimestamp::from_milliseconds(1_250).into();
        let requested_position = FrameTimestamp::from_milliseconds(2_000).into();
        assert_eq!(
            preview_top(LivePreviewStatus::Updating, requested_position),
            preview_top(LivePreviewStatus::Ready, retained_position)
        );
    }

    #[test]
    fn precision_surface_uses_one_current_position_and_one_duration_value() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let media = media(Some(Duration::from_millis(10_250)));
        let presentation = FrameCapturePresentation::new(&media);
        let mut state = FrameCaptureSelectionState::new(&media);
        let preview = FrameCaptureLivePreview {
            status: LivePreviewStatus::Idle,
            texture: None,
        };

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(
                ui,
                &media,
                &presentation,
                &mut state,
                FrameCapturePositionSurface::Precision {
                    preview,
                    navigation: FrameNavigationAvailability {
                        previous: false,
                        next: true,
                        loading: false,
                        failed: false,
                    },
                },
                true,
            );
        });

        assert!(
            output
                .shapes
                .iter()
                .any(|shape| { shape_contains(&shape.shape, "00:00:10.250") })
        );
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| { shape_contains(&shape.shape, "Duration 00:00:10.250") })
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn invalid_current_text_labels_a_retained_image_as_the_last_preview() {
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let media = media(Some(Duration::from_secs(10)));
        let presentation = FrameCapturePresentation::new(&media);
        let mut state = FrameCaptureSelectionState::new(&media);
        state.set_timestamp_text(&media, "invalid".to_owned());
        let retained = (
            egui::TextureId::Managed(9),
            [640, 360],
            FrameTimestamp::from_milliseconds(1_250).into(),
        );
        let preview = FrameCaptureLivePreview::from_preview(
            &crate::frame_capture::preview::PreviewState::Idle,
            Some(retained),
        );

        let output = context.run_ui(Default::default(), |ui| {
            let _ = super::show(
                ui,
                &media,
                &presentation,
                &mut state,
                FrameCapturePositionSurface::Precision {
                    preview,
                    navigation: FrameNavigationAvailability {
                        previous: false,
                        next: false,
                        loading: false,
                        failed: false,
                    },
                },
                true,
            );
        });

        assert!(
            output
                .shapes
                .iter()
                .any(|shape| { shape_contains(&shape.shape, "Last preview: 00:00:01.250") })
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn idle_validation_state_keeps_the_last_valid_preview_texture() {
        let retained = (
            egui::TextureId::Managed(7),
            [640, 360],
            FrameTimestamp::from_milliseconds(250).into(),
        );
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
            let _ = super::show(
                ui,
                &media,
                &presentation,
                &mut state,
                FrameCapturePositionSurface::Precision {
                    preview,
                    navigation: FrameNavigationAvailability {
                        previous: true,
                        next: true,
                        loading: false,
                        failed: false,
                    },
                },
                true,
            );
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
