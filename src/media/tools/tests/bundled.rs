use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct BundleFixture(PathBuf);

impl BundleFixture {
    fn new() -> Self {
        let path = env::temp_dir().join(format!(
            "apollo bundled tools {}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn tool(&self, kind: MediaToolKind) -> PathBuf {
        self.0.join(format!(
            "{}{}",
            kind.executable_name(),
            env::consts::EXE_SUFFIX
        ))
    }

    fn add(&self, kind: MediaToolKind) {
        fs::write(self.tool(kind), b"fixture executable").unwrap();
    }

    fn complete(&self) {
        self.add(MediaToolKind::Ffmpeg);
        self.add(MediaToolKind::Ffprobe);
    }
}

impl Drop for BundleFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct BundleResolver<'a> {
    fixture: &'a BundleFixture,
    path_calls: RefCell<Vec<MediaToolKind>>,
}

impl<'a> BundleResolver<'a> {
    fn new(fixture: &'a BundleFixture) -> Self {
        Self {
            fixture,
            path_calls: RefCell::default(),
        }
    }
}

impl ExecutableResolver for BundleResolver<'_> {
    fn bundle_directory(&self) -> Result<Option<PathBuf>, MediaToolsError> {
        Ok(Some(self.fixture.0.clone()))
    }

    fn resolve(&self, tool: MediaToolKind) -> Option<PathBuf> {
        self.path_calls.borrow_mut().push(tool);
        complete_resolver().resolve(tool)
    }
}

#[test]
fn complete_bundle_wins_without_querying_path_and_probes_resolved_ffmpeg() {
    let fixture = BundleFixture::new();
    fixture.complete();
    let resolver = BundleResolver::new(&fixture);
    let validator = FakeValidator::default();
    let capabilities = VideoEncoderCapabilities {
        h264_nvenc: VideoEncoderCapability::Available,
        h265_nvenc: VideoEncoderCapability::RuntimeUnavailable {
            reason: "driver rejected HEVC".into(),
        },
    };
    let detector = FakeCapabilityDetector::new(capabilities.clone());
    let tools = discover_with(&resolver, &validator, |ffmpeg| detector.detect(ffmpeg)).unwrap();
    assert_eq!(tools.ffmpeg, fixture.tool(MediaToolKind::Ffmpeg));
    assert_eq!(tools.ffprobe, fixture.tool(MediaToolKind::Ffprobe));
    assert_eq!(tools.video_encoders, capabilities);
    assert!(resolver.path_calls.borrow().is_empty());
    assert_eq!(*detector.calls.borrow(), vec![tools.ffmpeg.clone()]);
    assert_eq!(
        *validator.calls.borrow(),
        vec![
            (MediaToolKind::Ffmpeg, tools.ffmpeg),
            (MediaToolKind::Ffprobe, tools.ffprobe)
        ]
    );
}

#[test]
fn absent_bundle_uses_path_pair() {
    let fixture = BundleFixture::new();
    let resolver = BundleResolver::new(&fixture);
    let tools = discover_with(&resolver, &FakeValidator::default(), no_video_encoders).unwrap();
    assert_eq!(tools.ffmpeg, PathBuf::from("/tools/ffmpeg"));
    assert_eq!(tools.ffprobe, PathBuf::from("/tools/ffprobe"));
    assert_eq!(
        *resolver.path_calls.borrow(),
        vec![MediaToolKind::Ffmpeg, MediaToolKind::Ffprobe]
    );
}

