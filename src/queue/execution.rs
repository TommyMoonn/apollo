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
    JobId, QueueEnqueueError, QueueJobFailure, QueueJobState, QueuedJob, QueuedOperation,
};

pub struct MediaQueue {
    jobs: Vec<QueuedJob>,
    next_job_id: Option<u64>,
    started: bool,
    active_job: Option<JobId>,
    execution: MediaExecution,
}

impl Default for MediaQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaQueue {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_job_id: Some(1),
            started: false,
            active_job: None,
            execution: MediaExecution::new(),
        }
    }

    pub fn jobs(&self) -> &[QueuedJob] {
        &self.jobs
    }

    pub fn active_job(&self) -> Option<JobId> {
        self.active_job
    }

    pub fn active_progress(&self) -> Option<&MediaProgress> {
        self.execution.progress()
    }

    pub fn is_started(&self) -> bool {
        self.started
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
        if let Some(existing) = self.jobs.iter().find(|job| {
            !job.state().is_terminal()
                && output_paths_equal(job.normalized_output(), &normalized_output)
        }) {
            return Err(QueueEnqueueError::DuplicateDestination {
                existing_job: existing.id(),
            });
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

    pub fn start(&mut self, tools: &MediaTools, wake: MediaProcessWake) -> bool {
        let mut launcher = |command| MediaProcess::start_with_wake(command, wake.clone());
        self.start_with_launcher(tools, &mut launcher)
    }

    pub fn poll(&mut self, tools: &MediaTools, wake: MediaProcessWake) {
        let mut launcher = |command| MediaProcess::start_with_wake(command, wake.clone());
        self.poll_with_launcher(tools, &mut launcher);
    }

    pub fn cancel_current(&self) -> bool {
        self.execution.cancel()
    }

    pub fn stop(&mut self) -> bool {
        self.started = false;
        let mut changed = false;
        for job in &mut self.jobs {
            if matches!(job.state(), QueueJobState::Waiting) {
                job.set_state(QueueJobState::Cancelled);
                changed = true;
            }
        }
        self.execution.cancel() || changed
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

    fn start_with_launcher<F, P>(&mut self, tools: &MediaTools, launcher: &mut F) -> bool
    where
        F: FnMut(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.started || self.active_job.is_some() {
            return false;
        }

        self.started = true;
        self.advance_with_launcher(tools, launcher);
        self.active_job.is_some()
    }

    fn poll_with_launcher<F, P>(&mut self, tools: &MediaTools, launcher: &mut F)
    where
        F: FnMut(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
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

        if self.started {
            self.advance_with_launcher(tools, launcher);
        }
    }

    fn advance_with_launcher<F, P>(&mut self, tools: &MediaTools, launcher: &mut F)
    where
        F: FnMut(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
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
                .start_prepared_with_launcher(prepared, &mut *launcher)
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

        self.started = false;
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
        media::{
            container::MediaContainer,
            execution::ProcessControl,
            model::{MediaFile, MediaStream, StreamKind},
            process::{MediaProcessFailure, MediaProcessStartError, MediaProcessState},
            tools::MediaTools,
        },
        queue::model::{QueueEnqueueError, QueueJobFailure, QueueJobState, QueuedOperation},
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
    fn terminal_job_no_longer_reserves_its_destination() {
        let directory = TestDirectory::new("terminal-output");
        let media = media(&directory, "source.mkv");
        let output = directory.path().join("result.mkv");
        let mut queue = MediaQueue::new();
        let first = queue
            .enqueue(&media, remux(&media, output.clone()))
            .unwrap();
        assert!(queue.stop());
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
            let mut launcher = move |_| {
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
        let mut launcher = |_| Ok(FakeProcessControl::running());

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
            let mut launcher = move |_| {
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
        let mut launcher = move |_| {
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
    fn stop_cancels_active_and_waiting_jobs_without_advancing() {
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
        let mut launcher = move |_| {
            launch_count.fetch_add(1, Ordering::Relaxed);
            Ok(FakeProcessControl::running())
        };

        assert!(queue.start_with_launcher(&tools(&directory), &mut launcher));
        assert!(queue.stop());
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
        let mut launcher = move |_| Ok(process.take().expect("one process launch"));
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
