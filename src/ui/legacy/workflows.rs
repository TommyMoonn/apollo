//! Existing feature actions and worker coordination. Extracted incrementally by their owning phases.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use egui::{ColorImage, Context, TextureOptions};

use crate::app::{NavigationState, Operation, ProcessExecutionArbitration, QueueEditOrigin};
use crate::audio_extract::execution::{AudioExtractExecutionState, AudioExtractStartError};
use crate::audio_extract::model::AudioExtractFormat;
use crate::audio_mix::execution::{AudioMixExecutionState, AudioMixStartError};
use crate::audio_mix::model::{LoudnessNormalization, OutputContainer};
use crate::file_dialog::{
    DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogRequest, FileDialogStartError,
    OutputFormat, SaveDialogOptions,
};
use crate::frame_capture::clipboard::{CopyFrameError, CopyFrameRequest, CopyFrameWorker};
use crate::frame_capture::execution::{FrameCaptureExecutionState, FrameCaptureStartError};
use crate::frame_capture::frame_index::{
    FrameDirection, FrameIndexError, FrameIndexRequest, FrameIndexState, FrameNavigation,
};
use crate::frame_capture::model::{FrameCaptureFormat, FramePosition};
use crate::frame_capture::preview::{
    PREVIEW_DEBOUNCE, PreviewExtractionError, PreviewImage, PreviewRequest, PreviewState,
};
use crate::media::batch::BatchInspectionOwner;
use crate::media::container::MediaContainer;
use crate::media::error::MediaError;
use crate::media::intake::{MediaIntakeRequest, MediaLoadStart, MediaProbeDisposition};
use crate::media::model::MediaFile;
use crate::media::probe::MediaProbeError;
use crate::media::process::{MediaProcessEvent, MediaProcessWake};
use crate::media::tools::{MediaTools, VideoEncoderCapabilities};
use crate::queue::{
    JobId, QueueEnqueueError, QueueJobState, QueueRunState, QueuedJob, QueuedOperation,
    WaitingJobMove,
};
use crate::remux::execution::{RemuxExecutionState, RemuxStartError};
use crate::ui::audio_extract::{AudioExtractSelectionAction, AudioExtractSelectionState};
use crate::ui::audio_extract_progress::{
    AudioExtractLifecycleAction, AudioExtractLifecycleUiState,
};
use crate::ui::audio_mix::{AudioMixSelectionAction, AudioMixSelectionState};
use crate::ui::audio_mix_progress::{AudioMixLifecycleAction, AudioMixLifecycleUiState};
use crate::ui::batch::BatchReadiness;
use crate::ui::file_intake::FileIntakeAction;
use crate::ui::frame_capture::{
    FrameCaptureSelectionAction, FrameCaptureSelectionState, FrameCopyStatus, PositionSettle,
};
use crate::ui::frame_capture_progress::{
    FrameCaptureLifecycleAction, FrameCaptureLifecycleUiState, FrameCaptureThumbnailRequest,
};
use crate::ui::remux::{RemuxSelectionAction, RemuxSelectionState};
use crate::ui::remux_progress::{RemuxLifecycleAction, RemuxLifecycleUiState};
use crate::ui::source_presentation::SourcePresentation;
use crate::ui::upscale::{UpscaleSelectionAction, UpscaleSelectionState};
use crate::ui::upscale_progress::{UpscaleLifecycleAction, UpscaleLifecycleUiState};
use crate::ui::workspace::WorkspaceAction;
use crate::ui::{QueueAction, SourceAction};
use crate::upscale::execution::{UpscaleExecutionState, UpscaleStartError};
use crate::upscale::model::VideoCodec;
use crate::upscale::validation::validate_upscale_request;

use super::state::{
    AudioExtractToolState, AudioMixToolState, BatchDefaults, BatchEditState, BatchEditTarget,
    BatchInspectionSession, BatchItem, BatchSetupState, BatchSourceState, FrameCaptureEditorState,
    FrameCaptureThumbnailLoad, FrameCaptureToolState, FramePreviewDecode, FramePreviewTexture,
    LegacyEditorState, MediaInspection, OperationEditorState, PendingFrameStep, QueueEditState,
    QueueExecutionEvent, RemuxToolState, UpscaleToolState,
};
use super::{ApolloApp, completion_wake};

impl ApolloApp {
    pub(super) fn resolve_navigation(&mut self) -> Option<NavigationState> {
        match self.core.navigation() {
            NavigationState::BatchEdit if self.batch_edit.is_none() => {
                self.core.show_batch_setup();
                None
            }
            NavigationState::BatchSetup if self.batch_setup.is_none() => {
                self.core.show_no_source();
                None
            }
            NavigationState::QueueEdit { .. } if self.queue_edit.is_none() => {
                self.finish_queue_edit();
                None
            }
            NavigationState::SourceWorkspace { .. }
                if self.core.media_intake.current().is_none()
                    || self.state.source_presentation.is_none() =>
            {
                self.close_source();
                Some(self.core.navigation())
            }
            route => Some(route),
        }
    }

    fn enter_workspace(&mut self, operation: Operation) {
        self.core.enter_workspace(operation);
        self.state.intake_error_details_open = false;
    }

    pub(super) fn enter_queue(&mut self) {
        self.core.enter_queue();
        self.state.intake_error_details_open = false;
    }

    pub(super) fn ready_media_tools(&self) -> Option<&MediaTools> {
        self.core.discovery.ready()
    }

    pub(super) fn copy_frame_active(&self) -> bool {
        self.copy_frame_extraction.is_active() || self.copy_frame_worker.is_some()
    }

    pub(super) fn execution_arbitration(&self) -> ProcessExecutionArbitration {
        self.core.execution_arbitration(self.copy_frame_active())
    }

    pub(super) fn start_media_queue(&mut self) -> bool {
        if !self.execution_arbitration().queue_start_allowed() {
            return false;
        }
        let Some(tools) = self.ready_media_tools().cloned() else {
            return false;
        };
        let sender = self.queue_event_sender.clone();
        let progress_repaint = &mut self.progress_repaint;
        self.core.media_queue.start(&tools, |job_id| {
            queue_execution_wake(job_id, sender.clone(), progress_repaint.begin())
        })
    }

    pub(super) fn poll_media_queue(&mut self) {
        let mut current_event_received = false;
        while let Ok(event) = self.queue_event_receiver.try_recv() {
            if queue_event_matches_active(self.core.media_queue.active_job(), event) {
                current_event_received = match event.process_event {
                    MediaProcessEvent::Progress | MediaProcessEvent::Terminal => true,
                };
            }
        }
        if !current_event_received {
            return;
        }

        let Some(tools) = self.ready_media_tools().cloned() else {
            self.core.media_queue.cancel_queue();
            return;
        };
        let sender = self.queue_event_sender.clone();
        let progress_repaint = &mut self.progress_repaint;
        self.core.media_queue.poll(&tools, |job_id| {
            queue_execution_wake(job_id, sender.clone(), progress_repaint.begin())
        });
    }

