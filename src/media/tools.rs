use std::{
    fmt, io,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(not(windows))]
use std::env;

#[cfg(windows)]
use std::{
    ffi::{OsStr, OsString},
    iter,
    os::windows::ffi::{OsStrExt, OsStringExt},
    ptr,
};

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::SearchPathW;

use super::process::background_command;

const VERSION_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const VERSION_QUERY_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaToolKind {
    Ffmpeg,
    Ffprobe,
}

impl MediaToolKind {
    fn executable_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "FFmpeg",
            Self::Ffprobe => "FFprobe",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaToolsError {
    DiscoveryFailed(String),
    MissingFfmpeg,
    MissingFfprobe,
    MissingBoth,
    ValidationFailed {
        tool: MediaToolKind,
        path: PathBuf,
        reason: MediaToolValidationError,
    },
}

impl MediaToolsError {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::DiscoveryFailed(_) => "Media tools could not be checked.",
            Self::MissingFfmpeg => "FFmpeg could not be found.",
            Self::MissingFfprobe => "FFprobe could not be found.",
            Self::MissingBoth => "FFmpeg and FFprobe could not be found.",
            Self::ValidationFailed {
                tool: MediaToolKind::Ffmpeg,
                ..
            } => "FFmpeg was found but could not be validated.",
            Self::ValidationFailed {
                tool: MediaToolKind::Ffprobe,
                ..
            } => "FFprobe was found but could not be validated.",
        }
    }
}

impl fmt::Display for MediaToolsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiscoveryFailed(diagnostic) => {
                write!(f, "Media tool discovery failed: {diagnostic}")
            }
            Self::MissingFfmpeg => write!(f, "FFmpeg could not be found"),
            Self::MissingFfprobe => write!(f, "FFprobe could not be found"),
            Self::MissingBoth => write!(f, "FFmpeg and FFprobe could not be found"),
            Self::ValidationFailed { tool, path, reason } => write!(
                f,
                "{} at '{}' could not be validated: {reason}",
                tool.display_name(),
                path.display()
            ),
        }
    }
}

impl std::error::Error for MediaToolsError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaToolValidationError {
    LaunchFailed(io::ErrorKind),
    TimedOut,
    NonZeroExit(Option<i32>),
}

impl fmt::Display for MediaToolValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LaunchFailed(kind) => write!(f, "version query could not start ({kind:?})"),
            Self::TimedOut => write!(f, "version query timed out"),
            Self::NonZeroExit(Some(code)) => {
                write!(f, "version query exited with status code {code}")
            }
            Self::NonZeroExit(None) => write!(f, "version query exited unsuccessfully"),
        }
    }
}

trait ExecutableResolver {
    fn resolve(&self, tool: MediaToolKind) -> Option<PathBuf>;
}

trait VersionValidator {
    fn validate(
        &self,
        tool: MediaToolKind,
        executable: &Path,
    ) -> Result<(), MediaToolValidationError>;
}

#[derive(Debug, Default)]
struct SystemExecutableResolver;

impl ExecutableResolver for SystemExecutableResolver {
    fn resolve(&self, tool: MediaToolKind) -> Option<PathBuf> {
        resolve_executable(tool.executable_name())
    }
}

#[derive(Debug, Clone, Copy)]
struct SystemVersionValidator {
    timeout: Duration,
}

impl Default for SystemVersionValidator {
    fn default() -> Self {
        Self {
            timeout: VERSION_QUERY_TIMEOUT,
        }
    }
}

impl VersionValidator for SystemVersionValidator {
    fn validate(
        &self,
        _tool: MediaToolKind,
        executable: &Path,
    ) -> Result<(), MediaToolValidationError> {
        let mut child = background_command(executable)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| MediaToolValidationError::LaunchFailed(error.kind()))?;

        wait_for_version_query(&mut child, self.timeout)
    }
}

fn wait_for_version_query(
    child: &mut Child,
    timeout: Duration,
) -> Result<(), MediaToolValidationError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(MediaToolValidationError::NonZeroExit(status.code()));
            }
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(VERSION_QUERY_POLL_INTERVAL);
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(MediaToolValidationError::TimedOut);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(MediaToolValidationError::LaunchFailed(error.kind()));
            }
        }
    }
}

pub fn discover_media_tools() -> Result<MediaTools, MediaToolsError> {
    discover_with(
        &SystemExecutableResolver,
        &SystemVersionValidator::default(),
    )
}

