pub mod audio_extract;
pub mod audio_extract_progress;
pub mod audio_mix;
pub mod audio_mix_progress;
pub mod file_intake;
pub mod fonts;
pub mod frame_capture;
pub mod frame_capture_progress;
pub mod icon;
pub mod layout;
pub mod media_error;
mod media_lifecycle;
mod output_name;
pub mod remux;
pub mod remux_progress;
pub mod source_presentation;
pub mod theme;
pub mod upscale;
pub mod upscale_progress;
pub mod widgets;
pub mod workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAction {
    ChangeFile,
    CloseSource,
}
