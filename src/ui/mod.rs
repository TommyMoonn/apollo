pub mod audio_extract;
pub mod audio_extract_progress;
pub mod audio_mix;
pub mod audio_mix_progress;
pub mod batch;
pub mod file_intake;
pub mod fonts;
pub mod frame_capture;
pub mod frame_capture_progress;
pub mod icon;
pub mod layout;
pub mod media_error;
mod media_lifecycle;
mod output_name;
pub mod queue;
pub mod remux;
pub mod remux_progress;
pub mod source_presentation;
pub mod theme;
pub mod upscale;
pub mod upscale_progress;
pub mod widgets;
pub mod workspace;

use crate::media::error::MediaError;
use crate::queue::JobId;

pub const ADD_TO_QUEUE_LABEL: &str = "Add to queue";
pub const ADDED_TO_QUEUE_LABEL: &str = "Added to queue";
pub const SAVE_QUEUE_CHANGES_LABEL: &str = "Save changes";
pub const CANCEL_QUEUE_EDIT_LABEL: &str = "Cancel edit";
pub const APPLY_BATCH_DEFAULTS_LABEL: &str = "Apply defaults";
pub const SAVE_BATCH_OVERRIDE_LABEL: &str = "Save override";
pub const CANCEL_BATCH_EDIT_LABEL: &str = "Cancel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationSurfaceMode<'a> {
    Immediate {
        start_allowed: bool,
        queue_confirmation: bool,
        action_error: Option<&'a MediaError>,
    },
    QueueEdit,
    BatchDefaults {
        save_allowed: bool,
    },
    BatchOverride,
}

impl OperationSurfaceMode<'_> {
    pub const fn edit_labels(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Immediate { .. } => None,
            Self::QueueEdit => Some((SAVE_QUEUE_CHANGES_LABEL, CANCEL_QUEUE_EDIT_LABEL)),
            Self::BatchDefaults { .. } => {
                Some((APPLY_BATCH_DEFAULTS_LABEL, CANCEL_BATCH_EDIT_LABEL))
            }
            Self::BatchOverride => Some((SAVE_BATCH_OVERRIDE_LABEL, CANCEL_BATCH_EDIT_LABEL)),
        }
    }
}

impl<'a> OperationSurfaceMode<'a> {
    pub const fn inline_action_error(self) -> Option<&'a MediaError> {
        match self {
            Self::Immediate { action_error, .. } => action_error,
            _ => None,
        }
    }
}

impl From<bool> for OperationSurfaceMode<'_> {
    fn from(start_allowed: bool) -> Self {
        Self::Immediate {
            start_allowed,
            queue_confirmation: false,
            action_error: None,
        }
    }
}

pub(crate) fn show_queue_confirmation(ui: &mut egui::Ui, visible: bool) {
    if visible {
        let _ = widgets::inline_status(ui, ADDED_TO_QUEUE_LABEL, widgets::InlineStatusKind::Status);
    }
}

pub fn queue_count_label(count: usize) -> String {
    format!("Queue ({count})")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAction {
    ChangeFile,
    CloseSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueAction {
    AddFiles,
    Start,
    CancelCurrent,
    StopAfterCurrent,
    CancelQueue,
    Edit(JobId),
    MoveEarlier(JobId),
    MoveLater(JobId),
    Remove(JobId),
    OpenFolder(JobId),
    ClearFinished,
    Back,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessExecutionArbitration {
    immediate_running: bool,
    queue_running: bool,
}

impl ProcessExecutionArbitration {
    pub fn new(immediate_running: bool, queue_running: bool) -> Self {
        Self {
            immediate_running,
            queue_running,
        }
    }

    pub fn immediate_start_allowed(self) -> bool {
        !self.queue_running
    }

    pub fn queue_start_allowed(self) -> bool {
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
