use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioExtractFormat {
    Original,
    Aac,
    Flac,
}

impl AudioExtractFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Original => "mka",
            Self::Aac => "m4a",
            Self::Flac => "flac",
        }
    }

    pub const fn ffmpeg_muxer_name(self) -> &'static str {
        match self {
            Self::Original => "matroska",
            Self::Aac => "mp4",
            Self::Flac => "flac",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioExtractRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub audio_stream_index: u32,
    pub format: AudioExtractFormat,
}
