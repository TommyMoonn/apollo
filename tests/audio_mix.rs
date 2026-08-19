use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use apollo::{
    audio_mix::{
        model::{AudioMixRequest, OutputContainer},
        validation::{AudioMixValidationError, validate_audio_mix_request},
    },
    media::model::{AudioStream, MediaFile, VideoStream},
};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn media(path: PathBuf) -> MediaFile {
    MediaFile {
        path,
        duration: Some(Duration::from_secs(30)),
        video_streams: vec![VideoStream {
            stream_index: 0,
            codec: Some("h264".to_owned()),
            width: Some(1920),
            height: Some(1080),
            frame_rate: None,
        }],
        audio_streams: vec![
            AudioStream {
                stream_index: 1,
                codec: Some("aac".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: Some("Desktop".to_owned()),
                language: Some("eng".to_owned()),
            },
            AudioStream {
                stream_index: 3,
                codec: Some("aac".to_owned()),
                channels: Some(1),
                channel_layout: Some("mono".to_owned()),
                title: Some("Microphone".to_owned()),
                language: None,
            },
            AudioStream {
                stream_index: 7,
                codec: Some("opus".to_owned()),
                channels: Some(2),
                channel_layout: Some("stereo".to_owned()),
                title: None,
                language: None,
            },
        ],
    }
}

fn valid_request(temp: &TestDirectory) -> AudioMixRequest {
    AudioMixRequest {
        input: temp.path().join("input.mkv"),
        output: temp.path().join("mixed.mp4"),
        selected_streams: vec![1, 7],
        container: OutputContainer::Mp4,
    }
}

#[test]
fn valid_request_accepts_probed_non_contiguous_audio_stream_indexes() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
}

#[test]
fn media_without_video_stream_is_rejected() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    let mut media = media(request.input.clone());
    media.video_streams.clear();

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::MissingVideoStream)
    );
}

#[test]
fn empty_audio_selection_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_streams.clear();
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::NoAudioStreamsSelected)
    );
}

#[test]
fn duplicate_selected_stream_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_streams = vec![3, 3];
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::DuplicateSelectedStream(3))
    );
}

#[test]
fn unknown_audio_stream_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_streams = vec![99];
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::UnknownAudioStream(99))
    );
}

#[test]
fn numeric_video_stream_index_is_not_accepted_as_audio_selection() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_streams = vec![0];
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::UnknownAudioStream(0))
    );
}

#[test]
fn same_input_and_output_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.output = request.input.clone();
    request.container = OutputContainer::Mkv;
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::SameInputAndOutput(
            request.output.clone()
        ))
    );
}

#[test]
fn output_extension_must_match_selected_container() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.output = temp.path().join("mixed.mkv");
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::OutputExtensionMismatch {
            expected: "mp4",
            actual: Some("mkv".to_owned()),
        })
    );
}

#[test]
fn matching_output_extension_is_case_insensitive() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.output = temp.path().join("mixed.MP4");
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
}

#[test]
fn nonexistent_output_parent_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let missing_parent = temp.path().join("missing");
    request.output = missing_parent.join("mixed.mp4");
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::OutputParentUnavailable {
            path: missing_parent,
            kind: std::io::ErrorKind::NotFound,
        })
    );
}

#[test]
fn output_parent_must_be_a_directory() {
    let temp = TestDirectory::new();
    let parent_file = temp.path().join("not-a-directory");
    fs::write(&parent_file, b"occupied").expect("fixture file should be created");
    let mut request = valid_request(&temp);
    request.output = parent_file.join("mixed.mp4");
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::OutputParentNotDirectory(
            parent_file
        ))
    );
}

#[test]
fn output_container_exposes_its_required_extension() {
    assert_eq!(OutputContainer::Mp4.extension(), "mp4");
    assert_eq!(OutputContainer::Mkv.extension(), "mkv");
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "apollo-audio-mix-validation-{}-{sequence}",
            process::id()
        ));

        if path.exists() {
            fs::remove_dir_all(&path).expect("stale fixture directory should be removable");
        }
        fs::create_dir(&path).expect("fixture directory should be created");

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
