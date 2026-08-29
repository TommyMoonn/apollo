use std::fmt;

use crate::media::{
    model::MediaFile,
    output::{MediaOutputError, validate_output_path},
};

use super::{
    model::{UpscaleRequest, UpscaleTarget},
    sizing::oriented_target_boundary,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpscaleValidationError {
    MissingVideoStream,
    MissingSourceDimensions {
        stream_index: u32,
    },
    InvalidSourceDimensions {
        width: u32,
        height: u32,
    },
    TargetNotLarger {
        target: UpscaleTarget,
        source_width: u32,
        source_height: u32,
        target_width: u32,
        target_height: u32,
    },
    Output(MediaOutputError),
}

impl fmt::Display for UpscaleValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => write!(f, "video upscale requires a video stream"),
            Self::MissingSourceDimensions { stream_index } => write!(
                f,
                "video stream {stream_index} is missing source dimensions"
            ),
            Self::InvalidSourceDimensions { width, height } => {
                write!(f, "source dimensions must be non-zero: {width}x{height}")
            }
            Self::TargetNotLarger {
                target,
                source_width,
                source_height,
                target_width,
                target_height,
            } => write!(
                f,
                "{target:?} boundary {target_width}x{target_height} is not larger than source {source_width}x{source_height}"
            ),
            Self::Output(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for UpscaleValidationError {}

/// Validate an Upscale request against the primary probed video stream without filesystem access.
///
/// Target eligibility is orientation-independent: the source long and short edges
/// must both be strictly smaller than the selected target boundary. Exact output
/// sizing, aspect-ratio preservation, and portrait scaling remain owned by Phase
/// 0.1.0.30.
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
    validate_output_path(&request.output).map_err(UpscaleValidationError::Output)?;

    Ok(())
}

fn validate_target_is_larger(
    target: UpscaleTarget,
    source_width: u32,
    source_height: u32,
) -> Result<(), UpscaleValidationError> {
    let boundary = oriented_target_boundary(target, source_width, source_height);

    if source_width < boundary.width && source_height < boundary.height {
        return Ok(());
    }

    Err(UpscaleValidationError::TargetNotLarger {
        target,
        source_width,
        source_height,
        target_width: boundary.width,
        target_height: boundary.height,
    })
}
