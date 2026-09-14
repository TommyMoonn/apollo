use std::{
    borrow::Cow,
    cell::Cell,
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use egui::{RichText, Ui};

use crate::{
    media::{
        model::MediaFile,
        output::MediaOutputError,
        tools::{VideoEncoderCapabilities, VideoEncoderCapability},
    },
    upscale::{
        model::{
            CpuQuality, NvencQuality, UpscaleEncoderSettings, UpscaleRequest, UpscaleTarget,
            VideoCodec, VideoEncoderBackend, VideoEncoderSelection,
        },
        sizing::{OutputDimensions, calculate_output_dimensions, resolve_target_boundary},
        validation::{UpscaleValidationError, validate_upscale_request},
    },
};

use super::{
    ADD_TO_QUEUE_LABEL, OperationSurfaceMode, fonts, layout, media_error,
    output_name::OutputNameState, show_queue_confirmation, theme, widgets,
};

const PRESET_TARGETS: [UpscaleTarget; 3] = [
    UpscaleTarget::Qhd1440,
    UpscaleTarget::Uhd4k,
    UpscaleTarget::Uhd8k,
];

const UPSCALE_TO_LABEL: &str = "Upscale to";
const OUTPUT_LABEL: &str = "Output";
const BROWSE_LABEL: &str = "Browse";
const UPSCALE_LABEL: &str = "Upscale";
const CODEC_OPTIONS: [&str; 2] = ["H.264", "H.265"];
const ENCODER_OPTIONS: [&str; 3] = ["Auto", "NVIDIA NVENC", "CPU"];
const ENCODING_LABEL: &str = "Encoding";
const CODEC_LABEL: &str = "Codec";
const QUALITY_GUIDANCE: &str = "Lower value = better quality / larger file";

#[derive(Debug, Clone, Copy)]
pub enum UpscaleEncoderCapabilities<'a> {
    Checking,
    Ready(&'a VideoEncoderCapabilities),
}

/// Pre-execution Upscale state. Sizing remains owned by `upscale::sizing`, while execution stays
/// deferred to the shared process integration phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpscaleSelectionState {
    target: UpscaleTarget,
    custom_boundary: Option<(u32, u32)>,
    codec: VideoCodec,
    encoder_settings: UpscaleEncoderSettings,
    output_name: OutputNameState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpscaleSelectionAction {
    #[default]
    None,
    BrowseOutput,
    QueueRequested,
    UpscaleRequested,
    SaveQueueChanges,
    CancelQueueEdit,
}

impl UpscaleSelectionState {
    /// New Upscale sessions follow the selected prototype defaults: 4K, H.264, MP4 output.
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

    fn validation(&self, media: &MediaFile) -> Result<(), UpscaleValidationError> {
        validate_upscale_request(&self.request(media), media)
    }

    fn ready_validation(
        &self,
        media: &MediaFile,
        capabilities: UpscaleEncoderCapabilities<'_>,
    ) -> Option<Result<(), UpscaleValidationError>> {
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
        Some(validate_upscale_request(
            &self.request_with_nvenc_capability(media, capability),
            media,
        ))
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

/// Render the complete pre-execution Upscale configuration surface.
///
/// The caller owns source acquisition and execution orchestration. This surface only mutates
/// target, codec, and output-path state and emits user intent for those responsibilities.
pub fn show<'a>(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut UpscaleSelectionState,
    capabilities: UpscaleEncoderCapabilities<'_>,
    mode: impl Into<OperationSurfaceMode<'a>>,
) -> UpscaleSelectionAction {
    let mode = mode.into();
    let shared_defaults = matches!(mode, OperationSurfaceMode::BatchDefaults { .. });
    let action = Cell::new(UpscaleSelectionAction::None);
    let validation = state.ready_validation(media, capabilities);

    layout::operation_surface(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            let _ = widgets::section_label(ui, UPSCALE_TO_LABEL);
            ui.add_space(theme::SPACING.compact);
            show_target_list(ui, media, state);
            let custom_selected = matches!(state.target(), UpscaleTarget::Custom { .. });
            let _ = widgets::conditional_fade(
                ui,
                "upscale-custom-target-controls",
                custom_selected,
                |ui| {
                    ui.add_space(theme::SPACING.related);
                    let _ = show_custom_target_controls(ui, media, state);
                },
            );
            ui.add_space(theme::SPACING.section);

            if shared_defaults {
                let _ = widgets::section_label(ui, CODEC_LABEL);
                ui.add_space(theme::SPACING.compact);
                show_codec_selector(ui, state);
                ui.add_space(theme::SPACING.section);
            }

            let _ = widgets::section_label(ui, ENCODING_LABEL);
            ui.add_space(theme::SPACING.compact);
            show_encoding_controls(ui, state, capabilities);
            if !shared_defaults {
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
                    |ui| select_codec(ui, state, &mut requested_codec),
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
            }

            if let Some(feedback) = validation
                .as_ref()
                .and_then(|validation| validation.as_ref().err())
                .and_then(upscale_ready_feedback)
            {
                ui.add_space(theme::SPACING.tight);
                let _ = widgets::inline_status(ui, &feedback, widgets::InlineStatusKind::Error);
            }
            if let Some(error) = mode.inline_action_error() {
                ui.add_space(theme::SPACING.related);
                media_error::show_operation_action(ui, error);
            }
        },
        |ui| {
            let runnable = matches!(validation, Some(Ok(())));
            match mode {
                OperationSurfaceMode::Immediate {
                    start_allowed,
                    queue_confirmation,
                    ..
                } => {
                    if widgets::primary_button_enabled(ui, UPSCALE_LABEL, runnable && start_allowed)
                        .clicked()
                    {
                        action.set(UpscaleSelectionAction::UpscaleRequested);
                    }
                    if ui
                        .add_enabled_ui(runnable, |ui| {
                            widgets::secondary_button(ui, ADD_TO_QUEUE_LABEL)
                        })
                        .inner
                        .clicked()
                    {
                        action.set(UpscaleSelectionAction::QueueRequested);
                    }
                    show_queue_confirmation(ui, queue_confirmation);
                }
                edit_mode => {
                    let (save_label, cancel_label) = edit_mode
                        .edit_labels()
                        .expect("non-immediate operation mode has edit labels");
                    let save_allowed = match edit_mode {
                        OperationSurfaceMode::BatchDefaults { save_allowed } => save_allowed,
                        _ => runnable,
                    };
                    if widgets::primary_button_enabled(ui, save_label, save_allowed).clicked() {
                        action.set(UpscaleSelectionAction::SaveQueueChanges);
                    }
                    if widgets::secondary_button(ui, cancel_label).clicked() {
                        action.set(UpscaleSelectionAction::CancelQueueEdit);
                    }
                }
            }
        },
    );

    action.get()
}

fn show_codec_selector(ui: &mut Ui, state: &mut UpscaleSelectionState) {
    let mut requested = None;
    select_codec(ui, state, &mut requested);
    if let Some(codec) = requested {
        state.set_codec(codec);
    }
}

fn select_codec(ui: &mut Ui, state: &UpscaleSelectionState, requested: &mut Option<VideoCodec>) {
    widgets::select_menu(ui, codec_label(state.codec()), &CODEC_OPTIONS, |ui| {
        for codec in [VideoCodec::H264, VideoCodec::H265] {
            if widgets::select_menu_option(ui, codec_label(codec), state.codec() == codec).clicked()
            {
                *requested = Some(codec);
                ui.close();
            }
        }
    });
}

fn upscale_ready_feedback(error: &UpscaleValidationError) -> Option<Cow<'_, str>> {
    match error {
        UpscaleValidationError::UnsupportedVideoFormat { reason } => Some(Cow::Borrowed(reason)),
        UpscaleValidationError::Output(MediaOutputError::InvalidOutputPath(_)) => {
            Some(Cow::Borrowed("Enter a usable output filename."))
        }
        UpscaleValidationError::NvencUnavailable { reason } => Some(Cow::Owned(format!(
            "NVIDIA NVENC is unavailable: {reason}."
        ))),
        _ => None,
    }
}

