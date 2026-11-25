use std::io;

use super::{
    execution::MediaExecutionFailure,
    intake::MediaIntakeError,
    output::MediaOutputError,
    probe::MediaProbeError,
    process::{DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES, MediaProcessFailure, MediaProcessStartError},
    tools::{MediaToolKind, MediaToolsError},
};

const NO_DIAGNOSTIC: &str = "No diagnostic details were reported.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaOutputContainer {
    Mp4,
    Mkv,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaErrorKind {
    MediaToolsCheckFailed,
    FfmpegUnavailable,
    FfprobeUnavailable,
    MediaToolsUnavailable,
    IncompleteMediaToolsBundle,
    FfmpegValidationFailed,
    FfprobeValidationFailed,
    ProbeFailure,
    SourceUnavailable,
    MissingStream,
    Mp4Incompatibility,
    EncoderUnavailable,
    OutputNotWritable,
    FilePickerUnavailable,
    OutputPickerUnavailable,
    ClipboardUnavailable,
    FrameTooLargeToCopy,
    MultipleFiles,
    InvalidRequest,
    ChildProcessFailed,
    Cancellation,
    UnknownFfmpegFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaError {
    kind: MediaErrorKind,
    diagnostic: Option<String>,
}

impl MediaError {
    pub fn kind(&self) -> MediaErrorKind {
        self.kind
    }

    pub fn message(&self) -> &'static str {
        match self.kind {
            MediaErrorKind::MediaToolsCheckFailed => "Media tools could not be checked.",
            MediaErrorKind::FfmpegUnavailable => "FFmpeg is unavailable.",
            MediaErrorKind::FfprobeUnavailable => "FFprobe is unavailable.",
            MediaErrorKind::MediaToolsUnavailable => "FFmpeg and FFprobe are unavailable.",
            MediaErrorKind::IncompleteMediaToolsBundle => "The bundled media tools are incomplete.",
            MediaErrorKind::FfmpegValidationFailed => "FFmpeg could not be validated.",
            MediaErrorKind::FfprobeValidationFailed => "FFprobe could not be validated.",
            MediaErrorKind::ProbeFailure => "This file could not be inspected.",
            MediaErrorKind::SourceUnavailable => "This source file is unavailable.",
            MediaErrorKind::MissingStream => "A required media stream is missing or unsupported.",
            MediaErrorKind::Mp4Incompatibility => "A source stream cannot be written into MP4.",
            MediaErrorKind::EncoderUnavailable => "The required encoder is unavailable.",
            MediaErrorKind::OutputNotWritable => "The output location is not writable.",
            MediaErrorKind::FilePickerUnavailable => "The file picker could not be opened.",
            MediaErrorKind::OutputPickerUnavailable => "The output picker could not be opened.",
            MediaErrorKind::ClipboardUnavailable => "The frame could not be copied.",
            MediaErrorKind::FrameTooLargeToCopy => "This frame is too large to copy safely.",
            MediaErrorKind::MultipleFiles => "Drop one file at a time.",
            MediaErrorKind::InvalidRequest => "The requested media operation is not valid.",
            MediaErrorKind::ChildProcessFailed => "FFmpeg could not complete the media process.",
            MediaErrorKind::Cancellation => "The operation was cancelled.",
            MediaErrorKind::UnknownFfmpegFailure => "FFmpeg failed for an unknown reason.",
        }
    }

    pub fn guidance(&self) -> Option<&'static str> {
        match self.kind {
            MediaErrorKind::MediaToolsCheckFailed => Some("Retry the media tool check."),
            MediaErrorKind::IncompleteMediaToolsBundle => Some(
                "Restore both FFmpeg and FFprobe beside Apollo, or reinstall Apollo, then retry.",
            ),
            MediaErrorKind::FfmpegUnavailable => {
                Some("Install FFmpeg and make sure it is available on PATH, then retry.")
            }
            MediaErrorKind::FfprobeUnavailable => {
                Some("Install FFprobe and make sure it is available on PATH, then retry.")
            }
            MediaErrorKind::MediaToolsUnavailable => Some(
                "Install FFmpeg and FFprobe and make sure both are available on PATH, then retry.",
            ),
            MediaErrorKind::FfmpegValidationFailed => {
                Some("Check the installed FFmpeg executable and retry.")
            }
            MediaErrorKind::FfprobeValidationFailed => {
                Some("Check the installed FFprobe executable and retry.")
            }
            MediaErrorKind::ProbeFailure => {
                Some("Check that the source is a readable media file and try again.")
            }
            MediaErrorKind::SourceUnavailable => {
                Some("Choose an existing readable media file and try again.")
            }
            MediaErrorKind::MissingStream => {
                Some("Choose a source containing the required media streams.")
            }
            MediaErrorKind::Mp4Incompatibility => {
                Some("Use MKV or choose a source with MP4-compatible streams.")
            }
            MediaErrorKind::EncoderUnavailable => {
                Some("Use an FFmpeg build that includes the required encoder.")
            }
            MediaErrorKind::OutputNotWritable => {
                Some("Choose a writable output folder and try again.")
            }
            MediaErrorKind::FilePickerUnavailable => Some("Try Open files again."),
            MediaErrorKind::OutputPickerUnavailable => Some("Try Browse again."),
            MediaErrorKind::ClipboardUnavailable => {
                Some("Show details for the diagnostic, then try Copy frame again.")
            }
            MediaErrorKind::FrameTooLargeToCopy => {
                Some("Capture the frame to an image file instead.")
            }
            MediaErrorKind::MultipleFiles => None,
            MediaErrorKind::InvalidRequest => Some("Review the current settings and try again."),
            MediaErrorKind::ChildProcessFailed | MediaErrorKind::UnknownFfmpegFailure => {
                Some("Show details for the FFmpeg diagnostic, then retry.")
            }
            MediaErrorKind::Cancellation => None,
        }
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    pub fn diagnostic_or_fallback(&self) -> &str {
        self.diagnostic().unwrap_or(NO_DIAGNOSTIC)
    }

    pub fn is_failure(&self) -> bool {
        self.kind != MediaErrorKind::Cancellation
    }

    pub fn missing_stream(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::MissingStream, diagnostic)
    }

    pub fn invalid_request(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::InvalidRequest, diagnostic)
    }

    pub fn mp4_incompatibility(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::Mp4Incompatibility, diagnostic)
    }

    pub fn probe_failure(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::ProbeFailure, diagnostic)
    }

    pub fn source_unavailable(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::SourceUnavailable, diagnostic)
    }

    pub fn output_not_writable(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::OutputNotWritable, diagnostic)
    }

    pub fn output_picker_unavailable(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::OutputPickerUnavailable, diagnostic)
    }

    pub fn clipboard_unavailable(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::ClipboardUnavailable, diagnostic)
    }

    pub fn frame_too_large(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::FrameTooLargeToCopy, diagnostic)
    }

    pub fn file_picker_unavailable(diagnostic: impl Into<String>) -> Self {
        Self::with_diagnostic(MediaErrorKind::FilePickerUnavailable, diagnostic)
    }

    pub fn multiple_files() -> Self {
        Self::new(MediaErrorKind::MultipleFiles, None)
    }

    pub fn cancelled() -> Self {
        Self::new(MediaErrorKind::Cancellation, None)
    }

    pub fn from_tools_error(error: &MediaToolsError) -> Self {
        let kind = match error {
            MediaToolsError::DiscoveryFailed(_) => MediaErrorKind::MediaToolsCheckFailed,
            MediaToolsError::MissingFfmpeg => MediaErrorKind::FfmpegUnavailable,
            MediaToolsError::MissingFfprobe => MediaErrorKind::FfprobeUnavailable,
            MediaToolsError::MissingBoth => MediaErrorKind::MediaToolsUnavailable,
            MediaToolsError::PartialBundle { .. } => MediaErrorKind::IncompleteMediaToolsBundle,
            MediaToolsError::ValidationFailed {
                tool: MediaToolKind::Ffmpeg,
                ..
            } => MediaErrorKind::FfmpegValidationFailed,
            MediaToolsError::ValidationFailed {
                tool: MediaToolKind::Ffprobe,
                ..
            } => MediaErrorKind::FfprobeValidationFailed,
        };
        Self::with_diagnostic(kind, error.to_string())
    }

    pub fn from_intake_error(error: &MediaIntakeError) -> Self {
        match error {
            MediaIntakeError::Probe(error) => Self::from_probe_error(error),
            MediaIntakeError::Missing
            | MediaIntakeError::Directory
            | MediaIntakeError::Metadata(_) => {
                Self::with_diagnostic(MediaErrorKind::SourceUnavailable, error.to_string())
            }
        }
    }

    pub fn from_probe_error(error: &MediaProbeError) -> Self {
        match error {
            MediaProbeError::LaunchFailed(io::ErrorKind::NotFound) => {
                Self::with_diagnostic(MediaErrorKind::FfprobeUnavailable, error.to_string())
            }
            MediaProbeError::LaunchFailed(_) | MediaProbeError::InvalidOutput { .. } => {
                Self::with_diagnostic(MediaErrorKind::ProbeFailure, error.to_string())
            }
            MediaProbeError::FfprobeFailed { diagnostic, .. } => Self::new(
                MediaErrorKind::ProbeFailure,
                nonempty(diagnostic).or_else(|| Some(error.to_string())),
            ),
        }
    }

    pub fn from_output_error(error: &MediaOutputError) -> Self {
        let kind = match error {
            MediaOutputError::OutputParentUnavailable { .. }
            | MediaOutputError::TemporaryDirectoryCreationFailed { .. }
            | MediaOutputError::PublishFailed { .. }
            | MediaOutputError::CleanupFailed { .. } => MediaErrorKind::OutputNotWritable,
            MediaOutputError::InvalidOutputPath(_)
            | MediaOutputError::OutputParentNotDirectory(_)
            | MediaOutputError::TemporaryOutputMissing(_)
            | MediaOutputError::TemporaryOutputNotFile(_)
            | MediaOutputError::ProcessStillRunning
            | MediaOutputError::AlreadySettled => MediaErrorKind::InvalidRequest,
        };
        Self::with_diagnostic(kind, error.to_string())
    }

    pub fn from_process_start_error(error: &MediaProcessStartError) -> Self {
        let kind = match error {
            MediaProcessStartError::ProcessLaunchFailed(io::ErrorKind::NotFound) => {
                MediaErrorKind::FfmpegUnavailable
            }
            MediaProcessStartError::WorkerLaunchFailed(_)
            | MediaProcessStartError::ProcessLaunchFailed(_)
            | MediaProcessStartError::StreamReaderLaunchFailed(_)
            | MediaProcessStartError::WorkerTerminatedBeforeStart => {
                MediaErrorKind::ChildProcessFailed
            }
        };
        Self::with_diagnostic(kind, error.to_string())
    }

    pub fn from_execution_failure(
        failure: &MediaExecutionFailure,
        output_container: MediaOutputContainer,
    ) -> Self {
        match failure {
            MediaExecutionFailure::Output { reason, diagnostic } => {
                let mut classified = Self::from_output_error(reason);
                if !diagnostic.trim().is_empty() {
                    let combined = match classified.diagnostic.take() {
                        Some(reason) => format!("{reason}\n\n{diagnostic}"),
                        None => diagnostic.clone(),
                    };
                    classified.diagnostic = Some(bound_diagnostic(&combined));
                }
                classified
            }
            MediaExecutionFailure::Process { reason, diagnostic } => match reason {
                MediaProcessFailure::Io(_) => {
                    Self::new(MediaErrorKind::ChildProcessFailed, nonempty(diagnostic))
                }
                MediaProcessFailure::Exit(_) => {
                    let kind = classify_ffmpeg_diagnostic(diagnostic, output_container);
                    Self::new(kind, nonempty(diagnostic))
                }
            },
        }
    }

    fn with_diagnostic(kind: MediaErrorKind, diagnostic: impl Into<String>) -> Self {
        Self::new(kind, Some(diagnostic.into()))
    }

    fn new(kind: MediaErrorKind, diagnostic: Option<String>) -> Self {
        Self {
            kind,
            diagnostic: diagnostic
                .filter(|diagnostic| !diagnostic.trim().is_empty())
                .map(|diagnostic| bound_diagnostic(&diagnostic)),
        }
    }
}

