use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use super::process::MediaProcessState;

const STAGING_DIRECTORY_PREFIX: &str = ".apollo-output";
const MAX_STAGING_DIRECTORY_ATTEMPTS: usize = 1024;
static NEXT_STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaOutputDisposition {
    Published(PathBuf),
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaOutputError {
    InvalidOutputPath(PathBuf),
    OutputParentUnavailable {
        path: PathBuf,
        kind: io::ErrorKind,
    },
    OutputParentNotDirectory(PathBuf),
    TemporaryDirectoryCreationFailed {
        path: PathBuf,
        kind: io::ErrorKind,
    },
    TemporaryOutputMissing(PathBuf),
    TemporaryOutputNotFile(PathBuf),
    ProcessStillRunning,
    AlreadySettled,
    PublishFailed {
        from: PathBuf,
        to: PathBuf,
        kind: io::ErrorKind,
    },
    CleanupFailed {
        path: PathBuf,
        kind: io::ErrorKind,
    },
}

impl fmt::Display for MediaOutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOutputPath(path) => {
                write!(f, "output path has no file name: {}", path.display())
            }
            Self::OutputParentUnavailable { path, kind } => {
                write!(
                    f,
                    "output parent is unavailable ({}): {kind:?}",
                    path.display()
                )
            }
            Self::OutputParentNotDirectory(path) => {
                write!(f, "output parent is not a directory: {}", path.display())
            }
            Self::TemporaryDirectoryCreationFailed { path, kind } => write!(
                f,
                "temporary output directory could not be created ({}): {kind:?}",
                path.display()
            ),
            Self::TemporaryOutputMissing(path) => {
                write!(f, "temporary output is missing: {}", path.display())
            }
            Self::TemporaryOutputNotFile(path) => {
                write!(f, "temporary output is not a file: {}", path.display())
            }
            Self::ProcessStillRunning => write!(f, "media process is still running"),
            Self::AlreadySettled => write!(f, "media output has already been settled"),
            Self::PublishFailed { from, to, kind } => write!(
                f,
                "temporary output could not be published ({} -> {}): {kind:?}",
                from.display(),
                to.display()
            ),
            Self::CleanupFailed { path, kind } => {
                write!(
                    f,
                    "temporary output could not be cleaned ({}): {kind:?}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for MediaOutputError {}

#[derive(Debug)]
pub struct MediaOutput {
    requested_path: PathBuf,
    suffix: u64,
    final_path: PathBuf,
    temporary_path: PathBuf,
    staging_directory: PathBuf,
    state: OutputState,
}

#[derive(Debug, PartialEq, Eq)]
enum OutputState {
    Pending,
    CleanupPending,
    Settled,
}

struct ValidatedOutputDestination {
    parent: PathBuf,
    file_name: std::ffi::OsString,
    final_path: PathBuf,
}

/// Validate only the filesystem-independent shape of an output path.
pub fn validate_output_path(output: &Path) -> Result<(), MediaOutputError> {
    output
        .file_name()
        .filter(|name| !name.is_empty() && !name.as_encoded_bytes().contains(&0))
        .map(|_| ())
        .ok_or_else(|| MediaOutputError::InvalidOutputPath(output.to_path_buf()))
}

/// Check the selected extension without accessing the filesystem. The error carries the
/// actual extension (if valid UTF-8), so each domain can retain its own diagnostic.
pub(crate) fn check_output_extension(output: &Path, expected: &str) -> Result<(), Option<String>> {
    let actual = output.extension().and_then(|extension| extension.to_str());
    if actual.is_some_and(|extension| extension.eq_ignore_ascii_case(expected)) {
        Ok(())
    } else {
        Err(actual.map(str::to_owned))
    }
}

/// Perform the filesystem-backed destination safety preflight without creating staging state.
pub fn preflight_output_destination(output: &Path) -> Result<(), MediaOutputError> {
    validated_output_destination(output).map(|_| ())
}

fn validated_output_destination(
    output: &Path,
) -> Result<ValidatedOutputDestination, MediaOutputError> {
    validate_output_path(output)?;
    let file_name = output
        .file_name()
        .expect("validated output path has a file name")
        .to_os_string();
    let requested_parent = output_parent(output);
    let parent_metadata = fs::metadata(&requested_parent).map_err(|error| {
        MediaOutputError::OutputParentUnavailable {
            path: requested_parent.clone(),
            kind: error.kind(),
        }
    })?;

    if !parent_metadata.is_dir() {
        return Err(MediaOutputError::OutputParentNotDirectory(requested_parent));
    }

    let parent = std::path::absolute(&requested_parent).map_err(|error| {
        MediaOutputError::OutputParentUnavailable {
            path: requested_parent.clone(),
            kind: error.kind(),
        }
    })?;
    let final_path = parent.join(&file_name);
    Ok(ValidatedOutputDestination {
        parent,
        file_name,
        final_path,
    })
}

impl MediaOutput {
    pub fn prepare(output: &Path) -> Result<Self, MediaOutputError> {
        let destination = validated_output_destination(output)?;
        // Choose the name once at execution start for progress presentation. This is advisory:
        // if inspection fails or another writer claims the name, publication remains authoritative.
        let requested_path = destination.final_path;
        let mut final_path = requested_path.clone();
        let mut suffix = 0;
        while fs::symlink_metadata(&final_path).is_ok() {
            suffix += 1;
            final_path = numbered_output_path(&requested_path, suffix);
        }
        let staging_directory = create_staging_directory(&destination.parent)?;
        let temporary_path = staging_directory.join(&destination.file_name);

        Ok(Self {
            requested_path,
            suffix,
            final_path,
            temporary_path,
            staging_directory,
            state: OutputState::Pending,
        })
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn temporary_path(&self) -> &Path {
        &self.temporary_path
    }

    pub fn settle(
        &mut self,
        process_state: &MediaProcessState,
    ) -> Result<MediaOutputDisposition, MediaOutputError> {
        if self.state != OutputState::Pending {
            return Err(MediaOutputError::AlreadySettled);
        }
        if *process_state == MediaProcessState::Running {
            return Err(MediaOutputError::ProcessStillRunning);
        }

        // A terminal settlement is never retried as publication, even on failure.
        self.state = OutputState::CleanupPending;
        match process_state {
            MediaProcessState::Completed => {
                let result = self.publish();
                if result.is_ok() {
                    // Successful publication relinquishes the staged name.
                    self.state = OutputState::Settled;
                    let _ = remove_empty_staging_directory(&self.staging_directory);
                } else {
                    self.finish_cleanup();
                }
                result
            }
            MediaProcessState::Failed(_) | MediaProcessState::Cancelled => {
                self.cleanup_owned_temporary_output()?;
                self.state = OutputState::Settled;
                Ok(MediaOutputDisposition::Discarded)
            }
            MediaProcessState::Running => unreachable!("running state rejected before settlement"),
        }
    }

    fn finish_cleanup(&mut self) {
        if self.cleanup_owned_temporary_output().is_ok() {
            self.state = OutputState::Settled;
        }
    }

    fn publish(&mut self) -> Result<MediaOutputDisposition, MediaOutputError> {
        let temporary_metadata = match fs::symlink_metadata(&self.temporary_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(MediaOutputError::TemporaryOutputMissing(
                    self.temporary_path.clone(),
                ));
            }
            Err(error) => {
                return Err(MediaOutputError::PublishFailed {
                    from: self.temporary_path.clone(),
                    to: self.final_path.clone(),
                    kind: error.kind(),
                });
            }
        };

        if !temporary_metadata.file_type().is_file() {
            return Err(MediaOutputError::TemporaryOutputNotFile(
                self.temporary_path.clone(),
            ));
        }

        // Begin with the name shown during execution. Only the no-replace move can claim it.
        let mut candidate = self.final_path.clone();
        for index in self.suffix + 1.. {
            match publish_file_no_replace(&self.temporary_path, &candidate) {
                Ok(()) => break,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    candidate = numbered_output_path(&self.requested_path, index);
                }
                Err(error) => {
                    return Err(MediaOutputError::PublishFailed {
                        from: self.temporary_path.clone(),
                        to: candidate,
                        kind: error.kind(),
                    });
                }
            }
        }
        self.final_path = candidate;

        Ok(MediaOutputDisposition::Published(self.final_path.clone()))
    }

    fn cleanup_owned_temporary_output(&self) -> Result<(), MediaOutputError> {
        match fs::symlink_metadata(&self.temporary_path) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                return Err(MediaOutputError::TemporaryOutputNotFile(
                    self.temporary_path.clone(),
                ));
            }
            Ok(_) => fs::remove_file(&self.temporary_path).map_err(|error| {
                MediaOutputError::CleanupFailed {
                    path: self.temporary_path.clone(),
                    kind: error.kind(),
                }
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(MediaOutputError::CleanupFailed {
                    path: self.temporary_path.clone(),
                    kind: error.kind(),
                });
            }
        }

        remove_empty_staging_directory(&self.staging_directory)
    }
}

impl Drop for MediaOutput {
    fn drop(&mut self) {
        if self.state != OutputState::Settled {
            let _ = self.cleanup_owned_temporary_output();
        }
    }
}

fn output_parent(output: &Path) -> PathBuf {
    match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn numbered_output_path(path: &Path, index: u64) -> PathBuf {
    let mut name = path.file_stem().unwrap_or_default().to_os_string();
    name.push(format!(" ({index})"));
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    path.with_file_name(name)
}

fn create_staging_directory(parent: &Path) -> Result<PathBuf, MediaOutputError> {
    let process_id = process::id();
    let mut last_collision = None;

    for _ in 0..MAX_STAGING_DIRECTORY_ATTEMPTS {
        let sequence = NEXT_STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = staging_directory_candidate(parent, process_id, sequence);
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(candidate);
            }
            Err(error) => {
                return Err(MediaOutputError::TemporaryDirectoryCreationFailed {
                    path: candidate,
                    kind: error.kind(),
                });
            }
        }
    }

    Err(MediaOutputError::TemporaryDirectoryCreationFailed {
        path: last_collision.unwrap_or_else(|| parent.to_path_buf()),
        kind: io::ErrorKind::AlreadyExists,
    })
}

