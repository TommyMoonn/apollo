//! Queue presentation integration and native action dispatch.
use super::{ApolloApp, completion_wake, state::EditorPresentation};
use crate::frame_capture::thumbnail::{FrameCaptureThumbnail, decode_frame_capture_thumbnail};
use crate::ui::{theme, widgets};
use crate::{
    app::{
        NavigationState, Operation, QueueAction, QueueEditOrigin,
        dialogs::{DialogTarget, FileDialogRequest},
        queue::{QueueEditReturn, QueueEffect, QueueSubmission},
    },
    native::open_folder,
    queue::{JobId, QueueEnqueueError, QueueJobState, QueuedOperation},
};
use std::{path::PathBuf, sync::mpsc, thread};

impl ApolloApp {
    pub(super) fn enter_queue(&mut self) {
        if !self.file_intake_enabled() {
            return;
        }
        let previous = self.core.navigation();
        if self.core.enter_queue() {
            self.suspend_frame_capture_support_from(previous);
            self.state.intake_error_details_open = false;
            self.egui_context.request_repaint();
        }
    }

    pub(super) fn handle_queue_view_action(&mut self, action: QueueAction) {
        if self.confirm_cancel_queue || self.queue_frame_preview.is_some() {
            return;
        }
        if action == QueueAction::CancelQueue {
            if self.core.queue.scheduler().pending_count() > 0 {
                self.confirm_cancel_queue = true;
                self.egui_context.request_repaint();
            }
        } else {
            self.handle_queue_action(action);
        }
    }

    pub(super) fn resolve_queue_cancellation(&mut self, confirmed: bool) {
        if !std::mem::take(&mut self.confirm_cancel_queue) {
            return;
        }
        if confirmed {
            self.handle_queue_action(QueueAction::CancelQueue);
        }
        self.egui_context.request_repaint();
    }

    pub(super) fn handle_queue_action(&mut self, action: QueueAction) -> bool {
        if action == QueueAction::None {
            return false;
        }
        if action == QueueAction::AddFiles
            && (!self.file_intake_enabled() || self.ready_media_tools().is_none())
        {
            return false;
        }
        let progress = &mut self.progress_repaint;
        match self.core.queue_action(action, || progress.begin()) {
            QueueEffect::Handled(changed) => {
                if changed || self.core.queue_action_error.is_some() {
                    self.egui_context.request_repaint();
                }
                changed
            }
            QueueEffect::AddFiles => {
                self.start_file_dialog(FileDialogRequest::OpenSources {
                    initial_directory: None,
                    target: DialogTarget::BatchInput,
                });
                true
            }
            QueueEffect::Edit(id, origin) => self.begin_queue_edit(id, origin),
            QueueEffect::OpenFolder(directory) => {
                if let Err(error) = open_folder(&directory) {
                    eprintln!("Queue output folder could not be opened: {error}");
                }
                true
            }
            QueueEffect::PreviewFrame(id, output) => {
                self.queue_frame_preview = Some(QueueFramePreview::new(
                    id,
                    output,
                    completion_wake(&self.egui_context),
                ));
                self.egui_context.request_repaint();
                true
            }
            QueueEffect::Returned => {
                self.queue_frame_preview = None;
                self.schedule_active_frame_capture_support();
                true
            }
        }
    }

    pub(super) fn begin_queue_edit(&mut self, id: JobId, origin: QueueEditOrigin) -> bool {
        if self.ready_media_tools().is_none() {
            return false;
        }
        let Some(edit) = self.core.prepare_queue_edit(id, origin) else {
            return false;
        };
        self.suspend_frame_capture_support();
        self.queue_editor_presentation = Some(EditorPresentation::new(&edit.media));
        self.core.activate_queue_edit(edit, origin);
        self.schedule_active_frame_capture_support();
        true
    }

    pub(super) fn finish_queue_edit(&mut self) -> bool {
        if !matches!(self.core.navigation(), NavigationState::QueueEdit { .. }) {
            return false;
        }
        self.cancel_frame_capture_support();
        let Some(return_to) = self.core.finish_queue_edit() else {
            return false;
        };
        self.queue_editor_presentation = None;
        match return_to {
            QueueEditReturn::Queue => true,
            QueueEditReturn::Source(request) => self.restore_queued_job_workspace(&request),
        }
    }

    pub(super) fn select_queue_edit_tool(&mut self, operation: Operation) -> bool {
        if !matches!(self.core.navigation(), NavigationState::QueueEdit { .. }) {
            return false;
        }
        let Some(previous) = self.core.queue_edit().map(|edit| edit.active_operation) else {
            return false;
        };
        if previous == operation {
            return true;
        }
        if previous == Operation::FrameCapture {
            self.cancel_frame_capture_support();
        }
        if !self.core.select_queue_edit_tool(operation) {
            return false;
        }
        if operation == Operation::FrameCapture {
            self.schedule_active_frame_capture_support();
        }
        true
    }

    pub(super) fn save_queue_edit(&mut self) -> bool {
        self.core.save_queue_edit() && self.finish_queue_edit()
    }

