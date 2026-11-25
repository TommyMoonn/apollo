use std::{fmt, path::Path};

use crate::{
    audio_extract::{
        execution::{AudioExtractPreparationError, prepare_audio_extract_operation},
        model::AudioExtractRequest,
        validation::validate_audio_extract_request,
    },
    audio_mix::{
        execution::{AudioMixPreparationError, prepare_audio_mix_operation},
        model::AudioMixRequest,
        validation::validate_audio_mix_request,
    },
    frame_capture::{
        execution::{FrameCapturePreparationError, prepare_frame_capture_operation},
        model::FrameCaptureRequest,
        validation::validate_frame_capture_request,
    },
    media::{
        error::MediaError,
        execution::MediaExecutionFailure,
        intake::{MediaIntake, MediaIntakeError, SourceIdentity},
        model::MediaFile,
        operation::PreparedMediaOperation,
        output::{MediaOutputError, OutputCollisionPolicy},
        process::MediaProcessStartError,
        tools::{MediaTools, VideoEncoderCapabilities},
    },
    remux::{
        execution::{RemuxPreparationError, prepare_remux_operation},
        model::RemuxRequest,
        validation::validate_remux_request,
    },
    upscale::{
        execution::{UpscalePreparationError, prepare_upscale_operation},
        model::{UpscaleRequest, VideoEncoderSelection},
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
                    QueuePreparationError::AudioExtract(AudioExtractPreparationError::Validation(
                        error,
                    ))
                })
            }
            Self::Remux(request) => validate_remux_request(request, media).map_err(|error| {
                QueuePreparationError::Remux(RemuxPreparationError::Validation(error))
            }),
            Self::Upscale(request) => validate_upscale_request(request, media).map_err(|error| {
                QueuePreparationError::Upscale(UpscalePreparationError::Validation(error))
            }),
            Self::FrameCapture(request) => {
                validate_frame_capture_request(request, media).map_err(|error| {
                    QueuePreparationError::FrameCapture(FrameCapturePreparationError::Validation(
                        error,
                    ))
                })
            }
        }
    }

    pub(crate) fn requires_encoder_capabilities(&self) -> bool {
        matches!(self, Self::Upscale(request) if request.encoder_settings.selection() != VideoEncoderSelection::Cpu)
    }

    /// Refresh only the discovery snapshot; preserve the selected encoder policy.
    pub(crate) fn refresh_encoder_capabilities(
        &mut self,
        capabilities: Option<&VideoEncoderCapabilities>,
    ) -> Result<(), QueueMutationError> {
        if let Self::Upscale(request) = self
            && request.encoder_settings.selection() != VideoEncoderSelection::Cpu
        {
            let capabilities = capabilities.ok_or(QueueMutationError::CapabilitiesPending)?;
            request.nvenc_capability =
                crate::upscale::model::capability_for_codec(capabilities, request.codec).clone();
        }
        Ok(())
    }

    pub(crate) fn prepare(
        &self,
        tools: &MediaTools,
        media: &MediaFile,
        collision_policy: OutputCollisionPolicy,
    ) -> Result<PreparedMediaOperation, QueuePreparationError> {
        match self {
            Self::AudioMix(request) => {
                prepare_audio_mix_operation(tools, media, request, collision_policy)
                    .map_err(QueuePreparationError::AudioMix)
            }
            Self::Upscale(request) => {
                prepare_upscale_operation(tools, media, request, collision_policy)
                    .map_err(QueuePreparationError::Upscale)
            }
            Self::Remux(request) => {
                prepare_remux_operation(tools, media, request, collision_policy)
                    .map_err(QueuePreparationError::Remux)
            }
            Self::AudioExtract(request) => {
                prepare_audio_extract_operation(tools, media, request, collision_policy)
                    .map_err(QueuePreparationError::AudioExtract)
            }
            Self::FrameCapture(request) => {
                prepare_frame_capture_operation(tools, media, request, collision_policy)
                    .map_err(QueuePreparationError::FrameCapture)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuePreparationError {
    AudioMix(AudioMixPreparationError),
    Upscale(UpscalePreparationError),
    Remux(RemuxPreparationError),
    AudioExtract(AudioExtractPreparationError),
    FrameCapture(FrameCapturePreparationError),
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
    DestinationReserved(JobId),
    Source(MediaIntakeError),
    SourceDoesNotMatchMedia,
    SourceChanged,
    InvalidRequest(QueuePreparationError),
    Output(MediaOutputError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchEnqueueError {
    JobIdExhausted,
    DuplicateDestination {
        index: usize,
    },
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
            Self::DuplicateDestination { .. } => formatter.write_str("another included file uses this output; choose a different destination or skip that file"),
        }
    }
}

impl std::error::Error for BatchEnqueueError {}

impl fmt::Display for QueueEnqueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JobIdExhausted => write!(formatter, "queue job identifiers are exhausted"),
            Self::DestinationReserved(id) => write!(
                formatter,
                "output is reserved by pending job {id}; choose another destination or wait for that job"
            ),
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
    JobNotRetryable(JobId),
    DestinationReserved(JobId),
    CapabilitiesPending,
    JobIdExhausted,
    SourceUnavailable(MediaIntakeError),
    SourceChanged,
    InvalidRequest(QueuePreparationError),
    Output(MediaOutputError),
}

impl fmt::Display for QueueMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AutomaticAdvancementActive => {
                formatter.write_str("Editing requires a stopped queue. Use Stop after current and wait for the active job to finish.")
            }
            Self::JobNotFound(id) => write!(formatter, "queue job {id} was not found"),
            Self::JobNotWaiting(id) => write!(formatter, "queue job {id} is not waiting"),
            Self::JobNotRetryable(id) => write!(formatter, "job {id} is not failed or cancelled"),
            Self::DestinationReserved(id) => write!(formatter, "output is reserved by pending job {id}; choose another destination or wait for that job"),
            Self::CapabilitiesPending => formatter.write_str("Wait for NVIDIA NVENC discovery to finish, then retry."),
            Self::JobIdExhausted => formatter.write_str("queue job identifiers are exhausted"),
            Self::SourceUnavailable(error) => {
                write!(formatter, "queued source is unavailable: {error}; reopen the source before submitting a new job")
            }
            Self::SourceChanged => formatter.write_str("queued source changed or cannot be verified; reopen the source before submitting a new job"),
            Self::InvalidRequest(error) => {
                write!(formatter, "invalid queued operation: {error}")
            }
            Self::Output(error) => write!(formatter, "queue output is invalid: {error}"),
        }
    }
}

