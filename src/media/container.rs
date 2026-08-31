use super::model::{MediaStream, StreamKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaContainer {
    Mp4,
    Matroska,
}

impl MediaContainer {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Matroska => "mkv",
        }
    }

    pub const fn ffmpeg_muxer_name(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Matroska => "matroska",
        }
    }

    pub fn is_stream_copy_compatible(self, stream: &MediaStream) -> bool {
        self.is_stream_copy_codec_compatible(stream.kind, stream.codec.as_deref())
    }

    pub fn first_incompatible_stream(self, streams: &[MediaStream]) -> Option<&MediaStream> {
        streams
            .iter()
            .find(|stream| !self.is_stream_copy_compatible(stream))
    }

    pub fn is_inventory_stream_copy_compatible(self, streams: &[MediaStream]) -> bool {
        self.first_incompatible_stream(streams).is_none()
    }

    pub fn is_stream_copy_codec_compatible(self, kind: StreamKind, codec: Option<&str>) -> bool {
        let Some(codec) = codec.filter(|codec| !codec.is_empty()) else {
            return false;
        };

        match self {
            Self::Mp4 => mp4_codec_is_compatible(kind, codec),
            Self::Matroska => matches!(
                kind,
                StreamKind::Video
                    | StreamKind::Audio
                    | StreamKind::Subtitle
                    | StreamKind::Attachment
            ),
        }
    }
}

fn mp4_codec_is_compatible(kind: StreamKind, codec: &str) -> bool {
    let compatible_codecs: &[&str] = match kind {
        StreamKind::Video => &["h264", "hevc", "mpeg4", "av1", "vp9", "mjpeg"],
        StreamKind::Audio => &[
            "aac",
            "alac",
            "ac3",
            "eac3",
            "mp3",
            "opus",
            "flac",
            "vorbis",
            "dts",
            "pcm_s16le",
            "pcm_s16be",
            "pcm_s24le",
            "pcm_s24be",
            "pcm_s32le",
            "pcm_s32be",
        ],
        StreamKind::Subtitle => &["mov_text"],
        StreamKind::Data | StreamKind::Attachment | StreamKind::Unknown => return false,
    };

    compatible_codecs
        .iter()
        .any(|compatible| codec.eq_ignore_ascii_case(compatible))
}

#[cfg(test)]
mod tests {
    use super::MediaContainer;
    use crate::media::model::{MediaStream, StreamKind};

    fn stream(kind: StreamKind, codec: Option<&str>) -> MediaStream {
        MediaStream {
            stream_index: 7,
            kind,
            codec: codec.map(str::to_owned),
        }
    }

    #[test]
    fn containers_own_extensions_and_ffmpeg_muxer_names() {
        assert_eq!(MediaContainer::Mp4.extension(), "mp4");
        assert_eq!(MediaContainer::Mp4.ffmpeg_muxer_name(), "mp4");
        assert_eq!(MediaContainer::Matroska.extension(), "mkv");
        assert_eq!(MediaContainer::Matroska.ffmpeg_muxer_name(), "matroska");
    }

    #[test]
    fn mp4_accepts_known_stream_copy_codecs() {
        for (kind, codec) in [
            (StreamKind::Video, "h264"),
            (StreamKind::Video, "hevc"),
            (StreamKind::Audio, "aac"),
            (StreamKind::Audio, "pcm_s24le"),
            (StreamKind::Subtitle, "mov_text"),
        ] {
            assert!(
                MediaContainer::Mp4.is_stream_copy_compatible(&stream(kind, Some(codec))),
                "expected {kind:?}/{codec} to be compatible"
            );
        }
    }

    #[test]
    fn mp4_rejects_unsupported_or_unknown_inventory() {
        for candidate in [
            stream(StreamKind::Video, Some("ffv1")),
            stream(StreamKind::Audio, Some("wmav2")),
            stream(StreamKind::Subtitle, Some("subrip")),
            stream(StreamKind::Data, Some("bin_data")),
            stream(StreamKind::Attachment, Some("ttf")),
            stream(StreamKind::Unknown, Some("future_codec")),
            stream(StreamKind::Video, None),
        ] {
            assert!(!MediaContainer::Mp4.is_stream_copy_compatible(&candidate));
        }
    }

    #[test]
    fn matroska_accepts_supported_stream_kinds_including_attachments() {
        for kind in [
            StreamKind::Video,
            StreamKind::Audio,
            StreamKind::Subtitle,
            StreamKind::Attachment,
        ] {
            assert!(
                MediaContainer::Matroska
                    .is_stream_copy_compatible(&stream(kind, Some("known_codec")))
            );
        }
    }

    #[test]
    fn matroska_rejects_data_unknown_and_missing_codec_inventory() {
        assert!(
            !MediaContainer::Matroska
                .is_stream_copy_compatible(&stream(StreamKind::Data, Some("bin_data")))
        );
        assert!(
            !MediaContainer::Matroska
                .is_stream_copy_compatible(&stream(StreamKind::Unknown, Some("future_codec")))
        );
        assert!(
            !MediaContainer::Matroska.is_stream_copy_compatible(&stream(StreamKind::Video, None))
        );
    }

    #[test]
    fn inventory_query_returns_the_first_incompatible_absolute_stream() {
        let streams = vec![
            stream(StreamKind::Video, Some("h264")),
            MediaStream {
                stream_index: 11,
                kind: StreamKind::Subtitle,
                codec: Some("subrip".to_owned()),
            },
            MediaStream {
                stream_index: 14,
                kind: StreamKind::Data,
                codec: Some("bin_data".to_owned()),
            },
        ];

        assert_eq!(
            MediaContainer::Mp4
                .first_incompatible_stream(&streams)
                .map(|stream| stream.stream_index),
            Some(11)
        );
        assert!(!MediaContainer::Mp4.is_inventory_stream_copy_compatible(&streams));
        assert!(!MediaContainer::Matroska.is_inventory_stream_copy_compatible(&streams));

        let copyable = &streams[..1];
        assert!(MediaContainer::Mp4.is_inventory_stream_copy_compatible(copyable));
        assert!(MediaContainer::Matroska.is_inventory_stream_copy_compatible(copyable));
    }
}
