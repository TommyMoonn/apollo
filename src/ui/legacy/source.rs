//! Source actions and presentation settlement for the legacy renderer.

use super::audio_extract::AudioExtractToolState;
use super::audio_mix::AudioMixToolState;
use super::remux::RemuxToolState;
use super::upscale::UpscaleToolState;
use super::{
    ApolloApp, completion_wake,
    state::{FrameCaptureEditor, FrameCaptureToolState},
};
use crate::app::{Operation, QueueEditOrigin, source::SourceSettlement};
use crate::media::intake::{MediaIntakeRequest, MediaLoadStart};
#[cfg(test)]
use crate::media::{model::MediaFile, probe::MediaProbeError};
use crate::queue::QueuedOperation;
#[cfg(test)]
use crate::ui::frame_capture::FrameCopyStatus;
use crate::ui::frame_capture::PositionSettle;
use crate::ui::source_presentation::SourcePresentation;
use crate::{
    audio_extract::draft::AudioExtractDraft, audio_mix::draft::AudioMixDraft,
    frame_capture::draft::FrameCaptureDraft, remux::draft::RemuxDraft,
    upscale::draft::UpscaleDraft,
};
use std::{path::Path, sync::Arc, time::Instant};

impl ApolloApp {
    fn enter_workspace(&mut self, operation: Operation) {
        self.core.enter_workspace(operation);
        self.batch_editor_presentation = None;
        self.batch_append_error_details_open = false;
        self.state.intake_error_details_open = false;
    }

    pub(super) fn close_source(&mut self) {
        self.retire_source_presentation();
        self.core.close_source();
        self.batch_editor_presentation = None;
        self.batch_append_error_details_open = false;
    }

    fn retire_source_presentation(&mut self) {
        self.cancel_copy_frame_work();
        self.cancel_frame_capture_support();
        self.state.source_presentation = None;
        self.state.retire_tool_sessions();
        self.state.intake_error_details_open = false;
    }

    fn publish_source_presentation(&mut self, operation: Operation) -> bool {
        self.retire_source_presentation();
        self.state.source_presentation = self
            .core
            .source
            .current()
            .map(|media| Arc::new(SourcePresentation::new(media)));
        self.activate_cached_source(operation)
    }

    #[cfg(test)]
    pub(super) fn select_tool(&mut self, tool: Operation) {
        if !self.file_intake_enabled() {
            return;
        }
        let Some(previous_tool) = self.core.active_operation() else {
            return;
        };
        if previous_tool != tool {
            self.core.queue_confirmation = None;
            self.core.operation_action_error = None;
            if previous_tool == Operation::FrameCapture
                && let Some(session) = self.state.frame_capture.as_mut()
            {
                session.copy_status = FrameCopyStatus::Idle;
            }
            if previous_tool == Operation::FrameCapture {
                self.cancel_frame_capture_support();
            }
        }
        if self.activate_cached_source(tool) {
            self.core.select_operation(tool);
            if tool == Operation::FrameCapture {
                self.schedule_active_frame_capture_support();
            }
        }
    }

    fn activate_cached_source(&mut self, tool: Operation) -> bool {
        let Some(session_id) = self.core.source.session_id() else {
            self.close_source();
            return false;
        };
        let Some((source_identity, media)) = self.core.source.current_with_identity() else {
            self.close_source();
            return false;
        };

        match tool {
            Operation::AudioMix => {
                if !self
                    .state
                    .audio_mix
                    .as_ref()
                    .is_some_and(|session| session.session_id == session_id)
                {
                    self.state.audio_mix = Some(AudioMixToolState::new(session_id, media));
                }
            }
            Operation::VideoUpscale => {
                if !self
                    .state
                    .upscale
                    .as_ref()
                    .is_some_and(|session| session.session_id == session_id)
                {
                    self.state.upscale = Some(UpscaleToolState::new(session_id, media));
                }
            }
            Operation::Remux => {
                if !self
                    .state
                    .remux
                    .as_ref()
                    .is_some_and(|session| session.session_id == session_id)
                {
                    self.state.remux = Some(RemuxToolState::new(session_id, media));
                }
            }
            Operation::AudioExtract => {
                if !self
                    .state
                    .audio_extract
                    .as_ref()
                    .is_some_and(|session| session.session_id == session_id)
                {
                    self.state.audio_extract = Some(AudioExtractToolState::new(session_id, media));
                }
            }
            Operation::FrameCapture => {
                if !self
                    .state
                    .frame_capture
                    .as_ref()
                    .is_some_and(|session| session.session_id == session_id)
                {
                    self.state.frame_capture = Some(FrameCaptureToolState {
                        editor: FrameCaptureEditor::new(
                            session_id,
                            Some(source_identity.clone()),
                            media,
                        ),
                        picker_error: None,
                        picker_error_details_open: false,
                        media: media.clone(),
                    });
                }
                let position = self
                    .state
                    .frame_capture
                    .as_ref()
                    .and_then(|session| session.selection.valid_position(&session.media));
                if self.state.frame_capture.as_ref().is_some_and(|session| {
                    matches!(
                        session.preview.state(),
                        crate::frame_capture::preview::PreviewState::Idle
                    )
                }) {
                    self.update_frame_capture_position(
                        position,
                        PositionSettle::Debounced,
                        Instant::now(),
                    );
                }
            }
        }
        true
    }

