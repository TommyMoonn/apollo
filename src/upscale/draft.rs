//! Editable operation configuration and typed request construction.

use super::{
    model::{
        CpuQuality, NvencQuality, UpscaleEncoderSettings, UpscaleRequest, UpscaleTarget,
        VideoCodec, VideoEncoderSelection, capability_for_codec,
    },
    sizing::{OutputDimensions, calculate_output_dimensions, resolve_target_boundary},
    validation::{UpscaleValidationError, validate_upscale_request},
};
use crate::media::{
    model::MediaFile,
    output_name::OutputNameState,
    tools::{VideoEncoderCapabilities, VideoEncoderCapability},
};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

/// Editable upscale settings; sizing and execution remain in their domain owners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpscaleDraft {
    target: UpscaleTarget,
    custom_boundary: Option<(u32, u32)>,
    codec: VideoCodec,
    encoder_settings: UpscaleEncoderSettings,
    output_name: OutputNameState,
}

impl UpscaleDraft {
    /// New Upscale sessions default to 4K, H.264, MP4 output.
    pub fn new(media: &MediaFile) -> Self {
        let target = UpscaleTarget::Uhd4k;
        Self {
            target,
            custom_boundary: None,
            codec: VideoCodec::H264,
            encoder_settings: UpscaleEncoderSettings::default(),
            output_name: OutputNameState::new(
                &media.path,
                generated_output_stem(media, target)
                    .expect("the default preset has a valid generated output name"),
            ),
        }
    }

    pub fn from_request(media: &MediaFile, request: &UpscaleRequest) -> Self {
        let custom_boundary = request
            .target
            .is_custom()
            .then(|| request.target.boundary());
        let mut state = Self {
            target: request.target,
            custom_boundary,
            codec: request.codec,
            encoder_settings: request.encoder_settings.clone(),
            output_name: OutputNameState::new(
                &media.path,
                request
                    .output
                    .file_stem()
                    .unwrap_or_default()
                    .to_os_string(),
            ),
        };
        state.output_name.set_custom_path(request.output.clone());
        state
    }

    pub fn target(&self) -> UpscaleTarget {
        self.target
    }

    pub fn codec(&self) -> VideoCodec {
        self.codec
    }

    pub fn encoder_selection(&self) -> VideoEncoderSelection {
        self.encoder_settings.selection()
    }

    pub fn encoder_settings(&self) -> &UpscaleEncoderSettings {
        &self.encoder_settings
    }

    pub fn set_encoder_selection(&mut self, selection: VideoEncoderSelection) {
        self.encoder_settings.set_selection(selection);
    }

    pub fn cpu_quality(&self, codec: VideoCodec) -> CpuQuality {
        self.encoder_settings.cpu_quality(codec)
    }

    pub fn set_cpu_quality(&mut self, codec: VideoCodec, quality: CpuQuality) {
        self.encoder_settings.set_cpu_quality(codec, quality);
    }

    pub fn nvenc_quality(&self, codec: VideoCodec) -> NvencQuality {
        self.encoder_settings.nvenc_quality(codec)
    }

    pub fn set_nvenc_quality(&mut self, codec: VideoCodec, quality: NvencQuality) {
        self.encoder_settings.set_nvenc_quality(codec, quality);
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
        if let UpscaleTarget::Custom {
            max_width,
            max_height,
        } = target
        {
            self.custom_boundary = Some((max_width, max_height));
        }
        self.refresh_generated_output_stem(media);
    }

    pub fn select_custom_target(&mut self, media: &MediaFile) {
        if matches!(self.target, UpscaleTarget::Custom { .. }) {
            return;
        }

        let (max_width, max_height) = self.custom_boundary.unwrap_or_else(|| {
            source_dimensions(media)
                .and_then(|(width, height)| {
                    resolve_target_boundary(self.target, width, height).ok()
                })
                .map_or_else(
                    || self.target.boundary(),
                    |boundary| (boundary.width, boundary.height),
                )
        });
        self.set_target(
            media,
            UpscaleTarget::Custom {
                max_width,
                max_height,
            },
        );
    }

