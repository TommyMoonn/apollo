use std::fmt;

use crate::media::{
    error::MediaError,
    model::MediaFile,
    operation::PreparedMediaOperation,
    output::{MediaOutput, MediaOutputError},
    tools::MediaTools,
};

use super::{
    command::build_upscale_invocation_to,
    model::UpscaleRequest,
    validation::{UpscaleValidationError, validate_upscale_parameters},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpscalePreparationError {
    Validation(UpscaleValidationError),
    Output(MediaOutputError),
}

impl fmt::Display for UpscalePreparationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => write!(f, "upscale request is invalid: {error}"),
            Self::Output(error) => write!(f, "upscale output could not be prepared: {error}"),
        }
    }
}

impl std::error::Error for UpscalePreparationError {}

impl UpscalePreparationError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                UpscaleValidationError::MissingVideoStream => {
                    MediaError::missing_stream(error.to_string())
                }
                UpscaleValidationError::Output(error) => MediaError::from_output_error(error),
                UpscaleValidationError::MissingSourceDimensions { .. }
                | UpscaleValidationError::InvalidSourceDimensions { .. }
                | UpscaleValidationError::InvalidTargetBoundary { .. }
                | UpscaleValidationError::TargetNotLarger { .. }
                | UpscaleValidationError::NvencUnavailable { .. }
                | UpscaleValidationError::UnsupportedVideoFormat { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
            },
            Self::Output(error) => MediaError::from_output_error(error),
        }
    }
}

pub(crate) fn prepare_upscale_operation(
    tools: &MediaTools,
    media: &MediaFile,
    request: &UpscaleRequest,
) -> Result<PreparedMediaOperation, UpscalePreparationError> {
    let mut request = request.clone();
    request.nvenc_capability =
        super::model::capability_for_codec(&tools.video_encoders, request.codec).clone();
    let (dimensions, encoder, pixel_format) = validate_upscale_parameters(&request, media)
        .map_err(UpscalePreparationError::Validation)?;

    let output = MediaOutput::prepare(&request.output).map_err(UpscalePreparationError::Output)?;
    let invocation = build_upscale_invocation_to(
        &request,
        media,
        dimensions,
        encoder,
        pixel_format,
        output.temporary_path(),
    );
    Ok(PreparedMediaOperation::new(
        output,
        tools,
        invocation.args,
        media.duration,
    ))
}
