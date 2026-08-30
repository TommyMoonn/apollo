use std::fmt;

use crate::media::{
    container::MediaContainer,
    error::MediaError,
    execution::{
        MediaExecution, MediaExecutionFailure, MediaExecutionStartError, MediaExecutionState,
        ProcessControl,
    },
    model::MediaFile,
    output::{MediaOutput, MediaOutputError},
    process::{MediaProcessCommand, MediaProcessStartError, ProcessProgressSource},
    progress::MediaProgress,
    tools::MediaTools,
};

use super::{
    command::build_remux_invocation,
    model::RemuxRequest,
    validation::{RemuxValidationError, validate_remux_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemuxStartError {
    AlreadyRunning,
    Validation(RemuxValidationError),
    Output(MediaOutputError),
    Process(MediaProcessStartError),
}

impl fmt::Display for RemuxStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "a remux operation is already running"),
            Self::Validation(error) => write!(f, "remux request is invalid: {error}"),
            Self::Output(error) => write!(f, "remux output could not be prepared: {error}"),
            Self::Process(error) => write!(f, "remux process could not start: {error}"),
        }
    }
}

impl std::error::Error for RemuxStartError {}

impl RemuxStartError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                RemuxValidationError::NoStreams => MediaError::missing_stream(error.to_string()),
                RemuxValidationError::IncompatibleStream {
                    container: MediaContainer::Mp4,
                    ..
                } => MediaError::mp4_incompatibility(error.to_string()),
                RemuxValidationError::IncompatibleStream { .. }
                | RemuxValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
                RemuxValidationError::Output(error) => MediaError::from_output_error(error),
            },
            Self::Output(error) => MediaError::from_output_error(error),
            Self::Process(error) => MediaError::from_process_start_error(error),
            Self::AlreadyRunning => MediaError::invalid_request(self.to_string()),
        }
    }
}

pub type RemuxExecutionFailure = MediaExecutionFailure;
pub type RemuxExecutionState = MediaExecutionState;

pub struct RemuxExecutor {
    execution: MediaExecution,
}

impl fmt::Debug for RemuxExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemuxExecutor")
            .field("state", self.state())
            .field("is_running", &self.is_running())
            .finish()
    }
}

impl Default for RemuxExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl RemuxExecutor {
    pub fn new() -> Self {
        Self {
            execution: MediaExecution::new(),
        }
    }

    pub fn state(&self) -> &RemuxExecutionState {
        self.execution.state()
    }

    pub fn is_running(&self) -> bool {
        self.execution.is_running()
    }

    pub fn progress(&self) -> Option<&MediaProgress> {
        self.execution.progress()
    }

    pub fn reset(&mut self) -> bool {
        self.execution.reset()
    }

    pub fn start(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &RemuxRequest,
    ) -> Result<(), RemuxStartError> {
        self.start_with_launcher(
            tools,
            media,
            request,
            crate::media::process::MediaProcess::start,
        )
    }

    pub fn cancel(&self) -> bool {
        self.execution.cancel()
    }

    /// Refresh progress and settle the staged output exactly once after FFmpeg terminates.
    pub fn poll(&mut self) -> &RemuxExecutionState {
        self.execution.poll()
    }

    fn start_with_launcher<F, P>(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &RemuxRequest,
        launcher: F,
    ) -> Result<(), RemuxStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.execution.is_running() {
            return Err(RemuxStartError::AlreadyRunning);
        }

        validate_remux_request(request, media).map_err(RemuxStartError::Validation)?;

        let output = MediaOutput::prepare(&request.input, &request.output)
            .map_err(RemuxStartError::Output)?;
        let mut staged_request = request.clone();
        staged_request.output = output.temporary_path().to_path_buf();
        let invocation = build_remux_invocation(&staged_request);
        let command = MediaProcessCommand::new(&tools.ffmpeg)
            .args(invocation.args)
            .progress_source(ProcessProgressSource::Stdout);

        self.execution
            .start_with_launcher(output, command, media.duration, launcher)
            .map_err(map_execution_start_error)
    }
}

