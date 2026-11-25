use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AppSettings {
    last_source_directory: Option<PathBuf>,
    last_output_directory: Option<PathBuf>,
}

impl AppSettings {
    pub(crate) fn from_serialized(serialized: Option<&str>) -> Self {
        serialized
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default()
    }

    pub(crate) fn to_serialized(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

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

fn existing_directory(path: Option<&Path>) -> Option<PathBuf> {
    path.filter(|path| path.is_dir()).map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::AppSettings;

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

    fn temp_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "apollo-settings-{name}-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn source_and_output_directories_persist_independently() {
        let source = temp_directory("source");
        let output = temp_directory("output");
        let mut settings = AppSettings::default();
        settings.remember_source(&source.join("clip.mp4"));
        settings.remember_output(&output.join("clip.mkv"));

        let serialized = settings.to_serialized().unwrap();
        let restored = AppSettings::from_serialized(Some(&serialized));
        let legacy = serde_json::json!({
            "last_source_directory": source,
            "last_output_directory": output,
        });
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serialized).unwrap(),
            legacy
        );
        assert_eq!(
            AppSettings::from_serialized(Some(&legacy.to_string())),
            settings
        );

        assert_eq!(restored.source_directory(), Some(source.clone()));
        assert_eq!(restored.output_directory(&source), Some(output.clone()));
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(output).unwrap();
    }

    #[test]
    fn missing_saved_directories_fall_back_without_mutating_the_other_history() {
        let source = temp_directory("fallback-source");
        let missing_source = source.join("missing-source");
        let missing_output = source.join("missing-output");
        let settings = AppSettings {
            last_source_directory: Some(missing_source),
            last_output_directory: Some(missing_output),
        };

        assert_eq!(settings.source_directory(), None);
        assert_eq!(settings.output_directory(&source), Some(source.clone()));
        fs::remove_dir_all(source).unwrap();
    }

    #[test]
    fn invalid_persisted_data_uses_clean_defaults() {
        for value in [
            None,
            Some("not json"),
            Some("{}"),
            Some(r#"{"last_source_directory":42}"#),
        ] {
            assert_eq!(AppSettings::from_serialized(value), AppSettings::default());
        }
    }
}
