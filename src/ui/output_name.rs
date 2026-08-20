use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

/// Shared ready-state output-name policy.
///
/// The directory stays as structured path state. The editable filename stem is stored separately,
/// and feature owners supply the authoritative extension when constructing a request path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OutputNameState {
    directory: PathBuf,
    stem: OsString,
    customized: bool,
}

impl OutputNameState {
    pub(super) fn new(source: &Path, generated_stem: OsString) -> Self {
        Self {
            directory: parent_directory(source),
            stem: generated_stem,
            customized: false,
        }
    }

    pub(super) fn stem(&self) -> &OsStr {
        &self.stem
    }

    pub(super) fn is_customized(&self) -> bool {
        self.customized
    }

    pub(super) fn is_usable(&self) -> bool {
        filename_stem_is_usable(&self.stem)
    }

    pub(super) fn path(&self, extension: &str) -> PathBuf {
        if !self.is_usable() {
            return PathBuf::new();
        }

        let mut file_name = OsString::from(&self.stem);
        file_name.push(".");
        file_name.push(extension);

        if self.directory.as_os_str().is_empty() {
            PathBuf::from(file_name)
        } else {
            self.directory.join(file_name)
        }
    }

    pub(super) fn set_generated_stem(&mut self, generated_stem: OsString) {
        if !self.customized {
            self.stem = generated_stem;
        }
    }

    pub(super) fn set_stem(&mut self, stem: OsString) {
        if self.stem != stem {
            self.stem = stem;
            self.customized = true;
        }
    }

    pub(super) fn set_custom_path(&mut self, path: PathBuf) {
        self.directory = parent_directory(&path);
        self.stem = path
            .file_stem()
            .map(OsStr::to_os_string)
            .unwrap_or_default();
        self.customized = true;
    }
}

fn parent_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn filename_stem_is_usable(stem: &OsStr) -> bool {
    if stem.is_empty() || stem == OsStr::new(".") || stem == OsStr::new("..") {
        return false;
    }

    stem.to_str().is_none_or(|text| {
        !text.trim().is_empty()
            && !windows_reserved_device_basename(text)
            && !text.chars().any(|character| {
                matches!(
                    character,
                    '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
                ) || character.is_control()
            })
    })
}

fn windows_reserved_device_basename(stem: &str) -> bool {
    let basename = stem
        .split_once('.')
        .map_or(stem, |(basename, _)| basename)
        .trim_end_matches(' ');
    let basename = basename.to_ascii_uppercase();

    matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(basename.as_bytes(), [b'C', b'O', b'M', b'1'..=b'9'])
        || matches!(basename.as_bytes(), [b'L', b'P', b'T', b'1'..=b'9'])
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::{OsStr, OsString},
        path::{Path, PathBuf},
    };

    use super::OutputNameState;

    #[test]
    fn custom_path_keeps_directory_and_discards_non_authoritative_extension() {
        let mut state = OutputNameState::new(
            Path::new("captures/source.mkv"),
            OsString::from("source-mixed"),
        );

        state.set_custom_path(PathBuf::from("exports/final-name.mkv"));

        assert_eq!(state.stem(), OsStr::new("final-name"));
        assert_eq!(state.path("mp4"), PathBuf::from("exports/final-name.mp4"));
        assert!(state.is_customized());
    }

    #[test]
    fn unusable_stems_do_not_construct_request_paths() {
        let mut state = OutputNameState::new(
            Path::new("captures/source.mkv"),
            OsString::from("source-mixed"),
        );

        for stem in ["", "   ", "../escape", "bad\\name"] {
            state.set_stem(OsString::from(stem));
            assert!(!state.is_usable(), "{stem:?} should be unusable");
            assert!(state.path("mp4").as_os_str().is_empty());
        }
    }

    #[test]
    fn windows_reserved_device_basenames_are_unusable_case_insensitively() {
        let mut state = OutputNameState::new(
            Path::new("captures/source.mkv"),
            OsString::from("source-mixed"),
        );

        for stem in ["CON", "nul", "CoM1", "lPt1", "CON.notes"] {
            state.set_stem(OsString::from(stem));
            assert!(!state.is_usable(), "{stem:?} should be unusable");
            assert!(state.path("mp4").as_os_str().is_empty());
            assert!(state.path("mkv").as_os_str().is_empty());
        }

        state.set_stem(OsString::from("scene.v2"));
        assert!(state.is_usable());
        assert_eq!(state.path("mp4"), PathBuf::from("captures/scene.v2.mp4"));
    }
}
