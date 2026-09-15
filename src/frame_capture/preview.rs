use std::{
    fmt,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::media::{
    execution::ProcessControl,
    intake::SourceIdentity,
    model::MediaFile,
    process::{
        MediaProcess, MediaProcessCommand, MediaProcessFailure, MediaProcessStartError,
        MediaProcessState, MediaProcessWake,
    },
    tools::MediaTools,
};

use super::{
    command::build_preview_invocation, model::FrameTimestamp, validation::validate_frame_timestamp,
};

pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(200);
pub const PREVIEW_MAXIMUM_EDGE: u32 = 720;
const PREVIEW_STDOUT_LIMIT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewGeneration(u64);

impl PreviewGeneration {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRequest {
    pub generation: PreviewGeneration,
    pub source_identity: SourceIdentity,
    pub input: PathBuf,
    pub video_stream_index: u32,
    pub timestamp: FrameTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewExtractionError {
    Process {
        reason: MediaProcessFailure,
        diagnostic: String,
    },
    Cancelled,
}

impl fmt::Display for PreviewExtractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Process { reason, .. } => write!(formatter, "preview process failed: {reason:?}"),
            Self::Cancelled => formatter.write_str("preview extraction was cancelled"),
        }
    }
}

impl std::error::Error for PreviewExtractionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewCompletion {
    pub request: PreviewRequest,
    pub result: Result<Vec<u8>, PreviewExtractionError>,
}

pub struct PreviewExtraction {
    request: PreviewRequest,
    process: Option<Box<dyn PreviewProcessControl>>,
}

trait PreviewProcessControl: ProcessControl {
    fn captured_stdout_bytes(&self) -> Vec<u8>;
}

impl PreviewProcessControl for MediaProcess {
    fn captured_stdout_bytes(&self) -> Vec<u8> {
        MediaProcess::captured_stdout_bytes(self)
    }
}

impl fmt::Debug for PreviewExtraction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreviewExtraction")
            .field("request", &self.request)
            .field("running", &self.process.is_some())
            .finish()
    }
}

impl PreviewExtraction {
    pub fn start(
        tools: &MediaTools,
        request: PreviewRequest,
    ) -> Result<Self, MediaProcessStartError> {
        Self::start_with_launcher(tools, request, MediaProcess::start)
    }

    pub fn start_with_wake(
        tools: &MediaTools,
        request: PreviewRequest,
        wake: MediaProcessWake,
    ) -> Result<Self, MediaProcessStartError> {
        Self::start_with_launcher(tools, request, move |command| {
            MediaProcess::start_with_wake(command, wake)
        })
    }

    pub fn request(&self) -> &PreviewRequest {
        &self.request
    }

    pub fn cancel(&self) -> bool {
        self.process
            .as_ref()
            .is_some_and(|process| process.cancel())
    }

    pub fn poll(&mut self) -> Option<PreviewCompletion> {
        let state = self.process.as_ref()?.state();
        if state == MediaProcessState::Running {
            return None;
        }

        let process = self
            .process
            .take()
            .expect("preview process exists while terminal output is settled");
        let result = match state {
            MediaProcessState::Completed => Ok(process.captured_stdout_bytes()),
            MediaProcessState::Failed(reason) => Err(PreviewExtractionError::Process {
                reason,
                diagnostic: process.diagnostic_stderr(),
            }),
            MediaProcessState::Cancelled => Err(PreviewExtractionError::Cancelled),
            MediaProcessState::Running => unreachable!("running preview was returned above"),
        };
        Some(PreviewCompletion {
            request: self.request.clone(),
            result,
        })
    }