fn show_encoding_controls(
    ui: &mut Ui,
    state: &mut UpscaleSelectionState,
    capabilities: UpscaleEncoderCapabilities<'_>,
) {
    let selection = state.encoder_selection();
    let selected_label = encoder_selection_label(selection, state.codec(), capabilities);
    let mut requested_selection = None;
    widgets::labeled_control(ui, "Encoder", |ui| {
        widgets::select_menu(ui, &selected_label, &ENCODER_OPTIONS, |ui| {
            for candidate in [
                VideoEncoderSelection::Auto,
                VideoEncoderSelection::NvidiaNvenc,
                VideoEncoderSelection::Cpu,
            ] {
                if widgets::select_menu_option(
                    ui,
                    encoder_option_label(candidate),
                    selection == candidate,
                )
                .clicked()
                {
                    requested_selection = Some(candidate);
                    ui.close();
                }
            }
        });
    });
    if let Some(selection) = requested_selection {
        state.set_encoder_selection(selection);
    }

    ui.add_space(theme::SPACING.compact);
    match quality_backend(state.encoder_selection(), state.codec(), capabilities) {
        Some(VideoEncoderBackend::Cpu) => {
            let mut value = state.cpu_quality(state.codec()).value();
            let response = widgets::labeled_control(ui, "Quality", |ui| {
                widgets::encoder_quality_control(
                    ui,
                    "CPU quality",
                    &mut value,
                    CpuQuality::MIN..=CpuQuality::MAX,
                    "CRF",
                )
            });
            if response.changed() {
                state.set_cpu_quality(
                    state.codec(),
                    CpuQuality::new(value).expect("quality control enforces CPU range"),
                );
            }
        }
        Some(VideoEncoderBackend::NvidiaNvenc) => {
            let mut value = state.nvenc_quality(state.codec()).value();
            let response = widgets::labeled_control(ui, "Quality", |ui| {
                widgets::encoder_quality_control(
                    ui,
                    "NVIDIA NVENC quality",
                    &mut value,
                    NvencQuality::MIN..=NvencQuality::MAX,
                    "CQ",
                )
            });
            if response.changed() {
                state.set_nvenc_quality(
                    state.codec(),
                    NvencQuality::new(value).expect("quality control enforces NVENC range"),
                );
            }
        }
        None => {
            widgets::labeled_control(ui, "Quality", |ui| {
                ui.label(
                    RichText::new("Checking NVIDIA NVENC...")
                        .text_style(fonts::technical_text_style()),
                )
            });
        }
    }
    ui.add_space(theme::SPACING.tight);
    ui.label(RichText::new(QUALITY_GUIDANCE).color(theme::SECONDARY));
}