fn discover_with<R, V>(resolver: &R, validator: &V) -> Result<MediaTools, MediaToolsError>
where
    R: ExecutableResolver,
    V: VersionValidator,
{
    let ffmpeg = resolver.resolve(MediaToolKind::Ffmpeg);
    let ffprobe = resolver.resolve(MediaToolKind::Ffprobe);

    let (ffmpeg, ffprobe) = match (ffmpeg, ffprobe) {
        (Some(ffmpeg), Some(ffprobe)) => (ffmpeg, ffprobe),
        (None, Some(_)) => return Err(MediaToolsError::MissingFfmpeg),
        (Some(_), None) => return Err(MediaToolsError::MissingFfprobe),
        (None, None) => return Err(MediaToolsError::MissingBoth),
    };

    validate_tool(validator, MediaToolKind::Ffmpeg, &ffmpeg)?;
    validate_tool(validator, MediaToolKind::Ffprobe, &ffprobe)?;

    Ok(MediaTools { ffmpeg, ffprobe })
}

fn validate_tool<V>(validator: &V, tool: MediaToolKind, path: &Path) -> Result<(), MediaToolsError>
where
    V: VersionValidator,
{
    validator
        .validate(tool, path)
        .map_err(|reason| MediaToolsError::ValidationFailed {
            tool,
            path: path.to_path_buf(),
            reason,
        })
}

