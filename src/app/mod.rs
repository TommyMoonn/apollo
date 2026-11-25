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
    pub(crate) operation_action_error: Option<(Operation, crate::media::error::MediaError)>,
}

impl Application {
    pub(crate) fn with_settings(settings: crate::settings::AppSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    pub(crate) fn queue_start_allowed(&self) -> bool {
        !self.frame_work.copy_active() && self.discovery.ready().is_some()
    }

    pub(crate) fn clipboard_start_allowed(&self) -> bool {
        !self.frame_work.copy_active()
            && !self.queue.scheduler().is_started()
            && !self.queue.scheduler().is_running()
            && self.discovery.ready().is_some()
    }
}
