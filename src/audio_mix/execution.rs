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
    command::{AudioMixCommandError, build_audio_mix_invocation},
    model::AudioMixRequest,
    validation::{AudioMixValidationError, validate_audio_mix_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioMixStartError {
    AlreadyRunning,
    Validation(AudioMixValidationError),
    Output(MediaOutputError),
    Command(AudioMixCommandError),
    Process(MediaProcessStartError),
}

impl fmt::Display for AudioMixStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "an audio mix operation is already running"),
            Self::Validation(error) => write!(f, "audio mix request is invalid: {error}"),
            Self::Output(error) => write!(f, "audio mix output could not be prepared: {error}"),
            Self::Command(error) => write!(f, "audio mix command could not be built: {error}"),
            Self::Process(error) => write!(f, "audio mix process could not start: {error}"),
        }
    }
}

impl std::error::Error for AudioMixStartError {}

impl AudioMixStartError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                AudioMixValidationError::MissingVideoStream
                | AudioMixValidationError::NoAudioStreamsSelected
                | AudioMixValidationError::UnknownAudioStream(_) => {
                    MediaError::missing_stream(error.to_string())
                }
                AudioMixValidationError::Output(error) => MediaError::from_output_error(error),
                AudioMixValidationError::DuplicateSelectedStream(_)
                | AudioMixValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
            },
            Self::Output(error) => MediaError::from_output_error(error),
            Self::Command(error) => MediaError::missing_stream(error.to_string()),
            Self::Process(error) => MediaError::from_process_start_error(error),
            Self::AlreadyRunning => MediaError::invalid_request(self.to_string()),
        }
    }
}

pub type AudioMixExecutionFailure = MediaExecutionFailure;
pub type AudioMixExecutionState = MediaExecutionState;

pub struct AudioMixExecutor {
    execution: MediaExecution,
}

impl Default for AudioMixExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioMixExecutor {
    pub fn new() -> Self {
        Self {
            execution: MediaExecution::new(),
        }
    }

    pub fn state(&self) -> &AudioMixExecutionState {
        self.execution.state()
    }

    pub fn is_running(&self) -> bool {
        self.execution.is_running()
    }

    pub fn progress(&self) -> Option<&MediaProgress> {
        self.execution.progress()
    }

    /// Return a settled operation to its idle state so the existing configuration can be used
    /// again. Running work must be cancelled and settled before it can be reset.
    pub fn reset(&mut self) -> bool {
        self.execution.reset()
    }

    pub fn start(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &AudioMixRequest,
    ) -> Result<(), AudioMixStartError> {
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

    /// Refresh the operation lifecycle and settle output exactly once after process termination.
    pub fn poll(&mut self) -> &AudioMixExecutionState {
        self.execution.poll()
    }

    fn start_with_launcher<F, P>(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &AudioMixRequest,
        launcher: F,
    ) -> Result<(), AudioMixStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.execution.is_running() {
            return Err(AudioMixStartError::AlreadyRunning);
        }

        validate_audio_mix_request(request, media).map_err(AudioMixStartError::Validation)?;

        let output = MediaOutput::prepare(&request.input, &request.output)
            .map_err(AudioMixStartError::Output)?;
        let mut staged_request = request.clone();
        staged_request.output = output.temporary_path().to_path_buf();
        let invocation = build_audio_mix_invocation(&staged_request, media)
            .map_err(AudioMixStartError::Command)?;
        let command = MediaProcessCommand::new(&tools.ffmpeg)
            .args(invocation.args)
            .progress_source(ProcessProgressSource::Stdout);

        self.execution
            .start_with_launcher(output, command, media.duration, launcher)
            .map_err(map_execution_start_error)
    }
}

