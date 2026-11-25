//! Batch presentation and native effects.
use super::{
    ApolloApp, completion_wake,
    dialogs::editor_output_dialog_request,
    render::show_operation_editor,
    state::{EditorPresentation, QueueEditAction},
};
use crate::{
    app::{
        batch::{BatchAction, BatchEditTarget, BatchSourceState, batch_defaults_from_editor},
        dialogs::{DialogTarget, FileDialogRequest},
    },
    queue::QueuedOperation,
    ui::{
        OperationSurfaceMode,
        batch::{self as batch_ui, BatchItemView},
        layout, media_error, theme,
        workspace::{self, WorkspaceMode},
    },
};
use egui::Ui;
use std::{path::PathBuf, time::Instant};

impl ApolloApp {
    pub(super) fn start_batch_paths(&mut self, paths: Vec<PathBuf>) -> bool {
        if paths.is_empty() || !self.file_intake_enabled() {
            return false;
        }
        if paths.len() == 1 {
            self.load_media_path(&paths[0]);
            return true;
        }
        let previous = self.core.navigation();
        if !self
            .core
            .start_batch_paths(paths, completion_wake(&self.egui_context))
        {
            return false;
        }
        self.batch_editor_presentation = None;
        self.batch_append_error_details_open = false;
        self.suspend_frame_capture_support_from(previous);
        true
    }

    pub(super) fn append_batch_paths(&mut self, paths: Vec<PathBuf>) -> bool {
        let appended = self
            .core
            .append_batch_paths(paths, completion_wake(&self.egui_context));
        if self
            .core
            .batch_setup
            .as_ref()
            .is_some_and(|batch| batch.append_error.is_none())
        {
            self.batch_append_error_details_open = false;
        }
        appended
    }

    pub(super) fn begin_batch_edit(&mut self, target: BatchEditTarget) -> bool {
        if !self.core.begin_batch_edit(target) {
            return false;
        }
        self.batch_editor_presentation = self
            .core
            .batch_edit
            .as_ref()
            .map(|edit| EditorPresentation::new(&edit.media));
        self.schedule_active_frame_capture_support();
        true
    }

    pub(super) fn finish_batch_edit(&mut self) -> bool {
        if !self.core.finish_batch_edit() {
            return false;
        }
        self.cancel_frame_capture_support();
        self.batch_editor_presentation = None;
        true
    }

    pub(super) fn save_batch_edit(&mut self) -> bool {
        if !self.core.save_batch_edit() {
            return false;
        }
        self.cancel_frame_capture_support();
        self.batch_editor_presentation = None;
        true
    }

    pub(super) fn finish_batch_setup(&mut self) -> bool {
        if !self.core.finish_batch_setup() {
            return false;
        }
        self.batch_editor_presentation = None;
        self.schedule_active_frame_capture_support();
        true
    }

    pub(super) fn enqueue_batch(&mut self) -> bool {
        if !self.core.enqueue_batch() {
            return false;
        }
        self.batch_editor_presentation = None;
        self.schedule_active_frame_capture_support();
        true
    }

