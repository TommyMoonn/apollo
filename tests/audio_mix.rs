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
        model::{
            AudioMixRequest, LoudnessNormalization, LoudnessTargets, OutputContainer,
            SelectedAudioTrack, TrackGain,
        },
        validation::{AudioMixValidationError, validate_audio_mix_request},
    },
    media::{
        model::{AudioStream, MediaFile, VideoStream},
        output::{MediaOutputError, preflight_output_destination},
    },
};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn media(path: PathBuf) -> MediaFile {
    MediaFile {
        path,
        duration: Some(Duration::from_secs(30)),
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
        selected_tracks: vec![default_track(1), default_track(7)],
        normalization: LoudnessNormalization::Off,
        container: OutputContainer::Mp4,
    }
}

fn track(stream_index: u32, gain_tenths_db: i16) -> SelectedAudioTrack {
    SelectedAudioTrack::new(stream_index, TrackGain::from_tenths_db(gain_tenths_db))
}

fn default_track(stream_index: u32) -> SelectedAudioTrack {
    SelectedAudioTrack::with_default_gain(stream_index)
}

fn loudness_targets(integrated: i16, true_peak: i16, range: i16) -> LoudnessTargets {
    LoudnessTargets::from_tenths(integrated, true_peak, range)
}

#[test]
fn valid_request_accepts_probed_non_contiguous_audio_stream_indexes() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
}

#[test]
fn track_gain_defaults_to_zero_and_formats_integer_tenths() {
    assert_eq!(TrackGain::default(), TrackGain::ZERO);
    assert_eq!(TrackGain::default().tenths_db(), 0);
    assert_eq!(TrackGain::from_tenths_db(-600).to_string(), "-60.0dB");
    assert_eq!(TrackGain::from_tenths_db(-5).to_string(), "-0.5dB");
    assert_eq!(TrackGain::ZERO.to_string(), "0.0dB");
    assert_eq!(TrackGain::from_tenths_db(120).to_string(), "12.0dB");
}

#[test]
fn lower_and_upper_track_gain_bounds_are_accepted() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_tracks = vec![
        SelectedAudioTrack::new(1, TrackGain::MIN),
        SelectedAudioTrack::new(7, TrackGain::MAX),
    ];
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
}

#[test]
fn track_gain_outside_each_bound_is_rejected() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let media = media(request.input.clone());

    request.selected_tracks = vec![track(1, -601)];
    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::TrackGainOutOfRange {
            stream_index: 1,
            gain: TrackGain::from_tenths_db(-601),
        })
    );

    request.selected_tracks = vec![track(1, 121)];
    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::TrackGainOutOfRange {
            stream_index: 1,
            gain: TrackGain::from_tenths_db(121),
        })
    );
}

#[test]
fn normalization_and_target_defaults_match_the_domain_policy() {
    let targets = LoudnessTargets::default();

    assert_eq!(LoudnessNormalization::default(), LoudnessNormalization::Off);
    assert_eq!(targets.integrated_loudness_tenths(), -160);
    assert_eq!(targets.maximum_true_peak_tenths(), -15);
    assert_eq!(targets.loudness_range_tenths(), 110);
}

#[test]
fn loudness_target_lower_and_upper_bounds_are_accepted() {
    let temp = TestDirectory::new();
    let media_path = temp.path().join("input.mkv");
    let media = media(media_path);
    let mut request = valid_request(&temp);

    for targets in [
        loudness_targets(-700, -90, 10),
        loudness_targets(-50, 0, 500),
    ] {
        request.normalization = LoudnessNormalization::Enabled(targets);
        assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
    }
}