#[cfg(not(windows))]
fn resolve_executable(executable_name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;

    for directory in env::split_paths(&path) {
        let candidate = directory.join(executable_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(windows)]
fn resolve_executable(executable_name: &str) -> Option<PathBuf> {
    search_windows_executable(executable_name, None)
}

#[cfg(windows)]
fn search_windows_executable(
    executable_name: &str,
    search_path: Option<&OsStr>,
) -> Option<PathBuf> {
    let search_path = search_path.map(wide_nul);
    let executable_name = wide_nul(OsStr::new(executable_name));
    let executable_extension = wide_nul(OsStr::new(".exe"));
    let mut buffer = vec![0_u16; 260];

    loop {
        // SAFETY: every input is NUL-terminated and the output buffer is writable for its length.
        let length = unsafe {
            SearchPathW(
                search_path
                    .as_ref()
                    .map_or(ptr::null(), |path| path.as_ptr()),
                executable_name.as_ptr(),
                executable_extension.as_ptr(),
                buffer.len().try_into().ok()?,
                buffer.as_mut_ptr(),
                ptr::null_mut(),
            )
        } as usize;

        if length == 0 {
            return None;
        }

        if length < buffer.len() {
            return Some(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }

        buffer.resize(length.saturating_add(1), 0);
    }
}

#[cfg(windows)]
fn wide_nul(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::HashMap,
        env, io,
        path::{Path, PathBuf},
        process::Stdio,
        time::Duration,
    };

    use super::{
        ExecutableResolver, MediaToolKind, MediaToolValidationError, MediaTools, MediaToolsError,
        SystemVersionValidator, VersionValidator, background_command, discover_with,
        wait_for_version_query,
    };

    #[derive(Default)]
    struct FakeResolver {
        paths: HashMap<MediaToolKind, PathBuf>,
    }

    impl FakeResolver {
        fn with(mut self, tool: MediaToolKind, path: impl Into<PathBuf>) -> Self {
            self.paths.insert(tool, path.into());
            self
        }
    }

    impl ExecutableResolver for FakeResolver {
        fn resolve(&self, tool: MediaToolKind) -> Option<PathBuf> {
            self.paths.get(&tool).cloned()
        }
    }

    #[derive(Default)]
    struct FakeValidator {
        calls: RefCell<Vec<(MediaToolKind, PathBuf)>>,
        failure: Option<(MediaToolKind, MediaToolValidationError)>,
    }

    impl FakeValidator {
        fn failing(tool: MediaToolKind, reason: MediaToolValidationError) -> Self {
            Self {
                calls: RefCell::default(),
                failure: Some((tool, reason)),
            }
        }
    }

    impl VersionValidator for FakeValidator {
        fn validate(
            &self,
            tool: MediaToolKind,
            executable: &Path,
        ) -> Result<(), MediaToolValidationError> {
            self.calls
                .borrow_mut()
                .push((tool, executable.to_path_buf()));

            match &self.failure {
                Some((failed_tool, reason)) if *failed_tool == tool => Err(reason.clone()),
                _ => Ok(()),
            }
        }
    }

    fn complete_resolver() -> FakeResolver {
        FakeResolver::default()
            .with(MediaToolKind::Ffmpeg, "/tools/ffmpeg")
            .with(MediaToolKind::Ffprobe, "/tools/ffprobe")
    }

    #[test]
    fn valid_ffmpeg_and_ffprobe_are_validated() {
        let validator = FakeValidator::default();

        discover_with(&complete_resolver(), &validator).unwrap();

        assert_eq!(
            validator.calls.into_inner(),
            vec![
                (MediaToolKind::Ffmpeg, PathBuf::from("/tools/ffmpeg")),
                (MediaToolKind::Ffprobe, PathBuf::from("/tools/ffprobe")),
            ]
        );
    }

    #[test]
    fn missing_ffmpeg_is_distinct() {
        let resolver = FakeResolver::default().with(MediaToolKind::Ffprobe, "/tools/ffprobe");

        let error = discover_with(&resolver, &FakeValidator::default()).unwrap_err();

        assert_eq!(error, MediaToolsError::MissingFfmpeg);
    }

    #[test]
    fn missing_ffprobe_is_distinct() {
        let resolver = FakeResolver::default().with(MediaToolKind::Ffmpeg, "/tools/ffmpeg");

        let error = discover_with(&resolver, &FakeValidator::default()).unwrap_err();

        assert_eq!(error, MediaToolsError::MissingFfprobe);
    }

    #[test]
    fn missing_both_tools_is_reported_together() {
        let error = discover_with(&FakeResolver::default(), &FakeValidator::default()).unwrap_err();

        assert_eq!(error, MediaToolsError::MissingBoth);
    }

    #[test]
    fn executable_that_starts_and_returns_failure_is_rejected() {
        let validator = SystemVersionValidator {
            timeout: Duration::from_secs(1),
        };
        let current_executable = env::current_exe().unwrap();

        let error = validator
            .validate(MediaToolKind::Ffmpeg, &current_executable)
            .unwrap_err();

        assert!(matches!(error, MediaToolValidationError::NonZeroExit(_)));
    }

    #[test]
    fn executable_that_cannot_start_reports_launch_failure() {
        let validator = SystemVersionValidator {
            timeout: Duration::from_secs(1),
        };
        let missing = env::temp_dir().join(format!(
            "apollo-missing-media-tool-{}.exe",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);

        let error = validator
            .validate(MediaToolKind::Ffmpeg, &missing)
            .unwrap_err();

        assert_eq!(
            error,
            MediaToolValidationError::LaunchFailed(io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn version_query_that_finishes_within_bound_succeeds() {
        let mut child = background_command(env::current_exe().unwrap())
            .arg("--help")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let result = wait_for_version_query(&mut child, Duration::from_secs(1));

        assert_eq!(result, Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn version_query_that_exceeds_bound_is_terminated() {
        let mut child = background_command("sh")
            .args(["-c", "sleep 1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let result = wait_for_version_query(&mut child, Duration::from_millis(20));

        assert_eq!(result, Err(MediaToolValidationError::TimedOut));
    }

    #[test]
    fn executable_version_failure_is_typed() {
        let validator = FakeValidator::failing(
            MediaToolKind::Ffmpeg,
            MediaToolValidationError::NonZeroExit(Some(1)),
        );

        let error = discover_with(&complete_resolver(), &validator).unwrap_err();

        assert_eq!(
            error,
            MediaToolsError::ValidationFailed {
                tool: MediaToolKind::Ffmpeg,
                path: PathBuf::from("/tools/ffmpeg"),
                reason: MediaToolValidationError::NonZeroExit(Some(1)),
            }
        );
    }

    #[test]
    fn discovered_paths_are_retained_in_media_tools() {
        let tools = discover_with(&complete_resolver(), &FakeValidator::default()).unwrap();

        assert_eq!(
            tools,
            MediaTools {
                ffmpeg: PathBuf::from("/tools/ffmpeg"),
                ffprobe: PathBuf::from("/tools/ffprobe"),
            }
        );
    }

    #[test]
    fn resolved_paths_with_spaces_are_preserved() {
        let resolver = FakeResolver::default()
            .with(MediaToolKind::Ffmpeg, "/media tools/ffmpeg")
            .with(MediaToolKind::Ffprobe, "/media tools/ffprobe");

        let tools = discover_with(&resolver, &FakeValidator::default()).unwrap();

        assert_eq!(tools.ffmpeg, PathBuf::from("/media tools/ffmpeg"));
        assert_eq!(tools.ffprobe, PathBuf::from("/media tools/ffprobe"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolver_uses_native_exe_search_semantics() {
        use std::fs;

        use super::search_windows_executable;

        let directory = env::temp_dir().join(format!("apollo media tools {}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("ffmpeg.exe");
        fs::write(&executable, b"test executable placeholder").unwrap();

        let resolved_without_extension =
            search_windows_executable("ffmpeg", Some(directory.as_os_str()));
        let resolved_with_extension =
            search_windows_executable("ffmpeg.exe", Some(directory.as_os_str()));

        assert_eq!(
            resolved_without_extension.as_deref(),
            Some(executable.as_path())
        );
        assert_eq!(
            resolved_with_extension.as_deref(),
            Some(executable.as_path())
        );

        fs::remove_dir_all(directory).unwrap();
    }
}
