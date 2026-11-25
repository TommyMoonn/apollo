use super::audio_extract::AudioExtractToolState;
use super::audio_mix::AudioMixToolState;
use super::remux::RemuxToolState;
use super::upscale::UpscaleToolState;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::Arc;

use egui::TextureHandle;

use crate::app::frame_capture::{FrameCaptureEditorState, PositionSettle};
use crate::app::{BatchOrigin, Operation};
use crate::audio_extract::draft::AudioExtractDraft;
use crate::audio_extract::model::AudioExtractFormat;
use crate::audio_mix::draft::AudioMixDraft;
use crate::audio_mix::model::{LoudnessNormalization, OutputContainer};
use crate::frame_capture::frame_index::FrameDirection;
use crate::frame_capture::model::{FrameCaptureFormat, FramePosition, FrameTimestamp};
use crate::frame_capture::preview::PreviewGeneration;
use crate::media::batch::BatchInspectionOwner;
use crate::media::container::MediaContainer;
use crate::media::error::MediaError;
use crate::media::intake::SourceIdentity;
use crate::media::model::MediaFile;
use crate::media::process::MediaProcessEvent;
use crate::queue::{JobId, QueuedJob, QueuedOperation};
use crate::remux::draft::RemuxDraft;
use crate::ui::batch::BatchReadiness;
use crate::ui::frame_capture_progress::FrameCaptureLifecycleUiState;
use crate::ui::source_presentation::SourcePresentation;
use crate::upscale::draft::UpscaleDraft;
use crate::upscale::model::{UpscaleEncoderSettings, UpscaleTarget, VideoCodec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrameCaptureToolState {
    pub(super) media: MediaFile,
    pub(super) editor: FrameCaptureEditor,
    pub(super) lifecycle_ui: FrameCaptureLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

impl Deref for FrameCaptureToolState {
    type Target = FrameCaptureEditor;

    fn deref(&self) -> &Self::Target {
        &self.editor
    }
}

impl DerefMut for FrameCaptureToolState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.editor
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrameCaptureEditor {
    pub(super) state: FrameCaptureEditorState,
    pub(super) preview_texture: Option<FramePreviewTexture>,
}
impl FrameCaptureEditor {
    pub(super) fn new(
        session_id: u64,
        source_identity: Option<SourceIdentity>,
        media: &MediaFile,
    ) -> Self {
        Self {
            state: FrameCaptureEditorState::new(session_id, source_identity, media),
            preview_texture: None,
        }
    }
}
impl Deref for FrameCaptureEditor {
    type Target = FrameCaptureEditorState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
impl DerefMut for FrameCaptureEditor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct FramePreviewTexture {
    pub(super) generation: PreviewGeneration,
    pub(super) size: [usize; 2],
    pub(super) position: FramePosition,
    pub(super) handle: TextureHandle,
}

impl std::fmt::Debug for FramePreviewTexture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FramePreviewTexture")
            .field("generation", &self.generation)
            .field("size", &self.size)
            .field("position", &self.position)
            .field("texture_id", &self.handle.id())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OperationEditorState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) presentation: Arc<SourcePresentation>,
    pub(super) active_operation: Operation,
    pub(super) audio_mix: AudioMixDraft,
    pub(super) upscale: UpscaleDraft,
    pub(super) remux: RemuxDraft,
    pub(super) audio_extract: AudioExtractDraft,
    pub(super) frame_capture: FrameCaptureEditor,
    pub(super) error: Option<MediaError>,
    pub(super) error_details_open: bool,
}

impl OperationEditorState {
    pub(super) fn new(
        session_id: u64,
        source_identity: Option<SourceIdentity>,
        media: MediaFile,
        active_operation: Operation,
    ) -> Self {
        Self {
            session_id,
            presentation: Arc::new(SourcePresentation::new(&media)),
            active_operation,
            audio_mix: AudioMixDraft::new(&media),
            upscale: UpscaleDraft::new(&media),
            remux: RemuxDraft::new(&media),
            audio_extract: AudioExtractDraft::new(&media),
            frame_capture: FrameCaptureEditor::new(session_id, source_identity, &media),
            media,
            error: None,
            error_details_open: false,
        }
    }

    pub(super) fn hydrate(&mut self, operation: &QueuedOperation) {
        match operation {
            QueuedOperation::AudioMix(request) => {
                self.audio_mix = AudioMixDraft::from_request(&self.media, request);
            }
            QueuedOperation::Upscale(request) => {
                self.upscale = UpscaleDraft::from_request(&self.media, request);
            }
            QueuedOperation::Remux(request) => {
                self.remux = RemuxDraft::from_request(&self.media, request);
            }
            QueuedOperation::AudioExtract(request) => {
                self.audio_extract = AudioExtractDraft::from_request(&self.media, request);
            }
            QueuedOperation::FrameCapture(request) => {
                self.frame_capture.hydrate(&self.media, request);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueEditState {
    pub(super) job_id: JobId,
    pub(super) editor: OperationEditorState,
}

impl Deref for QueueEditState {
    type Target = OperationEditorState;

    fn deref(&self) -> &Self::Target {
        &self.editor
    }
}

impl DerefMut for QueueEditState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.editor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueueEditAction {
    None,
    BrowseOutput,
    Save,
    Cancel,
    FramePositionEdited(PositionSettle),
    FrameStepRequested(FrameDirection),
}

impl QueueEditState {
    pub(super) fn new(session_id: u64, job: &QueuedJob) -> Self {
        let media = job.media().clone();
        let source_identity = job.source_identity().clone();
        let mut editor = OperationEditorState::new(
            session_id,
            Some(source_identity),
            media,
            Operation::from(job.operation()),
        );
        editor.hydrate(job.operation());
        Self {
            job_id: job.id(),
            editor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BatchSourceState {
    Inspecting,
    Failed,
    Ready {
        source_identity: SourceIdentity,
        media: MediaFile,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BatchDefaults {
    AudioMix {
        container: OutputContainer,
        normalization: LoudnessNormalization,
    },
    Upscale {
        target: UpscaleTarget,
        codec: VideoCodec,
        encoder_settings: UpscaleEncoderSettings,
    },
    Remux {
        container: MediaContainer,
    },
    AudioExtract {
        format: AudioExtractFormat,
    },
    FrameCapture {
        timestamp: FrameTimestamp,
        format: FrameCaptureFormat,
    },
}

impl BatchDefaults {
    pub(super) const fn operation(&self) -> Operation {
        match self {
            Self::AudioMix { .. } => Operation::AudioMix,
            Self::Upscale { .. } => Operation::VideoUpscale,
            Self::Remux { .. } => Operation::Remux,
            Self::AudioExtract { .. } => Operation::AudioExtract,
            Self::FrameCapture { .. } => Operation::FrameCapture,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BatchItem {
    pub(super) path: PathBuf,
    pub(super) included: bool,
    pub(super) source: BatchSourceState,
    pub(super) operation: Option<QueuedOperation>,
    pub(super) readiness: BatchReadiness,
    pub(super) reason: Option<String>,
    pub(super) overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BatchSetupState {
    pub(super) origin: BatchOrigin,
    pub(super) operation: Operation,
    pub(super) defaults: Option<BatchDefaults>,
    pub(super) items: Vec<BatchItem>,
    pub(super) error: Option<String>,
    pub(super) append_error: Option<MediaError>,
    pub(super) append_error_details_open: bool,
}

pub(super) struct BatchInspectionSession {
    pub(super) owner: BatchInspectionOwner,
    pub(super) item_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BatchEditTarget {
    Defaults,
    Item(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BatchEditState {
    pub(super) target: BatchEditTarget,
    pub(super) editor: OperationEditorState,
}

impl Deref for BatchEditState {
    type Target = OperationEditorState;

    fn deref(&self) -> &Self::Target {
        &self.editor
    }
}

impl DerefMut for BatchEditState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.editor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct LegacyEditorState {
    pub(super) intake_error_details_open: bool,
    pub(super) source_presentation: Option<Arc<SourcePresentation>>,
    pub(super) audio_mix: Option<AudioMixToolState>,
    pub(super) upscale: Option<UpscaleToolState>,
    pub(super) remux: Option<RemuxToolState>,
    pub(super) audio_extract: Option<AudioExtractToolState>,
    pub(super) frame_capture: Option<FrameCaptureToolState>,
}

impl LegacyEditorState {
    pub(super) fn retire_tool_sessions(&mut self) {
        self.audio_mix = None;
        self.upscale = None;
        self.remux = None;
        self.audio_extract = None;
        self.frame_capture = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QueueExecutionEvent {
    pub(super) job_id: JobId,
    pub(super) process_event: MediaProcessEvent,
}
