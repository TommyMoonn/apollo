use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub path: PathBuf,
    pub duration: Option<Duration>,
    pub video_streams: Vec<VideoStream>,
    pub audio_streams: Vec<AudioStream>,
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