    pub(super) fn handle_queue_action(&mut self, action: QueueAction) -> bool {
        match action {
            QueueAction::AddFiles => {
                self.core.media_queue.clear_disposable_history();
                self.start_file_dialog(FileDialogRequest::OpenSources {
                    initial_directory: None,
                    target: DialogTarget::BatchInput,
                });
                true
            }
            QueueAction::Start => self.start_media_queue(),
            QueueAction::CancelCurrent => self.core.media_queue.cancel_current(),
            QueueAction::StopAfterCurrent => self.core.media_queue.stop_after_current(),
            QueueAction::CancelQueue => self.core.media_queue.cancel_queue(),
            QueueAction::Edit(job_id) => {
                let Some(origin) = self.core.queue_origin() else {
                    return false;
                };
                self.begin_queue_edit(job_id, QueueEditOrigin::Queue(origin))
            }
            QueueAction::MoveEarlier(job_id) => self
                .core
                .media_queue
                .move_waiting(job_id, WaitingJobMove::Earlier)
                .is_ok(),
            QueueAction::MoveLater(job_id) => self
                .core
                .media_queue
                .move_waiting(job_id, WaitingJobMove::Later)
                .is_ok(),
            QueueAction::Remove(job_id) => self.core.media_queue.remove_waiting(job_id),
            QueueAction::ClearFinished => self.core.media_queue.clear_finished() > 0,
            QueueAction::OpenFolder(job_id) => {
                let directory = self
                    .core
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
                if self.core.queue_origin().is_none() {
                    return false;
                }
                self.core.media_queue.clear_disposable_history();
                self.core.return_from_queue();
                true
            }
            QueueAction::None => false,
        }
    }

    pub(super) fn begin_queue_edit(&mut self, job_id: JobId, origin: QueueEditOrigin) -> bool {
        if self.core.media_queue.run_state() != QueueRunState::Stopped {
            return false;
        }
        if !self.core.can_enter_queue_edit(origin) {
            return false;
        }
        let Some(job) = self
            .core
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
        self.core.enter_queue_edit(origin);
        self.schedule_active_frame_capture_support();
        true
    }