impl std::error::Error for QueueMutationError {}

impl From<QueueEnqueueError> for QueueMutationError {
    fn from(error: QueueEnqueueError) -> Self {
        match error {
            QueueEnqueueError::JobIdExhausted => Self::JobIdExhausted,
            QueueEnqueueError::DestinationReserved(id) => Self::DestinationReserved(id),
            QueueEnqueueError::Source(error) => Self::SourceUnavailable(error),
            QueueEnqueueError::SourceChanged | QueueEnqueueError::SourceDoesNotMatchMedia => {
                Self::SourceChanged
            }
            QueueEnqueueError::InvalidRequest(error) => Self::InvalidRequest(error),
            QueueEnqueueError::Output(error) => Self::Output(error),
        }
    }
}

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
    collision_policy: OutputCollisionPolicy,
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
            collision_policy: OutputCollisionPolicy::Numbered,
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

    pub(crate) fn collision_policy(&self) -> OutputCollisionPolicy {
        self.collision_policy
    }

    pub(crate) fn with_collision_policy(mut self, policy: OutputCollisionPolicy) -> Self {
        self.collision_policy = policy;
        self
    }

    pub(crate) fn replace_operation(&mut self, operation: QueuedOperation) {
        debug_assert!(matches!(self.state, QueueJobState::Waiting));
        self.operation = operation;
    }
}
