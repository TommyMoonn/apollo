//! Frame Capture presentation and routing to the current editor.
use super::{
    ApolloApp, completion_wake,
    state::{BatchEditTarget, FramePreviewTexture},
};
use crate::app::editor::OperationEditorState;
use crate::app::frame_capture::{FrameCaptureEditorState, FrameCopyStatus, PositionSettle};
use crate::app::{NavigationState, Operation, dialogs::OutputDraft};
use crate::frame_capture::{
    clipboard::{CopyFrameError, CopyFrameRequest},
    execution::FrameCaptureStartError,
    frame_index::{
        FrameDirection, FrameIndexError, FrameIndexRequest, FrameIndexState, FrameNavigation,
    },
    model::FramePosition,
    preview::{PreviewExtractionError, PreviewImage, PreviewRequest, PreviewState},
};
use crate::media::{model::MediaFile, process::MediaProcessWake};
use crate::native::open_folder;
use crate::ui::{
    frame_capture::FrameCaptureSelectionAction,
    frame_capture_progress::{FrameCaptureLifecycleAction, FrameCaptureThumbnailRequest},
};
use egui::{ColorImage, TextureOptions};
use std::time::Instant;

impl ApolloApp {
    pub(super) fn copy_frame_active(&self) -> bool {
        self.core.frame_work.copy_active()
    }

    fn active_frame_capture_editor(&self) -> Option<(&MediaFile, &FrameCaptureEditorState)> {
        match self.core.navigation() {
            NavigationState::SourceWorkspace {
                active_operation: Operation::FrameCapture,
            } => self
                .state
                .frame_capture
                .as_ref()
                .map(|session| (&session.media, &session.editor.state)),
            NavigationState::QueueEdit { .. } => self
                .core
                .queue_edit()
                .filter(|edit| edit.active_operation == Operation::FrameCapture)
                .map(|edit| (&edit.media, &edit.frame_capture)),
            NavigationState::BatchEdit => self
                .batch_edit
                .as_ref()
                .filter(|edit| {
                    matches!(edit.target, BatchEditTarget::Item(_))
                        && edit.active_operation == Operation::FrameCapture
                })
                .map(|edit| (&edit.media, &edit.frame_capture)),
            _ => None,
        }
    }

    pub(super) fn active_frame_capture_editor_mut(
        &mut self,
    ) -> Option<(&MediaFile, &mut FrameCaptureEditorState)> {
        self.active_frame_capture_parts_mut()
            .map(|(media, editor, _)| (media, editor))
    }

    fn active_frame_capture_parts_mut(
        &mut self,
    ) -> Option<(
        &MediaFile,
        &mut FrameCaptureEditorState,
        &mut Option<FramePreviewTexture>,
    )> {
        match self.core.navigation() {
            NavigationState::SourceWorkspace {
                active_operation: Operation::FrameCapture,
            } => self.state.frame_capture.as_mut().map(|session| {
                (
                    &session.media,
                    &mut session.editor.state,
                    &mut session.editor.preview_texture,
                )
            }),
            NavigationState::QueueEdit { .. } => {
                let presentation = self.queue_editor_presentation.as_mut()?;
                let edit = self
                    .core
                    .queue_edit_mut()
                    .filter(|edit| edit.active_operation == Operation::FrameCapture)?;
                let OperationEditorState {
                    media,
                    frame_capture,
                    ..
                } = &mut edit.editor;
                Some((media, frame_capture, &mut presentation.preview_texture))
            }
            NavigationState::BatchEdit => {
                let edit = self.batch_edit.as_mut().filter(|edit| {
                    matches!(edit.target, BatchEditTarget::Item(_))
                        && edit.active_operation == Operation::FrameCapture
                })?;
                let OperationEditorState {
                    media,
                    frame_capture,
                    ..
                } = &mut edit.editor;
                Some((media, frame_capture, &mut edit.presentation.preview_texture))
            }
            _ => None,
        }
    }

