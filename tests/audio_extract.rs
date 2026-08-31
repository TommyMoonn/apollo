use std::path::PathBuf;

use apollo::{
    audio_extract::{
        command::build_audio_extract_invocation,
        model::{AudioExtractFormat, AudioExtractMetadata, AudioExtractRequest},
        validation::{AudioExtractValidationError, validate_audio_extract_request},
    },
    media::model::{AudioStream, MediaFile, MediaStream, StreamKind},
};

fn request(format: AudioExtractFormat, stream_index: u32) -> AudioExtractRequest {
    AudioExtractRequest {
        input: PathBuf::from("C:/media/source recording.mkv"),
        output: PathBuf::from(format!(
            "C:/media/source recording-audio.{}",
            format.extension()
        )),
        audio_stream_index: stream_index,
        format,
    }
}

fn media() -> MediaFile {
    MediaFile {
        path: PathBuf::from("C:/media/source recording.mkv"),
        duration: None,
        streams: vec![
            MediaStream {
                stream_index: 0,
                kind: StreamKind::Video,
                codec: Some("h264".to_owned()),
            },
            MediaStream {
                stream_index: 3,
                kind: StreamKind::Subtitle,
                codec: Some("subrip".to_owned()),
            },
            MediaStream {
                stream_index: 7,
                kind: StreamKind::Audio,
                codec: Some("opus".to_owned()),
            },
            MediaStream {
                stream_index: 11,
                kind: StreamKind::Data,
                codec: Some("bin_data".to_owned()),
            },
        ],
        video_streams: Vec::new(),
        audio_streams: vec![AudioStream {
            stream_index: 7,
            codec: Some("opus".to_owned()),
            channels: Some(2),
            channel_layout: Some("stereo".to_owned()),
            title: Some("Commentary".to_owned()),
            language: Some("eng".to_owned()),
        }],
    }
}

#[test]
fn formats_own_extensions_and_muxers() {
    assert_eq!(AudioExtractFormat::Original.extension(), "mka");
    assert_eq!(AudioExtractFormat::Original.ffmpeg_muxer_name(), "matroska");
    assert_eq!(AudioExtractFormat::Aac.extension(), "m4a");
    assert_eq!(AudioExtractFormat::Aac.ffmpeg_muxer_name(), "mp4");
    assert_eq!(AudioExtractFormat::Flac.extension(), "flac");
    assert_eq!(AudioExtractFormat::Flac.ffmpeg_muxer_name(), "flac");
}

#[test]
fn sparse_absolute_audio_index_is_valid() {
    assert_eq!(
        validate_audio_extract_request(&request(AudioExtractFormat::Original, 7), &media()),
        Ok(())
    );
}

#[test]
fn unknown_absolute_index_is_rejected() {
    assert_eq!(
        validate_audio_extract_request(&request(AudioExtractFormat::Original, 9), &media()),
        Err(AudioExtractValidationError::UnknownStreamIndex { stream_index: 9 })
    );
}

#[test]
fn every_non_audio_stream_kind_is_rejected() {
    for (stream_index, kind) in [
        (0, StreamKind::Video),
        (3, StreamKind::Subtitle),
        (11, StreamKind::Data),
    ] {
        assert_eq!(
            validate_audio_extract_request(
                &request(AudioExtractFormat::Original, stream_index),
                &media(),
            ),
            Err(AudioExtractValidationError::StreamIsNotAudio { stream_index, kind })
        );
    }
}

#[test]
fn output_extension_must_match_selected_format() {
    for (format, expected) in [
        (AudioExtractFormat::Original, "mka"),
        (AudioExtractFormat::Aac, "m4a"),
        (AudioExtractFormat::Flac, "flac"),
    ] {
        let mut matching = request(format, 7);
        matching.output.set_extension(expected.to_uppercase());
        assert_eq!(validate_audio_extract_request(&matching, &media()), Ok(()));

        matching.output.set_extension("wav");
        assert_eq!(
            validate_audio_extract_request(&matching, &media()),
            Err(AudioExtractValidationError::OutputExtensionMismatch {
                expected,
                actual: Some("wav".to_owned()),
            })
        );
    }
}

#[test]
fn original_format_copies_only_the_selected_audio_stream() {
    assert_invocation(
        AudioExtractFormat::Original,
        &["-c:a", "copy", "-f", "matroska"],
    );
}

