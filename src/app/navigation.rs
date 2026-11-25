use super::Application;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    AudioMix,
    VideoUpscale,
    Remux,
    AudioExtract,
    FrameCapture,
}

impl Operation {
    pub const ALL: [Self; 5] = [
        Self::AudioMix,
        Self::VideoUpscale,
        Self::Remux,
        Self::AudioExtract,
        Self::FrameCapture,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::AudioMix => "Audio Mix",
            Self::VideoUpscale => "Upscale",
            Self::Remux => "Remux",
            Self::AudioExtract => "Audio Extract",
            Self::FrameCapture => "Frame Capture",
        }
    }
}

impl From<&crate::queue::QueuedOperation> for Operation {
    fn from(operation: &crate::queue::QueuedOperation) -> Self {
        use crate::queue::QueuedOperation;
        match operation {
            QueuedOperation::AudioMix(_) => Self::AudioMix,
            QueuedOperation::Upscale(_) => Self::VideoUpscale,
            QueuedOperation::Remux(_) => Self::Remux,
            QueuedOperation::AudioExtract(_) => Self::AudioExtract,
            QueuedOperation::FrameCapture(_) => Self::FrameCapture,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavigationState {
    #[default]
    NoSource,
    Queue {
        origin: ShellOrigin,
    },
    Settings {
        origin: ShellOrigin,
    },
    QueueEdit {
        origin: QueueEditOrigin,
    },
    BatchSetup,
    BatchEdit,
    SourceWorkspace {
        active_operation: Operation,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellOrigin {
    NoSource,
    BatchSetup,
    SourceWorkspace { active_operation: Operation },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueEditOrigin {
    Queue(ShellOrigin),
    SourceWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOrigin {
    NoSource,
    Queue { origin: ShellOrigin },
    SourceWorkspace { active_operation: Operation },
}

impl NavigationState {
    fn retains_batch(self) -> bool {
        matches!(
            self,
            Self::BatchSetup
                | Self::BatchEdit
                | Self::Queue {
                    origin: ShellOrigin::BatchSetup
                }
                | Self::Settings {
                    origin: ShellOrigin::BatchSetup
                }
                | Self::QueueEdit {
                    origin: QueueEditOrigin::Queue(ShellOrigin::BatchSetup)
                }
        )
    }
}

impl Application {
    pub fn navigation(&self) -> NavigationState {
        self.navigation
    }

    pub(crate) fn navigation_revision(&self) -> u64 {
        self.navigation_revision
    }

    fn set_navigation(&mut self, navigation: NavigationState) {
        if self.navigation != navigation {
            if self.navigation.retains_batch() && !navigation.retains_batch() {
                self.retire_batch();
            }
            self.navigation_revision = self
                .navigation_revision
                .checked_add(1)
                .expect("Apollo navigation revision space is exhausted");
            self.navigation = navigation;
        }
    }

    pub fn active_operation(&self) -> Option<Operation> {
        match self.navigation {
            NavigationState::SourceWorkspace { active_operation } => Some(active_operation),
            _ => None,
        }
    }

    pub(crate) fn enter_workspace(&mut self, active_operation: Operation) {
        self.show_workspace(active_operation);
        self.source.clear_error();
    }

    pub(crate) fn show_workspace(&mut self, active_operation: Operation) {
        self.set_navigation(NavigationState::SourceWorkspace { active_operation });
    }

    pub(crate) fn show_no_source(&mut self) {
        self.set_navigation(NavigationState::NoSource);
    }

    fn shell_origin(&self) -> Option<ShellOrigin> {
        if self.is_closing() {
            return None;
        }
        match self.navigation {
            NavigationState::SourceWorkspace { active_operation } => {
                Some(ShellOrigin::SourceWorkspace { active_operation })
            }
            NavigationState::BatchSetup => Some(ShellOrigin::BatchSetup),
            NavigationState::NoSource => Some(ShellOrigin::NoSource),
            NavigationState::Queue { .. }
            | NavigationState::QueueEdit { .. }
            | NavigationState::Settings { .. }
            | NavigationState::BatchEdit => None,
        }
    }

    pub(crate) fn enter_queue(&mut self) -> bool {
        let Some(origin) = self.shell_origin() else {
            return false;
        };
        self.show_queue(origin);
        self.queue_action_error = None;
        self.queue_confirmation = None;
        self.operation_action_error = None;
        true
    }

    pub(crate) fn show_queue(&mut self, origin: ShellOrigin) {
        self.set_navigation(NavigationState::Queue { origin });
    }

    pub(crate) fn queue_origin(&self) -> Option<ShellOrigin> {
        match self.navigation {
            NavigationState::Queue { origin } => Some(origin),
            _ => None,
        }
    }

    pub(crate) fn return_from_queue(&mut self) {
        if let Some(origin) = self.queue_origin() {
            self.restore_shell_origin(origin);
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn enter_settings(&mut self) -> bool {
        let Some(origin) = self.shell_origin() else {
            return false;
        };
        self.set_navigation(NavigationState::Settings { origin });
        true
    }

    #[cfg(any(windows, test))]
    pub(crate) fn return_from_settings(&mut self) {
        if let NavigationState::Settings { origin } = self.navigation {
            self.restore_shell_origin(origin);
        }
    }

    fn restore_shell_origin(&mut self, origin: ShellOrigin) {
        match origin {
            ShellOrigin::NoSource => self.show_no_source(),
            ShellOrigin::BatchSetup => self.show_batch_setup(),
            ShellOrigin::SourceWorkspace { active_operation } => {
                self.show_workspace(active_operation)
            }
        }
    }

    /// Switching operations changes navigation, not the accepted source or its drafts.
    #[cfg(any(windows, test))]
    pub(crate) fn select_operation(&mut self, operation: Operation) -> bool {
        let Some(previous) = self.active_operation() else {
            return false;
        };
        if self.is_closing() || previous == operation || self.source.current().is_none() {
            return false;
        }
        self.queue_confirmation = None;
        self.operation_action_error = None;
        self.show_workspace(operation);
        true
    }

    /// An inspection can finish while Queue or Settings is visible. Update the
    /// return origin instead of taking the user away from that route.
    #[cfg(any(windows, test))]
    pub(crate) fn source_published(&mut self) {
        let workspace = |origin| match origin {
            ShellOrigin::NoSource => ShellOrigin::SourceWorkspace {
                active_operation: Operation::AudioMix,
            },
            other => other,
        };
        match self.navigation {
            NavigationState::Queue { origin } => self.show_queue(workspace(origin)),
            NavigationState::Settings { origin } => {
                self.set_navigation(NavigationState::Settings {
                    origin: workspace(origin),
                })
            }
            NavigationState::NoSource | NavigationState::SourceWorkspace { .. } => {
                self.enter_workspace(self.active_operation().unwrap_or(Operation::AudioMix));
            }
            _ => {}
        }
    }

    pub(crate) fn can_enter_queue_edit(&self, origin: QueueEditOrigin) -> bool {
        match (self.navigation, origin) {
            (NavigationState::Queue { origin: current }, QueueEditOrigin::Queue(requested)) => {
                current == requested
            }
            (NavigationState::SourceWorkspace { .. }, QueueEditOrigin::SourceWorkspace) => true,
            _ => false,
        }
    }

    pub(crate) fn enter_queue_edit(&mut self, origin: QueueEditOrigin) {
        self.set_navigation(NavigationState::QueueEdit { origin });
        self.queue_confirmation = None;
        self.operation_action_error = None;
    }

    pub(crate) fn batch_origin(&self) -> Option<BatchOrigin> {
        match self.navigation {
            NavigationState::NoSource => Some(BatchOrigin::NoSource),
            NavigationState::Queue {
                origin: ShellOrigin::BatchSetup,
            }
            | NavigationState::QueueEdit {
                origin: QueueEditOrigin::Queue(ShellOrigin::BatchSetup),
            } => None,
            NavigationState::Queue { origin }
            | NavigationState::QueueEdit {
                origin: QueueEditOrigin::Queue(origin),
            } => Some(BatchOrigin::Queue { origin }),
            NavigationState::SourceWorkspace { active_operation } => {
                Some(BatchOrigin::SourceWorkspace { active_operation })
            }
            NavigationState::QueueEdit {
                origin: QueueEditOrigin::SourceWorkspace,
            }
            | NavigationState::Settings { .. }
            | NavigationState::BatchSetup
            | NavigationState::BatchEdit => None,
        }
    }

    pub(crate) fn show_batch_setup(&mut self) {
        self.set_navigation(NavigationState::BatchSetup);
        self.refresh_batch_admission();
    }

    pub(crate) fn show_batch_edit(&mut self) {
        self.set_navigation(NavigationState::BatchEdit);
    }

    pub(crate) fn return_from_batch(&mut self, origin: BatchOrigin) {
        match origin {
            BatchOrigin::NoSource => self.show_no_source(),
            BatchOrigin::Queue { origin } => self.show_queue(origin),
            BatchOrigin::SourceWorkspace { active_operation } => {
                self.show_workspace(active_operation)
            }
        }
    }

    pub(crate) fn complete_batch(&mut self, origin: BatchOrigin) {
        self.show_queue(match origin {
            BatchOrigin::NoSource => ShellOrigin::NoSource,
            BatchOrigin::Queue { origin } => origin,
            BatchOrigin::SourceWorkspace { active_operation } => {
                ShellOrigin::SourceWorkspace { active_operation }
            }
        });
    }

    pub(crate) fn close_source(&mut self) {
        self.show_no_source();
        self.source.close();
        self.frame_work.cancel_copy();
        self.frame_work.cancel_preview();
        self.frame_work.cancel_index();
        self.queue_confirmation = None;
        self.operation_action_error = None;
    }
}

#[cfg(test)]
mod tests;
