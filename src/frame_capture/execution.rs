use std::fmt;

use crate::media::{
    error::MediaError,
    execution::{
        MediaExecution, MediaExecutionFailure, MediaExecutionStartError, MediaExecutionState,
        ProcessControl,
    },
    model::MediaFile,
    output::{MediaOutput, MediaOutputError},
    process::{
        MediaProcessCommand, MediaProcessStartError, MediaProcessWake, ProcessProgressSource,
    },
    progress::MediaProgress,
    tools::MediaTools,
};

use super::{
    command::build_frame_capture_invocation,
    model::FrameCaptureRequest,
    validation::{FrameCaptureValidationError, validate_frame_capture_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameCaptureStartError {
    AlreadyRunning,
    Validation(FrameCaptureValidationError),
    Output(MediaOutputError),
    Process(MediaProcessStartError),
}

impl fmt::Display for FrameCaptureStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => write!(formatter, "a frame capture is already running"),
            Self::Validation(error) => {
                write!(formatter, "frame capture request is invalid: {error}")
            }
            Self::Output(error) => write!(
                formatter,
                "frame capture output could not be prepared: {error}"
            ),
            Self::Process(error) => {
                write!(formatter, "frame capture process could not start: {error}")
            }
        }
    }
}

impl std::error::Error for FrameCaptureStartError {}

impl FrameCaptureStartError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                FrameCaptureValidationError::MissingVideoStream => {
                    MediaError::missing_stream(error.to_string())
                }
                FrameCaptureValidationError::NegativeTimestamp
                | FrameCaptureValidationError::TimestampAtOrBeyondDuration { .. }
                | FrameCaptureValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
                FrameCaptureValidationError::Output(error) => MediaError::from_output_error(error),
            },
            Self::Output(error) => MediaError::from_output_error(error),
            Self::Process(error) => MediaError::from_process_start_error(error),
            Self::AlreadyRunning => MediaError::invalid_request(self.to_string()),
        }
    }
}

pub type FrameCaptureExecutionFailure = MediaExecutionFailure;
pub type FrameCaptureExecutionState = MediaExecutionState;

pub struct FrameCaptureExecutor {
    execution: MediaExecution,
}

impl fmt::Debug for FrameCaptureExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameCaptureExecutor")
            .field("state", self.state())
            .field("is_running", &self.is_running())
            .finish()
    }
}

impl Default for FrameCaptureExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCaptureExecutor {
    pub fn new() -> Self {
        Self {
            execution: MediaExecution::new(),
        }
    }

    pub fn state(&self) -> &FrameCaptureExecutionState {
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
        request: &FrameCaptureRequest,
    ) -> Result<(), FrameCaptureStartError> {
        self.start_with_launcher(
            tools,
            media,
            request,
            crate::media::process::MediaProcess::start,
        )
    }

    pub fn start_with_wake(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &FrameCaptureRequest,
        wake: MediaProcessWake,
    ) -> Result<(), FrameCaptureStartError> {
        self.start_with_launcher(tools, media, request, move |command| {
            crate::media::process::MediaProcess::start_with_wake(command, wake)
        })
    }

    pub fn cancel(&self) -> bool {
        self.execution.cancel()
    }

    pub fn poll(&mut self) -> &FrameCaptureExecutionState {
        self.execution.poll()
    }

    fn start_with_launcher<F, P>(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &FrameCaptureRequest,
        launcher: F,
    ) -> Result<(), FrameCaptureStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.execution.is_running() {
            return Err(FrameCaptureStartError::AlreadyRunning);
        }
        validate_frame_capture_request(request, media)
            .map_err(FrameCaptureStartError::Validation)?;
        let primary_video_stream_index = media
            .video_streams
            .first()
            .expect("validated frame capture media owns a video stream")
            .stream_index;

