//! Root surface dispatch and egui rendering.
use crate::app::frame_capture::frame_navigation_availability;

use super::dialogs::editor_output_dialog_request;
use std::time::Instant;

use egui::{CentralPanel, Ui};

use crate::app::{NavigationState, Operation};
use crate::frame_capture::clipboard::copy_frame_action_eligible;
use crate::media::error::MediaError;
use crate::queue::QueueRunState;
use crate::ui::audio_extract::AudioExtractSelectionAction;
use crate::ui::audio_mix::AudioMixSelectionAction;
use crate::ui::frame_capture::{
    FrameCapturePositionSurface, FrameCaptureSelectionAction, FrameCopyActionState,
};
use crate::ui::remux::RemuxSelectionAction;
use crate::ui::upscale::UpscaleSelectionAction;
use crate::ui::workspace::{EditableQueuedJob, WorkspaceAction, WorkspaceMode};
use crate::ui::{
    OperationSurfaceMode, audio_extract as audio_extract_ui, audio_mix as audio_mix_ui,
    file_intake, frame_capture as frame_capture_ui, layout, media_error, queue as queue_ui,
    remux as remux_ui, theme, upscale as upscale_ui, widgets, workspace,
};

use super::ApolloApp;
use super::state::{EditorPresentation, FrameCaptureToolState, QueueEditAction};
use crate::app::{discovery::MediaToolsState, editor::OperationEditorState};