#[test]
fn each_loudness_target_rejects_values_outside_its_range() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let media = media(request.input.clone());

    for (targets, expected) in [
        (
            loudness_targets(-701, -15, 110),
            AudioMixValidationError::IntegratedLoudnessOutOfRange,
        ),
        (
            loudness_targets(-49, -15, 110),
            AudioMixValidationError::IntegratedLoudnessOutOfRange,
        ),
        (
            loudness_targets(-160, -91, 110),
            AudioMixValidationError::MaximumTruePeakOutOfRange,
        ),
        (
            loudness_targets(-160, 1, 110),
            AudioMixValidationError::MaximumTruePeakOutOfRange,
        ),
        (
            loudness_targets(-160, -15, 9),
            AudioMixValidationError::LoudnessRangeOutOfRange,
        ),
        (
            loudness_targets(-160, -15, 501),
            AudioMixValidationError::LoudnessRangeOutOfRange,
        ),
    ] {
        request.normalization = LoudnessNormalization::Enabled(targets);
        assert_eq!(validate_audio_mix_request(&request, &media), Err(expected));
    }
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
    request.selected_tracks.clear();
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
    request.selected_tracks = vec![default_track(3), default_track(3)];
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
    request.selected_tracks = vec![default_track(99)];
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
    request.selected_tracks = vec![default_track(0)];
    let media = media(request.input.clone());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::UnknownAudioStream(0))
    );
}

#[test]
fn wider_channel_layout_is_rejected_with_stream_specific_feedback() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_tracks = vec![default_track(1)];
    let mut media = media(request.input.clone());
    media.audio_streams[0].channels = Some(6);
    media.audio_streams[0].channel_layout = Some("5.1".to_owned());

    assert_eq!(
        validate_audio_mix_request(&request, &media),
        Err(AudioMixValidationError::UnsupportedChannelLayout {
            stream_index: 1,
            channels: Some(6),
            channel_layout: Some("5.1".to_owned()),
        })
    );
}

