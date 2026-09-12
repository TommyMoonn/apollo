use std::{fmt, path::Path};

use crate::{
    audio_extract::{
        execution::{AudioExtractStartError, prepare_audio_extract_operation},
        model::AudioExtractRequest,
    },
    audio_mix::{
        execution::{AudioMixStartError, prepare_audio_mix_operation},
        model::AudioMixRequest,
    },
    frame_capture::{
        execution::{FrameCaptureStartError, prepare_frame_capture_operation},
        model::FrameCaptureRequest,
    },
    media::{
        execution::MediaExecutionFailure,
        intake::{MediaIntakeError, SourceIdentity},
        model::MediaFile,
        operation::PreparedMediaOperation,
        output::MediaOutputError,
        process::MediaProcessStartError,
        tools::MediaTools,
    },
    remux::{
        execution::{RemuxStartError, prepare_remux_operation},
        model::RemuxRequest,
    },
    upscale::{
        execution::{UpscaleStartError, prepare_upscale_operation},
        model::UpscaleRequest,
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
    AudioMix(AudioMixStartError),
    Upscale(UpscaleStartError),
    Remux(RemuxStartError),
    AudioExtract(AudioExtractStartError),
    FrameCapture(FrameCaptureStartError),
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
    Running,
    Completed { output: std::path::PathBuf },
    Failed(QueueJobFailure),
    Cancelled,
}

impl QueueJobState {
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
    Output(MediaOutputError),
    DuplicateDestination { existing_job: JobId },
}

impl fmt::Display for QueueEnqueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JobIdExhausted => write!(formatter, "queue job identifiers are exhausted"),
            Self::Source(error) => {
                write!(formatter, "source identity could not be captured: {error}")
            }
            Self::SourceDoesNotMatchMedia => {
                write!(
                    formatter,
                    "operation source does not match the probed media"
                )
            }
            Self::Output(error) => write!(formatter, "queue output is invalid: {error}"),
            Self::DuplicateDestination { existing_job } => write!(
                formatter,
                "queue job {existing_job} already targets this output"
            ),
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
    Output(MediaOutputError),
    DuplicateDestination { existing_job: JobId },
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
            Self::Output(error) => write!(formatter, "replacement output is invalid: {error}"),
            Self::DuplicateDestination { existing_job } => write!(
                formatter,
                "queue job {existing_job} already targets this output"
            ),
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
    normalized_output: std::path::PathBuf,
    state: QueueJobState,
}

impl QueuedJob {
    pub(crate) fn new(
        id: JobId,
        source_identity: SourceIdentity,
        media: MediaFile,
        operation: QueuedOperation,
        normalized_output: std::path::PathBuf,
    ) -> Self {
        Self {
            id,
            source_identity,
            media,
            operation,
            normalized_output,
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

    pub(crate) fn source_identity(&self) -> &SourceIdentity {
        &self.source_identity
    }

    pub(crate) fn normalized_output(&self) -> &Path {
        &self.normalized_output
    }

    pub(crate) fn set_state(&mut self, state: QueueJobState) {
        self.state = state;
    }

    pub(crate) fn replace_operation(
        &mut self,
        operation: QueuedOperation,
        normalized_output: std::path::PathBuf,
    ) {
        debug_assert!(matches!(self.state, QueueJobState::Waiting));
        self.operation = operation;
        self.normalized_output = normalized_output;
    }
}
