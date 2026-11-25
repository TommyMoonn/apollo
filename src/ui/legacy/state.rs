use super::audio_extract::AudioExtractToolState;
use super::audio_mix::AudioMixToolState;
use super::remux::RemuxToolState;
use super::upscale::UpscaleToolState;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use egui::TextureHandle;

use crate::app::frame_capture::{FrameCaptureEditorState, PositionSettle};
use crate::frame_capture::frame_index::FrameDirection;
use crate::frame_capture::model::FramePosition;
use crate::frame_capture::preview::PreviewGeneration;
use crate::media::error::MediaError;
use crate::media::intake::SourceIdentity;
use crate::media::model::MediaFile;
use crate::ui::frame_capture_progress::FrameCaptureLifecycleUiState;
use crate::ui::source_presentation::SourcePresentation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrameCaptureToolState {
    pub(super) media: MediaFile,
    pub(super) editor: FrameCaptureEditor,
    pub(super) lifecycle_ui: FrameCaptureLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

impl Deref for FrameCaptureToolState {
    type Target = FrameCaptureEditor;

    fn deref(&self) -> &Self::Target {
        &self.editor
    }
}

impl DerefMut for FrameCaptureToolState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.editor
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrameCaptureEditor {
    pub(super) state: FrameCaptureEditorState,
    pub(super) preview_texture: Option<FramePreviewTexture>,
}
impl FrameCaptureEditor {
    pub(super) fn new(
        session_id: u64,
        source_identity: Option<SourceIdentity>,
        media: &MediaFile,
    ) -> Self {
        Self {
            state: FrameCaptureEditorState::new(session_id, source_identity, media),
            preview_texture: None,
        }
    }
}
impl Deref for FrameCaptureEditor {
    type Target = FrameCaptureEditorState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
impl DerefMut for FrameCaptureEditor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct FramePreviewTexture {
    pub(super) generation: PreviewGeneration,
    pub(super) size: [usize; 2],
    pub(super) position: FramePosition,
    pub(super) handle: TextureHandle,
}

impl std::fmt::Debug for FramePreviewTexture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FramePreviewTexture")
            .field("generation", &self.generation)
            .field("size", &self.size)
            .field("position", &self.position)
            .field("texture_id", &self.handle.id())
            .finish()
    }
}

pub(super) struct EditorPresentation {
    pub(super) source: SourcePresentation,
    pub(super) preview_texture: Option<FramePreviewTexture>,
}

impl EditorPresentation {
    pub(super) fn new(media: &MediaFile) -> Self {
        Self {
            source: SourcePresentation::new(media),
            preview_texture: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueueEditAction {
    None,
    BrowseOutput,
    Save,
    Cancel,
    FramePositionEdited(PositionSettle),
    FrameStepRequested(FrameDirection),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct LegacyEditorState {
    pub(super) intake_error_details_open: bool,
    pub(super) source_presentation: Option<Arc<SourcePresentation>>,
    pub(super) audio_mix: Option<AudioMixToolState>,
    pub(super) upscale: Option<UpscaleToolState>,
    pub(super) remux: Option<RemuxToolState>,
    pub(super) audio_extract: Option<AudioExtractToolState>,
    pub(super) frame_capture: Option<FrameCaptureToolState>,
}

impl LegacyEditorState {
    pub(super) fn retire_tool_sessions(&mut self) {
        self.audio_mix = None;
        self.upscale = None;
        self.remux = None;
        self.audio_extract = None;
        self.frame_capture = None;
    }
}
