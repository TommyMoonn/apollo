use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpscaleTarget {
    Qhd1440,
    Uhd4k,
    Uhd8k,
}

impl UpscaleTarget {
    pub const fn boundary(self) -> (u32, u32) {
        match self {
            Self::Qhd1440 => (2560, 1440),
            Self::Uhd4k => (3840, 2160),
            Self::Uhd8k => (7680, 4320),
        }
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
