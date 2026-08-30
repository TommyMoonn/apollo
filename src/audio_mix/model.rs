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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoudnessTargets {
    integrated_loudness_tenths: i16,
    maximum_true_peak_tenths: i16,
    loudness_range_tenths: i16,
}

impl LoudnessTargets {
    pub const MIN_INTEGRATED_LOUDNESS_TENTHS: i16 = -700;
    pub const MAX_INTEGRATED_LOUDNESS_TENTHS: i16 = -50;
    pub const MIN_MAXIMUM_TRUE_PEAK_TENTHS: i16 = -90;
    pub const MAX_MAXIMUM_TRUE_PEAK_TENTHS: i16 = 0;
    pub const MIN_LOUDNESS_RANGE_TENTHS: i16 = 10;
    pub const MAX_LOUDNESS_RANGE_TENTHS: i16 = 500;

    pub const fn from_tenths(
        integrated_loudness_tenths: i16,
        maximum_true_peak_tenths: i16,
        loudness_range_tenths: i16,
    ) -> Self {
        Self {
            integrated_loudness_tenths,
            maximum_true_peak_tenths,
            loudness_range_tenths,
        }
    }

    pub const fn integrated_loudness_tenths(self) -> i16 {
        self.integrated_loudness_tenths
    }

    pub const fn maximum_true_peak_tenths(self) -> i16 {
        self.maximum_true_peak_tenths
    }

    pub const fn loudness_range_tenths(self) -> i16 {
        self.loudness_range_tenths
    }

    pub const fn integrated_loudness_is_valid(self) -> bool {
        self.integrated_loudness_tenths >= Self::MIN_INTEGRATED_LOUDNESS_TENTHS
            && self.integrated_loudness_tenths <= Self::MAX_INTEGRATED_LOUDNESS_TENTHS
    }

    pub const fn maximum_true_peak_is_valid(self) -> bool {
        self.maximum_true_peak_tenths >= Self::MIN_MAXIMUM_TRUE_PEAK_TENTHS
            && self.maximum_true_peak_tenths <= Self::MAX_MAXIMUM_TRUE_PEAK_TENTHS
    }

    pub const fn loudness_range_is_valid(self) -> bool {
        self.loudness_range_tenths >= Self::MIN_LOUDNESS_RANGE_TENTHS
            && self.loudness_range_tenths <= Self::MAX_LOUDNESS_RANGE_TENTHS
    }
}

impl Default for LoudnessTargets {
    fn default() -> Self {
        Self::from_tenths(-160, -15, 110)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LoudnessNormalization {
    #[default]
    Off,
    Enabled(LoudnessTargets),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMixRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub selected_tracks: Vec<SelectedAudioTrack>,
    pub normalization: LoudnessNormalization,
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