#[test]
fn same_input_and_output_is_left_to_destination_preflight() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    fs::write(&request.input, b"input").expect("fixture input should be created");
    request.output = request.input.clone();
    request.container = OutputContainer::Mkv;
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
    assert_eq!(
        preflight_output_destination(&request.input, &request.output),
        Err(MediaOutputError::SameInputAndOutput(
            temp.path().join("input.mkv")
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
fn pure_request_validation_does_not_require_output_parent_to_exist() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let missing_parent = temp.path().join("missing");
    request.output = missing_parent.join("mixed.mp4");
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
    assert_eq!(
        preflight_output_destination(&request.input, &request.output),
        Err(MediaOutputError::OutputParentUnavailable {
            path: missing_parent,
            kind: std::io::ErrorKind::NotFound,
        })
    );
}

#[test]
fn output_parent_directory_check_is_left_to_destination_preflight() {
    let temp = TestDirectory::new();
    let parent_file = temp.path().join("not-a-directory");
    fs::write(&parent_file, b"occupied").expect("fixture file should be created");
    let mut request = valid_request(&temp);
    request.output = parent_file.join("mixed.mp4");
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
    assert_eq!(
        preflight_output_destination(&request.input, &request.output),
        Err(MediaOutputError::OutputParentNotDirectory(parent_file))
    );
}

#[test]
fn existing_output_check_is_left_to_destination_preflight() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    fs::write(&request.output, b"occupied").expect("fixture output should be created");
    let media = media(request.input.clone());

    assert_eq!(validate_audio_mix_request(&request, &media), Ok(()));
    assert!(matches!(
        preflight_output_destination(&request.input, &request.output),
        Err(MediaOutputError::OutputAlreadyExists(_))
    ));
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

// Validate the filter graph's connections as well as its media policy. The temporary
// label names are incidental; every produced label must be consumed by the next stage.
fn assert_mix_graph(args: &[String], tracks: &[(u32, &str)], layout: &str, final_filter: &str) {
    let filters = option_values(args, "-filter_complex");
    assert_eq!(filters.len(), 1);
    let stages: Vec<_> = filters[0].split(';').collect();
    assert_eq!(stages.len(), tracks.len() + 2);
    let mut labels = Vec::new();
    for (stage, (index, gain)) in stages.iter().zip(tracks) {
        let (body, label) = stage.rsplit_once('[').expect("volume output label");
        assert_eq!(
            body,
            format!("[0:{index}]volume={gain},aformat=channel_layouts={layout}")
        );
        let label = label.strip_suffix(']').expect("closing volume label");
        assert!(!label.is_empty() && !labels.contains(&label));
        labels.push(label);
    }
    let (body, label) = stages[tracks.len()]
        .rsplit_once('[')
        .expect("mix output label");
    let inputs: String = labels.iter().map(|label| format!("[{label}]")).collect();
    assert_eq!(
        body,
        format!(
            "{inputs}amix=inputs={}:duration=longest:normalize=0",
            tracks.len()
        )
    );
    let mix_label = label.strip_suffix(']').expect("closing mix label");
    assert!(!mix_label.is_empty() && !labels.contains(&mix_label));
    let (body, label) = stages[tracks.len() + 1]
        .rsplit_once('[')
        .expect("final output label");
    assert_eq!(body, format!("[{mix_label}]{final_filter}"));
    let output_label = label.strip_suffix(']').expect("closing output label");
    assert!(
        !output_label.is_empty() && output_label != mix_label && !labels.contains(&output_label)
    );
    let mapped = format!("[{output_label}]");
    assert_eq!(
        option_values(args, "-map")
            .iter()
            .filter(|&&value| value == mapped)
            .count(),
        1
    );
}

#[test]
fn mixing_preserves_stream_identity_gain_and_channel_policy() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let mut media = media(request.input.clone());
    for (index, channels, layout) in [(9, 1, "mono"), (11, 2, "stereo")] {
        media.audio_streams.push(AudioStream {
            stream_index: index,
            codec: Some("aac".into()),
            channels: Some(channels),
            channel_layout: Some(layout.into()),
            title: None,
            language: None,
        });
    }
    // Exercise one to four sparse streams, both track orders, two mono tracks,
    // and unchanged gain when the number of selected streams changes.
    type SelectionCase = (&'static [(u32, i16, &'static str)], &'static str);
    let cases: &[SelectionCase] = &[
        (&[(3, 0, "0.0dB")], "mono"),
        (&[(1, -60, "-6.0dB")], "stereo"),
        (&[(1, 0, "0.0dB"), (7, 0, "0.0dB")], "stereo"),
        (&[(3, 0, "0.0dB"), (1, 0, "0.0dB")], "stereo"),
        (&[(1, 0, "0.0dB"), (3, 0, "0.0dB")], "stereo"),
        (&[(3, 0, "0.0dB"), (9, 0, "0.0dB")], "mono"),
        (
            &[(7, 120, "12.0dB"), (1, -60, "-6.0dB"), (3, 0, "0.0dB")],
            "stereo",
        ),
        (
            &[
                (1, 0, "0.0dB"),
                (3, 0, "0.0dB"),
                (7, 0, "0.0dB"),
                (11, 0, "0.0dB"),
            ],
            "stereo",
        ),
    ];
    for (selection, layout) in cases {
        request.selected_tracks = selection
            .iter()
            .map(|&(index, gain, _)| track(index, gain))
            .collect();
        let expected: Vec<_> = selection
            .iter()
            .map(|&(index, _, gain)| (index, gain))
            .collect();
        assert_mix_graph(
            &invocation_args(&request, &media),
            &expected,
            layout,
            "alimiter=limit=0.95:level=0:latency=1",
        );
    }
}

