use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const OUTPUT_CHANNEL_CAPACITY: usize = 256;
const STREAM_READ_CHUNK_BYTES: usize = 4 * 1024;
const LIVE_OUTPUT_MESSAGE_LIMIT_BYTES: usize = 16 * 1024;
pub const DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaProcessEvent {
    Progress,
    Terminal,
}

#[derive(Clone, Default)]
pub struct MediaProcessWake {
    callback: Option<Arc<dyn Fn(MediaProcessEvent) + Send + Sync>>,
}

impl MediaProcessWake {
    pub fn new(callback: impl Fn(MediaProcessEvent) + Send + Sync + 'static) -> Self {
        Self {
            callback: Some(Arc::new(callback)),
        }
    }

    pub fn notify(&self, event: MediaProcessEvent) {
        if let Some(callback) = self.callback.as_ref() {
            callback(event);
        }
    }
}

#[cfg(windows)]
pub(super) fn background_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
pub(super) fn background_command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProcessProgressSource {
    #[default]
    None,
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaProcessCommand {
    program: PathBuf,
    args: Vec<OsString>,
    progress_source: ProcessProgressSource,
    captured_stdout_limit: usize,
    diagnostic_stderr_limit: usize,
}

impl MediaProcessCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            progress_source: ProcessProgressSource::None,
            captured_stdout_limit: 0,
            diagnostic_stderr_limit: DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES,
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn progress_source(mut self, source: ProcessProgressSource) -> Self {
        self.progress_source = source;
        self
    }

    pub fn diagnostic_stderr_limit(mut self, bytes: usize) -> Self {
        self.diagnostic_stderr_limit = bytes;
        self
    }

    pub fn captured_stdout_limit(mut self, bytes: usize) -> Self {
        self.captured_stdout_limit = bytes;
        self
    }

    pub fn program(&self) -> &std::path::Path {
        &self.program
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.args
    }

    #[cfg(test)]
    pub(crate) fn captured_stdout_limit_bytes(&self) -> usize {
        self.captured_stdout_limit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaProcessFailure {
    Exit(Option<i32>),
    Io(io::ErrorKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaProcessState {
    Running,
    Completed,
    Failed(MediaProcessFailure),
    Cancelled,
}

impl MediaProcessState {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaProcessStartError {
    WorkerLaunchFailed(io::ErrorKind),
    ProcessLaunchFailed(io::ErrorKind),
    StreamReaderLaunchFailed(io::ErrorKind),
    WorkerTerminatedBeforeStart,
}

impl fmt::Display for MediaProcessStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkerLaunchFailed(kind) => {
                write!(f, "media process worker could not start ({kind:?})")
            }
            Self::ProcessLaunchFailed(kind) => {
                write!(f, "media process could not start ({kind:?})")
            }
            Self::StreamReaderLaunchFailed(kind) => {
                write!(f, "media process output reader could not start ({kind:?})")
            }
            Self::WorkerTerminatedBeforeStart => {
                write!(
                    f,
                    "media process worker terminated before startup completed"
                )
            }
        }
    }
}

impl std::error::Error for MediaProcessStartError {}

pub struct MediaProcess {
    lifecycle: Arc<ProcessLifecycle>,
    captured_stdout: Arc<Mutex<BoundedDiagnostic>>,
    diagnostics: Arc<Mutex<BoundedDiagnostic>>,
    stdout_rx: Receiver<String>,
    stderr_rx: Receiver<String>,
    progress_rx: Receiver<String>,
}

impl MediaProcess {
    pub fn start(command: MediaProcessCommand) -> Result<Self, MediaProcessStartError> {
        Self::start_with_wake(command, MediaProcessWake::default())
    }

    pub fn start_with_wake(
        command: MediaProcessCommand,
        wake: MediaProcessWake,
    ) -> Result<Self, MediaProcessStartError> {
        let lifecycle = Arc::new(ProcessLifecycle::new());
        let captured_stdout = Arc::new(Mutex::new(BoundedDiagnostic::new(
            command.captured_stdout_limit,
        )));
        let diagnostics = Arc::new(Mutex::new(BoundedDiagnostic::new(
            command.diagnostic_stderr_limit,
        )));

        let (stdout_tx, stdout_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let (stderr_tx, stderr_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let (progress_tx, progress_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let (startup_tx, startup_rx) = sync_channel(1);

        let worker_lifecycle = Arc::clone(&lifecycle);
        let worker_captured_stdout = Arc::clone(&captured_stdout);
        let worker_diagnostics = Arc::clone(&diagnostics);
        thread::Builder::new()
            .name("apollo-media-process".to_owned())
            .spawn(move || {
                run_worker(
                    command,
                    worker_lifecycle,
                    worker_captured_stdout,
                    worker_diagnostics,
                    wake,
                    WorkerChannels {
                        stdout_tx,
                        stderr_tx,
                        progress_tx,
                        startup_tx,
                    },
                );
            })
            .map_err(|error| MediaProcessStartError::WorkerLaunchFailed(error.kind()))?;

        match startup_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                lifecycle,
                captured_stdout,
                diagnostics,
                stdout_rx,
                stderr_rx,
                progress_rx,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(MediaProcessStartError::WorkerTerminatedBeforeStart),
        }
    }

    pub fn state(&self) -> MediaProcessState {
        self.lifecycle.state()
    }

    pub fn cancel(&self) -> bool {
        self.lifecycle.request_cancel()
    }

    pub fn stdout_channel(&self) -> &Receiver<String> {
        &self.stdout_rx
    }

    pub fn stderr_channel(&self) -> &Receiver<String> {
        &self.stderr_rx
    }

    pub fn progress_channel(&self) -> &Receiver<String> {
        &self.progress_rx
    }

    pub fn diagnostic_stderr(&self) -> String {
        lock_unpoisoned(&self.diagnostics).as_lossy_string()
    }

    pub fn captured_stdout(&self) -> String {
        lock_unpoisoned(&self.captured_stdout).as_lossy_string()
    }

    pub fn captured_stdout_bytes(&self) -> Vec<u8> {
        lock_unpoisoned(&self.captured_stdout).bytes.clone()
    }
}

impl Drop for MediaProcess {
    fn drop(&mut self) {
        self.lifecycle.request_cancel();
    }
}

#[derive(Debug)]
struct ProcessLifecycle {
    inner: Mutex<ProcessLifecycleInner>,
    changed: Condvar,
}

#[derive(Debug)]
struct ProcessLifecycleInner {
    state: MediaProcessState,
    cancel_requested: bool,
    // Claimed as soon as the worker observes termination, before readers finish draining.
    // Once claimed, cancellation is rejected and cannot rewrite the outcome.
    terminal_claim: Option<MediaProcessState>,
}

impl ProcessLifecycle {
    fn new() -> Self {
        Self {
            inner: Mutex::new(ProcessLifecycleInner {
                state: MediaProcessState::Running,
                cancel_requested: false,
                terminal_claim: None,
            }),
            changed: Condvar::new(),
        }
    }

    fn state(&self) -> MediaProcessState {
        lock_unpoisoned(&self.inner).state.clone()
    }

    fn request_cancel(&self) -> bool {
        let mut inner = lock_unpoisoned(&self.inner);
        if inner.state.is_terminal() || inner.terminal_claim.is_some() || inner.cancel_requested {
            return false;
        }

        inner.cancel_requested = true;
        self.changed.notify_all();
        true
    }

    fn poll_owned_child(&self, child: &mut Child) -> ChildPoll {
        let mut inner = lock_unpoisoned(&self.inner);

        if inner.terminal_claim.is_some() {
            return ChildPoll::TerminalClaimed;
        }

        if inner.cancel_requested {
            inner.terminal_claim = Some(MediaProcessState::Cancelled);
            self.changed.notify_all();
            return ChildPoll::CancelClaimed;
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                inner.terminal_claim = Some(if status.success() {
                    MediaProcessState::Completed
                } else {
                    MediaProcessState::Failed(MediaProcessFailure::Exit(status.code()))
                });
                self.changed.notify_all();
                ChildPoll::TerminalClaimed
            }
            Ok(None) => ChildPoll::Running,
            Err(error) => {
                inner.terminal_claim = Some(MediaProcessState::Failed(MediaProcessFailure::Io(
                    error.kind(),
                )));
                self.changed.notify_all();
                ChildPoll::IoFailureClaimed
            }
        }
    }

    #[cfg(test)]
    fn claim_terminal(&self, requested: MediaProcessState) -> bool {
        debug_assert!(requested.is_terminal());

        let mut inner = lock_unpoisoned(&self.inner);
        if inner.state.is_terminal() || inner.terminal_claim.is_some() {
            return false;
        }

        inner.terminal_claim = Some(if inner.cancel_requested {
            MediaProcessState::Cancelled
        } else {
            requested
        });
        self.changed.notify_all();
        true
    }

    fn publish_terminal_claim(&self) -> bool {
        let mut inner = lock_unpoisoned(&self.inner);
        if inner.state.is_terminal() {
            return false;
        }

        let Some(claimed) = inner.terminal_claim.clone() else {
            return false;
        };

        inner.state = claimed;
        self.changed.notify_all();
        true
    }

    #[cfg(all(test, not(windows)))]
    fn wait_for_terminal_claim(&self, timeout: Duration) -> Option<MediaProcessState> {
        let inner = lock_unpoisoned(&self.inner);
        let (inner, _) = self
            .changed
            .wait_timeout_while(inner, timeout, |inner| inner.terminal_claim.is_none())
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.terminal_claim.clone()
    }

    #[cfg(test)]
    fn wait_for_terminal(&self, timeout: Duration) -> MediaProcessState {
        let inner = lock_unpoisoned(&self.inner);
        let (inner, _) = self
            .changed
            .wait_timeout_while(inner, timeout, |inner| !inner.state.is_terminal())
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.state.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildPoll {
    Running,
    CancelClaimed,
    TerminalClaimed,
    IoFailureClaimed,
}

#[derive(Debug)]
struct BoundedDiagnostic {
    limit: usize,
    bytes: Vec<u8>,
}

impl BoundedDiagnostic {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            bytes: Vec::with_capacity(limit.min(4096)),
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if self.limit == 0 || bytes.is_empty() {
            return;
        }

        if bytes.len() >= self.limit {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - self.limit..]);
            return;
        }

        let required = self.bytes.len() + bytes.len();
        if required > self.limit {
            let remove = required - self.limit;
            self.bytes.drain(..remove);
        }
        self.bytes.extend_from_slice(bytes);
    }

    fn as_lossy_string(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

struct WorkerChannels {
    stdout_tx: SyncSender<String>,
    stderr_tx: SyncSender<String>,
    progress_tx: SyncSender<String>,
    startup_tx: SyncSender<Result<(), MediaProcessStartError>>,
}

fn run_worker(
    command: MediaProcessCommand,
    lifecycle: Arc<ProcessLifecycle>,
    captured_stdout: Arc<Mutex<BoundedDiagnostic>>,
    diagnostics: Arc<Mutex<BoundedDiagnostic>>,
    wake: MediaProcessWake,
    channels: WorkerChannels,
) {
    let WorkerChannels {
        stdout_tx,
        stderr_tx,
        progress_tx,
        startup_tx,
    } = channels;
    let mut child = match spawn_child(&command) {
        Ok(child) => child,
        Err(error) => {
            let _ = startup_tx.send(Err(MediaProcessStartError::ProcessLaunchFailed(
                error.kind(),
            )));
            return;
        }
    };

    let stdout = child
        .stdout
        .take()
        .expect("stdout is piped by media process owner");
    let stderr = child
        .stderr
        .take()
        .expect("stderr is piped by media process owner");

    let stdout_progress = (command.progress_source == ProcessProgressSource::Stdout)
        .then(|| (progress_tx.clone(), wake.clone()));
    let stderr_progress = (command.progress_source == ProcessProgressSource::Stderr)
        .then(|| (progress_tx.clone(), wake.clone()));
    drop(progress_tx);

    let stdout_capture = (command.captured_stdout_limit > 0).then_some(captured_stdout);
    let stdout_reader =
        match spawn_stream_reader("stdout", stdout, stdout_tx, stdout_progress, stdout_capture) {
            Ok(reader) => reader,
            Err(error) => {
                stop_owned_child(&mut child);
                let _ = startup_tx.send(Err(MediaProcessStartError::StreamReaderLaunchFailed(
                    error.kind(),
                )));
                return;
            }
        };

    let stderr_reader = match spawn_stream_reader(
        "stderr",
        stderr,
        stderr_tx,
        stderr_progress,
        Some(diagnostics),
    ) {
        Ok(reader) => reader,
        Err(error) => {
            stop_owned_child(&mut child);
            let _ = stdout_reader.join();
            let _ = startup_tx.send(Err(MediaProcessStartError::StreamReaderLaunchFailed(
                error.kind(),
            )));
            return;
        }
    };

    if startup_tx.send(Ok(())).is_err() {
        stop_owned_child(&mut child);
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        return;
    }
    drop(startup_tx);

    run_owned_child(&mut child, &lifecycle);

    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    if lifecycle.publish_terminal_claim() {
        wake.notify(MediaProcessEvent::Terminal);
    }
}

fn spawn_child(command: &MediaProcessCommand) -> io::Result<Child> {
    background_command(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn run_owned_child(child: &mut Child, lifecycle: &ProcessLifecycle) {
    loop {
        match lifecycle.poll_owned_child(child) {
            ChildPoll::Running => thread::sleep(PROCESS_POLL_INTERVAL),
            ChildPoll::CancelClaimed | ChildPoll::IoFailureClaimed => {
                stop_owned_child(child);
                return;
            }
            ChildPoll::TerminalClaimed => return,
        }
    }
}

fn stop_owned_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_stream_reader<R>(
    name: &str,
    stream: R,
    output_tx: SyncSender<String>,
    progress: Option<(SyncSender<String>, MediaProcessWake)>,
    diagnostics: Option<Arc<Mutex<BoundedDiagnostic>>>,
) -> io::Result<JoinHandle<()>>
where
    R: Read + Send + 'static,
{
    thread::Builder::new()
        .name(format!("apollo-media-{name}"))
        .spawn(move || read_stream(stream, output_tx, progress, diagnostics))
}

fn read_stream<R: Read>(
    mut stream: R,
    output_tx: SyncSender<String>,
    progress: Option<(SyncSender<String>, MediaProcessWake)>,
    diagnostics: Option<Arc<Mutex<BoundedDiagnostic>>>,
) {
    let mut chunk = [0_u8; STREAM_READ_CHUNK_BYTES];
    let mut line = Vec::with_capacity(LIVE_OUTPUT_MESSAGE_LIMIT_BYTES.min(STREAM_READ_CHUNK_BYTES));
    let mut fragmented = false;

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => {
                emit_stream_eof(&mut line, &output_tx, progress.as_ref());
                break;
            }
            Ok(read) => {
                let bytes = &chunk[..read];
                if let Some(diagnostics) = diagnostics.as_ref() {
                    lock_unpoisoned(diagnostics).push(bytes);
                }

                consume_stream_bytes(
                    bytes,
                    &mut line,
                    &mut fragmented,
                    &output_tx,
                    progress.as_ref(),
                );
            }
            Err(_) => break,
        }
    }
}

fn consume_stream_bytes(
    mut bytes: &[u8],
    line: &mut Vec<u8>,
    fragmented: &mut bool,
    output_tx: &SyncSender<String>,
    progress: Option<&(SyncSender<String>, MediaProcessWake)>,
) {
    while let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
        append_stream_fragment(&bytes[..newline], line, fragmented, output_tx, progress);
        emit_stream_line(line, fragmented, output_tx, progress);
        bytes = &bytes[newline + 1..];
    }

    append_stream_fragment(bytes, line, fragmented, output_tx, progress);
}

fn append_stream_fragment(
    mut bytes: &[u8],
    line: &mut Vec<u8>,
    fragmented: &mut bool,
    output_tx: &SyncSender<String>,
    progress: Option<&(SyncSender<String>, MediaProcessWake)>,
) {
    while !bytes.is_empty() {
        if line.len() == LIVE_OUTPUT_MESSAGE_LIMIT_BYTES {
            emit_stream_message(line, output_tx, progress);
            line.clear();
            *fragmented = true;
        }

        let available = LIVE_OUTPUT_MESSAGE_LIMIT_BYTES - line.len();
        let take = available.min(bytes.len());
        line.extend_from_slice(&bytes[..take]);
        bytes = &bytes[take..];
    }
}

fn emit_stream_line(
    line: &mut Vec<u8>,
    fragmented: &mut bool,
    output_tx: &SyncSender<String>,
    progress: Option<&(SyncSender<String>, MediaProcessWake)>,
) {
    if line.last() == Some(&b'\r') {
        line.pop();
    }

    if !line.is_empty() || !*fragmented {
        emit_stream_message(line, output_tx, progress);
    }

    line.clear();
    *fragmented = false;
}

fn emit_stream_eof(
    line: &mut Vec<u8>,
    output_tx: &SyncSender<String>,
    progress: Option<&(SyncSender<String>, MediaProcessWake)>,
) {
    if !line.is_empty() {
        emit_stream_message(line, output_tx, progress);
        line.clear();
    }
}

fn emit_stream_message(
    bytes: &[u8],
    output_tx: &SyncSender<String>,
    progress: Option<&(SyncSender<String>, MediaProcessWake)>,
) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let _ = try_send_bounded(output_tx, text.clone());
    if let Some((progress_tx, wake)) = progress
        && try_send_bounded(progress_tx, text)
    {
        wake.notify(MediaProcessEvent::Progress);
    }
}

fn try_send_bounded(sender: &SyncSender<String>, value: String) -> bool {
    match sender.try_send(value) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Read},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc::sync_channel,
        },
        time::Duration,
    };

    use super::{
        BoundedDiagnostic, LIVE_OUTPUT_MESSAGE_LIMIT_BYTES, MediaProcess, MediaProcessCommand,
        MediaProcessEvent, MediaProcessFailure, MediaProcessStartError, MediaProcessState,
        MediaProcessWake, ProcessLifecycle, ProcessProgressSource, STREAM_READ_CHUNK_BYTES,
        lock_unpoisoned, read_stream,
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn successful_process_completes_and_exposes_stdout_and_progress() {
        let process = MediaProcess::start(
            shell_command(success_script()).progress_source(ProcessProgressSource::Stdout),
        )
        .expect("helper process should start");

        assert_eq!(wait_for_process(&process), MediaProcessState::Completed);
        assert_eq!(drain(process.stdout_channel()), ["stdout-line"]);
        assert_eq!(drain(process.progress_channel()), ["stdout-line"]);
        assert!(drain(process.stderr_channel()).is_empty());
    }

    #[test]
    fn optional_stdout_capture_is_bounded_and_available_after_settlement() {
        let process =
            MediaProcess::start(shell_command(success_script()).captured_stdout_limit(64))
                .expect("helper process should start");

        assert_eq!(wait_for_process(&process), MediaProcessState::Completed);
        assert_eq!(process.captured_stdout().trim(), "stdout-line");
        assert!(matches!(
            process.captured_stdout_bytes().as_slice(),
            b"stdout-line\n" | b"stdout-line\r\n"
        ));
    }

    #[test]
    fn successful_process_wakes_for_progress_and_terminal_settlement() {
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let wake = MediaProcessWake::new(move |event| {
            let _ = event_tx.send(event);
        });
        let process = MediaProcess::start_with_wake(
            shell_command(success_script()).progress_source(ProcessProgressSource::Stdout),
            wake,
        )
        .expect("helper process should start");

        assert_eq!(wait_for_process(&process), MediaProcessState::Completed);
        assert_eq!(
            event_rx.recv_timeout(TEST_TIMEOUT).unwrap(),
            MediaProcessEvent::Progress
        );
        assert_eq!(
            event_rx.recv_timeout(TEST_TIMEOUT).unwrap(),
            MediaProcessEvent::Terminal
        );
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn non_zero_exit_settles_as_failed_and_retains_stderr() {
        let process = MediaProcess::start(shell_command(failure_script()))
            .expect("helper process should start");

        assert_eq!(
            wait_for_process(&process),
            MediaProcessState::Failed(MediaProcessFailure::Exit(Some(7)))
        );
        assert_eq!(drain(process.stderr_channel()), ["failure-detail"]);
        assert!(process.diagnostic_stderr().contains("failure-detail"));
    }

    #[test]
    fn cancellation_terminates_the_owned_child_and_settles_cancelled() {
        let process = MediaProcess::start(shell_command(long_running_script()))
            .expect("helper process should start");

        assert!(process.cancel());
        assert_eq!(wait_for_process(&process), MediaProcessState::Cancelled);
        assert!(!process.cancel());
    }

    #[test]
    fn dropping_the_owner_cancels_the_running_child() {
        let process = MediaProcess::start(shell_command(long_running_script()))
            .expect("helper process should start");
        let lifecycle = Arc::clone(&process.lifecycle);

        drop(process);

        assert_eq!(
            lifecycle.wait_for_terminal(TEST_TIMEOUT),
            MediaProcessState::Cancelled
        );
    }

    #[test]
    fn stderr_is_streamed_and_captured_for_successful_processes() {
        let process = MediaProcess::start(shell_command(stderr_success_script()))
            .expect("helper process should start");

        assert_eq!(wait_for_process(&process), MediaProcessState::Completed);
        assert_eq!(drain(process.stderr_channel()), ["diagnostic-one"]);
        assert_eq!(process.diagnostic_stderr().trim(), "diagnostic-one");
    }

    #[test]
    fn stream_reader_bounds_newline_free_stderr_and_progress_without_shell() {
        const PAYLOAD_BYTES: usize = LIVE_OUTPUT_MESSAGE_LIMIT_BYTES * 32;

        let max_read_request = Arc::new(AtomicUsize::new(0));
        let diagnostics = Arc::new(Mutex::new(BoundedDiagnostic::new(64)));
        let reader = TrackingRead::new(PAYLOAD_BYTES, Arc::clone(&max_read_request));
        let (stderr_tx, stderr_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY_FOR_TEST);
        let (progress_tx, progress_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY_FOR_TEST);

        read_stream(
            reader,
            stderr_tx,
            Some((progress_tx, MediaProcessWake::default())),
            Some(Arc::clone(&diagnostics)),
        );

        assert!(max_read_request.load(Ordering::Relaxed) <= STREAM_READ_CHUNK_BYTES);

        let stderr_messages = drain(&stderr_rx);
        assert!(!stderr_messages.is_empty());
        assert!(
            stderr_messages
                .iter()
                .all(|message| message.len() <= LIVE_OUTPUT_MESSAGE_LIMIT_BYTES)
        );

        let progress_messages = drain(&progress_rx);
        assert!(!progress_messages.is_empty());
        assert!(
            progress_messages
                .iter()
                .all(|message| message.len() <= LIVE_OUTPUT_MESSAGE_LIMIT_BYTES)
        );

        let diagnostic = lock_unpoisoned(&diagnostics).as_lossy_string();
        assert_eq!(diagnostic.len(), 64);
        assert!(diagnostic.ends_with("TAIL-MARKER"));
    }

    #[test]
    fn missing_executable_returns_typed_launch_error() {
        let missing = std::env::temp_dir().join(format!(
            "apollo-missing-media-process-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);

        let error = MediaProcess::start(MediaProcessCommand::new(missing))
            .err()
            .expect("missing executable must fail startup");

        assert_eq!(
            error,
            MediaProcessStartError::ProcessLaunchFailed(std::io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn successful_terminal_claim_rejects_later_cancellation() {
        let lifecycle = ProcessLifecycle::new();

        assert!(lifecycle.claim_terminal(MediaProcessState::Completed));
        assert!(!lifecycle.request_cancel());
        assert_eq!(lifecycle.state(), MediaProcessState::Running);
        assert!(lifecycle.publish_terminal_claim());
        assert_eq!(lifecycle.state(), MediaProcessState::Completed);
    }

    #[test]
    fn failed_terminal_claim_rejects_later_cancellation() {
        let lifecycle = ProcessLifecycle::new();
        let failure = MediaProcessState::Failed(MediaProcessFailure::Exit(Some(9)));

        assert!(lifecycle.claim_terminal(failure.clone()));
        assert!(!lifecycle.request_cancel());
        assert!(lifecycle.publish_terminal_claim());
        assert_eq!(lifecycle.state(), failure);
    }

    #[test]
    fn duplicate_terminal_settlement_is_rejected() {
        let lifecycle = ProcessLifecycle::new();

        assert!(lifecycle.claim_terminal(MediaProcessState::Completed));
        assert!(
            !lifecycle.claim_terminal(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
                9
            ))))
        );
        assert!(lifecycle.publish_terminal_claim());
        assert!(!lifecycle.publish_terminal_claim());
        assert_eq!(lifecycle.state(), MediaProcessState::Completed);
    }

    #[test]
    fn accepted_cancellation_wins_the_single_terminal_settlement() {
        let lifecycle = ProcessLifecycle::new();

        assert!(lifecycle.request_cancel());
        assert!(lifecycle.claim_terminal(MediaProcessState::Completed));
        assert!(!lifecycle.request_cancel());
        assert!(lifecycle.publish_terminal_claim());
        assert_eq!(lifecycle.state(), MediaProcessState::Cancelled);
        assert!(!lifecycle.publish_terminal_claim());
    }

    #[cfg(not(windows))]
    #[test]
    fn successful_child_exit_claim_during_reader_drain_rejects_cancellation() {
        let process = MediaProcess::start(shell_command(exit_with_delayed_stderr_close_script(0)))
            .expect("helper process should start");

        assert_eq!(
            process.lifecycle.wait_for_terminal_claim(TEST_TIMEOUT),
            Some(MediaProcessState::Completed)
        );
        assert_eq!(process.state(), MediaProcessState::Running);
        assert!(!process.cancel());
        assert_eq!(wait_for_process(&process), MediaProcessState::Completed);
    }

    #[cfg(not(windows))]
    #[test]
    fn failed_child_exit_claim_during_reader_drain_rejects_cancellation() {
        let process = MediaProcess::start(shell_command(exit_with_delayed_stderr_close_script(7)))
            .expect("helper process should start");
        let failure = MediaProcessState::Failed(MediaProcessFailure::Exit(Some(7)));

        assert_eq!(
            process.lifecycle.wait_for_terminal_claim(TEST_TIMEOUT),
            Some(failure.clone())
        );
        assert_eq!(process.state(), MediaProcessState::Running);
        assert!(!process.cancel());
        assert_eq!(wait_for_process(&process), failure);
    }

    const OUTPUT_CHANNEL_CAPACITY_FOR_TEST: usize = 64;

    struct TrackingRead {
        remaining_x: usize,
        tail: &'static [u8],
        tail_offset: usize,
        max_read_request: Arc<AtomicUsize>,
    }

    impl TrackingRead {
        fn new(total_bytes: usize, max_read_request: Arc<AtomicUsize>) -> Self {
            let tail = b"TAIL-MARKER";
            assert!(total_bytes > tail.len());
            Self {
                remaining_x: total_bytes - tail.len(),
                tail,
                tail_offset: 0,
                max_read_request,
            }
        }
    }

    impl Read for TrackingRead {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.max_read_request
                .fetch_max(buffer.len(), Ordering::Relaxed);

            if buffer.is_empty() {
                return Ok(0);
            }

            if self.remaining_x > 0 {
                let count = self.remaining_x.min(buffer.len());
                buffer[..count].fill(b'x');
                self.remaining_x -= count;
                return Ok(count);
            }

            if self.tail_offset < self.tail.len() {
                let remaining = &self.tail[self.tail_offset..];
                let count = remaining.len().min(buffer.len());
                buffer[..count].copy_from_slice(&remaining[..count]);
                self.tail_offset += count;
                return Ok(count);
            }

            Ok(0)
        }
    }

    fn wait_for_process(process: &MediaProcess) -> MediaProcessState {
        process.lifecycle.wait_for_terminal(TEST_TIMEOUT)
    }

    fn drain(receiver: &std::sync::mpsc::Receiver<String>) -> Vec<String> {
        receiver.try_iter().collect()
    }

    fn shell_command(script: String) -> MediaProcessCommand {
        #[cfg(windows)]
        {
            MediaProcessCommand::new("cmd.exe").args(["/D", "/S", "/C", &script])
        }

        #[cfg(not(windows))]
        {
            MediaProcessCommand::new("sh").args(["-c", &script])
        }
    }

    fn success_script() -> String {
        #[cfg(windows)]
        {
            "echo stdout-line".to_owned()
        }
        #[cfg(not(windows))]
        {
            "printf 'stdout-line\\n'".to_owned()
        }
    }

    fn failure_script() -> String {
        #[cfg(windows)]
        {
            "1>&2 echo failure-detail& exit /b 7".to_owned()
        }
        #[cfg(not(windows))]
        {
            "printf 'failure-detail\\n' >&2; exit 7".to_owned()
        }
    }

    fn stderr_success_script() -> String {
        #[cfg(windows)]
        {
            "1>&2 echo diagnostic-one".to_owned()
        }
        #[cfg(not(windows))]
        {
            "printf 'diagnostic-one\\n' >&2".to_owned()
        }
    }

    fn long_running_script() -> String {
        #[cfg(windows)]
        {
            "for /L %i in (1,1,2147483647) do @rem".to_owned()
        }
        #[cfg(not(windows))]
        {
            "while :; do :; done".to_owned()
        }
    }

    #[cfg(not(windows))]
    fn exit_with_delayed_stderr_close_script(exit_code: i32) -> String {
        format!("(sleep 1; printf 'reader-drained\\n' >&2) & exit {exit_code}")
    }
}
