use std::fmt;

use crate::media::{
    container::MediaContainer,
    model::{MediaFile, StreamKind},
    output::{MediaOutputError, validate_output_path},
};

use super::model::RemuxRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemuxValidationError {
    NoStreams,
    IncompatibleStream {
        container: MediaContainer,
        stream_index: u32,
        kind: StreamKind,
        codec: Option<String>,
    },
    OutputExtensionMismatch {
        expected: &'static str,
        actual: Option<String>,
    },
    Output(MediaOutputError),
}

impl fmt::Display for RemuxValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStreams => write!(f, "remux requires at least one source stream"),
            Self::IncompatibleStream {
                container,
                stream_index,
                kind,
                codec,
            } => write!(
                f,
                "stream {stream_index} ({kind:?}, codec {}) cannot be copied into {container:?}",
                codec.as_deref().unwrap_or("unknown")
            ),
            Self::OutputExtensionMismatch { expected, actual } => match actual {
                Some(actual) => write!(
                    f,
                    "output extension '.{actual}' does not match the selected '.{expected}' container"
                ),
                None => write!(
                    f,
                    "output path must use the '.{expected}' extension for the selected container"
                ),
            },
            Self::Output(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RemuxValidationError {}

/// Validate the filesystem-independent Remux request and complete probed inventory.
///
/// Destination existence, source/destination identity, and writability remain owned by
/// `MediaOutput::prepare` immediately before process launch.
pub fn validate_remux_request(
    request: &RemuxRequest,
    media: &MediaFile,
) -> Result<(), RemuxValidationError> {
    if media.streams.is_empty() {
        return Err(RemuxValidationError::NoStreams);
    }

    if let Some(stream) = media
        .streams
        .iter()
        .find(|stream| !request.container.is_stream_copy_compatible(stream))
    {
        return Err(RemuxValidationError::IncompatibleStream {
            container: request.container,
            stream_index: stream.stream_index,
            kind: stream.kind,
            codec: stream.codec.clone(),
        });
    }

    validate_output_extension(request)?;
    validate_output_path(&request.output).map_err(RemuxValidationError::Output)
}

fn validate_output_extension(request: &RemuxRequest) -> Result<(), RemuxValidationError> {
    let expected = request.container.extension();
    let actual = request
        .output
        .extension()
        .and_then(|extension| extension.to_str());

    if actual.is_some_and(|extension| extension.eq_ignore_ascii_case(expected)) {
        return Ok(());
    }

    Err(RemuxValidationError::OutputExtensionMismatch {
        expected,
        actual: actual.map(str::to_owned),
    })
}
