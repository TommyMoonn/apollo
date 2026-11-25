//! Renderer-independent application state. Native adapters coordinate the existing
//! feature workflows until their dedicated owners are extracted.

pub(crate) mod dialogs;
pub(crate) mod discovery;
pub(crate) mod editor;
pub(crate) mod queue;
pub use queue::QueueAction;
pub(crate) mod frame_capture;
mod navigation;
pub(crate) mod source;
pub use navigation::{BatchOrigin, NavigationState, Operation, QueueEditOrigin, QueueOrigin};

use crate::{
    audio_extract::execution::AudioExtractExecutor,
    audio_mix::execution::AudioMixExecutor,
    frame_capture::execution::FrameCaptureExecutor,
    media::{error::MediaError, execution::MediaExecutionState},
    remux::execution::RemuxExecutor,
    upscale::execution::UpscaleExecutor,
};

#[derive(Default)]
pub struct Application {
    navigation: NavigationState,
    navigation_revision: u64,
    pub(crate) settings: crate::settings::AppSettings,
    pub(crate) discovery: discovery::MediaToolsDiscovery,
    pub(crate) source: source::SourceSession,
    pub(crate) frame_work: frame_capture::FrameCaptureWork,
    pub(crate) audio_mix_executor: AudioMixExecutor,
    pub(crate) upscale_executor: UpscaleExecutor,
    pub(crate) remux_executor: RemuxExecutor,
    pub(crate) audio_extract_executor: AudioExtractExecutor,
    pub(crate) frame_capture_executor: FrameCaptureExecutor,
    pub(crate) queue: queue::QueueWorkflow,
    queue_edit: Option<queue::QueueEditState>,
    pub(crate) queue_confirmation: Option<Operation>,
    pub(crate) operation_action_error: Option<(Operation, MediaError)>,
}

impl Application {
    pub(crate) fn with_settings(settings: crate::settings::AppSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    pub(crate) fn media_operation_running(&self) -> bool {
        self.audio_mix_executor.is_running()
            || self.upscale_executor.is_running()
            || self.remux_executor.is_running()
            || self.audio_extract_executor.is_running()
            || self.frame_capture_executor.is_running()
    }

    pub(crate) fn executors_idle(&self) -> bool {
        file_intake_enabled(
            self.audio_mix_executor.state(),
            self.upscale_executor.state(),
            self.remux_executor.state(),
            self.audio_extract_executor.state(),
            self.frame_capture_executor.state(),
        )
    }

    pub(crate) fn execution_arbitration(&self) -> ProcessExecutionArbitration {
        ProcessExecutionArbitration::new(
            self.media_operation_running() || self.frame_work.copy_active(),
            self.queue.scheduler().is_running(),
        )
    }
}

pub(crate) fn file_intake_enabled(
    audio_state: &MediaExecutionState,
    upscale_state: &MediaExecutionState,
    remux_state: &MediaExecutionState,
    audio_extract_state: &MediaExecutionState,
    frame_capture_state: &MediaExecutionState,
) -> bool {
    // Done/Back owns terminal settlement. Intake resumes only after every executor returns to Idle.
    matches!(audio_state, MediaExecutionState::Idle)
        && matches!(upscale_state, MediaExecutionState::Idle)
        && matches!(remux_state, MediaExecutionState::Idle)
        && matches!(audio_extract_state, MediaExecutionState::Idle)
        && matches!(frame_capture_state, MediaExecutionState::Idle)
}

pub(crate) fn operation_workspace_visible(
    execution_state: &MediaExecutionState,
    has_ready_session: bool,
) -> bool {
    matches!(execution_state, MediaExecutionState::Idle) && has_ready_session
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessExecutionArbitration {
    immediate_running: bool,
    queue_running: bool,
}

impl ProcessExecutionArbitration {
    fn new(immediate_running: bool, queue_running: bool) -> Self {
        Self {
            immediate_running,
            queue_running,
        }
    }

    pub(crate) fn immediate_start_allowed(self) -> bool {
        !self.queue_running
    }

    pub(crate) fn queue_start_allowed(self) -> bool {
        !self.immediate_running
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessExecutionArbitration;

    #[test]
    fn process_execution_owners_block_only_the_opposite_start_path() {
        let idle = ProcessExecutionArbitration::new(false, false);
        assert!(idle.immediate_start_allowed());
        assert!(idle.queue_start_allowed());

        let immediate = ProcessExecutionArbitration::new(true, false);
        assert!(immediate.immediate_start_allowed());
        assert!(!immediate.queue_start_allowed());

        let queue = ProcessExecutionArbitration::new(false, true);
        assert!(!queue.immediate_start_allowed());
        assert!(queue.queue_start_allowed());
    }
}