impl ApolloApp {
    pub(super) fn show_root(&mut self, ui: &mut Ui) {
        CentralPanel::default()
            .frame(layout::root_frame(ui.style()))
            .show(ui, |ui| {
                self.poll_background_work();
                if matches!(self.core.navigation(), NavigationState::Queue { .. })
                    && !matches!(self.core.discovery.state(), MediaToolsState::Ready(_))
                {
                    self.show_navigation(ui);
                    return;
                }
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
                if !self.confirm_cancel_queue && self.queue_frame_preview.is_none() {
                    self.handle_window_file_drop(ui.ctx());
                    if self.drag_hovered(ui.ctx()) {
                        file_intake::show_drag_hover(ui);
                    }
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
            ui.horizontal(|ui| {
                if widgets::secondary_button(ui, "Retry").clicked() {
                    self.retry_media_tools();
                }
                self.show_unavailable_queue_entry(ui);
            });
        });
    }

    fn show_media_tools_checking(&mut self, ui: &mut Ui) {
        layout::centered_content(ui, |ui| {
            ui.label("Checking media tools…");
            self.show_unavailable_queue_entry(ui);
        });
    }

    fn show_unavailable_queue_entry(&mut self, ui: &mut Ui) {
        let scheduler = self.core.queue.scheduler();
        if !scheduler.jobs().is_empty()
            && matches!(
                self.core.navigation(),
                NavigationState::NoSource
                    | NavigationState::SourceWorkspace { .. }
                    | NavigationState::BatchSetup
            )
            && widgets::secondary_button_enabled(
                ui,
                &crate::ui::queue_count_label(scheduler.pending_count()),
                self.file_intake_enabled(),
            )
            .clicked()
        {
            self.enter_queue();
        }
    }

    pub(super) fn operation_ready_surface(&self, active_operation: Operation) -> bool {
        match active_operation {
            Operation::AudioMix => self
                .state
                .audio_mix
                .as_ref()
                .is_some_and(|session| session.picker_error.is_none()),
            Operation::VideoUpscale => self
                .state
                .upscale
                .as_ref()
                .is_some_and(|session| session.picker_error.is_none()),
            Operation::Remux => self
                .state
                .remux
                .as_ref()
                .is_some_and(|session| session.picker_error.is_none()),
            Operation::AudioExtract => self
                .state
                .audio_extract
                .as_ref()
                .is_some_and(|session| session.picker_error.is_none()),
            Operation::FrameCapture => self
                .state
                .frame_capture
                .as_ref()
                .is_some_and(|session| session.picker_error.is_none()),
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
                .queue
                .scheduler()
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
                        pending_queue_count: self.core.queue.scheduler().pending_count(),
                        enabled: self.file_intake_enabled(),
                        editable_jobs: &editable_jobs,
                        queue_editing_allowed: self.core.queue.scheduler().run_state()
                            == QueueRunState::Stopped
                            && !self.core.queue.scheduler().is_running(),
                    },
                )
            })
            .inner;
            self.handle_workspace_action(workspace_action);
        }

        let Some(active_operation) = self.core.active_operation() else {
            ui.ctx().request_repaint();
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
            self.core.queue.scheduler().pending_count(),
            self.core.source.error(),
            &mut self.state.intake_error_details_open,
        );
        self.handle_file_intake_action(action);
    }

    pub(super) fn show_queue(&mut self, ui: &mut Ui) {
        let scheduler = self.core.queue.scheduler();
        let capabilities_ready = self.core.discovery.settled_capabilities().is_some();
        let waiting_for_encoder_discovery = self.ready_media_tools().is_some()
            && !capabilities_ready
            && scheduler.waiting_jobs().next().is_some()
            && !scheduler.next_job_ready(false);
        let action = queue_ui::show(
            ui,
            self.core.queue.scheduler().jobs(),
            self.core.queue.scheduler().active_job(),
            self.core.queue.scheduler().active_progress(),
            self.core.queue.scheduler().run_state(),
            queue_ui::QueueAvailability {
                can_start: self.core.queue_start_allowed()
                    && scheduler.next_job_ready(capabilities_ready),
                waiting_for_encoder_discovery,
                can_add_files: self.file_intake_enabled() && self.ready_media_tools().is_some(),
                can_edit: self.ready_media_tools().is_some(),
            },
            self.core.queue.scheduler().cancellation_pending(),
        );
        self.handle_queue_view_action(action);
        self.show_queue_frame_preview(ui);
        if self.confirm_cancel_queue {
            if self.core.queue.scheduler().pending_count() == 0 {
                self.resolve_queue_cancellation(false);
                return;
            }
            let (confirm, dismiss) = queue_ui::confirm_cancel(ui);
            if confirm || dismiss {
                self.resolve_queue_cancellation(confirm);
            }
        }
    }

    fn show_queue_edit(&mut self, ui: &mut Ui) {
        let Some(edit) = self.core.queue_edit() else {
            return;
        };
        let Some(presentation) = self.queue_editor_presentation.as_ref() else {
            return;
        };
        let presentation = &presentation.source;
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

        let Some((edit, capabilities)) = self.core.queue_edit_with_capabilities() else {
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

        let Some(presentation) = self.queue_editor_presentation.as_ref() else {
            return;
        };
        let action = show_operation_editor(
            ui,
            edit,
            presentation,
            capabilities,
            OperationSurfaceMode::QueueEdit,
            enabled,
            ("queue-editor", job_id.get()),
        );
        match action {
            QueueEditAction::BrowseOutput => {
                if let Some(edit) = self.core.queue_edit() {
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
                    .core
                    .queue_edit()
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
        if self.show_frame_capture_picker_error(ui) {
            return;
        }

        let presentation = self
            .state
            .source_presentation
            .as_ref()
            .expect("source presentation exists while workspace is shown")
            .clone();
        let process_now_allowed = self.core.process_now_allowed();
        let clipboard_allowed = self.core.clipboard_start_allowed();
        let queue_confirmation = self.core.queue_confirmation == Some(Operation::FrameCapture);
        let action_error = self
            .core
            .operation_action_error
            .as_ref()
            .filter(|(tool, _)| *tool == Operation::FrameCapture)
            .map(|(_, error)| error.clone());
        let action = ui
            .add_enabled_ui(
                self.file_intake_enabled() && self.ready_media_tools().is_some(),
                |ui| {
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
                        enabled: clipboard_allowed
                            && session
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
                        OperationSurfaceMode::Normal {
                            process_now_allowed,
                            queue_confirmation,
                            action_error: action_error.as_ref(),
                        },
                    )
                },
            )
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
        self.handle_frame_capture_action(action);
    }

    fn show_frame_capture_picker_error(&mut self, ui: &mut Ui) -> bool {
        let Some(session) = self.state.frame_capture.as_mut() else {
            return false;
        };
        let Some(error) = session.picker_error.as_ref() else {
            return false;
        };

        let response = layout::focus_content(ui, |ui| {
            let mut sizing_details_open = session.picker_error_details_open;
            let details_open = if ui.is_sizing_pass() {
                &mut sizing_details_open
            } else {
                &mut session.picker_error_details_open
            };
            media_error::show_failure_content(
                ui,
                "Output picker unavailable",
                error,
                details_open,
                "Back",
            )
        });
        if response.inner.clicked() {
            session.picker_error = None;
            session.picker_error_details_open = false;
        }
        true
    }
}

pub(super) fn show_operation_editor(
    ui: &mut Ui,
    edit: &mut OperationEditorState,
    presentation: &EditorPresentation,
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
                &presentation.source.audio_mix,
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
                &presentation.source.remux,
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
                &presentation.source.audio_extract,
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
                            presentation.preview_texture.as_ref().map(|texture| {
                                (texture.handle.id(), texture.size, texture.position)
                            }),
                        ),
                        navigation: frame_navigation_availability(&edit.media, &edit.frame_capture),
                    },
                );
                match frame_capture_ui::show(
                    ui,
                    &edit.media,
                    &presentation.source.frame_capture,
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
