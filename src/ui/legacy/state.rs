use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, mpsc};
use std::time::Instant;
use std::{io, thread};

use egui::TextureHandle;

use crate::app::{BatchOrigin, Operation};
use crate::audio_extract::model::AudioExtractFormat;
use crate::audio_mix::model::{LoudnessNormalization, OutputContainer};
use crate::frame_capture::frame_index::{
    FrameDirection, FrameIndexState, FrameNavigation, FrameNavigator, FrameStep,
};
use crate::frame_capture::model::{FrameCaptureFormat, FramePosition, FrameTimestamp};
use crate::frame_capture::preview::{
    FramePreview, PreviewExtractionError, PreviewGeneration, PreviewImage, PreviewRequest,
    decode_preview_png,
};
use crate::frame_capture::thumbnail::{FrameCaptureThumbnail, decode_frame_capture_thumbnail};
use crate::media::batch::BatchInspectionOwner;
use crate::media::container::MediaContainer;
use crate::media::error::MediaError;
use crate::media::intake::SourceIdentity;
use crate::media::model::MediaFile;
use crate::media::process::MediaProcessEvent;
use crate::queue::{JobId, QueuedJob, QueuedOperation};
use crate::ui::audio_extract::AudioExtractSelectionState;
use crate::ui::audio_extract_progress::AudioExtractLifecycleUiState;
use crate::ui::audio_mix::AudioMixSelectionState;
use crate::ui::audio_mix_progress::AudioMixLifecycleUiState;
use crate::ui::batch::BatchReadiness;
use crate::ui::frame_capture::{
    FrameCaptureSelectionState, FrameCopyStatus, FrameNavigationAvailability, PositionSettle,
};
use crate::ui::frame_capture_progress::{
    FrameCaptureLifecycleUiState, FrameCaptureThumbnailRequest,
};
use crate::ui::remux::RemuxSelectionState;
use crate::ui::remux_progress::RemuxLifecycleUiState;
use crate::ui::source_presentation::SourcePresentation;
use crate::ui::upscale::UpscaleSelectionState;
use crate::ui::upscale_progress::UpscaleLifecycleUiState;
use crate::upscale::model::{UpscaleEncoderSettings, UpscaleTarget, VideoCodec};

use super::workflows::queued_operation_tool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AudioMixToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: AudioMixSelectionState,
    pub(super) lifecycle_ui: AudioMixLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpscaleToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: UpscaleSelectionState,
    pub(super) lifecycle_ui: UpscaleLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemuxToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: RemuxSelectionState,
    pub(super) lifecycle_ui: RemuxLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AudioExtractToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: AudioExtractSelectionState,
    pub(super) lifecycle_ui: AudioExtractLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrameCaptureToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) editor: FrameCaptureEditorState,
    pub(super) lifecycle_ui: FrameCaptureLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
    pub(super) copy_status: FrameCopyStatus,
}

impl Deref for FrameCaptureToolState {
    type Target = FrameCaptureEditorState;

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
pub(super) struct FrameCaptureEditorState {
    pub(super) selection: FrameCaptureSelectionState,
    pub(super) session_id: u64,
    pub(super) source_identity: Option<SourceIdentity>,
    pub(super) preview: FramePreview<PreviewImage>,
    pub(super) preview_texture: Option<FramePreviewTexture>,
    pub(super) frame_navigator: FrameNavigator,
    pub(super) frame_index_due: Option<(FramePosition, Instant)>,
    pub(super) pending_step: Option<PendingFrameStep>,
}

impl FrameCaptureEditorState {
    pub(super) fn new(
        session_id: u64,
        source_identity: Option<SourceIdentity>,
        media: &MediaFile,
    ) -> Self {
        Self {
            selection: FrameCaptureSelectionState::new(media),
            session_id,
            source_identity,
            preview: FramePreview::default(),
            preview_texture: None,
            frame_navigator: FrameNavigator::default(),
            frame_index_due: None,
            pending_step: None,
        }
    }

    pub(super) fn hydrate(
        &mut self,
        media: &MediaFile,
        request: &crate::frame_capture::model::FrameCaptureRequest,
    ) {
        self.selection = FrameCaptureSelectionState::from_request(media, request);
    }
}

impl Deref for FrameCaptureEditorState {
    type Target = FrameCaptureSelectionState;

