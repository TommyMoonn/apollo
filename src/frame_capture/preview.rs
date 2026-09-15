use std::{
    collections::VecDeque,
    fmt,
    io::Cursor,
    path::PathBuf,
    sync::Arc,
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
    command::build_preview_invocation,
    model::{FramePosition, FrameTimestamp},
    validation::validate_frame_position,
};

pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(200);
pub const PREVIEW_MAXIMUM_EDGE: u32 = 720;
const PREVIEW_STDOUT_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const PREVIEW_CACHE_CAPACITY: usize = 8;

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
    pub position: FramePosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewImage {
    size: [usize; 2],
    rgba: Arc<[u8]>,
}

impl PreviewImage {
    pub fn from_rgba(size: [usize; 2], rgba: Vec<u8>) -> Option<Self> {
        let expected_length = size[0].checked_mul(size[1])?.checked_mul(4)?;
        (size[0] > 0
            && size[1] > 0
            && size[0] <= PREVIEW_MAXIMUM_EDGE as usize
            && size[1] <= PREVIEW_MAXIMUM_EDGE as usize
            && rgba.len() == expected_length)
            .then(|| Self {
                size,
                rgba: Arc::from(rgba),
            })
    }

    pub fn size(&self) -> [usize; 2] {
        self.size
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

pub fn decode_preview_png(bytes: &[u8]) -> Result<PreviewImage, PreviewExtractionError> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(PREVIEW_MAXIMUM_EDGE);
    limits.max_image_height = Some(PREVIEW_MAXIMUM_EDGE);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|error| PreviewExtractionError::Decode(error.to_string()))?
        .to_rgba8();
    PreviewImage::from_rgba(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.into_raw(),
    )
    .ok_or_else(|| {
        PreviewExtractionError::Decode("preview image dimensions are invalid".to_owned())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewExtractionError {
    Start(MediaProcessStartError),
    Process {
        reason: MediaProcessFailure,
        diagnostic: String,
    },
    Decode(String),
    Cancelled,
}

impl fmt::Display for PreviewExtractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(error) => write!(formatter, "preview process could not start: {error}"),
            Self::Process { reason, .. } => write!(formatter, "preview process failed: {reason:?}"),
            Self::Decode(reason) => {
                write!(formatter, "preview image could not be decoded: {reason}")
            }
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

#[derive(Debug, Default)]
pub struct PreviewExtractionOwner {
    active: Option<PreviewExtraction>,
}

impl PreviewExtractionOwner {
    pub fn start(
        &mut self,
        tools: &MediaTools,
        request: PreviewRequest,
        wake: MediaProcessWake,
    ) -> Result<(), MediaProcessStartError> {
        self.cancel();
        self.active = Some(PreviewExtraction::start_with_wake(tools, request, wake)?);
        Ok(())
    }

    #[cfg(test)]
    fn request(&self) -> Option<&PreviewRequest> {
        self.active.as_ref().map(PreviewExtraction::request)
    }

    pub fn poll(&mut self) -> Option<PreviewCompletion> {
        let completion = self.active.as_mut()?.poll()?;
        self.active = None;
        Some(completion)
    }

    pub fn cancel(&mut self) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        active.cancel()
    }

    #[cfg(test)]
    fn replace(&mut self, extraction: PreviewExtraction) {
        self.cancel();
        self.active = Some(extraction);
    }
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
        error: PreviewExtractionError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewCacheKey {
    source_identity: SourceIdentity,
    position: FramePosition,
    maximum_edge: u32,
}

impl From<&PreviewRequest> for PreviewCacheKey {
    fn from(request: &PreviewRequest) -> Self {
        Self {
            source_identity: request.source_identity.clone(),
            position: request.position,
            maximum_edge: PREVIEW_MAXIMUM_EDGE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramePreview<Image> {
    generation: PreviewGeneration,
    state: PreviewState<Image>,
    source_identity: Option<SourceIdentity>,
    cache: VecDeque<(PreviewCacheKey, Image)>,
}

impl<Image> Default for FramePreview<Image> {
    fn default() -> Self {
        Self {
            generation: PreviewGeneration(0),
            state: PreviewState::Idle,
            source_identity: None,
            cache: VecDeque::new(),
        }
    }
}

impl<Image: Clone> FramePreview<Image> {
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
        self.schedule_position(source_identity, media, timestamp.into(), now)
    }

    pub fn schedule_position(
        &mut self,
        source_identity: &SourceIdentity,
        media: &MediaFile,
        position: FramePosition,
        now: Instant,
    ) -> Option<PreviewGeneration> {
        self.schedule_position_after(source_identity, media, position, now, PREVIEW_DEBOUNCE)
    }

    pub fn schedule_position_immediately(
        &mut self,
        source_identity: &SourceIdentity,
        media: &MediaFile,
        position: FramePosition,
        now: Instant,
    ) -> Option<PreviewGeneration> {
        self.schedule_position_after(source_identity, media, position, now, Duration::ZERO)
    }

    fn schedule_position_after(
        &mut self,
        source_identity: &SourceIdentity,
        media: &MediaFile,
        position: FramePosition,
        now: Instant,
        delay: Duration,
    ) -> Option<PreviewGeneration> {
        if self.source_identity.as_ref() != Some(source_identity) {
            self.source_identity = Some(source_identity.clone());
            self.cache.clear();
        }
        let Some(video_stream) = media.video_streams.first() else {
            self.invalidate();
            return None;
        };
        if validate_frame_position(position, media.duration).is_err() {
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
                position,
            },
            ready_at: now + delay,
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
        if let Some(image) = self.cached(&request) {
            self.state = PreviewState::Ready { request, image };
            return None;
        }
        self.state = PreviewState::Loading {
            request: request.clone(),
        };
        Some(request)
    }

    pub fn is_loading(&self, request: &PreviewRequest) -> bool {
        matches!(&self.state, PreviewState::Loading { request: current } if current == request)
    }

    pub fn settle(
        &mut self,
        request: PreviewRequest,
        result: Result<Image, PreviewExtractionError>,
    ) -> bool {
        if !self.is_loading(&request) {
            return false;
        }
        self.state = match result {
            Ok(image) => {
                self.cache_insert(&request, image.clone());
                PreviewState::Ready { request, image }
            }
            Err(error) => PreviewState::Failed { request, error },
        };
        true
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

    fn cached(&mut self, request: &PreviewRequest) -> Option<Image> {
        let key = PreviewCacheKey::from(request);
        let index = self
            .cache
            .iter()
            .position(|(candidate, _)| *candidate == key)?;
        let entry = self
            .cache
            .remove(index)
            .expect("located preview cache entry remains present");
        let image = entry.1.clone();
        self.cache.push_back(entry);
        Some(image)
    }

    fn cache_insert(&mut self, request: &PreviewRequest, image: Image) {
        let key = PreviewCacheKey::from(request);
        if let Some(index) = self
            .cache
            .iter()
            .position(|(candidate, _)| *candidate == key)
        {
            self.cache.remove(index);
        }
        self.cache.push_back((key, image));
        if self.cache.len() > PREVIEW_CACHE_CAPACITY {
            self.cache.pop_front();
        }
    }

    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Cursor,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use image::{DynamicImage, ImageFormat};

    use crate::media::{
        execution::ProcessControl,
        intake::SourceIdentity,
        model::{MediaFile, VideoStream},
        process::{MediaProcessFailure, MediaProcessState},
        tools::MediaTools,
    };

    use super::{
        FramePreview, PREVIEW_CACHE_CAPACITY, PREVIEW_DEBOUNCE, PREVIEW_MAXIMUM_EDGE,
        PREVIEW_STDOUT_LIMIT_BYTES, PreviewExtraction, PreviewExtractionError,
        PreviewExtractionOwner, PreviewImage, PreviewState, decode_preview_png,
    };
    use crate::frame_capture::{
        command::{build_frame_capture_invocation, build_preview_invocation},
        model::{
            FrameCaptureFormat, FrameCaptureRequest, FrameTimeBase, FrameTimestamp,
            ResolvedFramePosition,
        },
    };

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
                    && request.position == timestamp.into()
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
                .position,
            timestamp.into()
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
        assert_eq!(request.position, second_timestamp.into());
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

        assert_eq!(extraction.request().position, timestamp.into());
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
    fn resolved_frame_uses_the_same_precise_seek_for_preview_and_capture() {
        let (directory, path, identity) = temp_source("resolved-command", b"source");
        let media = media(&path, None);
        let position =
            ResolvedFramePosition::new(1_001, FrameTimeBase::new(1, 30_000).unwrap()).unwrap();
        let now = Instant::now();
        let mut preview = FramePreview::<()>::default();
        preview
            .schedule_position(&identity, &media, position.into(), now)
            .unwrap();
        let preview_request = preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap();
        let capture_request = FrameCaptureRequest {
            input: path,
            output: directory.join("frame.png"),
            position: position.into(),
            format: FrameCaptureFormat::Png,
        };

        let preview_args = build_preview_invocation(&preview_request).args;
        let capture_args = build_frame_capture_invocation(&capture_request, 7).args;
        assert!(has_pair(&preview_args, "-ss", "0.033366666"));
        assert!(has_pair(&capture_args, "-ss", "0.033366666"));
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

    #[test]
    fn extraction_owner_replaces_and_cancels_the_only_active_process() {
        let (directory, path, identity) = temp_source("owner", b"source");
        let media = media(&path, None);
        let first_request =
            preview_request_at(&identity, &media, FrameTimestamp::from_milliseconds(100));
        let second_request =
            preview_request_at(&identity, &media, FrameTimestamp::from_milliseconds(200));
        let first_process = FakeProcess::running(Vec::new());
        let first_handle = first_process.clone();
        let first = PreviewExtraction::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            first_request,
            |_| Ok(first_process),
        )
        .unwrap();
        let second_process = FakeProcess::running(Vec::new());
        let second_handle = second_process.clone();
        let second = PreviewExtraction::start_with_launcher(
            &MediaTools::new("ffmpeg", "ffprobe"),
            second_request.clone(),
            |_| Ok(second_process),
        )
        .unwrap();
        let mut owner = PreviewExtractionOwner::default();

        owner.replace(first);
        owner.replace(second);

        assert_eq!(first_handle.state(), MediaProcessState::Cancelled);
        assert_eq!(owner.request(), Some(&second_request));
        assert!(owner.cancel());
        assert_eq!(second_handle.state(), MediaProcessState::Cancelled);
        assert!(owner.request().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_completion_cannot_replace_a_newer_generation() {
        let (directory, path, identity) = temp_source("stale", b"source");
        let media = media(&path, None);
        let now = Instant::now();
        let mut preview = FramePreview::<usize>::default();
        preview
            .schedule(
                &identity,
                &media,
                FrameTimestamp::from_milliseconds(100),
                now,
            )
            .unwrap();
        let stale = preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap();
        let current_generation = preview
            .schedule(
                &identity,
                &media,
                FrameTimestamp::from_milliseconds(200),
                now + PREVIEW_DEBOUNCE,
            )
            .unwrap();

        assert!(!preview.settle(stale, Ok(1)));
        assert!(matches!(
            preview.state(),
            PreviewState::Debouncing { request, .. }
                if request.generation == current_generation
                    && request.position == FrameTimestamp::from_milliseconds(200).into()
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cache_hit_avoids_a_request_and_lru_eviction_is_bounded() {
        let (directory, path, identity) = temp_source("cache", b"source");
        let media = media(&path, None);
        let now = Instant::now();
        let mut preview = FramePreview::<usize>::default();
        for index in 0..=PREVIEW_CACHE_CAPACITY {
            let timestamp = FrameTimestamp::from_milliseconds(index as i64);
            preview.schedule(&identity, &media, timestamp, now).unwrap();
            let request = preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap();
            assert!(preview.settle(request, Ok(index)));
        }

        assert_eq!(preview.cache_len(), PREVIEW_CACHE_CAPACITY);
        preview
            .schedule(&identity, &media, FrameTimestamp::ZERO, now)
            .unwrap();
        assert!(preview.take_due_request(now + PREVIEW_DEBOUNCE).is_some());
        preview
            .schedule(&identity, &media, FrameTimestamp::from_milliseconds(1), now)
            .unwrap();
        assert!(preview.take_due_request(now + PREVIEW_DEBOUNCE).is_none());
        assert!(matches!(
            preview.state(),
            PreviewState::Ready { image: 1, .. }
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changed_source_identity_clears_source_specific_cache() {
        let (directory, path, first_identity) = temp_source("cache-source", b"source");
        let media = media(&path, None);
        let now = Instant::now();
        let mut preview = FramePreview::<usize>::default();
        preview
            .schedule(&first_identity, &media, FrameTimestamp::ZERO, now)
            .unwrap();
        let request = preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap();
        assert!(preview.settle(request, Ok(1)));

        fs::write(&path, b"replacement source identity").unwrap();
        let replacement_identity = SourceIdentity::read(&path).unwrap();
        preview
            .schedule(&replacement_identity, &media, FrameTimestamp::ZERO, now)
            .unwrap();

        assert_eq!(preview.cache_len(), 0);
        assert!(preview.take_due_request(now + PREVIEW_DEBOUNCE).is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn png_decode_returns_bounded_rgba_and_rejects_oversized_images() {
        let image = decode_preview_png(&png_bytes(720, 405)).unwrap();
        assert_eq!(image.size(), [720, 405]);
        assert_eq!(image.rgba().len(), 720 * 405 * 4);
        assert!(matches!(
            decode_preview_png(&png_bytes(PREVIEW_MAXIMUM_EDGE + 1, 1)),
            Err(PreviewExtractionError::Decode(_))
        ));
        assert!(PreviewImage::from_rgba([2, 1], vec![0; 7]).is_none());
    }

    fn preview_request(identity: &SourceIdentity, media: &MediaFile) -> super::PreviewRequest {
        preview_request_at(identity, media, FrameTimestamp::ZERO)
    }

    fn preview_request_at(
        identity: &SourceIdentity,
        media: &MediaFile,
        timestamp: FrameTimestamp,
    ) -> super::PreviewRequest {
        let now = Instant::now();
        let mut preview = FramePreview::<()>::default();
        preview.schedule(identity, media, timestamp, now).unwrap();
        preview.take_due_request(now + PREVIEW_DEBOUNCE).unwrap()
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(width, height)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
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