fn staging_directory_candidate(parent: &Path, process_id: u32, sequence: u64) -> PathBuf {
    parent.join(format!(
        "{STAGING_DIRECTORY_PREFIX}-{process_id}-{sequence:016x}"
    ))
}

#[cfg(target_os = "linux")]
fn publish_file_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::{
        ffi::CString,
        os::{
            raw::{c_char, c_int, c_uint},
            unix::ffi::OsStrExt,
        },
    };

    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;

    #[link(name = "c")]
    unsafe extern "C" {
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let to = CString::new(to.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;

    // SAFETY: both C strings are NUL-terminated and remain alive for the call.
    let result = unsafe {
        renameat2(
            AT_FDCWD,
            from.as_ptr(),
            AT_FDCWD,
            to.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn publish_file_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileW(existing_file_name: *const u16, new_file_name: *const u16) -> i32;
    }

    let from: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(iter::once(0)).collect();

    // SAFETY: both buffers are NUL-terminated and remain alive for the call.
    let result = unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) };
    if result != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn publish_file_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::hard_link(from, to)?;
    match fs::remove_file(from) {
        Ok(()) => Ok(()),
        Err(error) => {
            // The final path is already a complete hard link. An unlink failure cannot
            // undo publication or justify replacing the destination.
            let _ = error;
            Ok(())
        }
    }
}

