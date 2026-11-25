use crate::upscale::draft::{UpscaleDraft, UpscaleEncoderCapabilities, capability_for_codec};

use std::{borrow::Cow, cell::Cell, ffi::OsString};

use egui::{RichText, Ui};

use crate::{
    media::{model::MediaFile, output::MediaOutputError, tools::VideoEncoderCapability},
    upscale::{
        model::{
            CpuQuality, NvencQuality, UpscaleTarget, VideoCodec, VideoEncoderBackend,
            VideoEncoderSelection,
        },
        validation::UpscaleValidationError,
    },
};

use super::{
    ADD_TO_QUEUE_LABEL, OperationSurfaceMode, fonts, layout, media_error, show_queue_confirmation,
    theme, widgets,
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

/// Render the complete pre-execution Upscale configuration surface.
///
/// The caller owns source acquisition and execution orchestration. This surface only mutates
/// target, codec, and output-path state and emits user intent for those responsibilities.
pub fn show<'a>(
    ui: &mut Ui,
    media: &MediaFile,
    state: &mut UpscaleDraft,
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

fn show_codec_selector(ui: &mut Ui, state: &mut UpscaleDraft) {
    let mut requested = None;
    select_codec(ui, state, &mut requested);
    if let Some(codec) = requested {
        state.set_codec(codec);
    }
}

fn select_codec(ui: &mut Ui, state: &UpscaleDraft, requested: &mut Option<VideoCodec>) {
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
    state: &mut UpscaleDraft,
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

fn show_target_list(ui: &mut Ui, media: &MediaFile, state: &mut UpscaleDraft) {
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
    state: &mut UpscaleDraft,
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
    use std::{ffi::OsString, path::PathBuf};

    use crate::{
        media::{output::MediaOutputError, tools::VideoEncoderCapability},
        upscale::{
            model::{VideoCodec, VideoEncoderBackend, VideoEncoderSelection},
            validation::UpscaleValidationError,
        },
    };

    use super::{
        UpscaleDraft, UpscaleEncoderCapabilities, encoder_selection_label, quality_backend,
        show_custom_target_controls, show_encoding_controls, upscale_ready_feedback,
    };

    use crate::upscale::draft::tests::{capabilities, media};

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
        let mut state = UpscaleDraft::new(&source);

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
        let mut state = UpscaleDraft::new(&source);
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
    fn encoding_controls_fit_narrow_and_normal_ready_content_widths() {
        let context = egui::Context::default();
        crate::ui::fonts::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            for width in [344.0, 564.0] {
                let source = media(Some(1920), Some(1080));
                let mut state = UpscaleDraft::new(&source);
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
    fn custom_controls_stack_at_narrow_width_and_stay_inline_at_normal_width() {
        let context = egui::Context::default();
        crate::ui::fonts::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            for (width, expected) in [
                (344.0, crate::ui::layout::FieldPairLayout::Stacked),
                (564.0, crate::ui::layout::FieldPairLayout::Columns),
            ] {
                let media = media(Some(1920), Some(1080));
                let mut state = UpscaleDraft::new(&media);
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
    fn ten_bit_source_disables_h264_with_visible_guidance_and_accepts_h265() {
        let mut source = media(Some(1920), Some(1080));
        source.video_streams[0].format.pixel_format = Some("yuv420p10le".to_owned());
        source.video_streams[0].format.bit_depth = Some(10);
        let mut state = UpscaleDraft::new(&source);
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
    fn output_feedback_is_limited_to_pure_request_errors() {
        let source = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&source);
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
        let output_error = UpscaleValidationError::Output(
            MediaOutputError::OutputParentNotDirectory(PathBuf::from("not-a-directory")),
        );

        assert_eq!(upscale_ready_feedback(&target_error), None);
        assert_eq!(upscale_ready_feedback(&output_error), None);
    }
    #[test]
    fn empty_or_unusable_filename_stem_disables_upscale_and_invalidates_request_output() {
        let media = media(Some(1920), Some(1080));
        let mut state = UpscaleDraft::new(&media);

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
}
