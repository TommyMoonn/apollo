use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMixRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub selected_streams: Vec<u32>,
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