fn map_execution_start_error(error: MediaExecutionStartError) -> AudioMixStartError {
    match error {
        MediaExecutionStartError::AlreadyRunning => AudioMixStartError::AlreadyRunning,
        MediaExecutionStartError::Process(error) => AudioMixStartError::Process(error),
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

    use crate::media::{
        error::MediaErrorKind,
        model::{AudioStream, MediaFile, VideoStream},
        output::MediaOutputError,
        process::{MediaProcessCommand, MediaProcessFailure, MediaProcessState},
        tools::MediaTools,
    };

    use super::{
        AudioMixExecutionFailure, AudioMixExecutionState, AudioMixExecutor, AudioMixStartError,
    };
    use crate::audio_mix::{
        model::{AudioMixRequest, OutputContainer},
        validation::AudioMixValidationError,
    };
    use crate::media::execution::ProcessControl;

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn prelaunch_validation_maps_to_shared_user_error_categories() {
        let missing_stream =
            AudioMixStartError::Validation(AudioMixValidationError::NoAudioStreamsSelected)
                .user_error();
        assert_eq!(missing_stream.kind(), MediaErrorKind::MissingStream);

        let output_not_writable = AudioMixStartError::Validation(AudioMixValidationError::Output(
            MediaOutputError::OutputParentUnavailable {
                path: PathBuf::from("missing-parent"),
                kind: std::io::ErrorKind::NotFound,
            },
        ))
        .user_error();
        assert_eq!(
            output_not_writable.kind(),
            MediaErrorKind::OutputNotWritable
        );
    }

    #[test]
    fn valid_request_starts_against_staged_output() {
        let directory = TestDirectory::new("valid-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let observed_args = Arc::new(Mutex::new(Vec::<OsString>::new()));
        let args_capture = Arc::clone(&observed_args);
        let control = FakeProcessControl::running();
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, move |command| {
                *args_capture.lock().unwrap() = command.arguments().to_vec();
                Ok(control)
            })
            .unwrap();

        assert!(executor.is_running());
        assert_eq!(executor.state(), &AudioMixExecutionState::Running);
        let args = observed_args.lock().unwrap();
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
        request.selected_streams.clear();
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let launch_count = Arc::clone(&launches);
        let mut executor = AudioMixExecutor::new();

        let error = executor
            .start_with_launcher(&tools, &media, &request, move |_| {
                launch_count.fetch_add(1, Ordering::Relaxed);
                Ok(FakeProcessControl::running())
            })
            .unwrap_err();

        assert!(matches!(error, AudioMixStartError::Validation(_)));
        assert_eq!(launches.load(Ordering::Relaxed), 0);
        assert_eq!(executor.state(), &AudioMixExecutionState::Idle);
        assert!(!executor.is_running());
    }

    #[test]
    fn successful_process_publishes_final_output() {
        let directory = TestDirectory::new("success-publishes");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let process_handle = process.clone();
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        write_temporary_output(&executor, b"complete media");
        process_handle.set_state(MediaProcessState::Completed);

        assert_eq!(
            executor.poll(),
            &AudioMixExecutionState::Completed {
                output: fs::canonicalize(request.output.parent().unwrap())
                    .unwrap()
                    .join(request.output.file_name().unwrap()),
            }
        );
        assert_eq!(fs::read(&request.output).unwrap(), b"complete media");
        assert!(!executor.is_running());
    }

    #[test]
    fn failed_process_discards_partial_output_and_retains_diagnostics() {
        let directory = TestDirectory::new("failure-discards");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::with_diagnostic("ffmpeg failure detail");
        let process_handle = process.clone();
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        let temporary = temporary_output(&executor);
        fs::write(&temporary, b"partial media").unwrap();
        process_handle.set_state(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
            7,
        ))));

        assert_eq!(
            executor.poll(),
            &AudioMixExecutionState::Failed(AudioMixExecutionFailure::Process {
                reason: MediaProcessFailure::Exit(Some(7)),
                diagnostic: "ffmpeg failure detail".to_owned(),
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
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        let temporary = temporary_output(&executor);
        fs::write(&temporary, b"partial media").unwrap();

        assert!(executor.cancel());
        assert_eq!(process_handle.state(), MediaProcessState::Cancelled);
        assert_eq!(executor.poll(), &AudioMixExecutionState::Cancelled);
        assert!(!request.output.exists());
        assert!(!temporary.exists());
        assert!(!executor.cancel());
    }

    #[test]
    fn second_start_is_rejected_while_operation_is_owned() {
        let directory = TestDirectory::new("duplicate-start");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let launches = Arc::new(AtomicUsize::new(0));
        let mut executor = AudioMixExecutor::new();

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

        assert_eq!(error, AudioMixStartError::AlreadyRunning);
        assert_eq!(launches.load(Ordering::Relaxed), 1);
        assert!(executor.is_running());
    }

    #[test]
    fn terminal_state_is_settled_only_once() {
        let directory = TestDirectory::new("single-settlement");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let process_handle = process.clone();
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        write_temporary_output(&executor, b"complete media");
        process_handle.set_state(MediaProcessState::Completed);

        let settled = executor.poll().clone();
        assert!(settled.is_terminal());
        assert_eq!(executor.poll(), &settled);
        assert_eq!(fs::read(&request.output).unwrap(), b"complete media");
    }

    #[test]
    fn running_progress_is_parsed_from_shared_process_progress_lines() {
        let directory = TestDirectory::new("progress-lines");
        let (request, mut media) = valid_request_and_media(&directory);
        media.duration = Some(std::time::Duration::from_secs(10));
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let progress_handle = process.clone();
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        progress_handle.push_progress(["frame=240", "out_time_us=2500000", "progress=continue"]);

        assert_eq!(executor.poll(), &AudioMixExecutionState::Running);
        let progress = executor.progress().expect("progress snapshot");
        assert_eq!(
            progress.processed,
            Some(std::time::Duration::from_millis(2_500))
        );
        assert_eq!(progress.total, media.duration);
        assert_eq!(progress.fraction, Some(0.25));
        assert_eq!(progress.frame, Some(240));
    }

    #[test]
    fn unknown_duration_progress_does_not_invent_fraction() {
        let directory = TestDirectory::new("unknown-progress");
        let (request, mut media) = valid_request_and_media(&directory);
        media.duration = None;
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let progress_handle = process.clone();
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        progress_handle.push_progress(["out_time_us=2500000", "progress=continue"]);
        executor.poll();

        let progress = executor.progress().expect("progress snapshot");
        assert_eq!(
            progress.processed,
            Some(std::time::Duration::from_millis(2_500))
        );
        assert_eq!(progress.total, None);
        assert_eq!(progress.fraction, None);
    }

    #[test]
    fn settled_executor_can_reset_to_idle_but_running_executor_cannot() {
        let directory = TestDirectory::new("reset");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let process = FakeProcessControl::running();
        let state_handle = process.clone();
        let mut executor = AudioMixExecutor::new();

        executor
            .start_with_launcher(&tools, &media, &request, |_| Ok(process))
            .unwrap();
        assert!(!executor.reset());

        write_temporary_output(&executor, b"mixed");
        state_handle.set_state(MediaProcessState::Completed);
        executor.poll();
        assert!(matches!(
            executor.state(),
            AudioMixExecutionState::Completed { .. }
        ));

        assert!(executor.reset());
        assert_eq!(executor.state(), &AudioMixExecutionState::Idle);
    }

    fn valid_request_and_media(directory: &TestDirectory) -> (AudioMixRequest, MediaFile) {
        let input = directory.path().join("source.mkv");
        fs::write(&input, b"source").unwrap();
        let request = AudioMixRequest {
            input: input.clone(),
            output: directory.path().join("mixed.mp4"),
            selected_streams: vec![1, 3],
            container: OutputContainer::Mp4,
        };
        let media = MediaFile {
            path: input,
            duration: None,
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
            }],
            audio_streams: vec![
                AudioStream {
                    stream_index: 1,
                    codec: Some("aac".to_owned()),
                    channels: Some(2),
                    channel_layout: Some("stereo".to_owned()),
                    title: Some("Desktop".to_owned()),
                    language: None,
                },
                AudioStream {
                    stream_index: 3,
                    codec: Some("aac".to_owned()),
                    channels: Some(1),
                    channel_layout: Some("mono".to_owned()),
                    title: Some("Microphone".to_owned()),
                    language: None,
                },
            ],
        };
        (request, media)
    }

    fn fake_tools(directory: &TestDirectory) -> MediaTools {
        MediaTools {
            ffmpeg: directory.path().join("fake-ffmpeg"),
            ffprobe: directory.path().join("fake-ffprobe"),
        }
    }

    fn temporary_output(executor: &AudioMixExecutor) -> PathBuf {
        executor
            .execution
            .temporary_output_path()
            .expect("running audio mix should own a temporary output")
            .to_path_buf()
    }

    fn write_temporary_output(executor: &AudioMixExecutor, bytes: &[u8]) {
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
                "apollo-audio-mix-execution-test-{}-{sequence}-{name}",
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

    #[test]
    fn production_command_keeps_ffmpeg_program_separate_from_arguments() {
        let directory = TestDirectory::new("structured-command");
        let tools = fake_tools(&directory);
        let command = MediaProcessCommand::new(&tools.ffmpeg).args(["-version"]);

        assert_eq!(command.program(), tools.ffmpeg);
        assert_eq!(command.arguments(), [OsString::from("-version")]);
    }
}