        let output = MediaOutput::prepare(&request.input, &request.output)
            .map_err(FrameCaptureStartError::Output)?;
        let mut staged_request = request.clone();
        staged_request.output = output.temporary_path().to_path_buf();
        let invocation =
            build_frame_capture_invocation(&staged_request, primary_video_stream_index);
        let command = MediaProcessCommand::new(&tools.ffmpeg)
            .args(invocation.args)
            .progress_source(ProcessProgressSource::Stdout);

        self.execution
            .start_with_launcher(output, command, None, launcher)
            .map_err(map_execution_start_error)
    }
}

fn map_execution_start_error(error: MediaExecutionStartError) -> FrameCaptureStartError {
    match error {
        MediaExecutionStartError::AlreadyRunning => FrameCaptureStartError::AlreadyRunning,
        MediaExecutionStartError::Process(error) => FrameCaptureStartError::Process(error),
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
    };

    use crate::{
        frame_capture::{
            model::{FrameCaptureFormat, FrameCaptureRequest, FrameTimestamp},
            validation::FrameCaptureValidationError,
        },
        media::{
            error::MediaErrorKind,
            execution::ProcessControl,
            model::{MediaFile, MediaStream, StreamKind, VideoStream},
            process::{MediaProcessFailure, MediaProcessState},
            tools::MediaTools,
        },
    };

    use super::{
        FrameCaptureExecutionFailure, FrameCaptureExecutionState, FrameCaptureExecutor,
        FrameCaptureStartError,
    };

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn validation_failures_map_to_shared_user_error_categories() {
        let missing =
            FrameCaptureStartError::Validation(FrameCaptureValidationError::MissingVideoStream)
                .user_error();
        assert_eq!(missing.kind(), MediaErrorKind::MissingStream);

        let invalid =
            FrameCaptureStartError::Validation(FrameCaptureValidationError::NegativeTimestamp)
                .user_error();
        assert_eq!(invalid.kind(), MediaErrorKind::InvalidRequest);
    }

    #[test]
    fn valid_request_uses_primary_absolute_video_index_and_staged_output() {
        let directory = TestDirectory::new("valid-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let observed_args = Arc::new(Mutex::new(Vec::<OsString>::new()));
        let capture = Arc::clone(&observed_args);
        let mut executor = FrameCaptureExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, move |command| {
                *capture.lock().unwrap() = command.arguments().to_vec();
                Ok(FakeProcessControl::running())
            })
            .unwrap();

        assert_eq!(executor.state(), &FrameCaptureExecutionState::Running);
        let args = observed_args.lock().unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-map", "0:7"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-abort_on", "empty_output"])
        );
        let staged_output = PathBuf::from(args.last().unwrap());
        assert_ne!(staged_output, request.output);
        assert_eq!(staged_output.file_name(), request.output.file_name());
        assert!(!request.output.exists());
    }

    #[test]
    fn invalid_request_and_existing_destination_never_launch() {
        for existing_output in [false, true] {
            let directory = TestDirectory::new(if existing_output {
                "existing-output"
            } else {
                "invalid-request"
            });
            let (mut request, media) = valid_request_and_media(&directory);
            if existing_output {
                fs::write(&request.output, b"occupied").unwrap();
            } else {
                request.timestamp = FrameTimestamp::from_milliseconds(-1);
            }
            let tools = fake_tools(&directory);
            let launches = Arc::new(AtomicUsize::new(0));
            let launch_count = Arc::clone(&launches);
            let mut executor = FrameCaptureExecutor::new();

            let error = executor
                .start_with_launcher(&tools, &media, &request, move |_| {
                    launch_count.fetch_add(1, Ordering::Relaxed);
                    Ok(FakeProcessControl::running())
                })
                .unwrap_err();

            if existing_output {
                assert!(matches!(error, FrameCaptureStartError::Output(_)));
            } else {
                assert!(matches!(error, FrameCaptureStartError::Validation(_)));
            }
            assert_eq!(launches.load(Ordering::Relaxed), 0);
            assert_eq!(executor.state(), &FrameCaptureExecutionState::Idle);
        }
    }

    #[test]
    fn exact_duration_timestamp_is_rejected_before_launch() {
        let directory = TestDirectory::new("duration-boundary-no-launch");
        let (mut request, media) = valid_request_and_media(&directory);
        request.timestamp = FrameTimestamp::from_milliseconds(10_000);
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = FrameCaptureExecutor::new();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(
            error,
            FrameCaptureStartError::Validation(
                FrameCaptureValidationError::TimestampAtOrBeyondDuration { .. }
            )
        ));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &FrameCaptureExecutionState::Idle);
    }

    #[test]
    fn success_failure_and_cancellation_settle_publication_once() {
        for outcome in [Outcome::Success, Outcome::Failure, Outcome::Cancellation] {
            let directory = TestDirectory::new(outcome.label());
            let (request, media) = valid_request_and_media(&directory);
            let tools = fake_tools(&directory);
            let process = FakeProcessControl::running();
            let handle = process.clone();
            let mut executor = FrameCaptureExecutor::new();
            executor
                .start_with_launcher(&tools, &media, &request, |_| Ok(process))
                .unwrap();
            let temporary = executor
                .execution
                .temporary_output_path()
                .expect("running capture should own staged output")
                .to_path_buf();
            fs::write(&temporary, b"captured frame").unwrap();

            match outcome {
                Outcome::Success => {
                    handle.set_state(MediaProcessState::Completed);
                    assert_eq!(
                        executor.poll(),
                        &FrameCaptureExecutionState::Completed {
                            output: request.output.clone()
                        }
                    );
                    assert_eq!(fs::read(&request.output).unwrap(), b"captured frame");
                }
                Outcome::Failure => {
                    handle.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
                        3,
                    ))));
                    assert_eq!(
                        executor.poll(),
                        &FrameCaptureExecutionState::Failed(
                            FrameCaptureExecutionFailure::Process {
                                reason: MediaProcessFailure::Exit(Some(3)),
                                diagnostic: String::new(),
                            }
                        )
                    );
                    assert!(!request.output.exists());
                    assert!(!temporary.exists());
                }
                Outcome::Cancellation => {
                    assert!(executor.cancel());
                    assert_eq!(executor.poll(), &FrameCaptureExecutionState::Cancelled);
                    assert!(!request.output.exists());
                    assert!(!temporary.exists());
                }
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Outcome {
        Success,
        Failure,
        Cancellation,
    }

    impl Outcome {
        fn label(self) -> &'static str {
            match self {
                Self::Success => "success",
                Self::Failure => "failure",
                Self::Cancellation => "cancellation",
            }
        }
    }

    fn valid_request_and_media(directory: &TestDirectory) -> (FrameCaptureRequest, MediaFile) {
        let input = directory.path().join("source.mkv");
        fs::write(&input, b"source").unwrap();
        (
            FrameCaptureRequest {
                input: input.clone(),
                output: directory.path().join("frame.png"),
                timestamp: FrameTimestamp::from_milliseconds(1_250),
                format: FrameCaptureFormat::Png,
            },
            MediaFile {
                path: input,
                duration: Some(std::time::Duration::from_secs(10)),
                streams: vec![
                    MediaStream {
                        stream_index: 2,
                        kind: StreamKind::Audio,
                        codec: Some("aac".into()),
                    },
                    MediaStream {
                        stream_index: 7,
                        kind: StreamKind::Video,
                        codec: Some("h264".into()),
                    },
                ],
                video_streams: vec![VideoStream {
                    stream_index: 7,
                    codec: Some("h264".into()),
                    width: Some(1920),
                    height: Some(1080),
                    frame_rate: None,
                }],
                audio_streams: Vec::new(),
            },
        )
    }

    fn fake_tools(directory: &TestDirectory) -> MediaTools {
        MediaTools {
            ffmpeg: directory.path().join("fake-ffmpeg"),
            ffprobe: directory.path().join("fake-ffprobe"),
        }
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

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "apollo-frame-capture-test-{}-{sequence}-{label}",
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
