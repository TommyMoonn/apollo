//! Shared legacy route and lifecycle presentation helpers.

use super::{ApolloApp, completion_wake};
use crate::app::NavigationState;
use crate::media::tools::MediaTools;

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