#[test]
fn either_partial_bundle_fails_without_mixing_or_probing_path_tools() {
    for (present, missing) in [
        (MediaToolKind::Ffmpeg, MediaToolKind::Ffprobe),
        (MediaToolKind::Ffprobe, MediaToolKind::Ffmpeg),
    ] {
        let fixture = BundleFixture::new();
        fixture.add(present);
        let resolver = BundleResolver::new(&fixture);
        let validator = FakeValidator::default();
        let error = discover_with(&resolver, &validator, |_| {
            panic!("must not probe incomplete pair")
        })
        .unwrap_err();
        assert_eq!(
            error,
            MediaToolsError::PartialBundle {
                directory: fixture.0.clone(),
                missing
            }
        );
        assert!(validator.calls.borrow().is_empty());
        assert!(resolver.path_calls.borrow().is_empty());
    }
}

#[test]
fn invalid_bundled_executables_retain_path_and_diagnostic_without_fallback() {
    for tool in [MediaToolKind::Ffmpeg, MediaToolKind::Ffprobe] {
        let fixture = BundleFixture::new();
        fixture.complete();
        let resolver = BundleResolver::new(&fixture);
        let reason = MediaToolValidationError::QueryFailed("required DLL missing".into());
        let error = discover_with(
            &resolver,
            &FakeValidator::failing(tool, reason.clone()),
            |_| panic!("invalid bundle must not probe"),
        )
        .unwrap_err();
        assert_eq!(
            error,
            MediaToolsError::ValidationFailed {
                tool,
                path: fixture.tool(tool),
                reason
            }
        );
        assert!(error.to_string().contains("required DLL missing"));
        assert!(resolver.path_calls.borrow().is_empty());
    }
}

#[test]
fn retry_observes_repaired_bundle_and_invalidates_path_capabilities() {
    let fixture = BundleFixture::new();
    let resolver = BundleResolver::new(&fixture);
    let mut cache = VideoEncoderCapabilityCache::default();
    let detector = FakeCapabilityDetector::new(VideoEncoderCapabilities::default());
    let path_tools = discover_with(&resolver, &FakeValidator::default(), |path| {
        cache.get_or_detect(path, &detector)
    })
    .unwrap();
    fixture.add(MediaToolKind::Ffmpeg);
    assert!(matches!(
        discover_with(&resolver, &FakeValidator::default(), no_video_encoders),
        Err(MediaToolsError::PartialBundle { .. })
    ));
    fixture.add(MediaToolKind::Ffprobe);
    let bundle_tools = discover_with(&resolver, &FakeValidator::default(), |path| {
        cache.get_or_detect(path, &detector)
    })
    .unwrap();
    discover_with(&resolver, &FakeValidator::default(), |path| {
        cache.get_or_detect(path, &detector)
    })
    .unwrap();
    assert_eq!(
        *detector.calls.borrow(),
        vec![path_tools.ffmpeg, bundle_tools.ffmpeg]
    );
}

#[test]
fn replacing_bundled_ffmpeg_invalidates_cached_capabilities() {
    let fixture = BundleFixture::new();
    fixture.complete();
    let path = fixture.tool(MediaToolKind::Ffmpeg);
    let mut cache = VideoEncoderCapabilityCache::default();
    let detector = FakeCapabilityDetector::new(VideoEncoderCapabilities::default());
    cache.get_or_detect(&path, &detector);
    fs::write(
        &path,
        b"different replacement executable with different length",
    )
    .unwrap();
    cache.get_or_detect(&path, &detector);
    assert_eq!(*detector.calls.borrow(), vec![path.clone(), path]);
}

#[test]
fn invalid_bundle_error_reaches_existing_user_error_owner() {
    use crate::media::error::{MediaError, MediaErrorKind};
    let error = MediaError::from_tools_error(&MediaToolsError::PartialBundle {
        directory: PathBuf::from("/Apollo installation"),
        missing: MediaToolKind::Ffprobe,
    });
    assert_eq!(error.kind(), MediaErrorKind::IncompleteMediaToolsBundle);
    assert!(
        error
            .guidance()
            .unwrap()
            .contains("both FFmpeg and FFprobe")
    );
    assert!(error.diagnostic().unwrap().contains("/Apollo installation"));
    assert!(!error.guidance().unwrap().contains("PATH"));
}
