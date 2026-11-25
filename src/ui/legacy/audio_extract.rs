//! AudioExtract integration for the legacy renderer. The operation domain owns the editable draft.

use super::ApolloApp;
use crate::app::dialogs::OutputDraft;
use crate::app::{Operation, queue::QueueSubmission};
use crate::audio_extract::draft::AudioExtractDraft;
use crate::media::{error::MediaError, model::MediaFile};
use crate::ui::audio_extract::AudioExtractSelectionAction;
use crate::ui::{OperationSurfaceMode, audio_extract as audio_extract_ui, layout, media_error};
use egui::Ui;

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

impl ApolloApp {
    pub(super) fn show_audio_extract_tool(&mut self, ui: &mut Ui) {
        if self.show_audio_extract_picker_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let process_now_allowed = self.core.process_now_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::AudioExtract);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::AudioExtract)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(
                self.file_intake_enabled() && self.ready_media_tools().is_some(),
                |ui| {
                    let session = self
                        .state
                        .audio_extract
                        .as_mut()
                        .expect("audio extract source exists while ready surface is shown");
                    audio_extract_ui::show(
                        ui,
                        &session.media,
                        &presentation.audio_extract,
                        &mut session.selection,
                        OperationSurfaceMode::Normal {
                            process_now_allowed,
                            queue_confirmation,
                            action_error: action_error.as_ref(),
                        },
                    )
                },
            )
            .inner;
        self.handle_audio_extract_action(action);
    }

    fn show_audio_extract_picker_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.audio_extract.as_mut() else {
            return false;
        };
        let Some(error) = session.picker_error.as_ref() else {
            return false;
        };

        let title = "Output picker unavailable";
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.picker_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.picker_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.picker_error = None;
            session.picker_error_details_open = false;
        }
        true
    }

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
