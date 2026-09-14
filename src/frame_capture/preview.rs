use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::media::{intake::SourceIdentity, model::MediaFile};

use super::{model::FrameTimestamp, validation::validate_frame_timestamp};

pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(200);

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
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    use crate::media::{
        intake::SourceIdentity,
        model::{MediaFile, VideoStream},
    };

    use super::{FramePreview, PREVIEW_DEBOUNCE, PreviewState};
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
}