fn remove_empty_staging_directory(path: &Path) -> Result<(), MediaOutputError> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) => Err(MediaOutputError::CleanupFailed {
            path: path.to_path_buf(),
            kind: error.kind(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        MediaOutput, MediaOutputDisposition, MediaOutputError, STAGING_DIRECTORY_PREFIX,
        check_output_extension, preflight_output_destination, validate_output_path,
    };
    use crate::media::process::{MediaProcessFailure, MediaProcessState};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn output_path_validation_is_filesystem_independent() {
        let output = PathBuf::from("missing-parent").join("output.mp4");

        assert_eq!(validate_output_path(&output), Ok(()));
        assert_eq!(
            validate_output_path(Path::new("")),
            Err(MediaOutputError::InvalidOutputPath(PathBuf::new()))
        );
        assert_eq!(
            check_output_extension(Path::new("missing-parent/movie.MP4"), "mp4"),
            Ok(())
        );
        assert_eq!(
            check_output_extension(Path::new("movie.mkv"), "mp4"),
            Err(Some("mkv".to_owned()))
        );
        assert_eq!(check_output_extension(Path::new("movie"), "mp4"), Err(None));
    }

    #[test]
    fn nonexistent_output_parent_is_rejected() {
        let directory = TestDirectory::new("missing-parent");
        let input = directory.path().join("input.mkv");
        fs::write(&input, b"input").unwrap();
        let output = directory.path().join("missing").join("output.mp4");

        let error = MediaOutput::prepare(&output).unwrap_err();

        assert!(matches!(
            error,
            MediaOutputError::OutputParentUnavailable {
                kind: std::io::ErrorKind::NotFound,
                ..
            }
        ));
    }

    #[test]
    fn output_parent_must_be_a_directory() {
        let directory = TestDirectory::new("parent-not-directory");
        let input = directory.path().join("input.mkv");
        let parent_file = directory.path().join("not-a-directory");
        fs::write(&input, b"input").unwrap();
        fs::write(&parent_file, b"file").unwrap();
        let output = parent_file.join("output.mp4");

        let error = MediaOutput::prepare(&output).unwrap_err();

        assert!(matches!(
            error,
            MediaOutputError::OutputParentNotDirectory(_)
        ));
    }

    #[test]
    fn destination_preflight_does_not_create_staging_state() {
        let directory = TestDirectory::new("preflight-no-staging");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();

        assert_eq!(preflight_output_destination(&output), Ok(()));
        assert!(!output.exists());
        assert!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(STAGING_DIRECTORY_PREFIX))
        );
    }

    #[test]
    fn temporary_paths_are_unique_adjacent_and_preserve_output_name() {
        let directory = TestDirectory::new("temporary-path");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("movie final.mp4");
        fs::write(&input, b"input").unwrap();
        let prepared = MediaOutput::prepare(&output).unwrap();

        assert_eq!(prepared.temporary_path().file_name(), output.file_name());
        assert_eq!(prepared.final_path(), output);
        assert_eq!(
            prepared.temporary_path().parent().unwrap().parent(),
            output.parent()
        );
        assert!(!prepared.temporary_path().exists());
        assert!(!prepared.final_path().exists());
        let second = MediaOutput::prepare(&output).unwrap();
        assert_ne!(prepared.temporary_path(), second.temporary_path());
    }

    #[test]
    fn successful_process_publishes_only_the_completed_temporary_output() {
        let directory = TestDirectory::new("publish-success");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        let temporary = prepared.temporary_path().to_path_buf();
        let staging = temporary.parent().unwrap().to_path_buf();
        fs::write(&temporary, b"complete-output").unwrap();

        let disposition = prepared.settle(&MediaProcessState::Completed).unwrap();

        assert_eq!(
            disposition,
            MediaOutputDisposition::Published(prepared.final_path().to_path_buf())
        );
        assert_eq!(fs::read(prepared.final_path()).unwrap(), b"complete-output");
        assert!(!temporary.exists());
        assert!(!staging.exists());
        assert_eq!(
            prepared.settle(&MediaProcessState::Completed),
            Err(MediaOutputError::AlreadySettled)
        );
    }

    #[test]
    fn failed_publication_is_terminal_and_cleans_owned_staging() {
        let directory = TestDirectory::new("publication-failure");
        let output = directory.path().join("result.mp4");
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        let temporary = prepared.temporary_path().to_path_buf();
        let staging = temporary.parent().unwrap();
        assert_eq!(
            prepared.settle(&MediaProcessState::Completed),
            Err(MediaOutputError::TemporaryOutputMissing(temporary.clone()))
        );
        assert!(!staging.exists());
        assert!(!output.exists());
        assert_eq!(
            prepared.settle(&MediaProcessState::Completed),
            Err(MediaOutputError::AlreadySettled)
        );
        assert_eq!(
            prepared.settle(&MediaProcessState::Cancelled),
            Err(MediaOutputError::AlreadySettled)
        );
    }

    #[cfg(unix)]
    #[test]
    fn destination_parent_preserves_symlink_parent_traversal() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new("symlink-parent");
        let parent = directory.path().join("actual");
        let nested = parent.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let link = directory.path().join("link");
        symlink(&nested, &link).unwrap();
        let output = link.join("..").join("result.mp4");
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        fs::write(prepared.temporary_path(), b"complete").unwrap();
        prepared.settle(&MediaProcessState::Completed).unwrap();
        assert_eq!(fs::read(parent.join("result.mp4")).unwrap(), b"complete");
        assert!(!directory.path().join("result.mp4").exists());
    }

    #[test]
    fn failed_process_cleans_temporary_output_without_publishing() {
        let directory = TestDirectory::new("failure-cleanup");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        let temporary = prepared.temporary_path().to_path_buf();
        let staging = temporary.parent().unwrap().to_path_buf();
        fs::write(&temporary, b"partial").unwrap();

        assert_eq!(
            prepared
                .settle(&MediaProcessState::Failed(MediaProcessFailure::Exit(Some(
                    1
                ))))
                .unwrap(),
            MediaOutputDisposition::Discarded
        );
        assert!(!output.exists());
        assert!(!temporary.exists());
        assert!(!staging.exists());
    }

    #[test]
    fn cancelled_process_cleans_temporary_output_without_publishing() {
        let directory = TestDirectory::new("cancel-cleanup");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        let temporary = prepared.temporary_path().to_path_buf();
        fs::write(&temporary, b"partial").unwrap();

        assert_eq!(
            prepared.settle(&MediaProcessState::Cancelled).unwrap(),
            MediaOutputDisposition::Discarded
        );
        assert!(!output.exists());
        assert!(!temporary.exists());
    }

    #[test]
    fn cleanup_does_not_delete_unrelated_files() {
        let directory = TestDirectory::new("unrelated-file");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        fs::write(prepared.temporary_path(), b"partial").unwrap();
        let unrelated = prepared
            .temporary_path()
            .parent()
            .unwrap()
            .join("keep-me.txt");
        fs::write(&unrelated, b"important").unwrap();

        prepared.settle(&MediaProcessState::Cancelled).unwrap();

        assert_eq!(fs::read(&unrelated).unwrap(), b"important");
        assert!(unrelated.parent().unwrap().exists());
    }

    #[test]
    fn unicode_and_space_containing_paths_publish_successfully() {
        let directory = TestDirectory::new("unicode-space");
        let input = directory.path().join("入力 source file.mkv");
        let output = directory.path().join("結果 final movie.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        fs::write(prepared.temporary_path(), b"unicode-output").unwrap();

        prepared.settle(&MediaProcessState::Completed).unwrap();

        assert_eq!(fs::read(&output).unwrap(), b"unicode-output");
    }

    #[test]
    fn running_process_cannot_publish_or_discard_output() {
        let directory = TestDirectory::new("running-state");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        fs::write(prepared.temporary_path(), b"partial").unwrap();

        assert_eq!(
            prepared.settle(&MediaProcessState::Running),
            Err(MediaOutputError::ProcessStillRunning)
        );
        assert!(prepared.temporary_path().exists());
        assert!(!output.exists());
    }

    #[test]
    fn no_clobber_publication_primitive_refuses_existing_destination() {
        let directory = TestDirectory::new("no-clobber-primitive");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let prepared = MediaOutput::prepare(&output).unwrap();
        let temporary = prepared.temporary_path().to_path_buf();
        let unrelated = prepared
            .temporary_path()
            .parent()
            .unwrap()
            .join("keep-me.txt");

        fs::write(&temporary, b"apollo-complete-output").unwrap();
        fs::write(&output, b"concurrent-owner").unwrap();
        fs::write(&unrelated, b"unrelated").unwrap();

        let error = super::publish_file_no_replace(&temporary, &output).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&output).unwrap(), b"concurrent-owner");
        assert_eq!(fs::read(&temporary).unwrap(), b"apollo-complete-output");

        prepared.cleanup_owned_temporary_output().unwrap();

        assert!(!temporary.exists());
        assert_eq!(fs::read(&output).unwrap(), b"concurrent-owner");
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated");
    }

    #[test]
    fn collisions_publish_numbered_files_without_replacing_existing_bytes() {
        let directory = TestDirectory::new("numbered-output");
        let output = directory.path().join("movie final.mp4");
        let first = directory.path().join("movie final (1).mp4");
        let second = directory.path().join("movie final (2).mp4");
        fs::write(&output, b"original source or existing output").unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        fs::write(prepared.temporary_path(), b"complete").unwrap();
        assert_eq!(prepared.final_path(), first);
        // Occupy the first suffix after preparation, not before.
        fs::write(&first, b"concurrent owner").unwrap();
        assert_eq!(
            prepared.settle(&MediaProcessState::Completed).unwrap(),
            MediaOutputDisposition::Published(second.clone())
        );
        assert_eq!(
            fs::read(&output).unwrap(),
            b"original source or existing output"
        );
        assert_eq!(fs::read(&first).unwrap(), b"concurrent owner");
        assert_eq!(fs::read(&second).unwrap(), b"complete");
        assert!(!prepared.temporary_path().exists());

        let mut next = MediaOutput::prepare(&output).unwrap();
        let third = directory.path().join("movie final (3).mp4");
        assert_eq!(next.final_path(), third);
        fs::remove_file(&first).unwrap();
        fs::write(next.temporary_path(), b"next complete").unwrap();
        assert_eq!(
            next.settle(&MediaProcessState::Completed).unwrap(),
            MediaOutputDisposition::Published(third.clone())
        );
        assert_eq!(fs::read(third).unwrap(), b"next complete");
        assert!(!first.exists());
    }

    #[cfg(unix)]
    #[test]
    fn existing_source_alias_is_not_replaced() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new("source-alias");
        let source = directory.path().join("source.mkv");
        let alias = directory.path().join("alias.mkv");
        fs::write(&source, b"source").unwrap();
        symlink(&source, &alias).unwrap();
        let mut prepared = MediaOutput::prepare(&alias).unwrap();
        let numbered = directory.path().join("alias (1).mkv");
        assert_eq!(prepared.final_path(), numbered);
        fs::write(prepared.temporary_path(), b"completed output").unwrap();
        assert_eq!(
            prepared.settle(&MediaProcessState::Completed).unwrap(),
            MediaOutputDisposition::Published(numbered.clone())
        );
        assert!(
            fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&alias).unwrap(), b"source");
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(numbered).unwrap(), b"completed output");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_not_replaced() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new("dangling-output");
        let output = directory.path().join("result.mp4");
        let target = directory.path().join("absent");
        symlink(&target, &output).unwrap();
        let mut prepared = MediaOutput::prepare(&output).unwrap();
        fs::write(prepared.temporary_path(), b"complete").unwrap();
        let numbered = directory.path().join("result (1).mp4");
        assert_eq!(prepared.final_path(), numbered);
        assert_eq!(
            prepared.settle(&MediaProcessState::Completed).unwrap(),
            MediaOutputDisposition::Published(numbered.clone())
        );
        assert_eq!(fs::read_link(&output).unwrap(), target);
        assert!(!target.exists());
        assert_eq!(fs::read(numbered).unwrap(), b"complete");
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "apollo-output-test-{}-{sequence}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