#[test]
fn normalization_changes_only_the_final_filter_and_uses_selected_targets() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    request.selected_tracks = vec![track(1, -60), track(3, 25)];
    let media = media(request.input.clone());
    for (normalization, expected) in [
        (
            LoudnessNormalization::Off,
            "alimiter=limit=0.95:level=0:latency=1",
        ),
        (
            LoudnessNormalization::Enabled(LoudnessTargets::default()),
            "loudnorm=I=-16.0:TP=-1.5:LRA=11.0:linear=false",
        ),
        (
            LoudnessNormalization::Enabled(loudness_targets(-235, -7, 185)),
            "loudnorm=I=-23.5:TP=-0.7:LRA=18.5:linear=false",
        ),
    ] {
        request.normalization = normalization;
        assert_mix_graph(
            &invocation_args(&request, &media),
            &[(1, "-6.0dB"), (3, "2.5dB")],
            "stereo",
            expected,
        );
    }
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
        format: Default::default(),
    });
    let args = invocation_args(&request, &media);

    let filter = option_values(&args, "-filter_complex")[0];
    let mixed = &filter[filter.rfind('[').expect("mixed output label")..];
    assert_eq!(option_values(&args, "-map"), vec!["0:5", mixed]);
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

    let filter = option_values(&args, "-filter_complex")[0];
    let mixed = &filter[filter.rfind('[').expect("mixed output label")..];
    assert_eq!(
        option_values(&args, "-map"),
        vec!["0", "-0:a", "-0:t?", mixed, "0:t?"]
    );
    assert_eq!(option_values(&args, "-c"), vec!["copy"]);
    assert_eq!(option_values(&args, "-c:v"), vec!["copy"]);
    assert_eq!(option_values(&args, "-map_metadata"), vec!["0"]);
    assert_eq!(option_values(&args, "-map_chapters"), vec!["0"]);
}

#[test]
fn mixed_audio_encoding_and_metadata_policy_is_explicit_for_both_containers() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let media = media(request.input.clone());

    let mp4_args = invocation_args(&request, &media);
    assert_eq!(option_values(&mp4_args, "-c:a"), vec!["aac"]);
    assert_eq!(option_values(&mp4_args, "-b:a"), vec!["320k"]);
    assert_eq!(option_values(&mp4_args, "-ar"), vec!["48000"]);
    assert_eq!(
        option_values(&mp4_args, "-metadata:s:a:0"),
        vec!["title=Mixed Audio"]
    );

    request.container = OutputContainer::Mkv;
    request.output = temp.path().join("mixed.mkv");
    let mkv_args = invocation_args(&request, &media);
    assert_eq!(option_values(&mkv_args, "-c:a"), vec!["aac"]);
    assert_eq!(option_values(&mkv_args, "-b:a"), vec!["320k"]);
    assert_eq!(option_values(&mkv_args, "-ar"), vec!["48000"]);
    assert_eq!(
        option_values(&mkv_args, "-metadata:s:a:0"),
        vec!["title=Mixed Audio"]
    );
}

#[test]
fn command_enables_machine_readable_progress_on_stdout() {
    let temp = TestDirectory::new();
    let request = valid_request(&temp);
    let media = media(request.input.clone());
    let args = invocation_args(&request, &media);
    let output = request.output.to_string_lossy().into_owned();

    assert_eq!(option_values(&args, "-progress"), vec!["pipe:1"]);
    assert!(args.iter().any(|arg| arg == "-nostats"));
    assert_eq!(args.last(), Some(&output));
}

#[test]
fn input_and_output_paths_with_spaces_remain_individual_arguments() {
    let temp = TestDirectory::new();
    let request = AudioMixRequest {
        input: temp.path().join("source video with spaces.mkv"),
        output: temp.path().join("mixed output with spaces.mp4"),
        selected_tracks: vec![default_track(1), default_track(3)],
        normalization: LoudnessNormalization::Off,
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
fn command_builder_rejects_missing_structural_inputs_without_revalidating_request() {
    let temp = TestDirectory::new();
    let mut request = valid_request(&temp);
    let mut media = media(request.input.clone());
    media.video_streams.clear();

    assert_eq!(
        build_audio_mix_invocation(&request, &media),
        Err(AudioMixCommandError::MissingVideoStream)
    );

    request.selected_tracks.clear();
    media.video_streams.push(VideoStream {
        stream_index: 0,
        codec: Some("h264".to_owned()),
        width: Some(1920),
        height: Some(1080),
        frame_rate: None,
        format: Default::default(),
    });
    assert_eq!(
        build_audio_mix_invocation(&request, &media),
        Err(AudioMixCommandError::NoAudioStreamsSelected)
    );
}