    pub fn custom_boundary(&self) -> Option<(u32, u32)> {
        self.custom_boundary
    }

    pub fn set_custom_boundary(&mut self, media: &MediaFile, max_width: u32, max_height: u32) {
        self.custom_boundary = Some((max_width, max_height));
        if matches!(self.target, UpscaleTarget::Custom { .. }) {
            self.target = UpscaleTarget::Custom {
                max_width,
                max_height,
            };
            self.refresh_generated_output_stem(media);
        }
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

    pub fn validation(&self, media: &MediaFile) -> Result<(), UpscaleValidationError> {
        validate_upscale_request(&self.request(media), media)
    }

    pub fn ready_validation(
        &self,
        media: &MediaFile,
        capabilities: UpscaleEncoderCapabilities<'_>,
    ) -> Option<Result<(), UpscaleValidationError>> {
        self.validated_request(media, capabilities)
            .map(|result| result.map(|_| ()))
    }

    /// Pending discovery admits explicit CPU requests only.
    pub fn request_for_capabilities(
        &self,
        media: &MediaFile,
        capabilities: UpscaleEncoderCapabilities<'_>,
    ) -> Option<UpscaleRequest> {
        let capability = match capabilities {
            UpscaleEncoderCapabilities::Checking
                if self.encoder_selection() != VideoEncoderSelection::Cpu =>
            {
                return None;
            }
            UpscaleEncoderCapabilities::Checking => VideoEncoderCapability::EncoderNotCompiled,
            UpscaleEncoderCapabilities::Ready(capabilities) => {
                capability_for_codec(capabilities, self.codec).clone()
            }
        };
        Some(self.request_with_nvenc_capability(media, capability))
    }

    pub fn validated_request(
        &self,
        media: &MediaFile,
        capabilities: UpscaleEncoderCapabilities<'_>,
    ) -> Option<Result<UpscaleRequest, UpscaleValidationError>> {
        let request = self.request_for_capabilities(media, capabilities)?;
        Some(validate_upscale_request(&request, media).map(|_| request))
    }

    /// Build the current typed request without starting work or calculating command dimensions.
    pub fn request(&self, media: &MediaFile) -> UpscaleRequest {
        self.request_with_nvenc_capability(media, VideoEncoderCapability::EncoderNotCompiled)
    }

    pub fn request_with_nvenc_capability(
        &self,
        media: &MediaFile,
        nvenc_capability: VideoEncoderCapability,
    ) -> UpscaleRequest {
        UpscaleRequest {
            input: media.path.clone(),
            output: self.output_path(),
            target: self.target,
            codec: self.codec,
            encoder_settings: self.encoder_settings.clone(),
            nvenc_capability,
        }
    }

    fn refresh_generated_output_stem(&mut self, media: &MediaFile) {
        if let Some(stem) = generated_output_stem(media, self.target) {
            self.output_name.set_generated_stem(stem);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UpscaleEncoderCapabilities<'a> {
    Checking,
    Ready(&'a VideoEncoderCapabilities),
}

fn source_dimensions(media: &MediaFile) -> Option<(u32, u32)> {
    let video = media.video_streams.first()?;
    match (video.width, video.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Some((width, height)),
        _ => None,
    }
}

fn generated_output_stem(media: &MediaFile, target: UpscaleTarget) -> Option<OsString> {
    let stem = media
        .path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));

    let mut output_stem = OsString::from(stem);
    output_stem.push("-");
    match target {
        UpscaleTarget::Qhd1440 => output_stem.push("1440"),
        UpscaleTarget::Uhd4k => output_stem.push("4k"),
        UpscaleTarget::Uhd8k => output_stem.push("8k"),
        UpscaleTarget::Custom { .. } => {
            let (source_width, source_height) = source_dimensions(media)?;
            let dimensions =
                calculate_output_dimensions(source_width, source_height, target).ok()?;
            output_stem.push(format!("{}x{}", dimensions.width, dimensions.height));
        }
    }
    Some(output_stem)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
    };

    use crate::{
        media::{
            model::{FrameRate, MediaFile, VideoStream},
            tools::{VideoEncoderCapabilities, VideoEncoderCapability},
        },
        upscale::{
            model::{CpuQuality, NvencQuality, UpscaleTarget, VideoCodec, VideoEncoderSelection},
            validation::UpscaleValidationError,
        },
    };

    use super::*;

    pub(crate) fn capabilities(
        h264_nvenc: VideoEncoderCapability,
        h265_nvenc: VideoEncoderCapability,
    ) -> VideoEncoderCapabilities {
        VideoEncoderCapabilities {
            h264_nvenc,
            h265_nvenc,
        }
    }

    pub(crate) fn media(width: Option<u32>, height: Option<u32>) -> MediaFile {
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
                format: Default::default(),
            }],
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn default_policy_selects_4k_h264_and_mp4_output() {
        let media = media(Some(1920), Some(1080));
        let state = UpscaleDraft::new(&media);

        assert_eq!(state.target(), UpscaleTarget::Uhd4k);
        assert_eq!(state.codec(), VideoCodec::H264);
        assert_eq!(state.output_path(), Path::new("captures/recording-4k.mp4"));
        assert!(!state.output_is_customized());
        assert_eq!(state.encoder_selection(), VideoEncoderSelection::Auto);
        assert_eq!(
            state.cpu_quality(VideoCodec::H264),
            CpuQuality::H264_DEFAULT
        );
        assert_eq!(
            state.cpu_quality(VideoCodec::H265),
            CpuQuality::H265_DEFAULT
        );
        assert_eq!(state.nvenc_quality(VideoCodec::H264), NvencQuality::DEFAULT);
        assert_eq!(state.nvenc_quality(VideoCodec::H265), NvencQuality::DEFAULT);
    }

