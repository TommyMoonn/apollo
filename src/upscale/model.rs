use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpscaleTarget {
    Qhd1440,
    Uhd4k,
    Uhd8k,
    Custom { max_width: u32, max_height: u32 },
}

impl UpscaleTarget {
    pub const fn boundary(self) -> (u32, u32) {
        match self {
            Self::Qhd1440 => (2560, 1440),
            Self::Uhd4k => (3840, 2160),
            Self::Uhd8k => (7680, 4320),
            Self::Custom {
                max_width,
                max_height,
            } => (max_width, max_height),
        }
    }

    pub const fn is_custom(self) -> bool {
        matches!(self, Self::Custom { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    H265,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpscaleRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub target: UpscaleTarget,
    pub codec: VideoCodec,
}
