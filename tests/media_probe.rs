use std::{path::Path, time::Duration};

use apollo::media::{
    model::{FrameRate, StreamKind},
    probe::{MediaProbeError, parse_ffprobe_json},
};

fn parse_fixture(name: &str) -> apollo::media::model::MediaFile {
    let json = match name {
        "one_audio" => include_bytes!("fixtures/ffprobe/one_audio.json").as_slice(),
        "multiple_audio" => include_bytes!("fixtures/ffprobe/multiple_audio.json").as_slice(),
        "missing_title" => include_bytes!("fixtures/ffprobe/missing_title.json").as_slice(),
        "missing_tags" => include_bytes!("fixtures/ffprobe/missing_tags.json").as_slice(),
        "missing_language" => include_bytes!("fixtures/ffprobe/missing_language.json").as_slice(),
        "missing_channel_layout" => {
            include_bytes!("fixtures/ffprobe/missing_channel_layout.json").as_slice()
        }
        "video_no_audio" => include_bytes!("fixtures/ffprobe/video_no_audio.json").as_slice(),
        "multiple_video" => include_bytes!("fixtures/ffprobe/multiple_video.json").as_slice(),
        "complete_inventory" => {
            include_bytes!("fixtures/ffprobe/complete_inventory.json").as_slice()
        }
        "unknown_codec" => include_bytes!("fixtures/ffprobe/unknown_codec.json").as_slice(),
        "missing_duration" => include_bytes!("fixtures/ffprobe/missing_duration.json").as_slice(),
        _ => panic!("unknown fixture: {name}"),
    };

    parse_ffprobe_json(Path::new("C:/media/input recording.mkv"), json)
        .expect("fixture should parse")
}

#[test]
fn one_audio_stream_is_normalized_with_actual_stream_index() {
    let media = parse_fixture("one_audio");

    assert_eq!(media.path, Path::new("C:/media/input recording.mkv"));
    assert_eq!(media.duration, Some(Duration::from_millis(12_500)));
    assert_eq!(media.video_streams.len(), 1);
    assert_eq!(media.video_streams[0].stream_index, 0);
    assert_eq!(
        media.video_streams[0].frame_rate,
        FrameRate::new(30_000, 1_001)
    );
    assert_eq!(media.audio_streams.len(), 1);
    assert_eq!(media.audio_streams[0].stream_index, 2);
    assert_eq!(media.audio_streams[0].title.as_deref(), Some("Main Mix"));
    assert_eq!(media.audio_streams[0].language.as_deref(), Some("eng"));
}

#[test]
fn multiple_audio_streams_preserve_non_contiguous_indexes_and_order() {
    let media = parse_fixture("multiple_audio");
    let indexes = media
        .audio_streams
        .iter()
        .map(|stream| stream.stream_index)
        .collect::<Vec<_>>();

    assert_eq!(indexes, [1, 3, 7]);
    assert_eq!(media.audio_streams[1].channels, Some(1));
    assert_eq!(
        media.audio_streams[1].channel_layout.as_deref(),
        Some("mono")
    );
}

#[test]
fn missing_title_is_tolerated() {
    let media = parse_fixture("missing_title");
    assert_eq!(media.audio_streams[0].title, None);
    assert_eq!(media.audio_streams[0].language.as_deref(), Some("eng"));
}

#[test]
fn missing_tags_object_is_tolerated() {
    let media = parse_fixture("missing_tags");
    assert_eq!(media.audio_streams[0].title, None);
    assert_eq!(media.audio_streams[0].language, None);
}

#[test]
fn missing_language_is_tolerated() {
    let media = parse_fixture("missing_language");
    assert_eq!(media.audio_streams[0].language, None);
    assert_eq!(media.audio_streams[0].title.as_deref(), Some("Lossless"));
}

#[test]
fn missing_channel_layout_is_tolerated() {
    let media = parse_fixture("missing_channel_layout");
    assert_eq!(media.audio_streams[0].channels, Some(6));
    assert_eq!(media.audio_streams[0].channel_layout, None);
}

#[test]
fn video_without_audio_has_empty_audio_collection() {
    let media = parse_fixture("video_no_audio");
    assert_eq!(media.video_streams.len(), 1);
    assert!(media.audio_streams.is_empty());
}

#[test]
fn multiple_video_streams_are_preserved_and_frame_rate_falls_back_safely() {
    let media = parse_fixture("multiple_video");
    let indexes = media
        .video_streams
        .iter()
        .map(|stream| stream.stream_index)
        .collect::<Vec<_>>();

    assert_eq!(indexes, [0, 4]);
    assert_eq!(media.video_streams[1].frame_rate, FrameRate::new(25, 1));
    assert_eq!(media.streams[2].kind, StreamKind::Subtitle);
    assert_eq!(media.streams[2].stream_index, 9);
}

#[test]
fn complete_stream_inventory_preserves_probe_order_indexes_kinds_and_codecs() {
    let media = parse_fixture("complete_inventory");
    let inventory = media
        .streams
        .iter()
        .map(|stream| (stream.stream_index, stream.kind, stream.codec.as_deref()))
        .collect::<Vec<_>>();

    assert_eq!(
        inventory,
        [
            (8, StreamKind::Audio, Some("aac")),
            (2, StreamKind::Video, Some("h264")),
            (11, StreamKind::Subtitle, Some("subrip")),
            (4, StreamKind::Data, Some("bin_data")),
            (15, StreamKind::Attachment, Some("ttf")),
            (6, StreamKind::Unknown, Some("future_codec")),
            (20, StreamKind::Unknown, None),
        ]
    );
    assert_eq!(media.video_streams[0].stream_index, 2);
    assert_eq!(media.audio_streams[0].stream_index, 8);
}

#[test]
fn unknown_codec_name_is_preserved_as_metadata() {
    let media = parse_fixture("unknown_codec");
    assert_eq!(
        media.audio_streams[0].codec.as_deref(),
        Some("future_codec")
    );
}

#[test]
fn malformed_json_returns_typed_error() {
    let error = parse_ffprobe_json(
        Path::new("input.mkv"),
        include_bytes!("fixtures/ffprobe/malformed.json"),
    )
    .expect_err("malformed JSON must fail");

    assert!(matches!(error, MediaProbeError::InvalidOutput { .. }));
}

#[test]
fn missing_duration_is_tolerated() {
    let media = parse_fixture("missing_duration");
    assert_eq!(media.duration, None);
}