    pub(super) fn enqueue_current_operation(
        &mut self,
        operation: Operation,
    ) -> Result<Option<JobId>, QueueEnqueueError> {
        // Borrow each draft and its source directly; the queue owns the resulting immutable request.
        let queued = match operation {
            Operation::AudioMix => {
                let session = self
                    .state
                    .audio_mix
                    .as_ref()
                    .ok_or(QueueEnqueueError::SourceDoesNotMatchMedia)?;
                Some((
                    &session.media,
                    QueuedOperation::AudioMix(session.selection.request(&session.media)),
                ))
            }
            Operation::VideoUpscale => {
                let Some(capabilities) = self.core.discovery.upscale_capabilities() else {
                    return Ok(None);
                };
                self.state.upscale.as_ref().and_then(|session| {
                    session
                        .selection
                        .request_for_capabilities(&session.media, capabilities)
                        .map(|request| (&session.media, QueuedOperation::Upscale(request)))
                })
            }
            Operation::Remux => {
                let session = self
                    .state
                    .remux
                    .as_ref()
                    .ok_or(QueueEnqueueError::SourceDoesNotMatchMedia)?;
                Some((
                    &session.media,
                    QueuedOperation::Remux(session.selection.request(&session.media)),
                ))
            }
            Operation::AudioExtract => self.state.audio_extract.as_ref().and_then(|session| {
                session
                    .selection
                    .request(&session.media)
                    .map(|request| (&session.media, QueuedOperation::AudioExtract(request)))
            }),
            Operation::FrameCapture => self.state.frame_capture.as_ref().and_then(|session| {
                session
                    .selection
                    .validated_request(&session.media)
                    .ok()
                    .map(|request| (&session.media, QueuedOperation::FrameCapture(request)))
            }),
        };
        let Some((media, request)) = queued else {
            return Ok(None);
        };
        let Some((identity, accepted_media)) = self.core.source.current_with_identity() else {
            return Err(QueueEnqueueError::SourceDoesNotMatchMedia);
        };
        if media != accepted_media {
            return Err(QueueEnqueueError::SourceDoesNotMatchMedia);
        }
        self.core
            .queue
            .enqueue(identity, accepted_media, request)
            .map(Some)
    }

    /// Both migrated form actions admit the same immutable request. Only Process now starts it.
    pub(super) fn submit_queued_operation(
        &mut self,
        operation: Operation,
        submission: QueueSubmission,
    ) {
        let process_now = submission == QueueSubmission::ProcessNow;
        if self.core.active_operation() != Some(operation)
            || !self.file_intake_enabled()
            || !self.operation_ready_surface(operation)
            || self.ready_media_tools().is_none()
            || (process_now && !self.core.process_now_allowed())
        {
            return;
        }
        let result = self.enqueue_current_operation(operation);
        let admitted = matches!(result, Ok(Some(_)));
        self.core.record_queue_submission(operation, result);
        if admitted && process_now {
            self.enter_queue();
            self.handle_queue_action(QueueAction::Start);
        }
    }
}

// One view-local decode request. Dropping the receiver retires the result without blocking UI.
pub(super) struct QueueFramePreview {
    job_id: JobId,
    state: QueueFramePreviewState,
}

enum QueueFramePreviewState {
    Loading(mpsc::Receiver<Result<FrameCaptureThumbnail, String>>),
    Ready(egui::TextureHandle),
    Failed(String),
}

impl QueueFramePreview {
    fn new(job_id: JobId, output: PathBuf, wake: impl FnOnce() + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let spawn = thread::Builder::new()
            .name("queue-frame-thumbnail".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| decode_frame_capture_thumbnail(&output))
                    .unwrap_or_else(|_| Err("Captured image decoding failed.".into()));
                let _ = sender.send(result);
                drop(sender);
                wake();
            });
        Self {
            job_id,
            state: match spawn {
                Ok(_) => QueueFramePreviewState::Loading(receiver),
                Err(error) => {
                    QueueFramePreviewState::Failed(format!("Preview could not start: {error}"))
                }
            },
        }
    }

    fn poll(&mut self, context: &egui::Context) {
        let QueueFramePreviewState::Loading(receiver) = &self.state else {
            return;
        };
        self.state = match receiver.try_recv() {
            Ok(Ok(image)) => QueueFramePreviewState::Ready(context.load_texture(
                "queue-frame-thumbnail",
                egui::ColorImage::from_rgba_unmultiplied(image.size(), image.rgba()),
                egui::TextureOptions::LINEAR,
            )),
            Ok(Err(error)) => QueueFramePreviewState::Failed(error),
            Err(mpsc::TryRecvError::Disconnected) => {
                QueueFramePreviewState::Failed("Preview worker disconnected.".into())
            }
            Err(mpsc::TryRecvError::Empty) => return,
        };
    }
}

impl ApolloApp {
    pub(super) fn show_queue_frame_preview(&mut self, ui: &egui::Ui) {
        let Some(preview) = self.queue_frame_preview.as_mut() else {
            return;
        };
        if !self.core.queue.scheduler().jobs().iter().any(|job| {
            job.id() == preview.job_id && matches!(job.state(), QueueJobState::Completed { .. })
        }) {
            self.queue_frame_preview = None;
            return;
        }
        preview.poll(ui.ctx());
        let modal = egui::Modal::new(egui::Id::new("queue-frame-preview")).show(ui.ctx(), |ui| {
            let available = ui.ctx().content_rect().size() - egui::vec2(64.0, 140.0);
            let size = available
                .min(egui::vec2(512.0, 512.0))
                .max(egui::vec2(32.0, 32.0));
            ui.set_max_width(size.x);
            ui.heading("Captured frame");
            ui.add_space(theme::SPACING.related);
            match &preview.state {
                QueueFramePreviewState::Loading(_) => {
                    ui.label("Loading preview…");
                }
                QueueFramePreviewState::Ready(texture) => {
                    ui.add(
                        egui::Image::new((texture.id(), texture.size_vec2()))
                            .max_size(size)
                            .alt_text("Captured frame"),
                    );
                }
                QueueFramePreviewState::Failed(error) => {
                    ui.label(error);
                }
            }
            ui.add_space(theme::SPACING.related);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::secondary_button(ui, "Close").clicked()
            })
            .inner
        });
        if modal.inner || modal.should_close() {
            self.queue_frame_preview = None;
        }
    }
}
