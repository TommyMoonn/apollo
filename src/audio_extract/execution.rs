use std::fmt;

use crate::media::{
    error::MediaError,
    model::MediaFile,
    operation::PreparedMediaOperation,
    output::{MediaOutput, MediaOutputError},
    tools::MediaTools,
};

use super::{
    command::build_audio_extract_invocation_to,
    model::{AudioExtractMetadata, AudioExtractRequest},
    validation::{AudioExtractValidationError, validate_audio_extract_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioExtractPreparationError {
    Validation(AudioExtractValidationError),
    Output(MediaOutputError),
}

impl fmt::Display for AudioExtractPreparationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => write!(f, "audio extraction request is invalid: {error}"),
            Self::Output(error) => {
                write!(f, "audio extraction output could not be prepared: {error}")
            }
        }
    }
}

impl std::error::Error for AudioExtractPreparationError {}

impl AudioExtractPreparationError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                AudioExtractValidationError::UnknownStreamIndex { .. } => {
                    MediaError::missing_stream(error.to_string())
                }
                AudioExtractValidationError::StreamIsNotAudio { .. }
                | AudioExtractValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
                AudioExtractValidationError::Output(error) => MediaError::from_output_error(error),
            },
            Self::Output(error) => MediaError::from_output_error(error),
        }
    }
}

pub(crate) fn prepare_audio_extract_operation(
    tools: &MediaTools,
    media: &MediaFile,
    request: &AudioExtractRequest,
) -> Result<PreparedMediaOperation, AudioExtractPreparationError> {
    validate_audio_extract_request(request, media)
        .map_err(AudioExtractPreparationError::Validation)?;

    let metadata = media
        .audio_streams
        .iter()
        .find(|stream| stream.stream_index == request.audio_stream_index)
        .map(AudioExtractMetadata::from)
        .unwrap_or_default();

    let output =
        MediaOutput::prepare(&request.output).map_err(AudioExtractPreparationError::Output)?;
    let invocation = build_audio_extract_invocation_to(request, &metadata, output.temporary_path());
    Ok(PreparedMediaOperation::new(
        output,
        tools,
        invocation.args,
        media.duration,
    ))
}
