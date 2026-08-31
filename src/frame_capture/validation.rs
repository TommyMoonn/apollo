use std::{fmt, time::Duration};

use crate::media::{
    model::MediaFile,
    output::{MediaOutputError, validate_output_path},
};

use super::model::FrameCaptureRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameCaptureValidationError {
    MissingVideoStream,
    NegativeTimestamp,
    TimestampAtOrBeyondDuration {
        timestamp_milliseconds: i64,
        duration_milliseconds: u128,
    },
    OutputExtensionMismatch {
        expected: &'static str,
        actual: Option<String>,
    },
    Output(MediaOutputError),
}

impl fmt::Display for FrameCaptureValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => formatter.write_str("source has no video stream"),
            Self::NegativeTimestamp => formatter.write_str("frame timestamp cannot be negative"),
            Self::TimestampAtOrBeyondDuration {
                timestamp_milliseconds,
                duration_milliseconds,
            } => write!(
                formatter,
                "frame timestamp {timestamp_milliseconds} ms is at or beyond source duration {duration_milliseconds} ms"
            ),
            Self::OutputExtensionMismatch { expected, actual } => match actual {
                Some(actual) => write!(
                    formatter,
                    "output extension '.{actual}' does not match the selected '.{expected}' format"
                ),
                None => write!(
                    formatter,
                    "output path must use the '.{expected}' extension for the selected format"
                ),
            },
            Self::Output(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for FrameCaptureValidationError {}

/// Validate a Frame Capture request without accessing the destination filesystem.
///
/// Destination identity, existence, and writability remain owned by `MediaOutput::prepare`
/// immediately before process launch.
pub fn validate_frame_capture_request(
    request: &FrameCaptureRequest,
    media: &MediaFile,
) -> Result<(), FrameCaptureValidationError> {
    if media.video_streams.is_empty() {
        return Err(FrameCaptureValidationError::MissingVideoStream);
    }
    validate_frame_timestamp(request.timestamp, media.duration)?;

    validate_output_extension(request)?;
    validate_output_path(&request.output).map_err(FrameCaptureValidationError::Output)
}

pub fn validate_frame_timestamp(
    timestamp: super::model::FrameTimestamp,
    duration: Option<Duration>,
) -> Result<(), FrameCaptureValidationError> {
    if timestamp.milliseconds() < 0 {
        return Err(FrameCaptureValidationError::NegativeTimestamp);
    }
    if let Some(duration) = duration {
        let duration_milliseconds = duration.as_millis();
        if Duration::from_millis(timestamp.milliseconds() as u64) >= duration {
            return Err(FrameCaptureValidationError::TimestampAtOrBeyondDuration {
                timestamp_milliseconds: timestamp.milliseconds(),
                duration_milliseconds,
            });
        }
    }
    Ok(())
}

fn validate_output_extension(
    request: &FrameCaptureRequest,
) -> Result<(), FrameCaptureValidationError> {
    let expected = request.format.extension();
    let actual = request
        .output
        .extension()
        .and_then(|extension| extension.to_str());
    if actual.is_some_and(|extension| extension.eq_ignore_ascii_case(expected)) {
        return Ok(());
    }
    Err(FrameCaptureValidationError::OutputExtensionMismatch {
        expected,
        actual: actual.map(str::to_owned),
    })
}
