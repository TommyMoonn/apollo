use std::fmt;

use crate::media::{
    error::MediaError,
    model::MediaFile,
    operation::PreparedMediaOperation,
    output::{MediaOutput, MediaOutputError, OutputCollisionPolicy},
    tools::MediaTools,
};

use super::{
    command::build_audio_mix_invocation_to,
    model::AudioMixRequest,
    validation::{AudioMixValidationError, validate_audio_mix_layout},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioMixPreparationError {
    Validation(AudioMixValidationError),
    Output(MediaOutputError),
}

impl fmt::Display for AudioMixPreparationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => write!(f, "audio mix request is invalid: {error}"),
            Self::Output(error) => write!(f, "audio mix output could not be prepared: {error}"),
        }
    }
}

impl std::error::Error for AudioMixPreparationError {}

impl AudioMixPreparationError {
    pub fn user_error(&self) -> MediaError {
        match self {
            Self::Validation(error) => match error {
                AudioMixValidationError::MissingVideoStream
                | AudioMixValidationError::NoAudioStreamsSelected
                | AudioMixValidationError::UnknownAudioStream(_) => {
                    MediaError::missing_stream(error.to_string())
                }
                AudioMixValidationError::Output(error) => MediaError::from_output_error(error),
                AudioMixValidationError::DuplicateSelectedStream(_)
                | AudioMixValidationError::TrackGainOutOfRange { .. }
                | AudioMixValidationError::UnsupportedChannelLayout { .. }
                | AudioMixValidationError::IntegratedLoudnessOutOfRange
                | AudioMixValidationError::MaximumTruePeakOutOfRange
                | AudioMixValidationError::LoudnessRangeOutOfRange
                | AudioMixValidationError::OutputExtensionMismatch { .. } => {
                    MediaError::invalid_request(error.to_string())
                }
            },
            Self::Output(error) => MediaError::from_output_error(error),
        }
    }
}

pub(crate) fn prepare_audio_mix_operation(
    tools: &MediaTools,
    media: &MediaFile,
    request: &AudioMixRequest,
    collision_policy: OutputCollisionPolicy,
) -> Result<PreparedMediaOperation, AudioMixPreparationError> {
    let layout =
        validate_audio_mix_layout(request, media).map_err(AudioMixPreparationError::Validation)?;

    let output = MediaOutput::prepare_with_policy(&request.output, collision_policy)
        .map_err(AudioMixPreparationError::Output)?;
    let invocation = build_audio_mix_invocation_to(
        request,
        media.video_streams[0].stream_index,
        layout,
        output.temporary_path(),
    );
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
    use crate::audio_mix::model::{LoudnessNormalization, OutputContainer, SelectedAudioTrack};
    use crate::media::{error::MediaErrorKind, model::*};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn prelaunch_validation_maps_to_shared_user_error_categories() {
        let missing_stream =
            AudioMixPreparationError::Validation(AudioMixValidationError::NoAudioStreamsSelected)
                .user_error();
        assert_eq!(missing_stream.kind(), MediaErrorKind::MissingStream);

        let output_not_writable = AudioMixPreparationError::Validation(
            AudioMixValidationError::Output(MediaOutputError::OutputParentUnavailable {
                path: PathBuf::from("missing-parent"),
                kind: std::io::ErrorKind::NotFound,
            }),
        )
        .user_error();
        assert_eq!(
            output_not_writable.kind(),
            MediaErrorKind::OutputNotWritable
        );
    }

    #[test]
    fn preparation_stages_output_and_preserves_command() {
        let directory = TestDirectory::new("prepare");
        let (request, media) = valid_request_and_media(&directory);
        let tools = fake_tools(&directory);
        let (output, command, duration) =
            prepare_audio_mix_operation(&tools, &media, &request, OutputCollisionPolicy::Numbered)
                .unwrap()
                .into_parts();
        assert_eq!(command.program(), tools.ffmpeg);
        assert_eq!(duration, media.duration);
        let expected =
            crate::audio_mix::command::build_audio_mix_invocation(&request, &media).unwrap();
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
        let (mut request, media) = valid_request_and_media(&directory);
        request.selected_tracks.clear();
        assert!(matches!(
            prepare_audio_mix_operation(
                &fake_tools(&directory),
                &media,
                &request,
                OutputCollisionPolicy::Numbered
            ),
            Err(AudioMixPreparationError::Validation(_))
        ));
        assert!(!request.output.exists());
    }

    fn valid_request_and_media(directory: &TestDirectory) -> (AudioMixRequest, MediaFile) {
        let input = directory.path().join("source.mkv");
        fs::write(&input, b"source").unwrap();
        let request = AudioMixRequest {
            input: input.clone(),
            output: directory.path().join("mixed.mp4"),
            selected_tracks: vec![
                SelectedAudioTrack::with_default_gain(1),
                SelectedAudioTrack::with_default_gain(3),
            ],
            normalization: LoudnessNormalization::Off,
            container: OutputContainer::Mp4,
        };
        let media = MediaFile {
            path: input,
            duration: None,
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 0,
                codec: Some("h264".to_owned()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
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
}
