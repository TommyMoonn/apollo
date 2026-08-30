use std::path::PathBuf;

use apollo::{
    media::{
        container::MediaContainer,
        model::{MediaFile, MediaStream, StreamKind},
    },
    remux::{
        command::build_remux_invocation,
        model::RemuxRequest,
        validation::{RemuxValidationError, validate_remux_request},
    },
};

fn request(container: MediaContainer) -> RemuxRequest {
    RemuxRequest {
        input: PathBuf::from("C:/media/source recording.mkv"),
        output: PathBuf::from(format!(
            "C:/media/source recording-remuxed.{}",
            container.extension()
        )),
        container,
    }
}

fn media(streams: impl IntoIterator<Item = (u32, StreamKind, Option<&'static str>)>) -> MediaFile {
    MediaFile {
        path: PathBuf::from("C:/media/source recording.mkv"),
        duration: None,
        streams: streams
            .into_iter()
            .map(|(stream_index, kind, codec)| MediaStream {
                stream_index,
                kind,
                codec: codec.map(str::to_owned),
            })
            .collect(),
        video_streams: Vec::new(),
        audio_streams: Vec::new(),
    }
}

#[test]
fn compatible_mp4_inventory_is_valid() {
    let request = request(MediaContainer::Mp4);
    let media = media([
        (0, StreamKind::Video, Some("h264")),
        (3, StreamKind::Audio, Some("aac")),
        (7, StreamKind::Subtitle, Some("mov_text")),
    ]);

    assert_eq!(validate_remux_request(&request, &media), Ok(()));
}

#[test]
fn incompatible_mp4_inventory_is_rejected_with_absolute_stream_identity() {
    let request = request(MediaContainer::Mp4);
    let media = media([
        (0, StreamKind::Video, Some("h264")),
        (9, StreamKind::Subtitle, Some("subrip")),
    ]);

    assert_eq!(
        validate_remux_request(&request, &media),
        Err(RemuxValidationError::IncompatibleStream {
            container: MediaContainer::Mp4,
            stream_index: 9,
            kind: StreamKind::Subtitle,
            codec: Some("subrip".to_owned()),
        })
    );
}

#[test]
fn matroska_accepts_copyable_inventory_and_rejects_data() {
    let request = request(MediaContainer::Matroska);
    let compatible = media([
        (1, StreamKind::Video, Some("ffv1")),
        (4, StreamKind::Audio, Some("flac")),
        (8, StreamKind::Subtitle, Some("subrip")),
        (12, StreamKind::Attachment, Some("ttf")),
    ]);
    let with_data = media([
        (1, StreamKind::Video, Some("ffv1")),
        (15, StreamKind::Data, Some("bin_data")),
    ]);

    assert_eq!(validate_remux_request(&request, &compatible), Ok(()));
    assert!(matches!(
        validate_remux_request(&request, &with_data),
        Err(RemuxValidationError::IncompatibleStream {
            stream_index: 15,
            kind: StreamKind::Data,
            ..
        })
    ));
}

#[test]
fn empty_inventory_is_rejected() {
    assert_eq!(
        validate_remux_request(&request(MediaContainer::Matroska), &media([])),
        Err(RemuxValidationError::NoStreams)
    );
}

#[test]
fn audio_only_and_video_only_sources_are_valid() {
    let mp4 = request(MediaContainer::Mp4);

    assert_eq!(
        validate_remux_request(&mp4, &media([(4, StreamKind::Audio, Some("aac"))])),
        Ok(())
    );
    assert_eq!(
        validate_remux_request(&mp4, &media([(8, StreamKind::Video, Some("h264"))])),
        Ok(())
    );
}

#[test]
fn output_extension_must_match_selected_container() {
    let mut mp4 = request(MediaContainer::Mp4);
    mp4.output.set_extension("mkv");
    let source = media([(0, StreamKind::Video, Some("h264"))]);

    assert_eq!(
        validate_remux_request(&mp4, &source),
        Err(RemuxValidationError::OutputExtensionMismatch {
            expected: "mp4",
            actual: Some("mkv".to_owned()),
        })
    );

    let mut mkv = request(MediaContainer::Matroska);
    mkv.output.set_extension("MKV");
    assert_eq!(validate_remux_request(&mkv, &source), Ok(()));
}

#[test]
fn command_maps_all_streams_copies_codecs_and_preserves_metadata_and_chapters() {
    let request = request(MediaContainer::Matroska);
    let invocation = build_remux_invocation(&request);
    let args = invocation
        .args
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        args,
        [
            "-i",
            "C:/media/source recording.mkv",
            "-map",
            "0",
            "-c",
            "copy",
            "-map_metadata",
            "0",
            "-map_chapters",
            "0",
            "-progress",
            "pipe:1",
            "-nostats",
            "-f",
            "matroska",
            "C:/media/source recording-remuxed.mkv",
        ]
    );
    assert!(!args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "-c:v" | "-c:a" | "-c:s" | "-codec:v" | "-codec:a" | "-codec:s"
        )
    }));
}

#[test]
fn mp4_command_uses_mp4_muxer_and_keeps_paths_as_single_arguments() {
    let request = request(MediaContainer::Mp4);
    let invocation = build_remux_invocation(&request);

    assert_eq!(invocation.args[1], request.input);
    assert_eq!(invocation.args.last().unwrap(), request.output.as_os_str());
    assert!(invocation.args.windows(2).any(|pair| pair == ["-f", "mp4"]));
}
