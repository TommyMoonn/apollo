//! Queue presentation integration and native action dispatch.
use super::{ApolloApp, state::EditorPresentation};
use crate::{
    app::{
        NavigationState, Operation, QueueAction, QueueEditOrigin,
        dialogs::{DialogTarget, FileDialogRequest},
        queue::{QueueEditReturn, QueueEffect, QueueSubmission},
    },
    native::open_folder,
    queue::{JobId, QueueEnqueueError, QueuedOperation},
};

impl ApolloApp {
    pub(super) fn enter_queue(&mut self) {
        if !self.file_intake_enabled() {
            return;
        }
        let previous = self.core.navigation();
        if self.core.enter_queue() {
            self.suspend_frame_capture_support_from(previous);
            self.state.intake_error_details_open = false;
            self.egui_context.request_repaint();
        }
    }

    pub(super) fn handle_queue_view_action(&mut self, action: QueueAction) {
        if self.confirm_cancel_queue {
            return;
        }
        if action == QueueAction::CancelQueue {
            if self.core.queue.scheduler().pending_count() > 0 {
                self.confirm_cancel_queue = true;
                self.egui_context.request_repaint();
            }
        } else {
            self.handle_queue_action(action);
        }
    }

    pub(super) fn resolve_queue_cancellation(&mut self, confirmed: bool) {
        if !std::mem::take(&mut self.confirm_cancel_queue) {
            return;
        }
        if confirmed {
            self.handle_queue_action(QueueAction::CancelQueue);
        }
        self.egui_context.request_repaint();
    }

    pub(super) fn handle_queue_action(&mut self, action: QueueAction) -> bool {
        if action == QueueAction::AddFiles
            && (!self.file_intake_enabled() || self.ready_media_tools().is_none())
        {
            return false;
        }
        let progress = &mut self.progress_repaint;
        match self.core.queue_action(action, || progress.begin()) {
            QueueEffect::Handled(changed) => {
                if changed {
                    self.egui_context.request_repaint();
                }
                changed
            }
            QueueEffect::AddFiles => {
                self.start_file_dialog(FileDialogRequest::OpenSources {
                    initial_directory: None,
                    target: DialogTarget::BatchInput,
                });
                true
            }
            QueueEffect::Edit(id, origin) => self.begin_queue_edit(id, origin),
            QueueEffect::OpenFolder(directory) => {
                if let Err(error) = open_folder(&directory) {
                    eprintln!("Queue output folder could not be opened: {error}");
                }
                true
            }
            QueueEffect::Returned => {
                self.schedule_active_frame_capture_support();
                true
            }
        }
    }

    pub(super) fn begin_queue_edit(&mut self, id: JobId, origin: QueueEditOrigin) -> bool {
        if self.ready_media_tools().is_none() {
            return false;
        }
        let Some(edit) = self.core.prepare_queue_edit(id, origin) else {
            return false;
        };
        self.suspend_frame_capture_support();
        self.queue_editor_presentation = Some(EditorPresentation::new(&edit.media));
        self.core.activate_queue_edit(edit, origin);
        self.schedule_active_frame_capture_support();
        true
    }

    pub(super) fn finish_queue_edit(&mut self) -> bool {
        if !matches!(self.core.navigation(), NavigationState::QueueEdit { .. }) {
            return false;
        }
        self.cancel_frame_capture_support();
        let Some(return_to) = self.core.finish_queue_edit() else {
            return false;
        };
        self.queue_editor_presentation = None;
        match return_to {
            QueueEditReturn::Queue => true,
            QueueEditReturn::Source(request) => self.restore_queued_job_workspace(&request),
        }
    }

    pub(super) fn select_queue_edit_tool(&mut self, operation: Operation) -> bool {
        if !matches!(self.core.navigation(), NavigationState::QueueEdit { .. }) {
            return false;
        }
        let Some(previous) = self.core.queue_edit().map(|edit| edit.active_operation) else {
            return false;
        };
        if previous == operation {
            return true;
        }
        if previous == Operation::FrameCapture {
            self.cancel_frame_capture_support();
        }
        if !self.core.select_queue_edit_tool(operation) {
            return false;
        }
        if operation == Operation::FrameCapture {
            self.schedule_active_frame_capture_support();
        }
        true
    }

    pub(super) fn save_queue_edit(&mut self) -> bool {
        self.core.save_queue_edit() && self.finish_queue_edit()
    }

    pub(super) fn enqueue_current_operation(
        &mut self,
        operation: Operation,
    ) -> Result<Option<JobId>, QueueEnqueueError> {
        // Borrow each draft and its source directly; the queue owns the resulting immutable request.
        let queued = match operation {
            Operation::AudioMix => {
                let session = self
                    .state
                    .audio_mix
                    .as_ref()
                    .ok_or(QueueEnqueueError::SourceDoesNotMatchMedia)?;
                Some((
                    &session.media,
                    QueuedOperation::AudioMix(session.selection.request(&session.media)),
                ))
            }
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
            Operation::Remux => {
                let session = self
                    .state
                    .remux
                    .as_ref()
                    .ok_or(QueueEnqueueError::SourceDoesNotMatchMedia)?;
                Some((
                    &session.media,
                    QueuedOperation::Remux(session.selection.request(&session.media)),
                ))
            }
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
        let Some((identity, accepted_media)) = self.core.source.current_with_identity() else {
            return Err(QueueEnqueueError::SourceDoesNotMatchMedia);
        };
        if media != accepted_media {
            return Err(QueueEnqueueError::SourceDoesNotMatchMedia);
        }
        self.core
            .queue
            .enqueue(identity, accepted_media, request)
            .map(Some)
    }

    /// Both migrated form actions admit the same immutable request. Only Process now starts it.
    pub(super) fn submit_queued_operation(
        &mut self,
        operation: Operation,
        submission: QueueSubmission,
    ) {
        let process_now = submission == QueueSubmission::ProcessNow;
        if self.core.active_operation() != Some(operation)
            || !self.file_intake_enabled()
            || !self.operation_ready_surface(operation)
            || self.ready_media_tools().is_none()
            || (process_now && !self.core.process_now_allowed())
        {
            return;
        }
        let result = self.enqueue_current_operation(operation);
        let admitted = matches!(result, Ok(Some(_)));
        self.core.record_queue_submission(operation, result);
        if admitted && process_now {
            self.enter_queue();
            self.handle_queue_action(QueueAction::Start);
        }
    }

    pub(super) fn enqueue_from_ready_surface(&mut self, operation: Operation) {
        let result = self.enqueue_current_operation(operation);
        self.core.record_queue_submission(operation, result);
    }
}