fn classify_ffmpeg_diagnostic(
    diagnostic: &str,
    output_container: MediaOutputContainer,
) -> MediaErrorKind {
    let diagnostic = diagnostic.to_ascii_lowercase();

    if [
        "unknown encoder",
        "encoder not found",
        "error selecting an encoder",
        "requested encoder",
    ]
    .iter()
    .any(|pattern| diagnostic.contains(pattern))
    {
        return MediaErrorKind::EncoderUnavailable;
    }

    if output_container == MediaOutputContainer::Mp4
        && [
            "could not find tag for codec",
            "not currently supported in container",
            "codec not supported in container",
            "incompatible with output codec id",
        ]
        .iter()
        .any(|pattern| diagnostic.contains(pattern))
    {
        return MediaErrorKind::Mp4Incompatibility;
    }

    MediaErrorKind::UnknownFfmpegFailure
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

fn bound_diagnostic(diagnostic: &str) -> String {
    if diagnostic.len() <= DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES {
        return diagnostic.to_owned();
    }

    let mut start = diagnostic.len() - DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES;
    while !diagnostic.is_char_boundary(start) {
        start += 1;
    }
    diagnostic[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{MediaError, MediaErrorKind, MediaOutputContainer};
    use crate::media::{
        execution::MediaExecutionFailure,
        probe::MediaProbeError,
        process::{
            DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES, MediaProcessFailure, MediaProcessStartError,
        },
        tools::{MediaToolKind, MediaToolValidationError, MediaToolsError},
    };

    #[test]
    fn known_tool_and_probe_failures_are_classified() {
        assert_eq!(
            MediaError::from_tools_error(&MediaToolsError::DiscoveryFailed(
                "worker stopped".to_owned(),
            ))
            .kind(),
            MediaErrorKind::MediaToolsCheckFailed
        );
        assert_eq!(
            MediaError::from_tools_error(&MediaToolsError::MissingFfmpeg).kind(),
            MediaErrorKind::FfmpegUnavailable
        );
        assert_eq!(
            MediaError::from_tools_error(&MediaToolsError::MissingFfprobe).kind(),
            MediaErrorKind::FfprobeUnavailable
        );
        let probe_error = MediaError::from_probe_error(&MediaProbeError::FfprobeFailed {
            code: Some(1),
            diagnostic: "Invalid data found".to_owned(),
        });
        assert_eq!(probe_error.kind(), MediaErrorKind::ProbeFailure);
        assert_eq!(probe_error.diagnostic(), Some("Invalid data found"));
    }

    #[test]
    fn tool_validation_failure_is_not_presented_as_missing() {
        let error = MediaError::from_tools_error(&MediaToolsError::ValidationFailed {
            tool: MediaToolKind::Ffmpeg,
            path: PathBuf::from(r"C:\Media Tools\ffmpeg.exe"),
            reason: MediaToolValidationError::TimedOut,
        });

        assert_eq!(error.kind(), MediaErrorKind::FfmpegValidationFailed);
        assert!(
            error
                .diagnostic()
                .is_some_and(|diagnostic| diagnostic.contains("version query timed out"))
        );
        assert!(
            error
                .guidance()
                .is_some_and(|guidance| !guidance.contains("Install FFmpeg"))
        );
    }

    #[test]
    fn mp4_stream_incompatibility_is_container_aware() {
        let failure = process_failure(
            "Could not find tag for codec h264 in stream #0, codec not currently supported in container",
        );

        let mp4 = MediaError::from_execution_failure(&failure, MediaOutputContainer::Mp4);
        assert_eq!(mp4.kind(), MediaErrorKind::Mp4Incompatibility);
        assert!(mp4.message().contains("MP4"));
        assert!(
            mp4.guidance()
                .is_some_and(|guidance| guidance.contains("Use MKV"))
        );

        let mkv = MediaError::from_execution_failure(&failure, MediaOutputContainer::Mkv);
        assert_eq!(mkv.kind(), MediaErrorKind::UnknownFfmpegFailure);
        assert!(!mkv.message().contains("MP4"));
        assert!(
            !mkv.guidance()
                .is_some_and(|guidance| guidance.contains("Use MKV"))
        );
    }

    #[test]
    fn unavailable_encoder_is_recognized_from_ffmpeg_diagnostic() {
        for (container, diagnostic) in [
            (MediaOutputContainer::Mp4, "Unknown encoder 'libx265'"),
            (MediaOutputContainer::Mp4, "Unknown encoder 'aac'"),
            (MediaOutputContainer::Other, "Unknown encoder 'flac'"),
        ] {
            let error = MediaError::from_execution_failure(&process_failure(diagnostic), container);
            assert_eq!(error.kind(), MediaErrorKind::EncoderUnavailable);
            assert_eq!(error.diagnostic(), Some(diagnostic));
            assert!(error.guidance().is_some());
        }
    }

    #[test]
    fn child_process_failure_is_distinct_from_unknown_ffmpeg_exit() {
        assert_eq!(
            MediaError::from_process_start_error(
                &MediaProcessStartError::WorkerTerminatedBeforeStart
            )
            .kind(),
            MediaErrorKind::ChildProcessFailed
        );
        assert_eq!(
            MediaError::from_execution_failure(
                &process_failure("unexpected ffmpeg detail"),
                MediaOutputContainer::Mp4,
            )
            .kind(),
            MediaErrorKind::UnknownFfmpegFailure
        );
    }

    #[test]
    fn raw_diagnostic_is_retained_but_bounded() {
        let raw = "private raw stderr marker";
        let error =
            MediaError::from_execution_failure(&process_failure(raw), MediaOutputContainer::Mp4);
        assert_eq!(error.diagnostic(), Some(raw));
        assert!(!error.message().contains(raw));

        let huge = format!(
            "{}TAIL",
            "x".repeat(DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES * 2)
        );
        let bounded =
            MediaError::from_execution_failure(&process_failure(&huge), MediaOutputContainer::Mp4);
        let diagnostic = bounded.diagnostic().expect("diagnostic should be retained");
        assert!(diagnostic.len() <= DEFAULT_DIAGNOSTIC_STDERR_LIMIT_BYTES);
        assert!(diagnostic.ends_with("TAIL"));
    }

    #[test]
    fn clipboard_failures_and_oversized_frames_have_actionable_distinct_copy() {
        let clipboard = MediaError::clipboard_unavailable("OpenClipboard failed");
        assert_eq!(clipboard.kind(), MediaErrorKind::ClipboardUnavailable);
        assert!(clipboard.guidance().unwrap().contains("Copy frame"));
        assert_eq!(clipboard.diagnostic(), Some("OpenClipboard failed"));

        let oversized = MediaError::frame_too_large("16384 x 16384 exceeds 256 MiB");
        assert_eq!(oversized.kind(), MediaErrorKind::FrameTooLargeToCopy);
        assert_ne!(clipboard.message(), oversized.message());
        assert!(oversized.guidance().unwrap().contains("image file"));
    }

    #[test]
    fn cancellation_is_not_a_failure() {
        let error = MediaError::cancelled();
        assert_eq!(error.kind(), MediaErrorKind::Cancellation);
        assert!(!error.is_failure());
    }

    fn process_failure(diagnostic: &str) -> MediaExecutionFailure {
        MediaExecutionFailure::Process {
            reason: MediaProcessFailure::Exit(Some(1)),
            diagnostic: diagnostic.to_owned(),
        }
    }
}
