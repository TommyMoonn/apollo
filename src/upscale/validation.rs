use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::media::model::MediaFile;

use super::model::{UpscaleRequest, UpscaleTarget};

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
    SameInputAndOutput(PathBuf),
    TargetNotLarger {
        target: UpscaleTarget,
        source_width: u32,
        source_height: u32,
        target_width: u32,
        target_height: u32,
    },
    InvalidOutputPath(PathBuf),
    OutputParentUnavailable {
        path: PathBuf,
        kind: io::ErrorKind,
    },
    OutputParentNotDirectory(PathBuf),
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
            Self::SameInputAndOutput(path) => {
                write!(f, "input and output must differ: {}", path.display())
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
            Self::InvalidOutputPath(path) => {
                write!(f, "output path has no usable file name: {}", path.display())
            }
            Self::OutputParentUnavailable { path, kind } => write!(
                f,
                "output parent is unavailable ({}): {kind:?}",
                path.display()
            ),
            Self::OutputParentNotDirectory(path) => {
                write!(f, "output parent is not a directory: {}", path.display())
            }
        }
    }
}

impl std::error::Error for UpscaleValidationError {}

/// Validate an Upscale request against the primary probed video stream.
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

    if same_requested_path(&request.input, &request.output) {
        return Err(UpscaleValidationError::SameInputAndOutput(
            request.output.clone(),
        ));
    }

    validate_target_is_larger(request.target, width, height)?;
    validate_output_path(&request.output)?;

    Ok(())
}

fn validate_target_is_larger(
    target: UpscaleTarget,
    source_width: u32,
    source_height: u32,
) -> Result<(), UpscaleValidationError> {
    let (target_width, target_height) = target.boundary();
    let (source_long, source_short) = ordered_edges(source_width, source_height);
    let (target_long, target_short) = ordered_edges(target_width, target_height);

    if source_long < target_long && source_short < target_short {
        return Ok(());
    }

    Err(UpscaleValidationError::TargetNotLarger {
        target,
        source_width,
        source_height,
        target_width,
        target_height,
    })
}

const fn ordered_edges(width: u32, height: u32) -> (u32, u32) {
    if width >= height {
        (width, height)
    } else {
        (height, width)
    }
}

fn same_requested_path(input: &Path, output: &Path) -> bool {
    #[cfg(windows)]
    {
        input
            .to_string_lossy()
            .eq_ignore_ascii_case(&output.to_string_lossy())
    }

    #[cfg(not(windows))]
    {
        input == output
    }
}

fn validate_output_path(output: &Path) -> Result<(), UpscaleValidationError> {
    output
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| UpscaleValidationError::InvalidOutputPath(output.to_path_buf()))?;

    if fs::metadata(output).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(UpscaleValidationError::InvalidOutputPath(
            output.to_path_buf(),
        ));
    }

    let parent = output_parent(output);
    let metadata =
        fs::metadata(&parent).map_err(|error| UpscaleValidationError::OutputParentUnavailable {
            path: parent.clone(),
            kind: error.kind(),
        })?;

    if !metadata.is_dir() {
        return Err(UpscaleValidationError::OutputParentNotDirectory(parent));
    }

    Ok(())
}

fn output_parent(output: &Path) -> PathBuf {
    match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}
