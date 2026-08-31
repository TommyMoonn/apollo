use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use egui::{Align, Label, Layout, RichText, ScrollArea, Sides, TextStyle, Ui};

use crate::{
    media::{
        model::{FrameRate, MediaFile},
        output::MediaOutputError,
    },
    upscale::{
        model::{UpscaleRequest, UpscaleTarget, VideoCodec},
        sizing::{OutputDimensions, calculate_output_dimensions},
        validation::{UpscaleValidationError, validate_upscale_request},
    },
};

use super::{fonts, layout, output_name::OutputNameState, theme, widgets};

const TARGETS: [UpscaleTarget; 3] = [
    UpscaleTarget::Qhd1440,
    UpscaleTarget::Uhd4k,
    UpscaleTarget::Uhd8k,
];

const UPSCALE_TO_LABEL: &str = "Upscale to";
const VIDEO_CODEC_LABEL: &str = "Video codec";
const ACTUAL_OUTPUT_LABEL: &str = "Actual output";
const OUTPUT_LABEL: &str = "Output";
const BROWSE_LABEL: &str = "Browse";
const UPSCALE_LABEL: &str = "Upscale";
const UNAVAILABLE_OUTPUT: &str = "Unavailable";

/// Pre-execution Upscale state. Sizing remains owned by `upscale::sizing`, while execution stays
/// deferred to the shared process integration phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpscaleSelectionState {
    target: UpscaleTarget,
    codec: VideoCodec,
    output_name: OutputNameState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpscaleSelectionAction {
    #[default]
    None,
    BrowseOutput,
    UpscaleRequested,
}

impl UpscaleSelectionState {
    /// New Upscale sessions follow the selected prototype defaults: 4K, H.264, MP4 output.
    pub fn new(media: &MediaFile) -> Self {
        let target = UpscaleTarget::Uhd4k;
        Self {
            target,
            codec: VideoCodec::H264,
            output_name: OutputNameState::new(
                &media.path,
                default_output_stem(&media.path, target),
            ),
        }
    }

    pub fn target(&self) -> UpscaleTarget {
        self.target
    }

    pub fn codec(&self) -> VideoCodec {
        self.codec
    }

    pub fn output_path(&self) -> PathBuf {
        self.output_name.path("mp4")
    }

    pub fn output_filename_stem(&self) -> &OsStr {
        self.output_name.stem()
    }

    pub fn output_is_customized(&self) -> bool {
        self.output_name.is_customized()
    }

    pub fn set_target(&mut self, media: &MediaFile, target: UpscaleTarget) {
        if self.target == target {
            return;
        }

        self.target = target;
        self.output_name
            .set_generated_stem(default_output_stem(&media.path, target));
    }

    pub fn set_codec(&mut self, codec: VideoCodec) {
        self.codec = codec;
    }

    pub fn set_output_filename_stem(&mut self, stem: OsString) {
        self.output_name.set_stem(stem);
    }

    pub fn set_custom_output_path(&mut self, output: PathBuf) {
        self.output_name.set_custom_path(output);
    }

    pub fn output_dimensions(&self, media: &MediaFile) -> Option<OutputDimensions> {
        let (width, height) = source_dimensions(media)?;
        calculate_output_dimensions(width, height, self.target).ok()
    }

    pub fn can_upscale(&self, media: &MediaFile) -> bool {
        self.validation(media).is_ok()
    }

    fn validation(&self, media: &MediaFile) -> Result<(), UpscaleValidationError> {
        validate_upscale_request(&self.request(media), media)
    }

    /// Build the Phase 29 request without starting work.
    pub fn request(&self, media: &MediaFile) -> UpscaleRequest {
        UpscaleRequest {
            input: media.path.clone(),
            output: self.output_path(),
            target: self.target,
            codec: self.codec,
        }
    }
}

