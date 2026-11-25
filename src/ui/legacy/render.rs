//! Root surface dispatch and egui rendering.

use super::dialogs::editor_output_dialog_request;
use std::time::Instant;

use egui::{CentralPanel, Ui};

use crate::app::dialogs::{DialogTarget, FileDialogRequest};
use crate::app::{NavigationState, Operation, operation_workspace_visible};
use crate::frame_capture::clipboard::copy_frame_action_eligible;
use crate::media::error::{MediaError, MediaErrorKind};
use crate::queue::{QueueRunState, QueuedOperation};
use crate::ui::audio_extract::AudioExtractSelectionAction;
use crate::ui::audio_mix::AudioMixSelectionAction;
use crate::ui::batch::{BatchAction, BatchItemEditAction, BatchItemView, BatchReadiness};
use crate::ui::frame_capture::{
    FrameCapturePositionSurface, FrameCaptureSelectionAction, FrameCopyActionState,
};
use crate::ui::remux::RemuxSelectionAction;
use crate::ui::upscale::UpscaleSelectionAction;
use crate::ui::workspace::{EditableQueuedJob, WorkspaceAction, WorkspaceMode};
use crate::ui::{
    OperationSurfaceMode, audio_extract as audio_extract_ui, audio_mix as audio_mix_ui,
    batch as batch_ui, file_intake, frame_capture as frame_capture_ui, frame_capture_progress,
    layout, media_error, queue as queue_ui, remux as remux_ui, theme, upscale as upscale_ui,
    widgets, workspace,
};

use super::ApolloApp;
use super::state::{
    BatchEditTarget, BatchSourceState, FrameCaptureToolState, OperationEditorState,
    QueueEditAction, frame_navigation_availability,
};
use super::workflows::{
    batch_defaults_from_editor, refresh_batch_items, shows_frame_capture_lifecycle,
};
use crate::app::discovery::MediaToolsState;

impl ApolloApp {
    pub(super) fn show_root(&mut self, ui: &mut Ui) {
        CentralPanel::default()
            .frame(layout::root_frame(ui.style()))
            .show(ui, |ui| {
                self.poll_background_work();
                match self.core.discovery.state() {
                    MediaToolsState::Checking => {
                        layout::padded_content(ui, |ui| self.show_media_tools_checking(ui));
                        return;
                    }
                    MediaToolsState::Unavailable(_) => {
                        layout::padded_content(ui, |ui| self.show_media_tools_unavailable(ui));
                        return;
                    }
                    MediaToolsState::Ready(_) => {}
                }
                self.handle_window_file_drop(ui.ctx());
                if self.drag_hovered(ui.ctx()) {
                    file_intake::show_drag_hover(ui);
                }
                self.show_navigation(ui);
            });
    }

    fn show_media_tools_unavailable(&mut self, ui: &mut Ui) {
        let error = match self.core.discovery.state() {
            MediaToolsState::Unavailable(error) => MediaError::from_tools_error(error),
            MediaToolsState::Checking | MediaToolsState::Ready(_) => {
                MediaError::invalid_request("media tools are unavailable")
            }
        };

        layout::centered_content(ui, |ui| {
            ui.heading("Media tools unavailable");
            ui.add_space(theme::SPACING.compact);
            media_error::show_inline(ui, &error, &mut self.tools_error_details_open);
            ui.add_space(theme::SPACING.related);
            if widgets::secondary_button(ui, "Retry").clicked() {
                self.retry_media_tools();
            }
        });
    }

    fn show_media_tools_checking(&self, ui: &mut Ui) {
        layout::centered_content(ui, |ui| {
            ui.label("Checking media tools…");
        });
    }

