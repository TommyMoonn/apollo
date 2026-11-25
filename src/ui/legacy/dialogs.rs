//! Route native completions to their initiating application context.

use std::path::PathBuf;

use super::{ApolloApp, state::LegacyEditorState};
use crate::{
    app::{
        NavigationState, Operation,
        dialogs::{
            DialogOutcome, DialogTarget, FileDialogCompletion, FileDialogRequest,
            InputDialogOrigin, OutputDraft,
        },
        editor::OperationEditorState,
    },
    audio_extract::{draft::AudioExtractDraft, execution::AudioExtractExecutionState},
    audio_mix::draft::AudioMixDraft,
    file_dialog::FileDialogStartError,
    frame_capture::{draft::FrameCaptureDraft, execution::FrameCaptureExecutionState},
    media::error::MediaError,
    remux::draft::RemuxDraft,
    upscale::{draft::UpscaleDraft, execution::UpscaleExecutionState},
};

impl ApolloApp {
    pub(super) fn start_file_dialog(&mut self, request: FileDialogRequest) {
        let request = self.core.prepare_file_dialog_request(request);
        let target = request.target();
        match self.file_dialog.request(request) {
            Ok(()) => {
                self.input_dialog_origin = target
                    .output()
                    .is_none()
                    .then(|| InputDialogOrigin::capture(&self.core))
            }
            Err(FileDialogStartError::AlreadyPending) => {}
            Err(error) => self.settle_file_dialog(FileDialogCompletion {
                target,
                outcome: DialogOutcome::Failed(error.to_string()),
            }),
        }
    }

    pub(super) fn poll_file_dialog(&mut self) {
        let Some(completion) = self.file_dialog.poll() else {
            return;
        };
        let origin = self.input_dialog_origin.take();
        if completion.target.output().is_none()
            && !origin.is_some_and(|origin| origin.is_current(&self.core))
        {
            return;
        }
        self.settle_file_dialog(completion);
    }

    pub(super) fn settle_file_dialog(&mut self, completion: FileDialogCompletion) {
        let FileDialogCompletion { target, outcome } = completion;
        if let Some((operation, session_id)) = target.output() {
            // Independent editors have unique session IDs; settle at most one owner.
            let editor = match self.core.navigation() {
                NavigationState::QueueEdit { .. } => {
                    self.core.queue_edit_mut().map(|edit| &mut edit.editor)
                }
                NavigationState::BatchEdit => {
                    self.core.batch_edit.as_mut().map(|edit| &mut edit.editor)
                }
                _ => None,
            }
            .filter(|edit| edit.session_id == session_id);
            let selected = if let Some(editor) = editor {
                settle_editor_output(editor, operation, &outcome)
            } else {
                settle_output_file_dialog(
                    &mut self.state,
                    ToolExecutionStates {
                        upscale: self.core.upscale_executor.state(),
                        audio_extract: self.core.audio_extract_executor.state(),
                        frame_capture: self.core.frame_capture_executor.state(),
                    },
                    target,
                    &outcome,
                )
            };
            if selected && let DialogOutcome::Selected(path) = outcome {
                self.core.settings.remember_output(&path);
            }
            return;
        }
        // A batch append is valid only while the original batch setup is active.
        if target == DialogTarget::BatchAppend
            && (self.core.navigation() != NavigationState::BatchSetup
                || self.core.batch_setup.is_none())
        {
            return;
        }
        match (target, outcome) {
            (DialogTarget::MediaInput, DialogOutcome::Selected(path)) => {
                self.core.settings.remember_source(&path);
                self.load_media_path(&path);
            }
            (
                DialogTarget::BatchInput | DialogTarget::BatchAppend,
                DialogOutcome::Selected(path),
            ) => {
                self.settle_batch_paths(target, vec![path]);
            }
            (
                DialogTarget::BatchInput | DialogTarget::BatchAppend,
                DialogOutcome::SelectedMany(paths),
            ) => {
                self.settle_batch_paths(target, paths);
            }
            (DialogTarget::BatchAppend, DialogOutcome::Failed(diagnostic)) => {
                self.core
                    .set_batch_append_error(MediaError::file_picker_unavailable(diagnostic));
                self.batch_append_error_details_open = false;
            }
            (
                DialogTarget::MediaInput | DialogTarget::BatchInput,
                DialogOutcome::Failed(diagnostic),
            ) => {
                self.core
                    .source
                    .set_error(MediaError::file_picker_unavailable(diagnostic));
                self.state.intake_error_details_open = false;
            }
            _ => {}
        }
    }

    fn settle_batch_paths(&mut self, target: DialogTarget, paths: Vec<PathBuf>) {
        if let Some(path) = paths.first() {
            self.core.settings.remember_source(path);
        }
        if target == DialogTarget::BatchAppend {
            self.append_batch_paths(paths);
        } else {
            self.start_batch_paths(paths);
        }
    }
}

