use std::fmt;

use crate::media::{
    container::MediaContainer,
    error::MediaError,
    model::MediaFile,
    operation::PreparedMediaOperation,
    output::{MediaOutput, MediaOutputError},
    tools::MediaTools,
};

use super::{
    command::build_remux_invocation_to,
    model::RemuxRequest,
    validation::{RemuxValidationError, validate_remux_request},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemuxPreparationError {
    Validation(RemuxValidationError),
    Output(MediaOutputError),
}

impl fmt::Display for RemuxPreparationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => write!(f, "remux request is invalid: {error}"),
            Self::Output(error) => write!(f, "remux output could not be prepared: {error}"),
        }
    }
}

impl std::error::Error for RemuxPreparationError {}

impl RemuxPreparationError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                RemuxValidationError::NoStreams => MediaError::missing_stream(error.to_string()),
                RemuxValidationError::IncompatibleStream {
                    container: MediaContainer::Mp4,
                    ..
                } => MediaError::mp4_incompatibility(error.to_string()),
                RemuxValidationError::IncompatibleStream { .. }
                | RemuxValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
                RemuxValidationError::Output(error) => MediaError::from_output_error(error),
            },
            Self::Output(error) => MediaError::from_output_error(error),
        }
    }
}

pub(crate) fn prepare_remux_operation(
    tools: &MediaTools,
    media: &MediaFile,
    request: &RemuxRequest,
) -> Result<PreparedMediaOperation, RemuxPreparationError> {
    validate_remux_request(request, media).map_err(RemuxPreparationError::Validation)?;

    let output = MediaOutput::prepare(&request.output).map_err(RemuxPreparationError::Output)?;
    let invocation = build_remux_invocation_to(request, output.temporary_path());
    Ok(PreparedMediaOperation::new(
        output,
        tools,
        invocation.args,
        media.duration,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{error::MediaErrorKind, model::*};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn validation_failures_map_to_shared_user_error_categories() {
        let no_streams = RemuxPreparationError::Validation(
            crate::remux::validation::RemuxValidationError::NoStreams,
        )
        .user_error();
        assert_eq!(no_streams.kind(), MediaErrorKind::MissingStream);

        let incompatible = RemuxPreparationError::Validation(
            crate::remux::validation::RemuxValidationError::IncompatibleStream {
                container: MediaContainer::Mp4,
                stream_index: 4,
                kind: StreamKind::Audio,
                codec: Some("wmav2".to_owned()),
            },
        )
        .user_error();
        assert_eq!(incompatible.kind(), MediaErrorKind::Mp4Incompatibility);
    }

    #[test]
    fn preparation_stages_output_and_preserves_command() {
        let directory = TestDirectory::new("prepare");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let (output, command, duration) = prepare_remux_operation(&tools, &media, &request)
            .unwrap()
            .into_parts();
        assert_eq!(command.program(), tools.ffmpeg);
        assert_eq!(duration, media.duration);
        let expected = crate::remux::command::build_remux_invocation(&request);
        let args = command.arguments();
        assert_eq!(
            &args[..args.len() - 1],
            &expected.args[..expected.args.len() - 1]
        );
        assert_eq!(args.last().unwrap(), output.temporary_path().as_os_str());
        assert_ne!(output.temporary_path(), request.output);
        assert!(!request.output.exists());
    }

    #[test]
    fn invalid_request_cannot_be_prepared() {
        let directory = TestDirectory::new("invalid");
        let (request, mut media) = valid_request_and_media(&directory);
        media.streams[1].codec = Some("wmav2".to_owned());
        assert!(matches!(
            prepare_remux_operation(&fake_tools(&directory), &media, &request),
            Err(RemuxPreparationError::Validation(_))
        ));
        assert!(!request.output.exists());
    }

    fn valid_request_and_media(directory: &TestDirectory) -> (RemuxRequest, MediaFile) {
        let input = directory.path().join("source.mkv");
        fs::write(&input, b"source").unwrap();
        let request = RemuxRequest {
            input: input.clone(),
            output: directory.path().join("remuxed.mp4"),
            container: MediaContainer::Mp4,
        };
        let media = MediaFile {
            path: input,
            duration: None,
            streams: vec![
                MediaStream {
                    stream_index: 0,
                    kind: StreamKind::Video,
                    codec: Some("h264".to_owned()),
                },
                MediaStream {
                    stream_index: 2,
                    kind: StreamKind::Audio,
                    codec: Some("aac".to_owned()),
                },
            ],
            video_streams: Vec::new(),
            audio_streams: Vec::new(),
        };
        (request, media)
    }

    fn fake_tools(directory: &TestDirectory) -> MediaTools {
        MediaTools::new(
            directory.path().join("fake-ffmpeg"),
            directory.path().join("fake-ffprobe"),
        )
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "apollo-remux-execution-test-{}-{sequence}-{name}",
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
}
