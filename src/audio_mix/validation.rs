use std::{
    collections::HashSet,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::media::model::MediaFile;

use super::model::AudioMixRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioMixValidationError {
    MissingVideoStream,
    NoAudioStreamsSelected,
    DuplicateSelectedStream(u32),
    UnknownAudioStream(u32),
    SameInputAndOutput(PathBuf),
    OutputExtensionMismatch {
        expected: &'static str,
        actual: Option<String>,
    },
    OutputParentUnavailable {
        path: PathBuf,
        kind: io::ErrorKind,
    },
    OutputParentNotDirectory(PathBuf),
}

impl fmt::Display for AudioMixValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => write!(f, "audio mix requires a video stream"),
            Self::NoAudioStreamsSelected => {
                write!(f, "at least one audio stream must be selected")
            }
            Self::DuplicateSelectedStream(stream_index) => {
                write!(f, "audio stream {stream_index} is selected more than once")
            }
            Self::UnknownAudioStream(stream_index) => {
                write!(
                    f,
                    "audio stream {stream_index} does not exist in the probed media"
                )
            }
            Self::SameInputAndOutput(path) => {
                write!(f, "input and output must differ: {}", path.display())
            }
            Self::OutputExtensionMismatch { expected, actual } => match actual {
                Some(actual) => write!(
                    f,
                    "output extension '.{actual}' does not match the selected '.{expected}' container"
                ),
                None => write!(
                    f,
                    "output path must use the '.{expected}' extension for the selected container"
                ),
            },
            Self::OutputParentUnavailable { path, kind } => write!(
                f,
                "output parent is unavailable ({}): {kind:?}",
                path.display()
            ),
            Self::OutputParentNotDirectory(path) => {
                write!(f, "output parent is not a directory: {}", path.display())
            }
        }
    }
}

impl std::error::Error for AudioMixValidationError {}

/// Validate an Audio Mix request against the probed media.
///
/// Duplicate selected stream indexes are rejected rather than silently normalized.
pub fn validate_audio_mix_request(
    request: &AudioMixRequest,
    media: &MediaFile,
) -> Result<(), AudioMixValidationError> {
    if media.video_streams.is_empty() {
        return Err(AudioMixValidationError::MissingVideoStream);
    }

    if request.selected_streams.is_empty() {
        return Err(AudioMixValidationError::NoAudioStreamsSelected);
    }

    reject_duplicate_streams(&request.selected_streams)?;
    validate_selected_streams(&request.selected_streams, media)?;

    if same_requested_path(&request.input, &request.output) {
        return Err(AudioMixValidationError::SameInputAndOutput(
            request.output.clone(),
        ));
    }

    validate_output_extension(request)?;
    validate_output_parent(&request.output)?;

    Ok(())
}

fn reject_duplicate_streams(selected_streams: &[u32]) -> Result<(), AudioMixValidationError> {
    let mut seen = HashSet::with_capacity(selected_streams.len());
    for &stream_index in selected_streams {
        if !seen.insert(stream_index) {
            return Err(AudioMixValidationError::DuplicateSelectedStream(
                stream_index,
            ));
        }
    }

    Ok(())
}

fn validate_selected_streams(
    selected_streams: &[u32],
    media: &MediaFile,
) -> Result<(), AudioMixValidationError> {
    let audio_streams = media
        .audio_streams
        .iter()
        .map(|stream| stream.stream_index)
        .collect::<HashSet<_>>();

    for &stream_index in selected_streams {
        if !audio_streams.contains(&stream_index) {
            return Err(AudioMixValidationError::UnknownAudioStream(stream_index));
        }
    }

    Ok(())
}

fn same_requested_path(input: &Path, output: &Path) -> bool {
    #[cfg(windows)]
    {
        input
            .to_string_lossy()
            .eq_ignore_ascii_case(&output.to_string_lossy())
    }

    #[cfg(not(windows))]
    {
        input == output
    }
}

fn validate_output_extension(request: &AudioMixRequest) -> Result<(), AudioMixValidationError> {
    let expected = request.container.extension();
    let actual = request
        .output
        .extension()
        .and_then(|extension| extension.to_str());

    if actual.is_some_and(|extension| extension.eq_ignore_ascii_case(expected)) {
        return Ok(());
    }

    Err(AudioMixValidationError::OutputExtensionMismatch {
        expected,
        actual: actual.map(str::to_owned),
    })
}

fn validate_output_parent(output: &Path) -> Result<(), AudioMixValidationError> {
    let parent = output_parent(output);
    let metadata = fs::metadata(&parent).map_err(|error| {
        AudioMixValidationError::OutputParentUnavailable {
            path: parent.clone(),
            kind: error.kind(),
        }
    })?;

    if !metadata.is_dir() {
        return Err(AudioMixValidationError::OutputParentNotDirectory(parent));
    }

    Ok(())
}

fn output_parent(output: &Path) -> PathBuf {
    match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}