/// Render the complete pre-execution Upscale configuration surface.
///
/// The caller owns source acquisition and execution orchestration. This surface only mutates
/// target, codec, and output-path state and emits user intent for those responsibilities.
pub fn show(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut UpscaleSelectionState,
) -> UpscaleSelectionAction {
    let mut action = UpscaleSelectionAction::None;
    let list_content_height = TARGETS.len() as f32 * theme::UPSCALE_TARGET_ROW_MIN_HEIGHT;
    let validation = state.validation(media);
    let has_validation_feedback = validation
        .as_ref()
        .err()
        .and_then(upscale_ready_feedback)
        .is_some();
    let plan = layout::task_surface_plan(
        ui.available_height(),
        upscale_fixed_flow_height(ui, media, has_validation_feedback),
        list_content_height,
        theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
    );

    layout::task_surface(ui, plan.overflow, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;

        show_source_header(ui, media);
        ui.add_space(theme::SPACING.section);

        let _ = widgets::section_label(ui, UPSCALE_TO_LABEL);
        ui.add_space(theme::SPACING.compact);
        show_target_list(ui, media, state, plan.list.max_height);
        ui.add_space(theme::SPACING.section);

        show_codec_and_actual_output(ui, media, state);
        ui.add_space(theme::SPACING.focal);

        let _ = widgets::section_label(ui, OUTPUT_LABEL);
        ui.add_space(theme::SPACING.compact);
        let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
        let row = widgets::output_filename_row(ui, &mut output_stem, "mp4", BROWSE_LABEL);
        if row.filename.changed() {
            state.set_output_filename_stem(OsString::from(output_stem));
        }
        if row.browse.clicked() {
            action = UpscaleSelectionAction::BrowseOutput;
        }

        if let Some(feedback) = validation.as_ref().err().and_then(upscale_ready_feedback) {
            ui.add_space(theme::SPACING.tight);
            let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
        }

        ui.add_space(theme::SPACING.related);
        let response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::primary_button_enabled(ui, UPSCALE_LABEL, validation.is_ok())
            })
            .inner;
        if response.clicked() {
            action = UpscaleSelectionAction::UpscaleRequested;
        }
    });

    action
}

fn upscale_ready_feedback(error: &UpscaleValidationError) -> Option<&'static str> {
    match error {
        UpscaleValidationError::Output(MediaOutputError::InvalidOutputPath(_)) => {
            Some("Enter a usable output filename.")
        }
        _ => None,
    }
}

fn show_source_header(ui: &mut Ui, media: &MediaFile) {
    let file_name = source_file_name(&media.path);
    let metadata = source_metadata(media);
    let metadata_gap = if metadata.is_some() {
        theme::SPACING.tight
    } else {
        0.0
    };
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
                    ui.spacing_mut().item_spacing.y = metadata_gap;
                    ui.add(
                        Label::new(RichText::new(file_name).text_style(TextStyle::Body))
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

fn show_target_list(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut UpscaleSelectionState,
    list_height: f32,
) {
    widgets::separator(ui);

    ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for target in TARGETS {
                let response = widgets::selectable_row(
                    ui,
                    target_label(target),
                    Some(&target_boundary_text(target)),
                    state.target() == target,
                    widgets::SelectionIndicator::Radio,
                    theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
                );

                if response.clicked() {
                    state.set_target(media, target);
                }
            }
        });
}

fn show_codec_and_actual_output(ui: &mut Ui, media: &MediaFile, state: &mut UpscaleSelectionState) {
    let actual_output = actual_output_text(state, media);
    let arrangement = layout::field_pair_layout(ui.available_width());
    let requested_codec = match arrangement {
        layout::FieldPairLayout::Columns => {
            let mut requested_codec = None;
            ui.columns(2, |columns| {
                requested_codec = show_codec_field(&mut columns[0], state.codec());
                show_actual_output_field(&mut columns[1], &actual_output);
            });
            requested_codec
        }
        layout::FieldPairLayout::Stacked => {
            let requested_codec = show_codec_field(ui, state.codec());
            ui.add_space(theme::SPACING.related);
            show_actual_output_field(ui, &actual_output);
            requested_codec
        }
    };

    if let Some(codec) = requested_codec {
        state.set_codec(codec);
    }
}

fn show_codec_field(ui: &mut Ui, codec: VideoCodec) -> Option<VideoCodec> {
    let _ = widgets::section_label(ui, VIDEO_CODEC_LABEL);
    ui.add_space(theme::SPACING.compact);

    let mut requested_codec = None;
    widgets::select_menu(ui, codec_label(codec), &["H.264", "H.265"], |ui| {
        if widgets::select_menu_option(ui, "H.264", codec == VideoCodec::H264).clicked() {
            requested_codec = Some(VideoCodec::H264);
            ui.close();
        }
        if widgets::select_menu_option(ui, "H.265", codec == VideoCodec::H265).clicked() {
            requested_codec = Some(VideoCodec::H265);
            ui.close();
        }
    });
    requested_codec
}

