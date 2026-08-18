use std::{
    env, fmt, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const VERSION_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const VERSION_QUERY_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaTools {
    pub(crate) ffmpeg: PathBuf,
    pub(crate) ffprobe: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MediaToolKind {
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
pub(crate) enum MediaToolsError {
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
    pub(crate) fn user_message(&self) -> &'static str {
        match self {
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
pub(crate) enum MediaToolValidationError {
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
        resolve_from_path(tool.executable_name())
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
        let mut child = Command::new(executable)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| MediaToolValidationError::LaunchFailed(error.kind()))?;

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    return Err(MediaToolValidationError::NonZeroExit(status.code()));
                }
                Ok(None) if started.elapsed() < self.timeout => {
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
}

pub(crate) fn discover_media_tools() -> Result<MediaTools, MediaToolsError> {
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

fn resolve_from_path(executable_name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let extensions = executable_extensions();

    for directory in env::split_paths(&path) {
        for candidate_name in candidate_names(executable_name, &extensions) {
            let candidate = directory.join(candidate_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn candidate_names(executable_name: &str, extensions: &[String]) -> Vec<String> {
    if Path::new(executable_name).extension().is_some() || extensions.is_empty() {
        return vec![executable_name.to_owned()];
    }

    extensions
        .iter()
        .map(|extension| format!("{executable_name}{extension}"))
        .collect()
}

fn executable_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        let raw = env::var_os("PATHEXT")
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_owned());

        return raw
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| extension.to_ascii_lowercase())
            .collect();
    }

    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::HashMap,
        env,
        path::{Path, PathBuf},
        time::Duration,
    };

    use super::{
        ExecutableResolver, MediaToolKind, MediaToolValidationError, MediaTools, MediaToolsError,
        SystemVersionValidator, VersionValidator, discover_with,
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
}