    #[test]
    fn backend_and_codec_quality_values_are_preserved_independently() {
        let source = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&source);
        state.set_cpu_quality(VideoCodec::H264, CpuQuality::new(12).unwrap());
        state.set_cpu_quality(VideoCodec::H265, CpuQuality::new(24).unwrap());
        state.set_nvenc_quality(VideoCodec::H264, NvencQuality::new(8).unwrap());
        state.set_nvenc_quality(VideoCodec::H265, NvencQuality::new(30).unwrap());

        state.set_encoder_selection(VideoEncoderSelection::NvidiaNvenc);
        state.set_codec(VideoCodec::H265);
        state.set_encoder_selection(VideoEncoderSelection::Cpu);
        state.set_codec(VideoCodec::H264);

        assert_eq!(state.cpu_quality(VideoCodec::H264).value(), 12);
        assert_eq!(state.cpu_quality(VideoCodec::H265).value(), 24);
        assert_eq!(state.nvenc_quality(VideoCodec::H264).value(), 8);
        assert_eq!(state.nvenc_quality(VideoCodec::H265).value(), 30);
    }

    #[test]
    fn target_selection_updates_generated_output_name() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);

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
    fn first_custom_selection_seeds_the_resolved_preset_boundary() {
        let landscape = media(Some(1920), Some(1080));
        let mut landscape_state = UpscaleDraft::new(&landscape);
        landscape_state.select_custom_target(&landscape);

        assert_eq!(
            landscape_state.target(),
            UpscaleTarget::Custom {
                max_width: 3840,
                max_height: 2160,
            }
        );
        assert_eq!(landscape_state.custom_boundary(), Some((3840, 2160)));
        assert_eq!(
            landscape_state.output_path(),
            PathBuf::from("captures/recording-3840x2160.mp4")
        );

        let portrait = media(Some(1080), Some(1920));
        let mut portrait_state = UpscaleDraft::new(&portrait);
        portrait_state.select_custom_target(&portrait);
        assert_eq!(portrait_state.custom_boundary(), Some((2160, 3840)));
    }

    #[test]
    fn custom_values_survive_preset_switches_and_update_the_request() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);
        state.select_custom_target(&media);
        state.set_custom_boundary(&media, 3000, 2000);

        state.set_target(&media, UpscaleTarget::Uhd8k);
        assert_eq!(state.custom_boundary(), Some((3000, 2000)));
        state.select_custom_target(&media);

        assert_eq!(
            state.request(&media).target,
            UpscaleTarget::Custom {
                max_width: 3000,
                max_height: 2000,
            }
        );
        assert_eq!(
            state.output_dimensions(&media),
            Some(crate::upscale::sizing::OutputDimensions {
                width: 3000,
                height: 1688,
            })
        );
        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-3000x1688.mp4")
        );
    }

    #[test]
    fn invalid_custom_values_are_retained_but_non_runnable() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);
        state.select_custom_target(&media);
        state.set_custom_boundary(&media, 2999, 0);

        assert_eq!(state.custom_boundary(), Some((2999, 0)));
        assert_eq!(state.output_dimensions(&media), None);
        assert!(!state.can_upscale(&media));
        assert!(matches!(
            state.validation(&media),
            Err(UpscaleValidationError::InvalidTargetBoundary {
                target_width: 2999,
                target_height: 0,
            })
        ));
    }

    #[test]
    fn manual_output_stem_survives_custom_dimension_changes() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);
        state.select_custom_target(&media);
        state.set_output_filename_stem(OsString::from("deliberate-name"));

        state.set_custom_boundary(&media, 3000, 2000);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/deliberate-name.mp4")
        );
        assert!(state.output_is_customized());
    }

    #[test]
    fn replacement_source_starts_without_retained_custom_values() {
        let first = media(Some(1920), Some(1080));
        let mut first_state = UpscaleDraft::new(&first);
        first_state.select_custom_target(&first);
        first_state.set_custom_boundary(&first, 3000, 2000);

        let replacement = media(Some(1280), Some(720));
        let replacement_state = UpscaleDraft::new(&replacement);

        assert_eq!(replacement_state.target(), UpscaleTarget::Uhd4k);
        assert_eq!(replacement_state.custom_boundary(), None);
    }

    #[test]
    fn codec_selection_updates_state() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);

        state.set_codec(VideoCodec::H265);

        assert_eq!(state.codec(), VideoCodec::H265);
    }

    #[test]
    fn ultrawide_actual_output_uses_shared_aspect_preserving_sizing() {
        let media = media(Some(1920), Some(800));
        let state = UpscaleDraft::new(&media);

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
        let mut state = UpscaleDraft::new(&source);
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
    fn invalid_dimensions_and_downscale_target_disable_primary_action() {
        let source = media(Some(3840), Some(2160));
        let mut state = UpscaleDraft::new(&source);
        state.set_custom_output_path(PathBuf::from("recording-upscaled.mp4"));

        assert!(!state.can_upscale(&source));

        state.set_target(&source, UpscaleTarget::Uhd8k);
        assert!(state.can_upscale(&source));

        let missing_dimensions = media(None, Some(1080));
        assert!(!UpscaleDraft::new(&missing_dimensions).can_upscale(&missing_dimensions));
    }

    #[test]
    fn generated_default_filename_uses_selected_target_suffix() {
        let media = media(Some(1920), Some(1080));
        let state = UpscaleDraft::new(&media);

        assert_eq!(
            state.output_path(),
            PathBuf::from("captures/recording-4k.mp4")
        );
        assert!(!state.output_is_customized());
    }

    #[test]
    fn customized_output_stem_is_preserved_across_target_changes() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);
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
        let mut state = UpscaleDraft::new(&media);

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
    fn windows_reserved_device_stem_disables_upscale() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);

        state.set_output_filename_stem(OsString::from("NuL"));

        assert!(!state.can_upscale(&media));
        assert!(state.request(&media).output.as_os_str().is_empty());
    }

    #[test]
    fn request_uses_current_pre_execution_selection() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);
        state.set_target(&media, UpscaleTarget::Uhd8k);
        state.set_codec(VideoCodec::H265);

        let request = state.request(&media);

        assert_eq!(request.input, media.path);
        assert_eq!(request.output, PathBuf::from("captures/recording-8k.mp4"));
        assert_eq!(request.target, UpscaleTarget::Uhd8k);
        assert_eq!(request.codec, VideoCodec::H265);
    }
}