fn settle_editor_output(
    edit: &mut OperationEditorState,
    operation: Operation,
    outcome: &DialogOutcome,
) -> bool {
    match outcome {
        DialogOutcome::Selected(path) => {
            match operation {
                Operation::AudioMix => edit.audio_mix.set_custom_output_path(path.clone()),
                Operation::VideoUpscale => edit.upscale.set_custom_output_path(path.clone()),
                Operation::Remux => edit.remux.set_custom_output_path(path.clone()),
                Operation::AudioExtract => edit.audio_extract.set_custom_output_path(path.clone()),
                Operation::FrameCapture => edit.frame_capture.set_custom_output_path(path.clone()),
            }
            true
        }
        DialogOutcome::Failed(diagnostic) if edit.active_operation == operation => {
            edit.error = Some(MediaError::output_picker_unavailable(diagnostic.clone()));
            edit.error_details_open = false;
            false
        }
        _ => false,
    }
}

pub(super) struct ToolExecutionStates<'a> {
    pub(super) upscale: &'a UpscaleExecutionState,
    pub(super) audio_extract: &'a AudioExtractExecutionState,
    pub(super) frame_capture: &'a FrameCaptureExecutionState,
}

/// Returns true only when a selected path was applied to the original editable draft.
pub(super) fn settle_output_file_dialog(
    state: &mut LegacyEditorState,
    execution: ToolExecutionStates<'_>,
    target: DialogTarget,
    outcome: &DialogOutcome,
) -> bool {
    let Some((operation, session_id)) = target.output() else {
        return false;
    };
    match operation {
        Operation::AudioMix => {
            let Some(session) = state
                .audio_mix
                .as_mut()
                .filter(|session| session.session_id == session_id)
            else {
                return false;
            };
            settle_normal_output(
                &mut session.selection,
                &mut session.picker_error,
                &mut session.picker_error_details_open,
                outcome,
                AudioMixDraft::set_custom_output_path,
            )
        }
        Operation::VideoUpscale if matches!(execution.upscale, UpscaleExecutionState::Idle) => {
            let Some(session) = state
                .upscale
                .as_mut()
                .filter(|session| session.session_id == session_id)
            else {
                return false;
            };
            settle_normal_output(
                &mut session.selection,
                &mut session.start_error,
                &mut session.start_error_details_open,
                outcome,
                UpscaleDraft::set_custom_output_path,
            )
        }
        Operation::Remux => {
            let Some(session) = state
                .remux
                .as_mut()
                .filter(|session| session.session_id == session_id)
            else {
                return false;
            };
            settle_normal_output(
                &mut session.selection,
                &mut session.picker_error,
                &mut session.picker_error_details_open,
                outcome,
                RemuxDraft::set_custom_output_path,
            )
        }
        Operation::AudioExtract
            if matches!(execution.audio_extract, AudioExtractExecutionState::Idle) =>
        {
            let Some(session) = state
                .audio_extract
                .as_mut()
                .filter(|session| session.session_id == session_id)
            else {
                return false;
            };
            settle_normal_output(
                &mut session.selection,
                &mut session.start_error,
                &mut session.start_error_details_open,
                outcome,
                AudioExtractDraft::set_custom_output_path,
            )
        }
        Operation::FrameCapture
            if matches!(execution.frame_capture, FrameCaptureExecutionState::Idle) =>
        {
            let Some(session) = state
                .frame_capture
                .as_mut()
                .filter(|session| session.session_id == session_id)
            else {
                return false;
            };
            settle_normal_output(
                &mut session.editor.selection,
                &mut session.start_error,
                &mut session.start_error_details_open,
                outcome,
                FrameCaptureDraft::set_custom_output_path,
            )
        }
        _ => false,
    }
}

fn settle_normal_output<D>(
    draft: &mut D,
    error: &mut Option<MediaError>,
    details_open: &mut bool,
    outcome: &DialogOutcome,
    set_path: impl FnOnce(&mut D, PathBuf),
) -> bool {
    match outcome {
        DialogOutcome::Selected(path) => {
            set_path(draft, path.clone());
            *error = None;
            *details_open = false;
            true
        }
        DialogOutcome::Failed(diagnostic) => {
            *error = Some(MediaError::output_picker_unavailable(diagnostic.clone()));
            *details_open = false;
            false
        }
        _ => false,
    }
}

pub(super) fn editor_output_dialog_request(edit: &OperationEditorState) -> FileDialogRequest {
    let draft = match edit.active_operation {
        Operation::AudioMix => OutputDraft::AudioMix(&edit.audio_mix),
        Operation::VideoUpscale => OutputDraft::Upscale(&edit.upscale),
        Operation::Remux => OutputDraft::Remux(&edit.remux),
        Operation::AudioExtract => OutputDraft::AudioExtract(&edit.audio_extract),
        Operation::FrameCapture => OutputDraft::FrameCapture(&edit.frame_capture.selection),
    };
    draft.request(edit.session_id)
}
