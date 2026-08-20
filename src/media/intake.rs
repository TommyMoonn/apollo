use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::{model::MediaFile, probe::MediaProbeError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaLoadDisposition {
    Probed,
    Reused,
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

/// Owns the one reusable successful probe used by both media tools.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MediaIntake {
    current: Option<CachedMedia>,
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
            self.clear();
        }
        self.current()
    }

    fn clear(&mut self) {
        self.current = None;
    }

    pub fn load_with_probe<F>(
        &mut self,
        path: &Path,
        probe: F,
    ) -> Result<MediaLoadDisposition, MediaIntakeError>
    where
        F: FnOnce(&Path) -> Result<MediaFile, MediaProbeError>,
    {
        let identity = match FileIdentity::read(path) {
            Ok(identity) => identity,
            Err(error) => {
                self.clear();
                return Err(error);
            }
        };

        if self
            .current
            .as_ref()
            .is_some_and(|cached| cached.identity.is_reliably_same(&identity))
        {
            return Ok(MediaLoadDisposition::Reused);
        }

        // Retire previous metadata before probing a replacement so a failure cannot present stale
        // source details as if they belonged to the newly selected path.
        self.clear();
        let media = probe(path).map_err(MediaIntakeError::Probe)?;
        self.current = Some(CachedMedia { identity, media });
        Ok(MediaLoadDisposition::Probed)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::{MediaIntake, MediaIntakeError, MediaLoadDisposition};
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
            video_streams: Vec::new(),
            audio_streams: Vec::new(),
        }
    }

    #[test]
    fn valid_file_is_probed_and_stored() {
        let directory = temp_directory("valid");
        let path = directory.join("video.mkv");
        fs::write(&path, b"video").unwrap();
        let mut intake = MediaIntake::default();

        assert_eq!(
            intake.load_with_probe(&path, |path| Ok(media(path))),
            Ok(MediaLoadDisposition::Probed)
        );
        assert_eq!(intake.current().map(|media| &media.path), Some(&path));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_is_rejected() {
        let directory = temp_directory("directory");
        let mut intake = MediaIntake::default();
        assert_eq!(
            intake.load_with_probe(&directory, |path| Ok(media(path))),
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
            intake.load_with_probe(&directory.join("missing.mkv"), |path| Ok(media(path))),
            Err(MediaIntakeError::Missing)
        );
        assert!(intake.current().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn probe_failure_retires_previous_metadata() {
        let directory = temp_directory("probe-failure");
        let first = directory.join("first.mkv");
        let second = directory.join("second.mkv");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let mut intake = MediaIntake::default();
        intake
            .load_with_probe(&first, |path| Ok(media(path)))
            .unwrap();

        let result =
            intake.load_with_probe(&second, |_| Err(MediaProbeError::FfprobeFailed(Some(1))));

        assert!(matches!(result, Err(MediaIntakeError::Probe(_))));
        assert!(intake.current().is_none());
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
        intake
            .load_with_probe(&first, |path| Ok(media(path)))
            .unwrap();
        intake
            .load_with_probe(&second, |path| Ok(media(path)))
            .unwrap();

        assert_eq!(intake.current().map(|media| &media.path), Some(&second));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unchanged_file_reuses_successful_probe() {
        let directory = temp_directory("reuse");
        let path = directory.join("video.mkv");
        fs::write(&path, b"video").unwrap();
        let mut intake = MediaIntake::default();
        intake
            .load_with_probe(&path, |path| Ok(media(path)))
            .unwrap();
        let probes = AtomicUsize::new(0);

        let result = intake.load_with_probe(&path, |_| {
            probes.fetch_add(1, Ordering::Relaxed);
            unreachable!("unchanged file should reuse the cached probe")
        });

        assert_eq!(result, Ok(MediaLoadDisposition::Reused));
        assert_eq!(probes.load(Ordering::Relaxed), 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changed_file_is_not_reused_as_current_probe() {
        let directory = temp_directory("changed-current");
        let path = directory.join("video.mkv");
        fs::write(&path, b"a").unwrap();
        let mut intake = MediaIntake::default();
        intake
            .load_with_probe(&path, |path| Ok(media(path)))
            .unwrap();
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
        intake
            .load_with_probe(&path, |path| Ok(media(path)))
            .unwrap();
        fs::write(&path, b"changed-size").unwrap();
        let probes = AtomicUsize::new(0);

        let result = intake.load_with_probe(&path, |path| {
            probes.fetch_add(1, Ordering::Relaxed);
            Ok(media(path))
        });

        assert_eq!(result, Ok(MediaLoadDisposition::Probed));
        assert_eq!(probes.load(Ordering::Relaxed), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
