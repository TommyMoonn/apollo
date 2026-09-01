use crate::media::model::MediaFile;

use super::{
    audio_extract::AudioExtractPresentation, audio_mix::AudioMixPresentation,
    frame_capture::FrameCapturePresentation, remux::RemuxPresentation,
    workspace::WorkspacePresentation,
};

/// Immutable UI data derived from one accepted media inspection.
///
/// This is built at the source publication boundary and shared by every operation. Mutable
/// selection and output state remains in the operation sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePresentation {
    pub workspace: WorkspacePresentation,
    pub audio_mix: AudioMixPresentation,
    pub audio_extract: AudioExtractPresentation,
    pub remux: RemuxPresentation,
    pub frame_capture: FrameCapturePresentation,
}

impl SourcePresentation {
    pub fn new(media: &MediaFile) -> Self {
        Self {
            workspace: WorkspacePresentation::new(media),
            audio_mix: AudioMixPresentation::new(media),
            audio_extract: AudioExtractPresentation::new(media),
            remux: RemuxPresentation::new(media),
            frame_capture: FrameCapturePresentation::new(media),
        }
    }
}