    fn start_with_launcher<F, P>(
        tools: &MediaTools,
        request: PreviewRequest,
        launcher: F,
    ) -> Result<Self, MediaProcessStartError>
    where
        F: FnOnce(MediaProcessCommand) -> Result<P, MediaProcessStartError>,
        P: PreviewProcessControl + 'static,
    {
        let invocation = build_preview_invocation(&request);
        let command = MediaProcessCommand::new(&tools.ffmpeg)
            .args(invocation.args)
            .captured_stdout_limit(PREVIEW_STDOUT_LIMIT_BYTES);
        let process = launcher(command)?;
        Ok(Self {
            request,
            process: Some(Box::new(process)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewState<Image> {
    Idle,
    Debouncing {
        request: PreviewRequest,
        ready_at: Instant,
    },
    Loading {
        request: PreviewRequest,
    },
    Ready {
        request: PreviewRequest,
        image: Image,
    },
    Failed {
        request: PreviewRequest,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramePreview<Image> {
    generation: PreviewGeneration,
    state: PreviewState<Image>,
}

impl<Image> Default for FramePreview<Image> {
    fn default() -> Self {
        Self {
            generation: PreviewGeneration(0),
            state: PreviewState::Idle,
        }
    }
}

impl<Image> FramePreview<Image> {
    pub fn state(&self) -> &PreviewState<Image> {
        &self.state
    }

    pub fn schedule(
        &mut self,
        source_identity: &SourceIdentity,
        media: &MediaFile,
        timestamp: FrameTimestamp,
        now: Instant,
    ) -> Option<PreviewGeneration> {
        let Some(video_stream) = media.video_streams.first() else {
            self.invalidate();
            return None;
        };
        if validate_frame_timestamp(timestamp, media.duration).is_err() {
            self.invalidate();
            return None;
        }

        let generation = self.advance_generation();
        self.state = PreviewState::Debouncing {
            request: PreviewRequest {
                generation,
                source_identity: source_identity.clone(),
                input: media.path.clone(),
                video_stream_index: video_stream.stream_index,
                timestamp,
            },
            ready_at: now + PREVIEW_DEBOUNCE,
        };
        Some(generation)
    }

    pub fn take_due_request(&mut self, now: Instant) -> Option<PreviewRequest> {
        let PreviewState::Debouncing { request, ready_at } = &self.state else {
            return None;
        };
        if now < *ready_at {
            return None;
        }

        let request = request.clone();
        self.state = PreviewState::Loading {
            request: request.clone(),
        };
        Some(request)
    }

    pub fn invalidate(&mut self) {
        self.advance_generation();
        self.state = PreviewState::Idle;
    }

    fn advance_generation(&mut self) -> PreviewGeneration {
        self.generation.0 = self
            .generation
            .0
            .checked_add(1)
            .expect("Frame Capture preview generation space is exhausted");
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use crate::media::{
        execution::ProcessControl,
        intake::SourceIdentity,
        model::{MediaFile, VideoStream},
        process::{MediaProcessFailure, MediaProcessState},
        tools::MediaTools,
    };

    use super::{
        FramePreview, PREVIEW_DEBOUNCE, PREVIEW_MAXIMUM_EDGE, PREVIEW_STDOUT_LIMIT_BYTES,
        PreviewExtraction, PreviewExtractionError, PreviewState,
    };
    use crate::frame_capture::model::FrameTimestamp;

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temp_source(label: &str, bytes: &[u8]) -> (PathBuf, PathBuf, SourceIdentity) {
        let directory = std::env::temp_dir().join(format!(
            "apollo-preview-{label}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("recording.mkv");
        fs::write(&path, bytes).unwrap();
        let identity = SourceIdentity::read(&path).unwrap();
        (directory, path, identity)
    }

    fn media(path: &Path, duration: Option<Duration>) -> MediaFile {
        MediaFile {
            path: path.to_path_buf(),
            duration,
            streams: Vec::new(),
            video_streams: vec![VideoStream {
                stream_index: 7,
                codec: Some("h264".into()),
                width: Some(1920),
                height: Some(1080),
                frame_rate: None,
                format: Default::default(),
            }],
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn valid_timestamp_debounces_before_yielding_primary_stream_request() {
        let (directory, path, identity) = temp_source("valid", b"first");
        let media = media(&path, Some(Duration::from_secs(10)));
        let now = Instant::now();
        let timestamp = FrameTimestamp::from_milliseconds(1_250);
        let mut preview = FramePreview::<()>::default();

        let generation = preview.schedule(&identity, &media, timestamp, now).unwrap();

        assert!(matches!(
            preview.state(),
            PreviewState::Debouncing { request, ready_at }
                if request.generation == generation
                    && request.source_identity == identity
                    && request.input == media.path
                    && request.video_stream_index == 7
                    && request.timestamp == timestamp
                    && *ready_at == now + PREVIEW_DEBOUNCE
        ));
        assert!(
            preview
                .take_due_request(now + PREVIEW_DEBOUNCE - Duration::from_millis(1))
                .is_none()
        );
        assert_eq!(
            preview
                .take_due_request(now + PREVIEW_DEBOUNCE)
                .unwrap()
                .timestamp,
            timestamp
        );
        assert!(matches!(preview.state(), PreviewState::Loading { .. }));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_timestamp_and_missing_video_schedule_nothing() {
        let now = Instant::now();
        let mut preview = FramePreview::<()>::default();
        let (directory, path, identity) = temp_source("invalid", b"first");
        let media = media(&path, Some(Duration::from_secs(2)));

        assert!(
            preview
                .schedule(
                    &identity,
                    &media,
                    FrameTimestamp::from_milliseconds(2_000),
                    now,
                )
                .is_none()
        );
        assert!(matches!(preview.state(), PreviewState::Idle));

        let mut no_video = media;
        no_video.video_streams.clear();
        assert!(
            preview
                .schedule(&identity, &no_video, FrameTimestamp::ZERO, now)
                .is_none()
        );
        assert!(matches!(preview.state(), PreviewState::Idle));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rapid_edits_collapse_to_the_newest_generation() {
        let (directory, path, identity) = temp_source("rapid", b"first");
        let media = media(&path, None);
        let now = Instant::now();
        let mut preview = FramePreview::<()>::default();
        let first = preview
            .schedule(
                &identity,
                &media,
                FrameTimestamp::from_milliseconds(100),
                now,
            )
            .unwrap();
        let second_timestamp = FrameTimestamp::from_milliseconds(900);
        let second = preview
            .schedule(
                &identity,
                &media,
                second_timestamp,
                now + Duration::from_millis(50),
            )
            .unwrap();

        assert!(second.get() > first.get());
        assert!(preview.take_due_request(now + PREVIEW_DEBOUNCE).is_none());
        let request = preview
            .take_due_request(now + PREVIEW_DEBOUNCE + Duration::from_millis(50))
            .unwrap();
        assert_eq!(request.generation, second);
        assert_eq!(request.timestamp, second_timestamp);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalidation_retires_pending_source_ownership() {
        let (first_directory, first_path, first_identity) = temp_source("first", b"first");
        let (replacement_directory, replacement_path, replacement_identity) =
            temp_source("replacement", b"replacement");
        let first = media(&first_path, None);
        let replacement = media(&replacement_path, None);
        let now = Instant::now();
        let mut preview = FramePreview::<()>::default();
        let first_generation = preview
            .schedule(&first_identity, &first, FrameTimestamp::ZERO, now)
            .unwrap();

        preview.invalidate();
        assert!(preview.take_due_request(now + PREVIEW_DEBOUNCE).is_none());

        let replacement_generation = preview
            .schedule(
                &replacement_identity,
                &replacement,
                FrameTimestamp::ZERO,
                now,
            )
            .unwrap();
        assert!(replacement_generation.get() > first_generation.get());
        assert!(matches!(
            preview.state(),
            PreviewState::Debouncing { request, .. }
                if request.source_identity == replacement_identity
                    && request.input == replacement.path
        ));
        fs::remove_dir_all(first_directory).unwrap();
        fs::remove_dir_all(replacement_directory).unwrap();
    }

    #[test]
    fn changed_snapshot_at_the_same_path_has_distinct_preview_identity() {
        let (directory, path, first_identity) = temp_source("same-path", b"first");
        let media = media(&path, None);
        let now = Instant::now();
        let mut preview = FramePreview::<()>::default();
        preview
            .schedule(&first_identity, &media, FrameTimestamp::ZERO, now)
            .unwrap();
        let first_request = preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap();

        fs::write(&path, b"replacement with a different length").unwrap();
        let replacement_identity = SourceIdentity::read(&path).unwrap();
        preview
            .schedule(&replacement_identity, &media, FrameTimestamp::ZERO, now)
            .unwrap();
        let replacement_request = preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap();

        assert_eq!(first_request.input, replacement_request.input);
        assert_ne!(
            first_request.source_identity,
            replacement_request.source_identity
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extraction_command_targets_one_bounded_primary_frame_in_memory() {
        let (directory, path, identity) = temp_source("command", b"source");
        let media = media(&path, None);
        let now = Instant::now();
        let timestamp = FrameTimestamp::from_milliseconds(12_500);
        let mut preview = FramePreview::<()>::default();
        preview.schedule(&identity, &media, timestamp, now).unwrap();
        let request = preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap();
        let observed = Arc::new(Mutex::new(None));
        let capture = Arc::clone(&observed);
        let process = FakeProcess::running(Vec::new());

        let extraction = PreviewExtraction::start_with_launcher(
            &MediaTools::new("resolved-ffmpeg", "resolved-ffprobe"),
            request,
            move |command| {
                *capture.lock().unwrap() = Some(command);
                Ok(process)
            },
        )
        .unwrap();

        assert_eq!(extraction.request().timestamp, timestamp);
        let observed = observed.lock().unwrap();
        let command = observed.as_ref().unwrap();
        assert_eq!(command.program(), Path::new("resolved-ffmpeg"));
        let args = command.arguments();
        assert!(has_pair(args, "-ss", "00:00:12.500"));
        assert!(has_pair(args, "-map", "0:7"));
        assert_eq!(args.iter().filter(|arg| *arg == "-map").count(), 1);
        assert!(has_pair(args, "-frames:v", "1"));
        assert!(args.iter().any(|arg| arg == "-an"));
        assert!(has_pair(args, "-pix_fmt", "rgba"));
        assert!(has_pair(args, "-c:v", "png"));
        assert!(has_pair(args, "-f", "image2pipe"));
        assert_eq!(args.last().unwrap(), "pipe:1");
        assert!(!args.iter().any(|arg| arg == "-progress"));
        assert!(args.iter().any(|arg| {
            arg.to_string_lossy()
                .contains(&format!("min({PREVIEW_MAXIMUM_EDGE},iw)"))
        }));
        assert_eq!(
            command.captured_stdout_limit_bytes(),
            PREVIEW_STDOUT_LIMIT_BYTES
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn completed_extraction_returns_exact_png_bytes_without_a_public_output() {
        let (directory, path, identity) = temp_source("bytes", b"source");
        let request = preview_request(&identity, &media(&path, None));
        let png_bytes = b"\x89PNG\r\n\x1a\nexact-phase-two-payload".to_vec();
        let process = FakeProcess::completed(png_bytes.clone());
        let mut extraction = PreviewExtraction::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            request.clone(),
            |_| Ok(process),
        )
        .unwrap();

        let completion = extraction.poll().unwrap();

        assert_eq!(completion.request, request);
        assert_eq!(completion.result, Ok(png_bytes));
        assert!(extraction.poll().is_none());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn extraction_failure_retains_stderr_and_cancellation_uses_process_owner() {
        let (directory, path, identity) = temp_source("failure", b"source");
        let request = preview_request(&identity, &media(&path, None));
        let failed =
            FakeProcess::failed(MediaProcessFailure::Exit(Some(7)), "ffmpeg preview failure");
        let mut extraction = PreviewExtraction::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            request.clone(),
            |_| Ok(failed),
        )
        .unwrap();

        assert_eq!(
            extraction.poll().unwrap().result,
            Err(PreviewExtractionError::Process {
                reason: MediaProcessFailure::Exit(Some(7)),
                diagnostic: "ffmpeg preview failure".to_owned(),
            })
        );

        let running = FakeProcess::running(Vec::new());
        let handle = running.clone();
        let mut extraction = PreviewExtraction::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            request,
            |_| Ok(running),
        )
        .unwrap();
        assert!(extraction.cancel());
        assert_eq!(handle.state(), MediaProcessState::Cancelled);
        assert_eq!(
            extraction.poll().unwrap().result,
            Err(PreviewExtractionError::Cancelled)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn preview_request(identity: &SourceIdentity, media: &MediaFile) -> super::PreviewRequest {
        let now = Instant::now();
        let mut preview = FramePreview::<()>::default();
        preview
            .schedule(identity, media, FrameTimestamp::ZERO, now)
            .unwrap();
        preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap()
    }

    fn has_pair(args: &[std::ffi::OsString], option: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == option && pair[1] == value)
    }

    #[derive(Clone)]
    struct FakeProcess {
        state: Arc<Mutex<MediaProcessState>>,
        stdout: Vec<u8>,
        diagnostic: String,
    }

    impl FakeProcess {
        fn running(stdout: Vec<u8>) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Running)),
                stdout,
                diagnostic: String::new(),
            }
        }

        fn completed(stdout: Vec<u8>) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Completed)),
                stdout,
                diagnostic: String::new(),
            }
        }

        fn failed(reason: MediaProcessFailure, diagnostic: &str) -> Self {
            Self {
                state: Arc::new(Mutex::new(MediaProcessState::Failed(reason))),
                stdout: Vec::new(),
                diagnostic: diagnostic.to_owned(),
            }
        }
    }

    impl ProcessControl for FakeProcess {
        fn state(&self) -> MediaProcessState {
            self.state.lock().unwrap().clone()
        }

        fn cancel(&self) -> bool {
            let mut state = self.state.lock().unwrap();
            if *state != MediaProcessState::Running {
                return false;
            }
            *state = MediaProcessState::Cancelled;
            true
        }

        fn diagnostic_stderr(&self) -> String {
            self.diagnostic.clone()
        }

        fn drain_progress(&self) -> Vec<String> {
            Vec::new()
        }
    }

    impl super::PreviewProcessControl for FakeProcess {
        fn captured_stdout_bytes(&self) -> Vec<u8> {
            self.stdout.clone()
        }
    }
}