    pub(super) fn load_media_path(&mut self, path: &Path) {
        let Some(tools) = self.ready_media_tools().cloned() else {
            return;
        };
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        self.core
            .source
            .start_inspection(tools, request, completion_wake(&self.egui_context));
    }

    pub(super) fn begin_media_path(&mut self, path: &Path) -> Option<MediaIntakeRequest> {
        if !self.file_intake_enabled() {
            return None;
        }

        self.core.queue_confirmation = None;
        self.core.operation_action_error = None;
        self.state.intake_error_details_open = false;
        match self.core.source.begin_load(path) {
            Some(MediaLoadStart::Reused) => {
                let operation = self.core.active_operation().unwrap_or(Operation::AudioMix);
                if self.activate_cached_source(operation) {
                    self.enter_workspace(operation);
                }
                None
            }
            Some(MediaLoadStart::Probe(request)) => Some(request),
            None => None,
        }
    }

    pub(super) fn poll_media_inspection(&mut self) {
        let settlement = self.core.source.poll();
        self.settle_source(settlement);
    }

    fn settle_source(&mut self, settlement: SourceSettlement) {
        match settlement {
            SourceSettlement::Published => {
                let operation = self.core.active_operation().unwrap_or(Operation::AudioMix);
                if self.publish_source_presentation(operation) {
                    self.enter_workspace(operation);
                }
            }
            SourceSettlement::Failed => self.state.intake_error_details_open = false,
            SourceSettlement::Unchanged => {}
        }
    }

    #[cfg(test)]
    pub(super) fn complete_media_probe(
        &mut self,
        request: MediaIntakeRequest,
        result: Result<MediaFile, MediaProbeError>,
    ) {
        let settlement = self.core.source.complete_probe(request, result);
        self.settle_source(settlement);
    }

    #[cfg(test)]
    pub(super) fn load_media_path_with_probe<F>(&mut self, path: &Path, probe: F)
    where
        F: FnOnce(&Path) -> Result<MediaFile, MediaProbeError>,
    {
        let Some(request) = self.begin_media_path(path) else {
            return;
        };
        let result = probe(request.path());
        self.complete_media_probe(request, result);
    }

    pub(super) fn file_intake_enabled(&self) -> bool {
        !self.core.is_closing()
            && !matches!(
                self.core.navigation(),
                crate::app::NavigationState::Queue {
                    origin: crate::app::ShellOrigin::BatchSetup
                } | crate::app::NavigationState::QueueEdit {
                    origin: QueueEditOrigin::Queue(crate::app::ShellOrigin::BatchSetup)
                }
            )
            && !self.core.source.is_inspecting()
            && !self.file_dialog.is_pending()
            && !self.copy_frame_active()
    }

    pub(super) fn restore_queued_job_workspace(&mut self, request: &QueuedOperation) -> bool {
        let operation = Operation::from(request);
        if !self.publish_source_presentation(operation) {
            return false;
        }
        self.hydrate_cached_source_operation(request);
        self.enter_workspace(operation);
        if operation == Operation::FrameCapture {
            self.schedule_active_frame_capture_support();
        }
        self.core.queue_confirmation = None;
        self.core.operation_action_error = None;
        true
    }
    fn hydrate_cached_source_operation(&mut self, operation: &QueuedOperation) {
        match operation {
            QueuedOperation::AudioMix(request) => {
                if let Some(session) = self.state.audio_mix.as_mut() {
                    session.selection = AudioMixDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::Upscale(request) => {
                if let Some(session) = self.state.upscale.as_mut() {
                    session.selection = UpscaleDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::Remux(request) => {
                if let Some(session) = self.state.remux.as_mut() {
                    session.selection = RemuxDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::AudioExtract(request) => {
                if let Some(session) = self.state.audio_extract.as_mut() {
                    session.selection = AudioExtractDraft::from_request(&session.media, request);
                }
            }
            QueuedOperation::FrameCapture(request) => {
                if let Some(session) = self.state.frame_capture.as_mut() {
                    session.selection = FrameCaptureDraft::from_request(&session.media, request);
                    let position = session.selection.valid_position(&session.media);
                    self.update_frame_capture_position(
                        position,
                        PositionSettle::Debounced,
                        Instant::now(),
                    );
                }
            }
        }
    }
}