fn map_execution_start_error(error: MediaExecutionStartError) -> RemuxStartError {
    match error {
        MediaExecutionStartError::AlreadyRunning => RemuxStartError::AlreadyRunning,
        MediaExecutionStartError::Process(error) => RemuxStartError::Process(error),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use crate::media::{
        container::MediaContainer,
        error::MediaErrorKind,
        execution::ProcessControl,
        model::{MediaFile, MediaStream, StreamKind},
        process::{MediaProcessFailure, MediaProcessState},
        tools::MediaTools,
    };

    use super::{RemuxExecutionFailure, RemuxExecutionState, RemuxExecutor, RemuxStartError};
    use crate::remux::model::RemuxRequest;

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn validation_failures_map_to_shared_user_error_categories() {
        let no_streams =
            RemuxStartError::Validation(crate::remux::validation::RemuxValidationError::NoStreams)
                .user_error();
        assert_eq!(no_streams.kind(), MediaErrorKind::MissingStream);

        let incompatible = RemuxStartError::Validation(
            crate::remux::validation::RemuxValidationError::IncompatibleStream {
                container: MediaContainer::Mp4,
                stream_index: 4,
                kind: StreamKind::Audio,
                codec: Some("wmav2".to_owned()),
            },
        )
        .user_error();
        assert_eq!(incompatible.kind(), MediaErrorKind::Mp4Incompatibility);
    }

    #[test]
    fn valid_request_starts_with_all_stream_copy_against_staged_output() {
        let directory = TestDirectory::new("valid-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let observed_args = Arc::new(Mutex::new(Vec::<OsString>::new()));
        let args_capture = Arc::clone(&observed_args);
        let mut executor = RemuxExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, move |command| {
                *args_capture.lock().unwrap() = command.arguments().to_vec();
                Ok(FakeProcessControl::running())
            })
            .unwrap();

        assert!(executor.is_running());
        assert_eq!(executor.state(), &RemuxExecutionState::Running);
        let args = observed_args.lock().unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-map", "0"]));
        assert!(args.windows(2).any(|pair| pair == ["-c", "copy"]));
        let staged_output = PathBuf::from(args.last().unwrap());
        assert_ne!(staged_output, request.output);
        assert_eq!(staged_output.file_name(), request.output.file_name());
        assert!(!request.output.exists());
    }

    #[test]
    fn incompatible_mp4_request_is_rejected_before_launch() {
        let directory = TestDirectory::new("invalid-no-spawn");
        let (request, mut media) = valid_request_and_media(&directory);
        media.streams[1].codec = Some("wmav2".to_owned());
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = RemuxExecutor::new();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(error, RemuxStartError::Validation(_)));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &RemuxExecutionState::Idle);
    }

    #[test]
    fn destination_preflight_rejects_existing_output_before_launch() {
        let directory = TestDirectory::new("existing-output");
        let (request, media) = valid_request_and_media(&directory);
        fs::write(&request.output, b"occupied").unwrap();
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = RemuxExecutor::new();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(error, RemuxStartError::Output(_)));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &RemuxExecutionState::Idle);
    }

    #[test]
    fn successful_process_publishes_final_output() {
        let directory = TestDirectory::new("success-publishes");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let process_handle = process.clone();
        let mut executor = RemuxExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        fs::write(temporary_output(&executor), b"complete remux").unwrap();
        process_handle.set_state(MediaProcessState::Completed);

        assert_eq!(
            executor.poll(),
            &RemuxExecutionState::Completed {
                output: request.output.clone(),
            }
        );
        assert_eq!(fs::read(&request.output).unwrap(), b"complete remux");
        assert!(!executor.is_running());
    }

    #[test]
    fn failed_process_discards_partial_output_and_retains_diagnostics() {
        let directory = TestDirectory::new("failure-discards");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::with_diagnostic("muxer failure detail");
        let process_handle = process.clone();
        let mut executor = RemuxExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        let temporary = temporary_output(&executor);
        fs::write(&temporary, b"partial remux").unwrap();
        process_handle.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
            7,
        ))));

        assert_eq!(
            executor.poll(),
            &RemuxExecutionState::Failed(RemuxExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(7)),
                diagnostic: "muxer failure detail".to_owned(),
            })
        );
        assert!(!request.output.exists());
        assert!(!temporary.exists());
    }

    #[test]
    fn cancellation_discards_partial_output() {
        let directory = TestDirectory::new("cancel-discards");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let process_handle = process.clone();
        let mut executor = RemuxExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        let temporary = temporary_output(&executor);
        fs::write(&temporary, b"partial remux").unwrap();

        assert!(executor.cancel());
        assert_eq!(process_handle.state(), MediaProcessState::Cancelled);
        assert_eq!(executor.poll(), &RemuxExecutionState::Cancelled);
        assert!(!request.output.exists());
        assert!(!temporary.exists());
    }

    #[test]
    fn running_progress_uses_shared_progress_parser() {
        let directory = TestDirectory::new("progress");
        let (request, mut media) = valid_request_and_media(&directory);
        media.duration = Some(Duration::from_secs(20));
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let process_handle = process.clone();
        let mut executor = RemuxExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        process_handle.push_progress(["out_time_us=5000000", "progress=continue"]);

        assert_eq!(executor.poll(), &RemuxExecutionState::Running);
        let progress = executor.progress().expect("progress snapshot");
        assert_eq!(progress.processed, Some(Duration::from_secs(5)));
        assert_eq!(progress.total, media.duration);
        assert_eq!(progress.fraction, Some(0.25));
    }

    #[test]
    fn second_start_is_rejected_while_process_is_owned() {
        let directory = TestDirectory::new("duplicate-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let mut executor = RemuxExecutor::new();

        let first_count = Arc::clone(&launches);
        executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                first_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap();

        let second_count = Arc::clone(&launches);
        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                second_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert_eq!(error, RemuxStartError::AlreadyRunning);
        assert_eq!(launches.load(Ordering::Relaxed), 1);
    }

    fn valid_request_and_media(directory: &TestDirectory) -> (RemuxRequest, MediaFile) {
        let input = directory.path().join("source.mkv");
        fs::write(&input, b"source").unwrap();
        let request = RemuxRequest {
            input: input.clone(),
            output: directory.path().join("remuxed.mp4"),
            container: MediaContainer::Mp4,
        };
        let media = MediaFile {
            path: input,
            duration: None,
            streams: vec![
                MediaStream {
                    stream_index: 0,
                    kind: StreamKind::Video,
                    codec: Some("h264".to_owned()),
                },
                MediaStream {
                    stream_index: 2,
                    kind: StreamKind::Audio,
                    codec: Some("aac".to_owned()),
                },
            ],
            video_streams: Vec::new(),
            audio_streams: Vec::new(),
        };
        (request, media)
    }

    fn fake_tools(directory: &TestDirectory) -> MediaTools {
        MediaTools {
            ffmpeg: directory.path().join("fake-ffmpeg"),
            ffprobe: directory.path().join("fake-ffprobe"),
        }
    }

    fn temporary_output(executor: &RemuxExecutor) -> PathBuf {
        executor
            .execution
            .temporary_output_path()
            .expect("running remux should own a temporary output")
            .to_path_buf()
    }

    #[derive(Clone)]
    struct FakeProcessControl {
        state: Arc<Mutex<MediaProcessState>>,
        diagnostic: Arc<String>,
        progress: Arc<Mutex<Vec<String>>>,
    }

    impl FakeProcessControl {
        fn running() -> Self {
            Self::with_diagnostic("")
        }

        fn with_diagnostic(diagnostic: &str) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Running)),
                diagnostic: Arc::new(diagnostic.to_owned()),
                progress: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn push_progress(&self, lines: impl IntoIterator<Item = impl Into<String>>) {
            self.progress
                .lock()
                .unwrap()
                .extend(lines.into_iter().map(Into::into));
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
            self.diagnostic.as_str().to_owned()
        }

        fn drain_progress(&self) -> Vec<String> {
            std::mem::take(&mut *self.progress.lock().unwrap())
        }
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "apollo-remux-execution-test-{}-{sequence}-{name}",
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
