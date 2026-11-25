use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AppSettings {
    last_source_directory: Option<PathBuf>,
    last_output_directory: Option<PathBuf>,
}

impl AppSettings {
    pub(crate) fn source_directory(&self) -> Option<PathBuf> {
        existing_directory(self.last_source_directory.as_deref())
    }

    pub(crate) fn output_directory(&self, source_fallback: &Path) -> Option<PathBuf> {
        existing_directory(self.last_output_directory.as_deref())
            .or_else(|| existing_directory(Some(source_fallback)))
    }

    pub(crate) fn remember_source(&mut self, selected_path: &Path) {
        self.last_source_directory = selected_path.parent().map(Path::to_path_buf);
    }

    pub(crate) fn remember_output(&mut self, selected_path: &Path) {
        self.last_output_directory = selected_path.parent().map(Path::to_path_buf);
    }
}

/// Windows preferences live beside (but never overwrite) the retired eframe data.
#[cfg(windows)]
pub(crate) fn settings_path() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("Apollo").join("data").join("settings.json"))
        .ok_or_else(|| "Could not locate AppData. Restore the APPDATA environment variable to save preferences.".into())
}

#[cfg(any(windows, test))]
impl AppSettings {
    pub(crate) fn load(path: &Path) -> (Self, Option<String>) {
        use std::{fs, io::ErrorKind};
        let result = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string()),
            Err(error) if error.kind() == ErrorKind::NotFound => return (Self::default(), None),
            Err(error) => Err(error.to_string()),
        };
        match result {
            Ok(settings) => (settings, None),
            Err(error) => (
                Self::default(),
                Some(format!(
                    "Could not read preferences at {}: {error}. Defaults are in use; check the file and its permissions.",
                    path.display()
                )),
            ),
        }
    }

    /// Publish only complete JSON. A failed write leaves the existing file intact.
    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        use std::{fs, io::Write};
        let write = || -> Result<(), Box<dyn std::error::Error>> {
            let directory = path
                .parent()
                .ok_or("Settings path has no parent directory")?;
            fs::create_dir_all(directory)?;
            let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
            serde_json::to_writer(&mut temporary, self)?;
            temporary.write_all(b"\n")?;
            temporary.as_file().sync_all()?;
            temporary.persist(path)?;
            Ok(())
        };
        write().map_err(|error| format!("Could not save preferences at {}: {error}. Check the directory and permissions, then retry.", path.display()))
    }
}

fn existing_directory(path: Option<&Path>) -> Option<PathBuf> {
    path.filter(|path| path.is_dir()).map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::AppSettings;
    use std::fs;

    #[test]
    fn first_launch_and_independent_directories_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences/settings.json");
        assert_eq!(AppSettings::load(&path), (AppSettings::default(), None));
        assert!(!path.exists());
        let source = root.path().join("nguồn video");
        let output = root.path().join("résultat export");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&output).unwrap();
        let mut settings = AppSettings::default();
        settings.remember_source(&source.join("clip.mp4"));
        settings.save(&path).unwrap();
        settings.remember_output(&output.join("clip.mkv"));
        settings.save(&path).unwrap();
        let (restored, error) = AppSettings::load(&path);
        assert_eq!(error, None);
        assert_eq!(restored, settings);
        assert_eq!(restored.source_directory(), Some(source.clone()));
        assert_eq!(restored.output_directory(&source), Some(output.clone()));
        let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            stored,
            serde_json::json!({
                "last_source_directory": source,
                "last_output_directory": output,
            })
        );
    }

    #[test]
    fn missing_saved_directories_fall_back_without_mutating_the_other_history() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path();
        let settings = AppSettings {
            last_source_directory: Some(source.join("missing-source")),
            last_output_directory: Some(source.join("missing-output")),
        };
        assert_eq!(settings.source_directory(), None);
        assert_eq!(
            settings.output_directory(source),
            Some(source.to_path_buf())
        );
    }

    #[test]
    fn malformed_preferences_report_error_without_rewriting_original() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        for invalid in ["not json", r#"{"last_source_directory":42}"#] {
            fs::write(&path, invalid).unwrap();
            let (settings, error) = AppSettings::load(&path);
            assert_eq!(settings, AppSettings::default());
            assert!(error.is_some());
            assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
        }
        fs::write(&path, "{}").unwrap();
        assert_eq!(AppSettings::load(&path), (AppSettings::default(), None));
    }

    #[test]
    fn failed_publish_leaves_existing_destination_and_no_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::create_dir(&path).unwrap();
        let marker = path.join("keep");
        fs::write(&marker, b"untouched").unwrap();
        let (loaded, error) = AppSettings::load(&path);
        assert_eq!(loaded, AppSettings::default());
        assert!(error.is_some());
        let mut settings = loaded;
        settings.remember_source(&root.path().join("clip.mp4"));
        assert!(settings.save(&path).is_err());
        assert_eq!(fs::read(&marker).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        fs::remove_dir_all(&path).unwrap();
        settings.save(&path).unwrap();
        assert_eq!(AppSettings::load(&path), (settings, None));
    }
}
