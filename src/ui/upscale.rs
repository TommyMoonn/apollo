use std::{
    cell::Cell,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use egui::Ui;

use crate::{
    media::{model::MediaFile, output::MediaOutputError},
    upscale::{
        model::{UpscaleRequest, UpscaleTarget, VideoCodec},
        sizing::{OutputDimensions, calculate_output_dimensions},
        validation::{UpscaleValidationError, validate_upscale_request},
    },
};

use super::{layout, output_name::OutputNameState, theme, widgets};

const TARGETS: [UpscaleTarget; 3] = [
    UpscaleTarget::Qhd1440,
    UpscaleTarget::Uhd4k,
    UpscaleTarget::Uhd8k,
];

const UPSCALE_TO_LABEL: &str = "Upscale to";
const OUTPUT_LABEL: &str = "Output";
const BROWSE_LABEL: &str = "Browse";
const UPSCALE_LABEL: &str = "Upscale";
const CODEC_OPTIONS: [&str; 2] = ["H.264", "H.265"];

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
    let action = Cell::new(UpscaleSelectionAction::None);
    let validation = state.validation(media);

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            let _ = widgets::section_label(ui, UPSCALE_TO_LABEL);
            ui.add_space(theme::SPACING.compact);
            show_target_list(ui, media, state);
            ui.add_space(theme::SPACING.section);

            let _ = widgets::section_label(ui, OUTPUT_LABEL);
            ui.add_space(theme::SPACING.compact);
            let mut output_stem = state.output_filename_stem().to_string_lossy().into_owned();
            let selector_width = widgets::select_menu_intrinsic_width(ui, &CODEC_OPTIONS);
            let mut requested_codec = None;
            let row = widgets::operation_output_row(
                ui,
                &mut output_stem,
                "mp4",
                BROWSE_LABEL,
                selector_width,
                |ui| {
                    widgets::select_menu(ui, codec_label(state.codec()), &CODEC_OPTIONS, |ui| {
                        for codec in [VideoCodec::H264, VideoCodec::H265] {
                            if widgets::select_menu_option(
                                ui,
                                codec_label(codec),
                                state.codec() == codec,
                            )
                            .clicked()
                            {
                                requested_codec = Some(codec);
                                ui.close();
                            }
                        }
                    });
                },
            );
            if row.filename.changed() {
                state.set_output_filename_stem(OsString::from(output_stem));
            }
            if let Some(codec) = requested_codec {
                state.set_codec(codec);
            }
            if row.browse.clicked() {
                action.set(UpscaleSelectionAction::BrowseOutput);
            }

            if let Some(feedback) = validation.as_ref().err().and_then(upscale_ready_feedback) {
                ui.add_space(theme::SPACING.tight);
                let _ = widgets::inline_status(ui, feedback, widgets::InlineStatusKind::Error);
            }
        },
        |ui| {
            if widgets::primary_button_enabled(ui, UPSCALE_LABEL, validation.is_ok()).clicked() {
                action.set(UpscaleSelectionAction::UpscaleRequested);
            }
        },
    );

    action.get()
}

fn upscale_ready_feedback(error: &UpscaleValidationError) -> Option<&'static str> {
    match error {
        UpscaleValidationError::Output(MediaOutputError::InvalidOutputPath(_)) => {
            Some("Enter a usable output filename.")
        }
        _ => None,
    }
}

fn show_target_list(ui: &mut Ui, media: &MediaFile, state: &mut UpscaleSelectionState) {
    widgets::separator(ui);

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
}

fn source_dimensions(media: &MediaFile) -> Option<(u32, u32)> {
    let video = media.video_streams.first()?;
    match (video.width, video.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Some((width, height)),
        _ => None,
    }
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
        upscale::{
            model::{UpscaleTarget, VideoCodec},
            validation::UpscaleValidationError,
        },
    };

    use super::{UpscaleSelectionState, upscale_ready_feedback};

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

        assert_eq!(
            state.output_dimensions(&media),
            Some(crate::upscale::sizing::OutputDimensions {
                width: 3840,
                height: 1600,
            })
        );
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
}
