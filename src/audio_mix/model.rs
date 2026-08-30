use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackGain {
    tenths_db: i16,
}

impl TrackGain {
    pub const MIN: Self = Self::from_tenths_db(-600);
    pub const MAX: Self = Self::from_tenths_db(120);
    pub const ZERO: Self = Self::from_tenths_db(0);

    pub const fn from_tenths_db(tenths_db: i16) -> Self {
        Self { tenths_db }
    }

    pub const fn tenths_db(self) -> i16 {
        self.tenths_db
    }

    pub const fn is_valid(self) -> bool {
        self.tenths_db >= Self::MIN.tenths_db && self.tenths_db <= Self::MAX.tenths_db
    }
}

impl Default for TrackGain {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Display for TrackGain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let magnitude = i32::from(self.tenths_db).abs();
        let sign = if self.tenths_db < 0 { "-" } else { "" };
        write!(f, "{sign}{}.{:01}dB", magnitude / 10, magnitude % 10)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectedAudioTrack {
    pub stream_index: u32,
    pub gain: TrackGain,
}

impl SelectedAudioTrack {
    pub const fn new(stream_index: u32, gain: TrackGain) -> Self {
        Self { stream_index, gain }
    }

    pub const fn with_default_gain(stream_index: u32) -> Self {
        Self::new(stream_index, TrackGain::ZERO)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMixRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub selected_tracks: Vec<SelectedAudioTrack>,
    pub container: OutputContainer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputContainer {
    Mp4,
    Mkv,
}

impl OutputContainer {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
        }
    }
}
