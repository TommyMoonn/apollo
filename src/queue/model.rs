use std::{fmt, path::Path};

use crate::{
    audio_extract::{
        execution::{AudioExtractStartError, prepare_audio_extract_operation},
        model::AudioExtractRequest,
        validation::validate_audio_extract_request,
    },
    audio_mix::{
        execution::{AudioMixPreparationError, prepare_audio_mix_operation},
        model::AudioMixRequest,
        validation::validate_audio_mix_request,
    },
    frame_capture::{
        execution::{FrameCaptureStartError, prepare_frame_capture_operation},
        model::FrameCaptureRequest,
        validation::validate_frame_capture_request,
    },
    media::{
        error::MediaError,
        execution::MediaExecutionFailure,
        intake::{MediaIntake, MediaIntakeError, SourceIdentity},
        model::MediaFile,
        operation::PreparedMediaOperation,
        output::MediaOutputError,
        process::MediaProcessStartError,
        tools::MediaTools,
    },
    remux::{
        execution::{RemuxPreparationError, prepare_remux_operation},
        model::RemuxRequest,
        validation::validate_remux_request,
    },
    upscale::{
        execution::{UpscaleStartError, prepare_upscale_operation},
        model::UpscaleRequest,
        validation::validate_upscale_request,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(u64);

impl JobId {
    pub fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedOperation {
    AudioMix(AudioMixRequest),
    Upscale(UpscaleRequest),
    Remux(RemuxRequest),
    AudioExtract(AudioExtractRequest),
    FrameCapture(FrameCaptureRequest),
}

impl QueuedOperation {
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::AudioMix(_) => "Audio Mix",
            Self::Upscale(_) => "Upscale",
            Self::Remux(_) => "Remux",
            Self::AudioExtract(_) => "Audio Extract",
            Self::FrameCapture(_) => "Frame Capture",
        }
    }

    pub fn input(&self) -> &Path {
        match self {
            Self::AudioMix(request) => &request.input,
            Self::Upscale(request) => &request.input,
            Self::Remux(request) => &request.input,
            Self::AudioExtract(request) => &request.input,
            Self::FrameCapture(request) => &request.input,
        }
    }

    pub fn output(&self) -> &Path {
        match self {
            Self::AudioMix(request) => &request.output,
            Self::Upscale(request) => &request.output,
            Self::Remux(request) => &request.output,
            Self::AudioExtract(request) => &request.output,
            Self::FrameCapture(request) => &request.output,
        }
    }

    /// Validate the typed request without starting a process or staging output.
    pub(crate) fn validate(&self, media: &MediaFile) -> Result<(), QueuePreparationError> {
        match self {
            Self::AudioMix(request) => {
                validate_audio_mix_request(request, media).map_err(|error| {
                    QueuePreparationError::AudioMix(AudioMixPreparationError::Validation(error))
                })
            }
            Self::AudioExtract(request) => {
                validate_audio_extract_request(request, media).map_err(|error| {
                    QueuePreparationError::AudioExtract(AudioExtractStartError::Validation(error))
                })
            }
            Self::Remux(request) => validate_remux_request(request, media).map_err(|error| {
                QueuePreparationError::Remux(RemuxPreparationError::Validation(error))
            }),
            Self::Upscale(request) => validate_upscale_request(request, media).map_err(|error| {
                QueuePreparationError::Upscale(UpscaleStartError::Validation(error))
            }),
            Self::FrameCapture(request) => {
                validate_frame_capture_request(request, media).map_err(|error| {
                    QueuePreparationError::FrameCapture(FrameCaptureStartError::Validation(error))
                })
            }
        }
    }

    pub(crate) fn prepare(
        &self,
        tools: &MediaTools,
        media: &MediaFile,
    ) -> Result<PreparedMediaOperation, QueuePreparationError> {
        match self {
            Self::AudioMix(request) => prepare_audio_mix_operation(tools, media, request)
                .map_err(QueuePreparationError::AudioMix),
            Self::Upscale(request) => prepare_upscale_operation(tools, media, request)
                .map_err(QueuePreparationError::Upscale),
            Self::Remux(request) => {
                prepare_remux_operation(tools, media, request).map_err(QueuePreparationError::Remux)
            }
            Self::AudioExtract(request) => prepare_audio_extract_operation(tools, media, request)
                .map_err(QueuePreparationError::AudioExtract),
            Self::FrameCapture(request) => prepare_frame_capture_operation(tools, media, request)
                .map_err(QueuePreparationError::FrameCapture),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuePreparationError {
    AudioMix(AudioMixPreparationError),
    Upscale(UpscaleStartError),
    Remux(RemuxPreparationError),
    AudioExtract(AudioExtractStartError),
    FrameCapture(FrameCaptureStartError),
}

impl QueuePreparationError {
    pub(crate) fn user_error(&self) -> MediaError {
        match self {
            Self::AudioMix(error) => error.user_error(),
            Self::Upscale(error) => error.user_error(),
            Self::Remux(error) => error.user_error(),
            Self::AudioExtract(error) => error.user_error(),
            Self::FrameCapture(error) => error.user_error(),
        }
    }
}

impl fmt::Display for QueuePreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AudioMix(error) => error.fmt(formatter),
            Self::Upscale(error) => error.fmt(formatter),
            Self::Remux(error) => error.fmt(formatter),
            Self::AudioExtract(error) => error.fmt(formatter),
            Self::FrameCapture(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for QueuePreparationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueJobFailure {
    SourceUnavailable(MediaIntakeError),
    SourceChanged,
    Preparation(QueuePreparationError),
    ProcessStart(MediaProcessStartError),
    Execution(MediaExecutionFailure),
}

impl fmt::Display for QueueJobFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceUnavailable(error) => write!(formatter, "source unavailable: {error}"),
            Self::SourceChanged => write!(formatter, "source changed after it was queued"),
            Self::Preparation(error) => write!(formatter, "operation preparation failed: {error}"),
            Self::ProcessStart(error) => write!(formatter, "process launch failed: {error}"),
            Self::Execution(error) => write!(formatter, "media execution failed: {error:?}"),
        }
    }
}

impl std::error::Error for QueueJobFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueJobState {
    Waiting,
    Running { output: std::path::PathBuf },
    Completed { output: std::path::PathBuf },
    Failed(QueueJobFailure),
    Cancelled,
}

impl QueueJobState {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Waiting | Self::Running { .. })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed(_) | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueEnqueueError {
    JobIdExhausted,
    Source(MediaIntakeError),
    SourceDoesNotMatchMedia,
    SourceChanged,
    InvalidRequest(QueuePreparationError),
    Output(MediaOutputError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchEnqueueError {
    JobIdExhausted,
    Item {
        index: usize,
        error: QueueEnqueueError,
    },
}

impl fmt::Display for BatchEnqueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JobIdExhausted => formatter.write_str("queue job identifiers are exhausted"),
            Self::Item { index, error } => write!(formatter, "batch item {index}: {error}"),
        }
    }
}

impl std::error::Error for BatchEnqueueError {}

impl fmt::Display for QueueEnqueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JobIdExhausted => write!(formatter, "queue job identifiers are exhausted"),
            Self::Source(error) => {
                write!(formatter, "source unavailable: {error}")
            }
            Self::SourceDoesNotMatchMedia => {
                write!(
                    formatter,
                    "operation source does not match the probed media"
                )
            }
            Self::SourceChanged => formatter.write_str(
                "source changed or could not be verified after inspection; reopen the file",
            ),
            Self::InvalidRequest(error) => write!(formatter, "invalid queued operation: {error}"),
            Self::Output(error) => write!(formatter, "queue output is invalid: {error}"),
        }
    }
}

