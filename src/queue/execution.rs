use crate::media::{
    execution::{MediaExecution, MediaExecutionStartError, MediaExecutionState, ProcessControl},
    intake::SourceIdentity,
    model::MediaFile,
    output::{
        OutputCollisionPolicy, normalized_output_destination, preflight_output_destination,
        require_vacant_destination,
    },
    process::{MediaProcess, MediaProcessCommand, MediaProcessStartError, MediaProcessWake},
    progress::MediaProgress,
    tools::{MediaTools, VideoEncoderCapabilities},
};

use super::model::{
    BatchEnqueueError, JobId, QueueEnqueueError, QueueJobFailure, QueueJobState,
    QueueMutationError, QueueRunState, QueuedJob, QueuedOperation, WaitingJobMove,
};

pub struct MediaQueue {
    jobs: Vec<QueuedJob>,
    next_job_id: Option<u64>,
    run_state: QueueRunState,
    active_job: Option<JobId>,
    cancellation_pending: bool,
    execution: MediaExecution,
}

impl Default for MediaQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MediaQueue {
    fn drop(&mut self) {
        self.cancel_for_shutdown();
    }
}

impl MediaQueue {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_job_id: Some(1),
            run_state: QueueRunState::Stopped,
            active_job: None,
            cancellation_pending: false,
            execution: MediaExecution::new(),
        }
    }

    pub fn jobs(&self) -> &[QueuedJob] {
        &self.jobs
    }

    pub fn waiting_jobs(&self) -> impl Iterator<Item = &QueuedJob> {
        self.jobs
            .iter()
            .filter(|job| matches!(job.state(), QueueJobState::Waiting))
    }

    pub fn pending_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.state().is_pending())
            .count()
    }

    pub fn active_job(&self) -> Option<JobId> {
        self.active_job
    }

    pub fn active_progress(&self) -> Option<&MediaProgress> {
        self.execution.progress()
    }

    pub fn cancellation_pending(&self) -> bool {
        self.cancellation_pending
    }

    pub fn is_started(&self) -> bool {
        self.run_state != QueueRunState::Stopped
    }

    pub fn run_state(&self) -> QueueRunState {
        self.run_state
    }

    pub fn is_running(&self) -> bool {
        self.active_job.is_some()
    }

    /// Admit a request using the identity captured by the successful inspection, not a fresh one.
    pub fn enqueue(
        &mut self,
        source_identity: &SourceIdentity,
        media: &MediaFile,
        operation: QueuedOperation,
    ) -> Result<JobId, QueueEnqueueError> {
        Self::validate_admission(source_identity, media, &operation)?;
        Self::validate_unreserved_destination(operation.output(), self.exact_reservations())?;
        self.append_validated(
            source_identity.clone(),
            media.clone(),
            operation,
            OutputCollisionPolicy::Numbered,
        )
    }

    fn append_validated(
        &mut self,
        source_identity: SourceIdentity,
        media: MediaFile,
        operation: QueuedOperation,
        policy: OutputCollisionPolicy,
    ) -> Result<JobId, QueueEnqueueError> {
        let raw_id = self.next_job_id.ok_or(QueueEnqueueError::JobIdExhausted)?;
        let id = JobId::new(raw_id);
        self.jobs.push(
            QueuedJob::new(id, source_identity, media, operation).with_collision_policy(policy),
        );
        self.next_job_id = raw_id.checked_add(1);
        Ok(id)
    }

    /// Validate every candidate and the full ID range before retaining any batch jobs.
    pub fn enqueue_batch(
        &mut self,
        items: &[(&SourceIdentity, &MediaFile, &QueuedOperation)],
    ) -> Result<Vec<JobId>, BatchEnqueueError> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let raw_start = self.next_job_id.ok_or(BatchEnqueueError::JobIdExhausted)?;
        let offset =
            u64::try_from(items.len() - 1).map_err(|_| BatchEnqueueError::JobIdExhausted)?;
        let raw_last = raw_start
            .checked_add(offset)
            .ok_or(BatchEnqueueError::JobIdExhausted)?;
        if let Some(error) = self.batch_admission_errors(items).into_iter().next() {
            return Err(error);
        }

        let mut ids = Vec::with_capacity(items.len());
        for (raw_id, &(identity, media, operation)) in (raw_start..=raw_last).zip(items) {
            let id = JobId::new(raw_id);
            self.jobs.push(QueuedJob::new(
                id,
                identity.clone(),
                media.clone(),
                operation.clone(),
            ));
            ids.push(id);
        }
        self.next_job_id = raw_last.checked_add(1);
        Ok(ids)
    }

    /// Check all included candidates using the same rules as atomic submission.
    pub(crate) fn batch_admission_errors(
        &self,
        items: &[(&SourceIdentity, &MediaFile, &QueuedOperation)],
    ) -> Vec<BatchEnqueueError> {
        // Resolve existing paths once for this admission pass, not once per candidate.
        let reserved: Vec<_> = self
            .pending_jobs()
            .flat_map(|job| Self::job_destinations(job).map(move |path| (job.id(), path)))
            .collect();
        let mut destinations = Vec::new();
        let mut errors = Vec::new();
        for (index, (identity, media, operation)) in items.iter().enumerate() {
            let result = Self::validate_admission(identity, media, operation)
                .and_then(|()| {
                    normalized_output_destination(operation.output())
                        .map_err(QueueEnqueueError::Output)
                })
                .and_then(|destination| {
                    if let Some((id, _)) = reserved
                        .iter()
                        .find(|(_, path)| same_destination(&destination, path))
                    {
                        Err(QueueEnqueueError::DestinationReserved(*id))
                    } else {
                        Ok(destination)
                    }
                });
            match result {
                Ok(destination) => {
                    if destinations
                        .iter()
                        .any(|other: &std::path::PathBuf| same_destination(&destination, other))
                    {
                        errors.push(BatchEnqueueError::DuplicateDestination { index });
                    } else {
                        destinations.push(destination);
                    }
                }
                Err(error) => errors.push(BatchEnqueueError::Item { index, error }),
            }
        }
        errors
    }

    pub(crate) fn next_job_ready(&self, capabilities_ready: bool) -> bool {
        self.waiting_jobs().next().is_some_and(|job| {
            capabilities_ready || !job.operation().requires_encoder_capabilities()
        })
    }

    /// A start is available only when the FIFO head can launch and no execution is owned.
    pub(crate) fn can_start(&self, capabilities_ready: bool) -> bool {
        self.can_edit() && self.next_job_ready(capabilities_ready)
    }

    /// Returns whether a start request changed queue state, even if all launches failed.
    pub fn start<F>(
        &mut self,
        tools: &MediaTools,
        capabilities_ready: bool,
        mut wake_for_job: F,
    ) -> bool
    where
        F: FnMut(JobId) -> MediaProcessWake,
    {
        let mut launcher =
            |job_id, command| MediaProcess::start_with_wake(command, wake_for_job(job_id));
        self.start_with_launcher(tools, capabilities_ready, &mut launcher)
    }

    pub fn poll<F>(
        &mut self,
        tools: Option<&MediaTools>,
        capabilities_ready: bool,
        mut wake_for_job: F,
    ) where
        F: FnMut(JobId) -> MediaProcessWake,
    {
        let mut launcher =
            |job_id, command| MediaProcess::start_with_wake(command, wake_for_job(job_id));
        self.poll_with_launcher(tools, capabilities_ready, &mut launcher);
    }

    pub fn cancel_current(&mut self) -> bool {
        if self.cancellation_pending || !self.execution.cancel() {
            return false;
        }
        self.cancellation_pending = true;
        true
    }

    pub fn stop_after_current(&mut self) -> bool {
        match (self.run_state, self.active_job) {
            (QueueRunState::Running, Some(_)) => {
                self.run_state = QueueRunState::StoppingAfterCurrent;
                true
            }
            (QueueRunState::Running, None) => {
                self.run_state = QueueRunState::Stopped;
                true
            }
            (QueueRunState::Stopped | QueueRunState::StoppingAfterCurrent, _) => false,
        }
    }

    pub fn cancel_queue(&mut self) -> bool {
        let mut changed = self.run_state != QueueRunState::Stopped;
        self.run_state = QueueRunState::Stopped;
        for job in &mut self.jobs {
            if matches!(job.state(), QueueJobState::Waiting) {
                job.set_state(QueueJobState::Cancelled);
                changed = true;
            }
        }
        self.cancel_current() || changed
    }

    pub fn replace_waiting_operation(
        &mut self,
        id: JobId,
        operation: QueuedOperation,
    ) -> Result<(), QueueMutationError> {
        self.require_mutation_stopped()?;
        let index = self.waiting_job_index(id)?;
        let job = &self.jobs[index];

        Self::validate_admission(job.source_identity(), job.media(), &operation)?;
        Self::validate_unreserved_destination(
            operation.output(),
            self.pending_jobs().filter(|job| job.id() != id),
        )?;
        if job.collision_policy() == OutputCollisionPolicy::Exact {
            require_vacant_destination(operation.output()).map_err(QueueMutationError::Output)?;
        }
        self.jobs[index].replace_operation(operation);
        Ok(())
    }

    pub fn retry(
        &mut self,
        id: JobId,
        capabilities: Option<&VideoEncoderCapabilities>,
    ) -> Result<JobId, QueueMutationError> {
        let job = self
            .jobs
            .iter()
            .find(|job| job.id() == id)
            .ok_or(QueueMutationError::JobNotFound(id))?;
        if !matches!(
            job.state(),
            QueueJobState::Failed(_) | QueueJobState::Cancelled
        ) {
            return Err(QueueMutationError::JobNotRetryable(id));
        }
        let identity = job.source_identity().clone();
        let media = job.media().clone();
        let mut operation = job.operation().clone();
        operation.refresh_encoder_capabilities(capabilities)?;
        Self::validate_admission(&identity, &media, &operation)?;
        Self::validate_unreserved_destination(operation.output(), self.pending_jobs())?;
        require_vacant_destination(operation.output()).map_err(QueueMutationError::Output)?;
        Ok(self.append_validated(identity, media, operation, OutputCollisionPolicy::Exact)?)
    }

    fn pending_jobs(&self) -> impl Iterator<Item = &QueuedJob> {
        self.jobs.iter().filter(|job| job.state().is_pending())
    }

    fn exact_reservations(&self) -> impl Iterator<Item = &QueuedJob> {
        self.pending_jobs()
            .filter(|job| job.collision_policy() == OutputCollisionPolicy::Exact)
    }

    /// The requested destination and, for an active job, the selected publication name.
    fn job_destinations(job: &QueuedJob) -> impl Iterator<Item = std::path::PathBuf> + '_ {
        let active = match job.state() {
            QueueJobState::Running { output } => Some(output.as_path()),
            _ => None,
        };
        std::iter::once(job.operation().output())
            .chain(active)
            .filter_map(|path| normalized_output_destination(path).ok())
    }

    /// Derive reservations from retained pending jobs, including the active selected name.
    /// Normal submissions can share numbered destinations, but not an exact retry reservation.
    fn validate_unreserved_destination<'a>(
        output: &std::path::Path,
        jobs: impl Iterator<Item = &'a QueuedJob>,
    ) -> Result<(), QueueEnqueueError> {
        let mut jobs = jobs.peekable();
        if jobs.peek().is_none() {
            return Ok(());
        }
        let destination =
            normalized_output_destination(output).map_err(QueueEnqueueError::Output)?;
        for job in jobs {
            if Self::job_destinations(job).any(|other| same_destination(&destination, &other)) {
                return Err(QueueEnqueueError::DestinationReserved(job.id()));
            }
        }
        Ok(())
    }

    pub fn move_waiting(
        &mut self,
        id: JobId,
        direction: WaitingJobMove,
    ) -> Result<bool, QueueMutationError> {
        let index = self.waiting_job_index(id)?;
        let adjacent = match direction {
            WaitingJobMove::Earlier => self.jobs[..index]
                .iter()
                .rposition(|job| matches!(job.state(), QueueJobState::Waiting)),
            WaitingJobMove::Later => self.jobs[index + 1..]
                .iter()
                .position(|job| matches!(job.state(), QueueJobState::Waiting))
                .map(|offset| index + 1 + offset),
        };
        let Some(adjacent) = adjacent else {
            return Ok(false);
        };
        self.jobs.swap(index, adjacent);
        Ok(true)
    }

    pub fn remove_waiting(&mut self, id: JobId) -> bool {
        let Some(index) = self
            .jobs
            .iter()
            .position(|job| job.id() == id && matches!(job.state(), QueueJobState::Waiting))
        else {
            return false;
        };
        self.jobs.remove(index);
        if self.active_job.is_none() && self.waiting_jobs().next().is_none() {
            self.run_state = QueueRunState::Stopped;
        }
        true
    }

    pub fn clear_finished(&mut self) -> usize {
        let previous_len = self.jobs.len();
        self.jobs.retain(|job| !job.state().is_terminal());
        previous_len - self.jobs.len()
    }

    fn start_with_launcher<F, P>(
        &mut self,
        tools: &MediaTools,
        capabilities_ready: bool,
        launcher: &mut F,
    ) -> bool
    where
        F: FnMut(JobId, MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if !self.can_start(capabilities_ready) {
            return false;
        }

        self.run_state = QueueRunState::Running;
        self.advance_with_launcher(tools, capabilities_ready, launcher);
        // Even if every launch fails, the waiting jobs have changed state.
        true
    }

    fn poll_with_launcher<F, P>(
        &mut self,
        tools: Option<&MediaTools>,
        capabilities_ready: bool,
        launcher: &mut F,
    ) where
        F: FnMut(JobId, MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if tools.is_none() {
            self.stop_after_current();
        }
        let Some(active_job) = self.active_job else {
            if let (QueueRunState::Running, Some(tools)) = (self.run_state, tools) {
                self.advance_with_launcher(tools, capabilities_ready, launcher);
            }
            return;
        };

        let execution_state = self.execution.poll();
        if execution_state.is_running() {
            return;
        }

        let terminal_state = match execution_state.clone() {
            MediaExecutionState::Completed { output } => QueueJobState::Completed { output },
            MediaExecutionState::Failed(error) => {
                QueueJobState::Failed(QueueJobFailure::Execution(error))
            }
            MediaExecutionState::Cancelled => QueueJobState::Cancelled,
            MediaExecutionState::Idle | MediaExecutionState::Running { .. } => {
                unreachable!("an active queue job must own a non-idle media execution")
            }
        };
        self.job_mut(active_job).set_state(terminal_state);
        self.active_job = None;
        self.cancellation_pending = false;
        assert!(
            self.execution.reset(),
            "terminal queue execution must reset before advancing"
        );

        if let (QueueRunState::Running, Some(tools)) = (self.run_state, tools) {
            self.advance_with_launcher(tools, capabilities_ready, launcher);
        } else {
            self.run_state = QueueRunState::Stopped;
        }
    }

    fn advance_with_launcher<F, P>(
        &mut self,
        tools: &MediaTools,
        capabilities_ready: bool,
        launcher: &mut F,
    ) where
        F: FnMut(JobId, MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        while let Some(index) = self
            .jobs
            .iter()
            .position(|job| matches!(job.state(), QueueJobState::Waiting))
        {
            let job = &self.jobs[index];
            if !capabilities_ready && job.operation().requires_encoder_capabilities() {
                // Discovery will wake the application when this FIFO head can launch.
                return;
            }
            let job_id = job.id();
            let current_identity = match SourceIdentity::read(job.operation().input()) {
                Ok(identity) => identity,
                Err(error) => {
                    self.jobs[index].set_state(QueueJobState::Failed(
                        QueueJobFailure::SourceUnavailable(error),
                    ));
                    continue;
                }
            };
            if !job.source_identity().is_reliably_same(&current_identity) {
                self.jobs[index].set_state(QueueJobState::Failed(QueueJobFailure::SourceChanged));
                continue;
            }

            let prepared = match job
                .operation()
                .prepare(tools, job.media(), job.collision_policy())
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.jobs[index]
                        .set_state(QueueJobState::Failed(QueueJobFailure::Preparation(error)));
                    continue;
                }
            };
            match self
                .execution
                .start_prepared_with_launcher(prepared, |command| launcher(job_id, command))
            {
                Ok(()) => {
                    let MediaExecutionState::Running { output } = self.execution.state() else {
                        unreachable!("successful launch must own a running execution");
                    };
                    self.jobs[index].set_state(QueueJobState::Running {
                        output: output.clone(),
                    });
                    self.active_job = Some(job_id);
                    return;
                }
                Err(MediaExecutionStartError::Process(error)) => {
                    self.jobs[index]
                        .set_state(QueueJobState::Failed(QueueJobFailure::ProcessStart(error)));
                }
                Err(MediaExecutionStartError::AlreadyRunning) => {
                    unreachable!("queue cannot launch while another media execution is owned")
                }
            }
        }

        self.run_state = QueueRunState::Stopped;
    }

    pub fn can_edit(&self) -> bool {
        self.run_state == QueueRunState::Stopped && self.active_job.is_none()
    }

    fn require_mutation_stopped(&self) -> Result<(), QueueMutationError> {
        if self.can_edit() {
            Ok(())
        } else {
            Err(QueueMutationError::AutomaticAdvancementActive)
        }
    }

    fn waiting_job_index(&self, id: JobId) -> Result<usize, QueueMutationError> {
        let index = self
            .jobs
            .iter()
            .position(|job| job.id() == id)
            .ok_or(QueueMutationError::JobNotFound(id))?;
        if !matches!(self.jobs[index].state(), QueueJobState::Waiting) {
            return Err(QueueMutationError::JobNotWaiting(id));
        }
        Ok(index)
    }

    fn cancel_for_shutdown(&mut self) {
        let _ = self.cancel_queue();
    }

    fn job_mut(&mut self, id: JobId) -> &mut QueuedJob {
        self.jobs
            .iter_mut()
            .find(|job| job.id() == id)
            .expect("active queue job remains retained until it reaches a terminal state")
    }

    pub(crate) fn validate_admission(
        source_identity: &SourceIdentity,
        media: &MediaFile,
        operation: &QueuedOperation,
    ) -> Result<(), QueueEnqueueError> {
        let current = SourceIdentity::read(&media.path).map_err(QueueEnqueueError::Source)?;
        if !source_identity.is_reliably_same(&current) {
            return Err(QueueEnqueueError::SourceChanged);
        }
        // The ordinary case needs one identity read; aliases still require equivalence.
        if operation.input() != media.path {
            let input =
                SourceIdentity::read(operation.input()).map_err(QueueEnqueueError::Source)?;
            if !source_identity.is_reliably_same(&input) {
                return Err(QueueEnqueueError::SourceDoesNotMatchMedia);
            }
        }
        operation
            .validate(media)
            .map_err(QueueEnqueueError::InvalidRequest)?;
        preflight_output_destination(operation.output()).map_err(QueueEnqueueError::Output)
    }

    #[cfg(test)]
    fn temporary_output_path(&self) -> Option<&std::path::Path> {
        self.execution.temporary_output_path()
    }
}