fn capability_for_codec(
    capabilities: &VideoEncoderCapabilities,
    codec: VideoCodec,
) -> &VideoEncoderCapability {
    match codec {
        VideoCodec::H264 => &capabilities.h264_nvenc,
        VideoCodec::H265 => &capabilities.h265_nvenc,
    }
}

fn quality_backend(
    selection: VideoEncoderSelection,
    codec: VideoCodec,
    capabilities: UpscaleEncoderCapabilities<'_>,
) -> Option<VideoEncoderBackend> {
    match selection {
        VideoEncoderSelection::Cpu => Some(VideoEncoderBackend::Cpu),
        VideoEncoderSelection::NvidiaNvenc => match capabilities {
            UpscaleEncoderCapabilities::Checking => None,
            UpscaleEncoderCapabilities::Ready(_) => Some(VideoEncoderBackend::NvidiaNvenc),
        },
        VideoEncoderSelection::Auto => match capabilities {
            UpscaleEncoderCapabilities::Checking => None,
            UpscaleEncoderCapabilities::Ready(capabilities) => {
                if *capability_for_codec(capabilities, codec) == VideoEncoderCapability::Available {
                    Some(VideoEncoderBackend::NvidiaNvenc)
                } else {
                    Some(VideoEncoderBackend::Cpu)
                }
            }
        },
    }
}

