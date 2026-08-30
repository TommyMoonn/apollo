use std::fmt;

use crate::media::{
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
    command::{UpscaleCommandError, build_upscale_invocation},
    model::UpscaleRequest,
    sizing::{UpscaleSizingError, calculate_output_dimensions},
    validation::{UpscaleValidationError, validate_upscale_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpscaleStartError {
    AlreadyRunning,
    Validation(UpscaleValidationError),
    Sizing(UpscaleSizingError),
    Output(MediaOutputError),
    Command(UpscaleCommandError),
    Process(MediaProcessStartError),
}

impl fmt::Display for UpscaleStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "an upscale operation is already running"),
            Self::Validation(error) => write!(f, "upscale request is invalid: {error}"),
            Self::Sizing(error) => write!(f, "upscale dimensions could not be calculated: {error}"),
            Self::Output(error) => write!(f, "upscale output could not be prepared: {error}"),
            Self::Command(error) => write!(f, "upscale command could not be built: {error}"),
            Self::Process(error) => write!(f, "upscale process could not start: {error}"),
        }
    }
}

impl std::error::Error for UpscaleStartError {}

impl UpscaleStartError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                UpscaleValidationError::MissingVideoStream => {
                    MediaError::missing_stream(error.to_string())
                }
                UpscaleValidationError::Output(error) => MediaError::from_output_error(error),
                UpscaleValidationError::MissingSourceDimensions { .. }
                | UpscaleValidationError::InvalidSourceDimensions { .. }
                | UpscaleValidationError::TargetNotLarger { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
            },
            Self::Sizing(error) => MediaError::invalid_request(error.to_string()),
            Self::Output(error) => MediaError::from_output_error(error),
            Self::Command(error) => MediaError::missing_stream(error.to_string()),
            Self::Process(error) => MediaError::from_process_start_error(error),
            Self::AlreadyRunning => MediaError::invalid_request(self.to_string()),
        }
    }
}

pub type UpscaleExecutionFailure = MediaExecutionFailure;
pub type UpscaleExecutionState = MediaExecutionState;

pub struct UpscaleExecutor {
    execution: MediaExecution,
}

impl fmt::Debug for UpscaleExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpscaleExecutor")
            .field("state", self.state())
            .field("is_running", &self.is_running())
            .finish()
    }
}

impl Default for UpscaleExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl UpscaleExecutor {
    pub fn new() -> Self {
        Self {
            execution: MediaExecution::new(),
        }
    }

    pub fn state(&self) -> &UpscaleExecutionState {
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
        request: &UpscaleRequest,
    ) -> Result<(), UpscaleStartError> {
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
    pub fn poll(&mut self) -> &UpscaleExecutionState {
        self.execution.poll()
    }

    fn start_with_launcher<F, P>(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &UpscaleRequest,
        launcher: F,
    ) -> Result<(), UpscaleStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.execution.is_running() {
            return Err(UpscaleStartError::AlreadyRunning);
        }

        validate_upscale_request(request, media).map_err(UpscaleStartError::Validation)?;

        let video = media
            .video_streams
            .first()
            .expect("validated upscale request has a primary video stream");
        let width = video
            .width
            .expect("validated upscale request has a source width");
        let height = video
            .height
            .expect("validated upscale request has a source height");
        let dimensions = calculate_output_dimensions(width, height, request.target)
            .map_err(UpscaleStartError::Sizing)?;

        let output = MediaOutput::prepare(&request.input, &request.output)
            .map_err(UpscaleStartError::Output)?;
        let mut staged_request = request.clone();
        staged_request.output = output.temporary_path().to_path_buf();
        let invocation = build_upscale_invocation(&staged_request, media, dimensions)
            .map_err(UpscaleStartError::Command)?;
        let command = MediaProcessCommand::new(&tools.ffmpeg)
            .args(invocation.args)
            .progress_source(ProcessProgressSource::Stdout);

        self.execution
            .start_with_launcher(output, command, media.duration, launcher)
            .map_err(map_execution_start_error)
    }
}

