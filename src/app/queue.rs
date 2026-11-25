//! Application integration with the existing queue scheduler.
use super::{
    Application, NavigationState, Operation, QueueEditOrigin, editor::OperationEditorState,
};
use crate::{
    media::{
        error::MediaError, intake::SourceIdentity, model::MediaFile, process::MediaProcessWake,
        tools::MediaTools,
    },
    queue::{
        BatchEnqueueError, JobId, MediaQueue, QueueEnqueueError, QueueJobState, QueueMutationError,
        QueueRunState, QueuedJob, QueuedOperation, WaitingJobMove,
    },
};
use std::{
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::mpsc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueAction {
    AddFiles,
    Start,
    CancelCurrent,
    StopAfterCurrent,
    CancelQueue,
    Edit(JobId),
    MoveEarlier(JobId),
    MoveLater(JobId),
    Remove(JobId),
    Retry(JobId),
    OpenFolder(JobId),
    PreviewFrame(JobId),
    ClearFinished,
    Back,
    #[default]
    None,
}

/// A normal form can append work or admit and start a job immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueSubmission {
    ProcessNow,
    AddToQueue,
}

/// Native and presentation work stays in the adapter.
pub(crate) enum QueueEffect {
    Handled(bool),
    AddFiles,
    Edit(JobId, QueueEditOrigin),
    OpenFolder(PathBuf),
    PreviewFrame(JobId, PathBuf),
    Returned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueExecutionEvent {
    job_id: JobId,
}

pub(crate) struct QueueWorkflow {
    scheduler: MediaQueue,
    sender: mpsc::Sender<QueueExecutionEvent>,
    receiver: mpsc::Receiver<QueueExecutionEvent>,
}

impl Default for QueueWorkflow {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            scheduler: MediaQueue::new(),
            sender,
            receiver,
        }
    }
}

impl QueueWorkflow {
    pub(crate) fn scheduler(&self) -> &MediaQueue {
        &self.scheduler
    }

    pub(crate) fn enqueue(
        &mut self,
        source_identity: &SourceIdentity,
        media: &MediaFile,
        operation: QueuedOperation,
    ) -> Result<JobId, QueueEnqueueError> {
        self.scheduler.enqueue(source_identity, media, operation)
    }

    pub(crate) fn enqueue_batch(
        &mut self,
        requests: &[(&SourceIdentity, &MediaFile, &QueuedOperation)],
    ) -> Result<Vec<JobId>, BatchEnqueueError> {
        self.scheduler.enqueue_batch(requests)
    }

    pub(crate) fn cancel(&mut self) -> bool {
        self.scheduler.cancel_queue()
    }

    fn start(
        &mut self,
        tools: &MediaTools,
        capabilities_ready: bool,
        mut wake: impl FnMut() -> MediaProcessWake,
    ) -> bool {
        let sender = self.sender.clone();
        self.scheduler.start(tools, capabilities_ready, |job_id| {
            queue_execution_wake(job_id, sender.clone(), wake())
        })
    }

    fn drain_current_events(&self, active: Option<JobId>) -> bool {
        // Drain all notifications, including retired job notifications, before advancing.
        let mut received = false;
        while let Ok(event) = self.receiver.try_recv() {
            received |= active == Some(event.job_id);
        }
        received
    }

    fn poll(
        &mut self,
        tools: Option<&MediaTools>,
        capabilities_ready: bool,
        mut wake: impl FnMut() -> MediaProcessWake,
    ) {
        let received = self.drain_current_events(self.scheduler.active_job());
        // Readiness loss stops advancement even if the active process has not notified us yet.
        let readiness_lost =
            tools.is_none() && self.scheduler.run_state() == QueueRunState::Running;
        let awaiting_launch = self.scheduler.run_state() == QueueRunState::Running
            && self.scheduler.active_job().is_none();
        if !received && !readiness_lost && !awaiting_launch {
            return;
        }
        let sender = self.sender.clone();
        self.scheduler.poll(tools, capabilities_ready, |job_id| {
            queue_execution_wake(job_id, sender.clone(), wake())
        });
    }
}

