//! AudioMix integration for the legacy renderer. The operation domain owns the editable draft.

use super::ApolloApp;
use super::workflows::{audio_mix_output_dialog_request, open_folder, shows_audio_mix_lifecycle};
use crate::app::Operation;
use crate::audio_mix::{draft::AudioMixDraft, execution::AudioMixStartError};
use crate::media::{
    error::{MediaError, MediaErrorKind},
    model::MediaFile,
};
use crate::ui::audio_mix::AudioMixSelectionAction;
use crate::ui::audio_mix_progress::{AudioMixLifecycleAction, AudioMixLifecycleUiState};
use crate::ui::{
    OperationSurfaceMode, audio_mix as audio_mix_ui, audio_mix_progress, layout, media_error,
};
use egui::Ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AudioMixToolState {
    pub(super) session_id: u64,
    pub(super) media: MediaFile,
    pub(super) selection: AudioMixDraft,
    pub(super) lifecycle_ui: AudioMixLifecycleUiState,
    pub(super) start_error: Option<MediaError>,
    pub(super) start_error_details_open: bool,
}

impl AudioMixToolState {
    pub(super) fn new(session_id: u64, media: &MediaFile) -> Self {
        Self {
            session_id,
            media: media.clone(),
            selection: AudioMixDraft::new(media),
            lifecycle_ui: AudioMixLifecycleUiState::default(),
            start_error: None,
            start_error_details_open: false,
        }
    }
}

impl ApolloApp {
    pub(super) fn show_audio_mix_tool(&mut self, ui: &mut Ui) {
        if shows_audio_mix_lifecycle(self.core.audio_mix_executor.state()) {
            let action = {
                let session = self
                    .state
                    .audio_mix
                    .as_mut()
                    .expect("audio mix source exists while lifecycle surface is shown");
                let request = session.selection.request(&session.media);
                audio_mix_progress::show(
                    ui,
                    &session.media,
                    &request,
                    &mut self.core.audio_mix_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_audio_mix_lifecycle_action(action);
            return;
        }

        if self.show_audio_mix_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed = self.execution_arbitration().immediate_start_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::AudioMix);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::AudioMix)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .audio_mix
                    .as_mut()
                    .expect("audio mix source exists while ready surface is shown");
                audio_mix_ui::show(
                    ui,
                    &session.media,
                    &presentation.audio_mix,
                    &mut session.selection,
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        if let Err(error) = self.handle_audio_mix_action(action)
            && let Some(session) = self.state.audio_mix.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_audio_mix_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.audio_mix.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Audio mix could not start"
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

    pub(super) fn handle_audio_mix_action(
        &mut self,
        action: AudioMixSelectionAction,
    ) -> Result<(), AudioMixStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != AudioMixSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            AudioMixSelectionAction::BrowseOutput => {
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let request = audio_mix_output_dialog_request(session);
                self.start_file_dialog(request);
            }
            AudioMixSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::AudioMix);
            }
            AudioMixSelectionAction::MixRequested => {
                if !self.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(session) = self.state.audio_mix.as_ref() else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let request = session.selection.request(&session.media);
                let wake = self.progress_repaint.begin();
                self.core.audio_mix_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
            }
            AudioMixSelectionAction::SaveQueueChanges
            | AudioMixSelectionAction::CancelQueueEdit
            | AudioMixSelectionAction::None => {}
        }
        Ok(())
    }

    pub(super) fn handle_audio_mix_lifecycle_action(&self, action: AudioMixLifecycleAction) {
        if let AudioMixLifecycleAction::OpenFolder(path) = action
            && let Err(error) = open_folder(&path)
        {
            eprintln!("Audio Mix output folder could not be opened: {error}");
        }
    }
}