    fn deref(&self) -> &Self::Target {
        &self.selection
    }
}

impl DerefMut for FrameCaptureEditorState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.selection
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct FramePreviewTexture {
    pub(super) generation: PreviewGeneration,
    pub(super) size: [usize; 2],
    pub(super) position: FramePosition,
    pub(super) handle: TextureHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingFrameStep {
    pub(super) origin: FramePosition,
    pub(super) direction: FrameDirection,
    pub(super) step: FrameStep,
}

pub(super) fn frame_navigation_availability(
    media: &MediaFile,
    editor: &FrameCaptureEditorState,
) -> FrameNavigationAvailability {
    if editor.source_identity.is_none() {
        return FrameNavigationAvailability {
            previous: false,
            next: false,
            loading: false,
            failed: false,
        };
    }
    let Some(position) = editor.selection.valid_position(media) else {
        return FrameNavigationAvailability {
            previous: false,
            next: false,
            loading: false,
            failed: false,
        };
    };
    let loading = editor.frame_index_due.is_some()
        || matches!(
            editor.frame_navigator.state(),
            FrameIndexState::Loading { .. }
        );
    let failed = matches!(
        editor.frame_navigator.state(),
        FrameIndexState::Failed { .. }
    );
    let step = editor.selection.step();
    let previous = !matches!(
        editor
            .frame_navigator
            .navigate(position, FrameDirection::Previous, step),
        FrameNavigation::Boundary
    );
    let next = !matches!(
        editor
            .frame_navigator
            .navigate(position, FrameDirection::Next, step),
        FrameNavigation::Boundary
    );
    FrameNavigationAvailability {
        previous,
        next,
        loading,
        failed,
    }
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
    pub(super) audio_mix: AudioMixSelectionState,
    pub(super) upscale: UpscaleSelectionState,
    pub(super) remux: RemuxSelectionState,
    pub(super) audio_extract: AudioExtractSelectionState,
    pub(super) frame_capture: FrameCaptureEditorState,
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
            audio_mix: AudioMixSelectionState::new(&media),
            upscale: UpscaleSelectionState::new(&media),
            remux: RemuxSelectionState::new(&media),
            audio_extract: AudioExtractSelectionState::new(&media),
            frame_capture: FrameCaptureEditorState::new(session_id, source_identity, &media),
            media,
            error: None,
            error_details_open: false,
        }
    }

    pub(super) fn hydrate(&mut self, operation: &QueuedOperation) {
        match operation {
            QueuedOperation::AudioMix(request) => {
                self.audio_mix = AudioMixSelectionState::from_request(&self.media, request);
            }
            QueuedOperation::Upscale(request) => {
                self.upscale = UpscaleSelectionState::from_request(&self.media, request);
            }
            QueuedOperation::Remux(request) => {
                self.remux = RemuxSelectionState::from_request(&self.media, request);
            }
            QueuedOperation::AudioExtract(request) => {
                self.audio_extract = AudioExtractSelectionState::from_request(&self.media, request);
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
            queued_operation_tool(job.operation()),
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

pub(super) struct FrameCaptureThumbnailLoad {
    pub(super) session_id: u64,
    pub(super) request_id: u64,
    pub(super) receiver: Receiver<Result<FrameCaptureThumbnail, String>>,
}

pub(super) struct FramePreviewDecode {
    pub(super) request: PreviewRequest,
    pub(super) receiver: Receiver<Result<PreviewImage, PreviewExtractionError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QueueExecutionEvent {
    pub(super) job_id: JobId,
    pub(super) process_event: MediaProcessEvent,
}

impl FrameCaptureThumbnailLoad {
    pub(super) fn spawn(
        session_id: u64,
        request: &FrameCaptureThumbnailRequest,
        wake: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        let output = request.output().to_path_buf();
        Self::spawn_with(session_id, request.request_id(), wake, move || {
            decode_frame_capture_thumbnail(&output)
        })
    }

    pub(super) fn spawn_with<F>(
        session_id: u64,
        request_id: u64,
        wake: impl FnOnce() + Send + 'static,
        decode: F,
    ) -> io::Result<Self>
    where
        F: FnOnce() -> Result<FrameCaptureThumbnail, String> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-frame-thumbnail".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode));
                if let Ok(result) = result {
                    let _ = sender.send(result);
                }
                drop(sender);
                wake();
            })?;
        Ok(Self {
            session_id,
            request_id,
            receiver,
        })
    }
}

impl FramePreviewDecode {
    pub(super) fn spawn(
        request: PreviewRequest,
        bytes: Vec<u8>,
        wake: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with(request, wake, move || decode_preview_png(&bytes))
    }

    pub(super) fn spawn_with<F>(
        request: PreviewRequest,
        wake: impl FnOnce() + Send + 'static,
        decode: F,
    ) -> io::Result<Self>
    where
        F: FnOnce() -> Result<PreviewImage, PreviewExtractionError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-frame-preview-decode".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode))
                    .unwrap_or_else(|_| {
                        Err(PreviewExtractionError::Decode(
                            "preview decode worker stopped unexpectedly".to_owned(),
                        ))
                    });
                let _ = sender.send(result);
                wake();
            })?;
        Ok(Self { request, receiver })
    }
}
