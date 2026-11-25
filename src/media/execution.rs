use std::path::PathBuf;

use super::{
    operation::PreparedMediaOperation,
    output::{MediaOutput, MediaOutputDisposition, MediaOutputError},
    process::{
        MediaProcess, MediaProcessCommand, MediaProcessFailure, MediaProcessStartError,
        MediaProcessState,
    },
    progress::{MediaProgress, MediaProgressParser},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaExecutionFailure {
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
pub enum MediaExecutionState {
    Idle,
    Running { output: PathBuf },
    Completed { output: PathBuf },
    Failed(MediaExecutionFailure),
    Cancelled,
}

impl MediaExecutionState {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed(_) | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaExecutionStartError {
    AlreadyRunning,
    Process(MediaProcessStartError),
}

pub(crate) struct MediaExecution {
    state: MediaExecutionState,
    operation: Option<RunningMediaOperation>,
}

impl Default for MediaExecution {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaExecution {
    pub(crate) fn new() -> Self {
        Self {
            state: MediaExecutionState::Idle,
            operation: None,
        }
    }

    pub(crate) fn state(&self) -> &MediaExecutionState {
        &self.state
    }

    pub(crate) fn progress(&self) -> Option<&MediaProgress> {
        self.operation
            .as_ref()
            .and_then(|operation| operation.progress.as_ref())
    }

    pub(crate) fn reset(&mut self) -> bool {
        if self.operation.is_some() {
            return false;
        }

        self.state = MediaExecutionState::Idle;
        true
    }

    pub(crate) fn cancel(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| operation.process.cancel())
    }

    pub(crate) fn poll(&mut self) -> &MediaExecutionState {
        let Some(operation) = self.operation.as_mut() else {
            return &self.state;
        };
        let process_state = operation.process.state();
        if process_state == MediaProcessState::Running {
            operation.refresh_progress();
            return &self.state;
        }

        let mut operation = self.operation.take().expect("terminal operation exists");
        self.state = match operation.output.settle(&process_state) {
            Err(reason) => MediaExecutionState::Failed(MediaExecutionFailure::Output {
                reason,
                diagnostic: operation.process.diagnostic_stderr(),
            }),
            Ok(disposition) => terminal_state(process_state, disposition, &*operation.process),
        };
        &self.state
    }

    pub(crate) fn start_prepared_with_launcher<F, P>(
        &mut self,
        operation: PreparedMediaOperation,
        launcher: F,
    ) -> Result<(), MediaExecutionStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: ProcessControl + 'static,
    {
        if self.operation.is_some() {
            return Err(MediaExecutionStartError::AlreadyRunning);
        }

        let (output, command, duration) = operation.into_parts();
        let process = launcher(command).map_err(MediaExecutionStartError::Process)?;
        self.state = MediaExecutionState::Running {
            output: output.final_path().to_path_buf(),
        };
        self.operation = Some(RunningMediaOperation {
            process: Box::new(process),
            output,
            progress_parser: MediaProgressParser::new(duration),
            progress: None,
        });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn temporary_output_path(&self) -> Option<&std::path::Path> {
        self.operation
            .as_ref()
            .map(|operation| operation.output.temporary_path())
    }
}

fn terminal_state(
    process_state: MediaProcessState,
    disposition: MediaOutputDisposition,
    process: &dyn ProcessControl,
) -> MediaExecutionState {
    match (process_state, disposition) {
        (MediaProcessState::Completed, MediaOutputDisposition::Published(output)) => {
            MediaExecutionState::Completed { output }
        }
        (MediaProcessState::Failed(reason), MediaOutputDisposition::Discarded) => {
            MediaExecutionState::Failed(MediaExecutionFailure::Process {
                reason,
                diagnostic: process.diagnostic_stderr(),
            })
        }
        (MediaProcessState::Cancelled, MediaOutputDisposition::Discarded) => {
            MediaExecutionState::Cancelled
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

struct RunningMediaOperation {
    process: Box<dyn ProcessControl>,
    output: MediaOutput,
    progress_parser: MediaProgressParser,
    progress: Option<MediaProgress>,
}

impl RunningMediaOperation {
    fn refresh_progress(&mut self) {
        for line in self.process.drain_progress() {
            if let Some(progress) = self.progress_parser.push_line(&line) {
                self.progress = Some(progress);
            }
        }
    }
}

impl Drop for RunningMediaOperation {
    fn drop(&mut self) {
        if self.process.state() == MediaProcessState::Running {
            self.process.stop_and_wait();
        }
        // MediaOutput drops only after the child has stopped writing.
    }
}

pub(crate) trait ProcessControl {
    fn state(&self) -> MediaProcessState;
    fn cancel(&self) -> bool;
    fn diagnostic_stderr(&self) -> String;
    fn drain_progress(&self) -> Vec<String>;
    // Simulated processes settle synchronously on cancellation. The real owner joins its worker.
    fn stop_and_wait(&mut self) {
        self.cancel();
    }
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

    fn drain_progress(&self) -> Vec<String> {
        self.progress_channel().try_iter().collect()
    }

    fn stop_and_wait(&mut self) {
        MediaProcess::stop_and_wait(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{
        process::{MediaProcessEvent, MediaProcessWake},
        tools::MediaTools,
    };
    use std::{ffi::OsString, fs, sync::mpsc, time::Duration};

    #[test]
    fn dropping_active_file_execution_releases_child_before_staging_cleanup() {
        let directory =
            std::env::temp_dir().join(format!("apollo-execution-drop-{}", std::process::id()));
        // This test owns a unique directory for its process; refuse to reuse an existing one.
        fs::create_dir(&directory).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let destination = directory.join("result.mp4");
        fs::write(&destination, b"existing").unwrap();
        let output = MediaOutput::prepare(&destination).unwrap();
        let temporary = output.temporary_path().to_path_buf();
        let staging = temporary.parent().unwrap().to_path_buf();
        fs::write(&temporary, b"partial").unwrap();
        #[cfg(windows)]
        let (program, args) = (
            "cmd.exe",
            vec![
                "/D",
                "/S",
                "/C",
                "echo ready& for /L %i in (1,1,2147483647) do @rem",
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = ("sh", vec!["-c", "printf 'ready\\n'; exec sleep 60"]);
        let tools = MediaTools::new(program, program);
        let operation = PreparedMediaOperation::new(
            output,
            &tools,
            args.into_iter().map(OsString::from).collect(),
            None,
        );
        let (tx, rx) = mpsc::channel();
        let staged = temporary.clone();
        let wake = MediaProcessWake::new(move |event| {
            let _ = tx.send((event, staged.exists()));
        });
        let mut execution = MediaExecution::new();
        execution
            .start_prepared_with_launcher(operation, |command| {
                MediaProcess::start_with_wake(command, wake)
            })
            .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap().0,
            MediaProcessEvent::Progress
        );
        drop(execution);
        assert_eq!(rx.try_recv().unwrap(), (MediaProcessEvent::Terminal, true));
        assert!(!staging.exists());
        assert_eq!(fs::read(&destination).unwrap(), b"existing");
        assert!(!directory.join("result (1).mp4").exists());
    }
}
