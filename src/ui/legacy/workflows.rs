//! Shared legacy route and lifecycle presentation helpers.

use super::{ApolloApp, completion_wake};
use crate::app::NavigationState;
use crate::audio_extract::execution::AudioExtractExecutionState;
use crate::audio_mix::execution::AudioMixExecutionState;
use crate::frame_capture::execution::FrameCaptureExecutionState;
use crate::media::tools::MediaTools;
use crate::remux::execution::RemuxExecutionState;
use crate::upscale::execution::UpscaleExecutionState;

impl ApolloApp {
    pub(super) fn resolve_navigation(&mut self) -> Option<NavigationState> {
        match self.core.navigation() {
            NavigationState::BatchEdit if self.core.batch_edit.is_none() => {
                self.batch_editor_presentation = None;
                self.core.show_batch_setup();
                None
            }
            NavigationState::BatchSetup if self.core.batch_setup.is_none() => {
                self.core.show_no_source();
                None
            }
            NavigationState::QueueEdit { .. } if self.core.queue_edit().is_none() => {
                self.finish_queue_edit();
                None
            }
            NavigationState::SourceWorkspace { .. }
                if self.core.source.current().is_none()
                    || self.state.source_presentation.is_none() =>
            {
                self.close_source();
                Some(self.core.navigation())
            }
            route => Some(route),
        }
    }

    pub(super) fn ready_media_tools(&self) -> Option<&MediaTools> {
        self.core.discovery.ready()
    }

    pub(super) fn retry_media_tools(&mut self) {
        if !self.core.discovery.is_pending() {
            self.tools_error_details_open = false;
            self.core
                .discovery
                .start(completion_wake(&self.egui_context));
        }
    }
}

pub(super) fn shows_audio_mix_lifecycle(state: &AudioMixExecutionState) -> bool {
    !matches!(state, AudioMixExecutionState::Idle)
}

pub(super) fn shows_upscale_lifecycle(state: &UpscaleExecutionState) -> bool {
    !matches!(state, UpscaleExecutionState::Idle)
}

pub(super) fn shows_remux_lifecycle(state: &RemuxExecutionState) -> bool {
    !matches!(state, RemuxExecutionState::Idle)
}

pub(super) fn shows_audio_extract_lifecycle(state: &AudioExtractExecutionState) -> bool {
    !matches!(state, AudioExtractExecutionState::Idle)
}

pub(super) fn shows_frame_capture_lifecycle(state: &FrameCaptureExecutionState) -> bool {
    !matches!(state, FrameCaptureExecutionState::Idle)
}
