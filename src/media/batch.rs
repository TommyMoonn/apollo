use std::{
    collections::VecDeque,
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use egui::Context;

use super::{model::MediaFile, tools::MediaTools};

pub const MAX_BATCH_INSPECTIONS: usize = 2;

#[derive(Debug)]
pub struct BatchInspectionResult {
    pub index: usize,
    pub result: Result<MediaFile, String>,
}

pub struct BatchInspectionOwner {
    receiver: Receiver<BatchInspectionResult>,
    cancellation: Arc<AtomicBool>,
    remaining: usize,
}

impl BatchInspectionOwner {
    pub fn spawn(tools: MediaTools, paths: Vec<PathBuf>, context: Context) -> io::Result<Self> {
        Self::spawn_with(paths, context, move |path| {
            super::probe::probe_media(&tools, path).map_err(|error| error.to_string())
        })
    }

    fn spawn_with<F>(paths: Vec<PathBuf>, context: Context, inspect: F) -> io::Result<Self>
    where
        F: Fn(&std::path::Path) -> Result<MediaFile, String> + Send + Sync + 'static,
    {
        let remaining = paths.len();
        let work = Arc::new(Mutex::new(
            paths.into_iter().enumerate().collect::<VecDeque<_>>(),
        ));
        let inspect = Arc::new(inspect);
        let cancellation = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let worker_count = remaining.min(MAX_BATCH_INSPECTIONS);

        for worker_index in 0..worker_count {
            let work = Arc::clone(&work);
            let inspect = Arc::clone(&inspect);
            let cancellation = Arc::clone(&cancellation);
            let sender = sender.clone();
            let context = context.clone();
            thread::Builder::new()
                .name(format!("apollo-batch-inspection-{worker_index}"))
                .spawn(move || {
                    loop {
                        if cancellation.load(Ordering::Acquire) {
                            break;
                        }
                        let next = work.lock().ok().and_then(|mut work| work.pop_front());
                        let Some((index, path)) = next else {
                            break;
                        };
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            inspect(&path)
                        }))
                        .unwrap_or_else(|_| {
                            Err("Media inspection worker stopped unexpectedly.".to_owned())
                        });
                        if cancellation.load(Ordering::Acquire) {
                            break;
                        }
                        if sender
                            .send(BatchInspectionResult { index, result })
                            .is_err()
                        {
                            break;
                        }
                        context.request_repaint();
                    }
                })?;
        }
        drop(sender);
        Ok(Self {
            receiver,
            cancellation,
            remaining,
        })
    }

    pub fn poll(&mut self) -> Vec<BatchInspectionResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.receiver.try_recv() {
            self.remaining = self.remaining.saturating_sub(1);
            results.push(result);
        }
        results
    }

    pub fn is_complete(&self) -> bool {
        self.remaining == 0
    }
}

impl Drop for BatchInspectionOwner {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use super::{BatchInspectionOwner, MAX_BATCH_INSPECTIONS};
    use crate::media::model::MediaFile;

    #[test]
    fn inspection_concurrency_is_bounded_and_results_keep_source_order_identity() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let active_for_worker = Arc::clone(&active);
        let peak_for_worker = Arc::clone(&peak);
        let paths = (0..8).map(|index| format!("{index}.mkv").into()).collect();
        let mut owner = BatchInspectionOwner::spawn_with(
            paths,
            egui::Context::default(),
            move |path: &Path| {
                let current = active_for_worker.fetch_add(1, Ordering::AcqRel) + 1;
                peak_for_worker.fetch_max(current, Ordering::AcqRel);
                thread::sleep(Duration::from_millis(2));
                active_for_worker.fetch_sub(1, Ordering::AcqRel);
                Ok(MediaFile {
                    path: path.to_path_buf(),
                    duration: None,
                    streams: Vec::new(),
                    video_streams: Vec::new(),
                    audio_streams: Vec::new(),
                })
            },
        )
        .unwrap();
        let mut indexes = Vec::new();
        while !owner.is_complete() {
            indexes.extend(owner.poll().into_iter().map(|result| result.index));
            thread::yield_now();
        }
        indexes.sort_unstable();
        assert_eq!(indexes, (0..8).collect::<Vec<_>>());
        assert!(peak.load(Ordering::Acquire) <= MAX_BATCH_INSPECTIONS);
    }
}
