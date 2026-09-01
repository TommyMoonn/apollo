use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use apollo::media::process::{MediaProcessEvent, MediaProcessWake};
use egui::Context;

const MIN_PROGRESS_REPAINT_INTERVAL: Duration = Duration::from_millis(34);

enum RepaintEvent {
    Begin(u64),
    Progress(u64),
    Terminal(u64),
}

pub(crate) struct MediaProgressRepaintOwner {
    sender: Sender<RepaintEvent>,
    next_operation_id: u64,
}

impl MediaProgressRepaintOwner {
    pub(crate) fn new(context: Context) -> Self {
        Self::new_with_repaint(move || context.request_repaint())
    }

    fn new_with_repaint(request_repaint: impl Fn() + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("apollo-progress-repaint".to_owned())
            .spawn(move || run_repaint_worker(request_repaint, receiver))
            .expect("Apollo progress repaint worker must start");
        Self {
            sender,
            next_operation_id: 1,
        }
    }

    pub(crate) fn begin(&mut self) -> MediaProcessWake {
        let operation_id = self.next_operation_id;
        self.next_operation_id = self
            .next_operation_id
            .checked_add(1)
            .expect("Apollo progress repaint operation identifier space is exhausted");
        let _ = self.sender.send(RepaintEvent::Begin(operation_id));
        let sender = self.sender.clone();
        MediaProcessWake::new(move |event| {
            let event = match event {
                MediaProcessEvent::Progress => RepaintEvent::Progress(operation_id),
                MediaProcessEvent::Terminal => RepaintEvent::Terminal(operation_id),
            };
            let _ = sender.send(event);
        })
    }
}

fn run_repaint_worker(request_repaint: impl Fn(), receiver: Receiver<RepaintEvent>) {
    let mut active_operation = None;
    let mut last_progress_repaint: Option<Instant> = None;
    let mut progress_pending = false;

    loop {
        let event = if progress_pending {
            let elapsed = last_progress_repaint
                .map(|last| last.elapsed())
                .unwrap_or(MIN_PROGRESS_REPAINT_INTERVAL);
            let remaining = MIN_PROGRESS_REPAINT_INTERVAL.saturating_sub(elapsed);
            match receiver.recv_timeout(remaining) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => {
                    request_repaint();
                    last_progress_repaint = Some(Instant::now());
                    progress_pending = false;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match receiver.recv() {
                Ok(event) => event,
                Err(_) => return,
            }
        };

        match event {
            RepaintEvent::Begin(operation_id) => {
                active_operation = Some(operation_id);
                last_progress_repaint = None;
                progress_pending = false;
            }
            RepaintEvent::Progress(operation_id) if active_operation == Some(operation_id) => {
                let repaint_due = last_progress_repaint
                    .is_none_or(|last| last.elapsed() >= MIN_PROGRESS_REPAINT_INTERVAL);
                if repaint_due {
                    request_repaint();
                    last_progress_repaint = Some(Instant::now());
                    progress_pending = false;
                } else {
                    progress_pending = true;
                }
            }
            RepaintEvent::Terminal(operation_id) if active_operation == Some(operation_id) => {
                active_operation = None;
                progress_pending = false;
                request_repaint();
            }
            RepaintEvent::Progress(_) | RepaintEvent::Terminal(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use apollo::media::process::MediaProcessEvent;

    use super::{MIN_PROGRESS_REPAINT_INTERVAL, MediaProgressRepaintOwner};

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    fn owner() -> (MediaProgressRepaintOwner, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        let owner = MediaProgressRepaintOwner::new_with_repaint(move || {
            let _ = sender.send(());
        });
        (owner, receiver)
    }

    #[test]
    fn burst_progress_is_coalesced_and_terminal_repaints_immediately() {
        let (mut owner, repaints) = owner();
        let wake = owner.begin();

        wake.notify(MediaProcessEvent::Progress);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        for _ in 0..20 {
            wake.notify(MediaProcessEvent::Progress);
        }
        assert!(
            repaints
                .recv_timeout(MIN_PROGRESS_REPAINT_INTERVAL / 2)
                .is_err()
        );
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(repaints.try_recv().is_err());

        wake.notify(MediaProcessEvent::Terminal);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(
            repaints
                .recv_timeout(MIN_PROGRESS_REPAINT_INTERVAL * 2)
                .is_err()
        );
    }

    #[test]
    fn isolated_progress_stays_at_its_natural_event_rate() {
        let (mut owner, repaints) = owner();
        let wake = owner.begin();

        wake.notify(MediaProcessEvent::Progress);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(
            repaints
                .recv_timeout(MIN_PROGRESS_REPAINT_INTERVAL * 2)
                .is_err()
        );

        wake.notify(MediaProcessEvent::Progress);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(
            repaints
                .recv_timeout(MIN_PROGRESS_REPAINT_INTERVAL * 2)
                .is_err()
        );
    }

    #[test]
    fn terminal_event_cancels_a_pending_progress_repaint() {
        let (mut owner, repaints) = owner();
        let wake = owner.begin();

        wake.notify(MediaProcessEvent::Progress);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        wake.notify(MediaProcessEvent::Progress);
        wake.notify(MediaProcessEvent::Terminal);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(
            repaints
                .recv_timeout(MIN_PROGRESS_REPAINT_INTERVAL * 2)
                .is_err()
        );
    }

    #[test]
    fn newer_operation_ignores_stale_progress_and_terminal_events() {
        let (mut owner, repaints) = owner();
        let stale = owner.begin();
        let current = owner.begin();

        stale.notify(MediaProcessEvent::Progress);
        stale.notify(MediaProcessEvent::Terminal);
        assert!(
            repaints
                .recv_timeout(MIN_PROGRESS_REPAINT_INTERVAL * 2)
                .is_err()
        );

        current.notify(MediaProcessEvent::Progress);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
        current.notify(MediaProcessEvent::Terminal);
        repaints.recv_timeout(TEST_TIMEOUT).unwrap();
    }
}