fn show_actual_output_field(ui: &mut Ui, actual_output: &str) {
    let _ = widgets::section_label(ui, ACTUAL_OUTPUT_LABEL);
    ui.add_space(theme::SPACING.compact);
    ui.label(
        RichText::new(actual_output)
            .text_style(fonts::technical_text_style())
            .color(theme::FOREGROUND),
    );
}

fn upscale_fixed_flow_height(ui: &Ui, media: &MediaFile, has_validation_feedback: bool) -> f32 {
    let section_label_height = ui.text_style_height(&TextStyle::Small);
    let technical_height = ui.text_style_height(&fonts::technical_text_style());
    let source_header_height = source_header_height(ui, source_metadata(media).is_some());
    let output_row_height = widgets::output_filename_row_plan(ui, None, BROWSE_LABEL).flow_height();
    let field_height = section_label_height
        + theme::SPACING.compact
        + theme::COMPACT_CONTROL_HEIGHT.max(technical_height);
    let field_flow_height = match layout::field_pair_layout(ui.available_width()) {
        layout::FieldPairLayout::Columns => field_height,
        layout::FieldPairLayout::Stacked => field_height * 2.0 + theme::SPACING.related,
    };
    let feedback_height = if has_validation_feedback {
        theme::SPACING.tight + ui.text_style_height(&TextStyle::Body)
    } else {
        0.0
    };

    source_header_height
        + theme::SPACING.section
        + section_label_height
        + theme::SPACING.compact
        + theme::SEPARATOR_WIDTH
        + theme::SPACING.section
        + field_flow_height
        + theme::SPACING.focal
        + section_label_height
        + theme::SPACING.compact
        + output_row_height
        + feedback_height
        + theme::SPACING.related
        + theme::PRIMARY_CONTROL_HEIGHT
}

fn source_header_height(ui: &Ui, has_metadata: bool) -> f32 {
    let body_height = ui.text_style_height(&TextStyle::Body);
    let metadata_height = if has_metadata {
        theme::SPACING.tight + ui.text_style_height(&fonts::technical_text_style())
    } else {
        0.0
    };

    (body_height + metadata_height).max(theme::COMPACT_CONTROL_HEIGHT)
}

fn source_dimensions(media: &MediaFile) -> Option<(u32, u32)> {
    let video = media.video_streams.first()?;
    match (video.width, video.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Some((width, height)),
        _ => None,
    }
}

fn actual_output_text(state: &UpscaleSelectionState, media: &MediaFile) -> String {
    state
        .output_dimensions(media)
        .map(|dimensions| format!("{} × {}", dimensions.width, dimensions.height))
        .unwrap_or_else(|| UNAVAILABLE_OUTPUT.to_owned())
}

fn source_file_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn source_metadata(media: &MediaFile) -> Option<String> {
    let video = media.video_streams.first()?;
    let mut parts = Vec::new();

    if let (Some(width), Some(height)) = (video.width, video.height)
        && width > 0
        && height > 0
    {
        parts.push(format!("{width}×{height}"));
    }

    if let Some(codec) = nonempty(video.codec.as_deref()) {
        parts.push(display_codec(codec));
    }

    if let Some(frame_rate) = video.frame_rate.and_then(display_frame_rate) {
        parts.push(frame_rate);
    }

    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn display_codec(codec: &str) -> String {
    match codec.to_ascii_lowercase().as_str() {
        "h264" | "avc" => "H.264".to_owned(),
        "h265" | "hevc" => "H.265".to_owned(),
        _ => codec.to_ascii_uppercase(),
    }
}

fn display_frame_rate(frame_rate: FrameRate) -> Option<String> {
    if frame_rate.numerator == 0 || frame_rate.denominator == 0 {
        return None;
    }

    if frame_rate.numerator.is_multiple_of(frame_rate.denominator) {
        return Some(format!(
            "{} FPS",
            frame_rate.numerator / frame_rate.denominator
        ));
    }

    Some(format!(
        "{:.2} FPS",
        f64::from(frame_rate.numerator) / f64::from(frame_rate.denominator)
    ))
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn default_output_stem(input: &Path, target: UpscaleTarget) -> OsString {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));

    let mut output_stem = OsString::from(stem);
    output_stem.push("-");
    output_stem.push(target_suffix(target));
    output_stem
}

