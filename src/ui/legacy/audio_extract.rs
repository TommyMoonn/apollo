//! AudioExtract integration for the legacy renderer. The operation domain owns the editable draft.

#[cfg(test)]
use super::ApolloApp;
#[cfg(test)]
use crate::app::dialogs::OutputDraft;
#[cfg(test)]
use crate::app::{Operation, queue::QueueSubmission};
use crate::audio_extract::draft::AudioExtractDraft;
use crate::media::{error::MediaError, model::MediaFile};
#[cfg(test)]
use crate::ui::audio_extract::AudioExtractSelectionAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AudioExtractToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: AudioExtractDraft,
    pub(super) picker_error: Option<MediaError>,
    pub(super) picker_error_details_open: bool,
}

impl AudioExtractToolState {
    pub(super) fn new(session_id: u64, media: &MediaFile) -> Self {
        Self {
            session_id,
            media: media.clone(),
            selection: AudioExtractDraft::new(media),
            picker_error: None,
            picker_error_details_open: false,
        }
    }
}

#[cfg(test)]
impl ApolloApp {
    pub(super) fn handle_audio_extract_action(&mut self, action: AudioExtractSelectionAction) {
        match action {
            AudioExtractSelectionAction::BrowseOutput => {
                if !self.file_intake_enabled()
                    || self.core.active_operation() != Some(Operation::AudioExtract)
                    || self.ready_media_tools().is_none()
                {
                    return;
                }
                self.core.queue_confirmation = None;
                self.core.operation_action_error = None;
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return;
                };
                let request =
                    OutputDraft::AudioExtract(&session.selection).request(session.session_id);
                self.start_file_dialog(request);
            }
            AudioExtractSelectionAction::QueueRequested => {
                self.submit_queued_operation(Operation::AudioExtract, QueueSubmission::AddToQueue);
            }
            AudioExtractSelectionAction::ExtractRequested => {
                self.submit_queued_operation(Operation::AudioExtract, QueueSubmission::ProcessNow);
            }
            AudioExtractSelectionAction::SaveQueueChanges
            | AudioExtractSelectionAction::CancelQueueEdit
            | AudioExtractSelectionAction::None => {}
        }
    }
}