    pub(super) fn schedule_active_frame_capture_support(&mut self) {
        let now = Instant::now();
        let Some((media, editor)) = self.active_frame_capture_editor_mut() else {
            return;
        };
        if editor.source_identity.is_none() {
            return;
        }
        let Some(position) = editor.selection.valid_position(media) else {
            return;
        };
        if matches!(editor.preview.state(), PreviewState::Idle) {
            self.update_frame_capture_position(Some(position), PositionSettle::Debounced, now);
        } else if matches!(editor.frame_navigator.state(), FrameIndexState::Idle)
            && editor.frame_index_due.is_none()
        {
            editor.frame_index_due = Some((position, now));
            self.egui_context.request_repaint();
        }
    }

    pub(super) fn update_frame_capture_position(
        &mut self,
        position: Option<FramePosition>,
        settle: PositionSettle,
        now: Instant,
    ) {
        self.core.frame_work.cancel_index();
        let Some((_, editor)) = self.active_frame_capture_editor_mut() else {
            return;
        };
        if editor.source_identity.is_none() {
            return;
        }
        editor.retire_index();
        let ready_at = self.schedule_frame_capture_preview(position, settle, now);
        if let (Some(position), Some(ready_at), Some((_, editor))) =
            (position, ready_at, self.active_frame_capture_editor_mut())
        {
            editor.frame_index_due = Some((position, ready_at));
        }
    }

    fn schedule_frame_capture_preview(
        &mut self,
        position: Option<FramePosition>,
        settle: PositionSettle,
        now: Instant,
    ) -> Option<Instant> {
        self.core.frame_work.cancel_preview();
        let (media, editor) = self.active_frame_capture_editor_mut()?;
        let deadline = editor.schedule_preview(media, position, settle, now)?;
        self.egui_context
            .request_repaint_after(deadline.saturating_duration_since(now));
        Some(deadline)
    }

    pub(super) fn poll_frame_capture_preview(&mut self) {
        let due = self
            .active_frame_capture_editor_mut()
            .and_then(|(_, editor)| {
                let session = editor.session_id;
                editor
                    .preview
                    .take_due_request(Instant::now())
                    .map(|request| (session, request))
            });
        if let Some((session, request)) = due {
            self.start_frame_preview_extraction(session, request);
        } else if self
            .active_frame_capture_editor()
            .is_some_and(|(_, editor)| matches!(editor.preview.state(), PreviewState::Ready { .. }))
        {
            self.core.frame_work.cancel_preview();
        }
        self.poll_frame_preview_extraction();
        self.poll_frame_preview_decode();
        self.sync_frame_preview_texture();
    }

    pub(super) fn poll_frame_capture_index(&mut self) {
        let now = Instant::now();
        if let Some(position) = self
            .active_frame_capture_editor_mut()
            .and_then(|(_, editor)| editor.take_due_index(now))
        {
            self.start_frame_index_query(position);
        }
        if let Some((session, completion)) = self.core.frame_work.poll_index() {
            self.settle_frame_index_request(session, completion.request, completion.result);
        }
    }

    fn start_frame_index_query(&mut self, position: FramePosition) {
        let Some(tools) = self.ready_media_tools().cloned() else {
            return;
        };
        if self
            .active_frame_capture_editor()
            .is_some_and(|(_, editor)| {
                editor.index_loading_at(position)
                    && self.core.frame_work.index_owned_by(editor.session_id)
            })
        {
            return;
        }
        let Some((media, editor)) = self.active_frame_capture_editor_mut() else {
            return;
        };
        let session = editor.session_id;
        let request = match editor.schedule_index(media, position) {
            Ok(Some(request)) => request,
            Ok(None) => {
                self.resolve_pending_frame_step();
                return;
            }
            Err(_) => return,
        };
        let context = self.egui_context.clone();
        let wake = MediaProcessWake::new(move |_| context.request_repaint());
        if let Err(error) = self
            .core
            .frame_work
            .start_index(session, &tools, request.clone(), wake)
        {
            self.settle_frame_index_request(session, request, Err(error));
        }
    }