fn encoder_selection_label(
    selection: VideoEncoderSelection,
    codec: VideoCodec,
    capabilities: UpscaleEncoderCapabilities<'_>,
) -> Cow<'static, str> {
    match selection {
        VideoEncoderSelection::Auto => match quality_backend(selection, codec, capabilities) {
            Some(VideoEncoderBackend::Cpu) => Cow::Borrowed("Auto (CPU)"),
            Some(VideoEncoderBackend::NvidiaNvenc) => Cow::Borrowed("Auto (NVIDIA NVENC)"),
            None => Cow::Borrowed("Auto (checking...)"),
        },
        VideoEncoderSelection::Cpu => Cow::Borrowed("CPU"),
        VideoEncoderSelection::NvidiaNvenc => Cow::Borrowed("NVIDIA NVENC"),
    }
}

fn encoder_option_label(selection: VideoEncoderSelection) -> &'static str {
    match selection {
        VideoEncoderSelection::Auto => "Auto",
        VideoEncoderSelection::Cpu => "CPU",
        VideoEncoderSelection::NvidiaNvenc => "NVIDIA NVENC",
    }
}

fn show_target_list(ui: &mut Ui, media: &MediaFile, state: &mut UpscaleSelectionState) {
    widgets::separator(ui);

    for target in PRESET_TARGETS {
        let response = widgets::selectable_row(
            ui,
            target_label(target),
            Some(target_boundary_text(target)),
            state.target() == target,
            widgets::SelectionIndicator::Radio,
            theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
        );

        if response.clicked() {
            state.set_target(media, target);
        }
    }

    let custom_selected = matches!(state.target(), UpscaleTarget::Custom { .. });
    let response = widgets::selectable_row(
        ui,
        "Custom",
        None,
        custom_selected,
        widgets::SelectionIndicator::Radio,
        theme::UPSCALE_TARGET_ROW_MIN_HEIGHT,
    );
    if response.clicked() {
        state.select_custom_target(media);
    }
}

fn show_custom_target_controls(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut UpscaleSelectionState,
) -> layout::FieldPairLayout {
    let (mut max_width, mut max_height) = state
        .custom_boundary()
        .expect("selected Custom target owns a retained boundary");
    let field_layout = layout::field_pair_layout(ui.available_width());

    match field_layout {
        layout::FieldPairLayout::Columns => {
            widgets::labeled_control(ui, "Fit within", |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACING.compact;
                    let width = widgets::pixel_dimension_control(
                        ui,
                        "Maximum output width",
                        &mut max_width,
                    );
                    ui.label("×");
                    let height = widgets::pixel_dimension_control(
                        ui,
                        "Maximum output height",
                        &mut max_height,
                    );
                    if width.changed() || height.changed() {
                        state.set_custom_boundary(media, max_width, max_height);
                    }
                });
            });
        }
        layout::FieldPairLayout::Stacked => {
            ui.label("Fit within");
            ui.add_space(theme::SPACING.tight);
            let width = widgets::labeled_control(ui, "Width", |ui| {
                widgets::pixel_dimension_control(ui, "Maximum output width", &mut max_width)
            });
            ui.add_space(theme::SPACING.compact);
            let height = widgets::labeled_control(ui, "Height", |ui| {
                widgets::pixel_dimension_control(ui, "Maximum output height", &mut max_height)
            });
            if width.changed() || height.changed() {
                state.set_custom_boundary(media, max_width, max_height);
            }
        }
    }

    ui.add_space(theme::SPACING.compact);
    let dimensions = state.output_dimensions(media);
    widgets::labeled_control(ui, "Actual output", |ui| {
        let value = dimensions.map_or_else(
            || "Unavailable".to_owned(),
            |dimensions| format!("{} × {}", dimensions.width, dimensions.height),
        );
        ui.label(RichText::new(value).text_style(fonts::technical_text_style()));
    });

    field_layout
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

pub(super) fn target_label(target: UpscaleTarget) -> &'static str {
    match target {
        UpscaleTarget::Qhd1440 => "1440p",
        UpscaleTarget::Uhd4k => "4K",
        UpscaleTarget::Uhd8k => "8K",
        UpscaleTarget::Custom { .. } => "Custom",
    }
}

