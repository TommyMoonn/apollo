use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::{model::MediaFile, probe::MediaProbeError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaProbeDisposition {
    Published,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaIntakeRequest {
    id: u64,
    path: PathBuf,
}

impl MediaIntakeRequest {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaLoadStart {
    Reused,
    Probe(MediaIntakeRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaIntakeError {
    Missing,
    Directory,
    Metadata(io::ErrorKind),
    Probe(MediaProbeError),
}

impl fmt::Display for MediaIntakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "The selected file no longer exists."),
            Self::Directory => write!(f, "Folders cannot be opened as media files."),
            Self::Metadata(kind) => {
                write!(f, "The selected file could not be inspected ({kind:?}).")
            }
            Self::Probe(error) => write!(f, "The selected file could not be probed: {error}"),
        }
    }
}

impl std::error::Error for MediaIntakeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    canonical_path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl FileIdentity {
    fn read(path: &Path) -> Result<Self, MediaIntakeError> {
        let metadata = fs::metadata(path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => MediaIntakeError::Missing,
            kind => MediaIntakeError::Metadata(kind),
        })?;
        if metadata.is_dir() {
            return Err(MediaIntakeError::Directory);
        }

        let canonical_path =
            fs::canonicalize(path).map_err(|error| MediaIntakeError::Metadata(error.kind()))?;
        Ok(Self {
            canonical_path,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }

    fn is_reliably_same(&self, other: &Self) -> bool {
        self.canonical_path == other.canonical_path
            && self.len == other.len
            && self.modified.is_some()
            && self.modified == other.modified
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedMedia {
    identity: FileIdentity,
    media: MediaFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingProbe {
    id: u64,
    identity: FileIdentity,
}

/// Owns the one reusable successful probe used by both media tools.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MediaIntake {
    current: Option<CachedMedia>,
    pending_probe: Option<PendingProbe>,
    next_request_id: u64,
}

impl MediaIntake {
    pub fn current(&self) -> Option<&MediaFile> {
        self.current.as_ref().map(|cached| &cached.media)
    }

    /// Return the cached probe only while the underlying file still matches the identity that was
    /// successfully probed. Changed, missing, or unreadable files retire the cache.
    pub fn current_if_unchanged(&mut self) -> Option<&MediaFile> {
        let (path, expected) = self
            .current
            .as_ref()
            .map(|cached| (cached.media.path.clone(), cached.identity.clone()))?;
        let unchanged =
            FileIdentity::read(&path).is_ok_and(|identity| expected.is_reliably_same(&identity));
        if !unchanged {
            self.clear_current();
        }
        self.current()
    }

    /// Validate a selected source and either reuse the unchanged cache or create the one
    /// authoritative probe request. A replacement remains pending until its probe succeeds, so
    /// the current source stays authoritative if validation or inspection fails.
    pub fn begin_load(&mut self, path: &Path) -> Result<MediaLoadStart, MediaIntakeError> {
        self.pending_probe = None;
        let identity = FileIdentity::read(path)?;

        if self
            .current
            .as_ref()
            .is_some_and(|cached| cached.identity.is_reliably_same(&identity))
        {
            return Ok(MediaLoadStart::Reused);
        }

        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending_probe = Some(PendingProbe { id, identity });
        Ok(MediaLoadStart::Probe(MediaIntakeRequest {
            id,
            path: path.to_path_buf(),
        }))
    }

    /// Publish a probe only when it still belongs to the current intake request.
    pub fn complete_probe(
        &mut self,
        request: MediaIntakeRequest,
        result: Result<MediaFile, MediaProbeError>,
    ) -> Result<MediaProbeDisposition, MediaIntakeError> {
        if !self
            .pending_probe
            .as_ref()
            .is_some_and(|pending| pending.id == request.id)
        {
            return Ok(MediaProbeDisposition::Retired);
        }

        let pending = self
            .pending_probe
            .take()
            .expect("matching pending probe exists while completion is published");
        let media = result.map_err(MediaIntakeError::Probe)?;
        self.current = Some(CachedMedia {
            identity: pending.identity,
            media,
        });
        Ok(MediaProbeDisposition::Published)
    }

    pub fn retire_probe(&mut self, request: &MediaIntakeRequest) {
        if self
            .pending_probe
            .as_ref()
            .is_some_and(|pending| pending.id == request.id)
        {
            self.pending_probe = None;
        }
    }

    pub fn close_source(&mut self) {
        self.pending_probe = None;
        self.clear_current();
    }

    fn clear_current(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::{MediaIntake, MediaIntakeError, MediaLoadStart, MediaProbeDisposition};
    use crate::media::{model::MediaFile, probe::MediaProbeError};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temp_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "apollo-phase35-{label}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("test directory should be created");
        path
    }

    fn media(path: &std::path::Path) -> MediaFile {
        MediaFile {
            path: path.to_path_buf(),
            duration: None,
            streams: Vec::new(),
            video_streams: Vec::new(),
            audio_streams: Vec::new(),
        }
    }

    fn inspect_successfully(intake: &mut MediaIntake, path: &std::path::Path) {
        let request = match intake.begin_load(path).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("new or changed source should require inspection"),
        };
        assert_eq!(
            intake.complete_probe(request, Ok(media(path))),
            Ok(MediaProbeDisposition::Published)
        );
    }

    #[test]
    fn valid_file_is_probed_and_stored() {
        let directory = temp_directory("valid");
        let path = directory.join("video.mkv");
        fs::write(&path, b"video").unwrap();
        let mut intake = MediaIntake::default();

        inspect_successfully(&mut intake, &path);
        assert_eq!(intake.current().map(|media| &media.path), Some(&path));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_is_rejected() {
        let directory = temp_directory("directory");
        let mut intake = MediaIntake::default();
        assert_eq!(
            intake.begin_load(&directory),
            Err(MediaIntakeError::Directory)
        );
        assert!(intake.current().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_path_is_rejected() {
        let directory = temp_directory("missing");
        let mut intake = MediaIntake::default();
        assert_eq!(
            intake.begin_load(&directory.join("missing.mkv")),
            Err(MediaIntakeError::Missing)
        );
        assert!(intake.current().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn probe_failure_preserves_previous_metadata() {
        let directory = temp_directory("probe-failure");
        let first = directory.join("first.mkv");
        let second = directory.join("second.mkv");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let mut intake = MediaIntake::default();
        inspect_successfully(&mut intake, &first);
        let request = match intake.begin_load(&second).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("replacement should require inspection"),
        };

        let result = intake.complete_probe(
            request,
            Err(MediaProbeError::FfprobeFailed {
                code: Some(1),
                diagnostic: "probe failed".to_owned(),
            }),
        );

        assert!(matches!(result, Err(MediaIntakeError::Probe(_))));
        assert_eq!(intake.current().map(|media| &media.path), Some(&first));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_replacement_preserves_previous_metadata() {
        let directory = temp_directory("invalid-replacement");
        let first = directory.join("first.mkv");
        fs::write(&first, b"first").unwrap();
        let mut intake = MediaIntake::default();
        inspect_successfully(&mut intake, &first);

        assert_eq!(
            intake.begin_load(&directory.join("missing.mkv")),
            Err(MediaIntakeError::Missing)
        );
        assert_eq!(intake.current().map(|media| &media.path), Some(&first));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn close_source_retires_current_metadata_and_pending_probe() {
        let directory = temp_directory("close-source");
        let first = directory.join("first.mkv");
        let second = directory.join("second.mkv");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let mut intake = MediaIntake::default();
        inspect_successfully(&mut intake, &first);
        let request = match intake.begin_load(&second).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("replacement should require inspection"),
        };

        intake.close_source();

        assert!(intake.current().is_none());
        assert_eq!(
            intake.complete_probe(request, Ok(media(&second))),
            Ok(MediaProbeDisposition::Retired)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replacing_file_replaces_cached_probe() {
        let directory = temp_directory("replace");
        let first = directory.join("first.mkv");
        let second = directory.join("second.mkv");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second source").unwrap();
        let mut intake = MediaIntake::default();
        inspect_successfully(&mut intake, &first);
        inspect_successfully(&mut intake, &second);

        assert_eq!(intake.current().map(|media| &media.path), Some(&second));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unchanged_file_reuses_successful_probe() {
        let directory = temp_directory("reuse");
        let path = directory.join("video.mkv");
        fs::write(&path, b"video").unwrap();
        let mut intake = MediaIntake::default();
        inspect_successfully(&mut intake, &path);

        assert_eq!(intake.begin_load(&path), Ok(MediaLoadStart::Reused));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changed_file_is_not_reused_as_current_probe() {
        let directory = temp_directory("changed-current");
        let path = directory.join("video.mkv");
        fs::write(&path, b"a").unwrap();
        let mut intake = MediaIntake::default();
        inspect_successfully(&mut intake, &path);
        fs::write(&path, b"changed-size").unwrap();

        assert!(intake.current_if_unchanged().is_none());
        assert!(intake.current().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changed_file_is_reprobed() {
        let directory = temp_directory("changed");
        let path = directory.join("video.mkv");
        fs::write(&path, b"a").unwrap();
        let mut intake = MediaIntake::default();
        inspect_successfully(&mut intake, &path);
        fs::write(&path, b"changed-size").unwrap();

        let request = match intake.begin_load(&path).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("changed source should require inspection"),
        };
        assert_eq!(
            intake.complete_probe(request, Ok(media(&path))),
            Ok(MediaProbeDisposition::Published)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn split_probe_publishes_only_after_completion() {
        let directory = temp_directory("split-probe");
        let path = directory.join("video.mkv");
        fs::write(&path, b"video").unwrap();
        let mut intake = MediaIntake::default();

        let request = match intake.begin_load(&path).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("new source should require inspection"),
        };
        assert!(intake.current().is_none());

        assert_eq!(
            intake.complete_probe(request, Ok(media(&path))),
            Ok(MediaProbeDisposition::Published)
        );
        assert_eq!(intake.current().map(|media| &media.path), Some(&path));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn retired_probe_cannot_overwrite_newer_intake_request() {
        let directory = temp_directory("retired-probe");
        let first = directory.join("first.mkv");
        let second = directory.join("second.mkv");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let mut intake = MediaIntake::default();

        let first_request = match intake.begin_load(&first).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("new source should require inspection"),
        };
        let second_request = match intake.begin_load(&second).unwrap() {
            MediaLoadStart::Probe(request) => request,
            MediaLoadStart::Reused => panic!("replacement should require inspection"),
        };

        assert_eq!(
            intake.complete_probe(first_request, Ok(media(&first))),
            Ok(MediaProbeDisposition::Retired)
        );
        assert!(intake.current().is_none());
        assert_eq!(
            intake.complete_probe(second_request, Ok(media(&second))),
            Ok(MediaProbeDisposition::Published)
        );
        assert_eq!(intake.current().map(|media| &media.path), Some(&second));
        fs::remove_dir_all(directory).unwrap();
    }
}
