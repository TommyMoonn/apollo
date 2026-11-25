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
        origin: QueueOrigin,
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
pub enum QueueOrigin {
    NoSource,
    SourceWorkspace { active_operation: Operation },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueEditOrigin {
    Queue(QueueOrigin),
    SourceWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOrigin {
    NoSource,
    Queue { origin: QueueOrigin },
    SourceWorkspace { active_operation: Operation },
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

    pub(crate) fn enter_queue(&mut self) {
        let origin = match self.navigation {
            NavigationState::SourceWorkspace { active_operation } => {
                QueueOrigin::SourceWorkspace { active_operation }
            }
            NavigationState::Queue { origin }
            | NavigationState::QueueEdit {
                origin: QueueEditOrigin::Queue(origin),
            } => origin,
            NavigationState::NoSource
            | NavigationState::QueueEdit {
                origin: QueueEditOrigin::SourceWorkspace,
            }
            | NavigationState::BatchSetup
            | NavigationState::BatchEdit => QueueOrigin::NoSource,
        };
        self.show_queue(origin);
        self.source.clear_error();
        self.queue_confirmation = None;
        self.operation_action_error = None;
    }

    pub(crate) fn show_queue(&mut self, origin: QueueOrigin) {
        self.set_navigation(NavigationState::Queue { origin });
    }

    pub(crate) fn queue_origin(&self) -> Option<QueueOrigin> {
        match self.navigation {
            NavigationState::Queue { origin } => Some(origin),
            _ => None,
        }
    }

    pub(crate) fn return_from_queue(&mut self) {
        if let Some(origin) = self.queue_origin() {
            match origin {
                QueueOrigin::NoSource => self.show_no_source(),
                QueueOrigin::SourceWorkspace { active_operation } => {
                    self.show_workspace(active_operation)
                }
            }
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
            | NavigationState::BatchSetup
            | NavigationState::BatchEdit => None,
        }
    }

    pub(crate) fn show_batch_setup(&mut self) {
        self.set_navigation(NavigationState::BatchSetup);
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
            BatchOrigin::NoSource => QueueOrigin::NoSource,
            BatchOrigin::Queue { origin } => origin,
            BatchOrigin::SourceWorkspace { active_operation } => {
                QueueOrigin::SourceWorkspace { active_operation }
            }
        });
    }

    pub(crate) fn close_source(&mut self) {
        self.show_no_source();
        self.source.close();
        self.queue_confirmation = None;
        self.operation_action_error = None;
    }
}

#[cfg(test)]
mod tests;
