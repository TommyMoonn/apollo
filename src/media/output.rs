use std::{
    env, fmt, fs, io,
    path::{Component, Path, PathBuf},
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
    SameInputAndOutput(PathBuf),
    OutputParentUnavailable {
        path: PathBuf,
        kind: io::ErrorKind,
    },
    OutputParentNotDirectory(PathBuf),
    OutputAlreadyExists(PathBuf),
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
            Self::SameInputAndOutput(path) => {
                write!(f, "input and output must differ: {}", path.display())
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
            Self::OutputAlreadyExists(path) => {
                write!(f, "output already exists: {}", path.display())
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
    final_path: PathBuf,
    temporary_path: PathBuf,
    staging_directory: PathBuf,
    settled: bool,
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
        .filter(|name| !name.is_empty())
        .map(|_| ())
        .ok_or_else(|| MediaOutputError::InvalidOutputPath(output.to_path_buf()))
}

/// Perform the filesystem-backed destination safety preflight without creating staging state.
pub fn preflight_output_destination(input: &Path, output: &Path) -> Result<(), MediaOutputError> {
    validated_output_destination(input, output).map(|_| ())
}

fn validated_output_destination(
    input: &Path,
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

    let parent = absolute_lexical(&requested_parent).map_err(|error| {
        MediaOutputError::OutputParentUnavailable {
            path: requested_parent.clone(),
            kind: error.kind(),
        }
    })?;
    let final_path = parent.join(&file_name);
    let input_path =
        comparison_path(input).map_err(|error| MediaOutputError::OutputParentUnavailable {
            path: input.to_path_buf(),
            kind: error.kind(),
        })?;
    let output_path = comparison_path(&final_path).map_err(|error| {
        MediaOutputError::OutputParentUnavailable {
            path: final_path.clone(),
            kind: error.kind(),
        }
    })?;

    if paths_equal(&input_path, &output_path) {
        return Err(MediaOutputError::SameInputAndOutput(final_path));
    }

    if path_exists_without_following(&final_path)? {
        return Err(MediaOutputError::OutputAlreadyExists(final_path));
    }

    Ok(ValidatedOutputDestination {
        parent,
        file_name,
        final_path,
    })
}

impl MediaOutput {
    pub fn prepare(input: &Path, output: &Path) -> Result<Self, MediaOutputError> {
        let destination = validated_output_destination(input, output)?;
        let staging_directory = create_staging_directory(&destination.parent)?;
        let temporary_path = staging_directory.join(&destination.file_name);

        Ok(Self {
            final_path: destination.final_path,
            temporary_path,
            staging_directory,
            settled: false,
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
        if self.settled {
            return Err(MediaOutputError::AlreadySettled);
        }

        match process_state {
            MediaProcessState::Running => Err(MediaOutputError::ProcessStillRunning),
            MediaProcessState::Completed => self.publish(),
            MediaProcessState::Failed(_) | MediaProcessState::Cancelled => self.discard(),
        }
    }

    fn publish(&mut self) -> Result<MediaOutputDisposition, MediaOutputError> {
        if path_exists_without_following(&self.final_path)? {
            let _ = self.cleanup_owned_temporary_output();
            return Err(MediaOutputError::OutputAlreadyExists(
                self.final_path.clone(),
            ));
        }

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

        match publish_file_no_replace(&self.temporary_path, &self.final_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = self.cleanup_owned_temporary_output();
                return Err(MediaOutputError::OutputAlreadyExists(
                    self.final_path.clone(),
                ));
            }
            Err(error) => {
                return Err(MediaOutputError::PublishFailed {
                    from: self.temporary_path.clone(),
                    to: self.final_path.clone(),
                    kind: error.kind(),
                });
            }
        }

        self.settled = true;
        let _ = remove_empty_staging_directory(&self.staging_directory);
        Ok(MediaOutputDisposition::Published(self.final_path.clone()))
    }

    fn discard(&mut self) -> Result<MediaOutputDisposition, MediaOutputError> {
        self.cleanup_owned_temporary_output()?;
        self.settled = true;
        Ok(MediaOutputDisposition::Discarded)
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
        if !self.settled {
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

fn comparison_path(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => absolute_lexical(path),
        Err(error) => Err(error),
    }
}

fn absolute_lexical(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };

    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }

    #[cfg(not(windows))]
    {
        left == right
    }
}

fn path_exists_without_following(path: &Path) -> Result<bool, MediaOutputError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(MediaOutputError::OutputParentUnavailable {
            path: path.to_path_buf(),
            kind: error.kind(),
        }),
    }
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
            // The final path is already a complete hard link at this point. Publication
            // succeeded; leave best-effort temporary cleanup to the MediaOutput owner.
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
        preflight_output_destination, staging_directory_candidate, validate_output_path,
    };
    use crate::media::process::{MediaProcessFailure, MediaProcessState};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn same_input_and_output_is_rejected_before_existing_output_check() {
        let directory = TestDirectory::new("same-path");
        let input = directory.path().join("movie.mp4");
        fs::write(&input, b"input").unwrap();

        let error = MediaOutput::prepare(&input, &input).unwrap_err();

        assert!(matches!(error, MediaOutputError::SameInputAndOutput(_)));
    }

    #[test]
    fn output_path_validation_is_filesystem_independent() {
        let output = PathBuf::from("missing-parent").join("output.mp4");

        assert_eq!(validate_output_path(&output), Ok(()));
        assert_eq!(
            validate_output_path(Path::new("")),
            Err(MediaOutputError::InvalidOutputPath(PathBuf::new()))
        );
    }

    #[test]
    fn canonical_equivalent_input_and_output_are_rejected() {
        let directory = TestDirectory::new("canonical-same-path");
        let nested = directory.path().join("nested");
        fs::create_dir(&nested).unwrap();
        let input = directory.path().join("movie.mp4");
        fs::write(&input, b"input").unwrap();
        let output = nested.join("..").join("movie.mp4");

        let error = preflight_output_destination(&input, &output).unwrap_err();

        assert!(matches!(error, MediaOutputError::SameInputAndOutput(_)));
    }

    #[test]
    fn nonexistent_output_parent_is_rejected() {
        let directory = TestDirectory::new("missing-parent");
        let input = directory.path().join("input.mkv");
        fs::write(&input, b"input").unwrap();
        let output = directory.path().join("missing").join("output.mp4");

        let error = MediaOutput::prepare(&input, &output).unwrap_err();

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

        let error = MediaOutput::prepare(&input, &output).unwrap_err();

        assert!(matches!(
            error,
            MediaOutputError::OutputParentNotDirectory(_)
        ));
    }

    #[test]
    fn preexisting_output_is_protected() {
        let directory = TestDirectory::new("existing-output");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        fs::write(&output, b"existing").unwrap();

        let error = MediaOutput::prepare(&input, &output).unwrap_err();

        assert!(matches!(error, MediaOutputError::OutputAlreadyExists(_)));
        assert_eq!(fs::read(&output).unwrap(), b"existing");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_output_symlink_is_treated_as_an_existing_destination() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("dangling-output-symlink");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        symlink(directory.path().join("missing-target"), &output).unwrap();

        let error = preflight_output_destination(&input, &output).unwrap_err();

        assert!(matches!(error, MediaOutputError::OutputAlreadyExists(_)));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_alias_to_input_is_rejected_as_the_same_file() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("symlink-input-alias");
        let real_parent = directory.path().join("real");
        let alias_parent = directory.path().join("alias");
        fs::create_dir(&real_parent).unwrap();
        symlink(&real_parent, &alias_parent).unwrap();
        let input = real_parent.join("movie.mp4");
        let output = alias_parent.join("movie.mp4");
        fs::write(&input, b"input").unwrap();

        let error = preflight_output_destination(&input, &output).unwrap_err();

        assert!(matches!(error, MediaOutputError::SameInputAndOutput(_)));
    }

    #[test]
    fn destination_preflight_does_not_create_staging_state() {
        let directory = TestDirectory::new("preflight-no-staging");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();

        assert_eq!(preflight_output_destination(&input, &output), Ok(()));
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
    fn temporary_path_is_adjacent_deterministic_and_preserves_output_name() {
        let directory = TestDirectory::new("temporary-path");
        let candidate = staging_directory_candidate(directory.path(), 42, 7);
        assert_eq!(
            candidate.file_name().unwrap(),
            ".apollo-output-42-0000000000000007"
        );

        let input = directory.path().join("input.mkv");
        let output = directory.path().join("movie final.mp4");
        fs::write(&input, b"input").unwrap();
        let prepared = MediaOutput::prepare(&input, &output).unwrap();

        assert_eq!(prepared.temporary_path().file_name(), output.file_name());
        assert_eq!(prepared.final_path(), output);
        assert_eq!(
            prepared.temporary_path().parent().unwrap().parent(),
            output.parent()
        );
        assert!(!prepared.temporary_path().exists());
        assert!(!prepared.final_path().exists());
    }

    #[test]
    fn successful_process_publishes_only_the_completed_temporary_output() {
        let directory = TestDirectory::new("publish-success");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&input, &output).unwrap();
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
    }

    #[test]
    fn failed_process_cleans_temporary_output_without_publishing() {
        let directory = TestDirectory::new("failure-cleanup");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&input, &output).unwrap();
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
        let mut prepared = MediaOutput::prepare(&input, &output).unwrap();
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
        let mut prepared = MediaOutput::prepare(&input, &output).unwrap();
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
        let mut prepared = MediaOutput::prepare(&input, &output).unwrap();
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
        let mut prepared = MediaOutput::prepare(&input, &output).unwrap();
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
        let prepared = MediaOutput::prepare(&input, &output).unwrap();
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
    fn output_created_after_prepare_is_not_overwritten() {
        let directory = TestDirectory::new("late-existing-output");
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mp4");
        fs::write(&input, b"input").unwrap();
        let mut prepared = MediaOutput::prepare(&input, &output).unwrap();
        fs::write(prepared.temporary_path(), b"complete").unwrap();
        fs::write(&output, b"someone-else").unwrap();

        let error = prepared.settle(&MediaProcessState::Completed).unwrap_err();

        assert!(matches!(error, MediaOutputError::OutputAlreadyExists(_)));
        assert_eq!(fs::read(&output).unwrap(), b"someone-else");
        assert!(!prepared.temporary_path().exists());
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
