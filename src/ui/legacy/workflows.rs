//! Existing feature actions and worker coordination. Extracted incrementally by their owning phases.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use crate::app::dialogs::{DialogTarget, FileDialogRequest};
use crate::app::frame_capture::PositionSettle;
use crate::app::{NavigationState, Operation, ProcessExecutionArbitration, QueueEditOrigin};
use crate::audio_extract::draft::AudioExtractDraft;
use crate::audio_extract::execution::AudioExtractExecutionState;
use crate::audio_mix::draft::AudioMixDraft;
use crate::audio_mix::execution::AudioMixExecutionState;
use crate::audio_mix::model::LoudnessNormalization;
use crate::frame_capture::draft::FrameCaptureDraft;
use crate::frame_capture::execution::FrameCaptureExecutionState;
use crate::media::batch::BatchInspectionOwner;
use crate::media::error::MediaError;
use crate::media::process::{MediaProcessEvent, MediaProcessWake};
use crate::media::tools::MediaTools;
use crate::native::open_folder;
use crate::queue::{
    JobId, QueueEnqueueError, QueueJobState, QueueRunState, QueuedOperation, WaitingJobMove,
};
use crate::remux::draft::RemuxDraft;
use crate::remux::execution::RemuxExecutionState;
use crate::ui::QueueAction;
use crate::ui::batch::BatchReadiness;
use crate::upscale::draft::{UpscaleDraft, UpscaleEncoderCapabilities};
use crate::upscale::execution::UpscaleExecutionState;
use crate::upscale::model::VideoCodec;

