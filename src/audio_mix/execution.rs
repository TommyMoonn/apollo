use std::{fmt, path::PathBuf};

use crate::media::{
    model::MediaFile,
    output::{MediaOutput, MediaOutputDisposition, MediaOutputError},
    process::{
        MediaProcess, MediaProcessCommand, MediaProcessFailure, MediaProcessStartError,
        MediaProcessState,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioMixExecutionFailure {
    Process {
        reason: MediaProcessFailure,
        diagnostic: String,
    },
    Output {
        reason: MediaOutputError,
        diagnostic: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioMixExecutionState {
    Idle,
    Running,
    Completed { output: PathBuf },
    Failed(AudioMixExecutionFailure),
    Cancelled,
}

impl AudioMixExecutionState {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed(_) | Self::Cancelled
        )
    }
}

pub struct AudioMixExecutor {
    state: AudioMixExecutionState,
    operation: Option<RunningAudioMix>,
}

impl Default for AudioMixExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioMixExecutor {
    pub fn new() -> Self {
        Self {
            state: AudioMixExecutionState::Idle,
            operation: None,
        }
    }

    pub fn state(&self) -> &AudioMixExecutionState {
        &self.state
    }

    pub fn is_running(&self) -> bool {
        self.operation.is_some()
    }

    pub fn start(
        &mut self,
        tools: &MediaTools,
        media: &MediaFile,
        request: &AudioMixRequest,
    ) -> Result<(), AudioMixStartError> {
        self.start_with_launcher(tools, media, request, MediaProcess::start)
    }

    pub fn cancel(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| operation.process.cancel())
    }

    /// Refresh the operation lifecycle and settle output exactly once after process termination.
    ///
    /// Callers should poll this from their normal application update path. A terminal process is
    /// removed only after its temporary output has either been published or discarded, which also
    /// prevents a second Audio Mix operation from starting during settlement.
    pub fn poll(&mut self) -> &AudioMixExecutionState {
        let Some(process_state) = self
            .operation
            .as_ref()
            .map(|operation| operation.process.state())
        else {
            return &self.state;
        };

        if process_state == MediaProcessState::Running {
            return &self.state;
        }

        let mut operation = self
            .operation
            .take()
            .expect("operation exists while its terminal state is being settled");
        let diagnostic = operation.process.diagnostic_stderr();
        let settlement = operation.output.settle(&process_state);

        self.state = match settlement {
            Err(reason) => AudioMixExecutionState::Failed(AudioMixExecutionFailure::Output {
                reason,
                diagnostic,
            }),
            Ok(disposition) => terminal_state(process_state, disposition, diagnostic),
        };

        &self.state
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
        if self.operation.is_some() {
            return Err(AudioMixStartError::AlreadyRunning);
        }

        validate_audio_mix_request(request, media).map_err(AudioMixStartError::Validation)?;

        let output = MediaOutput::prepare(&request.input, &request.output)
            .map_err(AudioMixStartError::Output)?;
        let mut staged_request = request.clone();
        staged_request.output = output.temporary_path().to_path_buf();
        let invocation = build_audio_mix_invocation(&staged_request, media)
            .map_err(AudioMixStartError::Command)?;
        let command = MediaProcessCommand::new(&tools.ffmpeg).args(invocation.args);
        let process = launcher(command).map_err(AudioMixStartError::Process)?;

        self.operation = Some(RunningAudioMix {
            process: Box::new(process),
            output,
        });
        self.state = AudioMixExecutionState::Running;
        Ok(())
    }
}

fn terminal_state(
    process_state: MediaProcessState,
    disposition: MediaOutputDisposition,
    diagnostic: String,
) -> AudioMixExecutionState {
    match (process_state, disposition) {
        (MediaProcessState::Completed, MediaOutputDisposition::Published(output)) => {
            AudioMixExecutionState::Completed { output }
        }
        (MediaProcessState::Failed(reason), MediaOutputDisposition::Discarded) => {
            AudioMixExecutionState::Failed(AudioMixExecutionFailure::Process { reason, diagnostic })
        }
        (MediaProcessState::Cancelled, MediaOutputDisposition::Discarded) => {
            AudioMixExecutionState::Cancelled
        }
        (MediaProcessState::Running, _) => {
            unreachable!("running media output cannot be settled")
        }
        (MediaProcessState::Completed, MediaOutputDisposition::Discarded)
        | (
            MediaProcessState::Failed(_) | MediaProcessState::Cancelled,
            MediaOutputDisposition::Published(_),
        ) => unreachable!("media output disposition does not match terminal process state"),
    }
}

struct RunningAudioMix {
    process: Box<dyn ProcessControl>,
    output: MediaOutput,
}

trait ProcessControl {
    fn state(&self) -> MediaProcessState;
    fn cancel(&self) -> bool;
    fn diagnostic_stderr(&self) -> String;
}

impl ProcessControl for MediaProcess {
    fn state(&self) -> MediaProcessState {
        MediaProcess::state(self)
    }

    fn cancel(&self) -> bool {
        MediaProcess::cancel(self)
    }

    fn diagnostic_stderr(&self) -> String {
        MediaProcess::diagnostic_stderr(self)
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
        model::{AudioStream, MediaFile, VideoStream},
        process::{MediaProcessCommand, MediaProcessFailure, MediaProcessState},
        tools::MediaTools,
    };

    use super::{
        AudioMixExecutionFailure, AudioMixExecutionState, AudioMixExecutor, AudioMixStartError,
        ProcessControl,
    };
    use crate::audio_mix::model::{AudioMixRequest, OutputContainer};

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

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
            .operation
            .as_ref()
            .unwrap()
            .output
            .temporary_path()
            .to_path_buf()
    }

    fn write_temporary_output(executor: &AudioMixExecutor, bytes: &[u8]) {
        fs::write(temporary_output(executor), bytes).unwrap();
    }

    #[derive(Clone)]
    struct FakeProcessControl {
        state: Arc<Mutex<MediaProcessState>>,
        diagnostic: Arc<String>,
    }

    impl FakeProcessControl {
        fn running() -> Self {
            Self::with_diagnostic("")
        }

        fn with_diagnostic(diagnostic: &str) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Running)),
                diagnostic: Arc::new(diagnostic.to_owned()),
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
            self.diagnostic.as_str().to_owned()
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
