use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub path: PathBuf,
    pub duration: Option<Duration>,
    pub streams: Vec<MediaStream>,
    pub video_streams: Vec<VideoStream>,
    pub audio_streams: Vec<AudioStream>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaStream {
    pub stream_index: u32,
    pub kind: StreamKind,
    pub codec: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStream {
    pub stream_index: u32,
    pub codec: Option<String>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub title: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoStream {
    pub stream_index: u32,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<FrameRate>,
    pub format: VideoFormat,
}

/// Optional source signaling. This does not describe HDR mastering or content-light metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoFormat {
    pub pixel_format: Option<String>,
    pub bit_depth: Option<u8>,
    pub color_range: Option<String>,
    pub color_space: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
}

/// Known FFmpeg pixel identities, not a guess based on codec or container.
pub fn pixel_format_bit_depth(pixel_format: &str) -> Option<u8> {
    match pixel_format {
        "yuv420p" | "yuvj420p" | "nv12" | "nv21" | "yuv422p" | "yuv444p" | "yuvj422p"
        | "yuvj444p" | "gbrp" | "gray" | "rgb24" | "bgr24" => Some(8),
        "p010le" | "p010be" => Some(10),
        "p016le" | "p016be" => Some(16),
        name => {
            let stem = name
                .strip_suffix("le")
                .or_else(|| name.strip_suffix("be"))?;
            for prefix in ["yuv420p", "yuv422p", "yuv444p", "gbrp", "gray"] {
                if let Some(depth) = stem.strip_prefix(prefix) {
                    return depth
                        .parse::<u8>()
                        .ok()
                        .filter(|depth| matches!(depth, 9 | 10 | 12 | 14 | 16));
                }
            }
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }

        Some(Self {
            numerator,
            denominator,
        })
    }
}