pub(super) fn target_label(target: UpscaleTarget) -> &'static str {
    match target {
        UpscaleTarget::Qhd1440 => "1440p",
        UpscaleTarget::Uhd4k => "4K",
        UpscaleTarget::Uhd8k => "8K",
    }
}

fn target_suffix(target: UpscaleTarget) -> &'static str {
    match target {
        UpscaleTarget::Qhd1440 => "1440",
        UpscaleTarget::Uhd4k => "4k",
        UpscaleTarget::Uhd8k => "8k",
    }
}

fn target_boundary_text(target: UpscaleTarget) -> String {
    let (width, height) = target.boundary();
    format!("{width} × {height}")
}

pub(super) fn codec_label(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "H.264",
        VideoCodec::H265 => "H.265",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
    };

    use crate::{
        media::{
            model::{FrameRate, MediaFile, VideoStream},
            output::MediaOutputError,
        },
        ui::{layout, theme},
        upscale::{
            model::{UpscaleTarget, VideoCodec},
            validation::UpscaleValidationError,
        },
    };

    use super::{
        UpscaleSelectionState, actual_output_text, source_metadata, upscale_fixed_flow_height,
        upscale_ready_feedback,
    };

    fn media(width: Option<u32>, height: Option<u32>) -> MediaFile {
        MediaFile {
            path: PathBuf::from("captures/recording.mkv"),
            duration: None,
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width,
                height,
                frame_rate: FrameRate::new(60, 1),
            }],
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn default_policy_selects_4k_h264_and_mp4_output() {
        let media = media(Some(1920), Some(1080));
        let state = UpscaleSelectionState::new(&media);

        assert_eq!(state.target(), UpscaleTarget::Uhd4k);
        assert_eq!(state.codec(), VideoCodec::H264);
        assert_eq!(state.output_path(), Path::new("captures/recording-4k.mp4"));
        assert!(!state.output_is_customized());
    }

    #[test]
    fn target_selection_updates_generated_output_name() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);

        state.set_target(&media, UpscaleTarget::Qhd1440);
        assert_eq!(state.target(), UpscaleTarget::Qhd1440);
        assert_eq!(
            state.output_path(),
            Path::new("captures/recording-1440.mp4")
        );

        state.set_target(&media, UpscaleTarget::Uhd8k);
        assert_eq!(state.target(), UpscaleTarget::Uhd8k);
        assert_eq!(state.output_path(), Path::new("captures/recording-8k.mp4"));
    }

    #[test]
    fn codec_selection_updates_state() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);

        state.set_codec(VideoCodec::H265);

        assert_eq!(state.codec(), VideoCodec::H265);
    }

    #[test]
    fn ultrawide_actual_output_uses_shared_aspect_preserving_sizing() {
        let media = media(Some(1920), Some(800));
        let state = UpscaleSelectionState::new(&media);

        assert_eq!(actual_output_text(&state, &media), "3840 × 1600");
    }

    #[test]
    fn pure_request_validity_controls_primary_action() {
        let source = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&source);
        state.set_custom_output_path(PathBuf::from("recording-4k.mp4"));

        assert!(state.can_upscale(&source));

        state.set_custom_output_path(source.path.clone());
        assert!(state.can_upscale(&source));

        state.set_custom_output_path(PathBuf::new());
        assert!(!state.can_upscale(&source));

        state.set_custom_output_path(PathBuf::from("missing-upscale-ui-parent/output.mp4"));
        assert!(state.can_upscale(&source));
    }

    #[test]
    fn output_feedback_is_limited_to_pure_request_errors() {
        let source = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&source);
        state.set_custom_output_path(PathBuf::new());

        let error = state.validation(&source).unwrap_err();
        assert!(matches!(
            &error,
            UpscaleValidationError::Output(MediaOutputError::InvalidOutputPath(_))
        ));
        assert_eq!(
            upscale_ready_feedback(&error),
            Some("Enter a usable output filename.")
        );
        assert!(!state.can_upscale(&source));

        state.set_custom_output_path(PathBuf::from("missing-parent/output.mp4"));

        assert!(state.validation(&source).is_ok());
        assert!(state.can_upscale(&source));
    }

    #[test]
    fn invalid_target_and_destination_preflight_errors_have_no_ready_feedback() {
        let target_error = UpscaleValidationError::TargetNotLarger {
            target: UpscaleTarget::Uhd4k,
            source_width: 3840,
            source_height: 2160,
            target_width: 3840,
            target_height: 2160,
        };
        let output_error = UpscaleValidationError::Output(MediaOutputError::OutputAlreadyExists(
            PathBuf::from("existing.mp4"),
        ));

        assert_eq!(upscale_ready_feedback(&target_error), None);
        assert_eq!(upscale_ready_feedback(&output_error), None);
    }

    #[test]
    fn invalid_dimensions_and_downscale_target_disable_primary_action() {
        let source = media(Some(3840), Some(2160));
        let mut state = UpscaleSelectionState::new(&source);
        state.set_custom_output_path(PathBuf::from("recording-upscaled.mp4"));

        assert!(!state.can_upscale(&source));

        state.set_target(&source, UpscaleTarget::Uhd8k);
        assert!(state.can_upscale(&source));

        let missing_dimensions = media(None, Some(1080));
        assert!(!UpscaleSelectionState::new(&missing_dimensions).can_upscale(&missing_dimensions));
    }

    #[test]
    fn generated_default_filename_uses_selected_target_suffix() {
        let media = media(Some(1920), Some(1080));
        let state = UpscaleSelectionState::new(&media);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-4k.mp4")
        );
        assert!(!state.output_is_customized());
    }

    #[test]
    fn customized_output_stem_is_preserved_across_target_changes() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);
        state.set_custom_output_path(PathBuf::from("exports/final-video.mkv"));

        assert_eq!(
            state.output_path(),
            PathBuf::from("exports/final-video.mp4")
        );
        state.set_output_filename_stem(OsString::from("renamed upscale"));
        assert_eq!(
            state.output_path(),
            PathBuf::from("exports/renamed upscale.mp4")
        );

        state.set_target(&media, UpscaleTarget::Uhd8k);

        assert_eq!(
            state.output_path(),
            PathBuf::from("exports/renamed upscale.mp4")
        );
        assert_eq!(
            state.output_filename_stem(),
            OsString::from("renamed upscale")
        );
        assert!(state.output_is_customized());
    }

    #[test]
    fn edited_filename_stem_preserves_unicode_spaces_and_mp4_extension() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);

        state.set_output_filename_stem(OsString::from("final upscale 日本語"));

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/final upscale 日本語.mp4")
        );
        assert_eq!(
            state.request(&media).output,
            PathBuf::from("captures/final upscale 日本語.mp4")
        );

        state.set_target(&media, UpscaleTarget::Uhd8k);
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/final upscale 日本語.mp4")
        );
    }

    #[test]
    fn empty_or_unusable_filename_stem_disables_upscale_and_invalidates_request_output() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);

        for stem in ["", "   ", "../escape", "bad\\name"] {
            state.set_output_filename_stem(OsString::from(stem));
            assert!(
                !state.can_upscale(&media),
                "{stem:?} should not be runnable"
            );
            assert!(state.request(&media).output.as_os_str().is_empty());
            let error = state.validation(&media).unwrap_err();
            assert_eq!(
                upscale_ready_feedback(&error),
                Some("Enter a usable output filename.")
            );
        }
    }

    #[test]
    fn windows_reserved_device_stem_disables_upscale() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);

        state.set_output_filename_stem(OsString::from("NuL"));

        assert!(!state.can_upscale(&media));
        assert!(state.request(&media).output.as_os_str().is_empty());
    }

    #[test]
    fn request_uses_current_pre_execution_selection() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);
        state.set_target(&media, UpscaleTarget::Uhd8k);
        state.set_codec(VideoCodec::H265);

        let request = state.request(&media);

        assert_eq!(request.input, media.path);
        assert_eq!(request.output, PathBuf::from("captures/recording-8k.mp4"));
        assert_eq!(request.target, UpscaleTarget::Uhd8k);
        assert_eq!(request.codec, VideoCodec::H265);
    }

    #[test]
    fn source_metadata_uses_dimensions_codec_and_frame_rate() {
        assert_eq!(
            source_metadata(&media(Some(1920), Some(1080))).as_deref(),
            Some("1920×1080 · H.264 · 60 FPS")
        );
    }

    #[test]
    fn source_metadata_falls_back_to_available_fields() {
        let mut media = media(Some(1920), Some(1080));
        media.video_streams[0].codec = None;
        media.video_streams[0].frame_rate = None;

        assert_eq!(source_metadata(&media).as_deref(), Some("1920×1080"));

        media.video_streams[0].width = None;
        media.video_streams[0].height = None;
        assert_eq!(source_metadata(&media), None);
    }

    #[test]
    fn fractional_frame_rate_is_presented_without_assuming_integer_fps() {
        let mut media = media(Some(1920), Some(1080));
        media.video_streams[0].frame_rate = FrameRate::new(60_000, 1001);

        assert_eq!(
            source_metadata(&media).as_deref(),
            Some("1920×1080 · H.264 · 59.94 FPS")
        );
    }

    #[test]
    fn minimum_height_uses_outer_task_scroll_before_shrinking_target_rows() {
        let context = egui::Context::default();
        super::fonts::install(&context);

        let output = context.run_ui(Default::default(), |ui| {
            let media = media(Some(1920), Some(1080));
            let usable_height =
                layout::MINIMUM_CONTENT_SIZE.y - f32::from(layout::CONTENT_PADDING) * 2.0;
            let fixed_height = upscale_fixed_flow_height(ui, &media, false);
            let plan = layout::task_surface_plan(
                usable_height,
                fixed_height,
                3.0 * theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
                theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
            );

            assert_eq!(usable_height, 204.0);
            assert_eq!(plan.overflow, layout::TaskOverflow::Scroll);
            assert_eq!(plan.list.max_height, theme::UPSCALE_TARGET_ROW_MIN_HEIGHT);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn narrow_stacked_fields_are_accounted_for_before_task_overflow() {
        let context = egui::Context::default();
        super::fonts::install(&context);

        let output = context.run_ui(Default::default(), |ui| {
            let media = media(Some(1920), Some(1080));
            let narrow_width =
                layout::MINIMUM_CONTENT_SIZE.x - f32::from(layout::CONTENT_PADDING) * 2.0;
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(narrow_width, 204.0), egui::Sense::hover());
            let child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            let fixed_height = upscale_fixed_flow_height(&child, &media, false);
            let plan = layout::task_surface_plan(
                child.available_height(),
                fixed_height,
                3.0 * theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
                theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
            );

            assert_eq!(
                layout::field_pair_layout(child.available_width()),
                layout::FieldPairLayout::Stacked
            );
            assert_eq!(plan.overflow, layout::TaskOverflow::Scroll);
            assert_eq!(plan.list.max_height, theme::UPSCALE_TARGET_ROW_MIN_HEIGHT);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn reference_height_keeps_target_list_nested_and_primary_action_in_flow() {
        let context = egui::Context::default();
        super::fonts::install(&context);

        let output = context.run_ui(Default::default(), |ui| {
            let media = media(Some(1920), Some(1080));
            let usable_height =
                layout::REFERENCE_CONTENT_SIZE.y - f32::from(layout::CONTENT_PADDING) * 2.0;
            let fixed_height = upscale_fixed_flow_height(ui, &media, false);
            let plan = layout::task_surface_plan(
                usable_height,
                fixed_height,
                3.0 * theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
                theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
            );

            assert_eq!(usable_height, 323.0);
            assert_eq!(plan.overflow, layout::TaskOverflow::Flow);
            assert_eq!(plan.list.behavior, layout::ListBehavior::Scroll);
            assert_eq!(
                plan.list.max_height,
                (usable_height - fixed_height).max(0.0)
            );
            assert!(plan.list.max_height >= theme::UPSCALE_TARGET_ROW_MIN_HEIGHT);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn all_three_targets_are_visible_when_height_can_fit_them() {
        let context = egui::Context::default();
        super::fonts::install(&context);

        let output = context.run_ui(Default::default(), |ui| {
            let media = media(Some(1920), Some(1080));
            let fixed_height = upscale_fixed_flow_height(ui, &media, false);
            let content_height = 3.0 * theme::UPSCALE_TARGET_ROW_MIN_HEIGHT;
            let plan = layout::task_surface_plan(
                fixed_height + content_height,
                fixed_height,
                content_height,
                theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
            );

            assert_eq!(plan.overflow, layout::TaskOverflow::Flow);
            assert_eq!(plan.list.behavior, layout::ListBehavior::Flow);
            assert_eq!(plan.list.max_height, content_height);
        });
        output.drop_without_applying_deltas();
    }
}
