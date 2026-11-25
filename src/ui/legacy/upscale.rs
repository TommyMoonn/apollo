//! Upscale integration for the legacy renderer. The operation domain owns the editable draft.

use super::ApolloApp;
use super::workflows::shows_upscale_lifecycle;
use crate::app::Operation;
use crate::app::dialogs::OutputDraft;
use crate::media::{
    error::{MediaError, MediaErrorKind},
    model::MediaFile,
};
use crate::native::open_folder;
use crate::ui::upscale::UpscaleSelectionAction;
use crate::ui::upscale_progress::{UpscaleLifecycleAction, UpscaleLifecycleUiState};
use crate::ui::{
    OperationSurfaceMode, layout, media_error, upscale as upscale_ui, upscale_progress,
};
use crate::upscale::{draft::UpscaleDraft, execution::UpscaleStartError};
use egui::Ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpscaleToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: UpscaleDraft,
    pub(super) lifecycle_ui: UpscaleLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

impl UpscaleToolState {
    pub(super) fn new(session_id: u64, media: &MediaFile) -> Self {
        Self {
            session_id,
            media: media.clone(),
            selection: UpscaleDraft::new(media),
            lifecycle_ui: UpscaleLifecycleUiState::default(),
            start_error: None,
            start_error_details_open: false,
        }
    }
}

impl ApolloApp {
    pub(super) fn show_upscale_tool(&mut self, ui: &mut Ui) {
        if shows_upscale_lifecycle(self.core.upscale_executor.state()) {
            let action = {
                let upscale = self
                    .state
                    .upscale
                    .as_mut()
                    .expect("upscale source exists while lifecycle surface is shown");
                let request = upscale.selection.request(&upscale.media);
                upscale_progress::show(
                    ui,
                    &upscale.media,
                    &request,
                    &mut self.core.upscale_executor,
                    &mut upscale.lifecycle_ui,
                )
            };
            self.handle_upscale_lifecycle_action(action);
            return;
        }

        if self.show_upscale_start_error(ui) {
            return;
        }

        let enabled = self.workspace_interaction_enabled();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::VideoUpscale);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::VideoUpscale)
            .map(|(_, error)| error.clone());
        let capabilities = self.core.discovery.settled_capabilities().map_or(
            crate::upscale::draft::UpscaleEncoderCapabilities::Checking,
            crate::upscale::draft::UpscaleEncoderCapabilities::Ready,
        );
        let action = ui
            .add_enabled_ui(enabled, |ui| {
                let upscale = self
                    .state
                    .upscale
                    .as_mut()
                    .expect("upscale source exists while ready surface is shown");
                upscale_ui::show(
                    ui,
                    &upscale.media,
                    &mut upscale.selection,
                    capabilities,
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_upscale_action(action)
            && let Some(session) = self.state.upscale.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    pub(super) fn show_upscale_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.upscale.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Upscale could not start"
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

    pub(super) fn handle_upscale_action(
        &mut self,
        action: UpscaleSelectionAction,
    ) -> Result<(), UpscaleStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != UpscaleSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            UpscaleSelectionAction::BrowseOutput => {
                let Some(session) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let request = OutputDraft::Upscale(&session.selection).request(session.session_id);
                self.start_file_dialog(request);
            }
            UpscaleSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::VideoUpscale);
            }
            UpscaleSelectionAction::UpscaleRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(upscale) = self.state.upscale.as_ref() else {
                    return Ok(());
                };
                let Some(capabilities) = self.core.discovery.upscale_capabilities() else {
                    return Ok(());
                };
                let Some(request) = upscale
                    .selection
                    .request_for_capabilities(&upscale.media, capabilities)
                else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let wake = self.progress_repaint.begin();
                self.core.upscale_executor.start_with_wake(
                    &tools,
                    &upscale.media,
                    &request,
                    wake,
                )?;
            }
            UpscaleSelectionAction::SaveQueueChanges
            | UpscaleSelectionAction::CancelQueueEdit
            | UpscaleSelectionAction::None => {}
        }
        Ok(())
    }

    pub(super) fn handle_upscale_lifecycle_action(&self, action: UpscaleLifecycleAction) {
        if let UpscaleLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Upscale output folder could not be opened: {error}");
        }
    }
}