    fn operation_ready_surface(&self, active_operation: Operation) -> bool {
        match active_operation {
            Operation::AudioMix => operation_workspace_visible(
                self.core.audio_mix_executor.state(),
                self.state
                    .audio_mix
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Operation::VideoUpscale => operation_workspace_visible(
                self.core.upscale_executor.state(),
                self.state
                    .upscale
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Operation::Remux => operation_workspace_visible(
                self.core.remux_executor.state(),
                self.state
                    .remux
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Operation::AudioExtract => operation_workspace_visible(
                self.core.audio_extract_executor.state(),
                self.state
                    .audio_extract
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
            Operation::FrameCapture => operation_workspace_visible(
                self.core.frame_capture_executor.state(),
                self.state
                    .frame_capture
                    .as_ref()
                    .is_some_and(|session| session.start_error.is_none()),
            ),
        }
    }

    pub(super) fn show_navigation(&mut self, ui: &mut Ui) {
        let Some(route) = self.resolve_navigation() else {
            return;
        };
        let active_operation = match route {
            NavigationState::BatchEdit => {
                self.show_batch_edit(ui);
                return;
            }
            NavigationState::BatchSetup => {
                self.show_batch_setup(ui);
                return;
            }
            NavigationState::QueueEdit { .. } => {
                self.show_queue_edit(ui);
                return;
            }
            NavigationState::Queue { .. } => {
                layout::surface_fade(ui, "queue-surface".into(), |ui| {
                    ui.push_id("queue-list", |ui| self.show_queue(ui));
                });
                return;
            }
            NavigationState::NoSource => {
                layout::padded_content(ui, |ui| self.show_file_intake(ui));
                return;
            }
            NavigationState::SourceWorkspace { active_operation } => active_operation,
        };

        let Some(media) = self.core.source.current() else {
            return;
        };
        let source_surface_key = egui::Id::new(media.path.as_path());
        let Some(source_presentation) = self.state.source_presentation.clone() else {
            return;
        };

        if self.operation_ready_surface(active_operation) {
            let editable_jobs = self
                .core
                .media_queue
                .waiting_jobs()
                .map(|job| EditableQueuedJob {
                    id: job.id(),
                    source: &job.media().path,
                    operation: job.operation().display_name(),
                    output: job.operation().output(),
                })
                .collect::<Vec<_>>();
            let workspace_action = layout::workspace_chrome(ui, |ui| {
                workspace::show(
                    ui,
                    &source_presentation.workspace,
                    active_operation,
                    WorkspaceMode::Source {
                        pending_queue_count: self.core.media_queue.pending_count(),
                        enabled: !self.core.source.is_inspecting() && self.core.executors_idle(),
                        editable_jobs: &editable_jobs,
                        queue_editing_allowed: self.core.media_queue.run_state()
                            == QueueRunState::Stopped,
                    },
                )
            })
            .inner;
            self.handle_workspace_action(workspace_action);
        }

        let Some(active_operation) = self.core.active_operation() else {
            layout::padded_content(ui, |ui| self.show_file_intake(ui));
            return;
        };
        let surface_key = egui::Id::new((active_operation.label(), source_surface_key));
        let ready_surface = self.operation_ready_surface(active_operation);
        if ready_surface && let Some(error) = self.core.source.error() {
            layout::content_gutter(ui, |ui| {
                media_error::show_inline(ui, error, &mut self.state.intake_error_details_open);
                ui.add_space(theme::SPACING.related);
            });
        }
        let show_operation = |app: &mut Self, ui: &mut Ui| match active_operation {
            Operation::AudioMix if app.state.audio_mix.is_some() => app.show_audio_mix_tool(ui),
            Operation::VideoUpscale if app.state.upscale.is_some() => app.show_upscale_tool(ui),
            Operation::Remux if app.state.remux.is_some() => app.show_remux_tool(ui),
            Operation::AudioExtract if app.state.audio_extract.is_some() => {
                app.show_audio_extract_tool(ui)
            }
            Operation::FrameCapture if app.state.frame_capture.is_some() => {
                app.show_frame_capture_tool(ui)
            }
            _ => {
                layout::padded_content(ui, |ui| app.show_file_intake(ui));
            }
        };
        layout::surface_fade(ui, surface_key, |ui| {
            if ready_surface {
                show_operation(self, ui);
            } else {
                layout::padded_content(ui, |ui| show_operation(self, ui));
            }
        });
    }

    fn show_file_intake(&mut self, ui: &mut Ui) {
        let action = file_intake::show(
            ui,
            self.core.source.is_inspecting(),
            self.core.media_queue.pending_count(),
            self.core.source.error(),
            &mut self.state.intake_error_details_open,
        );
        self.handle_file_intake_action(action);
    }

    pub(super) fn show_queue(&mut self, ui: &mut Ui) {
        let arbitration = self.execution_arbitration();
        let action = queue_ui::show(
            ui,
            self.core.media_queue.jobs(),
            self.core.media_queue.active_job(),
            self.core.media_queue.active_progress(),
            self.core.media_queue.run_state(),
            arbitration.queue_start_allowed(),
        );
        self.handle_queue_action(action);
    }

    pub(super) fn show_batch_setup(&mut self, ui: &mut Ui) {
        let Some(batch) = self.batch_setup.as_mut() else {
            return;
        };
        let views = batch
            .items
            .iter()
            .map(|item| BatchItemView {
                source: &item.path,
                output: item.operation.as_ref().map(QueuedOperation::output),
                readiness: item.readiness,
                reason: item.reason.as_deref(),
                included: item.included,
                edit_action: match (&item.source, item.readiness) {
                    (BatchSourceState::Ready { .. }, BatchReadiness::Ready) => {
                        Some(BatchItemEditAction::Override)
                    }
                    (BatchSourceState::Ready { .. }, BatchReadiness::NeedsAttention) => {
                        Some(BatchItemEditAction::Review)
                    }
                    _ => None,
                },
                can_toggle_included: matches!(item.source, BatchSourceState::Ready { .. }),
            })
            .collect::<Vec<_>>();
        let included = batch.items.iter().filter(|item| item.included).count();
        let can_enqueue =
            included > 0
                && batch.items.iter().filter(|item| item.included).all(|item| {
                    item.readiness == BatchReadiness::Ready && item.operation.is_some()
                });
        let action = batch_ui::show(
            ui,
            batch.operation,
            &views,
            batch
                .items
                .iter()
                .any(|item| matches!(item.source, BatchSourceState::Ready { .. })),
            can_enqueue,
            self.batch_inspection.is_none(),
            batch_ui::BatchFeedback {
                status: batch.error.as_deref(),
                append_error: batch.append_error.as_ref(),
                append_error_details_open: &mut batch.append_error_details_open,
            },
        );
        match action {
            BatchAction::SelectOperation(operation) => {
                let capabilities = self.core.discovery.upscale_capabilities();
                if let Some(batch) = self.batch_setup.as_mut()
                    && batch.operation != operation
                {
                    batch.operation = operation;
                    batch.defaults = None;
                    batch.error = None;
                    for item in &mut batch.items {
                        item.overridden = false;
                    }
                    if let Some(media) = batch.items.iter().find_map(|item| match &item.source {
                        BatchSourceState::Ready { media, .. } => Some(media.clone()),
                        BatchSourceState::Inspecting | BatchSourceState::Failed => None,
                    }) {
                        let editor = OperationEditorState::new(0, None, media, operation);
                        batch.defaults = batch_defaults_from_editor(&editor);
                    }
                    refresh_batch_items(batch, capabilities);
                }
            }
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
            BatchAction::ToggleIncluded(index) => {
                if let Some(item) = self
                    .batch_setup
                    .as_mut()
                    .and_then(|batch| batch.items.get_mut(index))
                {
                    item.included = !item.included;
                }
            }
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
        let Some(edit) = self.batch_edit.as_ref() else {
            return;
        };
        let target = edit.target;
        let presentation = edit.presentation.clone();
        let operation = edit.active_operation;
        let enabled = !self.file_dialog.is_pending();
        layout::workspace_chrome(ui, |ui| {
            workspace::show(
                ui,
                &presentation.workspace,
                operation,
                WorkspaceMode::FixedSourceEdit {
                    context: match target {
                        BatchEditTarget::Defaults => "Configuring batch defaults",
                        BatchEditTarget::Item(_) => "Overriding batch item",
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
        let Some(edit) = self.batch_edit.as_mut() else {
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
                BatchEditTarget::Item(index) => ("batch-item", index),
            },
        );
        match action {
            QueueEditAction::BrowseOutput => {
                if let Some(edit) = self.batch_edit.as_ref() {
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

    fn show_queue_edit(&mut self, ui: &mut Ui) {
        let Some(edit) = self.queue_edit.as_ref() else {
            return;
        };
        let presentation = edit.presentation.clone();
        let active_operation = edit.active_operation;
        let enabled = !self.file_dialog.is_pending();
        let workspace_action = layout::workspace_chrome(ui, |ui| {
            workspace::show(
                ui,
                &presentation.workspace,
                active_operation,
                WorkspaceMode::FixedSourceEdit {
                    context: "Editing queued item",
                    enabled,
                    operation_switching: true,
                },
            )
        })
        .inner;
        if let WorkspaceAction::SelectOperation(operation) = workspace_action {
            self.select_queue_edit_tool(operation);
        }

        let Some(edit) = self.queue_edit.as_mut() else {
            return;
        };
        let job_id = edit.job_id;
        let edit = &mut edit.editor;
        if let Some(error) = edit.error.clone() {
            layout::content_gutter(ui, |ui| {
                media_error::show_inline(ui, &error, &mut edit.error_details_open);
                ui.add_space(theme::SPACING.related);
            });
        }
        let capabilities = self.core.discovery.settled_capabilities().map_or(
            crate::upscale::draft::UpscaleEncoderCapabilities::Checking,
            crate::upscale::draft::UpscaleEncoderCapabilities::Ready,
        );
        let action = show_operation_editor(
            ui,
            edit,
            capabilities,
            OperationSurfaceMode::QueueEdit,
            enabled,
            ("queue-editor", job_id.get()),
        );
        match action {
            QueueEditAction::BrowseOutput => {
                if let Some(edit) = self.queue_edit.as_ref() {
                    self.start_file_dialog(editor_output_dialog_request(&edit.editor));
                }
            }
            QueueEditAction::Save => {
                self.save_queue_edit();
            }
            QueueEditAction::Cancel => {
                self.finish_queue_edit();
            }
            QueueEditAction::FramePositionEdited(settle) => {
                let position = self
                    .queue_edit
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

    fn show_frame_capture_tool(&mut self, ui: &mut Ui) {
        if shows_frame_capture_lifecycle(self.core.frame_capture_executor.state()) {
            let action = {
                let session = self
                    .state
                    .frame_capture
                    .as_mut()
                    .expect("frame capture source exists while lifecycle surface is shown");
                let request = session
                    .selection
                    .request(&session.media)
                    .expect("running frame capture session owns a valid timestamp");
                frame_capture_progress::show(
                    ui,
                    &request,
                    &mut self.core.frame_capture_executor,
                    &mut session.lifecycle_ui,
                )
            };
            self.handle_frame_capture_lifecycle_action(action);
            return;
        }

        if self.show_frame_capture_start_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let immediate_start_allowed =
            self.execution_arbitration().immediate_start_allowed() && !self.copy_frame_active();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::FrameCapture);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::FrameCapture)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(self.workspace_interaction_enabled(), |ui| {
                let session = self
                    .state
                    .frame_capture
                    .as_mut()
                    .expect("frame capture source exists while ready surface is shown");
                let live_preview = frame_capture_ui::FrameCaptureLivePreview::from_preview(
                    session.preview.state(),
                    session
                        .preview_texture
                        .as_ref()
                        .map(|texture| (texture.handle.id(), texture.size, texture.position)),
                );
                let navigation = frame_navigation_availability(&session.media, &session.editor);
                let copy = FrameCopyActionState {
                    enabled: session
                        .selection
                        .valid_position(&session.media)
                        .is_some_and(|position| {
                            copy_frame_action_eligible(&session.media, position)
                        }),
                    status: session.copy_status,
                };
                let FrameCaptureToolState { media, editor, .. } = session;
                frame_capture_ui::show(
                    ui,
                    media,
                    &presentation.frame_capture,
                    &mut editor.selection,
                    FrameCapturePositionSurface::Precision {
                        preview: live_preview,
                        navigation,
                    },
                    Some(copy),
                    OperationSurfaceMode::Immediate {
                        start_allowed: immediate_start_allowed,
                        queue_confirmation,
                        action_error: action_error.as_ref(),
                    },
                )
            })
            .inner;
        match action {
            FrameCaptureSelectionAction::PositionEdited(settle) => {
                let position = self
                    .state
                    .frame_capture
                    .as_ref()
                    .and_then(|session| session.selection.valid_position(&session.media));
                self.update_frame_capture_position(position, settle, Instant::now());
            }
            FrameCaptureSelectionAction::StepRequested(direction) => {
                self.request_frame_step(direction);
            }
            _ => {}
        }
        if let Err(error) = self.handle_frame_capture_action(action)
            && let Some(session) = self.state.frame_capture.as_mut()
        {
            session.start_error = Some(error.user_error());
            session.start_error_details_open = false;
        }
    }

    fn show_frame_capture_start_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.frame_capture.as_mut() else {
            return false;
        };
        let Some(error) = session.start_error.as_ref() else {
            return false;
        };

        let title = if error.kind() == MediaErrorKind::OutputPickerUnavailable {
            "Output picker unavailable"
        } else {
            "Frame capture could not start"
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
}

fn show_operation_editor(
    ui: &mut Ui,
    edit: &mut OperationEditorState,
    capabilities: crate::upscale::draft::UpscaleEncoderCapabilities<'_>,
    mode: OperationSurfaceMode,
    enabled: bool,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> QueueEditAction {
    let active_operation = edit.active_operation;
    layout::surface_fade(ui, egui::Id::new((id, active_operation.label())), |ui| {
        ui.add_enabled_ui(enabled, |ui| match active_operation {
            Operation::AudioMix => match audio_mix_ui::show(
                ui,
                &edit.media,
                &edit.presentation.audio_mix,
                &mut edit.audio_mix,
                mode,
            ) {
                AudioMixSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                AudioMixSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                AudioMixSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                _ => QueueEditAction::None,
            },
            Operation::VideoUpscale => {
                match upscale_ui::show(ui, &edit.media, &mut edit.upscale, capabilities, mode) {
                    UpscaleSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                    UpscaleSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                    UpscaleSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                    _ => QueueEditAction::None,
                }
            }
            Operation::Remux => match remux_ui::show(
                ui,
                &edit.media,
                &edit.presentation.remux,
                &mut edit.remux,
                mode,
            ) {
                RemuxSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                RemuxSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                RemuxSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                _ => QueueEditAction::None,
            },
            Operation::AudioExtract => match audio_extract_ui::show(
                ui,
                &edit.media,
                &edit.presentation.audio_extract,
                &mut edit.audio_extract,
                mode,
            ) {
                AudioExtractSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                AudioExtractSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                AudioExtractSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                _ => QueueEditAction::None,
            },
            Operation::FrameCapture => {
                let position_surface = edit.frame_capture.source_identity.as_ref().map_or(
                    FrameCapturePositionSurface::Simple,
                    |_| FrameCapturePositionSurface::Precision {
                        preview: frame_capture_ui::FrameCaptureLivePreview::from_preview(
                            edit.frame_capture.preview.state(),
                            edit.frame_capture.preview_texture.as_ref().map(|texture| {
                                (texture.handle.id(), texture.size, texture.position)
                            }),
                        ),
                        navigation: frame_navigation_availability(&edit.media, &edit.frame_capture),
                    },
                );
                match frame_capture_ui::show(
                    ui,
                    &edit.media,
                    &edit.presentation.frame_capture,
                    &mut edit.frame_capture.selection,
                    position_surface,
                    None,
                    mode,
                ) {
                    FrameCaptureSelectionAction::BrowseOutput => QueueEditAction::BrowseOutput,
                    FrameCaptureSelectionAction::SaveQueueChanges => QueueEditAction::Save,
                    FrameCaptureSelectionAction::CancelQueueEdit => QueueEditAction::Cancel,
                    FrameCaptureSelectionAction::PositionEdited(settle) => {
                        QueueEditAction::FramePositionEdited(settle)
                    }
                    FrameCaptureSelectionAction::StepRequested(direction) => {
                        QueueEditAction::FrameStepRequested(direction)
                    }
                    _ => QueueEditAction::None,
                }
            }
        })
        .inner
    })
}