fn queue_execution_wake(
    job_id: JobId,
    sender: mpsc::Sender<QueueExecutionEvent>,
    wake: MediaProcessWake,
) -> MediaProcessWake {
    MediaProcessWake::new(move |event| {
        // Make the event observable before asking the adapter to wake up.
        let _ = sender.send(QueueExecutionEvent { job_id });
        wake.notify(event);
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueueEditState {
    pub(crate) job_id: JobId,
    pub(crate) editor: OperationEditorState,
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
impl QueueEditState {
    fn new(session_id: u64, job: &QueuedJob) -> Self {
        let mut editor = OperationEditorState::new(
            session_id,
            Some(job.source_identity().clone()),
            job.media().clone(),
            Operation::from(job.operation()),
        );
        editor.hydrate(job.operation());
        Self {
            job_id: job.id(),
            editor,
        }
    }
}

pub(crate) enum QueueEditReturn {
    Queue,
    Source(Box<QueuedOperation>),
}

impl Application {
    pub(crate) fn process_now_allowed(&self) -> bool {
        self.queue.scheduler().pending_count() == 0
            && !self.queue.scheduler().is_started()
            && self.queue_start_allowed()
    }

    pub(crate) fn poll_media_queue(&mut self, wake: impl FnMut() -> MediaProcessWake) {
        let before = self.batch_setup.is_some().then(|| {
            (
                self.queue.scheduler().pending_count(),
                self.queue.scheduler().active_job(),
            )
        });
        self.queue.poll(
            self.discovery.ready(),
            self.discovery.settled_capabilities().is_some(),
            wake,
        );
        if before.is_some_and(|before| {
            before
                != (
                    self.queue.scheduler().pending_count(),
                    self.queue.scheduler().active_job(),
                )
        }) {
            self.refresh_batch_admission();
        }
    }

    pub(crate) fn queue_action(
        &mut self,
        action: QueueAction,
        wake: impl FnMut() -> MediaProcessWake,
    ) -> QueueEffect {
        use QueueEffect::*;
        if action != QueueAction::None {
            self.queue_action_error = None;
        }
        match action {
            QueueAction::AddFiles
                if self.queue_origin().is_some() && self.batch_origin().is_some() =>
            {
                AddFiles
            }
            QueueAction::AddFiles => Handled(false),
            QueueAction::Start => {
                if !self.queue_start_allowed()
                    || matches!(self.navigation(), NavigationState::QueueEdit { .. })
                {
                    return Handled(false);
                }
                let Some(tools) = self.discovery.ready() else {
                    return Handled(false);
                };
                Handled(self.queue.start(
                    tools,
                    self.discovery.settled_capabilities().is_some(),
                    wake,
                ))
            }
            QueueAction::CancelCurrent => Handled(self.queue.scheduler.cancel_current()),
            QueueAction::StopAfterCurrent => Handled(self.queue.scheduler.stop_after_current()),
            QueueAction::CancelQueue => Handled(self.queue.cancel()),
            QueueAction::Edit(id) => {
                let Some(origin) = self.queue_origin() else {
                    return Handled(false);
                };
                if !self.queue.scheduler.can_edit() {
                    self.queue_action_error =
                        Some(QueueMutationError::AutomaticAdvancementActive.to_string());
                    return Handled(false);
                }
                if !self
                    .queue
                    .scheduler
                    .waiting_jobs()
                    .any(|job| job.id() == id)
                {
                    self.queue_action_error =
                        Some("This job is no longer waiting. Choose a waiting job to edit.".into());
                    return Handled(false);
                }
                Edit(id, QueueEditOrigin::Queue(origin))
            }
            QueueAction::MoveEarlier(id) | QueueAction::MoveLater(id) => {
                let direction = if matches!(action, QueueAction::MoveEarlier(_)) {
                    WaitingJobMove::Earlier
                } else {
                    WaitingJobMove::Later
                };
                let result = self.queue.scheduler.move_waiting(id, direction);
                Handled(self.record_queue_mutation(result))
            }
            QueueAction::Remove(id) => {
                let removed = self.queue.scheduler.remove_waiting(id);
                if !removed {
                    self.queue_action_error =
                        Some("This job is no longer waiting and cannot be removed.".into());
                }
                Handled(removed)
            }
            QueueAction::Retry(id) => {
                if self.queue_origin().is_none() {
                    return Handled(false);
                }
                if self.discovery.ready().is_none() {
                    self.queue_action_error =
                        Some("Media tools are unavailable. Restore them before retrying.".into());
                    return Handled(false);
                }
                let result = self
                    .queue
                    .scheduler
                    .retry(id, self.discovery.settled_capabilities())
                    .map(|_| true);
                Handled(self.record_queue_mutation(result))
            }
            QueueAction::ClearFinished => Handled(self.queue.scheduler.clear_finished() > 0),
            QueueAction::OpenFolder(id) => self
                .queue
                .scheduler
                .jobs()
                .iter()
                .find(|job| job.id() == id)
                .and_then(|job| completed_output_folder(job.state()))
                .map_or(Handled(false), |path| OpenFolder(path.to_path_buf())),
            QueueAction::PreviewFrame(id) => {
                if self.queue_origin().is_none() {
                    return Handled(false);
                }
                self.queue
                    .scheduler
                    .jobs()
                    .iter()
                    .find(|job| job.id() == id)
                    .and_then(|job| match (job.operation(), job.state()) {
                        (QueuedOperation::FrameCapture(_), QueueJobState::Completed { output }) => {
                            Some(output.clone())
                        }
                        _ => None,
                    })
                    .map_or(Handled(false), |output| PreviewFrame(id, output))
            }
            QueueAction::Back => {
                if self.queue_origin().is_none() {
                    return Handled(false);
                }
                self.return_from_queue();
                Returned
            }
            QueueAction::None => Handled(false),
        }
    }

    fn record_queue_mutation(&mut self, result: Result<bool, QueueMutationError>) -> bool {
        match result {
            Ok(changed) => changed,
            Err(error) => {
                self.queue_action_error = Some(error.to_string());
                false
            }
        }
    }

    pub(crate) fn queue_edit(&self) -> Option<&QueueEditState> {
        self.queue_edit.as_ref()
    }
    pub(crate) fn queue_edit_with_capabilities(
        &mut self,
    ) -> Option<(
        &mut QueueEditState,
        crate::upscale::draft::UpscaleEncoderCapabilities<'_>,
    )> {
        let capabilities = self.discovery.settled_capabilities().map_or(
            crate::upscale::draft::UpscaleEncoderCapabilities::Checking,
            crate::upscale::draft::UpscaleEncoderCapabilities::Ready,
        );
        Some((self.queue_edit.as_mut()?, capabilities))
    }

    pub(crate) fn queue_edit_mut(&mut self) -> Option<&mut QueueEditState> {
        self.queue_edit.as_mut()
    }

    // Prepare before retiring the old editor, so rejected requests leave it intact.
    pub(crate) fn prepare_queue_edit(
        &mut self,
        id: JobId,
        origin: QueueEditOrigin,
    ) -> Option<QueueEditState> {
        if !self.queue.scheduler.can_edit() || !self.can_enter_queue_edit(origin) {
            return None;
        }
        let job = self
            .queue
            .scheduler
            .jobs()
            .iter()
            .find(|job| job.id() == id && matches!(job.state(), QueueJobState::Waiting))?;
        Some(QueueEditState::new(self.source.allocate_session_id(), job))
    }

    pub(crate) fn activate_queue_edit(&mut self, edit: QueueEditState, origin: QueueEditOrigin) {
        self.queue_edit = Some(edit);
        self.enter_queue_edit(origin);
    }

    pub(crate) fn finish_queue_edit(&mut self) -> Option<QueueEditReturn> {
        let NavigationState::QueueEdit { origin } = self.navigation() else {
            return None;
        };
        match origin {
            QueueEditOrigin::Queue(origin) => {
                self.queue_edit = None;
                self.show_queue(origin);
                Some(QueueEditReturn::Queue)
            }
            QueueEditOrigin::SourceWorkspace => {
                let id = self.queue_edit.as_ref()?.job_id;
                let job = self
                    .queue
                    .scheduler
                    .jobs()
                    .iter()
                    .find(|job| job.id() == id)?;
                let operation = Box::new(job.operation().clone());
                self.source
                    .restore(job.source_identity().clone(), job.media().clone());
                self.queue_edit = None;
                Some(QueueEditReturn::Source(operation))
            }
        }
    }

    pub(crate) fn select_queue_edit_tool(&mut self, operation: Operation) -> bool {
        if !matches!(self.navigation(), NavigationState::QueueEdit { .. }) {
            return false;
        }
        let Some(edit) = self.queue_edit.as_mut() else {
            return false;
        };
        if edit.active_operation != operation {
            edit.active_operation = operation;
            edit.error = None;
            edit.error_details_open = false;
        }
        true
    }

    pub(crate) fn save_queue_edit(&mut self) -> bool {
        if !matches!(self.navigation(), NavigationState::QueueEdit { .. }) {
            return false;
        }
        let capabilities = self.discovery.upscale_capabilities();
        let Some(edit) = self.queue_edit.as_mut() else {
            return false;
        };
        if !self.queue.scheduler.can_edit() {
            edit.error = Some(MediaError::invalid_request(
                QueueMutationError::AutomaticAdvancementActive.to_string(),
            ));
            edit.error_details_open = true;
            return false;
        }
        let Some(operation) = edit.validated_request(capabilities) else {
            return false;
        };
        match self
            .queue
            .scheduler
            .replace_waiting_operation(edit.job_id, operation)
        {
            Ok(()) => true,
            Err(error) => {
                edit.error = Some(match &error {
                    QueueMutationError::SourceUnavailable(error) => {
                        MediaError::from_intake_error(error)
                    }
                    QueueMutationError::SourceChanged => {
                        MediaError::source_changed(error.to_string())
                    }
                    QueueMutationError::InvalidRequest(error) => error.user_error(),
                    QueueMutationError::Output(error) => MediaError::from_output_error(error),
                    _ => MediaError::invalid_request(format!(
                        "Could not save queue changes: {error}"
                    )),
                });
                edit.error_details_open = true;
                false
            }
        }
    }

    pub(crate) fn record_queue_submission(
        &mut self,
        operation: Operation,
        result: Result<Option<JobId>, QueueEnqueueError>,
    ) {
        self.queue_confirmation = None;
        self.operation_action_error = None;
        match result {
            Ok(Some(_)) => self.queue_confirmation = Some(operation),
            Ok(None) => {}
            Err(error) => {
                self.operation_action_error =
                    Some((operation, media_error_from_queue_enqueue(&error)))
            }
        }
    }
}

fn media_error_from_queue_enqueue(error: &QueueEnqueueError) -> MediaError {
    match error {
        QueueEnqueueError::Source(error) => MediaError::from_intake_error(error),
        QueueEnqueueError::SourceDoesNotMatchMedia => {
            MediaError::source_unavailable(error.to_string())
        }
        QueueEnqueueError::Output(error) => MediaError::from_output_error(error),
        QueueEnqueueError::SourceChanged => MediaError::source_changed(error.to_string()),
        QueueEnqueueError::InvalidRequest(error) => error.user_error(),
        QueueEnqueueError::JobIdExhausted | QueueEnqueueError::DestinationReserved(_) => {
            MediaError::invalid_request(error.to_string())
        }
    }
}

fn completed_output_folder(state: &QueueJobState) -> Option<&Path> {
    match state {
        QueueJobState::Completed { output } => output.parent(),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
