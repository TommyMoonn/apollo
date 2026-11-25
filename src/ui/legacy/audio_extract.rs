//! AudioExtract integration for the legacy renderer. The operation domain owns the editable draft.

use super::ApolloApp;
use super::workflows::{
    audio_extract_output_dialog_request, open_folder, shows_audio_extract_lifecycle,
};
use crate::app::Operation;
use crate::audio_extract::{draft::AudioExtractDraft, execution::AudioExtractStartError};
use crate::media::{
    error::{MediaError, MediaErrorKind},
    model::MediaFile,
};
use crate::ui::audio_extract::AudioExtractSelectionAction;
use crate::ui::audio_extract_progress::{
    AudioExtractLifecycleAction, AudioExtractLifecycleUiState,
};
use crate::ui::{
    OperationSurfaceMode, audio_extract as audio_extract_ui, audio_extract_progress, layout,
    media_error,
};
use egui::Ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AudioExtractToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: AudioExtractDraft,
    pub(super) lifecycle_ui: AudioExtractLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

impl AudioExtractToolState {
    pub(super) fn new(session_id: u64, media: &MediaFile) -> Self {
        Self {
            session_id,
            media: media.clone(),
            selection: AudioExtractDraft::new(media),
            lifecycle_ui: AudioExtractLifecycleUiState::default(),
            start_error: None,
            start_error_details_open: false,
        }
    }
}

impl ApolloApp {
    pub(super) fn show_audio_extract_tool(&mut self, ui: &mut Ui) {
        if shows_audio_extract_lifecycle(self.core.audio_extract_executor.state()) {
            let action = {
                let session = self
                    .state
                    .audio_extract
                    .as_mut()
                    .expect("audio extract source exists while lifecycle surface is shown");
                let request = session
                    .selection
                    .request(&session.media)
                    .expect("running audio extract session owns a selected audio stream");
                audio_extract_progress::show(
                    ui,
                    &request,
                    &mut self.core.audio_extract_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_audio_extract_lifecycle_action(action);
            return;
        }

        if self.show_audio_extract_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::AudioExtract);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::AudioExtract)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
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
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_audio_extract_action(action)
            && let Some(session) = self.state.audio_extract.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_audio_extract_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.audio_extract.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Audio extraction could not start"
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

    pub(super) fn handle_audio_extract_action(
        &mut self,
        action: AudioExtractSelectionAction,
    ) -> Result<(), AudioExtractStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != AudioExtractSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            AudioExtractSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(audio_extract_output_dialog_request(session));
            }
            AudioExtractSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::AudioExtract);
            }
            AudioExtractSelectionAction::ExtractRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(session) = self.state.audio_extract.as_ref() else {
                    return Ok(());
                };
                let Some(request) = session.selection.request(&session.media) else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let wake = self.progress_repaint.begin();
                self.core.audio_extract_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
            }
            AudioExtractSelectionAction::SaveQueueChanges
            | AudioExtractSelectionAction::CancelQueueEdit
            | AudioExtractSelectionAction::None => {}
        }
        Ok(())
    }

    pub(super) fn handle_audio_extract_lifecycle_action(
        &self,
        action: AudioExtractLifecycleAction,
    ) {
        if let AudioExtractLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Extract output folder could not be opened: {error}");
        }
    }
}