impl std::error::Error for QueueEnqueueError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueMutationError {
    AutomaticAdvancementActive,
    JobNotFound(JobId),
    JobNotWaiting(JobId),
    SourceUnavailable(MediaIntakeError),
    SourceChanged,
    InvalidRequest(QueuePreparationError),
    Output(MediaOutputError),
}

impl fmt::Display for QueueMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AutomaticAdvancementActive => {
                formatter.write_str("queue mutation is unavailable during automatic advancement")
            }
            Self::JobNotFound(id) => write!(formatter, "queue job {id} was not found"),
            Self::JobNotWaiting(id) => write!(formatter, "queue job {id} is not waiting"),
            Self::SourceUnavailable(error) => {
                write!(formatter, "queued source is unavailable: {error}")
            }
            Self::SourceChanged => formatter.write_str("queued source changed"),
            Self::InvalidRequest(error) => {
                write!(formatter, "invalid replacement operation: {error}")
            }
            Self::Output(error) => write!(formatter, "replacement output is invalid: {error}"),
        }
    }
}

impl std::error::Error for QueueMutationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitingJobMove {
    Earlier,
    Later,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueRunState {
    #[default]
    Stopped,
    Running,
    StoppingAfterCurrent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedJob {
    id: JobId,
    source_identity: SourceIdentity,
    media: MediaFile,
    operation: QueuedOperation,
    state: QueueJobState,
}

impl QueuedJob {
    pub(crate) fn new(
        id: JobId,
        source_identity: SourceIdentity,
        media: MediaFile,
        operation: QueuedOperation,
    ) -> Self {
        Self {
            id,
            source_identity,
            media,
            operation,
            state: QueueJobState::Waiting,
        }
    }

    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn media(&self) -> &MediaFile {
        &self.media
    }

    pub fn operation(&self) -> &QueuedOperation {
        &self.operation
    }

    pub fn state(&self) -> &QueueJobState {
        &self.state
    }

    /// Rebuild the normal source workspace from the immutable media and identity snapshots
    /// captured when this job was queued. This does not inspect the source again.
    pub fn retained_intake(&self) -> MediaIntake {
        MediaIntake::from_retained(self.source_identity.clone(), self.media.clone())
    }

    pub(crate) fn source_identity(&self) -> &SourceIdentity {
        &self.source_identity
    }

    pub(crate) fn set_state(&mut self, state: QueueJobState) {
        self.state = state;
    }

    pub(crate) fn replace_operation(&mut self, operation: QueuedOperation) {
        debug_assert!(matches!(self.state, QueueJobState::Waiting));
        self.operation = operation;
    }
}
