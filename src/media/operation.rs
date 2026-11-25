use std::{ffi::OsString, time::Duration};

use super::{
    output::MediaOutput,
    process::{MediaProcessCommand, ProcessProgressSource},
    tools::MediaTools,
};

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
        tools: &MediaTools,
        args: Vec<OsString>,
        duration: Option<Duration>,
    ) -> Self {
        let command = MediaProcessCommand::new(&tools.ffmpeg)
            .args(args)
            .progress_source(ProcessProgressSource::Stdout);
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
