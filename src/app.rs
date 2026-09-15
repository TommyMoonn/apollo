use std::{
    io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Instant,
};

use egui::{CentralPanel, ColorImage, Context, TextureHandle, TextureOptions, Ui};

use crate::file_dialog::{
    DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogOwner, FileDialogRequest,
    FileDialogStartError, OutputFormat, SaveDialogOptions, WindowOwner,
};
use crate::progress_repaint::MediaProgressRepaintOwner;
use crate::settings::AppSettings;

use apollo::{
    audio_extract::{
        execution::{AudioExtractExecutionState, AudioExtractExecutor, AudioExtractStartError},
        model::AudioExtractFormat,
    },
    audio_mix::{
        execution::{AudioMixExecutionState, AudioMixExecutor, AudioMixStartError},
        model::{LoudnessNormalization, OutputContainer},
    },
    frame_capture::{
        execution::{FrameCaptureExecutionState, FrameCaptureExecutor, FrameCaptureStartError},
        model::{FrameCaptureFormat, FrameTimestamp},
        preview::{
            FramePreview, PREVIEW_DEBOUNCE, PreviewExtractionError, PreviewExtractionOwner,
            PreviewGeneration, PreviewImage, PreviewRequest, PreviewState, decode_preview_png,
        },
        thumbnail::{FrameCaptureThumbnail, decode_frame_capture_thumbnail},
    },
    media::{
        batch::BatchInspectionOwner,
        container::MediaContainer,
        error::{MediaError, MediaErrorKind},
        intake::SourceIdentity,
        intake::{MediaIntake, MediaIntakeRequest, MediaLoadStart, MediaProbeDisposition},
        model::MediaFile,
        probe::{MediaProbeError, probe_media},
        process::{MediaProcessEvent, MediaProcessWake},
        tools::{
            MediaTools, MediaToolsError, VideoEncoderCapabilities, VideoEncoderCapability,
            cached_video_encoder_capabilities, discover_base_media_tools,
        },
    },
    queue::{
        JobId, MediaQueue, QueueEnqueueError, QueueJobState, QueueRunState, QueuedJob,
        QueuedOperation, WaitingJobMove,
    },
    remux::execution::{RemuxExecutionState, RemuxExecutor, RemuxStartError},
    ui::{
        OperationSurfaceMode, ProcessExecutionArbitration, QueueAction, SourceAction,
        audio_extract as audio_extract_ui,
        audio_extract::{AudioExtractSelectionAction, AudioExtractSelectionState},
        audio_extract_progress::{self, AudioExtractLifecycleAction, AudioExtractLifecycleUiState},
        audio_mix as audio_mix_ui,
        audio_mix::{AudioMixSelectionAction, AudioMixSelectionState},
        audio_mix_progress::{self, AudioMixLifecycleAction, AudioMixLifecycleUiState},
        batch::{
            self as batch_ui, BatchAction, BatchItemEditAction, BatchItemView, BatchReadiness,
        },
        file_intake::{self, FileIntakeAction},
        fonts, frame_capture as frame_capture_ui,
        frame_capture::{FrameCaptureSelectionAction, FrameCaptureSelectionState},
        frame_capture_progress::{
            self, FrameCaptureLifecycleAction, FrameCaptureLifecycleUiState,
            FrameCaptureThumbnailRequest,
        },
        layout, media_error, queue as queue_ui, remux as remux_ui,
        remux::{RemuxSelectionAction, RemuxSelectionState},
        remux_progress::{self, RemuxLifecycleAction, RemuxLifecycleUiState},
        source_presentation::SourcePresentation,
        theme, upscale as upscale_ui,
        upscale::{UpscaleSelectionAction, UpscaleSelectionState},
        upscale_progress::{self, UpscaleLifecycleAction, UpscaleLifecycleUiState},
        widgets,
        workspace::{self, EditableQueuedJob, Operation as Tool, WorkspaceAction, WorkspaceMode},
    },
    upscale::execution::{UpscaleExecutionState, UpscaleExecutor, UpscaleStartError},
    upscale::{
        model::{UpscaleEncoderSettings, UpscaleTarget, VideoCodec},
        validation::validate_upscale_request,
    },
};

fn file_intake_enabled(
    audio_state: &AudioMixExecutionState,
    upscale_state: &UpscaleExecutionState,
    remux_state: &RemuxExecutionState,
    audio_extract_state: &AudioExtractExecutionState,
    frame_capture_state: &FrameCaptureExecutionState,
) -> bool {
    // Done/Back owns terminal settlement. Intake resumes only after every executor returns to Idle.
    matches!(audio_state, AudioMixExecutionState::Idle)
        && matches!(upscale_state, UpscaleExecutionState::Idle)
        && matches!(remux_state, RemuxExecutionState::Idle)
        && matches!(audio_extract_state, AudioExtractExecutionState::Idle)
        && matches!(frame_capture_state, FrameCaptureExecutionState::Idle)
}

