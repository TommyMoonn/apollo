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

use super::{intake::SourceIdentity, model::MediaFile, tools::MediaTools};

pub const MAX_BATCH_INSPECTIONS: usize = 2;

#[derive(Debug)]
pub struct BatchInspectionResult {
    pub index: usize,
    pub result: Result<BatchInspectedMedia, String>,
}

#[derive(Debug)]
pub struct BatchInspectedMedia {
    pub source_identity: SourceIdentity,
    pub media: MediaFile,
}

pub struct BatchInspectionOwner {
    receiver: Receiver<BatchInspectionResult>,
    cancellation: Arc<AtomicBool>,
    remaining: usize,
}

impl BatchInspectionOwner {
    pub fn spawn(tools: MediaTools, paths: Vec<PathBuf>, context: Context) -> io::Result<Self> {
        Self::spawn_with(paths, context, move |path| {
            let source_identity = SourceIdentity::read(path).map_err(|error| error.to_string())?;
            let media =
                super::probe::probe_media(&tools, path).map_err(|error| error.to_string())?;
            Ok(BatchInspectedMedia {
                source_identity,
                media,
            })
        })
    }

    fn spawn_with<F>(paths: Vec<PathBuf>, context: Context, inspect: F) -> io::Result<Self>
    where
        F: Fn(&std::path::Path) -> Result<BatchInspectedMedia, String> + Send + Sync + 'static,
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
        collections::HashSet,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use super::{BatchInspectedMedia, BatchInspectionOwner, MAX_BATCH_INSPECTIONS};
    use crate::media::{intake::SourceIdentity, model::MediaFile};

    const TIMEOUT: Duration = Duration::from_secs(10);
    const SOURCES: usize = 8;

    #[test]
    fn inspection_concurrency_is_bounded_and_results_keep_source_order_identity() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = Fixture(std::env::temp_dir().join(format!(
            "apollo-batch-inspection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
        std::fs::create_dir(&directory.0).unwrap();
        let paths: Vec<_> = (0..SOURCES)
            .map(|index| {
                let path = directory.0.join(format!("{index}.mkv"));
                std::fs::write(&path, [index as u8]).unwrap();
                path
            })
            .collect();
        let (started_tx, started_rx) = mpsc::channel();
        let (permits, gates): (Vec<_>, Vec<_>) = (0..SOURCES)
            .map(|_| {
                let (tx, rx) = mpsc::channel();
                (tx, Mutex::new(rx))
            })
            .unzip();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let worker_active = Arc::clone(&active);
        let worker_peak = Arc::clone(&peak);
        let mut owner = BatchInspectionOwner::spawn_with(
            paths.clone(),
            egui::Context::default(),
            move |path: &Path| {
                let index: usize = path.file_stem().unwrap().to_str().unwrap().parse().unwrap();
                let current = worker_active.fetch_add(1, Ordering::AcqRel) + 1;
                worker_peak.fetch_max(current, Ordering::AcqRel);
                started_tx.send((index, thread::current().id())).unwrap();
                let permit = gates[index].lock().unwrap().recv_timeout(TIMEOUT);
                worker_active.fetch_sub(1, Ordering::AcqRel);
                permit.map_err(|error| error.to_string())?;
                Ok(BatchInspectedMedia {
                    source_identity: SourceIdentity::read(path).unwrap(),
                    media: MediaFile {
                        path: path.to_path_buf(),
                        duration: None,
                        streams: Vec::new(),
                        video_streams: Vec::new(),
                        audio_streams: Vec::new(),
                    },
                })
            },
        )
        .unwrap();
        let deadline = Instant::now() + TIMEOUT;
        let mut workers = HashSet::new();
        let mut started = HashSet::new();
        let mut results = Vec::new();

        // Hold the first two inspections open until both have actually started.
        for _ in 0..MAX_BATCH_INSPECTIONS {
            let (index, worker) = started_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            assert!(started.insert(index));
            workers.insert(worker);
        }
        assert_eq!(started, HashSet::from([0, 1]));
        assert_eq!(active.load(Ordering::Acquire), MAX_BATCH_INSPECTIONS);
        assert!(owner.poll().is_empty());
        assert!(!owner.is_complete());

        // Finish the second source first. Its result must keep index 1 and its own media.
        permits[1].send(()).unwrap();
        while results.is_empty() {
            assert!(
                Instant::now() < deadline,
                "second inspection did not finish"
            );
            results.extend(owner.poll());
            if results.is_empty() {
                thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 1);
        // Release only jobs that have started; every job's worker identity is observed.
        permits[0].send(()).unwrap();
        for _ in MAX_BATCH_INSPECTIONS..SOURCES {
            let (index, worker) = started_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            assert!(started.insert(index));
            workers.insert(worker);
            assert!(workers.len() <= MAX_BATCH_INSPECTIONS);
            permits[index].send(()).unwrap();
        }
        while !owner.is_complete() {
            assert!(Instant::now() < deadline, "batch did not finish");
            results.extend(owner.poll());
            if !owner.is_complete() {
                thread::sleep(Duration::from_millis(1));
            }
        }
        results.sort_unstable_by_key(|result| result.index);
        assert_eq!(results.len(), SOURCES);
        for (index, result) in results.into_iter().enumerate() {
            assert_eq!(result.index, index);
            let inspected = result.result.unwrap();
            assert_eq!(inspected.media.path, paths[index]);
            assert_eq!(
                inspected.source_identity,
                SourceIdentity::read(&paths[index]).unwrap()
            );
        }
        assert_eq!(peak.load(Ordering::Acquire), MAX_BATCH_INSPECTIONS);
        assert_eq!(workers.len(), MAX_BATCH_INSPECTIONS);
        assert!(owner.poll().is_empty());
    }
}
