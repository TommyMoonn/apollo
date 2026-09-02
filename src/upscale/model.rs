use std::path::PathBuf;

use crate::media::tools::VideoEncoderCapability;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VideoEncoderSelection {
    #[default]
    Auto,
    Cpu,
    NvidiaNvenc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoEncoderBackend {
    Cpu,
    NvidiaNvenc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuQuality(u8);

impl CpuQuality {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 51;
    pub const H264_DEFAULT: Self = Self(18);
    pub const H265_DEFAULT: Self = Self(20);

    pub const fn new(value: u8) -> Option<Self> {
        if value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NvencQuality(u8);

impl NvencQuality {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 51;
    pub const DEFAULT: Self = Self(16);

    pub const fn new(value: u8) -> Option<Self> {
        if value >= Self::MIN && value <= Self::MAX {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderQuality {
    Cpu(CpuQuality),
    NvidiaNvenc(NvencQuality),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpscaleEncoderSettings {
    selection: VideoEncoderSelection,
    h264_cpu_quality: CpuQuality,
    h265_cpu_quality: CpuQuality,
    h264_nvenc_quality: NvencQuality,
    h265_nvenc_quality: NvencQuality,
}

impl Default for UpscaleEncoderSettings {
    fn default() -> Self {
        Self {
            selection: VideoEncoderSelection::Auto,
            h264_cpu_quality: CpuQuality::H264_DEFAULT,
            h265_cpu_quality: CpuQuality::H265_DEFAULT,
            h264_nvenc_quality: NvencQuality::DEFAULT,
            h265_nvenc_quality: NvencQuality::DEFAULT,
        }
    }
}

impl UpscaleEncoderSettings {
    pub const fn selection(&self) -> VideoEncoderSelection {
        self.selection
    }

    pub fn set_selection(&mut self, selection: VideoEncoderSelection) {
        self.selection = selection;
    }

    pub const fn cpu_quality(&self, codec: VideoCodec) -> CpuQuality {
        match codec {
            VideoCodec::H264 => self.h264_cpu_quality,
            VideoCodec::H265 => self.h265_cpu_quality,
        }
    }

    pub fn set_cpu_quality(&mut self, codec: VideoCodec, quality: CpuQuality) {
        match codec {
            VideoCodec::H264 => self.h264_cpu_quality = quality,
            VideoCodec::H265 => self.h265_cpu_quality = quality,
        }
    }

    pub const fn nvenc_quality(&self, codec: VideoCodec) -> NvencQuality {
        match codec {
            VideoCodec::H264 => self.h264_nvenc_quality,
            VideoCodec::H265 => self.h265_nvenc_quality,
        }
    }

    pub fn set_nvenc_quality(&mut self, codec: VideoCodec, quality: NvencQuality) {
        match codec {
            VideoCodec::H264 => self.h264_nvenc_quality = quality,
            VideoCodec::H265 => self.h265_nvenc_quality = quality,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedVideoEncoder {
    pub backend: VideoEncoderBackend,
    pub quality: EncoderQuality,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvencUnavailableError {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpscaleRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub target: UpscaleTarget,
    pub codec: VideoCodec,
    pub encoder_settings: UpscaleEncoderSettings,
    pub nvenc_capability: VideoEncoderCapability,
}

impl UpscaleRequest {
    pub fn resolve_video_encoder(&self) -> Result<ResolvedVideoEncoder, NvencUnavailableError> {
        let backend = match self.encoder_settings.selection() {
            VideoEncoderSelection::Cpu => VideoEncoderBackend::Cpu,
            VideoEncoderSelection::Auto => {
                if self.nvenc_capability == VideoEncoderCapability::Available {
                    VideoEncoderBackend::NvidiaNvenc
                } else {
                    VideoEncoderBackend::Cpu
                }
            }
            VideoEncoderSelection::NvidiaNvenc => {
                if let Some(reason) = self.nvenc_capability.unavailable_reason() {
                    return Err(NvencUnavailableError {
                        reason: reason.to_owned(),
                    });
                }
                VideoEncoderBackend::NvidiaNvenc
            }
        };

        let quality = match backend {
            VideoEncoderBackend::Cpu => {
                EncoderQuality::Cpu(self.encoder_settings.cpu_quality(self.codec))
            }
            VideoEncoderBackend::NvidiaNvenc => {
                EncoderQuality::NvidiaNvenc(self.encoder_settings.nvenc_quality(self.codec))
            }
        };

        Ok(ResolvedVideoEncoder { backend, quality })
    }
}
