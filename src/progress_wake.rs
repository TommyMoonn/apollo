use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use crate::media::process::{MediaProcessEvent, MediaProcessWake};

const MIN_PROGRESS_WAKE_INTERVAL: Duration = Duration::from_millis(34);

enum WakeEvent {
    Begin(u64),
    Progress(u64),
    Terminal(u64),
}

pub(crate) struct MediaProgressWakeOwner {
    sender: Sender<WakeEvent>,
    next_operation_id: u64,
}

impl MediaProgressWakeOwner {
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "Native GPUI entry is Windows-only")
    )]
    pub(crate) fn new(notify: impl Fn() + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("apollo-progress-wake".to_owned())
            .spawn(move || run_wake_worker(notify, receiver))
            .expect("Apollo progress wake worker must start");
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
            .expect("Apollo progress wake operation identifier space is exhausted");
        let _ = self.sender.send(WakeEvent::Begin(operation_id));
        let sender = self.sender.clone();
        MediaProcessWake::new(move |event| {
            let event = match event {
                MediaProcessEvent::Progress => WakeEvent::Progress(operation_id),
                MediaProcessEvent::Terminal => WakeEvent::Terminal(operation_id),
            };
            let _ = sender.send(event);
        })
    }
}

#[derive(Default)]
struct WakeSchedule {
    active_operation: Option<u64>,
    last_progress_wake: Option<Instant>,
    progress_pending: bool,
}

impl WakeSchedule {
    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.progress_pending.then(|| {
            let elapsed = self
                .last_progress_wake
                .map(|last| now.saturating_duration_since(last))
                .unwrap_or(MIN_PROGRESS_WAKE_INTERVAL);
            MIN_PROGRESS_WAKE_INTERVAL.saturating_sub(elapsed)
        })
    }

    fn on_event(&mut self, event: WakeEvent, now: Instant) -> bool {
        match event {
            WakeEvent::Begin(id) => {
                *self = Self {
                    active_operation: Some(id),
                    ..Self::default()
                };
                false
            }
            WakeEvent::Progress(id) if self.active_operation == Some(id) => {
                let due = self.last_progress_wake.is_none_or(|last| {
                    now.saturating_duration_since(last) >= MIN_PROGRESS_WAKE_INTERVAL
                });
                self.progress_pending = !due;
                due
            }
            WakeEvent::Terminal(id) if self.active_operation == Some(id) => {
                self.active_operation = None;
                self.progress_pending = false;
                true
            }
            WakeEvent::Progress(_) | WakeEvent::Terminal(_) => false,
        }
    }

    fn notified(&mut self, now: Instant) {
        self.last_progress_wake = Some(now);
        self.progress_pending = false;
    }
}

fn run_wake_worker(notify: impl Fn(), receiver: Receiver<WakeEvent>) {
    let mut schedule = WakeSchedule::default();
    loop {
        let event = if let Some(remaining) = schedule.remaining(Instant::now()) {
            match receiver.recv_timeout(remaining) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => {
                    notify();
                    schedule.notified(Instant::now());
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
            notify();
            schedule.notified(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_progress_is_coalesced_and_isolated_progress_stays_immediate() {
        let mut schedule = WakeSchedule::default();
        let now = Instant::now();
        let interval = MIN_PROGRESS_WAKE_INTERVAL;
        assert!(!schedule.on_event(WakeEvent::Begin(1), now));
        assert!(schedule.on_event(WakeEvent::Progress(1), now));
        schedule.notified(now);
        assert_eq!(schedule.remaining(now), None);
        for _ in 0..20 {
            assert!(!schedule.on_event(WakeEvent::Progress(1), now + interval / 2));
        }
        assert_eq!(schedule.remaining(now + interval / 2), Some(interval / 2));
        assert_eq!(schedule.remaining(now + interval), Some(Duration::ZERO));
        schedule.notified(now + interval);
        assert_eq!(schedule.remaining(now + interval), None);
        assert!(schedule.on_event(WakeEvent::Progress(1), now + interval * 2));
    }

    #[test]
    fn terminal_event_cancels_pending_progress_and_rejects_later_events() {
        let mut schedule = WakeSchedule::default();
        let now = Instant::now();
        schedule.on_event(WakeEvent::Begin(1), now);
        assert!(schedule.on_event(WakeEvent::Progress(1), now));
        schedule.notified(now);
        assert!(!schedule.on_event(WakeEvent::Progress(1), now));
        assert!(schedule.remaining(now).is_some());
        assert!(schedule.on_event(WakeEvent::Terminal(1), now));
        assert_eq!(schedule.remaining(now), None);
        assert!(!schedule.on_event(WakeEvent::Progress(1), now));
        assert!(!schedule.on_event(WakeEvent::Terminal(1), now));
    }

    #[test]
    fn new_operation_retires_pending_progress_and_ignores_stale_events() {
        let mut schedule = WakeSchedule::default();
        let now = Instant::now();
        schedule.on_event(WakeEvent::Begin(1), now);
        assert!(schedule.on_event(WakeEvent::Progress(1), now));
        schedule.notified(now);
        assert!(!schedule.on_event(WakeEvent::Progress(1), now));
        schedule.on_event(WakeEvent::Begin(2), now);
        assert_eq!(schedule.remaining(now), None);
        assert!(!schedule.on_event(WakeEvent::Progress(1), now));
        assert!(!schedule.on_event(WakeEvent::Terminal(1), now));
        assert!(schedule.on_event(WakeEvent::Progress(2), now));
    }

    #[test]
    fn worker_delivers_initial_coalesced_and_terminal_wakes() {
        let (sender, receiver) = mpsc::channel();
        let mut owner = MediaProgressWakeOwner::new(move || {
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
