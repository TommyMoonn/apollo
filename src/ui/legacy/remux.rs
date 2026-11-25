//! Remux integration for the legacy renderer. The operation domain owns the editable draft.

use super::ApolloApp;
use crate::app::dialogs::OutputDraft;
use crate::app::{Operation, queue::QueueSubmission};
use crate::media::{error::MediaError, model::MediaFile};
use crate::remux::draft::RemuxDraft;
use crate::ui::remux::RemuxSelectionAction;
use crate::ui::{OperationSurfaceMode, layout, media_error, remux as remux_ui};
use egui::Ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemuxToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: RemuxDraft,
    pub(super) picker_error: Option<MediaError>,
    pub(super) picker_error_details_open: bool,
}

impl RemuxToolState {
    pub(super) fn new(session_id: u64, media: &MediaFile) -> Self {
        Self {
            session_id,
            media: media.clone(),
            selection: RemuxDraft::new(media),
            picker_error: None,
            picker_error_details_open: false,
        }
    }
}

impl ApolloApp {
    pub(super) fn show_remux_tool(&mut self, ui: &mut Ui) {
        if self.show_remux_picker_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let process_now_allowed = self.core.process_now_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::Remux);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::Remux)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(
                self.file_intake_enabled() && self.ready_media_tools().is_some(),
                |ui| {
                    let remux = self
                        .state
                        .remux
                        .as_mut()
                        .expect("remux source exists while ready surface is shown");
                    remux_ui::show(
                        ui,
                        &remux.media,
                        &presentation.remux,
                        &mut remux.selection,
                        OperationSurfaceMode::Immediate {
                            start_allowed: process_now_allowed,
                            queue_confirmation,
                            action_error: action_error.as_ref(),
                        },
                    )
                },
            )
            .inner;
        self.handle_remux_action(action);
    }

    fn show_remux_picker_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.remux.as_mut() else {
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

    pub(super) fn handle_remux_action(&mut self, action: RemuxSelectionAction) {
        match action {
            RemuxSelectionAction::BrowseOutput => {
                if !self.file_intake_enabled()
                    || self.core.active_operation() != Some(Operation::Remux)
                    || self.ready_media_tools().is_none()
                {
                    return;
                }
                self.core.queue_confirmation = None;
                self.core.operation_action_error = None;
                let Some(session) = self.state.remux.as_ref() else {
                    return;
                };
                self.start_file_dialog(
                    OutputDraft::Remux(&session.selection).request(session.session_id),
                );
            }
            RemuxSelectionAction::QueueRequested => {
                self.submit_queued_operation(Operation::Remux, QueueSubmission::AddToQueue);
            }
            RemuxSelectionAction::RemuxRequested => {
                self.submit_queued_operation(Operation::Remux, QueueSubmission::ProcessNow);
            }
            RemuxSelectionAction::SaveQueueChanges
            | RemuxSelectionAction::CancelQueueEdit
            | RemuxSelectionAction::None => {}
        }
    }
}
