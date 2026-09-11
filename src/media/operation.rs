use std::time::Duration;

use super::{output::MediaOutput, process::MediaProcessCommand};

/// A fully validated media operation whose process can be launched without rebuilding request
/// state. Output settlement remains owned by `MediaExecution` after launch.
#[derive(Debug)]
pub(crate) struct PreparedMediaOperation {
    output: MediaOutput,
    command: MediaProcessCommand,
    duration: Option<Duration>,
}

impl PreparedMediaOperation {
    pub(crate) fn new(
        output: MediaOutput,
        command: MediaProcessCommand,
        duration: Option<Duration>,
    ) -> Self {
        Self {
            output,
            command,
            duration,
        }
    }

    pub(crate) fn into_parts(self) -> (MediaOutput, MediaProcessCommand, Option<Duration>) {
        (self.output, self.command, self.duration)
    }
}