fn same_destination(left: &std::path::Path, right: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
    };

    use crate::{
        audio_extract::model::{AudioExtractFormat, AudioExtractRequest},
        media::{
            container::MediaContainer,
            execution::{MediaExecutionFailure, ProcessControl},
            intake::SourceIdentity,
            model::{AudioStream, MediaFile, MediaStream, StreamKind, VideoStream},
            process::{
                MediaProcess, MediaProcessCommand, MediaProcessEvent, MediaProcessFailure,
                MediaProcessStartError, MediaProcessState, MediaProcessWake, ProcessProgressSource,
            },
            tools::{MediaTools, VideoEncoderCapability},
        },
        queue::model::{
            BatchEnqueueError, JobId, QueueEnqueueError, QueueJobFailure, QueueJobState,
            QueueMutationError, QueueRunState, QueuedOperation, WaitingJobMove,
        },
        remux::model::RemuxRequest,
        upscale::model::{
            UpscaleEncoderSettings, UpscaleRequest, UpscaleTarget, VideoCodec,
            VideoEncoderSelection,
        },
    };

    use super::MediaQueue;

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn job_ids_are_monotonic_and_removal_does_not_reuse_them() {
        let directory = TestDirectory::new("monotonic");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("first.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("second.mkv")),
            )
            .unwrap();
        assert!(queue.remove_waiting(first));
        let third = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("third.mkv")),
            )
            .unwrap();

        assert_eq!((first.get(), second.get(), third.get()), (1, 2, 3));
        assert_eq!(
            queue.jobs().iter().map(|job| job.id()).collect::<Vec<_>>(),
            vec![second, third]
        );
    }

    #[test]
    fn batch_enqueue_is_atomic_ordered_and_keeps_run_state() {
        let directory = TestDirectory::new("batch-atomic");
        let (first_media_identity, first_media) = media(&directory, "first.mkv");
        let (second_media_identity, second_media) = media(&directory, "second.mkv");
        let mut queue = MediaQueue::new();
        let before = queue.run_state();

        let ids = queue
            .enqueue_batch(&[
                (
                    &first_media_identity,
                    &first_media,
                    &remux(&first_media, directory.path().join("first-output.mkv")),
                ),
                (
                    &second_media_identity,
                    &second_media,
                    &remux(&second_media, directory.path().join("second-output.mkv")),
                ),
            ])
            .unwrap();

        assert_eq!(
            ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(queue.run_state(), before);
        assert_eq!(queue.jobs()[0].media().path, first_media.path);
        assert_eq!(queue.jobs()[1].media().path, second_media.path);

        let count = queue.jobs().len();
        let result = queue.enqueue_batch(&[
            (
                &first_media_identity,
                &first_media,
                &remux(&first_media, directory.path().join("third-output.mkv")),
            ),
            (
                &second_media_identity,
                &second_media,
                &remux(
                    &second_media,
                    directory.path().join("missing").join("invalid.mkv"),
                ),
            ),
        ]);
        assert!(matches!(
            result,
            Err(crate::queue::BatchEnqueueError::Item {
                index: 1,
                error: QueueEnqueueError::Output(_)
            })
        ));
        assert_eq!(queue.jobs().len(), count);
        let mismatched = remux(&first_media, directory.path().join("new.mkv"));
        assert!(matches!(
            queue.enqueue_batch(&[(&second_media_identity, &second_media, &mismatched)]),
            Err(crate::queue::BatchEnqueueError::Item {
                index: 0,
                error: QueueEnqueueError::SourceDoesNotMatchMedia
            })
        ));
        assert_eq!(queue.jobs().len(), count);
        fs::write(&second_media.path, b"changed after inspection").unwrap();
        let valid = remux(&first_media, directory.path().join("next.mkv"));
        let stale = remux(&second_media, directory.path().join("stale.mkv"));
        assert_eq!(
            queue.enqueue_batch(&[
                (&first_media_identity, &first_media, &valid),
                (&second_media_identity, &second_media, &stale),
            ]),
            Err(crate::queue::BatchEnqueueError::Item {
                index: 1,
                error: QueueEnqueueError::SourceChanged,
            })
        );
        assert_eq!(queue.jobs().len(), count);
        assert_eq!(queue.run_state(), before);
        let next = queue
            .enqueue(&first_media_identity, &first_media, valid)
            .unwrap();
        assert_eq!(next.get(), 3);
        assert_eq!(queue.jobs()[2].source_identity(), &first_media_identity);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn batch_conflicts_reject_atomically_and_active_reservations_release_on_settlement() {
        let directory = TestDirectory::new("batch-reservation");
        let (identity, source) = media(&directory, "source.mkv");
        let output = directory.path().join("result.mkv");
        let alias = directory.path().join(".").join("result.mkv");
        let request = remux(&source, output.clone());
        let duplicate = remux(&source, alias);
        let mut queue = MediaQueue::new();
        assert_eq!(
            queue.enqueue_batch(&[
                (&identity, &source, &request),
                (&identity, &source, &duplicate)
            ]),
            Err(BatchEnqueueError::DuplicateDestination { index: 1 })
        );
        assert!(queue.jobs().is_empty());
        let active = queue.enqueue(&identity, &source, request.clone()).unwrap();
        assert_eq!(active.get(), 1);
        assert_eq!(
            queue.enqueue_batch(&[(&identity, &source, &duplicate)]),
            Err(BatchEnqueueError::Item {
                index: 0,
                error: QueueEnqueueError::DestinationReserved(active)
            })
        );
        // A pre-existing file makes the active job reserve its selected numbered name too.
        fs::write(&output, b"existing").unwrap();
        let process = FakeProcessControl::running();
        let mut launcher = |_, _| Ok(process.clone());
        let tools = tools(&directory);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        let selected = remux(&source, directory.path().join("result (1).mkv"));
        assert_eq!(
            queue.enqueue_batch(&[(&identity, &source, &selected)]),
            Err(BatchEnqueueError::Item {
                index: 0,
                error: QueueEnqueueError::DestinationReserved(active)
            })
        );
        assert!(queue.stop_after_current());
        let other = remux(&source, directory.path().join("other.mkv"));
        let ids = queue
            .enqueue_batch(&[(&identity, &source, &other)])
            .unwrap();
        assert_eq!(ids[0].get(), 2);
        assert_eq!(queue.run_state(), QueueRunState::StoppingAfterCurrent);
        fs::write(queue.temporary_output_path().unwrap(), b"finished").unwrap();
        process.set_state(MediaProcessState::Completed);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        let history = queue.jobs()[0].clone();
        let ids = queue
            .enqueue_batch(&[(&identity, &source, &request)])
            .unwrap();
        assert_eq!(ids[0].get(), 3);
        assert_eq!(queue.jobs()[0], history);
        assert_eq!(queue.jobs()[1].state(), &QueueJobState::Waiting);
        assert_eq!(queue.jobs()[2].state(), &QueueJobState::Waiting);
    }

    #[test]
    fn job_id_exhaustion_fails_instead_of_wrapping() {
        let directory = TestDirectory::new("id-exhaustion");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        queue.next_job_id = Some(u64::MAX);
        let candidate = remux(&media, directory.path().join("last.mkv"));
        assert_eq!(
            queue.enqueue_batch(&[(&media_identity, &media, &candidate); 2]),
            Err(crate::queue::BatchEnqueueError::JobIdExhausted)
        );
        assert!(queue.jobs().is_empty());
        assert!(queue.enqueue_batch(&[]).unwrap().is_empty());

        let last = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("last.mkv")),
            )
            .unwrap();
        assert_eq!(last.get(), u64::MAX);
        assert_eq!(
            queue.enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("wrapped.mkv"))
            ),
            Err(QueueEnqueueError::JobIdExhausted)
        );
    }

    #[test]
    fn single_admission_rejects_changed_missing_and_unrelated_sources() {
        use crate::media::intake::MediaIntakeError;
        let directory = TestDirectory::new("admission-source");
        let (first_identity, first) = media(&directory, "first.mkv");
        let (second_identity, second) = media(&directory, "second.mkv");
        let output = directory.path().join("result.mkv");
        let mut queue = MediaQueue::new();
        assert_eq!(
            queue.enqueue(&first_identity, &first, remux(&second, output.clone())),
            Err(QueueEnqueueError::SourceDoesNotMatchMedia)
        );
        assert_eq!(
            queue.enqueue(&first_identity, &second, remux(&second, output.clone())),
            Err(QueueEnqueueError::SourceChanged)
        );
        fs::write(&first.path, b"changed after inspection").unwrap();
        assert_eq!(
            queue.enqueue(&first_identity, &first, remux(&first, output.clone())),
            Err(QueueEnqueueError::SourceChanged)
        );
        fs::remove_file(&first.path).unwrap();
        assert_eq!(
            queue.enqueue(&first_identity, &first, remux(&first, output.clone())),
            Err(QueueEnqueueError::Source(MediaIntakeError::Missing))
        );
        assert!(queue.jobs().is_empty());
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        let next = queue
            .enqueue(&second_identity, &second, remux(&second, output))
            .unwrap();
        assert_eq!(next.get(), 1);
        assert_eq!(queue.jobs()[0].source_identity(), &second_identity);
        assert_eq!(queue.jobs()[0].media(), &second);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn admission_preserves_source_path_equivalence_without_staging() {
        let directory = TestDirectory::new("admission-equivalent-source");
        let (identity, media) = media(&directory, "source.mkv");
        let subdir = directory.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        let alias = subdir.join("..").join("source.mkv");
        let operation = QueuedOperation::Remux(RemuxRequest {
            input: alias,
            output: directory.path().join("result.mkv"),
            container: MediaContainer::Matroska,
        });
        let mut queue = MediaQueue::new();
        queue.enqueue(&identity, &media, operation.clone()).unwrap();
        assert_eq!(queue.jobs()[0].operation(), &operation);
        assert_eq!(queue.jobs()[0].source_identity(), &identity);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn waiting_replacement_preserves_identity_snapshot_position_and_job_id() {
        let directory = TestDirectory::new("replace-operation");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("first.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("second.mkv")),
            )
            .unwrap();
        let source_identity = queue.jobs()[0].source_identity().clone();
        let media_snapshot = queue.jobs()[0].media().clone();
        let adjusted_output = directory.path().join("first-adjusted.mkv");
        let replacement_output = directory.path().join("first-audio.m4a");

        queue
            .replace_waiting_operation(first, remux(&media, adjusted_output.clone()))
            .unwrap();
        assert!(matches!(
            queue.jobs()[0].operation(),
            QueuedOperation::Remux(request) if request.output == adjusted_output
        ));

        queue
            .replace_waiting_operation(first, audio_extract(&media, replacement_output.clone()))
            .unwrap();

        assert_eq!(
            queue.jobs().iter().map(|job| job.id()).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(queue.jobs()[0].source_identity(), &source_identity);
        assert_eq!(queue.jobs()[0].media(), &media_snapshot);
        assert!(matches!(
            queue.jobs()[0].operation(),
            QueuedOperation::AudioExtract(request) if request.output == replacement_output
        ));
    }

    #[test]
    fn waiting_job_view_keeps_distinct_jobs_in_insertion_order_and_retained_media() {
        let directory = TestDirectory::new("waiting-view");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let terminal = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("terminal.mkv")),
            )
            .unwrap();
        assert!(queue.cancel_queue());
        let first = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("first.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("second.mkv")),
            )
            .unwrap();

        let waiting = queue.waiting_jobs().collect::<Vec<_>>();
        assert_eq!(
            waiting.iter().map(|job| job.id()).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_ne!(first, second);
        assert_eq!(waiting[0].retained_intake().current(), Some(&media));
        assert_eq!(queue.jobs()[0].id(), terminal);
        assert!(matches!(queue.jobs()[0].state(), QueueJobState::Cancelled));
    }

    #[test]
    fn waiting_replacement_revalidates_source_and_is_atomic_on_failure() {
        let directory = TestDirectory::new("replace-atomic");
        let (source_media_identity, source_media) = media(&directory, "source.mkv");
        let (_, other_media) = media(&directory, "other.mkv");
        let first_output = directory.path().join("first.mkv");
        let second_output = directory.path().join("second.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(
                &source_media_identity,
                &source_media,
                remux(&source_media, first_output.clone()),
            )
            .unwrap();
        let second = queue
            .enqueue(
                &source_media_identity,
                &source_media,
                remux(&source_media, second_output.clone()),
            )
            .unwrap();
        let original_operation = queue.jobs()[0].operation().clone();
        assert_eq!(
            queue.replace_waiting_operation(
                first,
                remux(&other_media, directory.path().join("other-output.mkv"))
            ),
            Err(QueueMutationError::SourceChanged)
        );
        assert_eq!(queue.jobs()[0].operation(), &original_operation);
        assert!(matches!(
            queue.replace_waiting_operation(
                first,
                remux(
                    &source_media,
                    directory.path().join("missing").join("invalid.mkv")
                )
            ),
            Err(QueueMutationError::Output(_))
        ));
        assert_eq!(queue.jobs()[0].operation(), &original_operation);
        assert!(matches!(
            queue.replace_waiting_operation(
                first,
                remux(&source_media, directory.path().join("wrong.mp4"))
            ),
            Err(QueueMutationError::InvalidRequest(_))
        ));
        assert_eq!(queue.jobs()[0].operation(), &original_operation);
        assert_eq!(
            queue.replace_waiting_operation(
                first,
                remux(&source_media, directory.path().join(".").join("second.mkv"))
            ),
            Err(QueueMutationError::DestinationReserved(second))
        );
        assert_eq!(queue.jobs()[0].operation(), &original_operation);
        assert_eq!(queue.jobs()[1].id(), second);

        queue
            .replace_waiting_operation(first, remux(&source_media, first_output))
            .unwrap();
    }

    #[test]
    fn waiting_replacement_rejects_a_source_changed_after_enqueue() {
        let directory = TestDirectory::new("replace-changed-source");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let id = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("first.mkv")),
            )
            .unwrap();
        let original = queue.jobs()[0].operation().clone();
        fs::write(&media.path, b"changed source with a different length").unwrap();

        assert_eq!(
            queue.replace_waiting_operation(
                id,
                remux(&media, directory.path().join("replacement.mkv"))
            ),
            Err(QueueMutationError::SourceChanged)
        );
        assert_eq!(queue.jobs()[0].operation(), &original);
    }

    #[test]
    fn waiting_reorder_moves_only_waiting_jobs_without_recreating_them() {
        let directory = TestDirectory::new("reorder");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let terminal = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("terminal.mkv")),
            )
            .unwrap();
        assert!(queue.cancel_queue());
        let first = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("first.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("second.mkv")),
            )
            .unwrap();
        let third = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("third.mkv")),
            )
            .unwrap();
        let operations = queue
            .jobs()
            .iter()
            .map(|job| (job.id(), job.operation().clone()))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert!(queue.move_waiting(third, WaitingJobMove::Earlier).unwrap());
        assert!(queue.move_waiting(first, WaitingJobMove::Later).unwrap());
        assert!(!queue.move_waiting(second, WaitingJobMove::Later).unwrap());
        assert_eq!(
            queue.jobs().iter().map(|job| job.id()).collect::<Vec<_>>(),
            vec![terminal, third, first, second]
        );
        assert!(matches!(queue.jobs()[0].state(), QueueJobState::Cancelled));
        for job in &queue.jobs()[1..] {
            assert_eq!(job.operation(), &operations[&job.id()]);
        }
    }

    #[test]
    fn terminal_job_does_not_block_reusing_its_destination() {
        let directory = TestDirectory::new("terminal-output");
        let (media_identity, media) = media(&directory, "source.mkv");
        let output = directory.path().join("result.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&media_identity, &media, remux(&media, output.clone()))
            .unwrap();
        assert!(queue.cancel_queue());
        assert!(matches!(queue.jobs()[0].state(), QueueJobState::Cancelled));

        let second = queue
            .enqueue(&media_identity, &media, remux(&media, output))
            .unwrap();

        assert!(second > first);
        assert_eq!(queue.jobs().len(), 2);
        assert!(matches!(queue.jobs()[1].state(), QueueJobState::Waiting));
    }

    #[test]
    fn changed_or_missing_source_fails_job_and_advances_in_order() {
        for missing in [false, true] {
            let directory = TestDirectory::new(if missing { "missing" } else { "changed" });
            let (first_media_identity, first_media) = media(&directory, "first-source.mkv");
            let (second_media_identity, second_media) = media(&directory, "second-source.mkv");
            let mut queue = MediaQueue::new();
            let first = queue
                .enqueue(
                    &first_media_identity,
                    &first_media,
                    remux(&first_media, directory.path().join("first-output.mkv")),
                )
                .unwrap();
            let second = queue
                .enqueue(
                    &second_media_identity,
                    &second_media,
                    remux(&second_media, directory.path().join("second-output.mkv")),
                )
                .unwrap();
            if missing {
                fs::remove_file(&first_media.path).unwrap();
            } else {
                fs::write(&first_media.path, b"changed source bytes").unwrap();
            }
            let launches = Arc::new(AtomicUsize::new(0));
            let launch_count = Arc::clone(&launches);
            let mut launcher = move |_, _| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            };

            assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
            assert_eq!(queue.active_job(), Some(second));
            assert_eq!(launches.load(Ordering::Relaxed), 1);
            assert!(
                matches!(
                    queue.jobs()[0].state(),
                    QueueJobState::Failed(QueueJobFailure::SourceUnavailable(_))
                        if missing
                ) || matches!(
                    queue.jobs()[0].state(),
                    QueueJobState::Failed(QueueJobFailure::SourceChanged) if !missing
                )
            );
            assert_eq!(queue.jobs()[0].id(), first);
        }
    }

    #[test]
    fn launch_rechecks_destination_and_preserves_typed_preparation_failure() {
        use crate::{
            media::output::MediaOutputError,
            queue::model::QueuePreparationError,
            remux::{execution::RemuxPreparationError, validation::RemuxValidationError},
        };

        let directory = TestDirectory::new("launch-preflight");
        let (source_identity, source) = media(&directory, "source.mkv");
        let parent = directory.path().join("removed");
        fs::create_dir(&parent).unwrap();
        let mut queue = MediaQueue::new();
        queue
            .enqueue(
                &source_identity,
                &source,
                remux(&source, parent.join("output.mkv")),
            )
            .unwrap();
        assert!(matches!(
            queue.enqueue(
                &source_identity,
                &source,
                remux(&source, directory.path().join("invalid.mp4"))
            ),
            Err(QueueEnqueueError::InvalidRequest(
                QueuePreparationError::Remux(RemuxPreparationError::Validation(
                    RemuxValidationError::OutputExtensionMismatch { .. }
                ))
            ))
        ));
        let valid = queue
            .enqueue(
                &source_identity,
                &source,
                remux(&source, directory.path().join("valid.mkv")),
            )
            .unwrap();
        fs::remove_dir(&parent).unwrap();

        let mut launches = Vec::new();
        let mut launcher = |id, _| {
            launches.push(id);
            Ok(FakeProcessControl::running())
        };
        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        assert_eq!(launches, vec![valid]);
        assert_eq!(queue.active_job(), Some(valid));
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Failed(QueueJobFailure::Preparation(QueuePreparationError::Remux(
                RemuxPreparationError::Output(MediaOutputError::OutputParentUnavailable {
                    kind: std::io::ErrorKind::NotFound,
                    ..
                })
            )))
        ));
        assert_eq!(queue.jobs().len(), 2);
        assert_eq!(valid.get(), 2);
    }

    #[test]
    fn shared_destinations_publish_distinct_outputs_and_advance() {
        let directory = TestDirectory::new("shared-output");
        let (source_identity, source) = media(&directory, "source.mkv");
        let output = directory.path().join("result.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&source_identity, &source, remux(&source, output.clone()))
            .unwrap();
        let second = queue
            .enqueue(&source_identity, &source, remux(&source, output.clone()))
            .unwrap();
        fs::write(&output, b"external output").unwrap();
        let first_process = FakeProcessControl::running();
        let second_process = FakeProcessControl::running();
        let mut processes = VecDeque::from([first_process.clone(), second_process.clone()]);
        let mut launcher = |_, _| Ok(processes.pop_front().unwrap());
        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        for (id, process, bytes, suffix) in [
            (first, first_process, b"first".as_slice(), 1),
            (second, second_process, b"second".as_slice(), 2),
        ] {
            assert_eq!(queue.active_job(), Some(id));
            let actual = directory.path().join(format!("result ({suffix}).mkv"));
            let job = queue.jobs().iter().find(|job| job.id() == id).unwrap();
            assert_eq!(
                job.state(),
                &QueueJobState::Running {
                    output: actual.clone()
                }
            );
            assert!(!actual.exists());
            fs::write(queue.temporary_output_path().unwrap(), bytes).unwrap();
            process.set_state(MediaProcessState::Completed);
            queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);
            assert_eq!(
                queue
                    .jobs()
                    .iter()
                    .find(|job| job.id() == id)
                    .unwrap()
                    .state(),
                &QueueJobState::Completed {
                    output: actual.clone()
                }
            );
            assert_eq!(fs::read(actual).unwrap(), bytes);
        }
        assert_eq!(fs::read(output).unwrap(), b"external output");
        assert!(!queue.is_running());
    }

    #[test]
    fn settled_outcomes_advance_to_one_next_process() {
        for outcome in [
            Outcome::Success,
            Outcome::Failure,
            Outcome::OutputFailure,
            Outcome::Cancel,
            Outcome::CancelFailure,
        ] {
            let directory = TestDirectory::new(outcome.label());
            let (first_media_identity, first_media) = media(&directory, "first-source.mkv");
            let (second_media_identity, second_media) = media(&directory, "second-source.mkv");
            let mut queue = MediaQueue::new();
            let first = queue
                .enqueue(
                    &first_media_identity,
                    &first_media,
                    remux(&first_media, directory.path().join("first-output.mkv")),
                )
                .unwrap();
            let second = queue
                .enqueue(
                    &second_media_identity,
                    &second_media,
                    remux(&second_media, directory.path().join("second-output.mkv")),
                )
                .unwrap();
            let first_process = FakeProcessControl::deferred_cancellation();
            let first_handle = first_process.clone();
            let second_process = FakeProcessControl::running();
            let mut processes = VecDeque::from([first_process, second_process]);
            let launches = Arc::new(AtomicUsize::new(0));
            let launch_count = Arc::clone(&launches);
            let mut launcher = move |_, _| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(processes.pop_front().expect("one fake per queued start"))
            };

            assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
            assert_eq!(queue.active_job(), Some(first));
            assert!(!queue.start_with_launcher(&tools(&directory), true, &mut launcher));
            assert_eq!(launches.load(Ordering::Relaxed), 1);
            match outcome {
                Outcome::Success => {
                    fs::write(queue.temporary_output_path().unwrap(), b"complete").unwrap();
                    first_handle.set_state(MediaProcessState::Completed);
                }
                Outcome::OutputFailure => first_handle.set_state(MediaProcessState::Completed),
                Outcome::Failure => first_handle.set_state(MediaProcessState::Failed(
                    MediaProcessFailure::Exit(Some(2)),
                )),
                Outcome::Cancel | Outcome::CancelFailure => {
                    assert!(queue.cancel_current());
                    let terminal = if matches!(outcome, Outcome::Cancel) {
                        MediaProcessState::Cancelled
                    } else {
                        MediaProcessState::Failed(MediaProcessFailure::Io(
                            std::io::ErrorKind::Other,
                        ))
                    };
                    first_handle.set_state(terminal);
                }
            }

            queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);

            assert_eq!(queue.active_job(), Some(second));
            assert_eq!(launches.load(Ordering::Relaxed), 2);
            assert!(!queue.cancellation_pending());
            let state = queue.jobs()[0].state();
            match outcome {
                Outcome::Success => assert!(matches!(state, QueueJobState::Completed { .. })),
                Outcome::Failure | Outcome::CancelFailure => assert!(matches!(
                    state,
                    QueueJobState::Failed(QueueJobFailure::Execution(
                        MediaExecutionFailure::Process { .. }
                    ))
                )),
                Outcome::OutputFailure => assert!(matches!(
                    state,
                    QueueJobState::Failed(QueueJobFailure::Execution(
                        MediaExecutionFailure::Output { .. }
                    ))
                )),
                Outcome::Cancel => assert_eq!(state, &QueueJobState::Cancelled),
            }
            assert!(matches!(
                queue.jobs()[1].state(),
                QueueJobState::Running { .. }
            ));
        }
    }

    #[test]
    fn running_and_terminal_jobs_reject_waiting_mutation() {
        let directory = TestDirectory::new("mutation-state");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let running = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("running.mkv")),
            )
            .unwrap();
        let waiting = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("waiting.mkv")),
            )
            .unwrap();
        let first = FakeProcessControl::running();
        let mut processes = VecDeque::from([first.clone(), FakeProcessControl::running()]);
        let mut launcher = |_, _| Ok(processes.pop_front().expect("two launches"));
        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));

        let appended = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("appended.mkv")),
            )
            .unwrap();
        let batched = queue
            .enqueue_batch(&[(
                &media_identity,
                &media,
                &remux(&media, directory.path().join("batched.mkv")),
            )])
            .unwrap();
        assert_eq!(queue.run_state(), QueueRunState::Running);
        assert_eq!(queue.active_job(), Some(running));
        assert_eq!(
            queue.waiting_jobs().map(|job| job.id()).collect::<Vec<_>>(),
            vec![waiting, appended, batched[0]]
        );
        assert_eq!(
            queue.replace_waiting_operation(
                running,
                remux(&media, directory.path().join("replacement.mkv"))
            ),
            Err(QueueMutationError::AutomaticAdvancementActive)
        );
        assert_eq!(
            queue.move_waiting(waiting, WaitingJobMove::Earlier),
            Ok(false)
        );
        assert_eq!(
            queue.move_waiting(running, WaitingJobMove::Later),
            Err(QueueMutationError::JobNotWaiting(running))
        );
        assert!(!queue.remove_waiting(running));
        assert_eq!(
            queue.move_waiting(batched[0], WaitingJobMove::Earlier),
            Ok(true)
        );
        assert_eq!(
            queue.waiting_jobs().map(|job| job.id()).collect::<Vec<_>>(),
            vec![waiting, batched[0], appended]
        );

        assert!(queue.remove_waiting(waiting));
        fs::write(queue.temporary_output_path().unwrap(), b"result").unwrap();
        first.set_state(MediaProcessState::Completed);
        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);
        assert_eq!(queue.active_job(), Some(batched[0]));
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Completed { .. }
        ));
        assert!(queue.cancel_queue());
        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);
        assert_eq!(
            queue.replace_waiting_operation(
                running,
                remux(&media, directory.path().join("terminal-replacement.mkv"))
            ),
            Err(QueueMutationError::JobNotWaiting(running))
        );
        assert_eq!(
            queue.move_waiting(appended, WaitingJobMove::Earlier),
            Err(QueueMutationError::JobNotWaiting(appended))
        );
    }

    #[test]
    fn stop_after_current_completes_active_then_preserves_and_restarts_waiting() {
        let directory = TestDirectory::new("stop-after-current");
        let (first_media_identity, first_media) = media(&directory, "first-source.mkv");
        let (second_media_identity, second_media) = media(&directory, "second-source.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(
                &first_media_identity,
                &first_media,
                remux(&first_media, directory.path().join("first-output.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
                &second_media_identity,
                &second_media,
                remux(&second_media, directory.path().join("second-output.mkv")),
            )
            .unwrap();
        let first_process = FakeProcessControl::running();
        let first_handle = first_process.clone();
        let second_process = FakeProcessControl::running();
        let mut processes = VecDeque::from([first_process, second_process]);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut launcher = move |_, _| {
            launch_count.fetch_add(1, Ordering::Relaxed);
            Ok(processes.pop_front().expect("one fake per queued start"))
        };

        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        assert_eq!(queue.run_state(), QueueRunState::Running);
        assert!(queue.stop_after_current());
        assert_eq!(queue.run_state(), QueueRunState::StoppingAfterCurrent);
        assert!(!queue.stop_after_current());
        fs::write(queue.temporary_output_path().unwrap(), b"complete").unwrap();
        first_handle.set_state(MediaProcessState::Completed);

        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);

        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.active_job(), None);
        assert_eq!(launches.load(Ordering::Relaxed), 1);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Completed { .. }
        ));
        assert!(matches!(queue.jobs()[1].state(), QueueJobState::Waiting));

        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        assert_eq!(queue.active_job(), Some(second));
        assert_eq!(launches.load(Ordering::Relaxed), 2);
        assert_eq!(queue.jobs()[0].id(), first);
    }

    #[test]
    fn stop_after_current_without_an_active_job_stops_and_preserves_waiting() {
        let directory = TestDirectory::new("stop-without-active");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let waiting = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("waiting.mkv")),
            )
            .unwrap();
        queue.run_state = QueueRunState::Running;

        assert!(queue.stop_after_current());

        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.jobs()[0].id(), waiting);
        assert!(matches!(queue.jobs()[0].state(), QueueJobState::Waiting));
    }

    #[test]
    fn cancel_queue_leaves_existing_terminal_history_unchanged() {
        let directory = TestDirectory::new("cancel-terminal-history");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let terminal = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("terminal.mkv")),
            )
            .unwrap();
        queue.jobs[0].set_state(QueueJobState::Failed(QueueJobFailure::SourceChanged));
        let terminal_state = queue.jobs()[0].state().clone();
        let active = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("active.mkv")),
            )
            .unwrap();
        let waiting = queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("waiting.mkv")),
            )
            .unwrap();
        let mut launcher = |_, _| Ok(FakeProcessControl::running());
        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));

        assert!(queue.cancel_queue());
        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);

        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.jobs()[0].id(), terminal);
        assert_eq!(queue.jobs()[0].state(), &terminal_state);
        assert_eq!(queue.jobs()[1].id(), active);
        assert!(matches!(queue.jobs()[1].state(), QueueJobState::Cancelled));
        assert_eq!(queue.jobs()[2].id(), waiting);
        assert!(matches!(queue.jobs()[2].state(), QueueJobState::Cancelled));
    }

    #[test]
    fn process_start_failure_advances_without_owning_two_processes() {
        let directory = TestDirectory::new("launch-failure");
        let (first_media_identity, first_media) = media(&directory, "first-source.mkv");
        let (second_media_identity, second_media) = media(&directory, "second-source.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(
                &first_media_identity,
                &first_media,
                remux(&first_media, directory.path().join("first-output.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
                &second_media_identity,
                &second_media,
                remux(&second_media, directory.path().join("second-output.mkv")),
            )
            .unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempt_count = Arc::clone(&attempts);
        let mut launcher = move |_, _| {
            if attempt_count.fetch_add(1, Ordering::Relaxed) == 0 {
                Err(MediaProcessStartError::ProcessLaunchFailed(
                    std::io::ErrorKind::NotFound,
                ))
            } else {
                Ok(FakeProcessControl::running())
            }
        };

        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        assert_eq!(queue.active_job(), Some(second));
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Failed(QueueJobFailure::ProcessStart(_))
        ));
    }

    #[test]
    fn waiting_controls_follow_order_while_running_stopping_and_settling() {
        use crate::ui::queue::{WaitingAction as Action, WaitingControls, waiting_focus_after};
        let directory = TestDirectory::new("waiting-controls");
        let (identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let enqueue = |queue: &mut MediaQueue, name: &str| {
            queue
                .enqueue(
                    &identity,
                    &media,
                    remux(&media, directory.path().join(name)),
                )
                .unwrap()
        };
        let terminal = enqueue(&mut queue, "terminal.mkv");
        assert!(queue.cancel_queue());
        let active = enqueue(&mut queue, "active.mkv");
        let first = enqueue(&mut queue, "first.mkv");
        let second = enqueue(&mut queue, "second.mkv");
        let third = enqueue(&mut queue, "third.mkv");
        let process = FakeProcessControl::deferred_cancellation();
        let handle = process.clone();
        let mut launched = Vec::new();
        let mut launcher = |id, _| {
            launched.push(id);
            Ok(if id == active {
                process.clone()
            } else {
                FakeProcessControl::running()
            })
        };
        let tools = tools(&directory);
        let stopped = WaitingControls::for_job(&queue, first).unwrap();
        assert!(stopped.allows(Action::Edit));
        assert!(stopped.edit_blocked.is_none());
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert_eq!(queue.active_job(), Some(active));
        assert!(WaitingControls::for_job(&queue, active).is_none());
        assert!(WaitingControls::for_job(&queue, terminal).is_none());
        let first_controls = WaitingControls::for_job(&queue, first).unwrap();
        assert_eq!(
            Action::ALL.map(|action| first_controls.allows(action)),
            [false, true, false, true]
        );
        assert!(
            first_controls
                .edit_blocked
                .unwrap()
                .contains("Stop the queue")
        );
        let third_controls = WaitingControls::for_job(&queue, third).unwrap();
        assert_eq!(
            Action::ALL.map(|action| third_controls.allows(action)),
            [true, false, false, true]
        );
        assert!(queue.move_waiting(third, WaitingJobMove::Earlier).unwrap());
        assert_eq!(
            waiting_focus_after(&queue, third, third_controls.index, Action::Earlier),
            Some((third, Action::Earlier))
        );
        assert!(queue.move_waiting(third, WaitingJobMove::Earlier).unwrap());
        assert_eq!(
            waiting_focus_after(&queue, third, 1, Action::Earlier),
            Some((third, Action::Later))
        );
        assert!(queue.remove_waiting(first));
        assert_eq!(
            waiting_focus_after(&queue, first, 1, Action::Remove),
            Some((second, Action::Remove))
        );
        assert!(WaitingControls::for_job(&queue, first).is_none());
        assert!(queue.move_waiting(first, WaitingJobMove::Later).is_err());
        assert!(!queue.remove_waiting(active));
        assert!(
            queue
                .move_waiting(terminal, WaitingJobMove::Earlier)
                .is_err()
        );
        assert_eq!(queue.run_state(), QueueRunState::Running);
        assert!(queue.stop_after_current());
        assert!(queue.move_waiting(second, WaitingJobMove::Earlier).unwrap());
        assert!(queue.remove_waiting(third));
        assert_eq!(
            waiting_focus_after(&queue, third, 1, Action::Remove),
            Some((second, Action::Remove))
        );
        assert_eq!(queue.run_state(), QueueRunState::StoppingAfterCurrent);
        assert!(queue.cancel_current());
        assert!(
            WaitingControls::for_job(&queue, second)
                .unwrap()
                .edit_blocked
                .unwrap()
                .contains("settle before editing")
        );
        handle.set_state(MediaProcessState::Cancelled);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        let remaining = WaitingControls::for_job(&queue, second).unwrap();
        assert_eq!(
            Action::ALL.map(|action| remaining.allows(action)),
            [false, false, true, true]
        );
        assert!(remaining.edit_blocked.is_none());
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert_eq!(queue.active_job(), Some(second));
        assert_eq!(launched, [active, second]);
        assert_eq!(queue.jobs()[0].id(), terminal);
        assert_eq!(queue.jobs()[0].state(), &QueueJobState::Cancelled);
        assert_eq!(queue.jobs()[1].id(), active);
        assert_eq!(queue.jobs()[1].state(), &QueueJobState::Cancelled);
        assert_eq!(waiting_focus_after(&queue, second, 0, Action::Remove), None);
    }

    #[test]
    fn scheduler_controls_follow_owned_job_across_cancel_stop_and_restart() {
        use crate::{
            app::QueueAction,
            ui::queue::{Controls, job_progress},
        };
        let directory = TestDirectory::new("queue-control-transitions");
        let (identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let first_id = queue
            .enqueue(
                &identity,
                &media,
                remux(&media, directory.path().join("first.mkv")),
            )
            .unwrap();
        let second_id = queue
            .enqueue(
                &identity,
                &media,
                remux(&media, directory.path().join("second.mkv")),
            )
            .unwrap();
        let first = FakeProcessControl::deferred_cancellation();
        let first_handle = first.clone();
        let second = FakeProcessControl::deferred_cancellation();
        let second_handle = second.clone();
        let mut processes = VecDeque::from([first, second]);
        let mut launcher = |_, _| Ok(processes.pop_front().expect("one launch per job"));
        let tools = tools(&directory);
        let controls = |queue: &MediaQueue| Controls::new(queue, true, true);
        assert!(controls(&queue).allows(QueueAction::Start, None));
        assert!(!Controls::new(&queue, false, true).allows(QueueAction::Start, None));
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert!(controls(&queue).allows(QueueAction::StopAfterCurrent, None));
        assert!(controls(&queue).allows(QueueAction::CancelCurrent, Some(first_id)));
        assert!(!controls(&queue).allows(QueueAction::CancelCurrent, Some(second_id)));
        first_handle.progress.lock().unwrap().extend([
            "out_time_us=5000000".to_owned(),
            "progress=continue".to_owned(),
        ]);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(
            job_progress(&queue, first_id).and_then(|progress| progress.fraction),
            Some(0.5)
        );
        assert!(job_progress(&queue, second_id).is_none());
        assert!(queue.stop_after_current());
        assert_eq!(controls(&queue).primary_label, "Stopping after current");
        assert!(queue.cancel_current());
        assert!(!controls(&queue).allows(QueueAction::CancelCurrent, Some(first_id)));
        assert!(!controls(&queue).allows(QueueAction::CancelQueue, None));
        assert!(!controls(&queue).allows(QueueAction::Start, None));
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.active_job(), Some(first_id));
        assert_eq!(
            job_progress(&queue, first_id).and_then(|progress| progress.fraction),
            Some(0.5)
        );
        first_handle.set_state(MediaProcessState::Cancelled);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert!(controls(&queue).allows(QueueAction::Start, None));
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert!(job_progress(&queue, first_id).is_none());
        assert!(job_progress(&queue, second_id).is_none()); // no measured progress yet
        assert!(!controls(&queue).allows(QueueAction::CancelCurrent, Some(first_id)));
        assert!(controls(&queue).allows(QueueAction::CancelCurrent, Some(second_id)));
        assert!(queue.cancel_queue());
        assert_eq!(controls(&queue).primary_label, "Cancelling");
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert!(!controls(&queue).allows(QueueAction::Start, None));
        assert!(!controls(&queue).allows(QueueAction::CancelQueue, None));
        second_handle.set_state(MediaProcessState::Cancelled);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(controls(&queue).primary_label, "Start queue");
        assert!(!controls(&queue).allows(QueueAction::Start, None));
        assert_eq!(queue.pending_count(), 0);
    }

    #[test]
    fn cancel_queue_cancels_active_and_waiting_jobs_without_advancing() {
        let directory = TestDirectory::new("stop");
        let (first_media_identity, first_media) = media(&directory, "first-source.mkv");
        let (second_media_identity, second_media) = media(&directory, "second-source.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(
                &first_media_identity,
                &first_media,
                remux(&first_media, directory.path().join("first-output.mkv")),
            )
            .unwrap();
        queue
            .enqueue(
                &second_media_identity,
                &second_media,
                remux(&second_media, directory.path().join("second-output.mkv")),
            )
            .unwrap();
        let process = FakeProcessControl::deferred_cancellation();
        let handle = process.clone();
        let mut process = Some(process);
        let mut launcher = move |_, _| Ok(process.take().expect("only one launch"));

        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        let active = queue.active_job().unwrap();
        assert!(queue.cancel_queue());
        assert!(!queue.cancel_queue());
        assert!(queue.cancellation_pending());
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.jobs()[1].state(), &QueueJobState::Cancelled);
        assert!(!queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);
        assert_eq!(queue.active_job(), Some(active));
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Running { .. }
        ));

        handle.set_state(MediaProcessState::Cancelled);
        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);

        assert!(!queue.cancellation_pending());
        assert!(!queue.is_started());
        assert!(!queue.is_running());
        assert!(
            queue
                .jobs()
                .iter()
                .all(|job| matches!(job.state(), QueueJobState::Cancelled))
        );
    }

    #[test]
    fn empty_start_is_unchanged_and_all_launch_failures_are_reported_as_changes() {
        let directory = TestDirectory::new("all-launches-fail");
        let (identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let tools = tools(&directory);
        let mut attempts = 0;
        let mut launcher = |_, _| -> Result<FakeProcessControl, _> {
            attempts += 1;
            Err(MediaProcessStartError::ProcessLaunchFailed(
                std::io::ErrorKind::NotFound,
            ))
        };
        assert!(!queue.start_with_launcher(&tools, true, &mut launcher));
        for name in ["first.mkv", "second.mkv"] {
            queue
                .enqueue(
                    &identity,
                    &media,
                    remux(&media, directory.path().join(name)),
                )
                .unwrap();
        }
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert!(!queue.start_with_launcher(&tools, true, &mut launcher));
        assert_eq!(attempts, 2);
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert!(queue.active_job().is_none());
        assert!(queue.jobs().iter().all(|job| matches!(
            job.state(),
            QueueJobState::Failed(QueueJobFailure::ProcessStart(_))
        )));
    }

    #[test]
    fn cancelling_after_stop_preserves_waiting_work_until_explicit_restart() {
        let directory = TestDirectory::new("cancel-after-stop");
        let (identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        for name in ["first.mkv", "second.mkv"] {
            queue
                .enqueue(
                    &identity,
                    &media,
                    remux(&media, directory.path().join(name)),
                )
                .unwrap();
        }
        let first = FakeProcessControl::deferred_cancellation();
        let handle = first.clone();
        let mut processes = VecDeque::from([first, FakeProcessControl::running()]);
        let mut launcher = |_, _| Ok(processes.pop_front().expect("one process per job"));
        let tools = tools(&directory);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        let active = queue.active_job();
        assert!(queue.stop_after_current());
        assert!(queue.cancel_current());
        assert!(!queue.cancel_current());
        assert!(queue.cancellation_pending());
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.active_job(), active);
        assert!(!queue.start_with_launcher(&tools, true, &mut launcher));
        let waiting = queue.jobs()[1].id();
        assert_eq!(
            queue.replace_waiting_operation(
                waiting,
                remux(&media, directory.path().join("edit.mkv"))
            ),
            Err(QueueMutationError::AutomaticAdvancementActive)
        );
        handle.set_state(MediaProcessState::Cancelled);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.jobs()[0].state(), &QueueJobState::Cancelled);
        assert_eq!(queue.jobs()[1].state(), &QueueJobState::Waiting);
        assert!(queue.can_edit());
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert_eq!(queue.active_job(), Some(queue.jobs()[1].id()));
        assert!(!queue.cancellation_pending());
    }

    #[test]
    fn readiness_loss_settles_active_output_without_advancing_even_if_tools_recover() {
        let directory = TestDirectory::new("tools-unavailable");
        let (identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let output = directory.path().join("first.mkv");
        queue
            .enqueue(&identity, &media, remux(&media, output.clone()))
            .unwrap();
        queue
            .enqueue(
                &identity,
                &media,
                remux(&media, directory.path().join("second.mkv")),
            )
            .unwrap();
        let process = FakeProcessControl::running();
        let handle = process.clone();
        let mut process = Some(process);
        let mut launcher = |_, _| Ok(process.take().expect("must not advance"));
        let tools = tools(&directory);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        let active = queue.active_job();
        queue.poll_with_launcher(None, true, &mut launcher);
        assert_eq!(queue.active_job(), active);
        assert_eq!(queue.run_state(), QueueRunState::StoppingAfterCurrent);
        assert!(!queue.cancellation_pending());
        fs::write(queue.temporary_output_path().unwrap(), b"complete").unwrap();
        handle.set_state(MediaProcessState::Completed);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.active_job(), None);
        assert_eq!(
            queue.jobs()[0].state(),
            &QueueJobState::Completed {
                output: output.clone()
            }
        );
        assert_eq!(queue.jobs()[1].state(), &QueueJobState::Waiting);
        assert_eq!(fs::read(output).unwrap(), b"complete");
    }

    #[test]
    fn bulk_cancellation_does_not_overwrite_a_completed_process_awaiting_settlement() {
        let directory = TestDirectory::new("cancel-terminal-claim");
        let (identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let output = directory.path().join("first.mkv");
        queue
            .enqueue(&identity, &media, remux(&media, output.clone()))
            .unwrap();
        let process = FakeProcessControl::running();
        let handle = process.clone();
        let mut process = Some(process);
        let mut launcher = |_, _| Ok(process.take().expect("only one launch"));
        let tools = tools(&directory);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        fs::write(queue.temporary_output_path().unwrap(), b"complete").unwrap();
        handle.set_state(MediaProcessState::Completed);
        assert!(!queue.cancel_current());
        // Even without waiting jobs, stopping advancement is a state change.
        assert!(queue.cancel_queue());
        assert!(!queue.cancel_queue());
        assert!(!queue.cancellation_pending());
        queue.poll_with_launcher(None, true, &mut launcher);
        assert_eq!(
            queue.jobs()[0].state(),
            &QueueJobState::Completed {
                output: output.clone()
            }
        );
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(fs::read(output).unwrap(), b"complete");
    }

    #[test]
    fn real_child_cancellation_settles_staged_output_without_touching_existing_file() {
        use std::time::Duration;
        let directory = TestDirectory::new("cancel-real-child");
        let (identity, media) = media(&directory, "source.mkv");
        let output = directory.path().join("existing.mkv");
        fs::write(&output, b"existing").unwrap();
        let mut queue = MediaQueue::new();
        queue
            .enqueue(&identity, &media, remux(&media, output.clone()))
            .unwrap();
        #[cfg(windows)]
        let command = MediaProcessCommand::new("cmd.exe").args([
            "/D",
            "/S",
            "/C",
            "echo ready& for /L %i in (1,1,2147483647) do @rem",
        ]);
        #[cfg(not(windows))]
        let command =
            MediaProcessCommand::new("sh").args(["-c", "printf 'ready\\n'; exec sleep 60"]);
        let (sender, receiver) = mpsc::channel();
        let wake = MediaProcessWake::new(move |event| {
            let _ = sender.send(event);
        });
        let mut launcher = |_, _| {
            MediaProcess::start_with_wake(
                command
                    .clone()
                    .progress_source(ProcessProgressSource::Stdout),
                wake.clone(),
            )
        };
        let tools = tools(&directory);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
            MediaProcessEvent::Progress
        );
        let temporary = queue.temporary_output_path().unwrap().to_path_buf();
        let staging = temporary.parent().unwrap();
        fs::write(&temporary, b"partial").unwrap();
        assert!(queue.cancel_current());
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
            MediaProcessEvent::Terminal
        );
        assert!(temporary.exists());
        assert!(queue.cancellation_pending());
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.jobs()[0].state(), &QueueJobState::Cancelled);
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert!(!staging.exists());
        assert_eq!(fs::read(output).unwrap(), b"existing");
        assert!(!directory.path().join("existing (1).mkv").exists());
    }

    #[test]
    fn migrated_operations_share_publication_and_cancellation_settlement() {
        use crate::audio_mix::draft::AudioMixDraft;

        let directory = TestDirectory::new("migrated-settlement");
        let (identity, media) = video_media(&directory);
        let mut audio = AudioMixDraft::new(&media);
        audio.set_custom_output_path(directory.path().join("mix.mp4"));
        for operation in [
            QueuedOperation::AudioMix(audio.request(&media)),
            remux(&media, directory.path().join("remux.mkv")),
            audio_extract(&media, directory.path().join("extract.m4a")),
            QueuedOperation::FrameCapture(crate::frame_capture::model::FrameCaptureRequest {
                input: media.path.clone(),
                output: directory.path().join("capture.png"),
                position: crate::frame_capture::model::FrameTimestamp::from_milliseconds(100)
                    .into(),
                format: crate::frame_capture::model::FrameCaptureFormat::Png,
            }),
            upscale(
                &media,
                directory.path().join("upscale.mp4"),
                VideoEncoderSelection::Cpu,
                VideoEncoderCapability::EncoderNotCompiled,
            ),
        ] {
            let output = operation.output().to_path_buf();
            let mut queue = MediaQueue::new();
            queue.enqueue(&identity, &media, operation.clone()).unwrap();
            queue.enqueue(&identity, &media, operation.clone()).unwrap();
            queue.enqueue(&identity, &media, operation).unwrap();
            let first = FakeProcessControl::running();
            let second = FakeProcessControl::deferred_cancellation();
            let third = FakeProcessControl::running();
            let mut processes = VecDeque::from([first.clone(), second.clone(), third.clone()]);
            let mut launch = |_, _| Ok(processes.pop_front().expect("one launch per job"));
            let tools = tools(&directory);
            assert!(queue.start_with_launcher(&tools, true, &mut launch));
            fs::write(queue.temporary_output_path().unwrap(), b"published").unwrap();
            first.set_state(MediaProcessState::Completed);
            queue.poll_with_launcher(Some(&tools), true, &mut launch);
            assert_eq!(
                queue.jobs()[0].state(),
                &QueueJobState::Completed {
                    output: output.clone()
                }
            );
            let temporary = queue.temporary_output_path().unwrap().to_path_buf();
            fs::write(&temporary, b"partial").unwrap();
            assert!(queue.cancel_current());
            queue.poll_with_launcher(Some(&tools), true, &mut launch);
            assert!(temporary.exists());
            assert!(queue.cancellation_pending());
            second.set_state(MediaProcessState::Cancelled);
            queue.poll_with_launcher(Some(&tools), true, &mut launch);
            assert_eq!(queue.jobs()[1].state(), &QueueJobState::Cancelled);
            assert!(!temporary.exists());
            let failed_output = queue.temporary_output_path().unwrap().to_path_buf();
            fs::write(&failed_output, b"partial").unwrap();
            third.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
                1,
            ))));
            queue.poll_with_launcher(Some(&tools), true, &mut launch);
            assert!(matches!(queue.jobs()[2].state(), QueueJobState::Failed(_)));
            assert!(!failed_output.exists());
            assert_eq!(queue.run_state(), QueueRunState::Stopped);
            assert_eq!(fs::read(output).unwrap(), b"published");
        }
    }

    #[test]
    fn mixed_fifo_waits_for_encoder_discovery_and_preserves_extraction_formats() {
        use crate::audio_mix::draft::AudioMixDraft;
        let directory = TestDirectory::new("mixed-fifo-capabilities");
        let (identity, mut media) = video_media(&directory);
        media.audio_streams[0].title = Some("Program".to_owned());
        media.audio_streams[0].language = Some("eng".to_owned());
        let mut mix = AudioMixDraft::new(&media);
        mix.set_custom_output_path(directory.path().join("mix.mp4"));
        let mut operations = vec![
            upscale(
                &media,
                directory.path().join("cpu.mp4"),
                VideoEncoderSelection::Cpu,
                VideoEncoderCapability::Available,
            ),
            upscale(
                &media,
                directory.path().join("auto.mp4"),
                VideoEncoderSelection::Auto,
                VideoEncoderCapability::EncoderNotCompiled,
            ),
            QueuedOperation::AudioMix(mix.request(&media)),
            remux(&media, directory.path().join("remux.mkv")),
        ];
        for format in [
            AudioExtractFormat::Original,
            AudioExtractFormat::Aac,
            AudioExtractFormat::Flac,
        ] {
            operations.push(QueuedOperation::AudioExtract(AudioExtractRequest {
                input: media.path.clone(),
                output: directory
                    .path()
                    .join(format!("extract.{}", format.extension())),
                audio_stream_index: 1,
                format,
            }));
        }
        use crate::frame_capture::model::{
            FrameCaptureFormat, FrameCaptureRequest, FrameTimeBase, ResolvedFramePosition,
        };
        operations.push(QueuedOperation::FrameCapture(FrameCaptureRequest {
            input: media.path.clone(),
            output: directory.path().join("frame.jpg"),
            position: ResolvedFramePosition::new(1001, FrameTimeBase::new(1, 24_000).unwrap())
                .unwrap()
                .into(),
            format: FrameCaptureFormat::Jpeg,
        }));
        let mut queue = MediaQueue::new();
        let ids: Vec<_> = operations
            .iter()
            .map(|operation| queue.enqueue(&identity, &media, operation.clone()).unwrap())
            .collect();
        let processes: Vec<_> = ids.iter().map(|_| FakeProcessControl::running()).collect();
        let mut launches = Vec::new();
        let mut launcher = |id, command: MediaProcessCommand| {
            let index = launches.len();
            assert_eq!(id, ids[index]);
            launches.push(command.arguments().to_vec());
            Ok(processes[index].clone())
        };
        let mut tools = tools(&directory);
        // CPU launches before optional encoder discovery settles, regardless of retained NVENC data.
        assert!(queue.start_with_launcher(&tools, false, &mut launcher));
        assert_eq!(queue.active_job(), Some(ids[0]));
        fs::write(queue.temporary_output_path().unwrap(), b"cpu").unwrap();
        processes[0].set_state(MediaProcessState::Completed);
        queue.poll_with_launcher(Some(&tools), false, &mut launcher);
        assert_eq!(queue.active_job(), None);
        assert_eq!(queue.run_state(), QueueRunState::Running);
        assert_eq!(queue.jobs()[1].state(), &QueueJobState::Waiting);
        assert!(!queue.start_with_launcher(&tools, false, &mut launcher));
        queue.poll_with_launcher(Some(&tools), false, &mut launcher);
        assert_eq!(queue.active_job(), None);

        // Settlement wakes the queue; Auto resolves against the new capability, not its admission snapshot.
        tools.video_encoders.h264_nvenc = VideoEncoderCapability::Available;
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        for (index, operation) in operations.iter().enumerate().skip(1) {
            assert_eq!(queue.active_job(), Some(ids[index]));
            assert_eq!(queue.jobs()[index].operation(), operation);
            fs::write(queue.temporary_output_path().unwrap(), b"published").unwrap();
            processes[index].set_state(MediaProcessState::Completed);
            queue.poll_with_launcher(Some(&tools), true, &mut launcher);
            assert_eq!(
                queue.jobs()[index].state(),
                &QueueJobState::Completed {
                    output: operation.output().to_path_buf()
                }
            );
            assert_eq!(fs::read(operation.output()).unwrap(), b"published");
        }
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.active_job(), None);
        assert_eq!(launches.len(), ids.len());
        assert!(
            launches[0]
                .windows(2)
                .any(|pair| pair == ["-c:v", "libx264"])
        );
        assert!(
            launches[1]
                .windows(2)
                .any(|pair| pair == ["-c:v", "h264_nvenc"])
        );
        for (args, (codec, title_flag, language_flag)) in launches[4..7].iter().zip([
            ("copy", "-metadata:s:a:0", "-metadata:s:a:0"),
            ("aac", "-metadata", "-metadata:s:a:0"),
            ("flac", "-metadata", "-metadata"),
        ]) {
            assert!(args.windows(2).any(|pair| pair == ["-map", "0:1"]));
            assert!(args.windows(2).any(|pair| pair == ["-c:a", codec]));
            assert!(
                args.windows(2)
                    .any(|pair| pair == [title_flag, "title=Program"])
            );
            assert!(
                args.windows(2)
                    .any(|pair| pair == [language_flag, "language=eng"])
            );
        }
        assert!(
            launches[7]
                .windows(2)
                .any(|pair| pair == ["-ss", "0.041708333"])
        );
        assert!(launches[7].windows(2).any(|pair| pair == ["-c:v", "mjpeg"]));
    }

    #[test]
    fn explicit_nvenc_revalidates_at_launch_and_auto_falls_back_to_current_cpu() {
        let directory = TestDirectory::new("upscale-capability-revalidation");
        let (identity, media) = video_media(&directory);
        let mut queue = MediaQueue::new();
        for (file, selection) in [
            ("explicit.mp4", VideoEncoderSelection::NvidiaNvenc),
            ("auto.mp4", VideoEncoderSelection::Auto),
        ] {
            queue
                .enqueue(
                    &identity,
                    &media,
                    upscale(
                        &media,
                        directory.path().join(file),
                        selection,
                        VideoEncoderCapability::Available,
                    ),
                )
                .unwrap();
        }
        let mut tools = tools(&directory);
        let reason = "new tool identity cannot access the NVENC driver";
        tools.video_encoders.h264_nvenc = VideoEncoderCapability::RuntimeUnavailable {
            reason: reason.to_owned(),
        };
        let mut launcher = |id: JobId, command: MediaProcessCommand| {
            assert_eq!(id.get(), 2, "unavailable explicit encoder must not launch");
            assert!(
                command
                    .arguments()
                    .windows(2)
                    .any(|pair| pair == ["-c:v", "libx264"])
            );
            Ok(FakeProcessControl::running())
        };
        assert!(!queue.start_with_launcher(&tools, false, &mut launcher));
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.jobs()[0].state(), &QueueJobState::Waiting);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        let QueueJobState::Failed(QueueJobFailure::Preparation(error)) = queue.jobs()[0].state()
        else {
            panic!(
                "explicit encoder should fail preparation: {:?}",
                queue.jobs()[0].state()
            );
        };
        assert!(error.to_string().contains(reason));
        assert!(!directory.path().join("explicit.mp4").exists());
        assert_eq!(queue.active_job(), Some(queue.jobs()[1].id()));
    }

    #[test]
    fn stop_or_cancel_while_waiting_for_capabilities_never_launches_after_discovery() {
        for cancel in [false, true] {
            let directory = TestDirectory::new("pending-capabilities-stop");
            let (identity, media) = video_media(&directory);
            let mut queue = MediaQueue::new();
            let first = queue
                .enqueue(
                    &identity,
                    &media,
                    remux(&media, directory.path().join("remux.mkv")),
                )
                .unwrap();
            let second = queue
                .enqueue(
                    &identity,
                    &media,
                    upscale(
                        &media,
                        directory.path().join("auto.mp4"),
                        VideoEncoderSelection::Auto,
                        VideoEncoderCapability::Available,
                    ),
                )
                .unwrap();
            let process = FakeProcessControl::running();
            let mut launcher = |id, _| {
                assert_eq!(
                    id, first,
                    "stopped or cancelled queue must not launch again"
                );
                Ok(process.clone())
            };
            let tools = tools(&directory);
            assert!(queue.start_with_launcher(&tools, false, &mut launcher));
            fs::write(queue.temporary_output_path().unwrap(), b"remux").unwrap();
            process.set_state(MediaProcessState::Completed);
            queue.poll_with_launcher(Some(&tools), false, &mut launcher);
            assert!(queue.is_started());
            assert!(queue.active_job().is_none());
            if cancel {
                assert!(queue.cancel_queue());
            } else {
                assert!(queue.stop_after_current());
            }
            queue.poll_with_launcher(Some(&tools), true, &mut launcher);
            assert_eq!(queue.run_state(), QueueRunState::Stopped);
            assert_eq!(queue.jobs()[1].id(), second);
            assert_eq!(
                queue.jobs()[1].state(),
                &if cancel {
                    QueueJobState::Cancelled
                } else {
                    QueueJobState::Waiting
                }
            );
        }
    }

    #[test]
    fn successful_output_is_published_once_and_terminal_job_remains_until_cleared() {
        let directory = TestDirectory::new("publish-once");
        let (media_identity, media) = media(&directory, "source.mkv");
        let output = directory.path().join("output.mkv");
        let mut queue = MediaQueue::new();
        let label = |queue: &MediaQueue| crate::ui::queue_count_label(queue.pending_count());
        assert_eq!(label(&queue), "Queue");
        queue
            .enqueue(&media_identity, &media, remux(&media, output.clone()))
            .unwrap();
        assert_eq!(label(&queue), "Queue (1)"); // Waiting only.
        let process = FakeProcessControl::running();
        let handle = process.clone();
        let mut process = Some(process);
        let mut launcher = move |_, _| Ok(process.take().expect("one process launch"));
        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));
        assert!(queue.active_job().is_some());
        assert_eq!(label(&queue), "Queue (1)"); // Active only.
        fs::write(queue.temporary_output_path().unwrap(), b"published").unwrap();
        handle.set_state(MediaProcessState::Completed);

        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);
        queue.poll_with_launcher(Some(&tools(&directory)), true, &mut launcher);

        assert_eq!(fs::read(&output).unwrap(), b"published");
        assert_eq!(queue.jobs().len(), 1);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Completed { output: completed } if completed == &output
        ));
        assert_eq!(label(&queue), "Queue"); // Terminal history only.
        assert_eq!(queue.clear_finished(), 1);
        assert!(queue.jobs().is_empty());
    }

    #[test]
    fn explicit_history_cleanup_preserves_pending_jobs_and_published_files() {
        let directory = TestDirectory::new("explicit-history");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        for index in 0..5 {
            queue
                .enqueue(
                    &media_identity,
                    &media,
                    remux(&media, directory.path().join(format!("output-{index}.mkv"))),
                )
                .unwrap();
        }
        queue.jobs[0].set_state(QueueJobState::Completed {
            output: directory.path().join("output-0.mkv"),
        });
        queue.jobs[1].set_state(QueueJobState::Cancelled);
        queue.jobs[2].set_state(QueueJobState::Failed(QueueJobFailure::SourceChanged));
        queue.jobs[3].set_state(QueueJobState::Running {
            output: directory.path().join("output-3.mkv"),
        });

        let published = directory.path().join("output-0.mkv");
        fs::write(&published, b"published").unwrap();
        assert_eq!(queue.pending_count(), 2);
        assert_eq!(queue.clear_finished(), 3);
        assert_eq!(queue.jobs().len(), 2);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Running { .. }
        ));
        assert!(matches!(queue.jobs()[1].state(), QueueJobState::Waiting));
        assert_eq!(queue.pending_count(), 2);
        assert_eq!(fs::read(&published).unwrap(), b"published");
        assert_eq!(queue.clear_finished(), 0);
    }

    #[test]
    fn dropping_queue_requests_cancellation_from_the_active_process_owner() {
        let directory = TestDirectory::new("drop-cancel");
        let (media_identity, media) = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(
                &media_identity,
                &media,
                remux(&media, directory.path().join("output.mkv")),
            )
            .unwrap();
        let process = FakeProcessControl::running();
        let handle = process.clone();
        let mut process = Some(process);
        let mut launcher = move |_, _| Ok(process.take().expect("one process launch"));
        assert!(queue.start_with_launcher(&tools(&directory), true, &mut launcher));

        drop(queue);

        assert_eq!(handle.state(), MediaProcessState::Cancelled);
    }

    #[test]
    fn retry_retains_history_and_reserves_destination_until_removed_or_settled() {
        use crate::queue::BatchEnqueueError;
        let directory = TestDirectory::new("retry-reservations");
        let (identity, source) = media(&directory, "source.mkv");
        let output = directory.path().join("retry.mkv");
        let request = remux(&source, output.clone());
        let mut queue = MediaQueue::new();
        let original = queue.enqueue(&identity, &source, request.clone()).unwrap();
        assert!(queue.cancel_queue());
        let history = queue.jobs()[0].clone();
        let retry = queue.retry(original, None).unwrap();
        assert_eq!(retry.get(), original.get() + 1);
        assert_eq!(queue.jobs()[0], history);
        assert_eq!(queue.jobs()[1].state(), &QueueJobState::Waiting);
        assert_eq!(queue.jobs()[1].source_identity(), &identity);
        assert_eq!(queue.jobs()[1].operation(), &request);
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        let snapshot = queue.jobs().to_vec();
        let next = queue.next_job_id;
        assert_eq!(
            queue.retry(original, None),
            Err(QueueMutationError::DestinationReserved(retry))
        );
        assert_eq!(
            queue.retry(retry, None),
            Err(QueueMutationError::JobNotRetryable(retry))
        );
        assert_eq!(
            queue.retry(JobId::new(999), None),
            Err(QueueMutationError::JobNotFound(JobId::new(999)))
        );
        assert_eq!(
            queue.enqueue(&identity, &source, request.clone()),
            Err(QueueEnqueueError::DestinationReserved(retry))
        );
        let other = remux(&source, directory.path().join("other.mkv"));
        assert_eq!(
            queue.enqueue_batch(&[(&identity, &source, &other), (&identity, &source, &request)]),
            Err(BatchEnqueueError::Item {
                index: 1,
                error: QueueEnqueueError::DestinationReserved(retry)
            })
        );
        assert_eq!(queue.jobs(), snapshot);
        assert_eq!(queue.next_job_id, next);
        assert!(queue.remove_waiting(retry));
        let retry = queue.retry(original, None).unwrap();
        assert_eq!(Some(retry.get()), next);
        let process = FakeProcessControl::running();
        let mut launcher = |_, _| Ok(process.clone());
        let tools = tools(&directory);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert_eq!(queue.active_job(), Some(retry));
        fs::write(queue.temporary_output_path().unwrap(), b"result").unwrap();
        process.set_state(MediaProcessState::Completed);
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert_eq!(queue.jobs()[0], history);
        assert_eq!(
            queue.jobs()[1].state(),
            &QueueJobState::Completed {
                output: output.clone()
            }
        );
        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(fs::read(&output).unwrap(), b"result");
        assert!(matches!(
            queue.retry(original, None),
            Err(QueueMutationError::Output(
                crate::media::output::MediaOutputError::DestinationOccupied(_)
            ))
        ));
        assert_eq!(
            queue.retry(retry, None),
            Err(QueueMutationError::JobNotRetryable(retry))
        );
        assert_eq!(queue.next_job_id, retry.get().checked_add(1));
    }

    #[test]
    fn retry_revalidates_source_and_current_encoder_without_allocating_on_rejection() {
        let directory = TestDirectory::new("retry-source-encoder");
        let (identity, source) = video_media(&directory);
        let output = directory.path().join("upscale.mp4");
        let mut queue = MediaQueue::new();
        let original = queue
            .enqueue(
                &identity,
                &source,
                upscale(
                    &source,
                    output,
                    VideoEncoderSelection::NvidiaNvenc,
                    VideoEncoderCapability::Available,
                ),
            )
            .unwrap();
        assert!(queue.cancel_queue());
        let before = queue.jobs().to_vec();
        let next = queue.next_job_id;
        let unavailable = tools(&directory).video_encoders;
        assert_eq!(
            queue.retry(original, None),
            Err(QueueMutationError::CapabilitiesPending)
        );
        assert!(matches!(
            queue.retry(original, Some(&unavailable)),
            Err(QueueMutationError::InvalidRequest(_))
        ));
        let mut available = unavailable;
        available.h264_nvenc = VideoEncoderCapability::Available;
        fs::write(&source.path, b"different source bytes").unwrap();
        assert_eq!(
            queue.retry(original, Some(&available)),
            Err(QueueMutationError::SourceChanged)
        );
        fs::remove_file(&source.path).unwrap();
        assert!(matches!(
            queue.retry(original, Some(&available)),
            Err(QueueMutationError::SourceUnavailable(_))
        ));
        assert_eq!(queue.jobs(), before);
        assert_eq!(queue.next_job_id, next);
    }

    #[test]
    fn retry_failed_preparation_and_execution_without_changing_run_state() {
        let directory = TestDirectory::new("retry-failed");
        let (identity, source) = media(&directory, "source.mkv");
        let parent = directory.path().join("outputs");
        fs::create_dir(&parent).unwrap();
        let mut queue = MediaQueue::new();
        let prep = queue
            .enqueue(&identity, &source, remux(&source, parent.join("prep.mkv")))
            .unwrap();
        let exec = queue
            .enqueue(
                &identity,
                &source,
                remux(&source, directory.path().join("exec.mkv")),
            )
            .unwrap();
        let active = queue
            .enqueue(
                &identity,
                &source,
                remux(&source, directory.path().join("active.mkv")),
            )
            .unwrap();
        fs::remove_dir(&parent).unwrap();
        let process = FakeProcessControl::running();
        let mut processes = VecDeque::from([process.clone(), FakeProcessControl::running()]);
        let mut launcher = |_, _| Ok(processes.pop_front().expect("exactly two launches"));
        let tools = tools(&directory);
        assert!(queue.start_with_launcher(&tools, true, &mut launcher));
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Failed(QueueJobFailure::Preparation(_))
        ));
        assert_eq!(queue.active_job(), Some(exec));
        process.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
            2,
        ))));
        queue.poll_with_launcher(Some(&tools), true, &mut launcher);
        assert!(matches!(
            queue.jobs()[1].state(),
            QueueJobState::Failed(QueueJobFailure::Execution(_))
        ));
        assert_eq!(queue.active_job(), Some(active));
        let history = queue.jobs()[..2].to_vec();
        let next = queue.next_job_id;
        assert!(matches!(
            queue.retry(prep, None),
            Err(QueueMutationError::Output(_))
        ));
        assert_eq!(queue.next_job_id, next);
        fs::create_dir(&parent).unwrap();
        let retried_prep = queue.retry(prep, None).unwrap();
        assert_eq!(queue.run_state(), QueueRunState::Running);
        assert_eq!(queue.active_job(), Some(active));
        assert!(queue.stop_after_current());
        let retried_exec = queue.retry(exec, None).unwrap();
        assert_eq!(queue.run_state(), QueueRunState::StoppingAfterCurrent);
        assert_eq!(queue.active_job(), Some(active));
        assert_eq!(&queue.jobs()[..2], history);
        assert_eq!(
            queue.waiting_jobs().map(|job| job.id()).collect::<Vec<_>>(),
            vec![retried_prep, retried_exec]
        );
        assert_eq!(
            queue.move_waiting(retried_exec, WaitingJobMove::Earlier),
            Ok(true)
        );
        assert!(queue.remove_waiting(retried_prep));
        assert_eq!(
            queue.waiting_jobs().map(|job| job.id()).collect::<Vec<_>>(),
            vec![retried_exec]
        );
    }

    #[test]
    fn retry_never_renames_or_replaces_output_claimed_after_admission() {
        use crate::media::output::MediaOutputError;
        for after_launch in [false, true] {
            let directory = TestDirectory::new("retry-race");
            let (identity, source) = media(&directory, "source.mkv");
            let output = directory.path().join("result.mkv");
            let mut queue = MediaQueue::new();
            let original = queue
                .enqueue(&identity, &source, remux(&source, output.clone()))
                .unwrap();
            assert!(queue.cancel_queue());
            let retry = queue.retry(original, None).unwrap();
            let process = FakeProcessControl::running();
            let mut launches = 0;
            let mut launcher = |_, _| {
                launches += 1;
                Ok(process.clone())
            };
            let tools = tools(&directory);
            if after_launch {
                assert!(queue.start_with_launcher(&tools, true, &mut launcher));
                let staged = queue.temporary_output_path().unwrap().to_path_buf();
                fs::write(&staged, b"retry output").unwrap();
                fs::write(&output, b"external file").unwrap();
                process.set_state(MediaProcessState::Completed);
                queue.poll_with_launcher(Some(&tools), true, &mut launcher);
                assert!(!staged.parent().unwrap().exists());
                assert!(matches!(
                    queue.jobs()[1].state(),
                    QueueJobState::Failed(QueueJobFailure::Execution(
                        MediaExecutionFailure::Output {
                            reason: MediaOutputError::DestinationOccupied(_),
                            ..
                        }
                    ))
                ));
            } else {
                fs::write(&output, b"external file").unwrap();
                assert!(queue.start_with_launcher(&tools, true, &mut launcher));
                assert!(matches!(
                    queue.jobs()[1].state(),
                    QueueJobState::Failed(QueueJobFailure::Preparation(_))
                ));
            }
            assert_eq!(launches, usize::from(after_launch));
            assert_eq!(queue.jobs()[1].id(), retry);
            assert_eq!(queue.jobs()[0].state(), &QueueJobState::Cancelled);
            assert_eq!(queue.run_state(), QueueRunState::Stopped);
            assert_eq!(fs::read(&output).unwrap(), b"external file");
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
        }
    }

    fn media(directory: &TestDirectory, file_name: &str) -> (SourceIdentity, MediaFile) {
        let path = directory.path().join(file_name);
        fs::write(&path, b"source").unwrap();
        let identity = SourceIdentity::read(&path).unwrap();
        (
            identity,
            MediaFile {
                path,
                duration: Some(std::time::Duration::from_secs(10)),
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
                video_streams: Vec::new(),
                audio_streams: vec![AudioStream {
                    stream_index: 1,
                    codec: Some("aac".to_owned()),
                    channels: Some(2),
                    channel_layout: Some("stereo".to_owned()),
                    title: None,
                    language: None,
                }],
            },
        )
    }

    fn video_media(directory: &TestDirectory) -> (SourceIdentity, MediaFile) {
        let (identity, mut media) = media(directory, "source.mkv");
        media.video_streams.push(VideoStream {
            stream_index: 0,
            codec: Some("h264".to_owned()),
            width: Some(320),
            height: Some(180),
            frame_rate: None,
            format: Default::default(),
        });
        (identity, media)
    }

    fn upscale(
        media: &MediaFile,
        output: PathBuf,
        selection: VideoEncoderSelection,
        capability: VideoEncoderCapability,
    ) -> QueuedOperation {
        let mut encoder_settings = UpscaleEncoderSettings::default();
        encoder_settings.set_selection(selection);
        QueuedOperation::Upscale(UpscaleRequest {
            input: media.path.clone(),
            output,
            target: UpscaleTarget::Custom {
                max_width: 640,
                max_height: 360,
            },
            codec: VideoCodec::H264,
            encoder_settings,
            nvenc_capability: capability,
        })
    }

    fn remux(media: &MediaFile, output: PathBuf) -> QueuedOperation {
        QueuedOperation::Remux(RemuxRequest {
            input: media.path.clone(),
            output,
            container: MediaContainer::Matroska,
        })
    }

    fn audio_extract(media: &MediaFile, output: PathBuf) -> QueuedOperation {
        QueuedOperation::AudioExtract(AudioExtractRequest {
            input: media.path.clone(),
            output,
            audio_stream_index: 1,
            format: AudioExtractFormat::Aac,
        })
    }

    fn tools(directory: &TestDirectory) -> MediaTools {
        MediaTools::new(
            directory.path().join("fake-ffmpeg"),
            directory.path().join("fake-ffprobe"),
        )
    }

    #[derive(Clone)]
    struct FakeProcessControl {
        state: Arc<Mutex<MediaProcessState>>,
        progress: Arc<Mutex<Vec<String>>>,
        defer_cancellation: bool,
    }

    impl FakeProcessControl {
        fn running() -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Running)),
                progress: Arc::new(Mutex::new(Vec::new())),
                defer_cancellation: false,
            }
        }

        fn deferred_cancellation() -> Self {
            Self {
                defer_cancellation: true,
                ..Self::running()
            }
        }

        fn set_state(&self, state: MediaProcessState) {
            *self.state.lock().unwrap() = state;
        }
    }

    impl ProcessControl for FakeProcessControl {
        fn state(&self) -> MediaProcessState {
            self.state.lock().unwrap().clone()
        }

        fn cancel(&self) -> bool {
            let mut state = self.state.lock().unwrap();
            if *state != MediaProcessState::Running {
                return false;
            }
            if !self.defer_cancellation {
                *state = MediaProcessState::Cancelled;
            }
            true
        }

        fn diagnostic_stderr(&self) -> String {
            String::new()
        }

        fn drain_progress(&self) -> Vec<String> {
            std::mem::take(&mut *self.progress.lock().unwrap())
        }
    }

    #[derive(Clone, Copy)]
    enum Outcome {
        Success,
        Failure,
        OutputFailure,
        Cancel,
        CancelFailure,
    }

    impl Outcome {
        fn label(self) -> &'static str {
            match self {
                Self::Success => "success",
                Self::Failure => "failure",
                Self::OutputFailure => "output-failure",
                Self::Cancel => "cancel",
                Self::CancelFailure => "cancel-failure",
            }
        }
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "apollo-queue-test-{}-{sequence}-{label}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
