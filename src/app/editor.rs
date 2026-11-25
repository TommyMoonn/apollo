//! Shared typed drafts for fixed-source operation editors.
use super::{Operation, frame_capture::FrameCaptureEditorState};
use crate::{
    audio_extract::draft::AudioExtractDraft,
    audio_mix::draft::AudioMixDraft,
    media::{error::MediaError, intake::SourceIdentity, model::MediaFile},
    queue::QueuedOperation,
    remux::draft::RemuxDraft,
    upscale::draft::{UpscaleDraft, UpscaleEncoderCapabilities},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationEditorState {
    pub(crate) session_id: u64,
    pub(crate) media: MediaFile,
    pub(crate) active_operation: Operation,
    pub(crate) audio_mix: AudioMixDraft,
    pub(crate) upscale: UpscaleDraft,
    pub(crate) remux: RemuxDraft,
    pub(crate) audio_extract: AudioExtractDraft,
    pub(crate) frame_capture: FrameCaptureEditorState,
    pub(crate) error: Option<MediaError>,
    pub(crate) error_details_open: bool,
}

impl OperationEditorState {
    pub(crate) fn new(
        session_id: u64,
        source_identity: Option<SourceIdentity>,
        media: MediaFile,
        active_operation: Operation,
    ) -> Self {
        Self {
            session_id,
            active_operation,
            audio_mix: AudioMixDraft::new(&media),
            upscale: UpscaleDraft::new(&media),
            remux: RemuxDraft::new(&media),
            audio_extract: AudioExtractDraft::new(&media),
            frame_capture: FrameCaptureEditorState::new(session_id, source_identity, &media),
            media,
            error: None,
            error_details_open: false,
        }
    }

    pub(crate) fn hydrate(&mut self, operation: &QueuedOperation) {
        match operation {
            QueuedOperation::AudioMix(request) => {
                self.audio_mix = AudioMixDraft::from_request(&self.media, request);
            }
            QueuedOperation::Upscale(request) => {
                self.upscale = UpscaleDraft::from_request(&self.media, request);
            }
            QueuedOperation::Remux(request) => {
                self.remux = RemuxDraft::from_request(&self.media, request);
            }
            QueuedOperation::AudioExtract(request) => {
                self.audio_extract = AudioExtractDraft::from_request(&self.media, request);
            }
            QueuedOperation::FrameCapture(request) => {
                self.frame_capture.hydrate(&self.media, request);
            }
        }
    }

    pub(crate) fn validated_request(
        &self,
        capabilities: Option<UpscaleEncoderCapabilities<'_>>,
    ) -> Option<QueuedOperation> {
        let media = &self.media;
        match self.active_operation {
            Operation::AudioMix => self
                .audio_mix
                .validated_request(media)
                .ok()
                .map(QueuedOperation::AudioMix),
            Operation::VideoUpscale => self
                .upscale
                .validated_request(media, capabilities?)?
                .ok()
                .map(QueuedOperation::Upscale),
            Operation::Remux => self
                .remux
                .validated_request(media)
                .ok()
                .map(QueuedOperation::Remux),
            Operation::AudioExtract => self
                .audio_extract
                .validated_request(media)
                .ok()
                .map(QueuedOperation::AudioExtract),
            Operation::FrameCapture => self
                .frame_capture
                .validated_request(media)
                .ok()
                .map(QueuedOperation::FrameCapture),
        }
    }
}
