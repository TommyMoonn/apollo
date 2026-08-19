use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use apollo::{
    audio_mix::{
        command::{AudioMixCommandError, build_audio_mix_invocation},
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

fn invocation_args(request: &AudioMixRequest, media: &MediaFile) -> Vec<String> {
    build_audio_mix_invocation(request, media)
        .expect("valid request should build an FFmpeg invocation")
        .args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn option_values<'a>(args: &'a [String], option: &str) -> Vec<&'a str> {
    args.windows(2)
        .filter_map(|pair| (pair[0] == option).then_some(pair[1].as_str()))
        .collect()
}

#[test]
fn one_selected_stream_builds_single_input_amix_filter() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_streams = vec![3];
    let media = media(request.input.clone());
    let args = invocation_args(&request, &media);

    assert_eq!(
        option_values(&args, "-filter_complex"),
        vec!["[0:3]amix=inputs=1:duration=longest[mixed]"]
    );
}

#[test]
fn two_selected_streams_build_two_input_amix_filter() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    let media = media(request.input.clone());
    let args = invocation_args(&request, &media);

    assert_eq!(
        option_values(&args, "-filter_complex"),
        vec!["[0:1][0:7]amix=inputs=2:duration=longest[mixed]"]
    );
}

#[test]
fn four_selected_streams_build_four_input_amix_filter() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_streams = vec![1, 3, 7, 11];
    let mut media = media(request.input.clone());
    media.audio_streams.push(AudioStream {
        stream_index: 11,
        codec: Some("aac".to_owned()),
        channels: Some(2),
        channel_layout: Some("stereo".to_owned()),
        title: Some("Game".to_owned()),
        language: None,
    });
    let args = invocation_args(&request, &media);

    assert_eq!(
        option_values(&args, "-filter_complex"),
        vec!["[0:1][0:3][0:7][0:11]amix=inputs=4:duration=longest[mixed]"]
    );
}

#[test]
fn filter_uses_non_contiguous_absolute_stream_indexes_in_selection_order() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_streams = vec![7, 1, 3];
    let media = media(request.input.clone());
    let args = invocation_args(&request, &media);

    assert_eq!(
        option_values(&args, "-filter_complex"),
        vec!["[0:7][0:1][0:3]amix=inputs=3:duration=longest[mixed]"]
    );
}

#[test]
fn mp4_maps_only_primary_video_and_mixed_audio() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    let mut media = media(request.input.clone());
    media.video_streams[0].stream_index = 5;
    media.video_streams.push(VideoStream {
        stream_index: 9,
        codec: Some("mjpeg".to_owned()),
        width: Some(640),
        height: Some(480),
        frame_rate: None,
    });
    let args = invocation_args(&request, &media);

    assert_eq!(option_values(&args, "-map"), vec!["0:5", "[mixed]"]);
    assert_eq!(option_values(&args, "-c:v"), vec!["copy"]);
    assert_eq!(option_values(&args, "-map_metadata"), vec!["0"]);
    assert_eq!(option_values(&args, "-map_chapters"), vec!["0"]);
}

#[test]
fn mkv_defers_optional_attachments_until_after_mixed_audio() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.container = OutputContainer::Mkv;
    request.output = temp.path().join("mixed.mkv");
    let media = media(request.input.clone());
    let args = invocation_args(&request, &media);

    assert_eq!(
        option_values(&args, "-map"),
        vec!["0", "-0:a", "-0:t?", "[mixed]", "0:t?"]
    );
    assert_eq!(option_values(&args, "-c"), vec!["copy"]);
    assert_eq!(option_values(&args, "-c:v"), vec!["copy"]);
    assert_eq!(option_values(&args, "-map_metadata"), vec!["0"]);
    assert_eq!(option_values(&args, "-map_chapters"), vec!["0"]);
}

#[test]
fn mkv_attachment_mapping_remains_safe_when_source_has_no_attachments() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.container = OutputContainer::Mkv;
    request.output = temp.path().join("mixed.mkv");
    let media = media(request.input.clone());
    let args = invocation_args(&request, &media);
    let maps = option_values(&args, "-map");

    assert!(maps.contains(&"-0:t?"));
    assert!(maps.contains(&"0:t?"));
    assert!(!maps.contains(&"-0:t"));
    assert!(!maps.contains(&"0:t"));
}

#[test]
fn mixed_audio_is_encoded_to_aac_for_both_containers() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let media = media(request.input.clone());

    let mp4_args = invocation_args(&request, &media);
    assert_eq!(option_values(&mp4_args, "-c:a"), vec!["aac"]);

    request.container = OutputContainer::Mkv;
    request.output = temp.path().join("mixed.mkv");
    let mkv_args = invocation_args(&request, &media);
    assert_eq!(option_values(&mkv_args, "-c:a"), vec!["aac"]);
}

#[test]
fn original_audio_streams_are_never_mapped_as_output_audio() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    let media = media(request.input.clone());
    let mp4_args = invocation_args(&request, &media);

    assert_eq!(option_values(&mp4_args, "-map"), vec!["0:0", "[mixed]"]);

    let mut mkv_request = request.clone();
    mkv_request.container = OutputContainer::Mkv;
    mkv_request.output = temp.path().join("mixed.mkv");
    let mkv_args = invocation_args(&mkv_request, &media);
    assert!(option_values(&mkv_args, "-map").contains(&"-0:a"));
}

#[test]
fn input_and_output_paths_with_spaces_remain_individual_arguments() {
    let temp = TestDirectory::new();
    let request = AudioMixRequest {
        input: temp.path().join("source video with spaces.mkv"),
        output: temp.path().join("mixed output with spaces.mp4"),
        selected_streams: vec![1, 3],
        container: OutputContainer::Mp4,
    };
    let media = media(request.input.clone());
    let invocation = build_audio_mix_invocation(&request, &media)
        .expect("valid request should build an FFmpeg invocation");

    assert_eq!(invocation.args[0], "-i");
    assert_eq!(invocation.args[1], request.input.as_os_str());
    assert_eq!(
        invocation.args.last().map(OsString::as_os_str),
        Some(request.output.as_os_str())
    );
    assert_eq!(
        invocation
            .args
            .iter()
            .filter(|arg| arg.as_os_str() == request.input.as_os_str())
            .count(),
        1
    );
    assert_eq!(
        invocation
            .args
            .iter()
            .filter(|arg| arg.as_os_str() == request.output.as_os_str())
            .count(),
        1
    );
}

#[test]
fn video_is_stream_copied_for_both_containers() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let media = media(request.input.clone());

    let mp4_args = invocation_args(&request, &media);
    assert_eq!(option_values(&mp4_args, "-c:v"), vec!["copy"]);

    request.container = OutputContainer::Mkv;
    request.output = temp.path().join("mixed.mkv");
    let mkv_args = invocation_args(&request, &media);
    assert_eq!(option_values(&mkv_args, "-c:v"), vec!["copy"]);
}

#[test]
fn command_builder_rejects_missing_structural_inputs_without_revalidating_request() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let mut media = media(request.input.clone());
    media.video_streams.clear();

    assert_eq!(
        build_audio_mix_invocation(&request, &media),
        Err(AudioMixCommandError::MissingVideoStream)
    );

    request.selected_streams.clear();
    media.video_streams.push(VideoStream {
        stream_index: 0,
        codec: Some("h264".to_owned()),
        width: Some(1920),
        height: Some(1080),
        frame_rate: None,
    });
    assert_eq!(
        build_audio_mix_invocation(&request, &media),
        Err(AudioMixCommandError::NoAudioStreamsSelected)
    );
}
