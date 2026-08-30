use std::fmt;

use crate::media::{
    model::{MediaFile, StreamKind},
    output::{MediaOutputError, validate_output_path},
};

use super::model::AudioExtractRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioExtractValidationError {
    UnknownStreamIndex {
        stream_index: u32,
    },
    StreamIsNotAudio {
        stream_index: u32,
        kind: StreamKind,
    },
    OutputExtensionMismatch {
        expected: &'static str,
        actual: Option<String>,
    },
    Output(MediaOutputError),
}

impl fmt::Display for AudioExtractValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStreamIndex { stream_index } => {
                write!(f, "source stream {stream_index} does not exist")
            }
            Self::StreamIsNotAudio { stream_index, kind } => {
                write!(f, "source stream {stream_index} is {kind:?}, not audio")
            }
            Self::OutputExtensionMismatch { expected, actual } => match actual {
                Some(actual) => write!(
                    f,
                    "output extension '.{actual}' does not match the selected '.{expected}' format"
                ),
                None => write!(
                    f,
                    "output path must use the '.{expected}' extension for the selected format"
                ),
            },
            Self::Output(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AudioExtractValidationError {}

/// Validate an Audio Extract request without accessing the destination filesystem.
///
/// Destination identity, existence, and writability remain owned by `MediaOutput::prepare`
/// immediately before process launch.
pub fn validate_audio_extract_request(
    request: &AudioExtractRequest,
    media: &MediaFile,
) -> Result<(), AudioExtractValidationError> {
    let stream = media
        .streams
        .iter()
        .find(|stream| stream.stream_index == request.audio_stream_index)
        .ok_or(AudioExtractValidationError::UnknownStreamIndex {
            stream_index: request.audio_stream_index,
        })?;

    if stream.kind != StreamKind::Audio {
        return Err(AudioExtractValidationError::StreamIsNotAudio {
            stream_index: stream.stream_index,
            kind: stream.kind,
        });
    }

    validate_output_extension(request)?;
    validate_output_path(&request.output).map_err(AudioExtractValidationError::Output)
}

fn validate_output_extension(
    request: &AudioExtractRequest,
) -> Result<(), AudioExtractValidationError> {
    let expected = request.format.extension();
    let actual = request
        .output
        .extension()
        .and_then(|extension| extension.to_str());

    if actual.is_some_and(|extension| extension.eq_ignore_ascii_case(expected)) {
        return Ok(());
    }

    Err(AudioExtractValidationError::OutputExtensionMismatch {
        expected,
        actual: actual.map(str::to_owned),
    })
}
