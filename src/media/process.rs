use std::{
    ffi::OsString,
    fmt, io,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const OUTPUT_CHANNEL_CAPACITY: usize = 256;
pub const DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES: usize = 64 * 1024;

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
    diagnostic_stderr_limit: usize,
}

impl MediaProcessCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            progress_source: ProcessProgressSource::None,
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

    pub fn program(&self) -> &std::path::Path {
        &self.program
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.args
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
    diagnostics: Arc<Mutex<BoundedDiagnostic>>,
    stdout_rx: Receiver<String>,
    stderr_rx: Receiver<String>,
    progress_rx: Receiver<String>,
}

impl MediaProcess {
    pub fn start(command: MediaProcessCommand) -> Result<Self, MediaProcessStartError> {
        let lifecycle = Arc::new(ProcessLifecycle::new());
        let diagnostics = Arc::new(Mutex::new(BoundedDiagnostic::new(
            command.diagnostic_stderr_limit,
        )));

        let (stdout_tx, stdout_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let (stderr_tx, stderr_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let (progress_tx, progress_rx) = sync_channel(OUTPUT_CHANNEL_CAPACITY);
        let (startup_tx, startup_rx) = sync_channel(1);

        let worker_lifecycle = Arc::clone(&lifecycle);
        let worker_diagnostics = Arc::clone(&diagnostics);
        thread::Builder::new()
            .name("apollo-media-process".to_owned())
            .spawn(move || {
                run_worker(
                    command,
                    worker_lifecycle,
                    worker_diagnostics,
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
}

impl ProcessLifecycle {
    fn new() -> Self {
        Self {
            inner: Mutex::new(ProcessLifecycleInner {
                state: MediaProcessState::Running,
                cancel_requested: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn state(&self) -> MediaProcessState {
        lock_unpoisoned(&self.inner).state.clone()
    }

    fn request_cancel(&self) -> bool {
        let mut inner = lock_unpoisoned(&self.inner);
        if inner.state.is_terminal() {
            return false;
        }

        inner.cancel_requested = true;
        self.changed.notify_all();
        true
    }

    fn cancel_requested(&self) -> bool {
        lock_unpoisoned(&self.inner).cancel_requested
    }

    fn settle(&self, requested: MediaProcessState) -> bool {
        debug_assert!(requested.is_terminal());

        let mut inner = lock_unpoisoned(&self.inner);
        if inner.state.is_terminal() {
            return false;
        }

        inner.state = if inner.cancel_requested {
            MediaProcessState::Cancelled
        } else {
            requested
        };
        self.changed.notify_all();
        true
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
    diagnostics: Arc<Mutex<BoundedDiagnostic>>,
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

    let stdout_progress =
        (command.progress_source == ProcessProgressSource::Stdout).then(|| progress_tx.clone());
    let stderr_progress =
        (command.progress_source == ProcessProgressSource::Stderr).then(|| progress_tx.clone());
    drop(progress_tx);

    let stdout_reader =
        match spawn_stream_reader("stdout", stdout, stdout_tx, stdout_progress, None) {
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

    let terminal = run_owned_child(&mut child, &lifecycle);

    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    lifecycle.settle(terminal);
}

fn spawn_child(command: &MediaProcessCommand) -> io::Result<Child> {
    Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn run_owned_child(child: &mut Child, lifecycle: &ProcessLifecycle) -> MediaProcessState {
    loop {
        if lifecycle.cancel_requested() {
            stop_owned_child(child);
            return MediaProcessState::Cancelled;
        }

        match child.try_wait() {
            Ok(Some(status)) if status.success() => return MediaProcessState::Completed,
            Ok(Some(status)) => {
                return MediaProcessState::Failed(MediaProcessFailure::Exit(status.code()));
            }
            Ok(None) => thread::sleep(PROCESS_POLL_INTERVAL),
            Err(error) => {
                stop_owned_child(child);
                return MediaProcessState::Failed(MediaProcessFailure::Io(error.kind()));
            }
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
    progress_tx: Option<SyncSender<String>>,
    diagnostics: Option<Arc<Mutex<BoundedDiagnostic>>>,
) -> io::Result<JoinHandle<()>>
where
    R: Read + Send + 'static,
{
    thread::Builder::new()
        .name(format!("apollo-media-{name}"))
        .spawn(move || read_stream(stream, output_tx, progress_tx, diagnostics))
}

fn read_stream<R: Read>(
    stream: R,
    output_tx: SyncSender<String>,
    progress_tx: Option<SyncSender<String>>,
    diagnostics: Option<Arc<Mutex<BoundedDiagnostic>>>,
) {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();

    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Ok(_) => {
                if let Some(diagnostics) = diagnostics.as_ref() {
                    lock_unpoisoned(diagnostics).push(&bytes);
                }

                let text = String::from_utf8_lossy(strip_line_ending(&bytes)).into_owned();
                try_send_bounded(&output_tx, text.clone());
                if let Some(progress_tx) = progress_tx.as_ref() {
                    try_send_bounded(progress_tx, text);
                }
            }
            Err(_) => break,
        }
    }
}

fn strip_line_ending(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

fn try_send_bounded(sender: &SyncSender<String>, value: String) {
    match sender.try_send(value) {
        Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::{
        MediaProcess, MediaProcessCommand, MediaProcessFailure, MediaProcessStartError,
        MediaProcessState, ProcessLifecycle, ProcessProgressSource,
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
    fn diagnostic_stderr_keeps_only_the_bounded_tail() {
        let process =
            MediaProcess::start(shell_command(large_stderr_script()).diagnostic_stderr_limit(64))
                .expect("helper process should start");

        assert_eq!(wait_for_process(&process), MediaProcessState::Completed);
        let diagnostic = process.diagnostic_stderr();
        assert!(diagnostic.len() <= 64);
        assert!(diagnostic.contains("TAIL-MARKER"));
        assert!(!diagnostic.contains("line-00"));
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
    fn duplicate_terminal_settlement_is_rejected() {
        let lifecycle = ProcessLifecycle::new();

        assert!(lifecycle.settle(MediaProcessState::Completed));
        assert!(
            !lifecycle.settle(MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
                9
            ))))
        );
        assert_eq!(lifecycle.state(), MediaProcessState::Completed);
    }

    #[test]
    fn accepted_cancellation_wins_the_single_terminal_settlement() {
        let lifecycle = Arc::new(ProcessLifecycle::new());

        assert!(lifecycle.request_cancel());
        assert!(lifecycle.settle(MediaProcessState::Completed));
        assert_eq!(lifecycle.state(), MediaProcessState::Cancelled);
        assert!(!lifecycle.settle(MediaProcessState::Completed));
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
            "echo failure-detail 1>&2 & exit /b 7".to_owned()
        }
        #[cfg(not(windows))]
        {
            "printf 'failure-detail\\n' >&2; exit 7".to_owned()
        }
    }

    fn stderr_success_script() -> String {
        #[cfg(windows)]
        {
            "echo diagnostic-one 1>&2".to_owned()
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

    fn large_stderr_script() -> String {
        #[cfg(windows)]
        {
            let mut commands = (0..20)
                .map(|index| format!("echo line-{index:02}-xxxxxxxx 1^>^&2"))
                .collect::<Vec<_>>();
            commands.push("echo TAIL-MARKER 1^>^&2".to_owned());
            commands.join(" & ")
        }
        #[cfg(not(windows))]
        {
            let mut commands = (0..20)
                .map(|index| format!("printf 'line-{index:02}-xxxxxxxx\\n' >&2"))
                .collect::<Vec<_>>();
            commands.push("printf 'TAIL-MARKER\\n' >&2".to_owned());
            commands.join("; ")
        }
    }
}