    pub(super) fn settle_frame_index_request(
        &mut self,
        session: u64,
        request: FrameIndexRequest,
        result: Result<Vec<u8>, FrameIndexError>,
    ) {
        if self
            .active_frame_capture_editor_mut()
            .is_some_and(|(_, editor)| editor.settle_index(session, request, result))
        {
            self.resolve_pending_frame_step();
        }
    }

    pub(super) fn request_frame_step(&mut self, direction: FrameDirection) {
        let outcome = self
            .active_frame_capture_editor_mut()
            .and_then(|(media, editor)| editor.request_step(media, direction));
        if let Some(outcome) = outcome {
            self.apply_frame_step_outcome(outcome);
        }
    }

    fn resolve_pending_frame_step(&mut self) {
        let outcome = self
            .active_frame_capture_editor_mut()
            .and_then(|(_, editor)| editor.pending_outcome());
        if let Some(outcome) = outcome {
            self.apply_frame_step_outcome(outcome);
        }
    }

    fn apply_frame_step_outcome(&mut self, outcome: FrameNavigation) {
        match outcome {
            FrameNavigation::Resolved(position) => {
                let Some((media, editor)) = self.active_frame_capture_editor_mut() else {
                    return;
                };
                if !editor.apply_resolved_step(media, position.into()) {
                    return;
                }
                self.schedule_frame_capture_preview(
                    Some(position.into()),
                    PositionSettle::Immediate,
                    Instant::now(),
                );
            }
            FrameNavigation::NeedsNeighborhood(anchor) => self.start_frame_index_query(anchor),
            FrameNavigation::Boundary => {}
        }
    }

    fn start_frame_preview_extraction(&mut self, session: u64, request: PreviewRequest) {
        let tools = self
            .ready_media_tools()
            .cloned()
            .expect("preview extraction starts only while media tools are ready");
        let context = self.egui_context.clone();
        let wake = MediaProcessWake::new(move |_| context.request_repaint());
        if let Err(error) =
            self.core
                .frame_work
                .start_preview(session, &tools, request.clone(), wake)
        {
            self.settle_frame_preview(session, request, Err(PreviewExtractionError::Start(error)));
        }
    }

    fn poll_frame_preview_extraction(&mut self) {
        let Some((session, completion)) = self.core.frame_work.poll_preview() else {
            return;
        };
        if !self
            .active_frame_capture_editor()
            .is_some_and(|(_, editor)| editor.accepts_preview(session, &completion.request))
        {
            return;
        }
        let request = completion.request;
        match completion.result {
            Ok(bytes) => {
                if let Err(error) = self.core.frame_work.start_decode(
                    session,
                    request.clone(),
                    bytes,
                    completion_wake(&self.egui_context),
                ) {
                    self.settle_frame_preview(session, request, Err(error));
                }
            }
            Err(error) => {
                self.settle_frame_preview(session, request, Err(error));
            }
        }
    }

    pub(super) fn poll_frame_preview_decode(&mut self) {
        if let Some((session, request, result)) = self.core.frame_work.poll_decode() {
            self.settle_frame_preview(session, request, result);
        }
    }

    pub(super) fn settle_frame_preview(
        &mut self,
        session: u64,
        request: PreviewRequest,
        result: Result<PreviewImage, PreviewExtractionError>,
    ) -> bool {
        self.active_frame_capture_editor_mut()
            .is_some_and(|(_, editor)| editor.settle_preview(session, request, result))
    }

    pub(super) fn sync_frame_preview_texture(&mut self) {
        let context = self.egui_context.clone();
        let Some((_, editor, texture)) = self.active_frame_capture_parts_mut() else {
            return;
        };
        let PreviewState::Ready { request, image } = editor.preview.state() else {
            return;
        };
        if texture
            .as_ref()
            .is_some_and(|texture| texture.generation == request.generation)
        {
            return;
        }
        let size = image.size();
        let color = ColorImage::from_rgba_unmultiplied(size, image.rgba());
        let handle = context.load_texture(
            format!(
                "frame-capture-preview-{}-{}",
                editor.session_id,
                request.generation.get()
            ),
            color,
            TextureOptions::LINEAR,
        );
        *texture = Some(FramePreviewTexture {
            generation: request.generation,
            size,
            position: request.position,
            handle,
        });
    }

