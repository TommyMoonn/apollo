use std::fmt;

use crate::media::{
    model::{MediaFile, VideoFormat, pixel_format_bit_depth},
    output::{MediaOutputError, validate_output_path},
};

use super::{
    model::{UpscaleRequest, UpscaleTarget, VideoCodec, VideoEncoderBackend},
    sizing::{UpscaleSizingError, resolve_target_boundary},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpscaleValidationError {
    UnsupportedVideoFormat {
        reason: String,
    },
    MissingVideoStream,
    MissingSourceDimensions {
        stream_index: u32,
    },
    InvalidSourceDimensions {
        width: u32,
        height: u32,
    },
    InvalidTargetBoundary {
        target_width: u32,
        target_height: u32,
    },
    TargetNotLarger {
        source_width: u32,
        source_height: u32,
        target_width: u32,
        target_height: u32,
    },
    NvencUnavailable {
        reason: String,
    },
    Output(MediaOutputError),
}

impl fmt::Display for UpscaleValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVideoFormat { reason } => write!(f, "{reason}"),
            Self::MissingVideoStream => write!(f, "video upscale requires a video stream"),
            Self::MissingSourceDimensions { stream_index } => write!(
                f,
                "video stream {stream_index} is missing source dimensions"
            ),
            Self::InvalidSourceDimensions { width, height } => {
                write!(f, "source dimensions must be non-zero: {width}x{height}")
            }
            Self::InvalidTargetBoundary {
                target_width,
                target_height,
            } => write!(
                f,
                "target boundary must use positive even dimensions: {target_width}x{target_height}"
            ),
            Self::TargetNotLarger {
                source_width,
                source_height,
                target_width,
                target_height,
            } => write!(
                f,
                "target boundary {target_width}x{target_height} is not larger than source {source_width}x{source_height}"
            ),
            Self::NvencUnavailable { reason } => {
                write!(f, "NVIDIA NVENC is unavailable: {reason}")
            }
            Self::Output(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for UpscaleValidationError {}

/// Validate an Upscale request against the primary probed video stream without filesystem access.
///
/// Preset eligibility remains orientation-aware, while custom width and height
/// are compared against the source exactly as entered. Exact boundary resolution,
/// target validation, output sizing, and aspect-ratio preservation remain owned
/// by `upscale::sizing`.
pub fn validate_upscale_request(
    request: &UpscaleRequest,
    media: &MediaFile,
) -> Result<(), UpscaleValidationError> {
    let video = media
        .video_streams
        .first()
        .ok_or(UpscaleValidationError::MissingVideoStream)?;
    let (width, height) = match (video.width, video.height) {
        (Some(width), Some(height)) => (width, height),
        _ => {
            return Err(UpscaleValidationError::MissingSourceDimensions {
                stream_index: video.stream_index,
            });
        }
    };

    if width == 0 || height == 0 {
        return Err(UpscaleValidationError::InvalidSourceDimensions { width, height });
    }

    validate_target_is_larger(request.target, width, height)?;
    let encoder = request.resolve_video_encoder().map_err(|error| {
        UpscaleValidationError::NvencUnavailable {
            reason: error.reason,
        }
    })?;
    resolve_output_pixel_format(&video.format, request.codec, encoder.backend)?;
    validate_output_path(&request.output).map_err(UpscaleValidationError::Output)?;

    Ok(())
}

/// One format policy shared by ready validation and command construction.
/// Strict output pixel selection also prevents an external encoder build silently substituting
/// a lower-depth format. NVENC runtime availability remains a separate capability check.
pub fn resolve_output_pixel_format(
    format: &VideoFormat,
    codec: VideoCodec,
    backend: VideoEncoderBackend,
) -> Result<&'static str, UpscaleValidationError> {
    let reject = |reason: &str| UpscaleValidationError::UnsupportedVideoFormat {
        reason: reason.to_owned(),
    };
    let inferred = format
        .pixel_format
        .as_deref()
        .and_then(pixel_format_bit_depth);
    if inferred.zip(format.bit_depth).is_some_and(|(a, b)| a != b) {
        return Err(reject(
            "Source pixel format and bit depth disagree. Upscale cannot safely preserve this format.",
        ));
    }
    let depth = inferred.or(format.bit_depth);
    let hdr = matches!(
        format.color_transfer.as_deref(),
        Some("smpte2084" | "arib-std-b67")
    );
    if hdr && depth != Some(10) {
        return Err(reject(
            "HDR Upscale requires a known 10-bit 4:2:0 source and H.265.",
        ));
    }
    match (format.pixel_format.as_deref(), depth) {
        (None, None) => Ok("+"), // Preserve unknown input format; never guess an 8-bit conversion.
        (Some("yuv420p"), Some(8)) => Ok("+yuv420p"),
        (Some("nv12"), Some(8)) => Ok(match backend {
            VideoEncoderBackend::Cpu => "+yuv420p",
            VideoEncoderBackend::NvidiaNvenc => "+nv12",
        }),
        (Some("yuvj420p"), Some(8)) if backend == VideoEncoderBackend::Cpu => Ok("+yuvj420p"),
        (Some("yuv420p10le" | "yuv420p10be" | "p010le" | "p010be"), Some(10)) => {
            if codec != VideoCodec::H265 {
                return Err(reject("Use H.265 to preserve this 10-bit source."));
            }
            Ok(match backend {
                VideoEncoderBackend::Cpu => "+yuv420p10le",
                VideoEncoderBackend::NvidiaNvenc => "+p010le",
            })
        }
        _ => Err(reject(
            "This source pixel format is not supported for safe Upscale preservation.",
        )),
    }
}

fn validate_target_is_larger(
    target: UpscaleTarget,
    source_width: u32,
    source_height: u32,
) -> Result<(), UpscaleValidationError> {
    let boundary = resolve_target_boundary(target, source_width, source_height).map_err(
        |error| match error {
            UpscaleSizingError::InvalidTargetBoundary {
                boundary_width,
                boundary_height,
            } => UpscaleValidationError::InvalidTargetBoundary {
                target_width: boundary_width,
                target_height: boundary_height,
            },
            UpscaleSizingError::InvalidSourceDimensions { .. }
            | UpscaleSizingError::TargetNotLarger { .. } => {
                unreachable!("source and target-size eligibility are validated separately")
            }
        },
    )?;

    if source_width < boundary.width && source_height < boundary.height {
        return Ok(());
    }

    Err(UpscaleValidationError::TargetNotLarger {
        source_width,
        source_height,
        target_width: boundary.width,
        target_height: boundary.height,
    })
}
