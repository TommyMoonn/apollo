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
    command::build_audio_extract_invocation,
    model::{AudioExtractMetadata, AudioExtractRequest},
    validation::{AudioExtractValidationError, validate_audio_extract_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioExtractStartError {
    AlreadyRunning,
    Validation(AudioExtractValidationError),
    Output(MediaOutputError),
    Process(MediaProcessStartError),
}

impl fmt::Display for AudioExtractStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "an audio extraction is already running"),
            Self::Validation(error) => write!(f, "audio extraction request is invalid: {error}"),
            Self::Output(error) => {
                write!(f, "audio extraction output could not be prepared: {error}")
            }
            Self::Process(error) => write!(f, "audio extraction process could not start: {error}"),
        }
    }
}

impl std::error::Error for AudioExtractStartError {}

impl AudioExtractStartError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                AudioExtractValidationError::UnknownStreamIndex { .. } => {
                    MediaError::missing_stream(error.to_string())
                }
                AudioExtractValidationError::StreamIsNotAudio { .. }
                | AudioExtractValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
                AudioExtractValidationError::Output(error) => MediaError::from_output_error(error),
            },
            Self::Output(error) => MediaError::from_output_error(error),
            Self::Process(error) => MediaError::from_process_start_error(error),
            Self::AlreadyRunning => MediaError::invalid_request(self.to_string()),
        }
    }
}

pub type AudioExtractExecutionFailure = MediaExecutionFailure;
pub type AudioExtractExecutionState = MediaExecutionState;

pub struct AudioExtractExecutor {
    execution: MediaExecution,
}

impl fmt::Debug for AudioExtractExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioExtractExecutor")
            .field("state", self.state())
            .field("is_running", &self.is_running())
            .finish()
    }
}

impl Default for AudioExtractExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioExtractExecutor {
    pub fn new() -> Self {
        Self {
            execution: MediaExecution::new(),
        }
    }

    pub fn state(&self) -> &AudioExtractExecutionState {
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
        request: &AudioExtractRequest,
    ) -> Result<(), AudioExtractStartError> {
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
        request: &AudioExtractRequest,
        wake: MediaProcessWake,
    ) -> Result<(), AudioExtractStartError> {
        self.start_with_launcher(tools, media, request, move |command| {
            crate::media::process::MediaProcess::start_with_wake(command, wake)
        })
    }

    pub fn cancel(&self) -> bool {
        self.execution.cancel()
    }

    pub fn poll(&mut self) -> &AudioExtractExecutionState {
        self.execution.poll()
    }

    fn start_with_launcher<F, P>(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &AudioExtractRequest,
        launcher: F,
    ) -> Result<(), AudioExtractStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.execution.is_running() {
            return Err(AudioExtractStartError::AlreadyRunning);
        }

        validate_audio_extract_request(request, media)
            .map_err(AudioExtractStartError::Validation)?;

        let metadata = media
            .audio_streams
            .iter()
            .find(|stream| stream.stream_index == request.audio_stream_index)
            .map(AudioExtractMetadata::from)
            .unwrap_or_default();

        let output = MediaOutput::prepare(&request.input, &request.output)
            .map_err(AudioExtractStartError::Output)?;
        let mut staged_request = request.clone();
        staged_request.output = output.temporary_path().to_path_buf();
        let invocation = build_audio_extract_invocation(&staged_request, &metadata);
        let command = MediaProcessCommand::new(&tools.ffmpeg)
            .args(invocation.args)
            .progress_source(ProcessProgressSource::Stdout);

        self.execution
            .start_with_launcher(output, command, media.duration, launcher)
            .map_err(map_execution_start_error)
    }
}

