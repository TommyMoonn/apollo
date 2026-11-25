use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use crate::media::process::{MediaProcessEvent, MediaProcessWake};

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
    #[allow(
        dead_code,
        reason = "Background notification integration resumes in U01.3"
    )]
    pub(crate) fn new(request_repaint: impl Fn() + Send + 'static) -> Self {
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

#[derive(Default)]
struct RepaintSchedule {
    active_operation: Option<u64>,
    last_progress_repaint: Option<Instant>,
    progress_pending: bool,
}

impl RepaintSchedule {
    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.progress_pending.then(|| {
            let elapsed = self
                .last_progress_repaint
                .map(|last| now.saturating_duration_since(last))
                .unwrap_or(MIN_PROGRESS_REPAINT_INTERVAL);
            MIN_PROGRESS_REPAINT_INTERVAL.saturating_sub(elapsed)
        })
    }

    fn on_event(&mut self, event: RepaintEvent, now: Instant) -> bool {
        match event {
            RepaintEvent::Begin(id) => {
                *self = Self {
                    active_operation: Some(id),
                    ..Self::default()
                };
                false
            }
            RepaintEvent::Progress(id) if self.active_operation == Some(id) => {
                let due = self.last_progress_repaint.is_none_or(|last| {
                    now.saturating_duration_since(last) >= MIN_PROGRESS_REPAINT_INTERVAL
                });
                self.progress_pending = !due;
                due
            }
            RepaintEvent::Terminal(id) if self.active_operation == Some(id) => {
                self.active_operation = None;
                self.progress_pending = false;
                true
            }
            RepaintEvent::Progress(_) | RepaintEvent::Terminal(_) => false,
        }
    }

    fn repainted(&mut self, now: Instant) {
        self.last_progress_repaint = Some(now);
        self.progress_pending = false;
    }
}

fn run_repaint_worker(request_repaint: impl Fn(), receiver: Receiver<RepaintEvent>) {
    let mut schedule = RepaintSchedule::default();
    loop {
        let event = if let Some(remaining) = schedule.remaining(Instant::now()) {
            match receiver.recv_timeout(remaining) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => {
                    request_repaint();
                    schedule.repainted(Instant::now());
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
        if schedule.on_event(event, Instant::now()) {
            request_repaint();
            schedule.repainted(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_progress_is_coalesced_and_isolated_progress_stays_immediate() {
        let mut schedule = RepaintSchedule::default();
        let now = Instant::now();
        let interval = MIN_PROGRESS_REPAINT_INTERVAL;
        assert!(!schedule.on_event(RepaintEvent::Begin(1), now));
        assert!(schedule.on_event(RepaintEvent::Progress(1), now));
        schedule.repainted(now);
        assert_eq!(schedule.remaining(now), None);
        for _ in 0..20 {
            assert!(!schedule.on_event(RepaintEvent::Progress(1), now + interval / 2));
        }
        assert_eq!(schedule.remaining(now + interval / 2), Some(interval / 2));
        assert_eq!(schedule.remaining(now + interval), Some(Duration::ZERO));
        schedule.repainted(now + interval);
        assert_eq!(schedule.remaining(now + interval), None);
        assert!(schedule.on_event(RepaintEvent::Progress(1), now + interval * 2));
    }

    #[test]
    fn terminal_event_cancels_pending_progress_and_rejects_later_events() {
        let mut schedule = RepaintSchedule::default();
        let now = Instant::now();
        schedule.on_event(RepaintEvent::Begin(1), now);
        assert!(schedule.on_event(RepaintEvent::Progress(1), now));
        schedule.repainted(now);
        assert!(!schedule.on_event(RepaintEvent::Progress(1), now));
        assert!(schedule.remaining(now).is_some());
        assert!(schedule.on_event(RepaintEvent::Terminal(1), now));
        assert_eq!(schedule.remaining(now), None);
        assert!(!schedule.on_event(RepaintEvent::Progress(1), now));
        assert!(!schedule.on_event(RepaintEvent::Terminal(1), now));
    }

    #[test]
    fn new_operation_retires_pending_progress_and_ignores_stale_events() {
        let mut schedule = RepaintSchedule::default();
        let now = Instant::now();
        schedule.on_event(RepaintEvent::Begin(1), now);
        assert!(schedule.on_event(RepaintEvent::Progress(1), now));
        schedule.repainted(now);
        assert!(!schedule.on_event(RepaintEvent::Progress(1), now));
        schedule.on_event(RepaintEvent::Begin(2), now);
        assert_eq!(schedule.remaining(now), None);
        assert!(!schedule.on_event(RepaintEvent::Progress(1), now));
        assert!(!schedule.on_event(RepaintEvent::Terminal(1), now));
        assert!(schedule.on_event(RepaintEvent::Progress(2), now));
    }

    #[test]
    fn worker_delivers_initial_coalesced_and_terminal_repaints() {
        let (sender, receiver) = mpsc::channel();
        let mut owner = MediaProgressRepaintOwner::new(move || {
            let _ = sender.send(());
        });
        let wake = owner.begin();
        let timeout = Duration::from_secs(10);
        wake.notify(MediaProcessEvent::Progress);
        receiver.recv_timeout(timeout).unwrap();
        wake.notify(MediaProcessEvent::Progress);
        receiver.recv_timeout(timeout).unwrap();
        wake.notify(MediaProcessEvent::Terminal);
        receiver.recv_timeout(timeout).unwrap();
    }
}
