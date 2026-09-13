use crate::media::{
    execution::{MediaExecution, MediaExecutionStartError, MediaExecutionState, ProcessControl},
    intake::SourceIdentity,
    model::MediaFile,
    output::{normalized_output_destination, output_paths_equal},
    process::{MediaProcess, MediaProcessCommand, MediaProcessStartError, MediaProcessWake},
    progress::MediaProgress,
    tools::MediaTools,
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
            execution: MediaExecution::new(),
        }
    }

    pub fn jobs(&self) -> &[QueuedJob] {
        &self.jobs
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

    pub fn is_started(&self) -> bool {
        self.run_state != QueueRunState::Stopped
    }

    pub fn run_state(&self) -> QueueRunState {
        self.run_state
    }

    pub fn is_running(&self) -> bool {
        self.active_job.is_some()
    }

    pub fn enqueue(
        &mut self,
        media: &MediaFile,
        operation: QueuedOperation,
    ) -> Result<JobId, QueueEnqueueError> {
        let raw_id = self.next_job_id.ok_or(QueueEnqueueError::JobIdExhausted)?;
        let source_identity =
            SourceIdentity::read(operation.input()).map_err(QueueEnqueueError::Source)?;
        let media_identity =
            SourceIdentity::read(&media.path).map_err(QueueEnqueueError::Source)?;
        if !source_identity.is_reliably_same(&media_identity) {
            return Err(QueueEnqueueError::SourceDoesNotMatchMedia);
        }

        let normalized_output =
            normalized_output_destination(operation.input(), operation.output())
                .map_err(QueueEnqueueError::Output)?;
        if let Some(existing_job) = self.duplicate_destination(&normalized_output, None) {
            return Err(QueueEnqueueError::DuplicateDestination { existing_job });
        }

        let id = JobId::new(raw_id);
        self.next_job_id = raw_id.checked_add(1);
        self.jobs.push(QueuedJob::new(
            id,
            source_identity,
            media.clone(),
            operation,
            normalized_output,
        ));
        Ok(id)
    }

    pub fn enqueue_batch(
        &mut self,
        items: &[(MediaFile, QueuedOperation)],
    ) -> Result<Vec<JobId>, BatchEnqueueError> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let raw_start = self.next_job_id.ok_or(BatchEnqueueError::JobIdExhausted)?;
        let mut prepared = Vec::with_capacity(items.len());
        let mut batch_outputs: Vec<std::path::PathBuf> = Vec::with_capacity(items.len());

        for (index, (media, operation)) in items.iter().enumerate() {
            if !output_paths_equal(operation.input(), &media.path) {
                return Err(BatchEnqueueError::Item {
                    index,
                    error: QueueEnqueueError::SourceDoesNotMatchMedia,
                });
            }
            let source_identity = SourceIdentity::read(operation.input()).map_err(|error| {
                BatchEnqueueError::Item {
                    index,
                    error: QueueEnqueueError::Source(error),
                }
            })?;
            let media_identity =
                SourceIdentity::read(&media.path).map_err(|error| BatchEnqueueError::Item {
                    index,
                    error: QueueEnqueueError::Source(error),
                })?;
            if !source_identity.is_reliably_same(&media_identity) {
                return Err(BatchEnqueueError::Item {
                    index,
                    error: QueueEnqueueError::SourceDoesNotMatchMedia,
                });
            }
            let normalized_output =
                normalized_output_destination(operation.input(), operation.output()).map_err(
                    |error| BatchEnqueueError::Item {
                        index,
                        error: QueueEnqueueError::Output(error),
                    },
                )?;
            if let Some(existing_job) = self.duplicate_destination(&normalized_output, None) {
                return Err(BatchEnqueueError::Item {
                    index,
                    error: QueueEnqueueError::DuplicateDestination { existing_job },
                });
            }
            if let Some(first_index) = batch_outputs
                .iter()
                .position(|output| output_paths_equal(output, &normalized_output))
            {
                return Err(BatchEnqueueError::DuplicateDestination { index, first_index });
            }
            batch_outputs.push(normalized_output.clone());
            prepared.push((
                media.clone(),
                operation.clone(),
                source_identity,
                normalized_output,
            ));
        }

        let mut raw_ids = Vec::with_capacity(items.len());
        let mut next = Some(raw_start);
        for _ in items {
            let raw = next.ok_or(BatchEnqueueError::JobIdExhausted)?;
            raw_ids.push(raw);
            next = raw.checked_add(1);
        }

        let ids = raw_ids.iter().copied().map(JobId::new).collect::<Vec<_>>();
        for (raw_id, (media, operation, identity, output)) in raw_ids.into_iter().zip(prepared) {
            self.jobs.push(QueuedJob::new(
                JobId::new(raw_id),
                identity,
                media,
                operation,
                output,
            ));
        }
        self.next_job_id = next;
        Ok(ids)
    }

    pub fn start<F>(&mut self, tools: &MediaTools, mut wake_for_job: F) -> bool
    where
        F: FnMut(JobId) -> MediaProcessWake,
    {
        let mut launcher =
            |job_id, command| MediaProcess::start_with_wake(command, wake_for_job(job_id));
        self.start_with_launcher(tools, &mut launcher)
    }

    pub fn poll<F>(&mut self, tools: &MediaTools, mut wake_for_job: F)
    where
        F: FnMut(JobId) -> MediaProcessWake,
    {
        let mut launcher =
            |job_id, command| MediaProcess::start_with_wake(command, wake_for_job(job_id));
        self.poll_with_launcher(tools, &mut launcher);
    }

    pub fn cancel_current(&self) -> bool {
        self.execution.cancel()
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
        self.run_state = QueueRunState::Stopped;
        let mut changed = false;
        for job in &mut self.jobs {
            if matches!(job.state(), QueueJobState::Waiting) {
                job.set_state(QueueJobState::Cancelled);
                changed = true;
            }
        }
        self.execution.cancel() || changed
    }

    pub fn replace_waiting_operation(
        &mut self,
        id: JobId,
        operation: QueuedOperation,
    ) -> Result<(), QueueMutationError> {
        self.require_mutation_stopped()?;
        let index = self.waiting_job_index(id)?;
        let job = &self.jobs[index];

        let replacement_identity = SourceIdentity::read(operation.input())
            .map_err(QueueMutationError::SourceUnavailable)?;
        if operation.input() != job.operation().input()
            || !job
                .source_identity()
                .is_reliably_same(&replacement_identity)
        {
            return Err(QueueMutationError::SourceChanged);
        }

        let normalized_output =
            normalized_output_destination(operation.input(), operation.output())
                .map_err(QueueMutationError::Output)?;
        if let Some(existing_job) = self.duplicate_destination(&normalized_output, Some(id)) {
            return Err(QueueMutationError::DuplicateDestination { existing_job });
        }

        self.jobs[index].replace_operation(operation, normalized_output);
        Ok(())
    }

    pub fn move_waiting(
        &mut self,
        id: JobId,
        direction: WaitingJobMove,
    ) -> Result<bool, QueueMutationError> {
        self.require_mutation_stopped()?;
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
        true
    }

    pub fn clear_finished(&mut self) -> usize {
        let previous_len = self.jobs.len();
        self.jobs.retain(|job| !job.state().is_terminal());
        previous_len - self.jobs.len()
    }

    pub fn clear_disposable_history(&mut self) -> usize {
        let previous_len = self.jobs.len();
        self.jobs.retain(|job| !job.state().is_disposable_history());
        previous_len - self.jobs.len()
    }

    fn start_with_launcher<F, P>(&mut self, tools: &MediaTools, launcher: &mut F) -> bool
    where
        F: FnMut(JobId, MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.run_state != QueueRunState::Stopped || self.active_job.is_some() {
            return false;
        }

        self.run_state = QueueRunState::Running;
        self.advance_with_launcher(tools, launcher);
        self.active_job.is_some()
    }

    fn poll_with_launcher<F, P>(&mut self, tools: &MediaTools, launcher: &mut F)
    where
        F: FnMut(JobId, MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        let Some(active_job) = self.active_job else {
            return;
        };

        let execution_state = self.execution.poll().clone();
        if matches!(execution_state, MediaExecutionState::Running) {
            return;
        }

        let terminal_state = match execution_state {
            MediaExecutionState::Completed { output } => QueueJobState::Completed { output },
            MediaExecutionState::Failed(error) => {
                QueueJobState::Failed(QueueJobFailure::Execution(error))
            }
            MediaExecutionState::Cancelled => QueueJobState::Cancelled,
            MediaExecutionState::Idle | MediaExecutionState::Running => {
                unreachable!("an active queue job must own a non-idle media execution")
            }
        };
        self.job_mut(active_job).set_state(terminal_state);
        self.active_job = None;
        assert!(
            self.execution.reset(),
            "terminal queue execution must reset before advancing"
        );

        match self.run_state {
            QueueRunState::Running => self.advance_with_launcher(tools, launcher),
            QueueRunState::StoppingAfterCurrent => self.run_state = QueueRunState::Stopped,
            QueueRunState::Stopped => {}
        }
    }

    fn advance_with_launcher<F, P>(&mut self, tools: &MediaTools, launcher: &mut F)
    where
        F: FnMut(JobId, MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        while let Some(index) = self
            .jobs
            .iter()
            .position(|job| matches!(job.state(), QueueJobState::Waiting))
        {
            let job_id = self.jobs[index].id();
            let input = self.jobs[index].operation().input().to_path_buf();
            let source_identity = self.jobs[index].source_identity().clone();
            let media = self.jobs[index].media().clone();
            let operation = self.jobs[index].operation().clone();

            let current_identity = match SourceIdentity::read(&input) {
                Ok(identity) => identity,
                Err(error) => {
                    self.jobs[index].set_state(QueueJobState::Failed(
                        QueueJobFailure::SourceUnavailable(error),
                    ));
                    continue;
                }
            };
            if !source_identity.is_reliably_same(&current_identity) {
                self.jobs[index].set_state(QueueJobState::Failed(QueueJobFailure::SourceChanged));
                continue;
            }

            let prepared = match operation.prepare(tools, &media) {
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
                    self.jobs[index].set_state(QueueJobState::Running);
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

    fn require_mutation_stopped(&self) -> Result<(), QueueMutationError> {
        if self.run_state == QueueRunState::Stopped && self.active_job.is_none() {
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

    fn duplicate_destination(
        &self,
        normalized_output: &std::path::Path,
        excluded_job: Option<JobId>,
    ) -> Option<JobId> {
        self.jobs
            .iter()
            .find(|job| {
                Some(job.id()) != excluded_job
                    && !job.state().is_terminal()
                    && output_paths_equal(job.normalized_output(), normalized_output)
            })
            .map(QueuedJob::id)
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

    #[cfg(test)]
    fn temporary_output_path(&self) -> Option<&std::path::Path> {
        self.execution.temporary_output_path()
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
        },
    };

    use crate::{
        audio_extract::model::{AudioExtractFormat, AudioExtractRequest},
        media::{
            container::MediaContainer,
            execution::ProcessControl,
            model::{MediaFile, MediaStream, StreamKind},
            process::{MediaProcessFailure, MediaProcessStartError, MediaProcessState},
            tools::MediaTools,
        },
        queue::model::{
            QueueEnqueueError, QueueJobFailure, QueueJobState, QueueMutationError, QueueRunState,
            QueuedOperation, WaitingJobMove,
        },
        remux::model::RemuxRequest,
    };

    use super::MediaQueue;

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn job_ids_are_monotonic_and_removal_does_not_reuse_them() {
        let directory = TestDirectory::new("monotonic");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&media, remux(&media, directory.path().join("first.mkv")))
            .unwrap();
        let second = queue
            .enqueue(&media, remux(&media, directory.path().join("second.mkv")))
            .unwrap();
        assert!(queue.remove_waiting(first));
        let third = queue
            .enqueue(&media, remux(&media, directory.path().join("third.mkv")))
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
        let first_media = media(&directory, "first.mkv");
        let second_media = media(&directory, "second.mkv");
        let mut queue = MediaQueue::new();
        let before = queue.run_state();

        let ids = queue
            .enqueue_batch(&[
                (
                    first_media.clone(),
                    remux(&first_media, directory.path().join("first-output.mkv")),
                ),
                (
                    second_media.clone(),
                    remux(&second_media, directory.path().join("second-output.mkv")),
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
        let error = queue.enqueue_batch(&[
            (
                first_media.clone(),
                remux(&first_media, directory.path().join("third-output.mkv")),
            ),
            (
                second_media.clone(),
                remux(&second_media, directory.path().join("third-output.mkv")),
            ),
        ]);
        assert!(matches!(
            error,
            Err(crate::queue::BatchEnqueueError::DuplicateDestination {
                index: 1,
                first_index: 0
            })
        ));
        assert_eq!(queue.jobs().len(), count);
    }

    #[test]
    fn batch_enqueue_rejects_existing_destination_and_input_mismatch_without_mutation() {
        let directory = TestDirectory::new("batch-validation");
        let first_media = media(&directory, "first.mkv");
        let second_media = media(&directory, "second.mkv");
        let reserved = directory.path().join("reserved.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(&first_media, remux(&first_media, reserved.clone()))
            .unwrap();
        let count = queue.jobs().len();

        assert!(
            queue
                .enqueue_batch(&[(second_media.clone(), remux(&second_media, reserved))])
                .is_err()
        );
        assert_eq!(queue.jobs().len(), count);

        let wrong_input = remux(&first_media, directory.path().join("new.mkv"));
        assert!(queue.enqueue_batch(&[(second_media, wrong_input)]).is_err());
        assert_eq!(queue.jobs().len(), count);
    }

    #[test]
    fn job_id_exhaustion_fails_instead_of_wrapping() {
        let directory = TestDirectory::new("id-exhaustion");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        queue.next_job_id = Some(u64::MAX);

        let last = queue
            .enqueue(&media, remux(&media, directory.path().join("last.mkv")))
            .unwrap();
        assert_eq!(last.get(), u64::MAX);
        assert_eq!(
            queue.enqueue(&media, remux(&media, directory.path().join("wrapped.mkv"))),
            Err(QueueEnqueueError::JobIdExhausted)
        );
    }

    #[test]
    fn normalized_duplicate_pending_destination_is_rejected() {
        let directory = TestDirectory::new("duplicate-output");
        let media = media(&directory, "source.mkv");
        let output = directory.path().join("result.mkv");
        let equivalent = directory.path().join(".").join("result.mkv");
        let mut queue = MediaQueue::new();
        let first = queue.enqueue(&media, remux(&media, output)).unwrap();

        assert_eq!(
            queue.enqueue(&media, remux(&media, equivalent)),
            Err(QueueEnqueueError::DuplicateDestination {
                existing_job: first
            })
        );
        assert_eq!(queue.jobs().len(), 1);
    }

    #[test]
    fn waiting_replacement_preserves_identity_snapshot_position_and_job_id() {
        let directory = TestDirectory::new("replace-operation");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&media, remux(&media, directory.path().join("first.mkv")))
            .unwrap();
        let second = queue
            .enqueue(&media, remux(&media, directory.path().join("second.mkv")))
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
    fn waiting_replacement_revalidates_source_and_is_atomic_on_failure() {
        let directory = TestDirectory::new("replace-atomic");
        let source_media = media(&directory, "source.mkv");
        let other_media = media(&directory, "other.mkv");
        let first_output = directory.path().join("first.mkv");
        let second_output = directory.path().join("second.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&source_media, remux(&source_media, first_output.clone()))
            .unwrap();
        let second = queue
            .enqueue(&source_media, remux(&source_media, second_output.clone()))
            .unwrap();
        let original_operation = queue.jobs()[0].operation().clone();
        let original_normalized_output = queue.jobs()[0].normalized_output().to_path_buf();

        assert_eq!(
            queue.replace_waiting_operation(
                first,
                remux(&other_media, directory.path().join("other-output.mkv"))
            ),
            Err(QueueMutationError::SourceChanged)
        );
        assert_eq!(queue.jobs()[0].operation(), &original_operation);
        assert_eq!(
            queue.jobs()[0].normalized_output(),
            original_normalized_output
        );

        assert_eq!(
            queue.replace_waiting_operation(first, remux(&source_media, second_output)),
            Err(QueueMutationError::DuplicateDestination {
                existing_job: second
            })
        );
        assert_eq!(queue.jobs()[0].operation(), &original_operation);
        assert_eq!(
            queue.jobs()[0].normalized_output(),
            original_normalized_output
        );

        assert_eq!(
            queue.replace_waiting_operation(first, remux(&source_media, source_media.path.clone())),
            Err(QueueMutationError::Output(
                crate::media::output::MediaOutputError::SameInputAndOutput(
                    source_media.path.clone()
                )
            ))
        );
        assert_eq!(queue.jobs()[0].operation(), &original_operation);
        assert_eq!(
            queue.jobs()[0].normalized_output(),
            original_normalized_output
        );

        queue
            .replace_waiting_operation(first, remux(&source_media, first_output))
            .unwrap();
    }

    #[test]
    fn waiting_replacement_rejects_a_source_changed_after_enqueue() {
        let directory = TestDirectory::new("replace-changed-source");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let id = queue
            .enqueue(&media, remux(&media, directory.path().join("first.mkv")))
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
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let terminal = queue
            .enqueue(&media, remux(&media, directory.path().join("terminal.mkv")))
            .unwrap();
        assert!(queue.cancel_queue());
        let first = queue
            .enqueue(&media, remux(&media, directory.path().join("first.mkv")))
            .unwrap();
        let second = queue
            .enqueue(&media, remux(&media, directory.path().join("second.mkv")))
            .unwrap();
        let third = queue
            .enqueue(&media, remux(&media, directory.path().join("third.mkv")))
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
    fn terminal_job_no_longer_reserves_its_destination() {
        let directory = TestDirectory::new("terminal-output");
        let media = media(&directory, "source.mkv");
        let output = directory.path().join("result.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&media, remux(&media, output.clone()))
            .unwrap();
        assert!(queue.cancel_queue());
        assert!(matches!(queue.jobs()[0].state(), QueueJobState::Cancelled));

        let second = queue.enqueue(&media, remux(&media, output)).unwrap();

        assert!(second > first);
        assert_eq!(queue.jobs().len(), 2);
        assert!(matches!(queue.jobs()[1].state(), QueueJobState::Waiting));
    }

    #[test]
    fn changed_or_missing_source_fails_job_and_advances_in_order() {
        for missing in [false, true] {
            let directory = TestDirectory::new(if missing { "missing" } else { "changed" });
            let first_media = media(&directory, "first-source.mkv");
            let second_media = media(&directory, "second-source.mkv");
            let mut queue = MediaQueue::new();
            let first = queue
                .enqueue(
                    &first_media,
                    remux(&first_media, directory.path().join("first-output.mkv")),
                )
                .unwrap();
            let second = queue
                .enqueue(
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

            assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
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
    fn destination_collision_after_enqueue_fails_job_and_advances() {
        let directory = TestDirectory::new("late-output");
        let first_media = media(&directory, "first-source.mkv");
        let second_media = media(&directory, "second-source.mkv");
        let first_output = directory.path().join("first-output.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&first_media, remux(&first_media, first_output.clone()))
            .unwrap();
        let second = queue
            .enqueue(
                &second_media,
                remux(&second_media, directory.path().join("second-output.mkv")),
            )
            .unwrap();
        fs::write(first_output, b"external output").unwrap();
        let mut launcher = |_, _| Ok(FakeProcessControl::running());

        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
        assert_eq!(queue.active_job(), Some(second));
        assert_eq!(queue.jobs()[0].id(), first);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Failed(QueueJobFailure::Preparation(_))
        ));
    }

    #[test]
    fn success_failure_and_cancel_each_advance_to_one_next_process() {
        for outcome in [Outcome::Success, Outcome::Failure, Outcome::Cancel] {
            let directory = TestDirectory::new(outcome.label());
            let first_media = media(&directory, "first-source.mkv");
            let second_media = media(&directory, "second-source.mkv");
            let mut queue = MediaQueue::new();
            let first = queue
                .enqueue(
                    &first_media,
                    remux(&first_media, directory.path().join("first-output.mkv")),
                )
                .unwrap();
            let second = queue
                .enqueue(
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

            assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
            assert_eq!(queue.active_job(), Some(first));
            assert!(!queue.start_with_launcher(&tools(&directory), &mut launcher));
            assert_eq!(launches.load(Ordering::Relaxed), 1);
            if matches!(outcome, Outcome::Success) {
                fs::write(queue.temporary_output_path().unwrap(), b"complete").unwrap();
                first_handle.set_state(MediaProcessState::Completed);
            } else if matches!(outcome, Outcome::Failure) {
                first_handle.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
                    2,
                ))));
            } else {
                assert!(queue.cancel_current());
            }

            queue.poll_with_launcher(&tools(&directory), &mut launcher);

            assert_eq!(queue.active_job(), Some(second));
            assert_eq!(launches.load(Ordering::Relaxed), 2);
            assert!(queue.jobs()[0].state().is_terminal());
            assert!(matches!(queue.jobs()[1].state(), QueueJobState::Running));
        }
    }

    #[test]
    fn running_and_terminal_jobs_reject_waiting_mutation() {
        let directory = TestDirectory::new("mutation-state");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let running = queue
            .enqueue(&media, remux(&media, directory.path().join("running.mkv")))
            .unwrap();
        let waiting = queue
            .enqueue(&media, remux(&media, directory.path().join("waiting.mkv")))
            .unwrap();
        let mut launcher = |_, _| Ok(FakeProcessControl::running());
        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));

        assert_eq!(
            queue.replace_waiting_operation(
                running,
                remux(&media, directory.path().join("replacement.mkv"))
            ),
            Err(QueueMutationError::AutomaticAdvancementActive)
        );
        assert_eq!(
            queue.move_waiting(waiting, WaitingJobMove::Earlier),
            Err(QueueMutationError::AutomaticAdvancementActive)
        );

        assert!(queue.cancel_queue());
        queue.poll_with_launcher(&tools(&directory), &mut launcher);
        assert_eq!(
            queue.replace_waiting_operation(
                running,
                remux(&media, directory.path().join("terminal-replacement.mkv"))
            ),
            Err(QueueMutationError::JobNotWaiting(running))
        );
        assert_eq!(
            queue.move_waiting(waiting, WaitingJobMove::Earlier),
            Err(QueueMutationError::JobNotWaiting(waiting))
        );
    }

    #[test]
    fn stop_after_current_completes_active_then_preserves_and_restarts_waiting() {
        let directory = TestDirectory::new("stop-after-current");
        let first_media = media(&directory, "first-source.mkv");
        let second_media = media(&directory, "second-source.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(
                &first_media,
                remux(&first_media, directory.path().join("first-output.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
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

        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
        assert_eq!(queue.run_state(), QueueRunState::Running);
        assert!(queue.stop_after_current());
        assert_eq!(queue.run_state(), QueueRunState::StoppingAfterCurrent);
        assert!(!queue.stop_after_current());
        fs::write(queue.temporary_output_path().unwrap(), b"complete").unwrap();
        first_handle.set_state(MediaProcessState::Completed);

        queue.poll_with_launcher(&tools(&directory), &mut launcher);

        assert_eq!(queue.run_state(), QueueRunState::Stopped);
        assert_eq!(queue.active_job(), None);
        assert_eq!(launches.load(Ordering::Relaxed), 1);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Completed { .. }
        ));
        assert!(matches!(queue.jobs()[1].state(), QueueJobState::Waiting));

        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
        assert_eq!(queue.active_job(), Some(second));
        assert_eq!(launches.load(Ordering::Relaxed), 2);
        assert_eq!(queue.jobs()[0].id(), first);
    }

    #[test]
    fn stop_after_current_without_an_active_job_stops_and_preserves_waiting() {
        let directory = TestDirectory::new("stop-without-active");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let waiting = queue
            .enqueue(&media, remux(&media, directory.path().join("waiting.mkv")))
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
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        let terminal = queue
            .enqueue(&media, remux(&media, directory.path().join("terminal.mkv")))
            .unwrap();
        queue.jobs[0].set_state(QueueJobState::Failed(QueueJobFailure::SourceChanged));
        let terminal_state = queue.jobs()[0].state().clone();
        let active = queue
            .enqueue(&media, remux(&media, directory.path().join("active.mkv")))
            .unwrap();
        let waiting = queue
            .enqueue(&media, remux(&media, directory.path().join("waiting.mkv")))
            .unwrap();
        let mut launcher = |_, _| Ok(FakeProcessControl::running());
        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));

        assert!(queue.cancel_queue());
        queue.poll_with_launcher(&tools(&directory), &mut launcher);

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
        let first_media = media(&directory, "first-source.mkv");
        let second_media = media(&directory, "second-source.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(
                &first_media,
                remux(&first_media, directory.path().join("first-output.mkv")),
            )
            .unwrap();
        let second = queue
            .enqueue(
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

        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
        assert_eq!(queue.active_job(), Some(second));
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Failed(QueueJobFailure::ProcessStart(_))
        ));
    }

    #[test]
    fn cancel_queue_cancels_active_and_waiting_jobs_without_advancing() {
        let directory = TestDirectory::new("stop");
        let first_media = media(&directory, "first-source.mkv");
        let second_media = media(&directory, "second-source.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(
                &first_media,
                remux(&first_media, directory.path().join("first-output.mkv")),
            )
            .unwrap();
        queue
            .enqueue(
                &second_media,
                remux(&second_media, directory.path().join("second-output.mkv")),
            )
            .unwrap();
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut launcher = move |_, _| {
            launch_count.fetch_add(1, Ordering::Relaxed);
            Ok(FakeProcessControl::running())
        };

        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
        assert!(queue.cancel_queue());
        queue.poll_with_launcher(&tools(&directory), &mut launcher);

        assert_eq!(launches.load(Ordering::Relaxed), 1);
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
    fn successful_output_is_published_once_and_terminal_job_remains_until_cleared() {
        let directory = TestDirectory::new("publish-once");
        let media = media(&directory, "source.mkv");
        let output = directory.path().join("output.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(&media, remux(&media, output.clone()))
            .unwrap();
        let process = FakeProcessControl::running();
        let handle = process.clone();
        let mut process = Some(process);
        let mut launcher = move |_, _| Ok(process.take().expect("one process launch"));
        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
        fs::write(queue.temporary_output_path().unwrap(), b"published").unwrap();
        handle.set_state(MediaProcessState::Completed);

        queue.poll_with_launcher(&tools(&directory), &mut launcher);
        queue.poll_with_launcher(&tools(&directory), &mut launcher);

        assert_eq!(fs::read(&output).unwrap(), b"published");
        assert_eq!(queue.jobs().len(), 1);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Completed { output: completed } if completed == &output
        ));
        assert_eq!(queue.clear_finished(), 1);
        assert!(queue.jobs().is_empty());
    }

    #[test]
    fn disposable_history_cleanup_preserves_failed_and_pending_jobs() {
        let directory = TestDirectory::new("disposable-history");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        for index in 0..5 {
            queue
                .enqueue(
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
        queue.jobs[3].set_state(QueueJobState::Running);

        assert_eq!(queue.pending_count(), 2);
        assert_eq!(queue.clear_disposable_history(), 2);
        assert_eq!(queue.jobs().len(), 3);
        assert!(matches!(
            queue.jobs()[0].state(),
            QueueJobState::Failed(QueueJobFailure::SourceChanged)
        ));
        assert!(matches!(queue.jobs()[1].state(), QueueJobState::Running));
        assert!(matches!(queue.jobs()[2].state(), QueueJobState::Waiting));
        assert_eq!(queue.pending_count(), 2);

        assert_eq!(queue.clear_finished(), 1);
        assert_eq!(queue.jobs().len(), 2);
        assert!(queue.jobs().iter().all(|job| job.state().is_pending()));
    }

    #[test]
    fn dropping_queue_requests_cancellation_from_the_active_process_owner() {
        let directory = TestDirectory::new("drop-cancel");
        let media = media(&directory, "source.mkv");
        let mut queue = MediaQueue::new();
        queue
            .enqueue(&media, remux(&media, directory.path().join("output.mkv")))
            .unwrap();
        let process = FakeProcessControl::running();
        let handle = process.clone();
        let mut process = Some(process);
        let mut launcher = move |_, _| Ok(process.take().expect("one process launch"));
        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));

        drop(queue);

        assert_eq!(handle.state(), MediaProcessState::Cancelled);
    }

    fn media(directory: &TestDirectory, file_name: &str) -> MediaFile {
        let path = directory.path().join(file_name);
        fs::write(&path, b"source").unwrap();
        MediaFile {
            path,
            duration: Some(std::time::Duration::from_secs(10)),
            streams: vec![MediaStream {
                stream_index: 0,
                kind: StreamKind::Video,
                codec: Some("h264".to_owned()),
            }],
            video_streams: Vec::new(),
            audio_streams: Vec::new(),
        }
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
            audio_stream_index: 0,
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
    }

    impl FakeProcessControl {
        fn running() -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Running)),
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
            *state = MediaProcessState::Cancelled;
            true
        }

        fn diagnostic_stderr(&self) -> String {
            String::new()
        }

        fn drain_progress(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[derive(Clone, Copy)]
    enum Outcome {
        Success,
        Failure,
        Cancel,
    }

    impl Outcome {
        fn label(self) -> &'static str {
            match self {
                Self::Success => "success",
                Self::Failure => "failure",
                Self::Cancel => "cancel",
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