fn map_execution_start_error(error: MediaExecutionStartError) -> UpscaleStartError {
    match error {
        MediaExecutionStartError::AlreadyRunning => UpscaleStartError::AlreadyRunning,
        MediaExecutionStartError::Process(error) => UpscaleStartError::Process(error),
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

    use crate::{
        media::{
            error::MediaErrorKind,
            execution::ProcessControl,
            model::{AudioStream, MediaFile, VideoStream},
            output::MediaOutputError,
            process::{MediaProcessFailure, MediaProcessState},
            tools::MediaTools,
        },
        upscale::{
            model::{UpscaleRequest, UpscaleTarget, VideoCodec},
            validation::{UpscaleValidationError, validate_upscale_request},
        },
    };

    use super::{
        UpscaleExecutionFailure, UpscaleExecutionState, UpscaleExecutor, UpscaleStartError,
    };

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn prelaunch_validation_maps_to_shared_user_error_categories() {
        let missing_stream =
            UpscaleStartError::Validation(UpscaleValidationError::MissingVideoStream).user_error();
        assert_eq!(missing_stream.kind(), MediaErrorKind::MissingStream);

        let invalid_request =
            UpscaleStartError::Validation(UpscaleValidationError::MissingSourceDimensions {
                stream_index: 0,
            })
            .user_error();
        assert_eq!(invalid_request.kind(), MediaErrorKind::InvalidRequest);
    }

    #[test]
    fn valid_request_starts_against_calculated_staged_output() {
        let directory = TestDirectory::new("valid-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let observed_args = Arc::new(Mutex::new(Vec::<OsString>::new()));
        let args_capture = Arc::clone(&observed_args);
        let mut executor = UpscaleExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, move |command| {
                *args_capture.lock().unwrap() = command.arguments().to_vec();
                Ok(FakeProcessControl::running())
            })
            .unwrap();

        assert!(executor.is_running());
        assert_eq!(executor.state(), &UpscaleExecutionState::Running);
        let args = observed_args.lock().unwrap();
        assert!(args.windows(2).any(|pair| {
            pair == [
                OsString::from("-vf"),
                OsString::from("scale=3840:2160:flags=lanczos"),
            ]
        }));
        let staged_output = PathBuf::from(args.last().unwrap());
        assert_ne!(staged_output, request.output);
        assert_eq!(staged_output.file_name(), request.output.file_name());
        assert_eq!(
            staged_output.parent().unwrap().parent(),
            request.output.parent()
        );
        assert!(!request.output.exists());
    }

    #[test]
    fn invalid_request_never_launches_process() {
        let directory = TestDirectory::new("invalid-no-spawn");
        let (mut request, media) = valid_request_and_media(&directory);
        request.output = PathBuf::new();
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = UpscaleExecutor::new();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(error, UpscaleStartError::Validation(_)));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &UpscaleExecutionState::Idle);
        assert!(!executor.is_running());
    }

    #[test]
    fn output_appearing_after_request_validation_is_rejected_before_launch() {
        let directory = TestDirectory::new("late-output-preflight");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = UpscaleExecutor::new();

        assert_eq!(validate_upscale_request(&request, &media), Ok(()));
        fs::write(&request.output, b"occupied").unwrap();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(
            error,
            UpscaleStartError::Output(MediaOutputError::OutputAlreadyExists(_))
        ));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &UpscaleExecutionState::Idle);
    }

    #[test]
    fn successful_process_publishes_final_output() {
        let directory = TestDirectory::new("success-publishes");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let process_handle = process.clone();
        let mut executor = UpscaleExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        write_temporary_output(&executor, b"complete upscale");
        process_handle.set_state(MediaProcessState::Completed);

        assert_eq!(
            executor.poll(),
            &UpscaleExecutionState::Completed {
                output: request.output.clone(),
            }
        );
        assert_eq!(fs::read(&request.output).unwrap(), b"complete upscale");
        assert!(!executor.is_running());
    }

    #[test]
    fn encoding_failure_discards_partial_output_and_retains_diagnostics() {
        let directory = TestDirectory::new("failure-discards");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::with_diagnostic("encoder failure detail");
        let process_handle = process.clone();
        let mut executor = UpscaleExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        let temporary = temporary_output(&executor);
        fs::write(&temporary, b"partial upscale").unwrap();
        process_handle.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
            7,
        ))));

        assert_eq!(
            executor.poll(),
            &UpscaleExecutionState::Failed(UpscaleExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(7)),
                diagnostic: "encoder failure detail".to_owned(),
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
        let mut executor = UpscaleExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        let temporary = temporary_output(&executor);
        fs::write(&temporary, b"partial upscale").unwrap();

        assert!(executor.cancel());
        assert_eq!(process_handle.state(), MediaProcessState::Cancelled);
        assert_eq!(executor.poll(), &UpscaleExecutionState::Cancelled);
        assert!(!request.output.exists());
        assert!(!temporary.exists());
        assert!(!executor.cancel());
    }

    #[test]
    fn process_state_settles_once() {
        let directory = TestDirectory::new("single-settlement");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let process_handle = process.clone();
        let mut executor = UpscaleExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        write_temporary_output(&executor, b"complete upscale");
        process_handle.set_state(MediaProcessState::Completed);

        let settled = executor.poll().clone();
        assert!(settled.is_terminal());
        assert_eq!(executor.poll(), &settled);
        assert_eq!(fs::read(&request.output).unwrap(), b"complete upscale");
    }

    #[test]
    fn running_progress_uses_shared_progress_parser() {
        let directory = TestDirectory::new("progress");
        let (request, mut media) = valid_request_and_media(&directory);
        media.duration = Some(Duration::from_secs(10));
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let progress_handle = process.clone();
        let mut executor = UpscaleExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        progress_handle.push_progress(["frame=240", "out_time_us=2500000", "progress=continue"]);

        assert_eq!(executor.poll(), &UpscaleExecutionState::Running);
        let progress = executor.progress().expect("progress snapshot");
        assert_eq!(progress.processed, Some(Duration::from_millis(2_500)));
        assert_eq!(progress.total, media.duration);
        assert_eq!(progress.fraction, Some(0.25));
        assert_eq!(progress.frame, Some(240));
    }

    #[test]
    fn second_start_is_rejected_while_process_handle_is_owned() {
        let directory = TestDirectory::new("duplicate-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let mut executor = UpscaleExecutor::new();

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

        assert_eq!(error, UpscaleStartError::AlreadyRunning);
        assert_eq!(launches.load(Ordering::Relaxed), 1);
        assert!(executor.is_running());
    }

    #[test]
    fn settled_executor_can_reset_but_running_executor_cannot() {
        let directory = TestDirectory::new("reset");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let state_handle = process.clone();
        let mut executor = UpscaleExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        assert!(!executor.reset());

        write_temporary_output(&executor, b"complete upscale");
        state_handle.set_state(MediaProcessState::Completed);
        executor.poll();
        assert!(executor.reset());
        assert_eq!(executor.state(), &UpscaleExecutionState::Idle);
    }

    fn valid_request_and_media(directory: &TestDirectory) -> (UpscaleRequest, MediaFile) {
        let input = directory.path().join("source.mkv");
        fs::write(&input, b"source").unwrap();
        let request = UpscaleRequest {
            input: input.clone(),
            output: directory.path().join("upscaled.mp4"),
            target: UpscaleTarget::Uhd4k,
            codec: VideoCodec::H264,
        };
        let media = MediaFile {
            path: input,
            duration: None,
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
            }],
            audio_streams: vec![AudioStream {
                stream_index: 1,
                codec: Some("aac".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: None,
                language: None,
            }],
        };
        (request, media)
    }

    fn fake_tools(directory: &TestDirectory) -> MediaTools {
        MediaTools {
            ffmpeg: directory.path().join("fake-ffmpeg"),
            ffprobe: directory.path().join("fake-ffprobe"),
        }
    }

    fn temporary_output(executor: &UpscaleExecutor) -> PathBuf {
        executor
            .execution
            .temporary_output_path()
            .expect("running upscale should own a temporary output")
            .to_path_buf()
    }

    fn write_temporary_output(executor: &UpscaleExecutor, bytes: &[u8]) {
        fs::write(temporary_output(executor), bytes).unwrap();
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
                "apollo-upscale-execution-test-{}-{sequence}-{name}",
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
