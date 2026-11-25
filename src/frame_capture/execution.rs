use std::fmt;

use crate::media::{
    error::MediaError,
    model::MediaFile,
    operation::PreparedMediaOperation,
    output::{MediaOutput, MediaOutputError, OutputCollisionPolicy},
    tools::MediaTools,
};

use super::{
    command::build_frame_capture_invocation_to,
    model::FrameCaptureRequest,
    validation::{FrameCaptureValidationError, validate_frame_capture_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameCapturePreparationError {
    Validation(FrameCaptureValidationError),
    Output(MediaOutputError),
}

impl fmt::Display for FrameCapturePreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => {
                write!(formatter, "frame capture request is invalid: {error}")
            }
            Self::Output(error) => write!(
                formatter,
                "frame capture output could not be prepared: {error}"
            ),
        }
    }
}

impl std::error::Error for FrameCapturePreparationError {}

impl FrameCapturePreparationError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                FrameCaptureValidationError::MissingVideoStream => {
                    MediaError::missing_stream(error.to_string())
                }
                FrameCaptureValidationError::NegativeTimestamp
                | FrameCaptureValidationError::TimestampAtOrBeyondDuration { .. }
                | FrameCaptureValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
                FrameCaptureValidationError::Output(error) => MediaError::from_output_error(error),
            },
            Self::Output(error) => MediaError::from_output_error(error),
        }
    }
}

pub(crate) fn prepare_frame_capture_operation(
    tools: &MediaTools,
    media: &MediaFile,
    request: &FrameCaptureRequest,
    collision_policy: OutputCollisionPolicy,
) -> Result<PreparedMediaOperation, FrameCapturePreparationError> {
    validate_frame_capture_request(request, media)
        .map_err(FrameCapturePreparationError::Validation)?;
    let primary_video_stream_index = media
        .video_streams
        .first()
        .expect("validated frame capture media owns a video stream")
        .stream_index;

    let output = MediaOutput::prepare_with_policy(&request.output, collision_policy)
        .map_err(FrameCapturePreparationError::Output)?;
    let invocation = build_frame_capture_invocation_to(
        request,
        primary_video_stream_index,
        output.temporary_path(),
    );
    Ok(PreparedMediaOperation::new(
        output,
        tools,
        invocation.args,
        None,
    ))
}