#[test]
fn aac_format_encodes_at_256k_and_maps_only_selected_audio() {
    assert_invocation(
        AudioExtractFormat::Aac,
        &["-c:a", "aac", "-b:a", "256k", "-f", "mp4"],
    );
}

#[test]
fn flac_format_encodes_flac_and_maps_only_selected_audio() {
    assert_invocation(AudioExtractFormat::Flac, &["-c:a", "flac", "-f", "flac"]);
}

#[test]
fn original_mka_preserves_selected_title_and_language_on_the_audio_stream() {
    let args = invocation_args(
        AudioExtractFormat::Original,
        &AudioExtractMetadata {
            title: Some("Director commentary".to_owned()),
            language: Some("eng".to_owned()),
        },
    );

    assert_metadata_pair(&args, "-metadata:s:a:0", "title=Director commentary");
    assert_metadata_pair(&args, "-metadata:s:a:0", "language=eng");
    assert!(
        !args
            .windows(2)
            .any(|pair| pair == ["-metadata", "title=Director commentary"])
    );
}

#[test]
fn aac_m4a_uses_standalone_title_and_stream_language() {
    let args = invocation_args(
        AudioExtractFormat::Aac,
        &AudioExtractMetadata {
            title: Some("Main program".to_owned()),
            language: Some("jpn".to_owned()),
        },
    );

    assert_metadata_pair(&args, "-metadata", "title=Main program");
    assert_metadata_pair(&args, "-metadata:s:a:0", "language=jpn");
    assert!(
        !args
            .windows(2)
            .any(|pair| pair == ["-metadata:s:a:0", "title=Main program"])
    );
}

#[test]
fn flac_uses_title_and_language_vorbis_comments() {
    let args = invocation_args(
        AudioExtractFormat::Flac,
        &AudioExtractMetadata {
            title: Some("Music".to_owned()),
            language: Some("deu".to_owned()),
        },
    );

    assert_metadata_pair(&args, "-metadata", "title=Music");
    assert_metadata_pair(&args, "-metadata", "language=deu");
}

#[test]
fn missing_selected_metadata_is_not_fabricated_or_copied_from_the_container() {
    let args = invocation_args(
        AudioExtractFormat::Original,
        &AudioExtractMetadata::default(),
    );

    assert!(args.windows(2).any(|pair| pair == ["-map_metadata", "-1"]));
    assert!(
        !args.iter().any(|argument| {
            argument.starts_with("title=") || argument.starts_with("language=")
        })
    );
}

fn assert_invocation(format: AudioExtractFormat, required_pairs: &[&str]) {
    let request = request(format, 7);
    let args = invocation_args(format, &AudioExtractMetadata::default());

    assert_eq!(args[0], "-i");
    assert_eq!(args[1], "C:/media/source recording.mkv");
    assert_eq!(
        args.iter().filter(|argument| *argument == "-map").count(),
        1
    );
    assert!(args.windows(2).any(|pair| pair == ["-map", "0:7"]));
    assert!(!args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "0" | "0:0" | "0:3" | "0:11" | "0:v" | "0:s" | "0:d"
        )
    }));
    for pair in required_pairs.chunks_exact(2) {
        assert!(
            args.windows(2).any(|actual| actual == [pair[0], pair[1]]),
            "missing pair {pair:?} from {args:?}"
        );
    }
    if format != AudioExtractFormat::Aac {
        assert!(!args.iter().any(|argument| argument == "-b:a"));
    }
    assert!(args.windows(2).any(|pair| pair == ["-progress", "pipe:1"]));
    assert!(args.windows(2).any(|pair| pair == ["-map_metadata", "-1"]));
    assert!(args.iter().any(|argument| argument == "-nostats"));
    assert_eq!(args.last().unwrap(), &request.output.to_string_lossy());
}

fn invocation_args(format: AudioExtractFormat, metadata: &AudioExtractMetadata) -> Vec<String> {
    build_audio_extract_invocation(&request(format, 7), metadata)
        .args
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

fn assert_metadata_pair(args: &[String], option: &str, value: &str) {
    assert!(
        args.windows(2).any(|pair| pair == [option, value]),
        "missing {option} {value:?} from {args:?}"
    );
}