use super::state::{
    BatchDefaults, BatchEditState, BatchEditTarget, BatchInspectionSession, BatchItem,
    BatchSetupState, BatchSourceState, OperationEditorState, QueueEditState, QueueExecutionEvent,
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
                if self.core.source.current().is_none()
                    || self.state.source_presentation.is_none() =>
            {
                self.close_source();
                Some(self.core.navigation())
            }
            route => Some(route),
        }
    }

    pub(super) fn enter_queue(&mut self) {
        self.suspend_frame_capture_support();
        self.core.enter_queue();
        self.state.intake_error_details_open = false;
    }

    pub(super) fn ready_media_tools(&self) -> Option<&MediaTools> {
        self.core.discovery.ready()
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
                self.schedule_active_frame_capture_support();
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
        let session_id = self.core.source.allocate_session_id();
        self.suspend_frame_capture_support();
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

    pub(super) fn hydrate_cached_source_operation(&mut self, operation: &QueuedOperation) {
        match operation {
            QueuedOperation::AudioMix(request) => {
                if let Some(session) = self.state.audio_mix.as_mut() {
                    session.selection = AudioMixDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::Upscale(request) => {
                if let Some(session) = self.state.upscale.as_mut() {
                    session.selection = UpscaleDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::Remux(request) => {
                if let Some(session) = self.state.remux.as_mut() {
                    session.selection = RemuxDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::AudioExtract(request) => {
                if let Some(session) = self.state.audio_extract.as_mut() {
                    session.selection = AudioExtractDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::FrameCapture(request) => {
                if let Some(session) = self.state.frame_capture.as_mut() {
                    session.selection = FrameCaptureDraft::from_request(&session.media, request);
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
        let nvenc_capabilities = self.core.discovery.upscale_capabilities();
        let Some(edit) = self.queue_edit.as_mut() else {
            return false;
        };
        let job_id = edit.job_id;
        let edit = &mut edit.editor;
        let Some(operation) = operation_from_editor(edit, nvenc_capabilities) else {
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
        // Borrow each draft and its source directly; the queue owns the resulting immutable request.
        let queued = match operation {
            Operation::AudioMix => self.state.audio_mix.as_ref().and_then(|session| {
                session
                    .selection
                    .validated_request(&session.media)
                    .ok()
                    .map(|request| (&session.media, QueuedOperation::AudioMix(request)))
            }),
            Operation::VideoUpscale => {
                let Some(capabilities) = self.core.discovery.upscale_capabilities() else {
                    return Ok(None);
                };
                self.state.upscale.as_ref().and_then(|session| {
                    session
                        .selection
                        .validated_request(&session.media, capabilities)?
                        .ok()
                        .map(|request| (&session.media, QueuedOperation::Upscale(request)))
                })
            }
            Operation::Remux => self.state.remux.as_ref().and_then(|session| {
                session
                    .selection
                    .validated_request(&session.media)
                    .ok()
                    .map(|request| (&session.media, QueuedOperation::Remux(request)))
            }),
            Operation::AudioExtract => self.state.audio_extract.as_ref().and_then(|session| {
                session
                    .selection
                    .validated_request(&session.media)
                    .ok()
                    .map(|request| (&session.media, QueuedOperation::AudioExtract(request)))
            }),
            Operation::FrameCapture => self.state.frame_capture.as_ref().and_then(|session| {
                session
                    .selection
                    .validated_request(&session.media)
                    .ok()
                    .map(|request| (&session.media, QueuedOperation::FrameCapture(request)))
            }),
        };
        let Some((media, request)) = queued else {
            return Ok(None);
        };
        self.core.media_queue.enqueue(media, request).map(Some)
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

    pub(super) fn retry_media_tools(&mut self) {
        if !self.core.discovery.is_pending() {
            self.tools_error_details_open = false;
            self.core
                .discovery
                .start(completion_wake(&self.egui_context));
        }
    }

    pub(super) fn start_batch_paths(&mut self, paths: Vec<PathBuf>) -> bool {
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
                    self.core
                        .source
                        .set_error(MediaError::probe_failure(format!(
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
        self.suspend_frame_capture_support();
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
        let capabilities = self.core.discovery.upscale_capabilities();
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
        refresh_batch_items(batch, capabilities);
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
        let session_id = self.core.source.allocate_session_id();
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
        let capabilities = self.core.discovery.upscale_capabilities();
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
                refresh_batch_items(batch, capabilities);
            }
            BatchEditTarget::Item(index) => {
                let Some(operation) = operation_from_editor(&edit.editor, capabilities) else {
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
        self.schedule_active_frame_capture_support();
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
                self.schedule_active_frame_capture_support();
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
}

fn operation_from_editor(
    edit: &OperationEditorState,
    capabilities: Option<UpscaleEncoderCapabilities<'_>>,
) -> Option<QueuedOperation> {
    let media = &edit.media;
    match edit.active_operation {
        Operation::AudioMix => edit
            .audio_mix
            .validated_request(media)
            .ok()
            .map(QueuedOperation::AudioMix),
        Operation::VideoUpscale => edit
            .upscale
            .validated_request(media, capabilities?)?
            .ok()
            .map(QueuedOperation::Upscale),
        Operation::Remux => edit
            .remux
            .validated_request(media)
            .ok()
            .map(QueuedOperation::Remux),
        Operation::AudioExtract => edit
            .audio_extract
            .validated_request(media)
            .ok()
            .map(QueuedOperation::AudioExtract),
        Operation::FrameCapture => edit
            .frame_capture
            .validated_request(media)
            .ok()
            .map(QueuedOperation::FrameCapture),
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
    let media = &edit.media;
    match defaults {
        BatchDefaults::AudioMix {
            container,
            normalization,
        } => {
            edit.audio_mix.set_container(media, *container);
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
            edit.upscale.set_target(media, *target);
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
                .selection
                .set_timestamp_text(media, timestamp.to_string());
            edit.frame_capture.set_format(*format);
        }
    }
}

pub(super) fn refresh_batch_items(
    batch: &mut BatchSetupState,
    capabilities: Option<UpscaleEncoderCapabilities<'_>>,
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
        match operation_from_editor(&editor, capabilities) {
            Some(operation) => {
                item.operation = Some(operation);
                item.readiness = BatchReadiness::Ready;
            }
            None => {
                item.readiness = BatchReadiness::NeedsAttention;
                item.reason = Some(
                    match batch.operation {
                        Operation::VideoUpscale
                            if matches!(
                                capabilities,
                                Some(UpscaleEncoderCapabilities::Checking)
                            ) && editor.upscale.encoder_selection()
                                != crate::upscale::model::VideoEncoderSelection::Cpu =>
                        {
                            "Checking NVIDIA NVENC..."
                        }
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

pub(super) fn completed_output_folder(state: &QueueJobState) -> Option<&Path> {
    match state {
        QueueJobState::Completed { output } => output.parent(),
        _ => None,
    }
}