    pub(super) fn suspend_frame_capture_support(&mut self) {
        self.core.frame_work.cancel_preview();
        self.core.frame_work.cancel_index();
        if let Some((_, editor)) = self.active_frame_capture_editor_mut() {
            editor.suspend_support();
        }
    }

    pub(super) fn cancel_frame_capture_support(&mut self) {
        self.core.frame_work.cancel_preview();
        self.core.frame_work.cancel_index();
        if let Some((_, editor, texture)) = self.active_frame_capture_parts_mut() {
            editor.retire_support();
            *texture = None;
        }
    }

    fn start_copy_frame(&mut self) {
        if self.copy_frame_active()
            || self.core.media_operation_running()
            || !self.core.execution_arbitration().immediate_start_allowed()
        {
            return;
        }
        let request = self.state.frame_capture.as_ref().and_then(|session| {
            Some((
                session.session_id,
                session.editor.state.copy_request(&session.media)?,
            ))
        });
        let Some((session_id, request)) = request else {
            return;
        };
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                self.settle_copy_frame(session_id, None, Err(error));
                return;
            }
        };
        let Some(tools) = self.ready_media_tools().cloned() else {
            return;
        };
        if let Some(session) = self.state.frame_capture.as_mut() {
            session.copy_status = FrameCopyStatus::Copying;
        }
        let context = self.egui_context.clone();
        let wake = MediaProcessWake::new(move |_| context.request_repaint());
        if let Err(error) =
            self.core
                .frame_work
                .start_copy(session_id, &tools, request.clone(), wake)
        {
            self.settle_copy_frame(session_id, Some(&request), Err(error));
        }
    }

    pub(super) fn poll_copy_frame(&mut self) {
        if let Some((session_id, request, result)) = self.core.frame_work.poll_copy_publication() {
            self.settle_copy_frame(session_id, Some(&request), result);
        }
        let Some((session_id, completion)) = self.core.frame_work.poll_copy_extraction() else {
            return;
        };
        if !self.copy_frame_request_is_current(session_id, &completion.request) {
            return;
        }
        let request = completion.request;
        match completion.result {
            Ok(bytes) => {
                if let Err(error) = self.core.frame_work.start_copy_publication(
                    session_id,
                    request.clone(),
                    bytes,
                    self.clipboard_owner,
                    completion_wake(&self.egui_context),
                ) {
                    self.settle_copy_frame(session_id, Some(&request), Err(error));
                }
            }
            Err(error) => self.settle_copy_frame(session_id, Some(&request), Err(error)),
        }
    }

    fn copy_frame_request_is_current(&self, session_id: u64, request: &CopyFrameRequest) -> bool {
        self.state
            .frame_capture
            .as_ref()
            .is_some_and(|session| session.editor.state.accepts_copy(session_id, Some(request)))
    }

    fn settle_copy_frame(
        &mut self,
        session_id: u64,
        request: Option<&CopyFrameRequest>,
        result: Result<(), CopyFrameError>,
    ) {
        let Some(session) = self
            .state
            .frame_capture
            .as_mut()
            .filter(|session| session.editor.state.accepts_copy(session_id, request))
        else {
            return;
        };
        let succeeded = result.is_ok();
        if let Some(error) = session.editor.state.settle_copy(result) {
            self.core.operation_action_error = Some((Operation::FrameCapture, error));
        } else if succeeded {
            self.core.operation_action_error = None;
        }
    }

    pub(super) fn cancel_copy_frame_work(&mut self) {
        self.core.frame_work.cancel_copy();
        if let Some(session) = self.state.frame_capture.as_mut() {
            session.copy_status = FrameCopyStatus::Idle;
        }
    }

    pub(super) fn poll_frame_capture_thumbnail(&mut self) {
        let Some((session_id, request_id, result)) = self.core.frame_work.poll_thumbnail() else {
            return;
        };
        if let Some(session) = self
            .state
            .frame_capture
            .as_mut()
            .filter(|session| session.session_id == session_id)
        {
            session.lifecycle_ui.settle_thumbnail(request_id, result);
        }
    }

    pub(super) fn handle_frame_capture_action(
        &mut self,
        action: FrameCaptureSelectionAction,
    ) -> Result<(), FrameCaptureStartError> {
        if self.file_dialog.is_pending() {
            return Ok(());
        }
        if action != FrameCaptureSelectionAction::None {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
        }
        match action {
            FrameCaptureSelectionAction::BrowseOutput => {
                let Some(session) = self.state.frame_capture.as_ref() else {
                    return Ok(());
                };
                self.start_file_dialog(
                    OutputDraft::FrameCapture(&session.selection).request(session.session_id),
                );
            }
            FrameCaptureSelectionAction::QueueRequested => {
                self.enqueue_from_ready_surface(Operation::FrameCapture);
            }
            FrameCaptureSelectionAction::CaptureRequested => {
                if !self.core.execution_arbitration().immediate_start_allowed() {
                    return Ok(());
                }
                let Some(session) = self.state.frame_capture.as_ref() else {
                    return Ok(());
                };
                let Some(request) = session.selection.request(&session.media) else {
                    return Ok(());
                };
                let Some(tools) = self.ready_media_tools().cloned() else {
                    return Ok(());
                };
                let wake = self.progress_repaint.begin();
                self.core.frame_capture_executor.start_with_wake(
                    &tools,
                    &session.media,
                    &request,
                    wake,
                )?;
                self.core.frame_work.cancel_thumbnail();
            }
            FrameCaptureSelectionAction::CopyRequested => {
                self.start_copy_frame();
            }
            FrameCaptureSelectionAction::SaveQueueChanges
            | FrameCaptureSelectionAction::CancelQueueEdit
            | FrameCaptureSelectionAction::None => {}
            FrameCaptureSelectionAction::PositionEdited(_)
            | FrameCaptureSelectionAction::StepRequested(_) => {
                if let Some(session) = self.state.frame_capture.as_mut() {
                    session.copy_status = FrameCopyStatus::Idle;
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_frame_capture_lifecycle_action(
        &mut self,
        action: FrameCaptureLifecycleAction,
    ) {
        match action {
            FrameCaptureLifecycleAction::LoadThumbnail(request) => {
                self.start_frame_capture_thumbnail_load(request);
            }
            FrameCaptureLifecycleAction::OpenFolder(path) => {
                if let Err(error) = open_folder(&path) {
                    eprintln!("Frame Capture output folder could not be opened: {error}");
                }
            }
            FrameCaptureLifecycleAction::None
            | FrameCaptureLifecycleAction::CancelRequested
            | FrameCaptureLifecycleAction::Done
            | FrameCaptureLifecycleAction::Back => {}
        }
    }

    fn start_frame_capture_thumbnail_load(&mut self, request: FrameCaptureThumbnailRequest) {
        let Some(session_id) = self
            .state
            .frame_capture
            .as_ref()
            .map(|session| session.session_id)
        else {
            return;
        };
        match self.core.frame_work.start_thumbnail(
            session_id,
            &request,
            completion_wake(&self.egui_context),
        ) {
            Ok(()) => {}
            Err(error) => {
                if let Some(session) = self
                    .state
                    .frame_capture
                    .as_mut()
                    .filter(|session| session.session_id == session_id)
                {
                    session.lifecycle_ui.settle_thumbnail(
                        request.request_id(),
                        Err(format!(
                            "Frame Capture preview worker could not start: {error}"
                        )),
                    );
                }
            }
        }
    }
}
