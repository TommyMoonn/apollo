//! Renderer-independent application composition and workflow state.

pub(crate) mod batch;
pub use batch::{BatchAction, BatchAvailability, BatchItemId, BatchReadiness};
pub(crate) mod dialogs;
pub(crate) mod discovery;
pub(crate) mod editor;
pub(crate) mod queue;
pub use queue::QueueAction;
pub(crate) mod frame_capture;
mod navigation;
pub(crate) mod source;
pub use navigation::{BatchOrigin, NavigationState, Operation, QueueEditOrigin, QueueOrigin};

#[derive(Default)]
pub struct Application {
    navigation: NavigationState,
    shutdown: Option<Shutdown>,
    navigation_revision: u64,
    pub(crate) settings: crate::settings::AppSettings,
    pub(crate) discovery: discovery::MediaToolsDiscovery,
    pub(crate) source: source::SourceSession,
    pub(crate) frame_work: frame_capture::FrameCaptureWork,
    pub(crate) queue: queue::QueueWorkflow,
    queue_edit: Option<queue::QueueEditState>,
    pub(crate) batch_setup: Option<batch::BatchSetupState>,
    pub(crate) batch_edit: Option<batch::BatchEditState>,
    batch_inspection: Option<batch::BatchInspectionSession>,
    pub(crate) queue_confirmation: Option<Operation>,
    pub(crate) queue_action_error: Option<String>,
    pub(crate) operation_action_error: Option<(Operation, crate::media::error::MediaError)>,
}

// Retain the active identity to report its actual terminal outcome, including a
// completion or failure that raced the cancellation request.
struct Shutdown {
    active_job: Option<crate::queue::JobId>,
}

impl Application {
    pub(crate) fn is_closing(&self) -> bool {
        self.shutdown.is_some()
    }

    pub(crate) fn begin_shutdown(&mut self) -> bool {
        if self.is_closing() {
            return false;
        }
        self.shutdown = Some(Shutdown {
            active_job: self.queue.scheduler().active_job(),
        });
        self.queue.cancel();
        self.frame_work.begin_shutdown();
        self.cancel_batch_inspection_for_shutdown();
        true
    }

    pub(crate) fn poll_shutdown(
        &mut self,
        wake: impl FnMut() -> crate::media::process::MediaProcessWake,
    ) -> bool {
        debug_assert!(self.is_closing());
        self.poll_media_queue(wake);
        self.discovery.poll();
        self.source.poll();
        self.poll_batch_inspection_for_shutdown();
        let frame_settled = self.frame_work.poll_shutdown();
        !self.queue.scheduler().is_running()
            && frame_settled
            && !self.source.is_inspecting()
            && !self.batch_inspecting()
            && !self.discovery.is_pending()
    }

    pub(crate) fn shutdown_failure(&self) -> Option<&crate::queue::QueueJobFailure> {
        let id = self.shutdown.as_ref()?.active_job?;
        self.queue.scheduler().jobs().iter().find_map(|job| {
            if job.id() == id
                && let crate::queue::QueueJobState::Failed(error) = job.state()
            {
                Some(error)
            } else {
                None
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn with_settings(settings: crate::settings::AppSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    pub(crate) fn queue_start_allowed(&self) -> bool {
        !self.is_closing() && !self.frame_work.copy_active() && self.discovery.ready().is_some()
    }

    pub(crate) fn clipboard_start_allowed(&self) -> bool {
        !self.is_closing()
            && !self.frame_work.copy_active()
            && !self.queue.scheduler().is_started()
            && !self.queue.scheduler().is_running()
            && self.discovery.ready().is_some()
    }
}
