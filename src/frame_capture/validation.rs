use std::fmt;

use crate::media::{
    model::MediaFile,
    output::{MediaOutputError, validate_output_path},
};

use super::model::FrameCaptureRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameCaptureValidationError {
    MissingVideoStream,
    NegativeTimestamp,
    TimestampExceedsDuration {
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
            Self::TimestampExceedsDuration {
                timestamp_milliseconds,
                duration_milliseconds,
            } => write!(
                formatter,
                "frame timestamp {timestamp_milliseconds} ms exceeds source duration {duration_milliseconds} ms"
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
    if request.timestamp.milliseconds() < 0 {
        return Err(FrameCaptureValidationError::NegativeTimestamp);
    }
    if let Some(duration) = media.duration {
        let duration_milliseconds = duration.as_millis();
        if request.timestamp.milliseconds() as u128 > duration_milliseconds {
            return Err(FrameCaptureValidationError::TimestampExceedsDuration {
                timestamp_milliseconds: request.timestamp.milliseconds(),
                duration_milliseconds,
            });
        }
    }

    validate_output_extension(request)?;
    validate_output_path(&request.output).map_err(FrameCaptureValidationError::Output)
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