    pub(super) fn finish_queue_edit(&mut self) -> bool {
        let NavigationState::QueueEdit { origin } = self.core.navigation() else {
            return false;
        };
        self.cancel_frame_capture_support();
        match origin {
            QueueEditOrigin::Queue(origin) => {
                self.queue_edit = None;
                self.core.show_queue(origin);
                true
            }
            QueueEditOrigin::SourceWorkspace => {
                let Some(job) = self.queue_edit.as_ref().and_then(|edit| {
                    self.core
                        .media_queue
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

    pub(super) fn select_queue_edit_tool(&mut self, operation: Operation) -> bool {
        if !matches!(self.core.navigation(), NavigationState::QueueEdit { .. }) {
            return false;
        }
        let Some(previous) = self.queue_edit.as_ref().map(|edit| edit.active_operation) else {
            return false;
        };
        if previous == operation {
            return true;
        }
        if previous == Operation::FrameCapture {
            self.cancel_frame_capture_support();
        }
        let Some(edit) = self.queue_edit.as_mut() else {
            return false;
        };
        edit.active_operation = operation;
        edit.error = None;
        edit.error_details_open = false;
        if operation == Operation::FrameCapture {
            self.schedule_active_frame_capture_support();
        }
        true
    }

    fn restore_queued_job_workspace(&mut self, job: &QueuedJob) -> bool {
        let operation = queued_operation_tool(job.operation());
        self.core.media_intake = job.retained_intake();
        self.state.source_presentation = Some(Arc::new(SourcePresentation::new(job.media())));
        self.frame_capture_thumbnail_load = None;
        self.cancel_frame_capture_support();
        self.state.retire_tool_sessions();
        if !self.activate_cached_source(operation) {
            return false;
        }
        self.hydrate_cached_source_operation(job.operation());
        self.enter_workspace(operation);
        if operation == Operation::FrameCapture {
            self.schedule_active_frame_capture_support();
        }
        self.core.queue_confirmation = None;
        self.core.operation_action_error = None;
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
                    let position = session.selection.valid_position(&session.media);
                    self.update_frame_capture_position(
                        position,
                        PositionSettle::Debounced,
                        Instant::now(),
                    );
                }
            }
        }
    }

    pub(super) fn save_queue_edit(&mut self) -> bool {
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
            .core
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

    pub(super) fn enqueue_current_operation(
        &mut self,
        operation: Operation,
    ) -> Result<Option<JobId>, QueueEnqueueError> {
        let queued = match operation {
            Operation::AudioMix => self.state.audio_mix.as_mut().and_then(|session| {
                session.selection.can_mix(&session.media).then(|| {
                    (
                        session.media.clone(),
                        QueuedOperation::AudioMix(session.selection.request(&session.media)),
                    )
                })
            }),
            Operation::VideoUpscale => {
                let Some(session) = self.state.upscale.as_ref() else {
                    return Ok(None);
                };
                if self.core.discovery.is_pending()
                    && session.selection.encoder_selection()
                        != crate::upscale::model::VideoEncoderSelection::Cpu
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
            Operation::Remux => self.state.remux.as_ref().and_then(|session| {
                session.selection.can_remux(&session.media).then(|| {
                    (
                        session.media.clone(),
                        QueuedOperation::Remux(session.selection.request(&session.media)),
                    )
                })
            }),
            Operation::AudioExtract => self.state.audio_extract.as_ref().and_then(|session| {
                session.selection.can_extract(&session.media).then(|| {
                    session.selection.request(&session.media).map(|request| {
                        (
                            session.media.clone(),
                            QueuedOperation::AudioExtract(request),
                        )
                    })
                })?
            }),
            Operation::FrameCapture => self.state.frame_capture.as_ref().and_then(|session| {
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
        self.core.media_queue.enqueue(&media, request).map(Some)
    }

    pub(super) fn enqueue_from_ready_surface(&mut self, operation: Operation) {
        self.core.queue_confirmation = None;
        self.core.operation_action_error = None;
        match self.enqueue_current_operation(operation) {
            Ok(Some(_)) => self.core.queue_confirmation = Some(operation),
            Ok(None) => {}
            Err(error) => {
                self.core.operation_action_error =
                    Some((operation, media_error_from_queue_enqueue(&error)));
            }
        }
    }

    pub(super) fn file_intake_enabled(&self) -> bool {
        self.media_inspection.is_none()
            && !self.file_dialog.is_pending()
            && self.core.executors_idle()
            && !self.copy_frame_active()
    }

    pub(super) fn workspace_interaction_enabled(&self) -> bool {
        self.media_inspection.is_none() && !self.copy_frame_active()
    }

    pub(super) fn handle_workspace_action(&mut self, action: WorkspaceAction) {
        match action {
            WorkspaceAction::Source(action) => self.handle_source_action(action),
            WorkspaceAction::SelectOperation(operation) => self.select_tool(operation),
            WorkspaceAction::EditQueuedJob(job_id) => {
                self.begin_queue_edit(job_id, QueueEditOrigin::SourceWorkspace);
            }
            WorkspaceAction::OpenQueue => self.enter_queue(),
            WorkspaceAction::None => {}
        }
    }

    pub(super) fn handle_source_action(&mut self, action: SourceAction) {
        if !self.file_intake_enabled() {
            return;
        }
        self.core.queue_confirmation = None;
        self.core.operation_action_error = None;
        match action {
            SourceAction::ChangeFile => self.start_file_dialog(FileDialogRequest::OpenSource {
                initial_directory: None,
            }),
            SourceAction::CloseSource => self.close_source(),
        }
    }

    pub(super) fn close_source(&mut self) {
        self.frame_capture_thumbnail_load = None;
        self.cancel_frame_capture_support();
        self.core.close_source();
        self.state.source_presentation = None;
        self.state.retire_tool_sessions();
        self.state.intake_error_details_open = false;
    }

    pub(super) fn select_tool(&mut self, tool: Operation) {
        if !self.file_intake_enabled() {
            return;
        }
        let Some(previous_tool) = self.core.active_operation() else {
            return;
        };
        if previous_tool != tool {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
            if previous_tool == Operation::FrameCapture
                && let Some(session) = self.state.frame_capture.as_mut()
            {
                session.preview.invalidate();
                session.preview_texture = None;
                session.copy_status = FrameCopyStatus::Idle;
            }
            if previous_tool == Operation::FrameCapture {
                self.cancel_frame_capture_support();
            }
        }
        if self.activate_cached_source(tool) {
            self.core.show_workspace(tool);
            if tool == Operation::FrameCapture {
                self.schedule_active_frame_capture_support();
            }
        }
    }

    fn activate_cached_source(&mut self, tool: Operation) -> bool {
        let Some((source_identity, media)) = self
            .core
            .media_intake
            .current_with_identity()
            .map(|(identity, media)| (identity.clone(), media.clone()))
        else {
            self.core.show_no_source();
            self.cancel_frame_capture_support();
            self.state.retire_tool_sessions();
            return false;
        };
        self.state
            .source_presentation
            .get_or_insert_with(|| Arc::new(SourcePresentation::new(&media)));

        match tool {
            Operation::AudioMix => {
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
            Operation::VideoUpscale => {
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
            Operation::Remux => {
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
            Operation::AudioExtract => {
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
            Operation::FrameCapture => {
                if !self.state.frame_capture.as_ref().is_some_and(|session| {
                    session.source_identity.as_ref() == Some(&source_identity)
                }) {
                    let session_id = self.take_session_id();
                    self.state.frame_capture = Some(FrameCaptureToolState {
                        session_id,
                        editor: FrameCaptureEditorState::new(
                            session_id,
                            Some(source_identity),
                            &media,
                        ),
                        lifecycle_ui: FrameCaptureLifecycleUiState::default(),
                        start_error: None,
                        start_error_details_open: false,
                        copy_status: FrameCopyStatus::Idle,
                        media,
                    });
                }
                let position = self
                    .state
                    .frame_capture
                    .as_ref()
                    .and_then(|session| session.selection.valid_position(&session.media));
                if self.state.frame_capture.as_ref().is_some_and(|session| {
                    matches!(
                        session.preview.state(),
                        crate::frame_capture::preview::PreviewState::Idle
                    )
                }) {
                    self.update_frame_capture_position(
                        position,
                        PositionSettle::Debounced,
                        Instant::now(),
                    );
                }
            }
        }
        true
    }

    fn active_frame_capture_editor(&self) -> Option<(&MediaFile, &FrameCaptureEditorState)> {
        match self.core.navigation() {
            NavigationState::SourceWorkspace {
                active_operation: Operation::FrameCapture,
            } => self
                .state
                .frame_capture
                .as_ref()
                .map(|session| (&session.media, &session.editor)),
            NavigationState::QueueEdit { .. } => self
                .queue_edit
                .as_ref()
                .filter(|edit| edit.active_operation == Operation::FrameCapture)
                .map(|edit| (&edit.media, &edit.frame_capture)),
            NavigationState::BatchEdit => self
                .batch_edit
                .as_ref()
                .filter(|edit| {
                    matches!(edit.target, BatchEditTarget::Item(_))
                        && edit.active_operation == Operation::FrameCapture
                })
                .map(|edit| (&edit.media, &edit.frame_capture)),
            _ => None,
        }
    }

    pub(super) fn active_frame_capture_editor_mut(
        &mut self,
    ) -> Option<(&MediaFile, &mut FrameCaptureEditorState)> {
        match self.core.navigation() {
            NavigationState::SourceWorkspace {
                active_operation: Operation::FrameCapture,
            } => self
                .state
                .frame_capture
                .as_mut()
                .map(|session| (&session.media, &mut session.editor)),
            NavigationState::QueueEdit { .. } => self
                .queue_edit
                .as_mut()
                .filter(|edit| edit.active_operation == Operation::FrameCapture)
                .map(|edit| {
                    let OperationEditorState {
                        media,
                        frame_capture,
                        ..
                    } = &mut edit.editor;
                    (&*media, frame_capture)
                }),
            NavigationState::BatchEdit => self
                .batch_edit
                .as_mut()
                .filter(|edit| {
                    matches!(edit.target, BatchEditTarget::Item(_))
                        && edit.active_operation == Operation::FrameCapture
                })
                .map(|edit| {
                    let OperationEditorState {
                        media,
                        frame_capture,
                        ..
                    } = &mut edit.editor;
                    (&*media, frame_capture)
                }),
            _ => None,
        }
    }

    fn schedule_active_frame_capture_support(&mut self) {
        let scheduled = self
            .active_frame_capture_editor()
            .and_then(|(media, editor)| {
                editor.source_identity.as_ref()?;
                matches!(editor.preview.state(), PreviewState::Idle)
                    .then(|| editor.selection.valid_position(media))
            })
            .flatten();
        if scheduled.is_some() {
            self.update_frame_capture_position(
                scheduled,
                PositionSettle::Debounced,
                Instant::now(),
            );
        }
    }

    pub(super) fn update_frame_capture_position(
        &mut self,
        position: Option<FramePosition>,
        settle: PositionSettle,
        now: Instant,
    ) {
        self.frame_index_query.cancel();
        let Some((_, editor)) = self.active_frame_capture_editor_mut() else {
            return;
        };
        if editor.source_identity.is_none() {
            return;
        }
        editor.pending_step = None;
        editor.frame_navigator.invalidate();
        editor.frame_index_due = None;
        let ready_at = self.schedule_frame_capture_preview(position, settle, now);
        if let (Some(position), Some(ready_at), Some((_, editor))) =
            (position, ready_at, self.active_frame_capture_editor_mut())
            && editor.source_identity.is_some()
        {
            editor.frame_index_due = Some((position, ready_at));
        }
    }

    fn schedule_frame_capture_preview(
        &mut self,
        position: Option<FramePosition>,
        settle: PositionSettle,
        now: Instant,
    ) -> Option<Instant> {
        self.cancel_frame_preview_work();
        let (media, editor) = self.active_frame_capture_editor_mut()?;
        let source_identity = editor.source_identity.clone()?;
        let Some(position) = position else {
            editor.preview.invalidate();
            return None;
        };
        let delay = match settle {
            PositionSettle::Debounced => PREVIEW_DEBOUNCE,
            PositionSettle::Immediate => std::time::Duration::ZERO,
        };
        let scheduled = match settle {
            PositionSettle::Debounced => {
                editor
                    .preview
                    .schedule_position(&source_identity, media, position, now)
            }
            PositionSettle::Immediate => {
                editor
                    .preview
                    .schedule_position_immediately(&source_identity, media, position, now)
            }
        };
        if scheduled.is_some() {
            if delay.is_zero() {
                self.egui_context.request_repaint();
            } else {
                self.egui_context.request_repaint_after(delay);
            }
        }
        scheduled.map(|_| now + delay)
    }

    pub(super) fn poll_frame_capture_preview(&mut self) {
        let due_request = self
            .active_frame_capture_editor_mut()
            .and_then(|(_, editor)| editor.preview.take_due_request(Instant::now()));
        if let Some(request) = due_request {
            self.start_frame_preview_extraction(request);
        } else if self
            .active_frame_capture_editor()
            .is_some_and(|(_, editor)| matches!(editor.preview.state(), PreviewState::Ready { .. }))
        {
            self.cancel_frame_preview_work();
        }

        self.poll_frame_preview_extraction();
        self.poll_frame_preview_decode();
        self.sync_frame_preview_texture();
    }

    pub(super) fn poll_frame_capture_index(&mut self) {
        let now = Instant::now();
        let due = self
            .active_frame_capture_editor_mut()
            .and_then(|(_, editor)| {
                editor
                    .frame_index_due
                    .filter(|(_, ready_at)| now >= *ready_at)?;
                editor.frame_index_due.take().map(|(position, _)| position)
            });
        if let Some(position) = due {
            self.start_frame_index_query(position);
        }

        let Some(completion) = self.frame_index_query.poll() else {
            return;
        };
        self.settle_frame_index_request(completion.request, completion.result);
    }

    fn start_frame_index_query(&mut self, position: FramePosition) {
        let Some(tools) = self.ready_media_tools().cloned() else {
            return;
        };
        let request = {
            let Some((media, editor)) = self.active_frame_capture_editor_mut() else {
                return;
            };
            let Some(source_identity) = editor.source_identity.clone() else {
                return;
            };
            match editor
                .frame_navigator
                .schedule(&source_identity, media, position)
            {
                Ok(Some(request)) => request,
                Ok(None) => {
                    self.resolve_pending_frame_step();
                    return;
                }
                Err(_) => return,
            }
        };
        let context = self.egui_context.clone();
        let wake = MediaProcessWake::new(move |_| context.request_repaint());
        if let Err(error) = self.frame_index_query.start(&tools, request.clone(), wake) {
            self.settle_frame_index_request(request, Err(error));
        }
    }

    pub(super) fn settle_frame_index_request(
        &mut self,
        request: FrameIndexRequest,
        result: Result<Vec<u8>, FrameIndexError>,
    ) {
        let ready = self
            .active_frame_capture_editor_mut()
            .is_some_and(|(_, editor)| {
                if !editor.frame_navigator.settle(request, result) {
                    return false;
                }
                if matches!(
                    editor.frame_navigator.state(),
                    FrameIndexState::Ready { .. }
                ) {
                    true
                } else {
                    editor.pending_step = None;
                    false
                }
            });
        if ready {
            self.resolve_pending_frame_step();
        }
    }

    pub(super) fn request_frame_step(&mut self, direction: FrameDirection) {
        let outcome = {
            let Some((media, editor)) = self.active_frame_capture_editor_mut() else {
                return;
            };
            let Some(origin) = editor.selection.valid_position(media) else {
                return;
            };
            if editor.source_identity.is_none() {
                return;
            }
            let step = editor.selection.step();
            let pending = PendingFrameStep {
                origin,
                direction,
                step,
            };
            editor.pending_step = Some(pending);
            (
                pending,
                editor
                    .frame_navigator
                    .navigate(origin, direction, pending.step),
            )
        };
        self.apply_frame_step_outcome(outcome.0, outcome.1);
    }

    fn resolve_pending_frame_step(&mut self) {
        let outcome = self.active_frame_capture_editor().and_then(|(_, editor)| {
            let pending = editor.pending_step?;
            Some((
                pending,
                editor
                    .frame_navigator
                    .navigate(pending.origin, pending.direction, pending.step),
            ))
        });
        if let Some((pending, outcome)) = outcome {
            self.apply_frame_step_outcome(pending, outcome);
        }
    }

    fn apply_frame_step_outcome(&mut self, pending: PendingFrameStep, outcome: FrameNavigation) {
        match outcome {
            FrameNavigation::Resolved(position) => {
                if let Some((media, editor)) = self.active_frame_capture_editor_mut() {
                    editor.pending_step = None;
                    editor.selection.set_position(media, position.into());
                }
                self.schedule_frame_capture_preview(
                    Some(position.into()),
                    PositionSettle::Immediate,
                    Instant::now(),
                );
            }
            FrameNavigation::NeedsNeighborhood(anchor) => {
                self.start_frame_index_query(anchor);
            }
            FrameNavigation::Boundary => {
                if let Some((_, editor)) = self.active_frame_capture_editor_mut()
                    && editor.pending_step == Some(pending)
                {
                    editor.pending_step = None;
                }
            }
        }
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
            .active_frame_capture_editor()
            .is_some_and(|(_, editor)| editor.preview.is_loading(&completion.request));
        if !authoritative {
            return;
        }

        match completion.result {
            Ok(bytes) => {
                let request = completion.request;
                match FramePreviewDecode::spawn(
                    request.clone(),
                    bytes,
                    completion_wake(&self.egui_context),
                ) {
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

    pub(super) fn poll_frame_preview_decode(&mut self) {
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

    pub(super) fn settle_frame_preview(
        &mut self,
        request: PreviewRequest,
        result: Result<PreviewImage, PreviewExtractionError>,
    ) -> bool {
        self.active_frame_capture_editor_mut()
            .is_some_and(|(_, editor)| editor.preview.settle(request, result))
    }

    pub(super) fn sync_frame_preview_texture(&mut self) {
        let context = self.egui_context.clone();
        let Some((_, editor)) = self.active_frame_capture_editor_mut() else {
            return;
        };
        let PreviewState::Ready { request, image } = editor.preview.state() else {
            return;
        };
        if editor
            .preview_texture
            .as_ref()
            .is_some_and(|texture| texture.generation == request.generation)
        {
            return;
        }
        let size = image.size();
        let color = ColorImage::from_rgba_unmultiplied(size, image.rgba());
        let handle = context.load_texture(
            format!(
                "frame-capture-preview-{}-{}",
                editor.session_id,
                request.generation.get()
            ),
            color,
            TextureOptions::LINEAR,
        );
        editor.preview_texture = Some(FramePreviewTexture {
            generation: request.generation,
            size,
            position: request.position,
            handle,
        });
    }

    fn cancel_frame_preview_work(&mut self) {
        self.frame_preview_extraction.cancel();
        self.frame_preview_decode = None;
    }

    fn cancel_frame_capture_support(&mut self) {
        self.cancel_frame_preview_work();
        self.frame_index_query.cancel();
        if let Some((_, editor)) = self.active_frame_capture_editor_mut() {
            editor.preview.invalidate();
            editor.preview_texture = None;
            editor.frame_index_due = None;
            editor.pending_step = None;
            editor.frame_navigator.invalidate();
        }
    }

    fn start_copy_frame(&mut self) {
        if self.copy_frame_active()
            || self.core.media_operation_running()
            || !self.execution_arbitration().immediate_start_allowed()
        {
            return;
        }
        let request = self.state.frame_capture.as_ref().and_then(|session| {
            let source_identity = session.source_identity.as_ref()?;
            let position = session.selection.valid_position(&session.media)?;
            Some((
                session.session_id,
                CopyFrameRequest::new(source_identity, &session.media, position),
            ))
        });
        let Some((session_id, request)) = request else {
            return;
        };
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                self.settle_copy_frame(session_id, None, Err(error));
                return;
            }
        };
        let Some(tools) = self.ready_media_tools().cloned() else {
            return;
        };
        if let Some(session) = self.state.frame_capture.as_mut() {
            session.copy_status = FrameCopyStatus::Copying;
        }
        self.copy_frame_session_id = Some(session_id);
        let context = self.egui_context.clone();
        let wake = MediaProcessWake::new(move |_| context.request_repaint());
        if let Err(error) = self
            .copy_frame_extraction
            .start(&tools, request.clone(), wake)
        {
            self.copy_frame_session_id = None;
            self.settle_copy_frame(session_id, Some(&request), Err(error));
        }
    }

    pub(super) fn poll_copy_frame(&mut self) {
        if let Some(result) = self
            .copy_frame_worker
            .as_ref()
            .and_then(CopyFrameWorker::poll)
        {
            let worker = self
                .copy_frame_worker
                .take()
                .expect("copy-frame worker exists while its result is settled");
            let session_id = self.copy_frame_session_id.take().unwrap_or_default();
            self.settle_copy_frame(session_id, Some(&worker.request), result);
        }

        let Some(completion) = self.copy_frame_extraction.poll() else {
            return;
        };
        let session_id = self.copy_frame_session_id.take().unwrap_or_default();
        if !self.copy_frame_request_is_current(session_id, &completion.request) {
            return;
        }
        match completion.result {
            Ok(bytes) => {
                match CopyFrameWorker::spawn(
                    completion.request.clone(),
                    bytes,
                    self.clipboard_owner,
                    completion_wake(&self.egui_context),
                ) {
                    Ok(worker) => {
                        self.copy_frame_worker = Some(worker);
                        self.copy_frame_session_id = Some(session_id);
                    }
                    Err(error) => {
                        self.settle_copy_frame(session_id, Some(&completion.request), Err(error))
                    }
                }
            }
            Err(error) => {
                self.settle_copy_frame(session_id, Some(&completion.request), Err(error));
            }
        }
    }

    fn copy_frame_request_is_current(&self, session_id: u64, request: &CopyFrameRequest) -> bool {
        self.state.frame_capture.as_ref().is_some_and(|session| {
            session.session_id == session_id
                && session.source_identity.as_ref() == Some(&request.source_identity)
        })
    }

    fn settle_copy_frame(
        &mut self,
        session_id: u64,
        request: Option<&CopyFrameRequest>,
        result: Result<(), CopyFrameError>,
    ) {
        let current = self.state.frame_capture.as_ref().is_some_and(|session| {
            session.session_id == session_id
                && request.is_none_or(|request| {
                    session.source_identity.as_ref() == Some(&request.source_identity)
                })
        });
        if !current {
            return;
        }
        let session = self
            .state
            .frame_capture
            .as_mut()
            .expect("current copy-frame session remains available");
        match result {
            Ok(()) => {
                session.copy_status = FrameCopyStatus::Copied;
                self.core.operation_action_error = None;
            }
            Err(CopyFrameError::Cancelled) => {
                session.copy_status = FrameCopyStatus::Idle;
            }
            Err(error) => {
                session.copy_status = FrameCopyStatus::Idle;
                self.core.operation_action_error =
                    Some((Operation::FrameCapture, error.user_error()));
            }
        }
    }

    pub(super) fn cancel_copy_frame_work(&mut self) {
        self.copy_frame_extraction.cancel();
        self.copy_frame_worker = None;
        self.copy_frame_session_id = None;
        if let Some(session) = self.state.frame_capture.as_mut() {
            session.copy_status = FrameCopyStatus::Idle;
        }
    }

    fn take_session_id(&mut self) -> u64 {
        let session_id = self.next_session_id;
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .expect("Apollo tool session identifier space is exhausted");
        session_id
    }

    pub(super) fn retry_media_tools(&mut self) {
        if !self.core.discovery.is_pending() {
            self.tools_error_details_open = false;
            self.core
                .discovery
                .start(completion_wake(&self.egui_context));
        }
    }

    pub(super) fn handle_file_intake_action(&mut self, action: FileIntakeAction) {
        match action {
            FileIntakeAction::OpenFile => self.start_file_dialog(FileDialogRequest::OpenSources {
                initial_directory: None,
                target: DialogTarget::BatchInput,
            }),
            FileIntakeAction::OpenQueue => self.enter_queue(),
            FileIntakeAction::None => {}
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

        match MediaInspection::spawn(tools, request, completion_wake(&self.egui_context)) {
            Ok(inspection) => self.media_inspection = Some(inspection),
            Err(error) => {
                self.core.media_intake.retire_probe(&request_for_failure);
                self.core.intake_error = Some(MediaError::probe_failure(format!(
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
        let Some(origin) = self.core.batch_origin() else {
            return false;
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
        let inspection =
            match BatchInspectionOwner::spawn(tools, paths, completion_wake(&self.egui_context)) {
                Ok(inspection) => inspection,
                Err(error) => {
                    self.core.intake_error = Some(MediaError::probe_failure(format!(
                        "Batch inspection workers could not start: {error}"
                    )));
                    return false;
                }
            };
        self.batch_edit = None;
        self.batch_setup = Some(BatchSetupState {
            origin,
            operation: Operation::AudioMix,
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
        self.core.show_batch_setup();
        true
    }

    pub(super) fn poll_batch_inspection(&mut self) {
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
                Ok(inspected) => {
                    item.source = BatchSourceState::Ready {
                        source_identity: inspected.source_identity,
                        media: inspected.media,
                    };
                }
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
                BatchSourceState::Ready { media, .. } => Some(media.clone()),
                BatchSourceState::Inspecting | BatchSourceState::Failed => None,
            })
        {
            let editor = OperationEditorState::new(0, None, media, batch.operation);
            batch.defaults = batch_defaults_from_editor(&editor);
        }
        refresh_batch_items(batch, capabilities.as_ref());
        if complete {
            self.batch_inspection = None;
        }
    }

    pub(super) fn append_batch_paths(&mut self, paths: Vec<PathBuf>) -> bool {
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
        let owner = match BatchInspectionOwner::spawn(
            tools,
            unique.clone(),
            completion_wake(&self.egui_context),
        ) {
            Ok(owner) => owner,
            Err(error) => {
                if let Some(batch) = self.batch_setup.as_mut() {
                    batch.error = Some(format!("Additional files could not be inspected: {error}"));
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

    pub(super) fn begin_batch_edit(&mut self, target: BatchEditTarget) -> bool {
        let Some(batch) = self.batch_setup.as_ref() else {
            return false;
        };
        let operation = batch.operation;
        let defaults = batch.defaults.clone();
        let (source_identity, media, request) = match target {
            BatchEditTarget::Defaults => {
                let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                    BatchSourceState::Ready { media, .. } => Some(media.clone()),
                    BatchSourceState::Inspecting | BatchSourceState::Failed => None,
                }) else {
                    return false;
                };
                (None, media, None)
            }
            BatchEditTarget::Item(index) => {
                let Some(item) = batch.items.get(index) else {
                    return false;
                };
                let BatchSourceState::Ready {
                    source_identity,
                    media,
                } = &item.source
                else {
                    return false;
                };
                (
                    Some(source_identity.clone()),
                    media.clone(),
                    item.operation.clone(),
                )
            }
        };
        let session_id = self.take_session_id();
        let mut editor = OperationEditorState::new(session_id, source_identity, media, operation);
        if let Some(request) = request.as_ref() {
            editor.hydrate(request);
        } else if let Some(defaults) = defaults.as_ref() {
            apply_batch_defaults(&mut editor, defaults);
        }
        self.batch_edit = Some(BatchEditState { target, editor });
        self.core.show_batch_edit();
        self.schedule_active_frame_capture_support();
        true
    }

    pub(super) fn finish_batch_edit(&mut self) -> bool {
        if !matches!(self.core.navigation(), NavigationState::BatchEdit) {
            return false;
        }
        self.cancel_frame_capture_support();
        self.batch_edit = None;
        self.core.show_batch_setup();
        true
    }

    pub(super) fn save_batch_edit(&mut self) -> bool {
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

    pub(super) fn finish_batch_setup(&mut self) -> bool {
        let Some(batch) = self.batch_setup.take() else {
            return false;
        };
        self.batch_inspection = None;
        self.batch_edit = None;
        self.core.return_from_batch(batch.origin);
        true
    }

    pub(super) fn enqueue_batch(&mut self) -> bool {
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
            let BatchSourceState::Ready { media, .. } = &item.source else {
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
        match self.core.media_queue.enqueue_batch(&queued) {
            Ok(_) => {
                let origin = batch.origin;
                self.batch_setup = None;
                self.batch_edit = None;
                self.batch_inspection = None;
                self.core.complete_batch(origin);
                true
            }
            Err(error) => {
                let (batch_index, message) = match &error {
                    crate::queue::BatchEnqueueError::Item { index, error } => {
                        (source_indexes.get(*index).copied(), error.to_string())
                    }
                    crate::queue::BatchEnqueueError::DuplicateDestination { index, .. } => (
                        source_indexes.get(*index).copied(),
                        "Output duplicates another included file.".to_owned(),
                    ),
                    crate::queue::BatchEnqueueError::JobIdExhausted => (None, error.to_string()),
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

    pub(super) fn begin_media_path(&mut self, path: &Path) -> Option<MediaIntakeRequest> {
        if !self.file_intake_enabled() {
            return None;
        }

        self.core.queue_confirmation = None;
        self.core.operation_action_error = None;
        self.core.intake_error = None;
        self.state.intake_error_details_open = false;
        match self.core.media_intake.begin_load(path) {
            Ok(MediaLoadStart::Reused) => {
                let operation = self.core.active_operation().unwrap_or(Operation::AudioMix);
                if self.activate_cached_source(operation) {
                    self.enter_workspace(operation);
                }
                None
            }
            Ok(MediaLoadStart::Probe(request)) => Some(request),
            Err(error) => {
                self.core.intake_error = Some(MediaError::from_intake_error(&error));
                None
            }
        }
    }

    pub(super) fn poll_media_inspection(&mut self) {
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
                self.core.media_intake.retire_probe(&inspection.request);
                self.core.intake_error = Some(MediaError::probe_failure(
                    "Media inspection worker stopped before returning a result.",
                ));
                self.state.intake_error_details_open = false;
            }
        }
    }

    pub(super) fn poll_frame_capture_thumbnail(&mut self) {
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

    pub(super) fn complete_media_probe(
        &mut self,
        request: MediaIntakeRequest,
        result: Result<MediaFile, MediaProbeError>,
    ) {
        match self.core.media_intake.complete_probe(request, result) {
            Ok(MediaProbeDisposition::Published) => {
                let operation = self.core.active_operation().unwrap_or(Operation::AudioMix);
                self.state.source_presentation = self
                    .core
                    .media_intake
                    .current()
                    .map(|media| Arc::new(SourcePresentation::new(media)));
                self.frame_capture_thumbnail_load = None;
                self.cancel_frame_capture_support();
                self.state.retire_tool_sessions();
                if self.activate_cached_source(operation) {
                    self.enter_workspace(operation);
                }
            }
            Ok(MediaProbeDisposition::Retired) => {}
            Err(error) => {
                self.core.intake_error = Some(MediaError::from_intake_error(&error));
                self.state.intake_error_details_open = false;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn load_media_path_with_probe<F>(&mut self, path: &Path, probe: F)
    where
        F: FnOnce(&Path) -> Result<MediaFile, MediaProbeError>,
    {
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        let result = probe(request.path());
        self.complete_media_probe(request, result);
    }

    pub(super) fn handle_window_file_drop(&mut self, context: &Context) {
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

    pub(super) fn drag_hovered(&self, context: &Context) -> bool {
        self.file_intake_enabled() && context.input(|input| !input.raw.hovered_files.is_empty())
    }

    pub(super) fn handle_audio_mix_action(
        &mut self,
        action: AudioMixSelectionAction,
    ) -> Result<(), AudioMixStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != AudioMixSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
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
                self.enqueue_from_ready_surface(Operation::AudioMix);
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
                self.core.audio_mix_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
            }
            AudioMixSelectionAction::SaveQueueChanges
            | AudioMixSelectionAction::CancelQueueEdit
            | AudioMixSelectionAction::None => {}
        }
        Ok(())
    }

    pub(super) fn handle_audio_mix_lifecycle_action(&self, action: AudioMixLifecycleAction) {
        if let AudioMixLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Mix output folder could not be opened: {error}");
        }
    }

    pub(super) fn handle_upscale_action(
        &mut self,
        action: UpscaleSelectionAction,
    ) -> Result<(), UpscaleStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != UpscaleSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
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
                self.enqueue_from_ready_surface(Operation::VideoUpscale);
            }
            UpscaleSelectionAction::UpscaleRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(upscale) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                if self.core.discovery.is_pending()
                    && upscale.selection.encoder_selection()
                        != crate::upscale::model::VideoEncoderSelection::Cpu
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
                self.core.upscale_executor.start_with_wake(
                    &tools,
                    &upscale.media,
                    &request,
                    wake,
                )?;
            }
            UpscaleSelectionAction::SaveQueueChanges
            | UpscaleSelectionAction::CancelQueueEdit
            | UpscaleSelectionAction::None => {}
        }
        Ok(())
    }

    pub(super) fn handle_upscale_lifecycle_action(&self, action: UpscaleLifecycleAction) {
        if let UpscaleLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Upscale output folder could not be opened: {error}");
        }
    }

    pub(super) fn handle_remux_action(
        &mut self,
        action: RemuxSelectionAction,
    ) -> Result<(), RemuxStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != RemuxSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            RemuxSelectionAction::BrowseOutput => {
                let Some(session) = self.state.remux.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(remux_output_dialog_request(session));
            }
            RemuxSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::Remux);
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
                self.core
                    .remux_executor
                    .start_with_wake(&tools, &remux.media, &request, wake)?;
            }
            RemuxSelectionAction::SaveQueueChanges
            | RemuxSelectionAction::CancelQueueEdit
            | RemuxSelectionAction::None => {}
        }
        Ok(())
    }

    pub(super) fn handle_remux_lifecycle_action(&self, action: RemuxLifecycleAction) {
        if let RemuxLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Remux output folder could not be opened: {error}");
        }
    }

    pub(super) fn handle_audio_extract_action(
        &mut self,
        action: AudioExtractSelectionAction,
    ) -> Result<(), AudioExtractStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != AudioExtractSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            AudioExtractSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(audio_extract_output_dialog_request(session));
            }
            AudioExtractSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::AudioExtract);
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
                self.core.audio_extract_executor.start_with_wake(
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

    pub(super) fn handle_audio_extract_lifecycle_action(
        &self,
        action: AudioExtractLifecycleAction,
    ) {
        if let AudioExtractLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Extract output folder could not be opened: {error}");
        }
    }

    pub(super) fn handle_frame_capture_action(
        &mut self,
        action: FrameCaptureSelectionAction,
    ) -> Result<(), FrameCaptureStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != FrameCaptureSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            FrameCaptureSelectionAction::BrowseOutput => {
                let Some(session) = self.state.frame_capture.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(frame_capture_output_dialog_request(session));
            }
            FrameCaptureSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::FrameCapture);
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
                self.core.frame_capture_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
                self.frame_capture_thumbnail_load = None;
            }
            FrameCaptureSelectionAction::CopyRequested => {
                self.start_copy_frame();
            }
            FrameCaptureSelectionAction::SaveQueueChanges
            | FrameCaptureSelectionAction::CancelQueueEdit
            | FrameCaptureSelectionAction::None => {}
            FrameCaptureSelectionAction::PositionEdited(_)
            | FrameCaptureSelectionAction::StepRequested(_) => {
                if let Some(session) = self.state.frame_capture.as_mut() {
                    session.copy_status = FrameCopyStatus::Idle;
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_frame_capture_lifecycle_action(
        &mut self,
        action: FrameCaptureLifecycleAction,
    ) {
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
        match FrameCaptureThumbnailLoad::spawn(
            session_id,
            &request,
            completion_wake(&self.egui_context),
        ) {
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

    pub(super) fn prepare_file_dialog_request(
        &self,
        mut request: FileDialogRequest,
    ) -> FileDialogRequest {
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

    pub(super) fn start_file_dialog(&mut self, request: FileDialogRequest) {
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

    pub(super) fn poll_file_dialog(&mut self) {
        if let Some(completion) = self.file_dialog.poll() {
            self.settle_file_dialog(completion);
        }
    }

    pub(super) fn settle_file_dialog(&mut self, completion: FileDialogCompletion) {
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
                self.core.intake_error = Some(MediaError::file_picker_unavailable(diagnostic));
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
                        audio_mix: self.core.audio_mix_executor.state(),
                        upscale: self.core.upscale_executor.state(),
                        remux: self.core.remux_executor.state(),
                        audio_extract: self.core.audio_extract_executor.state(),
                        frame_capture: self.core.frame_capture_executor.state(),
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

pub(super) struct ToolExecutionStates<'a> {
    pub(super) audio_mix: &'a AudioMixExecutionState,
    pub(super) upscale: &'a UpscaleExecutionState,
    pub(super) remux: &'a RemuxExecutionState,
    pub(super) audio_extract: &'a AudioExtractExecutionState,
    pub(super) frame_capture: &'a FrameCaptureExecutionState,
}

pub(super) fn settle_output_file_dialog(
    state: &mut LegacyEditorState,
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

pub(super) fn editor_output_dialog_request(edit: &OperationEditorState) -> FileDialogRequest {
    match edit.active_operation {
        Operation::AudioMix => {
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
        Operation::VideoUpscale => output_dialog_request(
            DialogTarget::UpscaleOutput {
                session_id: edit.session_id,
            },
            &edit.upscale.output_path(),
            OutputFormat::Mp4,
        ),
        Operation::Remux => {
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
        Operation::AudioExtract => {
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
        Operation::FrameCapture => {
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

pub(super) fn queued_operation_tool(operation: &QueuedOperation) -> Operation {
    match operation {
        QueuedOperation::AudioMix(_) => Operation::AudioMix,
        QueuedOperation::Upscale(_) => Operation::VideoUpscale,
        QueuedOperation::Remux(_) => Operation::Remux,
        QueuedOperation::AudioExtract(_) => Operation::AudioExtract,
        QueuedOperation::FrameCapture(_) => Operation::FrameCapture,
    }
}

fn operation_from_editor(
    edit: &mut OperationEditorState,
    nvenc_capabilities: Option<&VideoEncoderCapabilities>,
) -> Option<QueuedOperation> {
    let media = edit.media.clone();
    match edit.active_operation {
        Operation::AudioMix if edit.audio_mix.can_mix(&media) => {
            Some(QueuedOperation::AudioMix(edit.audio_mix.request(&media)))
        }
        Operation::VideoUpscale => {
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
        Operation::Remux if edit.remux.can_remux(&media) => {
            Some(QueuedOperation::Remux(edit.remux.request(&media)))
        }
        Operation::AudioExtract if edit.audio_extract.can_extract(&media) => edit
            .audio_extract
            .request(&media)
            .map(QueuedOperation::AudioExtract),
        Operation::FrameCapture if edit.frame_capture.can_capture(&media) => edit
            .frame_capture
            .request(&media)
            .map(QueuedOperation::FrameCapture),
        _ => None,
    }
}

pub(super) fn batch_defaults_from_editor(edit: &OperationEditorState) -> Option<BatchDefaults> {
    match edit.active_operation {
        Operation::AudioMix => Some(BatchDefaults::AudioMix {
            container: edit.audio_mix.container(),
            normalization: if edit.audio_mix.normalization_enabled() {
                LoudnessNormalization::Enabled(edit.audio_mix.normalization_targets())
            } else {
                LoudnessNormalization::Off
            },
        }),
        Operation::VideoUpscale => {
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
        Operation::Remux => Some(BatchDefaults::Remux {
            container: edit.remux.container(),
        }),
        Operation::AudioExtract => Some(BatchDefaults::AudioExtract {
            format: edit.audio_extract.format(),
        }),
        Operation::FrameCapture => Some(BatchDefaults::FrameCapture {
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

pub(super) fn refresh_batch_items(
    batch: &mut BatchSetupState,
    capabilities: Option<&VideoEncoderCapabilities>,
) {
    let defaults = batch.defaults.clone();
    for item in &mut batch.items {
        if item.overridden {
            continue;
        }
        let BatchSourceState::Ready { media, .. } = &item.source else {
            continue;
        };
        let media = media.clone();
        item.operation = None;
        item.reason = None;
        let incompatible = match batch.operation {
            Operation::AudioMix | Operation::AudioExtract => media.audio_streams.is_empty(),
            Operation::VideoUpscale | Operation::FrameCapture => media.video_streams.is_empty(),
            Operation::Remux => media.streams.is_empty(),
        };
        if incompatible {
            item.readiness = BatchReadiness::Incompatible;
            item.reason = Some(match batch.operation {
                Operation::AudioMix | Operation::AudioExtract => "No audio tracks.".to_owned(),
                Operation::VideoUpscale | Operation::FrameCapture => "No video track.".to_owned(),
                Operation::Remux => "No media streams.".to_owned(),
            });
            continue;
        }
        if batch.operation == Operation::AudioExtract && media.audio_streams.len() > 1 {
            item.readiness = BatchReadiness::NeedsAttention;
            item.reason = Some("Select one audio track.".to_owned());
            continue;
        }
        let mut editor = OperationEditorState::new(0, None, media, batch.operation);
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
                        Operation::VideoUpscale => "Shared target is not valid for this source.",
                        Operation::FrameCapture => "Timestamp is outside this source's duration.",
                        Operation::Remux => "Container is not compatible with this source.",
                        Operation::AudioMix => "Audio mix settings are not valid for this source.",
                        Operation::AudioExtract => "Audio extraction settings need review.",
                    }
                    .to_owned(),
                );
            }
        }
    }
}

pub(super) fn audio_mix_output_dialog_request(session: &AudioMixToolState) -> FileDialogRequest {
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

pub(super) fn remux_output_dialog_request(session: &RemuxToolState) -> FileDialogRequest {
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

pub(super) fn audio_extract_output_dialog_request(
    session: &AudioExtractToolState,
) -> FileDialogRequest {
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

pub(super) fn frame_capture_output_dialog_request(
    session: &FrameCaptureToolState,
) -> FileDialogRequest {
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

pub(super) fn shows_audio_mix_lifecycle(state: &AudioMixExecutionState) -> bool {
    !matches!(state, AudioMixExecutionState::Idle)
}

pub(super) fn shows_upscale_lifecycle(state: &UpscaleExecutionState) -> bool {
    !matches!(state, UpscaleExecutionState::Idle)
}

pub(super) fn shows_remux_lifecycle(state: &RemuxExecutionState) -> bool {
    !matches!(state, RemuxExecutionState::Idle)
}

pub(super) fn shows_audio_extract_lifecycle(state: &AudioExtractExecutionState) -> bool {
    !matches!(state, AudioExtractExecutionState::Idle)
}

pub(super) fn shows_frame_capture_lifecycle(state: &FrameCaptureExecutionState) -> bool {
    !matches!(state, FrameCaptureExecutionState::Idle)
}

pub(super) fn queue_execution_wake(
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

pub(super) fn queue_event_matches_active(
    active_job: Option<JobId>,
    event: QueueExecutionEvent,
) -> bool {
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

pub(super) fn completed_output_folder(state: &QueueJobState) -> Option<&Path> {
    match state {
        QueueJobState::Completed { output } => output.parent(),
        _ => None,
    }
}
