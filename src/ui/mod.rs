pub mod audio_extract;
pub mod audio_mix;
pub mod batch;
#[cfg(windows)]
pub mod components;
pub mod design;
pub mod file_intake;
pub mod fonts;
pub mod frame_capture;
pub mod layout;
pub mod legacy;
pub mod media_error;
mod media_lifecycle;
pub mod queue;
pub mod remux;
#[cfg(windows)]
pub mod root;
pub mod source_presentation;
pub mod theme;
pub mod upscale;
pub mod widgets;
pub mod workspace;

use crate::media::error::MediaError;

/// Default and minimum drawable window dimensions in logical pixels.
pub const REFERENCE_CONTENT_DIMENSIONS: (f32, f32) = (730.0, 489.0);
pub const MINIMUM_CONTENT_DIMENSIONS: (f32, f32) = (400.0, 260.0);

pub const PROCESS_NOW_LABEL: &str = "Process now";
pub const ADD_TO_QUEUE_LABEL: &str = "Add to queue";
pub const ADDED_TO_QUEUE_LABEL: &str = "Added to queue";
pub const SAVE_QUEUE_CHANGES_LABEL: &str = "Save changes";
pub const CANCEL_QUEUE_EDIT_LABEL: &str = "Cancel edit";
pub const APPLY_BATCH_DEFAULTS_LABEL: &str = "Apply settings";
pub const SAVE_BATCH_OVERRIDE_LABEL: &str = "Save changes";
pub const CANCEL_BATCH_EDIT_LABEL: &str = "Cancel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationSurfaceMode<'a> {
    Normal {
        process_now_allowed: bool,
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
            Self::Normal { .. } => None,
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
            Self::Normal { action_error, .. } => action_error,
            _ => None,
        }
    }
}

impl From<bool> for OperationSurfaceMode<'_> {
    fn from(process_now_allowed: bool) -> Self {
        Self::Normal {
            process_now_allowed,
            queue_confirmation: false,
            action_error: None,
        }
    }
}

/// Normal queued forms share the same file-submission controls and action priority.
pub(crate) fn show_queued_submission_actions(
    ui: &mut egui::Ui,
    process_now_label: &str,
    process_now_allowed: bool,
    request_valid: bool,
    queue_confirmation: bool,
) -> Option<crate::app::queue::QueueSubmission> {
    use crate::app::queue::QueueSubmission;
    let mut action = None;
    if process_now_allowed
        && widgets::primary_button_enabled(ui, process_now_label, request_valid).clicked()
    {
        action = Some(QueueSubmission::ProcessNow);
    }
    let add = if process_now_allowed {
        widgets::secondary_button_enabled(ui, ADD_TO_QUEUE_LABEL, request_valid)
    } else {
        widgets::primary_button_enabled(ui, ADD_TO_QUEUE_LABEL, request_valid)
    };
    if add.clicked() {
        action = Some(QueueSubmission::AddToQueue);
    }
    show_queue_confirmation(ui, queue_confirmation);
    action
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