    pub(super) fn show_batch_setup(&mut self, ui: &mut Ui) {
        let Some(availability) = self.core.batch_availability() else {
            return;
        };
        let Some(batch) = self.core.batch_setup.as_ref() else {
            return;
        };
        let views = batch
            .items
            .iter()
            .map(|item| BatchItemView {
                id: item.id,
                source: &item.path,
                output: item.operation.as_ref().map(QueuedOperation::output),
                readiness: item.readiness,
                reason: item.reason.as_deref(),
                included: item.included,
                can_edit: item.can_edit(),
                overridden: item.overridden,
                inspection_failed: matches!(item.source, BatchSourceState::Failed),
                can_toggle_included: item.can_toggle_included(),
            })
            .collect::<Vec<_>>();
        let queue_enabled = self.file_intake_enabled();
        let action = ui
            .add_enabled_ui(!self.file_dialog.is_pending(), |ui| {
                batch_ui::show(
                    ui,
                    batch.operation,
                    &views,
                    availability,
                    self.core.queue.scheduler().pending_count(),
                    queue_enabled,
                    batch_ui::BatchFeedback {
                        status: batch.error.as_deref(),
                        append_error: batch.append_error.as_ref(),
                        append_error_details_open: &mut self.batch_append_error_details_open,
                    },
                )
            })
            .inner;
        match action {
            BatchAction::OpenQueue => self.enter_queue(),
            BatchAction::SelectOperation(operation) => self.core.select_batch_operation(operation),
            BatchAction::ConfigureDefaults => {
                self.begin_batch_edit(BatchEditTarget::Defaults);
            }
            BatchAction::AddFiles => {
                self.start_file_dialog(FileDialogRequest::OpenSources {
                    initial_directory: None,
                    target: DialogTarget::BatchAppend,
                });
            }
            BatchAction::EditItem(index) => {
                self.begin_batch_edit(BatchEditTarget::Item(index));
            }
            BatchAction::ToggleIncluded(id) => self.core.toggle_batch_item(id),
            BatchAction::Enqueue => {
                self.enqueue_batch();
            }
            BatchAction::Back => {
                self.finish_batch_setup();
            }
            BatchAction::None => {}
        }
    }

    pub(super) fn show_batch_edit(&mut self, ui: &mut Ui) {
        let Some(edit) = self.core.batch_edit.as_ref() else {
            return;
        };
        let target = edit.target;
        let Some(presentation) = self.batch_editor_presentation.as_ref() else {
            return;
        };
        let presentation = &presentation.source;
        let operation = edit.active_operation;
        let enabled = !self.file_dialog.is_pending();
        layout::workspace_chrome(ui, |ui| {
            workspace::show(
                ui,
                &presentation.workspace,
                operation,
                WorkspaceMode::FixedSourceEdit {
                    context: match target {
                        BatchEditTarget::Defaults => "Edit shared settings",
                        BatchEditTarget::Item(_) => "Edit settings",
                    },
                    enabled,
                    operation_switching: false,
                },
            )
        });
        let capabilities = self.core.discovery.settled_capabilities().map_or(
            crate::upscale::draft::UpscaleEncoderCapabilities::Checking,
            crate::upscale::draft::UpscaleEncoderCapabilities::Ready,
        );
        let Some(edit) = self.core.batch_edit.as_mut() else {
            return;
        };
        if let Some(error) = edit.error.clone() {
            layout::content_gutter(ui, |ui| {
                media_error::show_inline(ui, &error, &mut edit.error_details_open);
                ui.add_space(theme::SPACING.related);
            });
        }
        let defaults_save_allowed = batch_defaults_from_editor(&edit.editor).is_some();
        let action = show_operation_editor(
            ui,
            &mut edit.editor,
            self.batch_editor_presentation
                .as_ref()
                .expect("active editor presentation"),
            capabilities,
            match target {
                BatchEditTarget::Defaults => OperationSurfaceMode::BatchDefaults {
                    save_allowed: defaults_save_allowed,
                },
                BatchEditTarget::Item(_) => OperationSurfaceMode::BatchOverride,
            },
            enabled,
            match target {
                BatchEditTarget::Defaults => ("batch-defaults", 0),
                BatchEditTarget::Item(index) => ("batch-item", index.0),
            },
        );
        match action {
            QueueEditAction::BrowseOutput => {
                if let Some(edit) = self.core.batch_edit.as_ref() {
                    self.start_file_dialog(editor_output_dialog_request(&edit.editor));
                }
            }
            QueueEditAction::Save => {
                self.save_batch_edit();
            }
            QueueEditAction::Cancel => {
                self.finish_batch_edit();
            }
            QueueEditAction::FramePositionEdited(settle) => {
                let position = self
                    .core
                    .batch_edit
                    .as_ref()
                    .and_then(|edit| edit.frame_capture.valid_position(&edit.media));
                self.update_frame_capture_position(position, settle, Instant::now());
            }
            QueueEditAction::FrameStepRequested(direction) => {
                self.request_frame_step(direction);
            }
            QueueEditAction::None => {}
        }
    }
}