fn map_execution_start_error(error: MediaExecutionStartError) -> AudioExtractStartError {
    match error {
        MediaExecutionStartError::AlreadyRunning => AudioExtractStartError::AlreadyRunning,
        MediaExecutionStartError::Process(error) => AudioExtractStartError::Process(error),
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
        audio_extract::model::{AudioExtractFormat, AudioExtractRequest},
        media::{
            error::MediaErrorKind,
            execution::ProcessControl,
            model::{AudioStream, MediaFile, MediaStream, StreamKind},
            process::{MediaProcessFailure, MediaProcessState},
            tools::MediaTools,
        },
    };

    use super::{
        AudioExtractExecutionFailure, AudioExtractExecutionState, AudioExtractExecutor,
        AudioExtractStartError,
    };

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn validation_failures_map_to_shared_user_error_categories() {
        let missing = AudioExtractStartError::Validation(
            crate::audio_extract::validation::AudioExtractValidationError::UnknownStreamIndex {
                stream_index: 9,
            },
        )
        .user_error();
        assert_eq!(missing.kind(), MediaErrorKind::MissingStream);

        let non_audio = AudioExtractStartError::Validation(
            crate::audio_extract::validation::AudioExtractValidationError::StreamIsNotAudio {
                stream_index: 0,
                kind: StreamKind::Video,
            },
        )
        .user_error();
        assert_eq!(non_audio.kind(), MediaErrorKind::InvalidRequest);
    }

    #[test]
    fn valid_request_starts_against_staged_output() {
        let directory = TestDirectory::new("valid-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let observed_args = Arc::new(Mutex::new(Vec::<OsString>::new()));
        let capture = Arc::clone(&observed_args);
        let mut executor = AudioExtractExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, move |command| {
                *capture.lock().unwrap() = command.arguments().to_vec();
                Ok(FakeProcessControl::running())
            })
            .unwrap();

        assert_eq!(executor.state(), &AudioExtractExecutionState::Running);
        let args = observed_args.lock().unwrap();
        assert!(args.windows(2).any(|pair| pair == ["-map", "0:3"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-metadata", "title=Program"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-metadata:s:a:0", "language=eng"])
        );
        let staged_output = PathBuf::from(args.last().unwrap());
        assert_ne!(staged_output, request.output);
        assert_eq!(staged_output.file_name(), request.output.file_name());
        assert!(!request.output.exists());
    }

    #[test]
    fn invalid_request_is_rejected_before_launch() {
        let directory = TestDirectory::new("invalid-no-launch");
        let (mut request, media) = valid_request_and_media(&directory);
        request.audio_stream_index = 99;
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = AudioExtractExecutor::new();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(error, AudioExtractStartError::Validation(_)));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &AudioExtractExecutionState::Idle);
    }

    #[test]
    fn destination_preflight_rejects_existing_output_before_launch() {
        let directory = TestDirectory::new("existing-output");
        let (request, media) = valid_request_and_media(&directory);
        fs::write(&request.output, b"occupied").unwrap();
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = AudioExtractExecutor::new();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(error, AudioExtractStartError::Output(_)));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &AudioExtractExecutionState::Idle);
    }

    #[test]
    fn success_failure_and_cancellation_settle_publication_once() {
        for outcome in [Outcome::Success, Outcome::Failure, Outcome::Cancellation] {
            let directory = TestDirectory::new(outcome.label());
            let (request, media) = valid_request_and_media(&directory);
            let tools = fake_tools(&directory);
            let process = FakeProcessControl::running();
            let handle = process.clone();
            let mut executor = AudioExtractExecutor::new();
            executor
                .start_with_launcher(&tools, &media, &request, |_| Ok(process))
                .unwrap();
            let temporary = temporary_output(&executor);
            fs::write(&temporary, b"extracted audio").unwrap();

            match outcome {
                Outcome::Success => {
                    handle.set_state(MediaProcessState::Completed);
                    assert_eq!(
                        executor.poll(),
                        &AudioExtractExecutionState::Completed {
                            output: request.output.clone(),
                        }
                    );
                    assert_eq!(fs::read(&request.output).unwrap(), b"extracted audio");
                }
                Outcome::Failure => {
                    handle.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
                        4,
                    ))));
                    assert_eq!(
                        executor.poll(),
                        &AudioExtractExecutionState::Failed(
                            AudioExtractExecutionFailure::Process {
                                reason: MediaProcessFailure::Exit(Some(4)),
                                diagnostic: String::new(),
                            }
                        )
                    );
                    assert!(!request.output.exists());
                    assert!(!temporary.exists());
                }
                Outcome::Cancellation => {
                    assert!(executor.cancel());
                    assert_eq!(executor.poll(), &AudioExtractExecutionState::Cancelled);
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

    fn valid_request_and_media(directory: &TestDirectory) -> (AudioExtractRequest, MediaFile) {
        let input = directory.path().join("source.mkv");
        fs::write(&input, b"source").unwrap();
        (
            AudioExtractRequest {
                input: input.clone(),
                output: directory.path().join("audio.m4a"),
                audio_stream_index: 3,
                format: AudioExtractFormat::Aac,
            },
            MediaFile {
                path: input,
                duration: None,
                streams: vec![MediaStream {
                    stream_index: 3,
                    kind: StreamKind::Audio,
                    codec: Some("opus".to_owned()),
                }],
                video_streams: Vec::new(),
                audio_streams: vec![AudioStream {
                    stream_index: 3,
                    codec: Some("opus".to_owned()),
                    channels: Some(2),
                    channel_layout: Some("stereo".to_owned()),
                    title: Some("Program".to_owned()),
                    language: Some("eng".to_owned()),
                }],
            },
        )
    }

    fn fake_tools(directory: &TestDirectory) -> MediaTools {
        MediaTools {
            ffmpeg: directory.path().join("fake-ffmpeg"),
            ffprobe: directory.path().join("fake-ffprobe"),
        }
    }

    fn temporary_output(executor: &AudioExtractExecutor) -> PathBuf {
        executor
            .execution
            .temporary_output_path()
            .expect("running extraction should own a staged output")
            .to_path_buf()
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
                "apollo-audio-extract-test-{}-{sequence}-{label}",
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
