//! Upscale integration for the legacy renderer. The operation domain owns the editable draft.

use super::ApolloApp;
use crate::app::dialogs::OutputDraft;
use crate::app::{Operation, queue::QueueSubmission};
use crate::media::{error::MediaError, model::MediaFile};
use crate::ui::upscale::UpscaleSelectionAction;
use crate::ui::{OperationSurfaceMode, layout, media_error, upscale as upscale_ui};
use crate::upscale::draft::{UpscaleDraft, UpscaleEncoderCapabilities};
use egui::Ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpscaleToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: UpscaleDraft,
    pub(super) picker_error: Option<MediaError>,
    pub(super) picker_error_details_open: bool,
}

impl UpscaleToolState {
    pub(super) fn new(session_id: u64, media: &MediaFile) -> Self {
        Self {
            session_id,
            media: media.clone(),
            selection: UpscaleDraft::new(media),
            picker_error: None,
            picker_error_details_open: false,
        }
    }
}

impl ApolloApp {
    pub(super) fn show_upscale_tool(&mut self, ui: &mut Ui) {
        if self.show_upscale_picker_error(ui) {
            return;
        }

        let capabilities = self.core.discovery.settled_capabilities().map_or(
            UpscaleEncoderCapabilities::Checking,
            UpscaleEncoderCapabilities::Ready,
        );
        let process_now_allowed = self.core.process_now_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::VideoUpscale);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::VideoUpscale)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(
                self.file_intake_enabled() && self.ready_media_tools().is_some(),
                |ui| {
                    let session = self
                        .state
                        .upscale
                        .as_mut()
                        .expect("upscale source exists while ready surface is shown");
                    upscale_ui::show(
                        ui,
                        &session.media,
                        &mut session.selection,
                        capabilities,
                        OperationSurfaceMode::Normal {
                            process_now_allowed,
                            queue_confirmation,
                            action_error: action_error.as_ref(),
                        },
                    )
                },
            )
            .inner;
        self.handle_upscale_action(action);
    }

    pub(super) fn show_upscale_picker_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.upscale.as_mut() else {
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

    pub(super) fn handle_upscale_action(&mut self, action: UpscaleSelectionAction) {
        match action {
            UpscaleSelectionAction::BrowseOutput => {
                if !self.file_intake_enabled()
                    || self.core.active_operation() != Some(Operation::VideoUpscale)
                    || self.ready_media_tools().is_none()
                {
                    return;
                }
                self.core.queue_confirmation = None;
                self.core.operation_action_error = None;
                let Some(session) = self.state.upscale.as_ref() else {
                    return;
                };
                let request = OutputDraft::Upscale(&session.selection).request(session.session_id);
                self.start_file_dialog(request);
            }
            UpscaleSelectionAction::QueueRequested => {
                self.submit_queued_operation(Operation::VideoUpscale, QueueSubmission::AddToQueue);
            }
            UpscaleSelectionAction::UpscaleRequested => {
                self.submit_queued_operation(Operation::VideoUpscale, QueueSubmission::ProcessNow);
            }
            UpscaleSelectionAction::SaveQueueChanges
            | UpscaleSelectionAction::CancelQueueEdit
            | UpscaleSelectionAction::None => {}
        }
    }
}
