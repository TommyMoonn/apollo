//! Foreground settlement of renderer-independent workers. Workers publish before waking.
use super::{Application, Operation, source::SourceSettlement};
use crate::media::process::MediaProcessWake;

impl Application {
    /// Called for a worker wake, never from rendering. Frame editor and native dialog
    /// results retain their adapter-specific routing until those surfaces are migrated.
    pub(crate) fn poll_background(&mut self, wake: impl FnMut() -> MediaProcessWake) -> bool {
        debug_assert!(!self.is_closing());
        let had_tools = self.discovery.ready().is_some();
        let discovery_finished = self.discovery.poll();
        let tools_changed = discovery_finished || had_tools != self.discovery.ready().is_some();
        if discovery_finished {
            self.refresh_batch_capabilities();
        }
        let queue_changed = self.poll_media_queue(wake);
        let settlement = self.source.poll();
        if settlement == SourceSettlement::Published {
            self.frame_work.cancel_copy();
            self.frame_work.cancel_preview();
            self.frame_work.cancel_index();
            self.enter_workspace(self.active_operation().unwrap_or(Operation::AudioMix));
        }
        let batch_changed = self.poll_batch_inspection();
        tools_changed || queue_changed || settlement != SourceSettlement::Unchanged || batch_changed
    }
}

/// A level-triggered wake, not a result channel. Multiple notifications may share
/// one slot; every owner drains its own results on the foreground thread.
pub(crate) fn wake_channel() -> (async_channel::Sender<()>, async_channel::Receiver<()>) {
    async_channel::bounded(1)
}

pub(crate) fn notify(sender: &async_channel::Sender<()>) {
    // Full means a wake is already pending. Closed means the root was released.
    let _ = sender.try_send(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::tools::{MediaTools, MediaToolsError};
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn wake_is_bounded_and_safe_after_receiver_drop() {
        let (sender, receiver) = wake_channel();
        for _ in 0..100 {
            notify(&sender);
        }
        assert_eq!(receiver.len(), 1);
        assert_eq!(receiver.try_recv(), Ok(()));
        notify(&sender);
        assert_eq!(receiver.try_recv(), Ok(()));
        drop(receiver);
        notify(&sender);
        assert!(sender.is_closed());
    }

    #[test]
    fn discovery_failure_and_retry_are_settled_without_navigation_or_polling() {
        let mut app = Application::default();
        let (sender, receiver) = wake_channel();
        let (notified, notification) = mpsc::channel();
        let start = |app: &mut Application, success: bool| {
            let sender = sender.clone();
            let notified = notified.clone();
            app.discovery.start_with(
                move || {
                    notify(&sender);
                    let _ = notified.send(());
                },
                move || {
                    if success {
                        Ok(MediaTools::new("ffmpeg", "ffprobe"))
                    } else {
                        Err(MediaToolsError::MissingFfprobe)
                    }
                },
                |_| Default::default(),
            )
        };
        let timeout = Duration::from_secs(10);
        assert!(start(&mut app, false));
        notification.recv_timeout(timeout).unwrap();
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(app.poll_background(|| panic!("idle queue must not launch")));
        assert!(!app.poll_background(|| panic!("idle queue must not launch")));
        assert!(app.discovery.ready().is_none());
        assert!(start(&mut app, true));
        notification.recv_timeout(timeout).unwrap();
        notification.recv_timeout(timeout).unwrap();
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(app.poll_background(|| panic!("idle queue must not launch")));
        assert!(app.discovery.ready().is_some());
        assert!(!app.discovery.is_pending());
        assert!(!app.poll_background(|| panic!("idle queue must not launch")));
    }
}
