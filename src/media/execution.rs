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

    pub(crate) fn is_running(&self) -> bool {
        self.operation.is_some()
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
        if let Some(operation) = self.operation.as_mut() {
            operation.refresh_progress();
        }

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
            Err(reason) => {
                MediaExecutionState::Failed(MediaExecutionFailure::Output { reason, diagnostic })
            }
            Ok(disposition) => terminal_state(process_state, disposition, diagnostic),
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
    diagnostic: String,
) -> MediaExecutionState {
    match (process_state, disposition) {
        (MediaProcessState::Completed, MediaOutputDisposition::Published(output)) => {
            MediaExecutionState::Completed { output }
        }
        (MediaProcessState::Failed(reason), MediaOutputDisposition::Discarded) => {
            MediaExecutionState::Failed(MediaExecutionFailure::Process { reason, diagnostic })
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

pub(crate) trait ProcessControl {
    fn state(&self) -> MediaProcessState;
    fn cancel(&self) -> bool;
    fn diagnostic_stderr(&self) -> String;
    fn drain_progress(&self) -> Vec<String>;
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
}
