//! Remux integration for the legacy renderer. The operation domain owns the editable draft.

use super::ApolloApp;
use super::workflows::{open_folder, remux_output_dialog_request, shows_remux_lifecycle};
use crate::app::Operation;
use crate::media::{
    error::{MediaError, MediaErrorKind},
    model::MediaFile,
};
use crate::remux::{draft::RemuxDraft, execution::RemuxStartError};
use crate::ui::remux::RemuxSelectionAction;
use crate::ui::remux_progress::{RemuxLifecycleAction, RemuxLifecycleUiState};
use crate::ui::{OperationSurfaceMode, layout, media_error, remux as remux_ui, remux_progress};
use egui::Ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemuxToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: RemuxDraft,
    pub(super) lifecycle_ui: RemuxLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

impl RemuxToolState {
    pub(super) fn new(session_id: u64, media: &MediaFile) -> Self {
        Self {
            session_id,
            media: media.clone(),
            selection: RemuxDraft::new(media),
            lifecycle_ui: RemuxLifecycleUiState::default(),
            start_error: None,
            start_error_details_open: false,
        }
    }
}

impl ApolloApp {
    pub(super) fn show_remux_tool(&mut self, ui: &mut Ui) {
        if shows_remux_lifecycle(self.core.remux_executor.state()) {
            let action = {
                let remux = self
                    .state
                    .remux
                    .as_mut()
                    .expect("remux source exists while lifecycle surface is shown");
                let request = remux.selection.request(&remux.media);
                remux_progress::show(
                    ui,
                    &request,
                    &mut self.core.remux_executor,
                    &mut remux.lifecycle_ui,
                )
            };
            self.handle_remux_lifecycle_action(action);
            return;
        }

        if self.show_remux_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::Remux);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::Remux)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
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
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_remux_action(action)
            && let Some(session) = self.state.remux.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_remux_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.remux.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Remux could not start"
        };
        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.start_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.start_error_details_open
            };
            media_error::show_failure_content(ui, title, error, details_open, "Back")
        });
        if response.inner.clicked() {
            session.start_error = None;
            session.start_error_details_open = false;
        }
        true
    }

    pub(super) fn handle_remux_action(
        &mut self,
        action: RemuxSelectionAction,
    ) -> Result<(), RemuxStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != RemuxSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            RemuxSelectionAction::BrowseOutput => {
                let Some(session) = self.state.remux.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(remux_output_dialog_request(session));
            }
            RemuxSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::Remux);
            }
            RemuxSelectionAction::RemuxRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(remux) = self.state.remux.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = remux.selection.request(&remux.media);
                let wake = self.progress_repaint.begin();
                self.core
                    .remux_executor
                    .start_with_wake(&tools, &remux.media, &request, wake)?;
            }
            RemuxSelectionAction::SaveQueueChanges
            | RemuxSelectionAction::CancelQueueEdit
            | RemuxSelectionAction::None => {}
        }
        Ok(())
    }

    pub(super) fn handle_remux_lifecycle_action(&self, action: RemuxLifecycleAction) {
        if let RemuxLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Remux output folder could not be opened: {error}");
        }
    }
}