fn target_boundary_text(target: UpscaleTarget) -> &'static str {
    match target {
        UpscaleTarget::Qhd1440 => "2560 × 1440",
        UpscaleTarget::Uhd4k => "3840 × 2160",
        UpscaleTarget::Uhd8k => "7680 × 4320",
        UpscaleTarget::Custom { .. } => "Custom boundary",
    }
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
            tools::{VideoEncoderCapabilities, VideoEncoderCapability},
        },
        upscale::{
            model::{
                CpuQuality, NvencQuality, UpscaleTarget, VideoCodec, VideoEncoderBackend,
                VideoEncoderSelection,
            },
            validation::UpscaleValidationError,
        },
    };

    use super::{
        UpscaleEncoderCapabilities, UpscaleSelectionState, encoder_selection_label,
        quality_backend, show_custom_target_controls, show_encoding_controls,
        upscale_ready_feedback,
    };

    fn capabilities(
        h264_nvenc: VideoEncoderCapability,
        h265_nvenc: VideoEncoderCapability,
    ) -> VideoEncoderCapabilities {
        VideoEncoderCapabilities {
            h264_nvenc,
            h265_nvenc,
        }
    }

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
                format: Default::default(),
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
    fn auto_label_and_quality_follow_the_selected_codec_capability() {
        let available = capabilities(
            VideoEncoderCapability::Available,
            VideoEncoderCapability::EncoderNotCompiled,
        );

        assert_eq!(
            encoder_selection_label(
                VideoEncoderSelection::Auto,
                VideoCodec::H264,
                UpscaleEncoderCapabilities::Ready(&available),
            ),
            "Auto (NVIDIA NVENC)"
        );
        assert_eq!(
            quality_backend(
                VideoEncoderSelection::Auto,
                VideoCodec::H264,
                UpscaleEncoderCapabilities::Ready(&available),
            ),
            Some(VideoEncoderBackend::NvidiaNvenc)
        );
        assert_eq!(
            encoder_selection_label(
                VideoEncoderSelection::Auto,
                VideoCodec::H265,
                UpscaleEncoderCapabilities::Ready(&available),
            ),
            "Auto (CPU)"
        );
        assert_eq!(
            quality_backend(
                VideoEncoderSelection::Auto,
                VideoCodec::H265,
                UpscaleEncoderCapabilities::Ready(&available),
            ),
            Some(VideoEncoderBackend::Cpu)
        );
    }

    #[test]
    fn checking_capability_is_compact_and_keeps_cpu_ready() {
        let source = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&source);

        assert_eq!(
            encoder_selection_label(
                VideoEncoderSelection::Auto,
                VideoCodec::H264,
                UpscaleEncoderCapabilities::Checking,
            ),
            "Auto (checking...)"
        );
        assert_eq!(
            quality_backend(
                VideoEncoderSelection::Auto,
                VideoCodec::H264,
                UpscaleEncoderCapabilities::Checking,
            ),
            None
        );
        assert!(
            state
                .ready_validation(&source, UpscaleEncoderCapabilities::Checking)
                .is_none()
        );

        state.set_encoder_selection(VideoEncoderSelection::Cpu);
        assert!(matches!(
            state.ready_validation(&source, UpscaleEncoderCapabilities::Checking),
            Some(Ok(()))
        ));
    }

    #[test]
    fn explicit_unavailable_nvenc_preserves_selection_and_reason() {
        let source = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&source);
        state.set_encoder_selection(VideoEncoderSelection::NvidiaNvenc);
        let unavailable = capabilities(
            VideoEncoderCapability::RuntimeUnavailable {
                reason: "installed NVIDIA driver is too old".to_owned(),
            },
            VideoEncoderCapability::EncoderNotCompiled,
        );

        let error = state
            .ready_validation(&source, UpscaleEncoderCapabilities::Ready(&unavailable))
            .expect("capability check completed")
            .unwrap_err();

        assert_eq!(
            state.encoder_selection(),
            VideoEncoderSelection::NvidiaNvenc
        );
        assert_eq!(
            upscale_ready_feedback(&error).as_deref(),
            Some("NVIDIA NVENC is unavailable: installed NVIDIA driver is too old.")
        );
    }

    #[test]
    fn backend_and_codec_quality_values_are_preserved_independently() {
        let source = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&source);
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
    fn encoding_controls_fit_narrow_and_normal_ready_content_widths() {
        let context = egui::Context::default();
        crate::ui::fonts::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            for width in [344.0, 564.0] {
                let source = media(Some(1920), Some(1080));
                let mut state = UpscaleSelectionState::new(&source);
                let capabilities = capabilities(
                    VideoEncoderCapability::Available,
                    VideoEncoderCapability::Available,
                );
                let rendered = ui.allocate_ui(egui::vec2(width, 160.0), |ui| {
                    ui.set_width(width);
                    show_encoding_controls(
                        ui,
                        &mut state,
                        UpscaleEncoderCapabilities::Ready(&capabilities),
                    );
                });

                assert!(rendered.response.rect.width() <= width);
                assert!(rendered.response.rect.height() > 0.0);
            }
        });
        output.textures_delta.clear();
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
    fn first_custom_selection_seeds_the_resolved_preset_boundary() {
        let landscape = media(Some(1920), Some(1080));
        let mut landscape_state = UpscaleSelectionState::new(&landscape);
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
        let mut portrait_state = UpscaleSelectionState::new(&portrait);
        portrait_state.select_custom_target(&portrait);
        assert_eq!(portrait_state.custom_boundary(), Some((2160, 3840)));
    }

    #[test]
    fn custom_values_survive_preset_switches_and_update_the_request() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);
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
        let mut state = UpscaleSelectionState::new(&media);
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
        let mut state = UpscaleSelectionState::new(&media);
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
        let mut first_state = UpscaleSelectionState::new(&first);
        first_state.select_custom_target(&first);
        first_state.set_custom_boundary(&first, 3000, 2000);

        let replacement = media(Some(1280), Some(720));
        let replacement_state = UpscaleSelectionState::new(&replacement);

        assert_eq!(replacement_state.target(), UpscaleTarget::Uhd4k);
        assert_eq!(replacement_state.custom_boundary(), None);
    }

    #[test]
    fn custom_controls_stack_at_narrow_width_and_stay_inline_at_normal_width() {
        let context = egui::Context::default();
        crate::ui::fonts::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            for (width, expected) in [
                (344.0, crate::ui::layout::FieldPairLayout::Stacked),
                (564.0, crate::ui::layout::FieldPairLayout::Columns),
            ] {
                let media = media(Some(1920), Some(1080));
                let mut state = UpscaleSelectionState::new(&media);
                state.select_custom_target(&media);
                let rendered = ui.allocate_ui(egui::vec2(width, 240.0), |ui| {
                    ui.set_width(width);
                    show_custom_target_controls(ui, &media, &mut state)
                });

                assert_eq!(rendered.inner, expected);
                assert!(rendered.response.rect.width() <= width);
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn codec_selection_updates_state() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleSelectionState::new(&media);

        state.set_codec(VideoCodec::H265);

        assert_eq!(state.codec(), VideoCodec::H265);
    }

    #[test]
    fn ten_bit_source_disables_h264_with_visible_guidance_and_accepts_h265() {
        let mut source = media(Some(1920), Some(1080));
        source.video_streams[0].format.pixel_format = Some("yuv420p10le".to_owned());
        source.video_streams[0].format.bit_depth = Some(10);
        let mut state = UpscaleSelectionState::new(&source);
        let error = state.validation(&source).unwrap_err();
        assert!(!state.can_upscale(&source));
        assert_eq!(
            upscale_ready_feedback(&error).as_deref(),
            Some("Use H.265 to preserve this 10-bit source.")
        );
        state.set_codec(VideoCodec::H265);
        assert!(state.can_upscale(&source));
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
            upscale_ready_feedback(&error).as_deref(),
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
                upscale_ready_feedback(&error).as_deref(),
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