fn operation_workspace_visible(
    execution_state: &AudioMixExecutionState,
    has_ready_session: bool,
) -> bool {
    matches!(execution_state, AudioMixExecutionState::Idle) && has_ready_session
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum NavigationState {
    #[default]
    NoSource,
    Queue {
        origin: QueueOrigin,
    },
    QueueEdit {
        origin: QueueEditOrigin,
    },
    BatchSetup,
    BatchEdit,
    SourceWorkspace {
        active_operation: Tool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueOrigin {
    NoSource,
    SourceWorkspace { active_operation: Tool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueEditOrigin {
    Queue(QueueOrigin),
    SourceWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchOrigin {
    NoSource,
    Queue { origin: QueueOrigin },
    SourceWorkspace { active_operation: Tool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioMixToolState {
    session_id: u64,
    media: MediaFile,
    selection: AudioMixSelectionState,
    lifecycle_ui: AudioMixLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpscaleToolState {
    session_id: u64,
    media: MediaFile,
    selection: UpscaleSelectionState,
    lifecycle_ui: UpscaleLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemuxToolState {
    session_id: u64,
    media: MediaFile,
    selection: RemuxSelectionState,
    lifecycle_ui: RemuxLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioExtractToolState {
    session_id: u64,
    media: MediaFile,
    selection: AudioExtractSelectionState,
    lifecycle_ui: AudioExtractLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameCaptureToolState {
    session_id: u64,
    source_identity: SourceIdentity,
    media: MediaFile,
    selection: FrameCaptureSelectionState,
    preview: FramePreview<PreviewImage>,
    preview_texture: Option<FramePreviewTexture>,
    lifecycle_ui: FrameCaptureLifecycleUiState,
    start_error: Option<MediaError>,
    start_error_details_open: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct FramePreviewTexture {
    generation: PreviewGeneration,
    size: [usize; 2],
    handle: TextureHandle,
}

impl std::fmt::Debug for FramePreviewTexture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FramePreviewTexture")
            .field("generation", &self.generation)
            .field("size", &self.size)
            .field("texture_id", &self.handle.id())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperationEditorState {
    session_id: u64,
    media: MediaFile,
    presentation: Arc<SourcePresentation>,
    active_operation: Tool,
    audio_mix: AudioMixSelectionState,
    upscale: UpscaleSelectionState,
    remux: RemuxSelectionState,
    audio_extract: AudioExtractSelectionState,
    frame_capture: FrameCaptureSelectionState,
    error: Option<MediaError>,
    error_details_open: bool,
}

impl OperationEditorState {
    fn new(session_id: u64, media: MediaFile, active_operation: Tool) -> Self {
        Self {
            session_id,
            presentation: Arc::new(SourcePresentation::new(&media)),
            active_operation,
            audio_mix: AudioMixSelectionState::new(&media),
            upscale: UpscaleSelectionState::new(&media),
            remux: RemuxSelectionState::new(&media),
            audio_extract: AudioExtractSelectionState::new(&media),
            frame_capture: FrameCaptureSelectionState::new(&media),
            media,
            error: None,
            error_details_open: false,
        }
    }

    fn hydrate(&mut self, operation: &QueuedOperation) {
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
                self.frame_capture = FrameCaptureSelectionState::from_request(&self.media, request);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueEditState {
    job_id: JobId,
    editor: OperationEditorState,
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
enum QueueEditAction {
    None,
    BrowseOutput,
    Save,
    Cancel,
}

impl QueueEditState {
    fn new(session_id: u64, job: &QueuedJob) -> Self {
        let media = job.media().clone();
        let mut editor =
            OperationEditorState::new(session_id, media, queued_operation_tool(job.operation()));
        editor.hydrate(job.operation());
        Self {
            job_id: job.id(),
            editor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BatchSourceState {
    Inspecting,
    Failed,
    Ready(MediaFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BatchDefaults {
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
    const fn operation(&self) -> Tool {
        match self {
            Self::AudioMix { .. } => Tool::AudioMix,
            Self::Upscale { .. } => Tool::VideoUpscale,
            Self::Remux { .. } => Tool::Remux,
            Self::AudioExtract { .. } => Tool::AudioExtract,
            Self::FrameCapture { .. } => Tool::FrameCapture,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BatchItem {
    path: PathBuf,
    included: bool,
    source: BatchSourceState,
    operation: Option<QueuedOperation>,
    readiness: BatchReadiness,
    reason: Option<String>,
    overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BatchSetupState {
    origin: BatchOrigin,
    operation: Tool,
    defaults: Option<BatchDefaults>,
    items: Vec<BatchItem>,
    error: Option<String>,
    append_error: Option<MediaError>,
    append_error_details_open: bool,
}

struct BatchInspectionSession {
    owner: BatchInspectionOwner,
    item_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchEditTarget {
    Defaults,
    Item(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BatchEditState {
    target: BatchEditTarget,
    editor: OperationEditorState,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppState {
    navigation: NavigationState,
    intake_error: Option<MediaError>,
    intake_error_details_open: bool,
    media_intake: MediaIntake,
    source_presentation: Option<Arc<SourcePresentation>>,
    audio_mix: Option<AudioMixToolState>,
    upscale: Option<UpscaleToolState>,
    remux: Option<RemuxToolState>,
    audio_extract: Option<AudioExtractToolState>,
    frame_capture: Option<FrameCaptureToolState>,
    queue_confirmation: Option<Tool>,
    operation_action_error: Option<(Tool, MediaError)>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            navigation: NavigationState::NoSource,
            intake_error: None,
            intake_error_details_open: false,
            media_intake: MediaIntake::default(),
            source_presentation: None,
            audio_mix: None,
            upscale: None,
            remux: None,
            audio_extract: None,
            frame_capture: None,
            queue_confirmation: None,
            operation_action_error: None,
        }
    }
}

struct MediaInspection {
    request: MediaIntakeRequest,
    receiver: Receiver<Result<MediaFile, MediaProbeError>>,
}

struct FrameCaptureThumbnailLoad {
    session_id: u64,
    request_id: u64,
    receiver: Receiver<Result<FrameCaptureThumbnail, String>>,
}

struct FramePreviewDecode {
    request: PreviewRequest,
    receiver: Receiver<Result<PreviewImage, PreviewExtractionError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueExecutionEvent {
    job_id: JobId,
    process_event: MediaProcessEvent,
}

impl FrameCaptureThumbnailLoad {
    fn spawn(
        session_id: u64,
        request: &FrameCaptureThumbnailRequest,
        context: Context,
    ) -> io::Result<Self> {
        let output = request.output().to_path_buf();
        Self::spawn_with(session_id, request.request_id(), context, move || {
            decode_frame_capture_thumbnail(&output)
        })
    }

    fn spawn_with<F>(
        session_id: u64,
        request_id: u64,
        context: Context,
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
                context.request_repaint();
            })?;
        Ok(Self {
            session_id,
            request_id,
            receiver,
        })
    }
}

impl FramePreviewDecode {
    fn spawn(request: PreviewRequest, bytes: Vec<u8>, context: Context) -> io::Result<Self> {
        Self::spawn_with(request, context, move || decode_preview_png(&bytes))
    }

    fn spawn_with<F>(request: PreviewRequest, context: Context, decode: F) -> io::Result<Self>
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
                context.request_repaint();
            })?;
        Ok(Self { request, receiver })
    }
}

impl MediaInspection {
    fn spawn(tools: MediaTools, request: MediaIntakeRequest, context: Context) -> io::Result<Self> {
        let path = request.path().to_path_buf();
        Self::spawn_with(request, context, move || probe_media(&tools, &path))
    }

    fn spawn_with<F>(request: MediaIntakeRequest, context: Context, inspect: F) -> io::Result<Self>
    where
        F: FnOnce() -> Result<MediaFile, MediaProbeError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-media-inspection".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(inspect));
                if let Ok(result) = result {
                    let _ = sender.send(result);
                }
                drop(sender);
                context.request_repaint();
            })?;

        Ok(Self { request, receiver })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MediaToolsState {
    Checking { attempt_id: u64 },
    Ready(MediaTools),
    Unavailable(MediaToolsError),
}

struct MediaToolsDiscovery {
    attempt_id: u64,
    receiver: Receiver<Result<MediaTools, MediaToolsError>>,
    base_receiver: Option<Receiver<MediaTools>>,
}

impl MediaToolsDiscovery {
    fn spawn<F>(attempt_id: u64, context: Context, discover: F) -> io::Result<Self>
    where
        F: FnOnce() -> Result<MediaTools, MediaToolsError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("apollo-media-tools".to_owned())
            .spawn(move || {
                let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(discover))
                {
                    Ok(result) => result,
                    Err(_) => Err(MediaToolsError::DiscoveryFailed(
                        "The media tool discovery worker stopped unexpectedly.".to_owned(),
                    )),
                };
                let _ = sender.send(result);
                context.request_repaint();
            })?;

        Ok(Self {
            attempt_id,
            receiver,
            base_receiver: None,
        })
    }
}

impl AppState {
    fn active_operation(&self) -> Option<Tool> {
        match self.navigation {
            NavigationState::NoSource
            | NavigationState::Queue { .. }
            | NavigationState::QueueEdit { .. }
            | NavigationState::BatchSetup
            | NavigationState::BatchEdit => None,
            NavigationState::SourceWorkspace { active_operation } => Some(active_operation),
        }
    }

    fn enter_workspace(&mut self, active_operation: Tool) {
        self.navigation = NavigationState::SourceWorkspace { active_operation };
        self.intake_error = None;
        self.intake_error_details_open = false;
    }

    fn enter_queue(&mut self) {
        let origin = match self.navigation {
            NavigationState::NoSource => QueueOrigin::NoSource,
            NavigationState::SourceWorkspace { active_operation } => {
                QueueOrigin::SourceWorkspace { active_operation }
            }
            NavigationState::Queue { origin } => origin,
            NavigationState::QueueEdit {
                origin: QueueEditOrigin::Queue(origin),
            } => origin,
            NavigationState::QueueEdit {
                origin: QueueEditOrigin::SourceWorkspace,
            } => QueueOrigin::NoSource,
            NavigationState::BatchSetup | NavigationState::BatchEdit => QueueOrigin::NoSource,
        };
        self.navigation = NavigationState::Queue { origin };
        self.intake_error = None;
        self.intake_error_details_open = false;
        self.queue_confirmation = None;
        self.operation_action_error = None;
    }

    fn close_source(&mut self) {
        self.navigation = NavigationState::NoSource;
        self.media_intake.close_source();
        self.source_presentation = None;
        self.retire_tool_sessions();
        self.intake_error = None;
        self.intake_error_details_open = false;
        self.queue_confirmation = None;
        self.operation_action_error = None;
    }

    fn retire_tool_sessions(&mut self) {
        self.audio_mix = None;
        self.upscale = None;
        self.remux = None;
        self.audio_extract = None;
        self.frame_capture = None;
    }
}

pub(crate) struct ApolloApp {
    state: AppState,
    media_tools: MediaToolsState,
    media_tools_discovery: Option<MediaToolsDiscovery>,
    audio_mix_executor: AudioMixExecutor,
    upscale_executor: UpscaleExecutor,
    remux_executor: RemuxExecutor,
    audio_extract_executor: AudioExtractExecutor,
    frame_capture_executor: FrameCaptureExecutor,
    media_queue: MediaQueue,
    queue_edit: Option<QueueEditState>,
    batch_setup: Option<BatchSetupState>,
    batch_edit: Option<BatchEditState>,
    batch_inspection: Option<BatchInspectionSession>,
    queue_event_sender: mpsc::Sender<QueueExecutionEvent>,
    queue_event_receiver: Receiver<QueueExecutionEvent>,
    frame_capture_thumbnail_load: Option<FrameCaptureThumbnailLoad>,
    frame_preview_extraction: PreviewExtractionOwner,
    frame_preview_decode: Option<FramePreviewDecode>,
    progress_repaint: MediaProgressRepaintOwner,
    media_inspection: Option<MediaInspection>,
    file_dialog: FileDialogOwner,
    settings: AppSettings,
    egui_context: Context,
    next_media_tools_attempt_id: u64,
    next_session_id: u64,
    tools_error_details_open: bool,
}

impl ApolloApp {
    pub(crate) fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        fonts::install(&creation_context.egui_ctx);
        theme::install(&creation_context.egui_ctx);

        let egui_context = creation_context.egui_ctx.clone();
        let native_window_owner = WindowOwner::from_window(creation_context);
        let (queue_event_sender, queue_event_receiver) = mpsc::channel();
        let mut app = Self {
            state: AppState::default(),
            media_tools: MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                "Media tool discovery has not started.".to_owned(),
            )),
            media_tools_discovery: None,
            audio_mix_executor: AudioMixExecutor::new(),
            upscale_executor: UpscaleExecutor::new(),
            remux_executor: RemuxExecutor::new(),
            audio_extract_executor: AudioExtractExecutor::new(),
            frame_capture_executor: FrameCaptureExecutor::new(),
            media_queue: MediaQueue::new(),
            queue_edit: None,
            batch_setup: None,
            batch_edit: None,
            batch_inspection: None,
            queue_event_sender,
            queue_event_receiver,
            frame_capture_thumbnail_load: None,
            frame_preview_extraction: PreviewExtractionOwner::default(),
            frame_preview_decode: None,
            progress_repaint: MediaProgressRepaintOwner::new(egui_context.clone()),
            media_inspection: None,
            file_dialog: FileDialogOwner::new(egui_context.clone(), native_window_owner),
            settings: AppSettings::load(creation_context.storage),
            egui_context: egui_context.clone(),
            next_media_tools_attempt_id: 0,
            next_session_id: 1,
            tools_error_details_open: false,
        };
        app.start_progressive_media_tools_discovery(
            egui_context,
            discover_base_media_tools,
            cached_video_encoder_capabilities,
        );
        app
    }

    fn ready_media_tools(&self) -> Option<&MediaTools> {
        match &self.media_tools {
            MediaToolsState::Ready(tools) => Some(tools),
            MediaToolsState::Checking { .. } | MediaToolsState::Unavailable(_) => None,
        }
    }

    fn immediate_operation_running(&self) -> bool {
        self.audio_mix_executor.is_running()
            || self.upscale_executor.is_running()
            || self.remux_executor.is_running()
            || self.audio_extract_executor.is_running()
            || self.frame_capture_executor.is_running()
    }

    fn execution_arbitration(&self) -> ProcessExecutionArbitration {
        ProcessExecutionArbitration::new(
            self.immediate_operation_running(),
            self.media_queue.is_running(),
        )
    }

    fn start_media_queue(&mut self) -> bool {
        if !self.execution_arbitration().queue_start_allowed() {
            return false;
        }
        let Some(tools) = self.ready_media_tools().cloned() else {
            return false;
        };
        let sender = self.queue_event_sender.clone();
        let progress_repaint = &mut self.progress_repaint;
        self.media_queue.start(&tools, |job_id| {
            queue_execution_wake(job_id, sender.clone(), progress_repaint.begin())
        })
    }

    fn poll_media_queue(&mut self) {
        let mut current_event_received = false;
        while let Ok(event) = self.queue_event_receiver.try_recv() {
            if queue_event_matches_active(self.media_queue.active_job(), event) {
                current_event_received = match event.process_event {
                    MediaProcessEvent::Progress | MediaProcessEvent::Terminal => true,
                };
            }
        }
        if !current_event_received {
            return;
        }

        let Some(tools) = self.ready_media_tools().cloned() else {
            self.media_queue.cancel_queue();
            return;
        };
        let sender = self.queue_event_sender.clone();
        let progress_repaint = &mut self.progress_repaint;
        self.media_queue.poll(&tools, |job_id| {
            queue_execution_wake(job_id, sender.clone(), progress_repaint.begin())
        });
    }

    fn handle_queue_action(&mut self, action: QueueAction) -> bool {
        match action {
            QueueAction::AddFiles => {
                self.media_queue.clear_disposable_history();
                self.start_file_dialog(FileDialogRequest::OpenSources {
                    initial_directory: None,
                    target: DialogTarget::BatchInput,
                });
                true
            }
            QueueAction::Start => self.start_media_queue(),
            QueueAction::CancelCurrent => self.media_queue.cancel_current(),
            QueueAction::StopAfterCurrent => self.media_queue.stop_after_current(),
            QueueAction::CancelQueue => self.media_queue.cancel_queue(),
            QueueAction::Edit(job_id) => {
                let NavigationState::Queue { origin } = self.state.navigation else {
                    return false;
                };
                self.begin_queue_edit(job_id, QueueEditOrigin::Queue(origin))
            }
            QueueAction::MoveEarlier(job_id) => self
                .media_queue
                .move_waiting(job_id, WaitingJobMove::Earlier)
                .is_ok(),
            QueueAction::MoveLater(job_id) => self
                .media_queue
                .move_waiting(job_id, WaitingJobMove::Later)
                .is_ok(),
            QueueAction::Remove(job_id) => self.media_queue.remove_waiting(job_id),
            QueueAction::ClearFinished => self.media_queue.clear_finished() > 0,
            QueueAction::OpenFolder(job_id) => {
                let directory = self
                    .media_queue
                    .jobs()
                    .iter()
                    .find(|job| job.id() == job_id)
                    .and_then(|job| completed_output_folder(job.state()))
                    .map(Path::to_path_buf);
                let Some(directory) = directory else {
                    return false;
                };
                if let Err(error) = open_folder(&directory) {
                    eprintln!("Queue output folder could not be opened: {error}");
                }
                true
            }
            QueueAction::Back => {
                let NavigationState::Queue { origin } = self.state.navigation else {
                    return false;
                };
                self.media_queue.clear_disposable_history();
                self.state.navigation = match origin {
                    QueueOrigin::NoSource => NavigationState::NoSource,
                    QueueOrigin::SourceWorkspace { active_operation } => {
                        NavigationState::SourceWorkspace { active_operation }
                    }
                };
                true
            }
            QueueAction::None => false,
        }
    }

    fn begin_queue_edit(&mut self, job_id: JobId, origin: QueueEditOrigin) -> bool {
        if self.media_queue.run_state() != QueueRunState::Stopped {
            return false;
        }
        let valid_origin = matches!(
            (self.state.navigation, origin),
            (NavigationState::Queue { origin: current }, QueueEditOrigin::Queue(requested))
                if current == requested
        ) || matches!(
            (self.state.navigation, origin),
            (
                NavigationState::SourceWorkspace { .. },
                QueueEditOrigin::SourceWorkspace
            )
        );
        if !valid_origin {
            return false;
        }
        let Some(job) = self
            .media_queue
            .jobs()
            .iter()
            .find(|job| job.id() == job_id && matches!(job.state(), QueueJobState::Waiting))
            .cloned()
        else {
            return false;
        };
        let session_id = self.take_session_id();
        self.queue_edit = Some(QueueEditState::new(session_id, &job));
        self.state.navigation = NavigationState::QueueEdit { origin };
        self.state.queue_confirmation = None;
        self.state.operation_action_error = None;
        true
    }

    fn finish_queue_edit(&mut self) -> bool {
        let NavigationState::QueueEdit { origin } = self.state.navigation else {
            return false;
        };
        match origin {
            QueueEditOrigin::Queue(origin) => {
                self.queue_edit = None;
                self.state.navigation = NavigationState::Queue { origin };
                true
            }
            QueueEditOrigin::SourceWorkspace => {
                let Some(job) = self.queue_edit.as_ref().and_then(|edit| {
                    self.media_queue
                        .jobs()
                        .iter()
                        .find(|job| job.id() == edit.job_id)
                        .cloned()
                }) else {
                    return false;
                };
                self.queue_edit = None;
                self.restore_queued_job_workspace(&job)
            }
        }
    }

    fn restore_queued_job_workspace(&mut self, job: &QueuedJob) -> bool {
        let operation = queued_operation_tool(job.operation());
        self.state.media_intake = job.retained_intake();
        self.state.source_presentation = Some(Arc::new(SourcePresentation::new(job.media())));
        self.frame_capture_thumbnail_load = None;
        self.cancel_frame_preview_work();
        self.state.retire_tool_sessions();
        if !self.activate_cached_source(operation) {
            return false;
        }
        self.hydrate_cached_source_operation(job.operation());
        self.state.enter_workspace(operation);
        self.state.queue_confirmation = None;
        self.state.operation_action_error = None;
        true
    }

    fn hydrate_cached_source_operation(&mut self, operation: &QueuedOperation) {
        match operation {
            QueuedOperation::AudioMix(request) => {
                if let Some(session) = self.state.audio_mix.as_mut() {
                    session.selection =
                        AudioMixSelectionState::from_request(&session.media, request);
                }
            }
            QueuedOperation::Upscale(request) => {
                if let Some(session) = self.state.upscale.as_mut() {
                    session.selection =
                        UpscaleSelectionState::from_request(&session.media, request);
                }
            }
            QueuedOperation::Remux(request) => {
                if let Some(session) = self.state.remux.as_mut() {
                    session.selection = RemuxSelectionState::from_request(&session.media, request);
                }
            }
            QueuedOperation::AudioExtract(request) => {
                if let Some(session) = self.state.audio_extract.as_mut() {
                    session.selection =
                        AudioExtractSelectionState::from_request(&session.media, request);
                }
            }
            QueuedOperation::FrameCapture(request) => {
                if let Some(session) = self.state.frame_capture.as_mut() {
                    session.selection =
                        FrameCaptureSelectionState::from_request(&session.media, request);
                    let timestamp = session.selection.valid_timestamp(&session.media);
                    self.update_frame_capture_preview(timestamp);
                }
            }
        }
    }

    fn save_queue_edit(&mut self) -> bool {
        let nvenc_capabilities = self
            .ready_media_tools()
            .map(|tools| tools.video_encoders.clone());
        let Some(edit) = self.queue_edit.as_mut() else {
            return false;
        };
        let job_id = edit.job_id;
        let edit = &mut edit.editor;
        let Some(operation) = operation_from_editor(edit, nvenc_capabilities.as_ref()) else {
            return false;
        };
        match self
            .media_queue
            .replace_waiting_operation(job_id, operation)
        {
            Ok(()) => self.finish_queue_edit(),
            Err(error) => {
                edit.error = Some(MediaError::invalid_request(format!(
                    "Could not save queue changes: {error}"
                )));
                edit.error_details_open = false;
                false
            }
        }
    }

    fn enqueue_current_operation(
        &mut self,
        operation: Tool,
    ) -> Result<Option<JobId>, QueueEnqueueError> {
        let queued = match operation {
            Tool::AudioMix => self.state.audio_mix.as_mut().and_then(|session| {
                session.selection.can_mix(&session.media).then(|| {
                    (
                        session.media.clone(),
                        QueuedOperation::AudioMix(session.selection.request(&session.media)),
                    )
                })
            }),
            Tool::VideoUpscale => {
                let Some(session) = self.state.upscale.as_ref() else {
                    return Ok(None);
                };
                if self.media_tools_discovery.is_some()
                    && session.selection.encoder_selection()
                        != apollo::upscale::model::VideoEncoderSelection::Cpu
                {
                    return Ok(None);
                }
                let Some(tools) = self.ready_media_tools() else {
                    return Ok(None);
                };
                let nvenc_capability = match session.selection.codec() {
                    VideoCodec::H264 => tools.video_encoders.h264_nvenc.clone(),
                    VideoCodec::H265 => tools.video_encoders.h265_nvenc.clone(),
                };
                let request = session
                    .selection
                    .request_with_nvenc_capability(&session.media, nvenc_capability);
                validate_upscale_request(&request, &session.media)
                    .is_ok()
                    .then(|| (session.media.clone(), QueuedOperation::Upscale(request)))
            }
            Tool::Remux => self.state.remux.as_ref().and_then(|session| {
                session.selection.can_remux(&session.media).then(|| {
                    (
                        session.media.clone(),
                        QueuedOperation::Remux(session.selection.request(&session.media)),
                    )
                })
            }),
            Tool::AudioExtract => self.state.audio_extract.as_ref().and_then(|session| {
                session.selection.can_extract(&session.media).then(|| {
                    session.selection.request(&session.media).map(|request| {
                        (
                            session.media.clone(),
                            QueuedOperation::AudioExtract(request),
                        )
                    })
                })?
            }),
            Tool::FrameCapture => self.state.frame_capture.as_ref().and_then(|session| {
                session.selection.can_capture(&session.media).then(|| {
                    session.selection.request(&session.media).map(|request| {
                        (
                            session.media.clone(),
                            QueuedOperation::FrameCapture(request),
                        )
                    })
                })?
            }),
        };
        let Some((media, request)) = queued else {
            return Ok(None);
        };
        self.media_queue.enqueue(&media, request).map(Some)
    }

    fn enqueue_from_ready_surface(&mut self, operation: Tool) {
        self.state.queue_confirmation = None;
        self.state.operation_action_error = None;
        match self.enqueue_current_operation(operation) {
            Ok(Some(_)) => self.state.queue_confirmation = Some(operation),
            Ok(None) => {}
            Err(error) => {
                self.state.operation_action_error =
                    Some((operation, media_error_from_queue_enqueue(&error)));
            }
        }
    }

    fn file_intake_enabled(&self) -> bool {
        self.media_inspection.is_none()
            && !self.file_dialog.is_pending()
            && file_intake_enabled(
                self.audio_mix_executor.state(),
                self.upscale_executor.state(),
                self.remux_executor.state(),
                self.audio_extract_executor.state(),
                self.frame_capture_executor.state(),
            )
    }

    fn workspace_interaction_enabled(&self) -> bool {
        self.media_inspection.is_none()
    }

    fn handle_source_action(&mut self, action: SourceAction) {
        if !self.file_intake_enabled() {
            return;
        }
        self.state.queue_confirmation = None;
        self.state.operation_action_error = None;
        match action {
            SourceAction::ChangeFile => self.start_file_dialog(FileDialogRequest::OpenSource {
                initial_directory: None,
            }),
            SourceAction::CloseSource => self.close_source(),
        }
    }

    fn close_source(&mut self) {
        self.frame_capture_thumbnail_load = None;
        self.cancel_frame_preview_work();
        self.state.close_source();
    }

    fn select_tool(&mut self, tool: Tool) {
        if !self.file_intake_enabled()
            || !matches!(
                self.state.navigation,
                NavigationState::SourceWorkspace { .. }
            )
        {
            return;
        }
        let previous_tool = self.state.active_operation();
        if previous_tool != Some(tool) {
            self.state.queue_confirmation = None;
            self.state.operation_action_error = None;
            if previous_tool == Some(Tool::FrameCapture)
                && let Some(session) = self.state.frame_capture.as_mut()
            {
                session.preview.invalidate();
                session.preview_texture = None;
            }
            if previous_tool == Some(Tool::FrameCapture) {
                self.cancel_frame_preview_work();
            }
        }
        if self.activate_cached_source(tool) {
            self.state.navigation = NavigationState::SourceWorkspace {
                active_operation: tool,
            };
        }
    }

    fn activate_cached_source(&mut self, tool: Tool) -> bool {
        let Some((source_identity, media)) = self
            .state
            .media_intake
            .current_with_identity()
            .map(|(identity, media)| (identity.clone(), media.clone()))
        else {
            self.state.navigation = NavigationState::NoSource;
            self.cancel_frame_preview_work();
            self.state.retire_tool_sessions();
            return false;
        };
        self.state
            .source_presentation
            .get_or_insert_with(|| Arc::new(SourcePresentation::new(&media)));

        match tool {
            Tool::AudioMix => {
                if !self
                    .state
                    .audio_mix
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.audio_mix = Some(AudioMixToolState {
                        session_id,
                        selection: AudioMixSelectionState::new(&media),
                        lifecycle_ui: AudioMixLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
            Tool::VideoUpscale => {
                if !self
                    .state
                    .upscale
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.upscale = Some(UpscaleToolState {
                        session_id,
                        selection: UpscaleSelectionState::new(&media),
                        lifecycle_ui: UpscaleLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
            Tool::Remux => {
                if !self
                    .state
                    .remux
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.remux = Some(RemuxToolState {
                        session_id,
                        selection: RemuxSelectionState::new(&media),
                        lifecycle_ui: RemuxLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
            Tool::AudioExtract => {
                if !self
                    .state
                    .audio_extract
                    .as_ref()
                    .is_some_and(|session| session.media == media)
                {
                    let session_id = self.take_session_id();
                    self.state.audio_extract = Some(AudioExtractToolState {
                        session_id,
                        selection: AudioExtractSelectionState::new(&media),
                        lifecycle_ui: AudioExtractLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
            }
            Tool::FrameCapture => {
                if !self
                    .state
                    .frame_capture
                    .as_ref()
                    .is_some_and(|session| session.source_identity == source_identity)
                {
                    let session_id = self.take_session_id();
                    let selection = FrameCaptureSelectionState::new(&media);
                    self.state.frame_capture = Some(FrameCaptureToolState {
                        session_id,
                        source_identity,
                        selection,
                        preview: FramePreview::default(),
                        preview_texture: None,
                        lifecycle_ui: FrameCaptureLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        media,
                    });
                }
                let timestamp = self
                    .state
                    .frame_capture
                    .as_ref()
                    .and_then(|session| session.selection.valid_timestamp(&session.media));
                if self.state.frame_capture.as_ref().is_some_and(|session| {
                    matches!(
                        session.preview.state(),
                        apollo::frame_capture::preview::PreviewState::Idle
                    )
                }) {
                    self.update_frame_capture_preview(timestamp);
                }
            }
        }
        true
    }

    fn update_frame_capture_preview(&mut self, timestamp: Option<FrameTimestamp>) {
        let Some(session) = self.state.frame_capture.as_mut() else {
            return;
        };
        let Some(timestamp) = timestamp else {
            session.preview.invalidate();
            self.cancel_frame_preview_work();
            return;
        };
        if session
            .preview
            .schedule(
                &session.source_identity,
                &session.media,
                timestamp,
                Instant::now(),
            )
            .is_some()
        {
            self.egui_context.request_repaint_after(PREVIEW_DEBOUNCE);
        }
    }

    fn poll_frame_capture_preview(&mut self) {
        let due_request = self
            .state
            .frame_capture
            .as_mut()
            .and_then(|session| session.preview.take_due_request(Instant::now()));
        if let Some(request) = due_request {
            self.start_frame_preview_extraction(request);
        } else if self
            .state
            .frame_capture
            .as_ref()
            .is_some_and(|session| matches!(session.preview.state(), PreviewState::Ready { .. }))
        {
            self.cancel_frame_preview_work();
        }

        self.poll_frame_preview_extraction();
        self.poll_frame_preview_decode();
        self.sync_frame_preview_texture();
    }

    fn start_frame_preview_extraction(&mut self, request: PreviewRequest) {
        self.frame_preview_decode = None;
        let tools = self
            .ready_media_tools()
            .cloned()
            .expect("preview extraction starts only while media tools are ready");
        let context = self.egui_context.clone();
        let wake = MediaProcessWake::new(move |_| context.request_repaint());
        if let Err(error) = self
            .frame_preview_extraction
            .start(&tools, request.clone(), wake)
        {
            self.settle_frame_preview(request, Err(PreviewExtractionError::Start(error)));
        }
    }

    fn poll_frame_preview_extraction(&mut self) {
        let Some(completion) = self.frame_preview_extraction.poll() else {
            return;
        };
        let authoritative = self
            .state
            .frame_capture
            .as_ref()
            .is_some_and(|session| session.preview.is_loading(&completion.request));
        if !authoritative {
            return;
        }

        match completion.result {
            Ok(bytes) => {
                let request = completion.request;
                match FramePreviewDecode::spawn(request.clone(), bytes, self.egui_context.clone()) {
                    Ok(decode) => self.frame_preview_decode = Some(decode),
                    Err(error) => {
                        self.settle_frame_preview(
                            request,
                            Err(PreviewExtractionError::Decode(format!(
                                "preview decode worker could not start: {error}"
                            ))),
                        );
                    }
                }
            }
            Err(error) => {
                self.settle_frame_preview(completion.request, Err(error));
            }
        }
    }

    fn poll_frame_preview_decode(&mut self) {
        let result = match self.frame_preview_decode.as_ref() {
            Some(decode) => decode.receiver.try_recv(),
            None => return,
        };
        let result = match result {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(PreviewExtractionError::Decode(
                "preview decode worker stopped before returning a result".to_owned(),
            )),
        };
        let decode = self
            .frame_preview_decode
            .take()
            .expect("preview decode exists while its result is settled");
        self.settle_frame_preview(decode.request, result);
    }

    fn settle_frame_preview(
        &mut self,
        request: PreviewRequest,
        result: Result<PreviewImage, PreviewExtractionError>,
    ) -> bool {
        self.state
            .frame_capture
            .as_mut()
            .is_some_and(|session| session.preview.settle(request, result))
    }

    fn sync_frame_preview_texture(&mut self) {
        let Some(session) = self.state.frame_capture.as_mut() else {
            return;
        };
        let PreviewState::Ready { request, image } = session.preview.state() else {
            return;
        };
        if session
            .preview_texture
            .as_ref()
            .is_some_and(|texture| texture.generation == request.generation)
        {
            return;
        }
        let size = image.size();
        let color = ColorImage::from_rgba_unmultiplied(size, image.rgba());
        let handle = self.egui_context.load_texture(
            format!(
                "frame-capture-preview-{}-{}",
                session.session_id,
                request.generation.get()
            ),
            color,
            TextureOptions::LINEAR,
        );
        session.preview_texture = Some(FramePreviewTexture {
            generation: request.generation,
            size,
            handle,
        });
    }

    fn cancel_frame_preview_work(&mut self) {
        self.frame_preview_extraction.cancel();
        self.frame_preview_decode = None;
    }

    fn take_session_id(&mut self) -> u64 {
        let session_id = self.next_session_id;
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .expect("Apollo tool session identifier space is exhausted");
        session_id
    }

    fn show_media_tools_unavailable(&mut self, ui: &mut Ui) {
        let error = match &self.media_tools {
            MediaToolsState::Unavailable(error) => MediaError::from_tools_error(error),
            MediaToolsState::Checking { .. } | MediaToolsState::Ready(_) => {
                MediaError::invalid_request("media tools are unavailable")
            }
        };

        layout::centered_content(ui, |ui| {
            ui.heading("Media tools unavailable");
            ui.add_space(theme::SPACING.compact);
            media_error::show_inline(ui, &error, &mut self.tools_error_details_open);
            ui.add_space(theme::SPACING.related);
            if widgets::secondary_button(ui, "Retry").clicked() {
                self.retry_media_tools();
            }
        });
    }

    fn show_media_tools_checking(&self, ui: &mut Ui) {
        layout::centered_content(ui, |ui| {
            ui.label("Checking media tools…");
        });
    }

    fn retry_media_tools(&mut self) {
        self.start_progressive_media_tools_discovery(
            self.egui_context.clone(),
            discover_base_media_tools,
            cached_video_encoder_capabilities,
        );
    }

    fn start_progressive_media_tools_discovery<B, C>(
        &mut self,
        context: Context,
        base: B,
        capabilities: C,
    ) -> bool
    where
        B: FnOnce() -> Result<MediaTools, MediaToolsError> + Send + 'static,
        C: FnOnce(&Path) -> VideoEncoderCapabilities + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let wake = context.clone();
        if !self.start_media_tools_discovery_with(context, move || {
            let mut tools = base()?;
            let _ = sender.send(tools.clone());
            wake.request_repaint();
            tools.video_encoders = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capabilities(&tools.ffmpeg)
            }))
            .unwrap_or_else(|_| {
                let failure = VideoEncoderCapability::RuntimeUnavailable {
                    reason: "The encoder capability worker stopped unexpectedly.".to_owned(),
                };
                VideoEncoderCapabilities {
                    h264_nvenc: failure.clone(),
                    h265_nvenc: failure,
                }
            });
            Ok(tools)
        }) {
            return false;
        }
        self.media_tools_discovery
            .as_mut()
            .expect("discovery started")
            .base_receiver = Some(receiver);
        true
    }

    fn start_media_tools_discovery_with<F>(&mut self, context: Context, discover: F) -> bool
    where
        F: FnOnce() -> Result<MediaTools, MediaToolsError> + Send + 'static,
    {
        if matches!(&self.media_tools, MediaToolsState::Checking { .. })
            || self.media_tools_discovery.is_some()
        {
            return false;
        }

        let attempt_id = self.next_media_tools_attempt_id;
        self.next_media_tools_attempt_id = self.next_media_tools_attempt_id.wrapping_add(1);
        self.tools_error_details_open = false;

        match MediaToolsDiscovery::spawn(attempt_id, context, discover) {
            Ok(discovery) => {
                self.media_tools = MediaToolsState::Checking { attempt_id };
                self.media_tools_discovery = Some(discovery);
                true
            }
            Err(error) => {
                self.media_tools = MediaToolsState::Unavailable(MediaToolsError::DiscoveryFailed(
                    format!("Media tool discovery worker could not start: {error}"),
                ));
                false
            }
        }
    }

    fn poll_media_tools_discovery(&mut self) {
        if let Some(discovery) = self.media_tools_discovery.as_mut()
            && let Some(receiver) = discovery.base_receiver.as_ref()
            && let Ok(tools) = receiver.try_recv()
        {
            if matches!(self.media_tools, MediaToolsState::Checking { attempt_id } if attempt_id == discovery.attempt_id)
            {
                self.media_tools = MediaToolsState::Ready(tools);
            }
            discovery.base_receiver = None;
        }
        let result = match self.media_tools_discovery.as_ref() {
            Some(discovery) => discovery.receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(result) => {
                let attempt_id = self
                    .media_tools_discovery
                    .as_ref()
                    .expect("media tool discovery exists while its result is settled")
                    .attempt_id;
                self.complete_media_tools_discovery(attempt_id, result);
                self.media_tools_discovery = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let attempt_id = self
                    .media_tools_discovery
                    .as_ref()
                    .expect("media tool discovery exists while disconnect is settled")
                    .attempt_id;
                self.complete_media_tools_discovery(
                    attempt_id,
                    Err(MediaToolsError::DiscoveryFailed(
                        "The media tool discovery worker stopped before returning a result."
                            .to_owned(),
                    )),
                );
                self.media_tools_discovery = None;
            }
        }
    }

    fn complete_media_tools_discovery(
        &mut self,
        attempt_id: u64,
        result: Result<MediaTools, MediaToolsError>,
    ) -> bool {
        if let MediaToolsState::Ready(tools) = &mut self.media_tools
            && let Err(error) = &result
            && self
                .media_tools_discovery
                .as_ref()
                .is_some_and(|discovery| discovery.attempt_id == attempt_id)
            && self.next_media_tools_attempt_id == attempt_id.wrapping_add(1)
        {
            let failure = VideoEncoderCapability::RuntimeUnavailable {
                reason: error.to_string(),
            };
            tools.video_encoders = VideoEncoderCapabilities {
                h264_nvenc: failure.clone(),
                h265_nvenc: failure,
            };
            return true;
        }
        let pending_base_matches = matches!((&self.media_tools, &result),
            (MediaToolsState::Ready(current), Ok(completed))
                if self.next_media_tools_attempt_id == attempt_id.wrapping_add(1)
                    && self.media_tools_discovery.as_ref().is_some_and(|discovery| discovery.attempt_id == attempt_id)
                    && current.ffmpeg == completed.ffmpeg && current.ffprobe == completed.ffprobe);
        if !pending_base_matches
            && !matches!(
                &self.media_tools,
                MediaToolsState::Checking {
                    attempt_id: current
                } if *current == attempt_id
            )
        {
            return false;
        }

        self.media_tools = match result {
            Ok(tools) => MediaToolsState::Ready(tools),
            Err(error) => MediaToolsState::Unavailable(error),
        };
        self.tools_error_details_open = false;
        true
    }

    fn operation_ready_surface(&self, active_operation: Tool) -> bool {
        match active_operation {
            Tool::AudioMix => operation_workspace_visible(
                self.audio_mix_executor.state(),
                self.state
                    .audio_mix
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::VideoUpscale => operation_workspace_visible(
                self.upscale_executor.state(),
                self.state
                    .upscale
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::Remux => operation_workspace_visible(
                self.remux_executor.state(),
                self.state
                    .remux
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::AudioExtract => operation_workspace_visible(
                self.audio_extract_executor.state(),
                self.state
                    .audio_extract
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Tool::FrameCapture => operation_workspace_visible(
                self.frame_capture_executor.state(),
                self.state
                    .frame_capture
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
        }
    }

    fn show_navigation(&mut self, ui: &mut Ui) {
        if matches!(self.state.navigation, NavigationState::BatchEdit) {
            self.show_batch_edit(ui);
            return;
        }
        if matches!(self.state.navigation, NavigationState::BatchSetup) {
            self.show_batch_setup(ui);
            return;
        }
        if matches!(self.state.navigation, NavigationState::QueueEdit { .. }) {
            self.show_queue_edit(ui);
            return;
        }
        if matches!(self.state.navigation, NavigationState::Queue { .. }) {
            layout::surface_fade(ui, "queue-surface".into(), |ui| {
                ui.push_id("queue-list", |ui| self.show_queue(ui));
            });
            return;
        }

        let NavigationState::SourceWorkspace { active_operation } = self.state.navigation else {
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };

        let Some(media) = self.state.media_intake.current() else {
            self.close_source();
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };
        let source_surface_key = egui::Id::new(media.path.as_path());
        let Some(source_presentation) = self.state.source_presentation.clone() else {
            self.close_source();
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };

        if self.operation_ready_surface(active_operation) {
            let editable_jobs = self
                .media_queue
                .waiting_jobs()
                .map(|job| EditableQueuedJob {
                    id: job.id(),
                    source: &job.media().path,
                    operation: job.operation().display_name(),
                    output: job.operation().output(),
                })
                .collect::<Vec<_>>();
            let workspace_action = layout::workspace_chrome(ui, |ui| {
                workspace::show(
                    ui,
                    &source_presentation.workspace,
                    active_operation,
                    WorkspaceMode::Source {
                        pending_queue_count: self.media_queue.pending_count(),
                        enabled: self.media_inspection.is_none()
                            && file_intake_enabled(
                                self.audio_mix_executor.state(),
                                self.upscale_executor.state(),
                                self.remux_executor.state(),
                                self.audio_extract_executor.state(),
                                self.frame_capture_executor.state(),
                            ),
                        editable_jobs: &editable_jobs,
                        queue_editing_allowed: self.media_queue.run_state()
                            == QueueRunState::Stopped,
                    },
                )
            })
            .inner;
            match workspace_action {
                WorkspaceAction::Source(action) => self.handle_source_action(action),
                WorkspaceAction::SelectOperation(operation) => self.select_tool(operation),
                WorkspaceAction::EditQueuedJob(job_id) => {
                    self.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace);
                }
                WorkspaceAction::OpenQueue => self.state.enter_queue(),
                WorkspaceAction::None => {}
            }
        }

        let Some(active_operation) = self.state.active_operation() else {
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };
        let surface_key = egui::Id::new((active_operation.label(), source_surface_key));
        let ready_surface = self.operation_ready_surface(active_operation);
        if ready_surface && let Some(error) = self.state.intake_error.as_ref() {
            layout::content_gutter(ui, |ui| {
                media_error::show_inline(ui, error, &mut self.state.intake_error_details_open);
                ui.add_space(theme::SPACING.related);
            });
        }
        let show_operation = |app: &mut Self, ui: &mut Ui| match active_operation {
            Tool::AudioMix if app.state.audio_mix.is_some() => app.show_audio_mix_tool(ui),
            Tool::VideoUpscale if app.state.upscale.is_some() => app.show_upscale_tool(ui),
            Tool::Remux if app.state.remux.is_some() => app.show_remux_tool(ui),
            Tool::AudioExtract if app.state.audio_extract.is_some() => {
                app.show_audio_extract_tool(ui)
            }
            Tool::FrameCapture if app.state.frame_capture.is_some() => {
                app.show_frame_capture_tool(ui)
            }
            _ => {
                layout::padded_content(ui, |ui| app.show_file_intake(ui));
            }
        };
        layout::surface_fade(ui, surface_key, |ui| {
            if ready_surface {
                show_operation(self, ui);
            } else {
                layout::padded_content(ui, |ui| show_operation(self, ui));
            }
        });
    }

    #[cfg(test)]
    fn show_tool(&mut self, ui: &mut Ui, tool: Tool) {
        self.select_tool(tool);
        self.show_navigation(ui);
    }

    fn show_file_intake(&mut self, ui: &mut Ui) {
        let action = file_intake::show(
            ui,
            self.media_inspection.is_some(),
            self.media_queue.pending_count(),
            self.state.intake_error.as_ref(),
            &mut self.state.intake_error_details_open,
        );
        self.handle_file_intake_action(action);
    }

    fn handle_file_intake_action(&mut self, action: FileIntakeAction) {
        match action {
            FileIntakeAction::OpenFile => self.start_file_dialog(FileDialogRequest::OpenSources {
                initial_directory: None,
                target: DialogTarget::BatchInput,
            }),
            FileIntakeAction::OpenQueue => self.state.enter_queue(),
            FileIntakeAction::None => {}
        }
    }

    fn show_queue(&mut self, ui: &mut Ui) {
        let arbitration = self.execution_arbitration();
        let action = queue_ui::show(
            ui,
            self.media_queue.jobs(),
            self.media_queue.active_job(),
            self.media_queue.active_progress(),
            self.media_queue.run_state(),
            arbitration.queue_start_allowed(),
        );
        self.handle_queue_action(action);
    }

    fn show_batch_setup(&mut self, ui: &mut Ui) {
        let Some(batch) = self.batch_setup.as_mut() else {
            self.state.navigation = NavigationState::NoSource;
            return;
        };
        let views = batch
            .items
            .iter()
            .map(|item| BatchItemView {
                source: &item.path,
                output: item.operation.as_ref().map(QueuedOperation::output),
                readiness: item.readiness,
                reason: item.reason.as_deref(),
                included: item.included,
                edit_action: match (&item.source, item.readiness) {
                    (BatchSourceState::Ready(_), BatchReadiness::Ready) => {
                        Some(BatchItemEditAction::Override)
                    }
                    (BatchSourceState::Ready(_), BatchReadiness::NeedsAttention) => {
                        Some(BatchItemEditAction::Review)
                    }
                    _ => None,
                },
                can_toggle_included: matches!(item.source, BatchSourceState::Ready(_)),
            })
            .collect::<Vec<_>>();
        let included = batch.items.iter().filter(|item| item.included).count();
        let can_enqueue =
            included > 0
                && batch.items.iter().filter(|item| item.included).all(|item| {
                    item.readiness == BatchReadiness::Ready && item.operation.is_some()
                });
        let action = batch_ui::show(
            ui,
            batch.operation,
            &views,
            batch
                .items
                .iter()
                .any(|item| matches!(item.source, BatchSourceState::Ready(_))),
            can_enqueue,
            self.batch_inspection.is_none(),
            batch_ui::BatchFeedback {
                status: batch.error.as_deref(),
                append_error: batch.append_error.as_ref(),
                append_error_details_open: &mut batch.append_error_details_open,
            },
        );
        match action {
            BatchAction::SelectOperation(operation) => {
                let capabilities = self
                    .ready_media_tools()
                    .map(|tools| tools.video_encoders.clone());
                if let Some(batch) = self.batch_setup.as_mut()
                    && batch.operation != operation
                {
                    batch.operation = operation;
                    batch.defaults = None;
                    batch.error = None;
                    for item in &mut batch.items {
                        item.overridden = false;
                    }
                    if let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                        BatchSourceState::Ready(media) => Some(media.clone()),
                        BatchSourceState::Inspecting | BatchSourceState::Failed => None,
                    }) {
                        let editor = OperationEditorState::new(0, media, operation);
                        batch.defaults = batch_defaults_from_editor(&editor);
                    }
                    refresh_batch_items(batch, capabilities.as_ref());
                }
            }
            BatchAction::ConfigureDefaults => {
                self.begin_batch_edit(BatchEditTarget::Defaults);
            }
            BatchAction::AddFiles => {
                self.start_file_dialog(FileDialogRequest::OpenSources {
                    initial_directory: None,
                    target: DialogTarget::BatchAppend,
                });
            }
            BatchAction::EditItem(index) => {
                self.begin_batch_edit(BatchEditTarget::Item(index));
            }
            BatchAction::ToggleIncluded(index) => {
                if let Some(item) = self
                    .batch_setup
                    .as_mut()
                    .and_then(|batch| batch.items.get_mut(index))
                {
                    item.included = !item.included;
                }
            }
            BatchAction::Enqueue => {
                self.enqueue_batch();
            }
            BatchAction::Back => {
                self.finish_batch_setup();
            }
            BatchAction::None => {}
        }
    }

    fn show_batch_edit(&mut self, ui: &mut Ui) {
        let Some(edit) = self.batch_edit.as_ref() else {
            self.state.navigation = NavigationState::BatchSetup;
            return;
        };
        let target = edit.target;
        let presentation = edit.presentation.clone();
        let operation = edit.active_operation;
        let enabled = !self.file_dialog.is_pending();
        layout::workspace_chrome(ui, |ui| {
            workspace::show(
                ui,
                &presentation.workspace,
                operation,
                WorkspaceMode::FixedSourceEdit {
                    context: match target {
                        BatchEditTarget::Defaults => "Configuring batch defaults",
                        BatchEditTarget::Item(_) => "Overriding batch item",
                    },
                    enabled,
                    operation_switching: false,
                },
            )
        });
        let capabilities = match &self.media_tools {
            MediaToolsState::Ready(tools) if self.media_tools_discovery.is_none() => {
                upscale_ui::UpscaleEncoderCapabilities::Ready(&tools.video_encoders)
            }
            _ => upscale_ui::UpscaleEncoderCapabilities::Checking,
        };
        let Some(edit) = self.batch_edit.as_mut() else {
            return;
        };
        if let Some(error) = edit.error.clone() {
            layout::content_gutter(ui, |ui| {
                media_error::show_inline(ui, &error, &mut edit.error_details_open);
                ui.add_space(theme::SPACING.related);
            });
        }
        let defaults_save_allowed = batch_defaults_from_editor(&edit.editor).is_some();
        let action = show_operation_editor(
            ui,
            &mut edit.editor,
            capabilities,
            match target {
                BatchEditTarget::Defaults => OperationSurfaceMode::BatchDefaults {
                    save_allowed: defaults_save_allowed,
                },
                BatchEditTarget::Item(_) => OperationSurfaceMode::BatchOverride,
            },
            enabled,
            match target {
                BatchEditTarget::Defaults => ("batch-defaults", 0),
                BatchEditTarget::Item(index) => ("batch-item", index),
            },
        );
        match action {
            QueueEditAction::BrowseOutput => {
                if let Some(edit) = self.batch_edit.as_ref() {
                    self.start_file_dialog(editor_output_dialog_request(&edit.editor));
                }
            }
            QueueEditAction::Save => {
                self.save_batch_edit();
            }
            QueueEditAction::Cancel => {
                self.finish_batch_edit();
            }
            QueueEditAction::None => {}
        }
    }

    fn show_queue_edit(&mut self, ui: &mut Ui) {
        let Some(edit) = self.queue_edit.as_ref() else {
            let _ = self.finish_queue_edit();
            return;
        };
        let presentation = edit.presentation.clone();
        let active_operation = edit.active_operation;
        let enabled = !self.file_dialog.is_pending();
        let workspace_action = layout::workspace_chrome(ui, |ui| {
            workspace::show(
                ui,
                &presentation.workspace,
                active_operation,
                WorkspaceMode::FixedSourceEdit {
                    context: "Editing queued item",
                    enabled,
                    operation_switching: true,
                },
            )
        })
        .inner;
        if let WorkspaceAction::SelectOperation(operation) = workspace_action
            && let Some(edit) = self.queue_edit.as_mut()
        {
            edit.active_operation = operation;
            edit.error = None;
            edit.error_details_open = false;
        }

        let Some(edit) = self.queue_edit.as_mut() else {
            return;
        };
        let job_id = edit.job_id;
        let edit = &mut edit.editor;
        if let Some(error) = edit.error.clone() {
            layout::content_gutter(ui, |ui| {
                media_error::show_inline(ui, &error, &mut edit.error_details_open);
                ui.add_space(theme::SPACING.related);
            });
        }
        let capabilities = match &self.media_tools {
            MediaToolsState::Ready(tools) if self.media_tools_discovery.is_none() => {
                upscale_ui::UpscaleEncoderCapabilities::Ready(&tools.video_encoders)
            }
            _ => upscale_ui::UpscaleEncoderCapabilities::Checking,
        };
        let action = show_operation_editor(
            ui,
            edit,
            capabilities,
            OperationSurfaceMode::QueueEdit,
            enabled,
            ("queue-editor", job_id.get()),
        );
        match action {
            QueueEditAction::BrowseOutput => {
                if let Some(edit) = self.queue_edit.as_ref() {
                    self.start_file_dialog(editor_output_dialog_request(&edit.editor));
                }
            }
            QueueEditAction::Save => {
                self.save_queue_edit();
            }
            QueueEditAction::Cancel => {
                self.finish_queue_edit();
            }
            QueueEditAction::None => {}
        }
    }

    fn load_media_path(&mut self, path: &Path) {
        let Some(tools) = self.ready_media_tools() else {
            return;
        };
        let tools = tools.clone();
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        let request_for_failure = request.clone();

        match MediaInspection::spawn(tools, request, self.egui_context.clone()) {
            Ok(inspection) => self.media_inspection = Some(inspection),
            Err(error) => {
                self.state.media_intake.retire_probe(&request_for_failure);
                self.state.intake_error = Some(MediaError::probe_failure(format!(
                    "Media inspection worker could not start: {error}"
                )));
            }
        }
    }

    fn start_batch_paths(&mut self, paths: Vec<PathBuf>) -> bool {
        if paths.is_empty() || !self.file_intake_enabled() {
            return false;
        }
        if paths.len() == 1 {
            self.load_media_path(&paths[0]);
            return true;
        }
        let Some(tools) = self.ready_media_tools().cloned() else {
            return false;
        };
        let origin = match self.state.navigation {
            NavigationState::NoSource => BatchOrigin::NoSource,
            NavigationState::Queue { origin } => BatchOrigin::Queue { origin },
            NavigationState::SourceWorkspace { active_operation } => {
                BatchOrigin::SourceWorkspace { active_operation }
            }
            NavigationState::QueueEdit {
                origin: QueueEditOrigin::Queue(origin),
            } => BatchOrigin::Queue { origin },
            NavigationState::QueueEdit {
                origin: QueueEditOrigin::SourceWorkspace,
            } => return false,
            NavigationState::BatchSetup | NavigationState::BatchEdit => return false,
        };
        let items = paths
            .iter()
            .map(|path| BatchItem {
                path: path.clone(),
                included: true,
                source: BatchSourceState::Inspecting,
                operation: None,
                readiness: BatchReadiness::Inspecting,
                reason: None,
                overridden: false,
            })
            .collect();
        let inspection = match BatchInspectionOwner::spawn(tools, paths, self.egui_context.clone())
        {
            Ok(inspection) => inspection,
            Err(error) => {
                self.state.intake_error = Some(MediaError::probe_failure(format!(
                    "Batch inspection workers could not start: {error}"
                )));
                return false;
            }
        };
        self.batch_edit = None;
        self.batch_setup = Some(BatchSetupState {
            origin,
            operation: Tool::AudioMix,
            defaults: None,
            items,
            error: None,
            append_error: None,
            append_error_details_open: false,
        });
        self.batch_inspection = Some(BatchInspectionSession {
            owner: inspection,
            item_offset: 0,
        });
        self.state.navigation = NavigationState::BatchSetup;
        true
    }

    fn poll_batch_inspection(&mut self) {
        let Some(owner) = self.batch_inspection.as_mut() else {
            return;
        };
        let results = owner.owner.poll();
        let complete = owner.owner.is_complete();
        let item_offset = owner.item_offset;
        let capabilities = self
            .ready_media_tools()
            .map(|tools| tools.video_encoders.clone());
        let Some(batch) = self.batch_setup.as_mut() else {
            self.batch_inspection = None;
            return;
        };
        for result in results {
            let Some(item) = batch.items.get_mut(item_offset + result.index) else {
                continue;
            };
            match result.result {
                Ok(media) => item.source = BatchSourceState::Ready(media),
                Err(reason) => {
                    item.source = BatchSourceState::Failed;
                    item.included = false;
                    item.readiness = BatchReadiness::Incompatible;
                    item.reason = Some(reason);
                }
            }
        }
        if batch.defaults.is_none()
            && let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                BatchSourceState::Ready(media) => Some(media.clone()),
                BatchSourceState::Inspecting | BatchSourceState::Failed => None,
            })
        {
            let editor = OperationEditorState::new(0, media, batch.operation);
            batch.defaults = batch_defaults_from_editor(&editor);
        }
        refresh_batch_items(batch, capabilities.as_ref());
        if complete {
            self.batch_inspection = None;
        }
    }

    fn append_batch_paths(&mut self, paths: Vec<PathBuf>) -> bool {
        if paths.is_empty() || self.batch_inspection.is_some() {
            return false;
        }
        let Some(tools) = self.ready_media_tools().cloned() else {
            return false;
        };
        let Some(batch) = self.batch_setup.as_ref() else {
            return false;
        };
        let mut unique: Vec<PathBuf> = Vec::new();
        let mut duplicate_count = 0;
        for path in paths {
            if batch
                .items
                .iter()
                .any(|item| normalized_source_paths_equal(&item.path, &path))
                || unique
                    .iter()
                    .any(|existing| normalized_source_paths_equal(existing, &path))
            {
                duplicate_count += 1;
            } else {
                unique.push(path);
            }
        }
        if unique.is_empty() {
            if let Some(batch) = self.batch_setup.as_mut() {
                batch.append_error = None;
                batch.append_error_details_open = false;
                batch.error = Some(duplicate_source_message(duplicate_count));
            }
            return false;
        }
        let offset = batch.items.len();
        let owner =
            match BatchInspectionOwner::spawn(tools, unique.clone(), self.egui_context.clone()) {
                Ok(owner) => owner,
                Err(error) => {
                    if let Some(batch) = self.batch_setup.as_mut() {
                        batch.error =
                            Some(format!("Additional files could not be inspected: {error}"));
                    }
                    return false;
                }
            };
        let batch = self.batch_setup.as_mut().expect("batch remains active");
        batch.append_error = None;
        batch.append_error_details_open = false;
        batch.items.extend(unique.into_iter().map(|path| BatchItem {
            path,
            included: true,
            source: BatchSourceState::Inspecting,
            operation: None,
            readiness: BatchReadiness::Inspecting,
            reason: None,
            overridden: false,
        }));
        batch.error = (duplicate_count > 0).then(|| duplicate_source_message(duplicate_count));
        self.batch_inspection = Some(BatchInspectionSession {
            owner,
            item_offset: offset,
        });
        true
    }

    fn begin_batch_edit(&mut self, target: BatchEditTarget) -> bool {
        let Some(batch) = self.batch_setup.as_ref() else {
            return false;
        };
        let operation = batch.operation;
        let defaults = batch.defaults.clone();
        let (media, request) = match target {
            BatchEditTarget::Defaults => {
                let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                    BatchSourceState::Ready(media) => Some(media.clone()),
                    BatchSourceState::Inspecting | BatchSourceState::Failed => None,
                }) else {
                    return false;
                };
                (media, None)
            }
            BatchEditTarget::Item(index) => {
                let Some(item) = batch.items.get(index) else {
                    return false;
                };
                let BatchSourceState::Ready(media) = &item.source else {
                    return false;
                };
                (media.clone(), item.operation.clone())
            }
        };
        let session_id = self.take_session_id();
        let mut editor = OperationEditorState::new(session_id, media, operation);
        if let Some(request) = request.as_ref() {
            editor.hydrate(request);
        } else if let Some(defaults) = defaults.as_ref() {
            apply_batch_defaults(&mut editor, defaults);
        }
        self.batch_edit = Some(BatchEditState { target, editor });
        self.state.navigation = NavigationState::BatchEdit;
        true
    }

    fn finish_batch_edit(&mut self) -> bool {
        if !matches!(self.state.navigation, NavigationState::BatchEdit) {
            return false;
        }
        self.batch_edit = None;
        self.state.navigation = NavigationState::BatchSetup;
        true
    }

    fn save_batch_edit(&mut self) -> bool {
        let capabilities = self
            .ready_media_tools()
            .map(|tools| tools.video_encoders.clone());
        let Some(edit) = self.batch_edit.as_mut() else {
            return false;
        };
        let target = edit.target;
        let Some(batch) = self.batch_setup.as_mut() else {
            return false;
        };
        match target {
            BatchEditTarget::Defaults => {
                let Some(defaults) = batch_defaults_from_editor(&edit.editor) else {
                    return false;
                };
                batch.defaults = Some(defaults);
                refresh_batch_items(batch, capabilities.as_ref());
            }
            BatchEditTarget::Item(index) => {
                let Some(operation) =
                    operation_from_editor(&mut edit.editor, capabilities.as_ref())
                else {
                    return false;
                };
                let Some(item) = batch.items.get_mut(index) else {
                    return false;
                };
                item.operation = Some(operation);
                item.readiness = BatchReadiness::Ready;
                item.reason = None;
                item.overridden = true;
            }
        }
        self.finish_batch_edit()
    }

    fn finish_batch_setup(&mut self) -> bool {
        let Some(batch) = self.batch_setup.take() else {
            return false;
        };
        self.batch_inspection = None;
        self.batch_edit = None;
        self.state.navigation = match batch.origin {
            BatchOrigin::NoSource => NavigationState::NoSource,
            BatchOrigin::Queue { origin } => NavigationState::Queue { origin },
            BatchOrigin::SourceWorkspace { active_operation } => {
                NavigationState::SourceWorkspace { active_operation }
            }
        };
        true
    }

    fn enqueue_batch(&mut self) -> bool {
        let Some(batch) = self.batch_setup.as_mut() else {
            return false;
        };
        batch.error = None;
        let mut source_indexes = Vec::new();
        let mut queued = Vec::new();
        for (index, item) in batch.items.iter().enumerate() {
            if !item.included {
                continue;
            }
            let BatchSourceState::Ready(media) = &item.source else {
                batch.error = Some("Every included file must finish inspection.".to_owned());
                return false;
            };
            if item.readiness != BatchReadiness::Ready {
                batch.error = Some("Resolve or exclude every included file.".to_owned());
                return false;
            }
            let Some(operation) = item.operation.clone() else {
                batch.error = Some("Every included file needs a valid operation.".to_owned());
                return false;
            };
            source_indexes.push(index);
            queued.push((media.clone(), operation));
        }
        if queued.is_empty() {
            batch.error = Some("Include at least one ready file.".to_owned());
            return false;
        }
        match self.media_queue.enqueue_batch(&queued) {
            Ok(_) => {
                let origin = batch.origin;
                self.batch_setup = None;
                self.batch_edit = None;
                self.batch_inspection = None;
                self.state.navigation = NavigationState::Queue {
                    origin: match origin {
                        BatchOrigin::NoSource => QueueOrigin::NoSource,
                        BatchOrigin::Queue { origin } => origin,
                        BatchOrigin::SourceWorkspace { active_operation } => {
                            QueueOrigin::SourceWorkspace { active_operation }
                        }
                    },
                };
                true
            }
            Err(error) => {
                let (batch_index, message) = match &error {
                    apollo::queue::BatchEnqueueError::Item { index, error } => {
                        (source_indexes.get(*index).copied(), error.to_string())
                    }
                    apollo::queue::BatchEnqueueError::DuplicateDestination { index, .. } => (
                        source_indexes.get(*index).copied(),
                        "Output duplicates another included file.".to_owned(),
                    ),
                    apollo::queue::BatchEnqueueError::JobIdExhausted => (None, error.to_string()),
                };
                if let Some(index) = batch_index
                    && let Some(item) = batch.items.get_mut(index)
                {
                    item.readiness = BatchReadiness::NeedsAttention;
                    item.reason = Some(message.clone());
                }
                batch.error = Some(format!("Batch was not queued: {message}"));
                false
            }
        }
    }

    fn begin_media_path(&mut self, path: &Path) -> Option<MediaIntakeRequest> {
        if !self.file_intake_enabled() {
            return None;
        }

        self.state.queue_confirmation = None;
        self.state.operation_action_error = None;
        self.state.intake_error = None;
        self.state.intake_error_details_open = false;
        match self.state.media_intake.begin_load(path) {
            Ok(MediaLoadStart::Reused) => {
                let operation = self.state.active_operation().unwrap_or(Tool::AudioMix);
                if self.activate_cached_source(operation) {
                    self.state.enter_workspace(operation);
                }
                None
            }
            Ok(MediaLoadStart::Probe(request)) => Some(request),
            Err(error) => {
                self.state.intake_error = Some(MediaError::from_intake_error(&error));
                None
            }
        }
    }

    fn poll_media_inspection(&mut self) {
        let result = match self.media_inspection.as_ref() {
            Some(inspection) => inspection.receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(result) => {
                let inspection = self
                    .media_inspection
                    .take()
                    .expect("inspection exists while its result is settled");
                self.complete_media_probe(inspection.request, result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let inspection = self
                    .media_inspection
                    .take()
                    .expect("inspection exists while its worker disconnect is settled");
                self.state.media_intake.retire_probe(&inspection.request);
                self.state.intake_error = Some(MediaError::probe_failure(
                    "Media inspection worker stopped before returning a result.",
                ));
                self.state.intake_error_details_open = false;
            }
        }
    }

    fn poll_frame_capture_thumbnail(&mut self) {
        let result = match self.frame_capture_thumbnail_load.as_ref() {
            Some(load) => load.receiver.try_recv(),
            None => return,
        };

        let result = match result {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Frame Capture preview worker stopped before returning a result.".to_owned())
            }
        };
        let load = self
            .frame_capture_thumbnail_load
            .take()
            .expect("thumbnail load exists while its result is settled");
        let Some(session) = self
            .state
            .frame_capture
            .as_mut()
            .filter(|session| session.session_id == load.session_id)
        else {
            return;
        };
        session
            .lifecycle_ui
            .settle_thumbnail(load.request_id, result);
    }

    fn complete_media_probe(
        &mut self,
        request: MediaIntakeRequest,
        result: Result<MediaFile, MediaProbeError>,
    ) {
        match self.state.media_intake.complete_probe(request, result) {
            Ok(MediaProbeDisposition::Published) => {
                let operation = self.state.active_operation().unwrap_or(Tool::AudioMix);
                self.state.source_presentation = self
                    .state
                    .media_intake
                    .current()
                    .map(|media| Arc::new(SourcePresentation::new(media)));
                self.frame_capture_thumbnail_load = None;
                self.cancel_frame_preview_work();
                self.state.retire_tool_sessions();
                if self.activate_cached_source(operation) {
                    self.state.enter_workspace(operation);
                }
            }
            Ok(MediaProbeDisposition::Retired) => {}
            Err(error) => {
                self.state.intake_error = Some(MediaError::from_intake_error(&error));
                self.state.intake_error_details_open = false;
            }
        }
    }

    #[cfg(test)]
    fn load_media_path_with_probe<F>(&mut self, path: &Path, probe: F)
    where
        F: FnOnce(&Path) -> Result<MediaFile, MediaProbeError>,
    {
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        let result = probe(request.path());
        self.complete_media_probe(request, result);
    }

    fn handle_window_file_drop(&mut self, context: &Context) {
        if !self.file_intake_enabled() {
            return;
        }
        let paths = context.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if !paths.is_empty() {
            self.start_batch_paths(paths);
        }
    }

    fn drag_hovered(&self, context: &Context) -> bool {
        self.file_intake_enabled() && context.input(|input| !input.raw.hovered_files.is_empty())
    }

    fn show_audio_mix_tool(&mut self, ui: &mut Ui) {
        if shows_audio_mix_lifecycle(self.audio_mix_executor.state()) {
            let action = {
                let session = self
                    .state
                    .audio_mix
                    .as_mut()
                    .expect("audio mix source exists while lifecycle surface is shown");
                let request = session.selection.request(&session.media);
                audio_mix_progress::show(
                    ui,
                    &session.media,
                    &request,
                    &mut self.audio_mix_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_audio_mix_lifecycle_action(action);
            return;
        }

        if self.show_audio_mix_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.state.queue_confirmation == Some(Tool::AudioMix);
        let action_error = self
            .state
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Tool::AudioMix)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .audio_mix
                    .as_mut()
                    .expect("audio mix source exists while ready surface is shown");
                audio_mix_ui::show(
                    ui,
                    &session.media,
                    &presentation.audio_mix,
                    &mut session.selection,
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_audio_mix_action(action)
            && let Some(session) = self.state.audio_mix.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_audio_mix_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.audio_mix.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Audio mix could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    fn handle_audio_mix_action(
        &mut self,
        action: AudioMixSelectionAction,
    ) -> Result<(), AudioMixStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != AudioMixSelectionAction::None {
            self.state.queue_confirmation = None;
            self.state.operation_action_error = None;
        }
        match action {
            AudioMixSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let request = audio_mix_output_dialog_request(session);
                self.start_file_dialog(request);
            }
            AudioMixSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Tool::AudioMix);
            }
            AudioMixSelectionAction::MixRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = session.selection.request(&session.media);
                let wake = self.progress_repaint.begin();
                self.audio_mix_executor
                    .start_with_wake(&tools, &session.media, &request, wake)?;
            }
            AudioMixSelectionAction::SaveQueueChanges
            | AudioMixSelectionAction::CancelQueueEdit
            | AudioMixSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_audio_mix_lifecycle_action(&self, action: AudioMixLifecycleAction) {
        if let AudioMixLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Mix output folder could not be opened: {error}");
        }
    }

    fn show_upscale_tool(&mut self, ui: &mut Ui) {
        if shows_upscale_lifecycle(self.upscale_executor.state()) {
            let action = {
                let upscale = self
                    .state
                    .upscale
                    .as_mut()
                    .expect("upscale source exists while lifecycle surface is shown");
                let request = upscale.selection.request(&upscale.media);
                upscale_progress::show(
                    ui,
                    &upscale.media,
                    &request,
                    &mut self.upscale_executor,
                    &mut upscale.lifecycle_ui,
                )
            };
            self.handle_upscale_lifecycle_action(action);
            return;
        }

        if self.show_upscale_start_error(ui) {
            return;
        }

        let enabled = self.workspace_interaction_enabled();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.state.queue_confirmation == Some(Tool::VideoUpscale);
        let action_error = self
            .state
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Tool::VideoUpscale)
            .map(|(_, error)| error.clone());
        let capabilities = match &self.media_tools {
            MediaToolsState::Ready(tools) if self.media_tools_discovery.is_none() => {
                upscale_ui::UpscaleEncoderCapabilities::Ready(&tools.video_encoders)
            }
            _ => upscale_ui::UpscaleEncoderCapabilities::Checking,
        };
        let action = ui
            .add_enabled_ui(enabled, |ui| {
                let upscale = self
                    .state
                    .upscale
                    .as_mut()
                    .expect("upscale source exists while ready surface is shown");
                upscale_ui::show(
                    ui,
                    &upscale.media,
                    &mut upscale.selection,
                    capabilities,
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_upscale_action(action)
            && let Some(session) = self.state.upscale.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_upscale_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.upscale.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Upscale could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    fn handle_upscale_action(
        &mut self,
        action: UpscaleSelectionAction,
    ) -> Result<(), UpscaleStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != UpscaleSelectionAction::None {
            self.state.queue_confirmation = None;
            self.state.operation_action_error = None;
        }
        match action {
            UpscaleSelectionAction::BrowseOutput => {
                let Some(session) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let request = upscale_output_dialog_request(session);
                self.start_file_dialog(request);
            }
            UpscaleSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Tool::VideoUpscale);
            }
            UpscaleSelectionAction::UpscaleRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(upscale) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                if self.media_tools_discovery.is_some()
                    && upscale.selection.encoder_selection()
                        != apollo::upscale::model::VideoEncoderSelection::Cpu
                {
                    return Ok(());
                }
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let nvenc_capability = match upscale.selection.codec() {
                    VideoCodec::H264 => tools.video_encoders.h264_nvenc.clone(),
                    VideoCodec::H265 => tools.video_encoders.h265_nvenc.clone(),
                };
                let request = upscale
                    .selection
                    .request_with_nvenc_capability(&upscale.media, nvenc_capability);
                let wake = self.progress_repaint.begin();
                self.upscale_executor
                    .start_with_wake(&tools, &upscale.media, &request, wake)?;
            }
            UpscaleSelectionAction::SaveQueueChanges
            | UpscaleSelectionAction::CancelQueueEdit
            | UpscaleSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_upscale_lifecycle_action(&self, action: UpscaleLifecycleAction) {
        if let UpscaleLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Upscale output folder could not be opened: {error}");
        }
    }

    fn show_remux_tool(&mut self, ui: &mut Ui) {
        if shows_remux_lifecycle(self.remux_executor.state()) {
            let action = {
                let remux = self
                    .state
                    .remux
                    .as_mut()
                    .expect("remux source exists while lifecycle surface is shown");
                let request = remux.selection.request(&remux.media);
                remux_progress::show(
                    ui,
                    &request,
                    &mut self.remux_executor,
                    &mut remux.lifecycle_ui,
                )
            };
            self.handle_remux_lifecycle_action(action);
            return;
        }

        if self.show_remux_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.state.queue_confirmation == Some(Tool::Remux);
        let action_error = self
            .state
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Tool::Remux)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let remux = self
                    .state
                    .remux
                    .as_mut()
                    .expect("remux source exists while ready surface is shown");
                remux_ui::show(
                    ui,
                    &remux.media,
                    &presentation.remux,
                    &mut remux.selection,
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_remux_action(action)
            && let Some(session) = self.state.remux.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_remux_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.remux.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Remux could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    fn handle_remux_action(&mut self, action: RemuxSelectionAction) -> Result<(), RemuxStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != RemuxSelectionAction::None {
            self.state.queue_confirmation = None;
            self.state.operation_action_error = None;
        }
        match action {
            RemuxSelectionAction::BrowseOutput => {
                let Some(session) = self.state.remux.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(remux_output_dialog_request(session));
            }
            RemuxSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Tool::Remux);
            }
            RemuxSelectionAction::RemuxRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(remux) = self.state.remux.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = remux.selection.request(&remux.media);
                let wake = self.progress_repaint.begin();
                self.remux_executor
                    .start_with_wake(&tools, &remux.media, &request, wake)?;
            }
            RemuxSelectionAction::SaveQueueChanges
            | RemuxSelectionAction::CancelQueueEdit
            | RemuxSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_remux_lifecycle_action(&self, action: RemuxLifecycleAction) {
        if let RemuxLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Remux output folder could not be opened: {error}");
        }
    }

    fn show_audio_extract_tool(&mut self, ui: &mut Ui) {
        if shows_audio_extract_lifecycle(self.audio_extract_executor.state()) {
            let action = {
                let session = self
                    .state
                    .audio_extract
                    .as_mut()
                    .expect("audio extract source exists while lifecycle surface is shown");
                let request = session
                    .selection
                    .request(&session.media)
                    .expect("running audio extract session owns a selected audio stream");
                audio_extract_progress::show(
                    ui,
                    &request,
                    &mut self.audio_extract_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_audio_extract_lifecycle_action(action);
            return;
        }

        if self.show_audio_extract_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.state.queue_confirmation == Some(Tool::AudioExtract);
        let action_error = self
            .state
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Tool::AudioExtract)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .audio_extract
                    .as_mut()
                    .expect("audio extract source exists while ready surface is shown");
                audio_extract_ui::show(
                    ui,
                    &session.media,
                    &presentation.audio_extract,
                    &mut session.selection,
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_audio_extract_action(action)
            && let Some(session) = self.state.audio_extract.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_audio_extract_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.audio_extract.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Audio extraction could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    fn handle_audio_extract_action(
        &mut self,
        action: AudioExtractSelectionAction,
    ) -> Result<(), AudioExtractStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != AudioExtractSelectionAction::None {
            self.state.queue_confirmation = None;
            self.state.operation_action_error = None;
        }
        match action {
            AudioExtractSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(audio_extract_output_dialog_request(session));
            }
            AudioExtractSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Tool::AudioExtract);
            }
            AudioExtractSelectionAction::ExtractRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return Ok(());
                };
                let Some(request) = session.selection.request(&session.media) else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let wake = self.progress_repaint.begin();
                self.audio_extract_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
            }
            AudioExtractSelectionAction::SaveQueueChanges
            | AudioExtractSelectionAction::CancelQueueEdit
            | AudioExtractSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_audio_extract_lifecycle_action(&self, action: AudioExtractLifecycleAction) {
        if let AudioExtractLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Extract output folder could not be opened: {error}");
        }
    }

    fn show_frame_capture_tool(&mut self, ui: &mut Ui) {
        if shows_frame_capture_lifecycle(self.frame_capture_executor.state()) {
            let action = {
                let session = self
                    .state
                    .frame_capture
                    .as_mut()
                    .expect("frame capture source exists while lifecycle surface is shown");
                let request = session
                    .selection
                    .request(&session.media)
                    .expect("running frame capture session owns a valid timestamp");
                frame_capture_progress::show(
                    ui,
                    &request,
                    &mut self.frame_capture_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_frame_capture_lifecycle_action(action);
            return;
        }

        if self.show_frame_capture_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.state.queue_confirmation == Some(Tool::FrameCapture);
        let action_error = self
            .state
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Tool::FrameCapture)
            .map(|(_, error)| error.clone());
        let (action, timestamp_edit) = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .frame_capture
                    .as_mut()
                    .expect("frame capture source exists while ready surface is shown");
                let live_preview = frame_capture_ui::FrameCaptureLivePreview::from_preview(
                    session.preview.state(),
                    session
                        .preview_texture
                        .as_ref()
                        .map(|texture| (texture.handle.id(), texture.size)),
                );
                let timestamp_before = session.selection.timestamp_text().to_owned();
                let action = frame_capture_ui::show(
                    ui,
                    &session.media,
                    &presentation.frame_capture,
                    &mut session.selection,
                    Some(live_preview),
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                );
                let timestamp_edit = (session.selection.timestamp_text() != timestamp_before)
                    .then(|| session.selection.valid_timestamp(&session.media));
                (action, timestamp_edit)
            })
            .inner;
        if let Some(timestamp) = timestamp_edit {
            self.update_frame_capture_preview(timestamp);
        }
        if let Err(error) = self.handle_frame_capture_action(action)
            && let Some(session) = self.state.frame_capture.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_frame_capture_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.frame_capture.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Frame capture could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    fn handle_frame_capture_action(
        &mut self,
        action: FrameCaptureSelectionAction,
    ) -> Result<(), FrameCaptureStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != FrameCaptureSelectionAction::None {
            self.state.queue_confirmation = None;
            self.state.operation_action_error = None;
        }
        match action {
            FrameCaptureSelectionAction::BrowseOutput => {
                let Some(session) = self.state.frame_capture.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(frame_capture_output_dialog_request(session));
            }
            FrameCaptureSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Tool::FrameCapture);
            }
            FrameCaptureSelectionAction::CaptureRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(session) = self.state.frame_capture.as_ref() else {
                    return Ok(());
                };
                let Some(request) = session.selection.request(&session.media) else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let wake = self.progress_repaint.begin();
                self.frame_capture_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
                self.frame_capture_thumbnail_load = None;
            }
            FrameCaptureSelectionAction::SaveQueueChanges
            | FrameCaptureSelectionAction::CancelQueueEdit
            | FrameCaptureSelectionAction::None => {}
        }
        Ok(())
    }

    fn handle_frame_capture_lifecycle_action(&mut self, action: FrameCaptureLifecycleAction) {
        match action {
            FrameCaptureLifecycleAction::LoadThumbnail(request) => {
                self.start_frame_capture_thumbnail_load(request);
            }
            FrameCaptureLifecycleAction::OpenFolder(path) => {
                if let Err(error) = open_folder(&path) {
                    eprintln!("Frame Capture output folder could not be opened: {error}");
                }
            }
            FrameCaptureLifecycleAction::None
            | FrameCaptureLifecycleAction::CancelRequested
            | FrameCaptureLifecycleAction::Done
            | FrameCaptureLifecycleAction::Back => {}
        }
    }

    fn start_frame_capture_thumbnail_load(&mut self, request: FrameCaptureThumbnailRequest) {
        let Some(session_id) = self
            .state
            .frame_capture
            .as_ref()
            .map(|session| session.session_id)
        else {
            return;
        };
        match FrameCaptureThumbnailLoad::spawn(session_id, &request, self.egui_context.clone()) {
            Ok(load) => self.frame_capture_thumbnail_load = Some(load),
            Err(error) => {
                if let Some(session) = self
                    .state
                    .frame_capture
                    .as_mut()
                    .filter(|session| session.session_id == session_id)
                {
                    session.lifecycle_ui.settle_thumbnail(
                        request.request_id(),
                        Err(format!(
                            "Frame Capture preview worker could not start: {error}"
                        )),
                    );
                }
            }
        }
    }

    fn prepare_file_dialog_request(&self, mut request: FileDialogRequest) -> FileDialogRequest {
        match &mut request {
            FileDialogRequest::OpenSource { initial_directory }
            | FileDialogRequest::OpenSources {
                initial_directory, ..
            } => {
                *initial_directory = self.settings.source_directory();
            }
            FileDialogRequest::SaveOutput { options, .. } => {
                let source_fallback = options
                    .initial_directory
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("."));
                options.initial_directory = self.settings.output_directory(&source_fallback);
            }
        }
        request
    }

    fn start_file_dialog(&mut self, request: FileDialogRequest) {
        let request = self.prepare_file_dialog_request(request);
        let target = request.target();
        if let Err(error) = self.file_dialog.request(request)
            && !matches!(error, FileDialogStartError::AlreadyPending)
        {
            self.settle_file_dialog(FileDialogCompletion {
                target,
                outcome: DialogOutcome::Failed(error.to_string()),
            });
        }
    }

    fn poll_file_dialog(&mut self) {
        if let Some(completion) = self.file_dialog.poll() {
            self.settle_file_dialog(completion);
        }
    }

    fn settle_file_dialog(&mut self, completion: FileDialogCompletion) {
        if let DialogOutcome::Selected(path) = &completion.outcome {
            match completion.target {
                DialogTarget::MediaInput | DialogTarget::BatchInput | DialogTarget::BatchAppend => {
                    self.settings.remember_source(path)
                }
                _ => self.settings.remember_output(path),
            }
        } else if let DialogOutcome::SelectedMany(paths) = &completion.outcome
            && let Some(path) = paths.first()
        {
            self.settings.remember_source(path);
        }
        match (completion.target, completion.outcome) {
            (_, DialogOutcome::Cancelled) => {}
            (DialogTarget::MediaInput, DialogOutcome::Selected(path)) => {
                self.load_media_path(&path);
            }
            (DialogTarget::BatchInput, DialogOutcome::Selected(path)) => {
                self.start_batch_paths(vec![path]);
            }
            (DialogTarget::BatchInput, DialogOutcome::SelectedMany(paths)) => {
                self.start_batch_paths(paths);
            }
            (DialogTarget::BatchAppend, DialogOutcome::Selected(path)) => {
                self.append_batch_paths(vec![path]);
            }
            (DialogTarget::BatchAppend, DialogOutcome::SelectedMany(paths)) => {
                self.append_batch_paths(paths);
            }
            (DialogTarget::BatchAppend, DialogOutcome::Failed(diagnostic)) => {
                if let Some(batch) = self.batch_setup.as_mut() {
                    batch.error = None;
                    batch.append_error = Some(MediaError::file_picker_unavailable(diagnostic));
                    batch.append_error_details_open = false;
                }
            }
            (
                DialogTarget::MediaInput | DialogTarget::BatchInput,
                DialogOutcome::Failed(diagnostic),
            ) => {
                self.state.intake_error = Some(MediaError::file_picker_unavailable(diagnostic));
                self.state.intake_error_details_open = false;
            }
            (target, outcome) => {
                if let Some(edit) = self.queue_edit.as_mut() {
                    settle_editor_output_file_dialog(
                        Some(&mut edit.editor),
                        target,
                        outcome.clone(),
                    );
                }
                if let Some(edit) = self.batch_edit.as_mut() {
                    settle_editor_output_file_dialog(
                        Some(&mut edit.editor),
                        target,
                        outcome.clone(),
                    );
                }
                settle_output_file_dialog(
                    &mut self.state,
                    ToolExecutionStates {
                        audio_mix: self.audio_mix_executor.state(),
                        upscale: self.upscale_executor.state(),
                        remux: self.remux_executor.state(),
                        audio_extract: self.audio_extract_executor.state(),
                        frame_capture: self.frame_capture_executor.state(),
                    },
                    target,
                    outcome,
                );
            }
        }
    }
}

fn settle_editor_output_file_dialog(
    edit: Option<&mut OperationEditorState>,
    target: DialogTarget,
    outcome: DialogOutcome,
) {
    let Some(edit) = edit else {
        return;
    };
    let target_session_id = match target {
        DialogTarget::MediaInput | DialogTarget::BatchInput | DialogTarget::BatchAppend => return,
        DialogTarget::AudioMixOutput { session_id }
        | DialogTarget::UpscaleOutput { session_id }
        | DialogTarget::RemuxOutput { session_id }
        | DialogTarget::AudioExtractOutput { session_id }
        | DialogTarget::FrameCaptureOutput { session_id } => session_id,
    };
    if target_session_id != edit.session_id {
        return;
    }
    match (target, outcome) {
        (DialogTarget::AudioMixOutput { .. }, DialogOutcome::Selected(path)) => {
            edit.audio_mix.set_custom_output_path(path);
        }
        (DialogTarget::UpscaleOutput { .. }, DialogOutcome::Selected(path)) => {
            edit.upscale.set_custom_output_path(path);
        }
        (DialogTarget::RemuxOutput { .. }, DialogOutcome::Selected(path)) => {
            edit.remux.set_custom_output_path(path);
        }
        (DialogTarget::AudioExtractOutput { .. }, DialogOutcome::Selected(path)) => {
            edit.audio_extract.set_custom_output_path(path);
        }
        (DialogTarget::FrameCaptureOutput { .. }, DialogOutcome::Selected(path)) => {
            edit.frame_capture.set_custom_output_path(path);
        }
        (_, DialogOutcome::Failed(diagnostic)) => {
            edit.error = Some(MediaError::output_picker_unavailable(diagnostic));
            edit.error_details_open = false;
        }
        (_, DialogOutcome::Cancelled) => {}
        (DialogTarget::MediaInput | DialogTarget::BatchInput | DialogTarget::BatchAppend, _) => {}
        (_, DialogOutcome::SelectedMany(_)) => {}
    }
}

struct ToolExecutionStates<'a> {
    audio_mix: &'a AudioMixExecutionState,
    upscale: &'a UpscaleExecutionState,
    remux: &'a RemuxExecutionState,
    audio_extract: &'a AudioExtractExecutionState,
    frame_capture: &'a FrameCaptureExecutionState,
}

fn settle_output_file_dialog(
    state: &mut AppState,
    execution: ToolExecutionStates<'_>,
    target: DialogTarget,
    outcome: DialogOutcome,
) {
    match (target, outcome) {
        (DialogTarget::AudioMixOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.audio_mix, AudioMixExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_mix
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::AudioMixOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.audio_mix, AudioMixExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_mix
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::UpscaleOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.upscale, UpscaleExecutionState::Idle) =>
        {
            if let Some(session) = state
                .upscale
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::UpscaleOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.upscale, UpscaleExecutionState::Idle) =>
        {
            if let Some(session) = state
                .upscale
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::RemuxOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.remux, RemuxExecutionState::Idle) =>
        {
            if let Some(session) = state
                .remux
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::RemuxOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.remux, RemuxExecutionState::Idle) =>
        {
            if let Some(session) = state
                .remux
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::AudioExtractOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.audio_extract, AudioExtractExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_extract
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::AudioExtractOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.audio_extract, AudioExtractExecutionState::Idle) =>
        {
            if let Some(session) = state
                .audio_extract
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::FrameCaptureOutput { session_id }, DialogOutcome::Selected(path))
            if matches!(execution.frame_capture, FrameCaptureExecutionState::Idle) =>
        {
            if let Some(session) = state
                .frame_capture
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.selection.set_custom_output_path(path);
                session.start_error = None;
                session.start_error_details_open = false;
            }
        }
        (DialogTarget::FrameCaptureOutput { session_id }, DialogOutcome::Failed(diagnostic))
            if matches!(execution.frame_capture, FrameCaptureExecutionState::Idle) =>
        {
            if let Some(session) = state
                .frame_capture
                .as_mut()
                .filter(|session| session.session_id == session_id)
            {
                session.start_error = Some(MediaError::output_picker_unavailable(diagnostic));
                session.start_error_details_open = false;
            }
        }
        _ => {}
    }
}

fn save_dialog_options(output: &Path, format: OutputFormat) -> SaveDialogOptions {
    let initial_directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let file_name = output.file_name().unwrap_or_default().to_os_string();
    SaveDialogOptions {
        initial_directory: Some(initial_directory),
        file_name,
        format,
    }
}

fn output_dialog_request(
    target: DialogTarget,
    output: &Path,
    format: OutputFormat,
) -> FileDialogRequest {
    FileDialogRequest::SaveOutput {
        target,
        options: save_dialog_options(output, format),
    }
}

fn editor_output_dialog_request(edit: &OperationEditorState) -> FileDialogRequest {
    match edit.active_operation {
        Tool::AudioMix => {
            let format = match edit.audio_mix.container() {
                OutputContainer::Mp4 => OutputFormat::Mp4,
                OutputContainer::Mkv => OutputFormat::Mkv,
            };
            output_dialog_request(
                DialogTarget::AudioMixOutput {
                    session_id: edit.session_id,
                },
                &edit.audio_mix.output_path(),
                format,
            )
        }
        Tool::VideoUpscale => output_dialog_request(
            DialogTarget::UpscaleOutput {
                session_id: edit.session_id,
            },
            &edit.upscale.output_path(),
            OutputFormat::Mp4,
        ),
        Tool::Remux => {
            let format = match edit.remux.container() {
                MediaContainer::Mp4 => OutputFormat::Mp4,
                MediaContainer::Matroska => OutputFormat::Mkv,
            };
            output_dialog_request(
                DialogTarget::RemuxOutput {
                    session_id: edit.session_id,
                },
                &edit.remux.output_path(),
                format,
            )
        }
        Tool::AudioExtract => {
            let format = match edit.audio_extract.format() {
                AudioExtractFormat::Original => OutputFormat::Mka,
                AudioExtractFormat::Aac => OutputFormat::M4a,
                AudioExtractFormat::Flac => OutputFormat::Flac,
            };
            output_dialog_request(
                DialogTarget::AudioExtractOutput {
                    session_id: edit.session_id,
                },
                &edit.audio_extract.output_path(),
                format,
            )
        }
        Tool::FrameCapture => {
            let format = match edit.frame_capture.format() {
                FrameCaptureFormat::Png => OutputFormat::Png,
                FrameCaptureFormat::Jpeg => OutputFormat::Jpeg,
            };
            output_dialog_request(
                DialogTarget::FrameCaptureOutput {
                    session_id: edit.session_id,
                },
                &edit.frame_capture.output_path(),
                format,
            )
        }
    }
}

fn queued_operation_tool(operation: &QueuedOperation) -> Tool {
    match operation {
        QueuedOperation::AudioMix(_) => Tool::AudioMix,
        QueuedOperation::Upscale(_) => Tool::VideoUpscale,
        QueuedOperation::Remux(_) => Tool::Remux,
        QueuedOperation::AudioExtract(_) => Tool::AudioExtract,
        QueuedOperation::FrameCapture(_) => Tool::FrameCapture,
    }
}

fn operation_from_editor(
    edit: &mut OperationEditorState,
    nvenc_capabilities: Option<&VideoEncoderCapabilities>,
) -> Option<QueuedOperation> {
    let media = edit.media.clone();
    match edit.active_operation {
        Tool::AudioMix if edit.audio_mix.can_mix(&media) => {
            Some(QueuedOperation::AudioMix(edit.audio_mix.request(&media)))
        }
        Tool::VideoUpscale => {
            let capabilities = nvenc_capabilities?;
            let capability = match edit.upscale.codec() {
                VideoCodec::H264 => capabilities.h264_nvenc.clone(),
                VideoCodec::H265 => capabilities.h265_nvenc.clone(),
            };
            let request = edit
                .upscale
                .request_with_nvenc_capability(&media, capability);
            validate_upscale_request(&request, &media).ok()?;
            Some(QueuedOperation::Upscale(request))
        }
        Tool::Remux if edit.remux.can_remux(&media) => {
            Some(QueuedOperation::Remux(edit.remux.request(&media)))
        }
        Tool::AudioExtract if edit.audio_extract.can_extract(&media) => edit
            .audio_extract
            .request(&media)
            .map(QueuedOperation::AudioExtract),
        Tool::FrameCapture if edit.frame_capture.can_capture(&media) => edit
            .frame_capture
            .request(&media)
            .map(QueuedOperation::FrameCapture),
        _ => None,
    }
}

fn batch_defaults_from_editor(edit: &OperationEditorState) -> Option<BatchDefaults> {
    match edit.active_operation {
        Tool::AudioMix => Some(BatchDefaults::AudioMix {
            container: edit.audio_mix.container(),
            normalization: if edit.audio_mix.normalization_enabled() {
                LoudnessNormalization::Enabled(edit.audio_mix.normalization_targets())
            } else {
                LoudnessNormalization::Off
            },
        }),
        Tool::VideoUpscale => {
            let target = edit.upscale.target();
            let (max_width, max_height) = target.boundary();
            if max_width == 0
                || max_height == 0
                || !max_width.is_multiple_of(2)
                || !max_height.is_multiple_of(2)
            {
                return None;
            }
            Some(BatchDefaults::Upscale {
                target,
                codec: edit.upscale.codec(),
                encoder_settings: edit.upscale.encoder_settings().clone(),
            })
        }
        Tool::Remux => Some(BatchDefaults::Remux {
            container: edit.remux.container(),
        }),
        Tool::AudioExtract => Some(BatchDefaults::AudioExtract {
            format: edit.audio_extract.format(),
        }),
        Tool::FrameCapture => Some(BatchDefaults::FrameCapture {
            timestamp: edit.frame_capture.timestamp_text().parse().ok()?,
            format: edit.frame_capture.format(),
        }),
    }
}

fn apply_batch_defaults(edit: &mut OperationEditorState, defaults: &BatchDefaults) {
    let media = edit.media.clone();
    match defaults {
        BatchDefaults::AudioMix {
            container,
            normalization,
        } => {
            edit.audio_mix.set_container(&media, *container);
            match normalization {
                LoudnessNormalization::Off => {
                    edit.audio_mix.set_normalization_enabled(false);
                }
                LoudnessNormalization::Enabled(targets) => {
                    edit.audio_mix.set_normalization_enabled(true);
                    let _ = edit.audio_mix.set_normalization_targets(*targets);
                }
            }
        }
        BatchDefaults::Upscale {
            target,
            codec,
            encoder_settings,
        } => {
            edit.upscale.set_target(&media, *target);
            edit.upscale.set_codec(*codec);
            edit.upscale
                .set_encoder_selection(encoder_settings.selection());
            for codec in [VideoCodec::H264, VideoCodec::H265] {
                edit.upscale
                    .set_cpu_quality(codec, encoder_settings.cpu_quality(codec));
                edit.upscale
                    .set_nvenc_quality(codec, encoder_settings.nvenc_quality(codec));
            }
        }
        BatchDefaults::Remux { container } => {
            edit.remux.set_container(*container);
        }
        BatchDefaults::AudioExtract { format } => {
            edit.audio_extract.set_format(*format);
        }
        BatchDefaults::FrameCapture { timestamp, format } => {
            edit.frame_capture
                .set_timestamp_text(&media, timestamp.to_string());
            edit.frame_capture.set_format(*format);
        }
    }
}

fn refresh_batch_items(
    batch: &mut BatchSetupState,
    capabilities: Option<&VideoEncoderCapabilities>,
) {
    let defaults = batch.defaults.clone();
    for item in &mut batch.items {
        if item.overridden {
            continue;
        }
        let BatchSourceState::Ready(media) = &item.source else {
            continue;
        };
        let media = media.clone();
        item.operation = None;
        item.reason = None;
        let incompatible = match batch.operation {
            Tool::AudioMix | Tool::AudioExtract => media.audio_streams.is_empty(),
            Tool::VideoUpscale | Tool::FrameCapture => media.video_streams.is_empty(),
            Tool::Remux => media.streams.is_empty(),
        };
        if incompatible {
            item.readiness = BatchReadiness::Incompatible;
            item.reason = Some(match batch.operation {
                Tool::AudioMix | Tool::AudioExtract => "No audio tracks.".to_owned(),
                Tool::VideoUpscale | Tool::FrameCapture => "No video track.".to_owned(),
                Tool::Remux => "No media streams.".to_owned(),
            });
            continue;
        }
        if batch.operation == Tool::AudioExtract && media.audio_streams.len() > 1 {
            item.readiness = BatchReadiness::NeedsAttention;
            item.reason = Some("Select one audio track.".to_owned());
            continue;
        }
        let mut editor = OperationEditorState::new(0, media, batch.operation);
        if let Some(defaults) = defaults.as_ref()
            && defaults.operation() == batch.operation
        {
            apply_batch_defaults(&mut editor, defaults);
        }
        match operation_from_editor(&mut editor, capabilities) {
            Some(operation) => {
                item.operation = Some(operation);
                item.readiness = BatchReadiness::Ready;
            }
            None => {
                item.readiness = BatchReadiness::NeedsAttention;
                item.reason = Some(
                    match batch.operation {
                        Tool::VideoUpscale => "Shared target is not valid for this source.",
                        Tool::FrameCapture => "Timestamp is outside this source's duration.",
                        Tool::Remux => "Container is not compatible with this source.",
                        Tool::AudioMix => "Audio mix settings are not valid for this source.",
                        Tool::AudioExtract => "Audio extraction settings need review.",
                    }
                    .to_owned(),
                );
            }
        }
    }
}

fn show_operation_editor(
    ui: &mut Ui,
    edit: &mut OperationEditorState,
    capabilities: upscale_ui::UpscaleEncoderCapabilities<'_>,
    mode: OperationSurfaceMode,
    enabled: bool,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> QueueEditAction {
    let active_operation = edit.active_operation;
    layout::surface_fade(ui, egui::Id::new((id, active_operation.label())), |ui| {
        ui.add_enabled_ui(enabled, |ui| match active_operation {
            Tool::AudioMix => match audio_mix_ui::show(
                ui,
                &edit.media,
                &edit.presentation.audio_mix,
                &mut edit.audio_mix,
                mode,
            ) {
                AudioMixSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                AudioMixSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                AudioMixSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                _ => QueueEditAction::None,
            },
            Tool::VideoUpscale => {
                match upscale_ui::show(ui, &edit.media, &mut edit.upscale, capabilities, mode) {
                    UpscaleSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                    UpscaleSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                    UpscaleSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                    _ => QueueEditAction::None,
                }
            }
            Tool::Remux => match remux_ui::show(
                ui,
                &edit.media,
                &edit.presentation.remux,
                &mut edit.remux,
                mode,
            ) {
                RemuxSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                RemuxSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                RemuxSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                _ => QueueEditAction::None,
            },
            Tool::AudioExtract => match audio_extract_ui::show(
                ui,
                &edit.media,
                &edit.presentation.audio_extract,
                &mut edit.audio_extract,
                mode,
            ) {
                AudioExtractSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                AudioExtractSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                AudioExtractSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                _ => QueueEditAction::None,
            },
            Tool::FrameCapture => match frame_capture_ui::show(
                ui,
                &edit.media,
                &edit.presentation.frame_capture,
                &mut edit.frame_capture,
                None,
                mode,
            ) {
                FrameCaptureSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                FrameCaptureSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                FrameCaptureSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                _ => QueueEditAction::None,
            },
        })
        .inner
    })
}

fn audio_mix_output_dialog_request(session: &AudioMixToolState) -> FileDialogRequest {
    let format = match session.selection.container() {
        OutputContainer::Mp4 => OutputFormat::Mp4,
        OutputContainer::Mkv => OutputFormat::Mkv,
    };
    output_dialog_request(
        DialogTarget::AudioMixOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn upscale_output_dialog_request(session: &UpscaleToolState) -> FileDialogRequest {
    output_dialog_request(
        DialogTarget::UpscaleOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        OutputFormat::Mp4,
    )
}

fn remux_output_dialog_request(session: &RemuxToolState) -> FileDialogRequest {
    let format = match session.selection.container() {
        MediaContainer::Mp4 => OutputFormat::Mp4,
        MediaContainer::Matroska => OutputFormat::Mkv,
    };
    output_dialog_request(
        DialogTarget::RemuxOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn audio_extract_output_dialog_request(session: &AudioExtractToolState) -> FileDialogRequest {
    let format = match session.selection.format() {
        AudioExtractFormat::Original => OutputFormat::Mka,
        AudioExtractFormat::Aac => OutputFormat::M4a,
        AudioExtractFormat::Flac => OutputFormat::Flac,
    };
    output_dialog_request(
        DialogTarget::AudioExtractOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn frame_capture_output_dialog_request(session: &FrameCaptureToolState) -> FileDialogRequest {
    let format = match session.selection.format() {
        FrameCaptureFormat::Png => OutputFormat::Png,
        FrameCaptureFormat::Jpeg => OutputFormat::Jpeg,
    };
    output_dialog_request(
        DialogTarget::FrameCaptureOutput {
            session_id: session.session_id,
        },
        &session.selection.output_path(),
        format,
    )
}

fn shows_audio_mix_lifecycle(state: &AudioMixExecutionState) -> bool {
    !matches!(state, AudioMixExecutionState::Idle)
}

fn shows_upscale_lifecycle(state: &UpscaleExecutionState) -> bool {
    !matches!(state, UpscaleExecutionState::Idle)
}

fn shows_remux_lifecycle(state: &RemuxExecutionState) -> bool {
    !matches!(state, RemuxExecutionState::Idle)
}

fn shows_audio_extract_lifecycle(state: &AudioExtractExecutionState) -> bool {
    !matches!(state, AudioExtractExecutionState::Idle)
}

fn shows_frame_capture_lifecycle(state: &FrameCaptureExecutionState) -> bool {
    !matches!(state, FrameCaptureExecutionState::Idle)
}

fn queue_execution_wake(
    job_id: JobId,
    sender: mpsc::Sender<QueueExecutionEvent>,
    repaint: MediaProcessWake,
) -> MediaProcessWake {
    MediaProcessWake::new(move |process_event| {
        repaint.notify(process_event);
        let _ = sender.send(QueueExecutionEvent {
            job_id,
            process_event,
        });
    })
}

fn queue_event_matches_active(active_job: Option<JobId>, event: QueueExecutionEvent) -> bool {
    active_job == Some(event.job_id)
}

fn normalized_source_paths_equal(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn media_error_from_queue_enqueue(error: &QueueEnqueueError) -> MediaError {
    match error {
        QueueEnqueueError::Source(error) => MediaError::from_intake_error(error),
        QueueEnqueueError::SourceDoesNotMatchMedia => {
            MediaError::source_unavailable(error.to_string())
        }
        QueueEnqueueError::Output(error) => MediaError::from_output_error(error),
        QueueEnqueueError::DuplicateDestination { .. } => {
            MediaError::queue_destination_reserved(error.to_string())
        }
        QueueEnqueueError::JobIdExhausted => MediaError::invalid_request(error.to_string()),
    }
}

fn duplicate_source_message(count: usize) -> String {
    if count == 1 {
        "Ignored 1 file that is already in this batch.".to_owned()
    } else {
        format!("Ignored {count} files that are already in this batch.")
    }
}

fn open_folder(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    #[cfg(any(target_os = "windows", target_os = "macos", unix))]
    {
        command.arg(path).spawn().map(|_| ())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "opening folders is not supported on this platform",
        ))
    }
}

fn completed_output_folder(state: &QueueJobState) -> Option<&Path> {
    match state {
        QueueJobState::Completed { output } => output.parent(),
        _ => None,
    }
}

impl Drop for ApolloApp {
    fn drop(&mut self) {
        self.handle_queue_action(QueueAction::CancelQueue);
    }
}

impl eframe::App for ApolloApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        CentralPanel::default()
            .frame(layout::root_frame(ui.style()))
            .show(ui, |ui| {
                self.poll_media_tools_discovery();
                self.poll_media_queue();
                if matches!(&self.media_tools, MediaToolsState::Checking { .. }) {
                    layout::padded_content(ui, |ui| self.show_media_tools_checking(ui));
                    return;
                }
                if matches!(&self.media_tools, MediaToolsState::Unavailable(_)) {
                    layout::padded_content(ui, |ui| self.show_media_tools_unavailable(ui));
                    return;
                }

                self.poll_file_dialog();
                self.poll_media_inspection();
                self.poll_batch_inspection();
                self.poll_frame_capture_preview();
                self.poll_frame_capture_thumbnail();

                self.handle_window_file_drop(ui.ctx());
                if self.drag_hovered(ui.ctx()) {
                    file_intake::show_drag_hover(ui);
                }

                self.show_navigation(ui);
            });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.save(storage);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use super::{
        ApolloApp, AppSettings, AppState, BatchDefaults, BatchEditTarget, BatchItem, BatchOrigin,
        BatchSetupState, BatchSourceState, FrameCaptureThumbnailLoad, FramePreviewDecode,
        MediaInspection, MediaToolsState, NavigationState, PreviewExtractionOwner, QueueEditOrigin,
        QueueExecutionEvent, QueueOrigin, Tool, ToolExecutionStates,
        audio_extract_output_dialog_request, audio_mix_output_dialog_request,
        completed_output_folder, file_intake_enabled, frame_capture_output_dialog_request,
        operation_workspace_visible, queue_event_matches_active, queue_execution_wake,
        refresh_batch_items, remux_output_dialog_request, settle_output_file_dialog,
        shows_audio_extract_lifecycle, shows_audio_mix_lifecycle, shows_frame_capture_lifecycle,
        shows_remux_lifecycle, shows_upscale_lifecycle,
    };
    use crate::file_dialog::{
        DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogOwner, FileDialogRequest,
        FileDialogStartError, OutputFormat,
    };
    use crate::progress_repaint::MediaProgressRepaintOwner;
    use apollo::{
        audio_extract::{execution::AudioExtractExecutionState, model::AudioExtractFormat},
        audio_mix::{
            execution::AudioMixExecutionState,
            model::{LoudnessNormalization, LoudnessTargets, OutputContainer, TrackGain},
        },
        frame_capture::{
            execution::FrameCaptureExecutionState,
            model::{FrameCaptureFormat, FrameTimestamp},
            preview::{PREVIEW_DEBOUNCE, PreviewImage, PreviewState},
            thumbnail::FrameCaptureThumbnail,
        },
        media::{
            container::MediaContainer,
            error::{MediaError, MediaErrorKind},
            execution::MediaExecutionFailure,
            intake::MediaLoadStart,
            model::{AudioStream, FrameRate, MediaFile, MediaStream, StreamKind, VideoStream},
            probe::MediaProbeError,
            process::{MediaProcessEvent, MediaProcessFailure, MediaProcessWake},
            tools::{MediaTools, MediaToolsError},
        },
        queue::{MediaQueue, QueueJobFailure, QueueJobState, QueueRunState, QueuedOperation},
        remux::execution::RemuxExecutionState,
        ui::{
            ProcessExecutionArbitration, QueueAction, SourceAction,
            audio_extract::AudioExtractSelectionAction, audio_mix::AudioMixSelectionAction,
            batch::BatchReadiness, file_intake::FileIntakeAction, fonts,
            frame_capture::FrameCaptureSelectionAction,
            frame_capture_progress::FrameCaptureThumbnailState, layout, media_error,
            remux::RemuxSelectionAction, theme, upscale::UpscaleSelectionAction,
        },
        upscale::{
            execution::UpscaleExecutionState,
            model::{UpscaleTarget, VideoCodec},
        },
    };

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn pending_capabilities_render_in_the_existing_workspace_and_settle_without_reset() {
        use apollo::media::tools::{VideoEncoderCapabilities, VideoEncoderCapability};
        use apollo::upscale::model::{CpuQuality, VideoEncoderSelection};
        let (directory, path) = temp_file("pending-nvenc-workspace");
        let mut app = app();
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let (wake_tx, wake_rx) = mpsc::channel();
        context.set_request_repaint_callback(move |_| {
            let _ = wake_tx.send(());
        });
        let (release_tx, release_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        assert!(app.start_progressive_media_tools_discovery(
            context.clone(),
            || Ok(MediaTools::new("missing-ffmpeg", "missing-ffprobe")),
            move |_| {
                started_tx
                    .send(std::thread::current().name().map(str::to_owned))
                    .unwrap();
                release_rx.recv().unwrap();
                VideoEncoderCapabilities {
                    h264_nvenc: VideoEncoderCapability::Available,
                    h265_nvenc: VideoEncoderCapability::RuntimeUnavailable {
                        reason: "driver lacks HEVC".to_owned(),
                    },
                }
            },
        ));
        assert_eq!(
            started_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .as_deref(),
            Some("apollo-media-tools")
        );
        app.poll_media_tools_discovery();
        assert!(app.ready_media_tools().is_some());
        assert!(app.media_tools_discovery.is_some());
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let render = |app: &mut ApolloApp| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(730.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ui| app.show_navigation(ui),
            );
            output.textures_delta.clear();
            output
        };
        let output = render(&mut app);
        assert!(output_contains_text(&output, "Auto (checking...)"));
        assert!(output_contains_text(&output, "Checking NVIDIA NVENC..."));
        assert!(
            app.handle_upscale_action(UpscaleSelectionAction::UpscaleRequested)
                .is_ok()
        );
        assert!(matches!(
            app.upscale_executor.state(),
            UpscaleExecutionState::Idle
        ));
        app.state
            .upscale
            .as_mut()
            .unwrap()
            .selection
            .set_encoder_selection(VideoEncoderSelection::NvidiaNvenc);
        let output = render(&mut app);
        assert!(output_contains_text(&output, "Checking NVIDIA NVENC..."));
        assert!(
            app.handle_upscale_action(UpscaleSelectionAction::UpscaleRequested)
                .is_ok()
        );
        assert!(matches!(
            app.upscale_executor.state(),
            UpscaleExecutionState::Idle
        ));
        app.state
            .upscale
            .as_mut()
            .unwrap()
            .selection
            .set_encoder_selection(VideoEncoderSelection::Cpu);
        // The missing executable produces a start error only after CPU passes ready validation.
        assert!(matches!(
            app.handle_upscale_action(UpscaleSelectionAction::UpscaleRequested),
            Err(apollo::upscale::execution::UpscaleStartError::Process(_))
        ));
        let session = app.state.upscale.as_mut().unwrap();
        session
            .selection
            .set_encoder_selection(VideoEncoderSelection::Auto);
        session
            .selection
            .set_cpu_quality(VideoCodec::H264, CpuQuality::new(22).unwrap());
        session
            .selection
            .set_target(&session.media, UpscaleTarget::Uhd8k);
        session
            .selection
            .set_output_filename_stem("retained-output".into());
        let before = session.selection.clone();
        assert_eq!(
            settled_navigation_repaint_delay(&context, &mut app),
            Duration::MAX
        );
        while wake_rx.try_recv().is_ok() {}
        release_tx.send(()).unwrap();
        wake_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("capability completion wakes egui");
        app.poll_media_tools_discovery();
        assert!(app.media_tools_discovery.is_none());
        assert_eq!(app.state.upscale.as_ref().unwrap().selection, before);
        assert!(output_contains_text(
            &render(&mut app),
            "Auto (NVIDIA NVENC)"
        ));
        app.state
            .upscale
            .as_mut()
            .unwrap()
            .selection
            .set_codec(VideoCodec::H265);
        assert!(output_contains_text(&render(&mut app), "Auto (CPU)"));
        app.state
            .upscale
            .as_mut()
            .unwrap()
            .selection
            .set_encoder_selection(VideoEncoderSelection::NvidiaNvenc);
        assert!(output_contains_text(&render(&mut app), "driver lacks HEVC"));
        assert_eq!(
            settled_navigation_repaint_delay(&context, &mut app),
            Duration::MAX
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_capability_attempt_or_path_cannot_replace_current_tools() {
        let mut app = app();
        app.next_media_tools_attempt_id = 3;
        let (_sender, receiver) = mpsc::sync_channel(1);
        app.media_tools_discovery = Some(super::MediaToolsDiscovery {
            attempt_id: 2,
            receiver,
            base_receiver: None,
        });
        let current = app.ready_media_tools().unwrap().clone();
        assert!(!app.complete_media_tools_discovery(1, Ok(current.clone())));
        assert!(!app.complete_media_tools_discovery(
            2,
            Ok(MediaTools::new("other-ffmpeg", "other-ffprobe"))
        ));
        assert_eq!(app.ready_media_tools(), Some(&current));
        assert!(app.complete_media_tools_discovery(2, Ok(current)));
    }

    #[test]
    fn capability_disconnect_preserves_base_tools_and_retires_pending_state() {
        let mut app = app();
        app.next_media_tools_attempt_id = 1;
        let (sender, receiver) = mpsc::sync_channel(1);
        app.media_tools_discovery = Some(super::MediaToolsDiscovery {
            attempt_id: 0,
            receiver,
            base_receiver: None,
        });
        drop(sender);
        app.poll_media_tools_discovery();
        let tools = app.ready_media_tools().expect("base tools remain usable");
        assert!(matches!(&tools.video_encoders.h264_nvenc,
            apollo::media::tools::VideoEncoderCapability::RuntimeUnavailable { reason }
            if reason.contains("stopped before returning")));
        assert_eq!(
            tools.video_encoders.h264_nvenc,
            tools.video_encoders.h265_nvenc
        );
        assert!(app.media_tools_discovery.is_none());
    }

    #[derive(Debug)]
    struct TestDroppedFile {
        path: PathBuf,
    }

    impl egui::DroppedFile for TestDroppedFile {
        fn path(&self) -> &Path {
            &self.path
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            fs::read(&self.path).map_err(|error| error.to_string())
        }
    }

    fn temp_file(label: &str) -> (PathBuf, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "apollo-phase35-app-{label}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("recording.mkv");
        fs::write(&path, b"media").unwrap();
        (directory, path)
    }

    fn media(path: &Path) -> MediaFile {
        MediaFile {
            path: path.to_path_buf(),
            duration: None,
            streams: vec![
                MediaStream {
                    stream_index: 0,
                    kind: StreamKind::Video,
                    codec: Some("h264".to_owned()),
                },
                MediaStream {
                    stream_index: 1,
                    kind: StreamKind::Audio,
                    codec: Some("aac".to_owned()),
                },
            ],
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: FrameRate::new(60, 1),
                format: Default::default(),
            }],
            audio_streams: vec![AudioStream {
                stream_index: 1,
                codec: Some("aac".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: Some("Desktop Audio".to_owned()),
                language: None,
            }],
        }
    }

    fn thumbnail() -> FrameCaptureThumbnail {
        FrameCaptureThumbnail::from_rgba([2, 1], vec![255; 8]).unwrap()
    }

    fn app() -> ApolloApp {
        let egui_context = egui::Context::default();
        let (queue_event_sender, queue_event_receiver) = mpsc::channel();
        ApolloApp {
            state: AppState::default(),
            media_tools: MediaToolsState::Ready(MediaTools::new(
                "missing-ffmpeg",
                "missing-ffprobe",
            )),
            media_tools_discovery: None,
            audio_mix_executor: apollo::audio_mix::execution::AudioMixExecutor::new(),
            upscale_executor: apollo::upscale::execution::UpscaleExecutor::new(),
            remux_executor: apollo::remux::execution::RemuxExecutor::new(),
            audio_extract_executor: apollo::audio_extract::execution::AudioExtractExecutor::new(),
            frame_capture_executor: apollo::frame_capture::execution::FrameCaptureExecutor::new(),
            media_queue: MediaQueue::new(),
            queue_edit: None,
            batch_setup: None,
            batch_edit: None,
            batch_inspection: None,
            queue_event_sender,
            queue_event_receiver,
            frame_capture_thumbnail_load: None,
            frame_preview_extraction: PreviewExtractionOwner::default(),
            frame_preview_decode: None,
            progress_repaint: MediaProgressRepaintOwner::new(egui_context.clone()),
            media_inspection: None,
            file_dialog: FileDialogOwner::new_for_test(egui_context.clone(), || {
                Ok(|_: &FileDialogRequest| Ok(None))
            }),
            settings: AppSettings::default(),
            egui_context,
            next_media_tools_attempt_id: 0,
            next_session_id: 1,
            tools_error_details_open: false,
        }
    }

    fn assert_no_batch_execution(app: &ApolloApp) {
        assert!(app.media_queue.jobs().is_empty());
        assert!(!app.audio_mix_executor.is_running());
        assert!(!app.upscale_executor.is_running());
        assert!(!app.remux_executor.is_running());
        assert!(!app.audio_extract_executor.is_running());
        assert!(!app.frame_capture_executor.is_running());
    }

    fn populate_cancelled_and_failed_queue_history(
        app: &mut ApolloApp,
        path: &Path,
        directory: &Path,
    ) {
        app.load_media_path_with_probe(path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        assert!(app.media_queue.cancel_queue());
        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(directory.join("failed-output.mkv"));
        app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        assert!(!app.start_media_queue());
        assert_eq!(app.media_queue.pending_count(), 0);
        assert!(matches!(
            app.media_queue.jobs()[0].state(),
            QueueJobState::Cancelled
        ));
        assert!(matches!(
            app.media_queue.jobs()[1].state(),
            QueueJobState::Failed(_)
        ));
    }

    fn assert_no_source(state: &AppState) {
        assert_eq!(state.navigation, NavigationState::NoSource);
        assert!(state.media_intake.current().is_none());
    }

    fn assert_workspace(state: &AppState, operation: Tool) {
        assert_eq!(
            state.navigation,
            NavigationState::SourceWorkspace {
                active_operation: operation,
            }
        );
        assert!(state.media_intake.current().is_some());
    }

    #[test]
    fn media_tool_discovery_starts_in_checking_without_blocking() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        let (release_sender, release_receiver) = mpsc::channel();

        assert!(
            app.start_media_tools_discovery_with(egui::Context::default(), move || {
                release_receiver.recv().unwrap();
                Err(MediaToolsError::MissingBoth)
            },)
        );

        assert!(matches!(app.media_tools, MediaToolsState::Checking { .. }));
        assert!(app.media_tools_discovery.is_some());
        release_sender.send(()).unwrap();
    }

    #[test]
    fn media_tool_capability_work_runs_on_the_dedicated_discovery_worker() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        let (thread_sender, thread_receiver) = mpsc::sync_channel(1);

        assert!(
            app.start_media_tools_discovery_with(egui::Context::default(), move || {
                thread_sender
                    .send(std::thread::current().name().map(str::to_owned))
                    .unwrap();
                Err(MediaToolsError::MissingBoth)
            },)
        );

        assert_eq!(
            thread_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Some("apollo-media-tools".to_owned())
        );
    }

    #[test]
    fn media_tool_discovery_completion_requests_repaint_and_transitions_to_ready() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        app.tools_error_details_open = true;
        let expected = MediaTools::new(
            PathBuf::from(r"C:\Media Tools\ffmpeg.exe"),
            PathBuf::from(r"C:\Media Tools\ffprobe.exe"),
        );
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
        });

        assert!(app.start_media_tools_discovery_with(context, {
            let expected = expected.clone();
            move || Ok(expected)
        }));
        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("discovery completion should request repaint");
        app.poll_media_tools_discovery();

        assert_eq!(app.media_tools, MediaToolsState::Ready(expected));
        assert!(app.media_tools_discovery.is_none());
        assert!(!app.tools_error_details_open);
    }

    #[test]
    fn media_tool_discovery_failure_transitions_to_unavailable() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
        });

        assert!(app.start_media_tools_discovery_with(context, || {
            Err(MediaToolsError::MissingFfprobe)
        }));
        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("failed discovery completion should request repaint");
        app.poll_media_tools_discovery();

        assert_eq!(
            app.media_tools,
            MediaToolsState::Unavailable(MediaToolsError::MissingFfprobe)
        );
    }

    #[test]
    fn media_tool_retry_is_single_flight() {
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);
        let (release_sender, release_receiver) = mpsc::channel();

        assert!(
            app.start_media_tools_discovery_with(egui::Context::default(), move || {
                release_receiver.recv().unwrap();
                Err(MediaToolsError::MissingBoth)
            },)
        );
        assert!(
            !app.start_media_tools_discovery_with(egui::Context::default(), || {
                panic!("overlapping retry discovery must not start")
            })
        );

        release_sender.send(()).unwrap();
    }

    #[test]
    fn stale_media_tool_discovery_completion_is_rejected() {
        let mut app = app();
        app.media_tools = MediaToolsState::Checking { attempt_id: 7 };
        let stale = MediaTools::new("stale-ffmpeg", "stale-ffprobe");

        assert!(!app.complete_media_tools_discovery(6, Ok(stale)));
        assert_eq!(app.media_tools, MediaToolsState::Checking { attempt_id: 7 });
    }

    fn dropped_file(path: &Path) -> egui::DroppedFileHandle {
        Arc::new(TestDroppedFile {
            path: path.to_path_buf(),
        })
    }

    fn handle_drop(app: &mut ApolloApp, paths: &[&Path]) {
        let context = egui::Context::default();
        let input = egui::RawInput {
            dropped_files: paths.iter().map(|path| dropped_file(path)).collect(),
            ..Default::default()
        };
        let output = context.run_ui(input, |ui| app.handle_window_file_drop(ui.ctx()));
        output.drop_without_applying_deltas();
    }

    fn drag_hovered(app: &ApolloApp, paths: &[&Path]) -> bool {
        let context = egui::Context::default();
        let input = egui::RawInput {
            hovered_files: paths
                .iter()
                .map(|path| egui::HoveredFile {
                    path: Some((*path).to_path_buf()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut hovered = false;
        let output = context.run_ui(input, |ui| hovered = app.drag_hovered(ui.ctx()));
        output.drop_without_applying_deltas();
        hovered
    }

    fn begin_pending_inspection(
        app: &mut ApolloApp,
        path: &Path,
    ) -> mpsc::SyncSender<Result<MediaFile, MediaProbeError>> {
        let request = app
            .begin_media_path(path)
            .expect("new test source should require inspection");
        let (sender, receiver) = mpsc::sync_channel(1);
        app.media_inspection = Some(MediaInspection { request, receiver });
        sender
    }

    fn shape_contains_text(shape: &egui::Shape, needle: &str) -> bool {
        let mut pending = vec![shape];
        while let Some(shape) = pending.pop() {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(needle) => return true,
                egui::Shape::Vec(shapes) => pending.extend(shapes),
                _ => {}
            }
        }
        false
    }

    fn output_contains_text(output: &egui::FullOutput, needle: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|clipped| shape_contains_text(&clipped.shape, needle))
    }

    fn settled_navigation_repaint_delay(context: &egui::Context, app: &mut ApolloApp) -> Duration {
        for _ in 0..8 {
            let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
            let repaint_delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
            if repaint_delay == Duration::MAX {
                return repaint_delay;
            }
        }
        Duration::ZERO
    }

    fn shape_text_count(shape: &egui::Shape, needle: &str) -> usize {
        let mut count = 0;
        let mut pending = vec![shape];
        while let Some(shape) = pending.pop() {
            match shape {
                egui::Shape::Text(text) => count += usize::from(text.galley.job.text == needle),
                egui::Shape::Vec(shapes) => pending.extend(shapes),
                _ => {}
            }
        }
        count
    }

    fn output_text_count(output: &egui::FullOutput, needle: &str) -> usize {
        output
            .shapes
            .iter()
            .map(|shape| shape_text_count(&shape.shape, needle))
            .sum()
    }

    fn shape_text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        let mut pending = vec![shape];
        while let Some(shape) = pending.pop() {
            match shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => {
                    return Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()));
                }
                egui::Shape::Vec(shapes) => pending.extend(shapes.iter().rev()),
                _ => {}
            }
        }
        None
    }

    fn output_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|clipped| shape_text_rect(&clipped.shape, text))
            .unwrap_or_else(|| panic!("missing rendered text {text:?}"))
    }

    fn output_bottom_text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
        let mut matches = Vec::new();
        for clipped in &output.shapes {
            let mut pending = vec![&clipped.shape];
            while let Some(shape) = pending.pop() {
                match shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => {
                        matches.push(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
                    }
                    egui::Shape::Vec(shapes) => pending.extend(shapes),
                    _ => {}
                }
            }
        }
        matches
            .into_iter()
            .max_by(|left, right| left.bottom().total_cmp(&right.bottom()))
            .unwrap_or_else(|| panic!("missing rendered text {text:?}"))
    }

    fn terminal_execution_states() -> [AudioMixExecutionState; 3] {
        [
            AudioMixExecutionState::Completed {
                output: PathBuf::from("finished.mp4"),
            },
            AudioMixExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: "encoding failed".to_owned(),
            }),
            AudioMixExecutionState::Cancelled,
        ]
    }

    #[test]
    fn fresh_application_state_starts_without_a_source() {
        let state = AppState::default();

        assert_no_source(&state);
        assert!(state.audio_mix.is_none());
        assert!(state.upscale.is_none());
        assert!(state.remux.is_none());
        assert!(state.audio_extract.is_none());
    }

    #[test]
    fn initialization_has_no_source_without_starting_an_executor() {
        let app = app();

        assert_no_source(&app.state);
        assert!(!app.audio_mix_executor.is_running());
        assert!(!app.upscale_executor.is_running());
        assert!(!app.remux_executor.is_running());
        assert!(!app.audio_extract_executor.is_running());
    }

    #[test]
    fn fresh_application_state_renders_unified_file_intake() {
        let mut app = app();
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });

        assert!(output_contains_text(&output, "Open files"));
        assert!(!output_contains_text(&output, "Audio Mix"));
        assert!(!output_contains_text(&output, "Upscale"));
        output.textures_delta.clear();
    }

    #[test]
    fn idle_no_source_surface_schedules_no_application_repaint() {
        let mut app = app();
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        theme::set_motion_durations(&context, theme::MotionDurations::DISABLED);

        let repaint_delay = settled_navigation_repaint_delay(&context, &mut app);

        assert_eq!(repaint_delay, Duration::MAX);
    }

    #[test]
    fn idle_source_workspace_schedules_no_application_repaint() {
        let (directory, path) = temp_file("idle-workspace-repaint");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        theme::set_motion_durations(&context, theme::MotionDurations::DISABLED);

        let repaint_delay = settled_navigation_repaint_delay(&context, &mut app);

        assert_eq!(repaint_delay, Duration::MAX);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspecting_state_is_sparse_and_blocks_source_replacement() {
        let (directory, path) = temp_file("inspecting-state");
        let mut app = app();
        let _sender = begin_pending_inspection(&mut app, &path);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        assert_eq!(app.state.navigation, NavigationState::NoSource);
        assert!(app.media_inspection.is_some());
        assert!(!app.file_intake_enabled());

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });

        assert!(output_contains_text(&output, "Inspecting media…"));
        assert!(!output_contains_text(&output, "Audio Mix"));
        assert!(!output_contains_text(&output, "Open files"));
        assert!(!output_contains_text(&output, "or drop a video anywhere"));
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_completion_enters_default_source_workspace() {
        let (directory, path) = temp_file("inspection-mode-switch");
        let mut app = app();
        let sender = begin_pending_inspection(&mut app, &path);

        sender.send(Ok(media(&path))).unwrap();
        app.poll_media_inspection();

        assert!(app.media_inspection.is_none());
        assert_workspace(&app.state, Tool::AudioMix);
        assert!(app.state.audio_mix.is_some());
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .map(|session| &session.media.path),
            Some(&path)
        );
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_completion_requests_repaint_without_fixed_polling() {
        let (directory, path) = temp_file("inspection-repaint");
        let mut app = app();
        let request = app
            .begin_media_path(&path)
            .expect("new source should require inspection");
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
        });
        let media = media(&path);
        app.media_inspection = Some(
            MediaInspection::spawn_with(request, context, move || Ok(media))
                .expect("inspection worker should start"),
        );

        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("inspection completion should request repaint");
        app.poll_media_inspection();

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn inspection_worker_panic_requests_repaint_and_surfaces_probe_failure() {
        let (directory, path) = temp_file("inspection-worker-panic");
        let mut app = app();
        let request = app
            .begin_media_path(&path)
            .expect("new source should require inspection");
        let context = egui::Context::default();
        let (repaint_sender, repaint_receiver) = mpsc::sync_channel(1);
        let repaint_barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&repaint_barrier);
        context.set_request_repaint_callback(move |_| {
            let _ = repaint_sender.try_send(());
            worker_barrier.wait();
        });
        app.media_inspection = Some(
            MediaInspection::spawn_with(request, context, || {
                panic!("simulated inspection worker failure")
            })
            .expect("inspection worker should start"),
        );

        repaint_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("failed inspection worker should request repaint");
        app.poll_media_inspection();
        repaint_barrier.wait();

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn frame_thumbnail_decode_runs_on_the_dedicated_worker() {
        let (thread_sender, thread_receiver) = mpsc::sync_channel(1);
        let load =
            FrameCaptureThumbnailLoad::spawn_with(7, 11, egui::Context::default(), move || {
                thread_sender
                    .send(std::thread::current().name().map(str::to_owned))
                    .unwrap();
                Ok(thumbnail())
            })
            .expect("thumbnail worker should start");

        assert_eq!(
            thread_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Some("apollo-frame-thumbnail".to_owned())
        );
        assert!(
            load.receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_ok()
        );
    }

    #[test]
    fn live_preview_decode_runs_on_the_dedicated_worker() {
        let (directory, path) = temp_file("preview-decode-worker");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let request = match app.state.frame_capture.as_ref().unwrap().preview.state() {
            PreviewState::Debouncing { request, .. } => request.clone(),
            state => panic!("expected debouncing preview, got {state:?}"),
        };
        let (thread_sender, thread_receiver) = mpsc::sync_channel(1);
        let decode = FramePreviewDecode::spawn_with(request, egui::Context::default(), move || {
            thread_sender
                .send(std::thread::current().name().map(str::to_owned))
                .unwrap();
            Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
        })
        .expect("preview decode worker should start");

        assert_eq!(
            thread_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Some("apollo-frame-preview-decode".to_owned())
        );
        assert!(
            decode
                .receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_ok()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn authoritative_preview_texture_is_stable_across_unrelated_repaints() {
        let (directory, path) = temp_file("preview-texture-stability");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let request = session
            .preview
            .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
            .unwrap();
        assert!(session.preview.settle(
            request,
            Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
        ));

        app.sync_frame_preview_texture();
        let first_texture = app
            .state
            .frame_capture
            .as_ref()
            .unwrap()
            .preview_texture
            .as_ref()
            .unwrap()
            .handle
            .id();
        app.egui_context.request_repaint();
        app.sync_frame_preview_texture();
        let second_texture = app
            .state
            .frame_capture
            .as_ref()
            .unwrap()
            .preview_texture
            .as_ref()
            .unwrap()
            .handle
            .id();

        assert_eq!(first_texture, second_texture);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_timestamp_retires_work_but_keeps_the_last_valid_preview_visible() {
        let (directory, path) = temp_file("preview-invalid-input-retains-texture");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let request = session
            .preview
            .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
            .unwrap();
        assert!(session.preview.settle(
            request,
            Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
        ));
        app.sync_frame_preview_texture();

        app.update_frame_capture_preview(None);

        let session = app.state.frame_capture.as_ref().unwrap();
        assert!(matches!(session.preview.state(), PreviewState::Idle));
        assert!(session.preview_texture.is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cached_timestamp_becomes_ready_without_launching_ffmpeg() {
        let (directory, path) = temp_file("preview-cache-hit");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let first = session
            .preview
            .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
            .unwrap();
        assert!(session.preview.settle(
            first,
            Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap())
        ));
        let cached_generation = session
            .preview
            .schedule(
                &session.source_identity,
                &session.media,
                FrameTimestamp::ZERO,
                Instant::now() - PREVIEW_DEBOUNCE,
            )
            .unwrap();

        app.poll_frame_capture_preview();

        assert!(matches!(
            app.state.frame_capture.as_ref().unwrap().preview.state(),
            PreviewState::Ready { request, image }
                if request.generation == cached_generation && image.size() == [2, 1]
        ));
        assert!(app.frame_preview_extraction.poll().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_decode_completion_is_rejected_after_a_newer_edit() {
        let (directory, path) = temp_file("preview-stale-decode");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let stale_request = session
            .preview
            .take_due_request(Instant::now() + PREVIEW_DEBOUNCE)
            .unwrap();
        let current_generation = session
            .preview
            .schedule(
                &session.source_identity,
                &session.media,
                FrameTimestamp::from_milliseconds(500),
                Instant::now(),
            )
            .unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        app.frame_preview_decode = Some(FramePreviewDecode {
            request: stale_request,
            receiver,
        });
        sender
            .send(Ok(PreviewImage::from_rgba([2, 1], vec![255; 8]).unwrap()))
            .unwrap();

        app.poll_frame_preview_decode();

        assert!(app.frame_preview_decode.is_none());
        assert!(matches!(
            app.state.frame_capture.as_ref().unwrap().preview.state(),
            PreviewState::Debouncing { request, .. }
                if request.generation == current_generation
                    && request.position.display_timestamp()
                        == FrameTimestamp::from_milliseconds(500)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn thumbnail_completion_routes_to_its_originating_frame_capture_session() {
        let (directory, path) = temp_file("thumbnail-routing");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let session_id = session.session_id;
        let request = session
            .lifecycle_ui
            .begin_thumbnail(Path::new("capture.png"))
            .unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        app.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
            session_id,
            request_id: request.request_id(),
            receiver,
        });

        sender.send(Ok(thumbnail())).unwrap();
        app.poll_frame_capture_thumbnail();

        assert!(app.frame_capture_thumbnail_load.is_none());
        assert!(matches!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .lifecycle_ui
                .thumbnail(),
            FrameCaptureThumbnailState::Ready { request_id, .. }
                if *request_id == request.request_id()
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_replacement_clears_thumbnail_and_rejects_stale_completion() {
        let (first_directory, first_path) = temp_file("thumbnail-first-source");
        let (second_directory, second_path) = temp_file("thumbnail-second-source");
        let mut app = app();
        app.load_media_path_with_probe(&first_path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let old_session = app.state.frame_capture.as_mut().unwrap();
        let old_session_id = old_session.session_id;
        let old_request = old_session
            .lifecycle_ui
            .begin_thumbnail(Path::new("old-capture.png"))
            .unwrap();

        app.load_media_path_with_probe(&second_path, |path| Ok(media(path)));
        assert!(app.frame_capture_thumbnail_load.is_none());
        let replacement = app.state.frame_capture.as_ref().unwrap();
        assert_ne!(replacement.session_id, old_session_id);
        assert!(matches!(
            replacement.lifecycle_ui.thumbnail(),
            FrameCaptureThumbnailState::Empty
        ));

        let (sender, receiver) = mpsc::sync_channel(1);
        app.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
            session_id: old_session_id,
            request_id: old_request.request_id(),
            receiver,
        });
        sender.send(Ok(thumbnail())).unwrap();
        app.poll_frame_capture_thumbnail();
        assert!(matches!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .lifecycle_ui
                .thumbnail(),
            FrameCaptureThumbnailState::Empty
        ));

        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn frame_preview_ownership_follows_source_and_operation_sessions() {
        use apollo::frame_capture::preview::PreviewState;

        let (first_directory, first_path) = temp_file("preview-first-source");
        let (second_directory, second_path) = temp_file("preview-second-source");
        let mut app = app();
        app.load_media_path_with_probe(&first_path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);

        let first_generation = match app
            .state
            .frame_capture
            .as_ref()
            .map(|session| session.preview.state())
        {
            Some(PreviewState::Debouncing { request, .. }) => {
                assert_eq!(request.input, first_path);
                assert_eq!(
                    &request.source_identity,
                    &app.state.frame_capture.as_ref().unwrap().source_identity
                );
                request.generation
            }
            state => panic!("expected a debounced preview for the first source, got {state:?}"),
        };

        app.select_tool(Tool::Remux);
        assert!(matches!(
            app.state
                .frame_capture
                .as_ref()
                .map(|session| session.preview.state()),
            Some(PreviewState::Idle)
        ));

        app.select_tool(Tool::FrameCapture);
        assert!(matches!(
            app.state
                .frame_capture
                .as_ref()
                .map(|session| session.preview.state()),
            Some(PreviewState::Debouncing { request, .. })
                if request.generation.get() > first_generation.get()
                    && request.input == first_path
        ));

        let old_session_id = app.state.frame_capture.as_ref().unwrap().session_id;
        app.load_media_path_with_probe(&second_path, |path| Ok(media(path)));
        assert!(matches!(
            app.state.frame_capture.as_ref(),
            Some(session)
                if session.session_id != old_session_id
                    && matches!(
                        session.preview.state(),
                        PreviewState::Debouncing { request, .. }
                            if request.input == second_path
                    )
        ));

        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn missing_video_preview_request_schedules_no_delayed_repaint() {
        let (directory, path) = temp_file("preview-missing-video-repaint");
        let mut app = app();
        let (delayed_sender, delayed_receiver) = mpsc::channel();
        app.egui_context.set_request_repaint_callback(move |info| {
            if info.delay > Duration::ZERO {
                let _ = delayed_sender.send(info.delay);
            }
        });
        let mut source = media(&path);
        source.video_streams.clear();
        app.load_media_path_with_probe(&path, |_| Ok(source));

        app.select_tool(Tool::FrameCapture);

        assert!(matches!(
            app.state
                .frame_capture
                .as_ref()
                .map(|session| session.preview.state()),
            Some(apollo::frame_capture::preview::PreviewState::Idle)
        ));
        assert!(matches!(
            delayed_receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn close_source_releases_thumbnail_worker_and_session_state() {
        let (directory, path) = temp_file("thumbnail-close-source");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app.state.frame_capture.as_mut().unwrap();
        let request = session
            .lifecycle_ui
            .begin_thumbnail(Path::new("capture.png"))
            .unwrap();
        let (_sender, receiver) = mpsc::sync_channel(1);
        app.frame_capture_thumbnail_load = Some(FrameCaptureThumbnailLoad {
            session_id: session.session_id,
            request_id: request.request_id(),
            receiver,
        });

        app.handle_source_action(SourceAction::CloseSource);

        assert!(app.frame_capture_thumbnail_load.is_none());
        assert!(app.state.frame_capture.is_none());
        assert_no_source(&app.state);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_inspection_completion_cannot_replace_newer_source() {
        let (directory, first) = temp_file("stale-inspection");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        let first_request = app
            .begin_media_path(&first)
            .expect("first source should require inspection");
        let second_request = match app.state.media_intake.begin_load(&second).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("replacement should require inspection"),
        };

        app.complete_media_probe(first_request, Ok(media(&first)));
        assert!(app.state.media_intake.current().is_none());
        app.complete_media_probe(second_request, Ok(media(&second)));

        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&second)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_inspection_returns_to_intake_without_current_metadata() {
        let (directory, path) = temp_file("inspection-failure");
        let mut app = app();
        let sender = begin_pending_inspection(&mut app, &path);

        sender
            .send(Err(MediaProbeError::FfprobeFailed {
                code: Some(1),
                diagnostic: "probe failed".to_owned(),
            }))
            .unwrap();
        app.poll_media_inspection();

        assert!(app.media_inspection.is_none());
        assert_no_source(&app.state);
        assert!(app.state.media_intake.current().is_none());
        assert!(app.state.audio_mix.is_none());
        assert!(app.state.upscale.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });
        output.textures_delta.clear();
        assert!(output_contains_text(
            &output,
            "This file could not be inspected."
        ));
        assert!(output_contains_text(&output, "Open files"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn operation_selection_is_navigation_inside_a_loaded_workspace() {
        let (directory, path) = temp_file("mode-switch");
        let mut app = app();

        app.select_tool(Tool::VideoUpscale);
        assert_no_source(&app.state);

        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        assert_workspace(&app.state, Tool::AudioMix);
        let source_path = app.state.media_intake.current().unwrap().path.clone();

        for operation in [
            Tool::VideoUpscale,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
            Tool::AudioMix,
        ] {
            app.select_tool(operation);
            assert_workspace(&app.state, operation);
            assert_eq!(app.state.media_intake.current().unwrap().path, source_path);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn accepted_source_presentation_is_reused_until_source_replacement() {
        let (directory, first) = temp_file("source-presentation");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        let original_presentation = app.state.source_presentation.as_ref().unwrap().clone();

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        for operation in [
            Tool::AudioMix,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
        ] {
            app.select_tool(operation);
            let output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
            output.drop_without_applying_deltas();
            assert!(Arc::ptr_eq(
                app.state.source_presentation.as_ref().unwrap(),
                &original_presentation
            ));
        }

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        assert!(!Arc::ptr_eq(
            app.state.source_presentation.as_ref().unwrap(),
            &original_presentation
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn change_file_cancel_preserves_complete_workspace() {
        let (directory, path) = temp_file("change-cancel");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        app.state
            .frame_capture
            .as_mut()
            .unwrap()
            .selection
            .set_format(FrameCaptureFormat::Jpeg);
        let before = app.state.clone();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Cancelled,
        });

        assert_eq!(app.state, before);
        assert!(app.state.intake_error.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn change_file_keeps_workspace_until_replacement_probe_publishes() {
        let (directory, first) = temp_file("change-transaction");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_container(MediaContainer::Mp4);
        app.state.intake_error = Some(MediaError::probe_failure("previous replacement failed"));
        let old_session = app.state.remux.clone().unwrap();

        let request = app
            .begin_media_path(&second)
            .expect("replacement should require inspection");

        assert_workspace(&app.state, Tool::Remux);
        assert!(app.state.intake_error.is_none());
        assert_eq!(app.state.remux.as_ref(), Some(&old_session));
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&first)
        );

        app.complete_media_probe(request, Ok(media(&second)));

        assert_workspace(&app.state, Tool::Remux);
        let replacement = app.state.remux.as_ref().unwrap();
        assert_ne!(replacement.session_id, old_session.session_id);
        assert_eq!(replacement.media.path, second);
        assert_eq!(replacement.selection.container(), MediaContainer::Matroska);
        assert!(app.state.intake_error.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn close_source_returns_to_true_empty_intake() {
        let (directory, path) = temp_file("close-source");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        for operation in [
            Tool::VideoUpscale,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
        ] {
            app.select_tool(operation);
        }

        app.handle_source_action(SourceAction::CloseSource);

        assert_no_source(&app.state);
        assert!(app.state.audio_mix.is_none());
        assert!(app.state.upscale.is_none());
        assert!(app.state.remux.is_none());
        assert!(app.state.audio_extract.is_none());
        assert!(app.state.frame_capture.is_none());
        assert!(app.state.source_presentation.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_valid_file_routes_audio_mix_to_ready_state() {
        let (directory, path) = temp_file("open");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        assert_workspace(&app.state, Tool::AudioMix);
        assert_eq!(
            app.state.audio_mix.as_ref().map(|s| &s.media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dropped_valid_file_enters_default_workspace() {
        let (directory, path) = temp_file("drop");
        let mut app = app();
        assert!(file_intake_enabled(
            app.audio_mix_executor.state(),
            app.upscale_executor.state(),
            app.remux_executor.state(),
            app.audio_extract_executor.state(),
            app.frame_capture_executor.state(),
        ));
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        assert_workspace(&app.state, Tool::AudioMix);
        assert_eq!(
            app.state.audio_mix.as_ref().map(|s| &s.media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn one_file_drop_starts_normal_media_inspection() {
        let (directory, path) = temp_file("single-drop");
        let mut app = app();

        handle_drop(&mut app, &[&path]);

        assert_eq!(
            app.media_inspection
                .as_ref()
                .map(|inspection| inspection.request.path()),
            Some(path.as_path())
        );
        assert!(app.state.intake_error.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn multiple_file_drop_enters_batch_setup_in_source_order() {
        let (first_directory, first) = temp_file("multi-drop-first");
        let (second_directory, second) = temp_file("multi-drop-second");
        let mut app = app();

        handle_drop(&mut app, &[&first, &second]);

        assert!(app.media_inspection.is_none());
        assert!(app.state.media_intake.current().is_none());
        assert_eq!(app.state.navigation, NavigationState::BatchSetup);
        let batch = app.batch_setup.as_ref().unwrap();
        assert_eq!(
            batch
                .items
                .iter()
                .map(|item| &item.path)
                .collect::<Vec<_>>(),
            vec![&first, &second]
        );
        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn audio_extract_batch_requires_explicit_selection_for_multi_track_sources() {
        let (first_directory, first_path) = temp_file("batch-audio-first");
        let (second_directory, second_path) = temp_file("batch-audio-second");
        let first_media = media(&first_path);
        let mut second_media = media(&second_path);
        second_media.streams.push(MediaStream {
            stream_index: 2,
            kind: StreamKind::Audio,
            codec: Some("aac".to_owned()),
        });
        second_media.audio_streams.push(AudioStream {
            stream_index: 2,
            codec: Some("aac".to_owned()),
            channels: Some(2),
            channel_layout: Some("stereo".to_owned()),
            title: Some("Commentary".to_owned()),
            language: Some("eng".to_owned()),
        });
        let mut batch = BatchSetupState {
            origin: BatchOrigin::NoSource,
            operation: Tool::AudioExtract,
            defaults: Some(BatchDefaults::AudioExtract {
                format: AudioExtractFormat::Original,
            }),
            items: vec![
                BatchItem {
                    path: first_path,
                    included: true,
                    source: BatchSourceState::Ready(first_media),
                    operation: None,
                    readiness: BatchReadiness::Inspecting,
                    reason: None,
                    overridden: false,
                },
                BatchItem {
                    path: second_path,
                    included: true,
                    source: BatchSourceState::Ready(second_media),
                    operation: None,
                    readiness: BatchReadiness::Inspecting,
                    reason: None,
                    overridden: false,
                },
            ],
            error: None,
            append_error: None,
            append_error_details_open: false,
        };

        refresh_batch_items(&mut batch, None);

        assert_eq!(batch.items[0].readiness, BatchReadiness::Ready);
        assert_eq!(batch.items[1].readiness, BatchReadiness::NeedsAttention);
        assert_eq!(
            batch.items[1].reason.as_deref(),
            Some("Select one audio track.")
        );
        assert_ne!(
            batch.items[0]
                .operation
                .as_ref()
                .map(QueuedOperation::output),
            batch.items[1]
                .operation
                .as_ref()
                .map(QueuedOperation::output)
        );
        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn excluded_video_only_first_source_does_not_block_audio_extract_defaults() {
        let (first_directory, first_path) = temp_file("batch-video-only-first");
        let (second_directory, second_path) = temp_file("batch-audio-valid-second");
        let mut first_media = media(&first_path);
        first_media
            .streams
            .retain(|stream| stream.kind != StreamKind::Audio);
        first_media.audio_streams.clear();
        let second_media = media(&second_path);
        let mut app = app();
        app.batch_setup = Some(BatchSetupState {
            origin: BatchOrigin::NoSource,
            operation: Tool::AudioExtract,
            defaults: Some(BatchDefaults::AudioExtract {
                format: AudioExtractFormat::Original,
            }),
            items: vec![
                BatchItem {
                    path: first_path,
                    included: false,
                    source: BatchSourceState::Ready(first_media),
                    operation: None,
                    readiness: BatchReadiness::Inspecting,
                    reason: None,
                    overridden: false,
                },
                BatchItem {
                    path: second_path.clone(),
                    included: true,
                    source: BatchSourceState::Ready(second_media),
                    operation: None,
                    readiness: BatchReadiness::Inspecting,
                    reason: None,
                    overridden: false,
                },
            ],
            error: None,
            append_error: None,
            append_error_details_open: false,
        });
        app.state.navigation = NavigationState::BatchSetup;
        refresh_batch_items(app.batch_setup.as_mut().unwrap(), None);

        assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
        app.batch_edit
            .as_mut()
            .unwrap()
            .audio_extract
            .set_format(AudioExtractFormat::Flac);
        assert!(app.save_batch_edit());

        let batch = app.batch_setup.as_ref().unwrap();
        assert_eq!(batch.items[0].readiness, BatchReadiness::Incompatible);
        assert_eq!(batch.items[0].reason.as_deref(), Some("No audio tracks."));
        assert_eq!(batch.items[1].readiness, BatchReadiness::Ready);
        assert!(matches!(
            batch.items[1].operation.as_ref(),
            Some(QueuedOperation::AudioExtract(request))
                if request.input == second_path && request.format == AudioExtractFormat::Flac
        ));
        assert_no_batch_execution(&app);
        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn frame_capture_defaults_apply_before_per_source_timestamp_validation() {
        let (first_directory, first_path) = temp_file("batch-short-video-first");
        let (second_directory, second_path) = temp_file("batch-long-video-second");
        let mut first_media = media(&first_path);
        first_media.duration = Some(Duration::from_secs(5));
        let mut second_media = media(&second_path);
        second_media.duration = Some(Duration::from_secs(20));
        let mut app = app();
        app.batch_setup = Some(BatchSetupState {
            origin: BatchOrigin::NoSource,
            operation: Tool::FrameCapture,
            defaults: Some(BatchDefaults::FrameCapture {
                timestamp: FrameTimestamp::ZERO,
                format: FrameCaptureFormat::Png,
            }),
            items: vec![
                BatchItem {
                    path: first_path,
                    included: true,
                    source: BatchSourceState::Ready(first_media),
                    operation: None,
                    readiness: BatchReadiness::Inspecting,
                    reason: None,
                    overridden: false,
                },
                BatchItem {
                    path: second_path,
                    included: true,
                    source: BatchSourceState::Ready(second_media),
                    operation: None,
                    readiness: BatchReadiness::Inspecting,
                    reason: None,
                    overridden: false,
                },
            ],
            error: None,
            append_error: None,
            append_error_details_open: false,
        });
        app.state.navigation = NavigationState::BatchSetup;

        assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
        let edit = app.batch_edit.as_mut().unwrap();
        let representative = edit.media.clone();
        edit.frame_capture
            .set_timestamp_text(&representative, "00:00:10.000".to_owned());
        edit.frame_capture.set_format(FrameCaptureFormat::Jpeg);
        assert!(app.save_batch_edit());

        let batch = app.batch_setup.as_ref().unwrap();
        assert_eq!(batch.items[0].readiness, BatchReadiness::NeedsAttention);
        assert_eq!(
            batch.items[0].reason.as_deref(),
            Some("Timestamp is outside this source's duration.")
        );
        assert_eq!(batch.items[1].readiness, BatchReadiness::Ready);
        assert!(matches!(
            batch.items[1].operation.as_ref(),
            Some(QueuedOperation::FrameCapture(request))
                if request.position.to_string() == "00:00:10.000"
                    && request.format == FrameCaptureFormat::Jpeg
        ));
        assert_no_batch_execution(&app);
        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn batch_override_cancel_is_non_mutating_and_save_updates_only_the_item() {
        let (directory, path) = temp_file("batch-override");
        let source = media(&path);
        let mut app = app();
        let selection = apollo::ui::remux::RemuxSelectionState::new(&source);
        let original = QueuedOperation::Remux(selection.request(&source));
        app.batch_setup = Some(BatchSetupState {
            origin: BatchOrigin::NoSource,
            operation: Tool::Remux,
            defaults: Some(BatchDefaults::Remux {
                container: MediaContainer::Matroska,
            }),
            items: vec![BatchItem {
                path: path.clone(),
                included: true,
                source: BatchSourceState::Ready(source),
                operation: Some(original.clone()),
                readiness: BatchReadiness::Ready,
                reason: None,
                overridden: false,
            }],
            error: None,
            append_error: None,
            append_error_details_open: false,
        });
        app.state.navigation = NavigationState::BatchSetup;

        assert!(app.begin_batch_edit(BatchEditTarget::Item(0)));
        app.batch_edit
            .as_mut()
            .unwrap()
            .remux
            .set_custom_output_path(directory.join("cancelled.mkv"));
        assert!(app.finish_batch_edit());
        assert_eq!(
            app.batch_setup.as_ref().unwrap().items[0].operation,
            Some(original.clone())
        );

        assert!(app.begin_batch_edit(BatchEditTarget::Item(0)));
        let saved_output = directory.join("saved.mkv");
        app.batch_edit
            .as_mut()
            .unwrap()
            .remux
            .set_custom_output_path(saved_output.clone());
        assert!(app.save_batch_edit());
        assert_eq!(
            app.batch_setup.as_ref().unwrap().items[0]
                .operation
                .as_ref()
                .map(QueuedOperation::output),
            Some(saved_output.as_path())
        );
        let saved_operation = app.batch_setup.as_ref().unwrap().items[0].operation.clone();
        assert!(app.begin_batch_edit(BatchEditTarget::Defaults));
        app.batch_edit
            .as_mut()
            .unwrap()
            .remux
            .set_container(MediaContainer::Mp4);
        assert!(app.save_batch_edit());
        let item = &app.batch_setup.as_ref().unwrap().items[0];
        assert!(item.overridden);
        assert_eq!(item.operation, saved_operation);
        assert_no_batch_execution(&app);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_drop_uses_shared_intake_error_path() {
        let directory = std::env::temp_dir().join(format!(
            "apollo-phase49-directory-drop-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let mut app = app();

        handle_drop(&mut app, &[&directory]);

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::SourceUnavailable)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_path_drop_uses_shared_intake_error_path() {
        let directory = std::env::temp_dir().join(format!(
            "apollo-phase49-missing-drop-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        let missing = directory.join("missing.mkv");
        let mut app = app();

        handle_drop(&mut app, &[&missing]);

        assert!(app.media_inspection.is_none());
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::SourceUnavailable)
        );
    }

    #[test]
    fn opening_upscale_source_renders_ready_surface() {
        let (directory, path) = temp_file("upscale-ready-render");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::VideoUpscale);
        });
        output.textures_delta.clear();

        assert!(output_contains_text(&output, "Upscale to"));
        assert!(output_contains_text(&output, "Output"));
        assert!(output_contains_text(&output, "H.264"));
        assert!(output_contains_text(&output, "Browse"));
        assert!(!output_contains_text(&output, "Video codec"));
        assert!(!output_contains_text(&output, "Actual output"));
        assert!(!output_contains_text(&output, "← Back"));
        assert!(output_contains_text(&output, "Change file"));
        assert!(output_contains_text(&output, "Close source"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_remux_source_renders_ready_surface() {
        let (directory, path) = temp_file("remux-ready-render");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::Remux);
        });
        output.textures_delta.clear();

        for text in [
            "recording.mkv",
            "1920×1080",
            "MKV",
            "Output",
            "Browse",
            "Remux",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        assert!(!output_contains_text(&output, "Container"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_audio_extract_source_renders_ready_surface() {
        let (directory, path) = temp_file("audio-extract-ready-render");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioExtract);
        });
        output.textures_delta.clear();
        for text in [
            "recording.mkv",
            "1920×1080",
            "Audio track",
            "Desktop Audio",
            "stream 1",
            "Original (copy)",
            "no re-encode · MKA",
            "Output",
            "Browse",
            "Extract",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        assert!(!output_contains_text(&output, "Format"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_frame_capture_source_renders_ready_surface() {
        let (directory, path) = temp_file("frame-capture-ready-render");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::FrameCapture);
        });
        output.textures_delta.clear();
        for text in [
            "recording.mkv",
            "1920×1080",
            "Timestamp",
            "00:00:00.000",
            "PNG",
            "lossless",
            "Output",
            "Browse",
            "Capture frame",
        ] {
            assert!(output_contains_text(&output, text), "missing {text:?}");
        }
        assert!(!output_contains_text(&output, "Format"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn opening_audio_mix_source_renders_ready_surface() {
        let (directory, path) = temp_file("audio-ready-render");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioMix);
        });
        output.textures_delta.clear();

        assert!(output_contains_text(&output, "Audio tracks"));
        assert!(output_contains_text(&output, "Loudness normalization"));
        assert!(output_contains_text(&output, "Output"));
        assert!(output_contains_text(&output, "MP4"));
        assert!(output_contains_text(&output, "Browse"));
        assert!(!output_contains_text(&output, "Integrated loudness"));
        assert!(output_contains_text(&output, "Mix selected tracks"));
        assert!(!output_contains_text(&output, "← Back"));
        assert!(output_contains_text(&output, "Change file"));
        let audio_tracks = output_text_rect(&output, "Audio tracks");
        let select_all = output_text_rect(&output, "Select all");
        let clear = output_text_rect(&output, "Clear");
        let selected = output_text_rect(&output, "1 selected");
        let reset_gains = output_text_rect(&output, "Reset gains");
        let loudness = output_text_rect(&output, "Loudness normalization");
        assert!((audio_tracks.center().y - select_all.center().y).abs() <= 1.0);
        assert!((audio_tracks.center().y - clear.center().y).abs() <= 1.0);
        assert!((selected.center().y - reset_gains.center().y).abs() <= 1.0);
        assert!(selected.top() > audio_tracks.bottom());
        assert!(loudness.top() > selected.bottom());

        app.state
            .audio_mix
            .as_mut()
            .unwrap()
            .selection
            .set_normalization_enabled(true);
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_tool(ui, Tool::AudioMix);
        });
        output.textures_delta.clear();
        for label in [
            "Integrated loudness",
            "Maximum true peak",
            "Loudness range",
            "Reset normalization targets",
        ] {
            assert!(output_contains_text(&output, label), "missing {label}");
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shared_workspace_chrome_is_identical_across_all_ready_operations() {
        let (directory, path) = temp_file("shared-workspace-chrome");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        for operation in Tool::ALL {
            let mut output = context.run_ui(Default::default(), |ui| {
                app.show_tool(ui, operation);
            });
            output.textures_delta.clear();

            assert_eq!(output_text_count(&output, "recording.mkv"), 1);
            for label in [
                "1920×1080",
                "Change file",
                "Close source",
                "Audio Mix",
                "Upscale",
                "Remux",
                "Audio Extract",
                "Frame Capture",
            ] {
                assert!(
                    output_contains_text(&output, label),
                    "{operation:?} missing workspace label {label:?}"
                );
            }
            assert!(!output_contains_text(&output, "← Back"));
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn running_and_terminal_operation_states_hide_workspace_chrome() {
        let non_ready_states = [
            AudioMixExecutionState::Running,
            AudioMixExecutionState::Completed {
                output: PathBuf::from("finished.mp4"),
            },
            AudioMixExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: "failed".to_owned(),
            }),
            AudioMixExecutionState::Cancelled,
        ];

        assert!(operation_workspace_visible(
            &AudioMixExecutionState::Idle,
            true
        ));
        assert!(!operation_workspace_visible(
            &AudioMixExecutionState::Idle,
            false
        ));
        for state in non_ready_states {
            assert!(!operation_workspace_visible(&state, true));
        }
    }

    #[test]
    fn standalone_operation_feedback_does_not_render_workspace_components() {
        let (directory, path) = temp_file("standalone-operation-feedback");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        app.state.frame_capture.as_mut().unwrap().start_error =
            Some(MediaError::invalid_request("simulated start failure"));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        output.textures_delta.clear();

        assert!(output_contains_text(
            &output,
            "Frame capture could not start"
        ));
        for workspace_label in [
            "Change file",
            "Close source",
            "Audio Mix",
            "Upscale",
            "Remux",
            "Audio Extract",
            "Frame Capture",
        ] {
            assert!(!output_contains_text(&output, workspace_label));
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_ready_operation_keeps_its_primary_action_bottom_owned_at_all_heights() {
        let (directory, path) = temp_file("shared-operation-shell");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let action_height = theme::PRIMARY_CONTROL_HEIGHT
            + f32::from(layout::ACTION_BAR_VERTICAL_PADDING) * 2.0
            + theme::SEPARATOR_WIDTH;

        for (operation, label) in [
            (Tool::AudioMix, "Mix selected tracks"),
            (Tool::VideoUpscale, "Upscale"),
            (Tool::Remux, "Remux"),
            (Tool::AudioExtract, "Extract"),
            (Tool::FrameCapture, "Capture frame"),
        ] {
            for (width, height) in [
                (400.0, 260.0),
                (620.0, 379.0),
                (1_200.0, 700.0),
                (620.0, 900.0),
            ] {
                let mut viewport = egui::Rect::NOTHING;
                let mut output = context.run_ui(Default::default(), |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                    viewport = rect;
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    );
                    child.set_clip_rect(rect);
                    app.show_tool(&mut child, operation);
                });
                output.textures_delta.clear();
                let action = output_bottom_text_rect(&output, label);
                let source = output_text_rect(&output, "recording.mkv");

                assert!(
                    (action.center().y - (viewport.bottom() - action_height * 0.5)).abs() <= 1.0,
                    "{operation:?} at {width}x{height}: action={action:?}, viewport={viewport:?}"
                );
                assert!(source.top() >= viewport.top() + f32::from(layout::CONTENT_PADDING));
                assert!(source.bottom() < action.top());
            }
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_ready_selection_survives_same_source_navigation() {
        let (directory, path) = temp_file("selection-preserve");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let ready = app
            .state
            .upscale
            .as_mut()
            .expect("upscale session should exist");
        ready
            .selection
            .set_target(&ready.media, UpscaleTarget::Uhd8k);
        ready.selection.set_codec(VideoCodec::H265);

        app.select_tool(Tool::VideoUpscale);

        let ready = app
            .state
            .upscale
            .as_ref()
            .expect("upscale session should persist");
        assert_eq!(ready.selection.target(), UpscaleTarget::Uhd8k);
        assert_eq!(ready.selection.codec(), VideoCodec::H265);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_mix_output_dialog_request_follows_current_container() {
        let (directory, path) = temp_file("audio-browse");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        let session = app.state.audio_mix.as_ref().unwrap();
        let request = audio_mix_output_dialog_request(session);
        let FileDialogRequest::SaveOutput { target, options } = request else {
            panic!("audio browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::AudioMixOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, Some(directory.clone()));
        assert_eq!(options.file_name, "recording-mixed.mp4");
        assert_eq!(options.format, OutputFormat::Mp4);

        let session = app.state.audio_mix.as_mut().unwrap();
        let media = session.media.clone();
        session
            .selection
            .set_container(&media, OutputContainer::Mkv);
        let request = audio_mix_output_dialog_request(session);
        let FileDialogRequest::SaveOutput { options, .. } = request else {
            unreachable!();
        };
        assert_eq!(options.file_name, "recording-mixed.mkv");
        assert_eq!(options.format, OutputFormat::Mkv);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remux_output_dialog_request_follows_current_container() {
        let (directory, path) = temp_file("remux-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);

        let session = app.state.remux.as_ref().unwrap();
        let FileDialogRequest::SaveOutput { target, options } =
            remux_output_dialog_request(session)
        else {
            panic!("remux browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::RemuxOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, Some(directory.clone()));
        assert_eq!(options.file_name, "recording-remuxed.mkv");
        assert_eq!(options.format, OutputFormat::Mkv);

        let session = app.state.remux.as_mut().unwrap();
        session.selection.set_container(MediaContainer::Mp4);
        let FileDialogRequest::SaveOutput { options, .. } = remux_output_dialog_request(session)
        else {
            unreachable!();
        };
        assert_eq!(options.file_name, "recording-remuxed.mp4");
        assert_eq!(options.format, OutputFormat::Mp4);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_extract_output_dialog_request_follows_current_format() {
        let (directory, path) = temp_file("audio-extract-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);

        let session = app.state.audio_extract.as_ref().unwrap();
        let FileDialogRequest::SaveOutput { target, options } =
            audio_extract_output_dialog_request(session)
        else {
            panic!("audio extract Browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::AudioExtractOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, Some(directory.clone()));
        assert_eq!(options.file_name, "recording-audio.mka");
        assert_eq!(options.format, OutputFormat::Mka);

        let session = app.state.audio_extract.as_mut().unwrap();
        for (format, name, dialog_format) in [
            (
                AudioExtractFormat::Aac,
                "recording-audio.m4a",
                OutputFormat::M4a,
            ),
            (
                AudioExtractFormat::Flac,
                "recording-audio.flac",
                OutputFormat::Flac,
            ),
        ] {
            session.selection.set_format(format);
            let FileDialogRequest::SaveOutput { options, .. } =
                audio_extract_output_dialog_request(session)
            else {
                unreachable!();
            };
            assert_eq!(options.file_name, name);
            assert_eq!(options.format, dialog_format);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn frame_capture_output_dialog_request_follows_current_format() {
        let (directory, path) = temp_file("frame-capture-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);

        let session = app.state.frame_capture.as_ref().unwrap();
        let FileDialogRequest::SaveOutput { target, options } =
            frame_capture_output_dialog_request(session)
        else {
            panic!("frame capture Browse must create a save request");
        };
        assert_eq!(
            target,
            DialogTarget::FrameCaptureOutput {
                session_id: session.session_id,
            }
        );
        assert_eq!(options.initial_directory, Some(directory.clone()));
        assert_eq!(options.file_name, "recording-frame-00-00-00-000.png");
        assert_eq!(options.format, OutputFormat::Png);

        let session = app.state.frame_capture.as_mut().unwrap();
        session.selection.set_format(FrameCaptureFormat::Jpeg);
        let FileDialogRequest::SaveOutput { options, .. } =
            frame_capture_output_dialog_request(session)
        else {
            unreachable!();
        };
        assert_eq!(options.file_name, "recording-frame-00-00-00-000.jpg");
        assert_eq!(options.format, OutputFormat::Jpeg);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dialog_requests_prefer_independent_remembered_directories() {
        let (source_directory, source_path) = temp_file("remember-source");
        let output_directory = source_directory.join("remembered-output");
        fs::create_dir_all(&output_directory).unwrap();
        let mut app = app();
        app.settings
            .remember_source(&source_directory.join("previous-source.mp4"));
        app.settings
            .remember_output(&output_directory.join("previous-output.mkv"));

        let open = app.prepare_file_dialog_request(FileDialogRequest::OpenSource {
            initial_directory: None,
        });
        assert_eq!(
            open,
            FileDialogRequest::OpenSource {
                initial_directory: Some(source_directory.clone()),
            }
        );

        app.load_media_path_with_probe(&source_path, |path| Ok(media(path)));
        let save = app.prepare_file_dialog_request(audio_mix_output_dialog_request(
            app.state.audio_mix.as_ref().unwrap(),
        ));
        let FileDialogRequest::SaveOutput { options, .. } = save else {
            unreachable!();
        };
        assert_eq!(options.initial_directory, Some(output_directory));
        fs::remove_dir_all(source_directory).unwrap();
    }

    #[test]
    fn dialog_cancel_does_not_change_directory_memory() {
        let (directory, path) = temp_file("remember-cancel");
        let mut app = app();
        app.settings.remember_source(&path);
        app.settings
            .remember_output(&directory.join("existing.mkv"));
        let before = app.settings.clone();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Cancelled,
        });
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput { session_id: 1 },
            outcome: DialogOutcome::Cancelled,
        });

        assert_eq!(app.settings, before);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn successful_dialog_selections_update_only_their_directory_history() {
        let (directory, source_path) = temp_file("remember-success");
        let output_directory = directory.join("output");
        fs::create_dir_all(&output_directory).unwrap();
        let mut app = app();
        app.media_tools = MediaToolsState::Unavailable(MediaToolsError::MissingBoth);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Selected(source_path),
        });
        assert_eq!(app.settings.source_directory(), Some(directory.clone()));
        assert_eq!(app.settings.output_directory(Path::new("missing")), None);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput { session_id: 999 },
            outcome: DialogOutcome::Selected(output_directory.join("chosen.mkv")),
        });
        assert_eq!(app.settings.source_directory(), Some(directory.clone()));
        assert_eq!(
            app.settings.output_directory(Path::new("missing")),
            Some(output_directory)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn selected_output_paths_route_to_the_owning_tool() {
        let (directory, path) = temp_file("upscale-browse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let upscale_session_id = app.state.upscale.as_ref().unwrap().session_id;
        let destination = directory.join("chosen upscale.mp4");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::UpscaleOutput {
                session_id: upscale_session_id,
            },
            outcome: DialogOutcome::Selected(destination.clone()),
        });

        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            destination
        );

        app.select_tool(Tool::AudioMix);
        let audio_session_id = app.state.audio_mix.as_ref().unwrap().session_id;
        let audio_destination = directory.join("chosen mix.mp4");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput {
                session_id: audio_session_id,
            },
            outcome: DialogOutcome::Selected(audio_destination.clone()),
        });
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            audio_destination
        );

        app.select_tool(Tool::Remux);
        let remux_session_id = app.state.remux.as_ref().unwrap().session_id;
        let remux_destination = directory.join("chosen remux.mkv");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::RemuxOutput {
                session_id: remux_session_id,
            },
            outcome: DialogOutcome::Selected(remux_destination.clone()),
        });
        assert_eq!(
            app.state.remux.as_ref().unwrap().selection.output_path(),
            remux_destination
        );

        app.select_tool(Tool::AudioExtract);
        let extract_session_id = app.state.audio_extract.as_ref().unwrap().session_id;
        let extract_destination = directory.join("chosen audio.mka");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioExtractOutput {
                session_id: extract_session_id,
            },
            outcome: DialogOutcome::Selected(extract_destination.clone()),
        });
        assert_eq!(
            app.state
                .audio_extract
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            extract_destination
        );

        app.select_tool(Tool::FrameCapture);
        let capture_session_id = app.state.frame_capture.as_ref().unwrap().session_id;
        let capture_destination = directory.join("chosen frame.png");
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::FrameCaptureOutput {
                session_id: capture_session_id,
            },
            outcome: DialogOutcome::Selected(capture_destination.clone()),
        });
        assert_eq!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            capture_destination
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_audio_extract_completion_cannot_mutate_replacement_session() {
        let (directory, first) = temp_file("stale-audio-extract");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);
        let stale_session_id = app.state.audio_extract.as_ref().unwrap().session_id;

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_before = app
            .state
            .audio_extract
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioExtractOutput {
                session_id: stale_session_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale.mka")),
        });
        assert_eq!(
            app.state
                .audio_extract
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            replacement_before
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_frame_capture_completion_cannot_mutate_replacement_session() {
        let (directory, first) = temp_file("stale-frame-capture");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let stale_session_id = app.state.frame_capture.as_ref().unwrap().session_id;

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_before = app
            .state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::FrameCaptureOutput {
                session_id: stale_session_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale.png")),
        });
        assert_eq!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            replacement_before
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_remux_output_completion_cannot_mutate_replacement_session() {
        let (directory, first) = temp_file("stale-remux");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let stale_session_id = app.state.remux.as_ref().unwrap().session_id;

        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_before = app.state.remux.as_ref().unwrap().selection.output_path();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::RemuxOutput {
                session_id: stale_session_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale-result.mkv")),
        });

        let replacement = app.state.remux.as_ref().unwrap();
        assert_ne!(replacement.session_id, stale_session_id);
        assert_eq!(replacement.selection.output_path(), replacement_before);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cancelled_output_browse_preserves_ready_state() {
        let (directory, path) = temp_file("browse-cancel");
        let mut app = app();
        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let before = app.state.clone();
        let session_id = app.state.audio_mix.as_ref().unwrap().session_id;

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput { session_id },
            outcome: DialogOutcome::Cancelled,
        });

        assert_eq!(app.state, before);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pending_dialog_blocks_conflicting_app_actions_and_a_second_request() {
        let (directory, path) = temp_file("pending-dialog-boundary");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let (release_sender, release_receiver) = mpsc::channel();
        let mut release_receiver = Some(release_receiver);
        app.file_dialog = FileDialogOwner::new_for_test(egui::Context::default(), move || {
            Ok(move |_: &FileDialogRequest| {
                if let Some(receiver) = release_receiver.take() {
                    receiver.recv().unwrap();
                }
                Ok(None)
            })
        });
        app.file_dialog
            .request(FileDialogRequest::OpenSource {
                initial_directory: None,
            })
            .unwrap();

        assert!(!app.file_intake_enabled());
        assert!(app.workspace_interaction_enabled());
        assert_workspace(&app.state, Tool::AudioMix);
        assert_eq!(
            app.file_dialog.request(FileDialogRequest::OpenSource {
                initial_directory: None,
            }),
            Err(FileDialogStartError::AlreadyPending)
        );

        release_sender.send(()).unwrap();
        for _ in 0..100 {
            if app.file_dialog.poll().is_some() {
                fs::remove_dir_all(directory).unwrap();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("test dialog worker did not complete");
    }

    #[test]
    fn stale_output_completions_cannot_mutate_replacement_sessions() {
        let (directory, first) = temp_file("stale-dialog-session");
        let second = directory.join("replacement.mkv");
        let third = directory.join("third.mkv");
        fs::write(&second, b"replacement").unwrap();
        fs::write(&third, b"third").unwrap();
        let mut app = app();

        app.select_tool(Tool::AudioMix);
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        let old_audio_id = app.state.audio_mix.as_ref().unwrap().session_id;
        app.load_media_path_with_probe(&second, |path| Ok(media(path)));
        let replacement_audio = app.state.audio_mix.as_ref().unwrap();
        let replacement_audio_id = replacement_audio.session_id;
        let replacement_audio_output = replacement_audio.selection.output_path();
        assert_ne!(old_audio_id, replacement_audio_id);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::AudioMixOutput {
                session_id: old_audio_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale-mix.mp4")),
        });
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            replacement_audio_output
        );

        app.select_tool(Tool::VideoUpscale);
        let old_upscale_id = app.state.upscale.as_ref().unwrap().session_id;
        app.load_media_path_with_probe(&third, |path| Ok(media(path)));
        let replacement_upscale = app.state.upscale.as_ref().unwrap();
        let replacement_upscale_id = replacement_upscale.session_id;
        let replacement_upscale_output = replacement_upscale.selection.output_path();
        assert_ne!(old_upscale_id, replacement_upscale_id);

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::UpscaleOutput {
                session_id: old_upscale_id,
            },
            outcome: DialogOutcome::Selected(directory.join("stale-upscale.mp4")),
        });
        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            replacement_upscale_output
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn output_completion_cannot_mutate_selection_after_execution_starts() {
        let (directory, path) = temp_file("dialog-after-execution");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let audio_id = app.state.audio_mix.as_ref().unwrap().session_id;
        let audio_output = app
            .state
            .audio_mix
            .as_ref()
            .unwrap()
            .selection
            .output_path();

        settle_output_file_dialog(
            &mut app.state,
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Running,
                upscale: &UpscaleExecutionState::Idle,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Idle,
                frame_capture: &FrameCaptureExecutionState::Idle,
            },
            DialogTarget::AudioMixOutput {
                session_id: audio_id,
            },
            DialogOutcome::Selected(directory.join("late-mix.mp4")),
        );
        assert_eq!(
            app.state
                .audio_mix
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            audio_output
        );

        app.select_tool(Tool::VideoUpscale);
        let upscale_id = app.state.upscale.as_ref().unwrap().session_id;
        let upscale_output = app.state.upscale.as_ref().unwrap().selection.output_path();
        settle_output_file_dialog(
            &mut app.state,
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Idle,
                upscale: &UpscaleExecutionState::Running,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Idle,
                frame_capture: &FrameCaptureExecutionState::Idle,
            },
            DialogTarget::UpscaleOutput {
                session_id: upscale_id,
            },
            DialogOutcome::Selected(directory.join("late-upscale.mp4")),
        );
        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            upscale_output
        );

        app.select_tool(Tool::AudioExtract);
        let extract_id = app.state.audio_extract.as_ref().unwrap().session_id;
        let extract_output = app
            .state
            .audio_extract
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        settle_output_file_dialog(
            &mut app.state,
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Idle,
                upscale: &UpscaleExecutionState::Idle,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Running,
                frame_capture: &FrameCaptureExecutionState::Idle,
            },
            DialogTarget::AudioExtractOutput {
                session_id: extract_id,
            },
            DialogOutcome::Selected(directory.join("late-audio.mka")),
        );
        assert_eq!(
            app.state
                .audio_extract
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            extract_output
        );

        app.select_tool(Tool::FrameCapture);
        let capture_id = app.state.frame_capture.as_ref().unwrap().session_id;
        let capture_output = app
            .state
            .frame_capture
            .as_ref()
            .unwrap()
            .selection
            .output_path();
        settle_output_file_dialog(
            &mut app.state,
            ToolExecutionStates {
                audio_mix: &AudioMixExecutionState::Idle,
                upscale: &UpscaleExecutionState::Idle,
                remux: &RemuxExecutionState::Idle,
                audio_extract: &AudioExtractExecutionState::Idle,
                frame_capture: &FrameCaptureExecutionState::Running,
            },
            DialogTarget::FrameCaptureOutput {
                session_id: capture_id,
            },
            DialogOutcome::Selected(directory.join("late-frame.png")),
        );
        assert_eq!(
            app.state
                .frame_capture
                .as_ref()
                .unwrap()
                .selection
                .output_path(),
            capture_output
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_dialog_selected_path_starts_the_shared_intake_route() {
        let (directory, path) = temp_file("dialog-open-route");
        let mut app = app();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Selected(path.clone()),
        });

        let inspection = app
            .media_inspection
            .as_ref()
            .expect("selected Open path should start media inspection");
        assert_eq!(inspection.request.path(), path);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_dialog_failure_surfaces_bounded_intake_feedback() {
        let mut app = app();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::MediaInput,
            outcome: DialogOutcome::Failed("native open diagnostic marker".to_owned()),
        });

        let error = app
            .state
            .intake_error
            .as_ref()
            .expect("open dialog failure should become intake state");
        assert_eq!(error.kind(), MediaErrorKind::FilePickerUnavailable);
        assert_eq!(error.diagnostic(), Some("native open diagnostic marker"));
        assert!(!app.state.intake_error_details_open);
    }

    #[test]
    fn output_picker_failure_surfaces_bounded_user_feedback() {
        let (directory, path) = temp_file("browse-failure");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let output_before = app
            .state
            .upscale
            .as_ref()
            .unwrap()
            .selection
            .output_path()
            .to_path_buf();
        let session_id = app.state.upscale.as_ref().unwrap().session_id;

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::UpscaleOutput { session_id },
            outcome: DialogOutcome::Failed("native picker diagnostic marker".to_owned()),
        });

        let error = app
            .state
            .upscale
            .as_ref()
            .unwrap()
            .start_error
            .as_ref()
            .expect("picker failure should become user-facing state");
        assert_eq!(error.kind(), MediaErrorKind::OutputPickerUnavailable);
        assert_eq!(error.diagnostic(), Some("native picker diagnostic marker"));
        assert_eq!(
            app.state.upscale.as_ref().unwrap().selection.output_path(),
            output_before
        );

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut rendered = context.run_ui(Default::default(), |ui| {
            assert!(app.show_upscale_start_error(ui));
        });
        assert!(output_contains_text(&rendered, "Output picker unavailable"));
        assert!(output_contains_text(
            &rendered,
            "The output picker could not be opened."
        ));
        assert!(!output_contains_text(
            &rendered,
            "native picker diagnostic marker"
        ));
        rendered.textures_delta.clear();

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_intent_reaches_executor() {
        let (directory, path) = temp_file("execute");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);

        let error = app
            .handle_upscale_action(UpscaleSelectionAction::UpscaleRequested)
            .expect_err("missing test FFmpeg should prove the execution path was attempted");

        assert!(matches!(
            error,
            apollo::upscale::execution::UpscaleStartError::Process(
                apollo::media::process::MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound
                )
            )
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_ready_request_owner_builds_an_immutable_queue_operation() {
        let (directory, path) = temp_file("queue-requests");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        for operation in [
            Tool::AudioMix,
            Tool::VideoUpscale,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
        ] {
            app.select_tool(operation);
            assert!(app.enqueue_current_operation(operation).unwrap().is_some());
        }

        assert_eq!(app.media_queue.jobs().len(), 5);
        assert!(matches!(
            app.media_queue.jobs()[0].operation(),
            QueuedOperation::AudioMix(_)
        ));
        assert!(matches!(
            app.media_queue.jobs()[1].operation(),
            QueuedOperation::Upscale(_)
        ));
        assert!(matches!(
            app.media_queue.jobs()[2].operation(),
            QueuedOperation::Remux(_)
        ));
        assert!(matches!(
            app.media_queue.jobs()[3].operation(),
            QueuedOperation::AudioExtract(_)
        ));
        assert!(matches!(
            app.media_queue.jobs()[4].operation(),
            QueuedOperation::FrameCapture(_)
        ));
        assert!(
            app.media_queue
                .jobs()
                .iter()
                .all(|job| matches!(job.state(), QueueJobState::Waiting))
        );
        assert!(!app.immediate_operation_running());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_ready_enqueue_action_remains_in_the_workspace_without_launching() {
        let (directory, path) = temp_file("queue-actions");
        let mut app = app();

        for operation in [
            Tool::AudioMix,
            Tool::VideoUpscale,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
        ] {
            app.load_media_path_with_probe(&path, |path| Ok(media(path)));
            app.select_tool(operation);
            let previous_count = app.media_queue.jobs().len();

            match operation {
                Tool::AudioMix => app
                    .handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
                    .unwrap(),
                Tool::VideoUpscale => app
                    .handle_upscale_action(UpscaleSelectionAction::QueueRequested)
                    .unwrap(),
                Tool::Remux => app
                    .handle_remux_action(RemuxSelectionAction::QueueRequested)
                    .unwrap(),
                Tool::AudioExtract => app
                    .handle_audio_extract_action(AudioExtractSelectionAction::QueueRequested)
                    .unwrap(),
                Tool::FrameCapture => app
                    .handle_frame_capture_action(FrameCaptureSelectionAction::QueueRequested)
                    .unwrap(),
            }

            assert_eq!(app.media_queue.jobs().len(), previous_count + 1);
            assert_eq!(
                app.state.navigation,
                NavigationState::SourceWorkspace {
                    active_operation: operation
                }
            );
            assert!(!app.immediate_operation_running());
            assert!(!app.media_queue.is_running());
        }

        assert!(
            app.media_queue
                .jobs()
                .iter()
                .all(|job| matches!(job.state(), QueueJobState::Waiting))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_ready_request_cannot_be_enqueued_by_direct_intent() {
        let (directory, path) = temp_file("invalid-queue-action");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.state.audio_mix.as_mut().unwrap().selection.clear();

        app.handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
            .unwrap();

        assert!(app.media_queue.jobs().is_empty());
        assert_eq!(app.state.queue_confirmation, None);
        assert!(matches!(
            app.state.navigation,
            NavigationState::SourceWorkspace {
                active_operation: Tool::AudioMix
            }
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn duplicate_queued_destination_is_truthful_operation_local_feedback() {
        let (directory, path) = temp_file("queue-confirmation");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        app.handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
            .unwrap();

        assert_eq!(app.media_queue.pending_count(), 1);
        let output_path = app.media_queue.jobs()[0].operation().output().to_path_buf();
        assert!(!output_path.exists());
        assert_eq!(app.state.queue_confirmation, Some(Tool::AudioMix));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        assert!(output_contains_text(&output, "Added to queue"));
        output.textures_delta.clear();

        app.handle_audio_mix_action(AudioMixSelectionAction::QueueRequested)
            .unwrap();
        assert_eq!(app.media_queue.pending_count(), 1);
        assert!(!output_path.exists());
        assert_eq!(app.state.queue_confirmation, None);
        assert!(app.state.intake_error.is_none());
        assert!(matches!(
            app.state.operation_action_error,
            Some((Tool::AudioMix, ref error))
                if error.kind() == MediaErrorKind::QueueDestinationReserved
                    && error.diagnostic().is_some_and(|diagnostic| diagnostic.contains("queue job 1"))
        ));

        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        assert!(output_contains_text(
            &output,
            "Another queued job already targets this output."
        ));
        output.textures_delta.clear();

        let error = app
            .state
            .operation_action_error
            .as_ref()
            .map(|(_, error)| error.clone())
            .unwrap();
        let mut details_open = true;
        let mut output = context.run_ui(Default::default(), |ui| {
            media_error::show_inline(ui, &error, &mut details_open)
        });
        assert!(output_contains_text(&output, "queue job 1"));
        output.textures_delta.clear();

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn genuine_existing_output_keeps_filesystem_output_exists_feedback() {
        let (directory, path) = temp_file("queue-existing-output");
        let existing_output = directory.join("already-there.mp4");
        fs::write(&existing_output, b"existing output").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.state
            .audio_mix
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(existing_output);

        app.enqueue_from_ready_surface(Tool::AudioMix);

        assert!(app.media_queue.jobs().is_empty());
        assert!(app.state.intake_error.is_none());
        assert!(matches!(
            app.state.operation_action_error,
            Some((Tool::AudioMix, ref error))
                if error.kind() == MediaErrorKind::OutputExists
                    && error.guidance() == Some("Choose another output path or move the existing file.")
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn batch_append_ignores_duplicates_without_mutating_existing_configuration() {
        let (directory, path) = temp_file("batch-append-duplicate");
        let source = media(&path);
        let operation = QueuedOperation::Remux(
            apollo::ui::remux::RemuxSelectionState::new(&source).request(&source),
        );
        let mut app = app();
        app.batch_setup = Some(BatchSetupState {
            origin: BatchOrigin::NoSource,
            operation: Tool::Remux,
            defaults: Some(BatchDefaults::Remux {
                container: MediaContainer::Matroska,
            }),
            items: vec![BatchItem {
                path: path.clone(),
                included: true,
                source: BatchSourceState::Ready(source),
                operation: Some(operation.clone()),
                readiness: BatchReadiness::Ready,
                reason: None,
                overridden: true,
            }],
            error: None,
            append_error: None,
            append_error_details_open: false,
        });

        assert!(!app.append_batch_paths(vec![path.clone(), path.clone()]));
        let batch = app.batch_setup.as_ref().unwrap();
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].operation, Some(operation));
        assert!(batch.items[0].overridden);
        assert_eq!(
            batch.error.as_deref(),
            Some("Ignored 2 files that are already in this batch.")
        );
        assert!(app.batch_inspection.is_none());
        assert!(app.media_queue.jobs().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn batch_append_picker_failure_is_visible_and_never_leaks_to_intake() {
        let (directory, path) = temp_file("batch-append-picker-failure");
        let source = media(&path);
        let operation = QueuedOperation::Remux(
            apollo::ui::remux::RemuxSelectionState::new(&source).request(&source),
        );
        let mut app = app();
        app.batch_setup = Some(BatchSetupState {
            origin: BatchOrigin::NoSource,
            operation: Tool::Remux,
            defaults: Some(BatchDefaults::Remux {
                container: MediaContainer::Matroska,
            }),
            items: vec![BatchItem {
                path,
                included: true,
                source: BatchSourceState::Ready(source),
                operation: Some(operation),
                readiness: BatchReadiness::Ready,
                reason: None,
                overridden: true,
            }],
            error: None,
            append_error: None,
            append_error_details_open: false,
        });
        app.state.navigation = NavigationState::BatchSetup;

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::BatchAppend,
            outcome: DialogOutcome::Failed("native picker diagnostic".to_owned()),
        });

        assert!(app.state.intake_error.is_none());
        let batch = app.batch_setup.as_ref().unwrap();
        assert!(matches!(
            batch.append_error,
            Some(ref error)
                if error.kind() == MediaErrorKind::FilePickerUnavailable
                    && error.diagnostic() == Some("native picker diagnostic")
        ));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        assert!(output_contains_text(
            &output,
            "The file picker could not be opened."
        ));
        output.textures_delta.clear();

        assert!(app.finish_batch_setup());
        assert!(app.state.intake_error.is_none());
        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));
        assert!(!output_contains_text(
            &output,
            "The file picker could not be opened."
        ));
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn batch_append_picker_cancellation_preserves_the_complete_batch_state() {
        let (directory, path) = temp_file("batch-append-picker-cancel");
        let source = media(&path);
        let operation = QueuedOperation::Remux(
            apollo::ui::remux::RemuxSelectionState::new(&source).request(&source),
        );
        let mut app = app();
        app.batch_setup = Some(BatchSetupState {
            origin: BatchOrigin::NoSource,
            operation: Tool::Remux,
            defaults: Some(BatchDefaults::Remux {
                container: MediaContainer::Matroska,
            }),
            items: vec![BatchItem {
                path,
                included: false,
                source: BatchSourceState::Ready(source),
                operation: Some(operation),
                readiness: BatchReadiness::Ready,
                reason: None,
                overridden: true,
            }],
            error: Some("Existing batch feedback".to_owned()),
            append_error: Some(MediaError::file_picker_unavailable(
                "existing append diagnostic",
            )),
            append_error_details_open: true,
        });
        app.state.navigation = NavigationState::BatchSetup;
        let before = app.batch_setup.clone();

        app.settle_file_dialog(FileDialogCompletion {
            target: DialogTarget::BatchAppend,
            outcome: DialogOutcome::Cancelled,
        });

        assert_eq!(app.batch_setup, before);
        assert!(app.state.intake_error.is_none());
        assert!(app.media_queue.jobs().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn relevant_workspace_actions_clear_enqueue_confirmation() {
        let (directory, path) = temp_file("queue-confirmation-clear");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.enqueue_from_ready_surface(Tool::AudioMix);
        assert_eq!(app.state.queue_confirmation, Some(Tool::AudioMix));

        app.select_tool(Tool::Remux);
        assert_eq!(app.state.queue_confirmation, None);

        app.select_tool(Tool::AudioMix);
        app.state
            .audio_mix
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(directory.join("another.mkv"));
        app.enqueue_from_ready_surface(Tool::AudioMix);
        let job_id = app.media_queue.jobs()[1].id();
        assert_eq!(app.state.queue_confirmation, Some(Tool::AudioMix));
        assert!(app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
        assert_eq!(app.state.queue_confirmation, None);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn intake_queue_navigation_and_back_preserve_jobs() {
        let (directory, path) = temp_file("queue-navigation");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.enqueue_from_ready_surface(Tool::AudioMix);
        let job_id = app.media_queue.jobs()[0].id();
        app.state.close_source();

        app.handle_file_intake_action(FileIntakeAction::OpenQueue);
        assert_eq!(
            app.state.navigation,
            NavigationState::Queue {
                origin: QueueOrigin::NoSource
            }
        );
        assert!(app.handle_queue_action(QueueAction::Back));

        assert_eq!(app.state.navigation, NavigationState::NoSource);
        assert_eq!(app.media_queue.jobs()[0].id(), job_id);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn workspace_queue_navigation_restores_the_same_source_and_operation() {
        let (directory, path) = temp_file("workspace-queue-navigation");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let session_id = app.state.remux.as_ref().unwrap().session_id;

        app.state.enter_queue();
        assert_eq!(
            app.state.navigation,
            NavigationState::Queue {
                origin: QueueOrigin::SourceWorkspace {
                    active_operation: Tool::Remux
                }
            }
        );
        assert!(app.handle_queue_action(QueueAction::Back));

        assert_workspace(&app.state, Tool::Remux);
        assert_eq!(app.state.media_intake.current().unwrap().path, path);
        assert_eq!(app.state.remux.as_ref().unwrap().session_id, session_id);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_back_clears_disposable_history_and_retains_failures() {
        let (directory, path) = temp_file("queue-back-history");
        let mut app = app();
        populate_cancelled_and_failed_queue_history(&mut app, &path, &directory);
        app.state.enter_queue();

        assert!(app.handle_queue_action(QueueAction::Back));

        assert_eq!(app.media_queue.jobs().len(), 1);
        assert!(matches!(
            app.media_queue.jobs()[0].state(),
            QueueJobState::Failed(_)
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_add_files_clears_disposable_history_before_opening_intake() {
        let (directory, path) = temp_file("queue-add-files-history");
        let mut app = app();
        populate_cancelled_and_failed_queue_history(&mut app, &path, &directory);
        app.state.enter_queue();

        assert!(app.handle_queue_action(QueueAction::AddFiles));

        assert_eq!(app.media_queue.jobs().len(), 1);
        assert!(matches!(
            app.media_queue.jobs()[0].state(),
            QueueJobState::Failed(_)
        ));
        assert!(app.handle_queue_action(QueueAction::ClearFinished));
        assert!(app.media_queue.jobs().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_edit_hydrates_the_queued_request_and_cancel_preserves_it() {
        let (directory, path) = temp_file("queue-edit-cancel");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let custom_output = directory.join("queued-custom.mp4");
        let selection = &mut app.state.remux.as_mut().unwrap().selection;
        selection.set_container(MediaContainer::Mp4);
        selection.set_custom_output_path(custom_output);
        let job_id = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        let original = app.media_queue.jobs()[0].operation().clone();
        app.state.enter_queue();

        assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
        let edit = app.queue_edit.as_mut().unwrap();
        assert_eq!(edit.active_operation, Tool::Remux);
        assert_eq!(
            QueuedOperation::Remux(edit.remux.request(&edit.media)),
            original
        );
        edit.remux.set_container(MediaContainer::Matroska);

        assert!(app.finish_queue_edit());
        assert_eq!(app.media_queue.jobs()[0].id(), job_id);
        assert_eq!(app.media_queue.jobs()[0].operation(), &original);
        assert!(matches!(
            app.state.navigation,
            NavigationState::Queue { .. }
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn workspace_queue_edit_targets_exact_job_and_restores_retained_source_without_probe() {
        let (first_directory, first_path) = temp_file("workspace-edit-first");
        let (second_directory, second_path) = temp_file("workspace-edit-second");
        let mut app = app();
        app.load_media_path_with_probe(&first_path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let first_output = first_directory.join("first-output.mp4");
        let first_selection = &mut app.state.remux.as_mut().unwrap().selection;
        first_selection.set_container(MediaContainer::Mp4);
        first_selection.set_custom_output_path(first_output);
        let first_job = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        let first_request = app.media_queue.jobs()[0].operation().clone();

        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(first_directory.join("second-output.mkv"));
        let second_job = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        app.load_media_path_with_probe(&second_path, |path| Ok(media(path)));
        assert_eq!(app.state.media_intake.current().unwrap().path, second_path);

        assert!(app.begin_queue_edit(first_job, QueueEditOrigin::SourceWorkspace));
        assert_eq!(app.queue_edit.as_ref().unwrap().job_id, first_job);
        assert_eq!(
            QueuedOperation::Remux(
                app.queue_edit
                    .as_ref()
                    .unwrap()
                    .remux
                    .request(&app.queue_edit.as_ref().unwrap().media)
            ),
            first_request
        );
        assert!(app.media_inspection.is_none());

        assert!(app.finish_queue_edit());
        assert_workspace(&app.state, Tool::Remux);
        assert_eq!(app.state.media_intake.current().unwrap().path, first_path);
        assert_eq!(
            QueuedOperation::Remux(
                app.state
                    .remux
                    .as_ref()
                    .unwrap()
                    .selection
                    .request(&app.state.remux.as_ref().unwrap().media)
            ),
            first_request
        );
        assert!(app.media_inspection.is_none());
        assert_eq!(app.media_queue.jobs()[1].id(), second_job);

        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn workspace_queue_edit_save_returns_to_saved_operation_with_same_job_id() {
        let (directory, path) = temp_file("workspace-edit-save");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let job_id = app
            .enqueue_current_operation(Tool::AudioMix)
            .unwrap()
            .unwrap();

        assert!(app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
        app.queue_edit.as_mut().unwrap().active_operation = Tool::FrameCapture;
        assert!(app.save_queue_edit());

        assert_workspace(&app.state, Tool::FrameCapture);
        assert_eq!(app.media_queue.jobs().len(), 1);
        assert_eq!(app.media_queue.jobs()[0].id(), job_id);
        assert!(matches!(
            app.media_queue.jobs()[0].operation(),
            QueuedOperation::FrameCapture(_)
        ));
        assert!(app.media_inspection.is_none());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn workspace_queue_edit_is_blocked_during_automatic_advancement() {
        let (directory, path) = temp_file("workspace-edit-running");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let job_id = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        app.media_tools = MediaToolsState::Ready(MediaTools::new("cmd.exe", "missing-ffprobe"));

        assert!(app.start_media_queue());
        assert_ne!(app.media_queue.run_state(), QueueRunState::Stopped);
        assert!(!app.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace));
        assert!(app.queue_edit.is_none());
        assert_workspace(&app.state, Tool::Remux);
        assert!(app.media_queue.cancel_queue());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_edit_hydrates_each_operation_from_its_immutable_request() {
        let (directory, path) = temp_file("queue-edit-hydration");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let mut job_ids = Vec::new();
        for operation in [
            Tool::AudioMix,
            Tool::VideoUpscale,
            Tool::Remux,
            Tool::AudioExtract,
            Tool::FrameCapture,
        ] {
            app.select_tool(operation);
            job_ids.push(app.enqueue_current_operation(operation).unwrap().unwrap());
        }
        app.state.enter_queue();

        for job_id in job_ids {
            let original = app
                .media_queue
                .jobs()
                .iter()
                .find(|job| job.id() == job_id)
                .unwrap()
                .operation()
                .clone();
            assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
            let edit = app.queue_edit.as_mut().unwrap();
            let hydrated = match &original {
                QueuedOperation::AudioMix(_) => {
                    QueuedOperation::AudioMix(edit.audio_mix.request(&edit.media))
                }
                QueuedOperation::Upscale(request) => {
                    QueuedOperation::Upscale(edit.upscale.request_with_nvenc_capability(
                        &edit.media,
                        request.nvenc_capability.clone(),
                    ))
                }
                QueuedOperation::Remux(_) => {
                    QueuedOperation::Remux(edit.remux.request(&edit.media))
                }
                QueuedOperation::AudioExtract(_) => {
                    QueuedOperation::AudioExtract(edit.audio_extract.request(&edit.media).unwrap())
                }
                QueuedOperation::FrameCapture(_) => {
                    QueuedOperation::FrameCapture(edit.frame_capture.request(&edit.media).unwrap())
                }
            };
            assert_eq!(hydrated, original);
            assert!(app.finish_queue_edit());
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_edit_can_change_operation_with_atomic_same_id_replacement() {
        let (directory, path) = temp_file("queue-edit-replace");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let job_id = app
            .enqueue_current_operation(Tool::AudioMix)
            .unwrap()
            .unwrap();
        app.state.enter_queue();
        assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
        app.queue_edit.as_mut().unwrap().active_operation = Tool::FrameCapture;

        assert!(app.save_queue_edit());

        assert_eq!(app.media_queue.jobs().len(), 1);
        assert_eq!(app.media_queue.jobs()[0].id(), job_id);
        assert!(matches!(
            app.media_queue.jobs()[0].operation(),
            QueuedOperation::FrameCapture(_)
        ));
        assert!(matches!(
            app.state.navigation,
            NavigationState::Queue { .. }
        ));
        assert!(app.queue_edit.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_edit_reuses_workspace_without_source_or_execution_actions() {
        let (directory, path) = temp_file("queue-edit-surface");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let job_id = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        app.state.enter_queue();
        assert!(app.handle_queue_action(QueueAction::Edit(job_id)));
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));

        for label in [
            "Editing queued item",
            "Audio Mix",
            "Upscale",
            "Remux",
            "Audio Extract",
            "Frame Capture",
            "Save changes",
            "Cancel edit",
        ] {
            assert!(output_contains_text(&output, label), "missing {label:?}");
        }
        for label in ["Change file", "Close source", "Add to queue"] {
            assert!(
                !output_contains_text(&output, label),
                "unexpected {label:?}"
            );
        }
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stopped_queue_surface_exposes_edit_remove_and_bounded_reorder_controls() {
        let (directory, path) = temp_file("queue-controls-surface");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        app.state.enter_queue();
        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);

        let mut output = context.run_ui(Default::default(), |ui| app.show_navigation(ui));

        for label in ["Edit", "Remove", "↑", "↓", "Start queue", "Back"] {
            assert!(output_contains_text(&output, label), "missing {label:?}");
        }
        output.textures_delta.clear();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_edit_failed_replacement_keeps_original_job_and_editor() {
        let (directory, path) = temp_file("queue-edit-failure");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let first = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(directory.join("second.mkv"));
        let second = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        let original = app.media_queue.jobs()[0].operation().clone();
        let duplicate_output = app.media_queue.jobs()[1].operation().output().to_path_buf();
        app.state.enter_queue();
        assert!(app.handle_queue_action(QueueAction::Edit(first)));
        app.queue_edit
            .as_mut()
            .unwrap()
            .remux
            .set_custom_output_path(duplicate_output);

        assert!(!app.save_queue_edit());

        assert_eq!(app.media_queue.jobs()[0].id(), first);
        assert_eq!(app.media_queue.jobs()[0].operation(), &original);
        assert_eq!(app.media_queue.jobs()[1].id(), second);
        assert!(app.queue_edit.as_ref().unwrap().error.is_some());
        assert!(matches!(
            app.state.navigation,
            NavigationState::QueueEdit { .. }
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_reorder_actions_move_only_the_requested_waiting_item() {
        let (directory, path) = temp_file("queue-reorder-actions");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let first = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(directory.join("later.mkv"));
        let second = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();

        assert!(app.handle_queue_action(QueueAction::MoveEarlier(second)));
        assert_eq!(app.media_queue.jobs()[0].id(), second);
        assert_eq!(app.media_queue.jobs()[1].id(), first);
        assert!(app.handle_queue_action(QueueAction::MoveLater(second)));
        assert_eq!(app.media_queue.jobs()[0].id(), first);
        assert_eq!(app.media_queue.jobs()[1].id(), second);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn completed_queue_output_resolves_its_parent_folder_only_for_success() {
        let completed = QueueJobState::Completed {
            output: PathBuf::from("exports/output.mkv"),
        };
        assert_eq!(
            completed_output_folder(&completed),
            Some(Path::new("exports"))
        );
        assert_eq!(completed_output_folder(&QueueJobState::Waiting), None);
        assert_eq!(
            completed_output_folder(&QueueJobState::Failed(QueueJobFailure::SourceChanged)),
            None
        );
    }

    #[test]
    fn queue_remove_and_clear_actions_respect_job_state() {
        let (directory, path) = temp_file("queue-row-actions");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let waiting = app
            .enqueue_current_operation(Tool::AudioMix)
            .unwrap()
            .unwrap();

        assert!(app.handle_queue_action(QueueAction::Remove(waiting)));
        assert!(app.media_queue.jobs().is_empty());

        let terminal = app
            .enqueue_current_operation(Tool::AudioMix)
            .unwrap()
            .unwrap();
        assert!(app.handle_queue_action(QueueAction::CancelQueue));
        assert!(!app.handle_queue_action(QueueAction::Remove(terminal)));
        assert!(app.handle_queue_action(QueueAction::ClearFinished));
        assert!(app.media_queue.jobs().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_terminal_failure_does_not_replace_the_ready_source_session() {
        let (directory, path) = temp_file("queue-session");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let session_id = app.state.remux.as_ref().unwrap().session_id;
        let job_id = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();

        assert!(!app.start_media_queue());

        assert_workspace(&app.state, Tool::Remux);
        assert_eq!(app.state.remux.as_ref().unwrap().session_id, session_id);
        assert_eq!(app.media_queue.jobs()[0].id(), job_id);
        assert!(matches!(
            app.media_queue.jobs()[0].state(),
            QueueJobState::Failed(QueueJobFailure::ProcessStart(_))
        ));
        assert!(!app.immediate_operation_running());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_process_events_are_tagged_and_filtered_by_job_id() {
        let (directory, path) = temp_file("queue-events");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let first = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        app.state
            .remux
            .as_mut()
            .unwrap()
            .selection
            .set_custom_output_path(directory.join("second-remux.mkv"));
        let second = app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();
        let (sender, receiver) = mpsc::channel();
        let wake = queue_execution_wake(first, sender, MediaProcessWake::default());

        wake.notify(MediaProcessEvent::Progress);
        let event = receiver.recv_timeout(Duration::from_secs(1)).unwrap();

        assert_eq!(
            event,
            QueueExecutionEvent {
                job_id: first,
                process_event: MediaProcessEvent::Progress,
            }
        );
        assert!(queue_event_matches_active(Some(first), event));
        assert!(!queue_event_matches_active(Some(second), event));
        assert!(!queue_event_matches_active(None, event));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_control_stop_cancels_waiting_work_used_by_app_shutdown() {
        let (directory, path) = temp_file("queue-shutdown");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        app.enqueue_current_operation(Tool::Remux).unwrap().unwrap();

        assert!(app.handle_queue_action(QueueAction::CancelQueue));

        assert!(matches!(
            app.media_queue.jobs()[0].state(),
            QueueJobState::Cancelled
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn application_arbitration_blocks_only_the_competing_execution_owner() {
        let immediate = ProcessExecutionArbitration::new(true, false);
        assert!(!immediate.queue_start_allowed());
        assert!(immediate.immediate_start_allowed());

        let queued = ProcessExecutionArbitration::new(false, true);
        assert!(!queued.immediate_start_allowed());
        assert!(queued.queue_start_allowed());
    }

    #[test]
    fn remux_intent_reaches_executor() {
        let (directory, path) = temp_file("remux-execute");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);

        let error = app
            .handle_remux_action(RemuxSelectionAction::RemuxRequested)
            .expect_err("missing test FFmpeg should prove the execution path was attempted");

        assert!(matches!(
            error,
            apollo::remux::execution::RemuxStartError::Process(
                apollo::media::process::MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound
                )
            )
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_extract_intent_reaches_executor() {
        let (directory, path) = temp_file("audio-extract-execute");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);

        let error = app
            .handle_audio_extract_action(AudioExtractSelectionAction::ExtractRequested)
            .expect_err("missing test FFmpeg should prove the execution path was attempted");
        assert!(matches!(
            error,
            apollo::audio_extract::execution::AudioExtractStartError::Process(
                apollo::media::process::MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound
                )
            )
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn frame_capture_intent_reaches_executor() {
        let (directory, path) = temp_file("frame-capture-execute");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);

        let error = app
            .handle_frame_capture_action(FrameCaptureSelectionAction::CaptureRequested)
            .expect_err("missing test FFmpeg should prove the execution path was attempted");
        assert!(matches!(
            error,
            apollo::frame_capture::execution::FrameCaptureStartError::Process(
                apollo::media::process::MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound
                )
            )
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        use apollo::media::{execution::MediaExecutionFailure, process::MediaProcessFailure};

        assert!(!shows_upscale_lifecycle(&UpscaleExecutionState::Idle));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Running));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Completed {
            output: PathBuf::from("recording-4k.mp4"),
        }));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Failed(
            MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            }
        )));
        assert!(shows_upscale_lifecycle(&UpscaleExecutionState::Cancelled));
    }

    #[test]
    fn remux_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        assert!(!shows_remux_lifecycle(&RemuxExecutionState::Idle));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Running));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Completed {
            output: PathBuf::from("recording-remuxed.mkv"),
        }));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Failed(
            MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            }
        )));
        assert!(shows_remux_lifecycle(&RemuxExecutionState::Cancelled));
    }

    #[test]
    fn audio_extract_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        assert!(!shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Idle
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Running
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Completed {
                output: PathBuf::from("recording-audio.mka"),
            }
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            })
        ));
        assert!(shows_audio_extract_lifecycle(
            &AudioExtractExecutionState::Cancelled
        ));
    }

    #[test]
    fn frame_capture_execution_state_routes_running_and_terminal_surfaces_to_lifecycle_ui() {
        assert!(!shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Idle
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Running
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Completed {
                output: PathBuf::from("recording-frame.png"),
            }
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Failed(MediaExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(1)),
                diagnostic: String::new(),
            })
        ));
        assert!(shows_frame_capture_lifecycle(
            &FrameCaptureExecutionState::Cancelled
        ));
    }

    #[test]
    fn change_file_probe_failure_is_rendered_and_survives_operation_switching() {
        let (directory, first) = temp_file("stale-first");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let before = app.state.clone();
        app.load_media_path_with_probe(&second, |_| {
            Err(apollo::media::probe::MediaProbeError::FfprobeFailed {
                code: Some(1),
                diagnostic: "probe failed".to_owned(),
            })
        });

        assert_workspace(&app.state, Tool::VideoUpscale);
        assert_eq!(app.state.upscale, before.upscale);
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&first)
        );
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );

        let context = egui::Context::default();
        fonts::install(&context);
        theme::install(&context);
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });
        output.textures_delta.clear();
        assert!(output_contains_text(
            &output,
            "This file could not be inspected."
        ));
        assert!(output_contains_text(
            &output,
            "Check that the source is a readable media file and try again."
        ));
        assert!(output_contains_text(&output, "Upscale to"));
        assert!(output_contains_text(&output, "Change file"));
        assert!(output_contains_text(&output, "Audio Mix"));

        app.select_tool(Tool::Remux);

        assert_workspace(&app.state, Tool::Remux);
        assert_eq!(app.state.upscale, before.upscale);
        assert_eq!(
            app.state.intake_error.as_ref().map(MediaError::kind),
            Some(MediaErrorKind::ProbeFailure)
        );
        let mut output = context.run_ui(Default::default(), |ui| {
            app.show_navigation(ui);
        });
        output.textures_delta.clear();
        assert!(output_contains_text(
            &output,
            "This file could not be inspected."
        ));
        assert!(output_contains_text(&output, "Remux"));
        assert!(output_contains_text(&output, "Change file"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switching_tools_reuses_successful_probe_without_probing_again() {
        let (directory, path) = temp_file("reuse");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));

        app.select_tool(Tool::VideoUpscale);

        assert_workspace(&app.state, Tool::VideoUpscale);
        assert_eq!(
            app.state.upscale.as_ref().map(|s| &s.media.path),
            Some(&path)
        );
        assert_eq!(
            app.state.media_intake.current().map(|m| &m.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_mix_state_survives_operation_switching() {
        let (directory, path) = temp_file("audio-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        let session = app
            .state
            .audio_mix
            .as_mut()
            .expect("audio mix session should exist");
        session.selection.toggle_stream(&session.media, 1);
        assert!(session.selection.set_track_gain(
            &session.media,
            1,
            TrackGain::from_tenths_db(-55),
        ));
        session
            .selection
            .set_container(&session.media, OutputContainer::Mkv);
        let custom_output = directory.join("custom-mix.mkv");
        session
            .selection
            .set_custom_output_path(custom_output.clone());
        let normalization_targets = LoudnessTargets::from_tenths(-205, -8, 145);
        assert!(
            session
                .selection
                .set_normalization_targets(normalization_targets)
        );
        session.selection.set_normalization_enabled(true);

        app.select_tool(Tool::VideoUpscale);
        app.select_tool(Tool::AudioMix);

        let session = app
            .state
            .audio_mix
            .as_ref()
            .expect("same-source Audio Mix session should be restored");
        assert_workspace(&app.state, Tool::AudioMix);
        assert_eq!(session.selection.selected_count(), 0);
        assert_eq!(
            session.selection.track_gain(1),
            Some(TrackGain::from_tenths_db(-55))
        );
        assert_eq!(session.selection.container(), OutputContainer::Mkv);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        assert!(session.selection.normalization_enabled());
        assert_eq!(
            session.selection.normalization_targets(),
            normalization_targets
        );
        assert_eq!(
            session.selection.request(&session.media).normalization,
            LoudnessNormalization::Enabled(normalization_targets)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upscale_state_survives_operation_switching() {
        let (directory, path) = temp_file("upscale-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        let session = app
            .state
            .upscale
            .as_mut()
            .expect("upscale session should exist");
        session.selection.select_custom_target(&session.media);
        session
            .selection
            .set_custom_boundary(&session.media, 3000, 2000);
        session.selection.set_codec(VideoCodec::H265);
        let custom_output = directory.join("custom-upscale.mp4");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::VideoUpscale);

        let session = app
            .state
            .upscale
            .as_ref()
            .expect("same-source Upscale session should be restored");
        assert_workspace(&app.state, Tool::VideoUpscale);
        assert_eq!(
            session.selection.target(),
            UpscaleTarget::Custom {
                max_width: 3000,
                max_height: 2000,
            }
        );
        assert_eq!(session.selection.custom_boundary(), Some((3000, 2000)));
        assert_eq!(session.selection.codec(), VideoCodec::H265);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remux_state_survives_operation_switching() {
        let (directory, path) = temp_file("remux-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::Remux);
        let session = app
            .state
            .remux
            .as_mut()
            .expect("remux session should exist");
        session.selection.set_container(MediaContainer::Mp4);
        let custom_output = directory.join("custom-remux.mp4");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::Remux);

        let session = app
            .state
            .remux
            .as_ref()
            .expect("same-source Remux session should be restored");
        assert_workspace(&app.state, Tool::Remux);
        assert_eq!(session.selection.container(), MediaContainer::Mp4);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_extract_state_survives_operation_switching() {
        let (directory, path) = temp_file("audio-extract-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::AudioExtract);
        let session = app
            .state
            .audio_extract
            .as_mut()
            .expect("audio extract session should exist");
        session.selection.set_format(AudioExtractFormat::Flac);
        let custom_output = directory.join("dialogue.flac");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::AudioExtract);
        let session = app
            .state
            .audio_extract
            .as_ref()
            .expect("same-source Audio Extract session should be restored");
        assert_workspace(&app.state, Tool::AudioExtract);
        assert_eq!(session.selection.selected_stream_index(), Some(1));
        assert_eq!(session.selection.format(), AudioExtractFormat::Flac);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn frame_capture_state_survives_operation_switching() {
        let (directory, path) = temp_file("frame-capture-back");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        app.select_tool(Tool::FrameCapture);
        let session = app
            .state
            .frame_capture
            .as_mut()
            .expect("frame capture session should exist");
        let inspected_media = media(&path);
        session
            .selection
            .set_timestamp_text(&inspected_media, "00:00:03.250".into());
        session.selection.set_format(FrameCaptureFormat::Jpeg);
        let custom_output = directory.join("chosen-frame.jpg");
        session
            .selection
            .set_custom_output_path(custom_output.clone());

        app.select_tool(Tool::AudioMix);
        app.select_tool(Tool::FrameCapture);
        let session = app
            .state
            .frame_capture
            .as_ref()
            .expect("same-source Frame Capture session should be restored");
        assert_workspace(&app.state, Tool::FrameCapture);
        assert_eq!(session.selection.timestamp_text(), "00:00:03.250");
        assert_eq!(session.selection.format(), FrameCaptureFormat::Jpeg);
        assert_eq!(session.selection.output_path(), custom_output);
        assert!(session.selection.output_is_customized());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replacement_source_retires_preserved_ready_sessions() {
        let (directory, first) = temp_file("replace-after-back");
        let second = directory.join("replacement.mkv");
        fs::write(&second, b"replacement").unwrap();
        let mut app = app();
        app.load_media_path_with_probe(&first, |path| Ok(media(path)));
        app.select_tool(Tool::VideoUpscale);
        app.select_tool(Tool::AudioMix);
        assert!(app.state.audio_mix.is_some());
        let old_audio_mix = app.state.audio_mix.as_mut().unwrap();
        old_audio_mix.selection.set_normalization_enabled(true);
        assert!(
            old_audio_mix
                .selection
                .set_normalization_targets(LoudnessTargets::from_tenths(-220, -9, 135))
        );
        app.select_tool(Tool::VideoUpscale);
        let old_output = directory.join("old-custom.mp4");
        app.state
            .upscale
            .as_mut()
            .expect("upscale session should exist")
            .selection
            .set_custom_output_path(old_output);
        app.load_media_path_with_probe(&second, |path| Ok(media(path)));

        assert!(app.state.audio_mix.is_none());
        let session = app
            .state
            .upscale
            .as_ref()
            .expect("replacement should create a new Upscale session");
        assert_workspace(&app.state, Tool::VideoUpscale);
        assert_eq!(session.media.path, second);
        assert!(!session.selection.output_is_customized());
        assert_eq!(session.selection.target(), UpscaleTarget::Uhd4k);
        assert_eq!(session.selection.custom_boundary(), None);
        assert_eq!(session.selection.codec(), VideoCodec::H264);
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&second)
        );

        app.select_tool(Tool::AudioMix);
        let audio_mix = app
            .state
            .audio_mix
            .as_ref()
            .expect("replacement should create a new Audio Mix session");
        assert!(
            audio_mix
                .selection
                .request(&audio_mix.media)
                .selected_tracks
                .iter()
                .all(|track| track.gain == TrackGain::ZERO)
        );
        assert!(!audio_mix.selection.normalization_enabled());
        assert_eq!(
            audio_mix.selection.normalization_targets(),
            LoudnessTargets::default()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switching_operations_does_not_revalidate_or_clear_the_source() {
        let (directory, path) = temp_file("changed-switch");
        let mut app = app();
        app.load_media_path_with_probe(&path, |path| Ok(media(path)));
        fs::write(&path, b"changed-size").unwrap();

        app.select_tool(Tool::VideoUpscale);

        assert_workspace(&app.state, Tool::VideoUpscale);
        assert!(app.state.upscale.is_some());
        assert_eq!(
            app.state.media_intake.current().map(|media| &media.path),
            Some(&path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lifecycle_routing_remains_operation_specific() {
        assert!(!shows_audio_mix_lifecycle(&AudioMixExecutionState::Idle));
        assert!(!shows_upscale_lifecycle(&UpscaleExecutionState::Idle));
        assert!(!shows_remux_lifecycle(&RemuxExecutionState::Idle));
    }

    #[test]
    fn file_drop_policy_disables_replacement_while_any_operation_runs() {
        assert!(file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Running,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Running,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Running,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Running,
            &FrameCaptureExecutionState::Idle,
        ));
        assert!(!file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Running,
        ));
    }

    #[test]
    fn drag_hover_accepts_single_and_multiple_files_when_intake_is_available() {
        let (first_directory, first) = temp_file("hover-first");
        let (second_directory, second) = temp_file("hover-second");
        let mut app = app();

        assert!(drag_hovered(&app, &[&first]));
        assert!(drag_hovered(&app, &[&first, &second]));

        let _sender = begin_pending_inspection(&mut app, &first);
        assert!(!drag_hovered(&app, &[&second]));

        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(second_directory).unwrap();
    }

    #[test]
    fn audio_mix_terminal_states_reject_source_replacement() {
        for state in terminal_execution_states() {
            assert!(!file_intake_enabled(
                &state,
                &UpscaleExecutionState::Idle,
                &RemuxExecutionState::Idle,
                &AudioExtractExecutionState::Idle,
                &FrameCaptureExecutionState::Idle,
            ));
        }
    }

    #[test]
    fn upscale_terminal_states_reject_source_replacement() {
        for state in terminal_execution_states() {
            assert!(!file_intake_enabled(
                &AudioMixExecutionState::Idle,
                &state,
                &RemuxExecutionState::Idle,
                &AudioExtractExecutionState::Idle,
                &FrameCaptureExecutionState::Idle,
            ));
        }
    }

    #[test]
    fn terminal_reset_to_idle_reenables_file_intake() {
        let completed = AudioMixExecutionState::Completed {
            output: PathBuf::from("finished.mp4"),
        };
        assert!(!file_intake_enabled(
            &completed,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));

        assert!(file_intake_enabled(
            &AudioMixExecutionState::Idle,
            &UpscaleExecutionState::Idle,
            &RemuxExecutionState::Idle,
            &AudioExtractExecutionState::Idle,
            &FrameCaptureExecutionState::Idle,
        ));
    }
}
