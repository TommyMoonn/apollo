use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "apollo.settings.v1";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AppSettings {
    last_source_directory: Option<PathBuf>,
    last_output_directory: Option<PathBuf>,
}

impl AppSettings {
    pub(crate) fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| storage.get_string(STORAGE_KEY))
            .and_then(|serialized| serde_json::from_str(&serialized).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, storage: &mut dyn eframe::Storage) {
        if let Ok(serialized) = serde_json::to_string(self) {
            storage.set_string(STORAGE_KEY, serialized);
        }
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
        collections::HashMap,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use eframe::Storage;

    use super::{AppSettings, STORAGE_KEY};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

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

        let mut storage = MemoryStorage::default();
        settings.save(&mut storage);
        let restored = AppSettings::load(Some(&storage));

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
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "not json".to_owned());

        assert_eq!(AppSettings::load(Some(&storage)), AppSettings::default());
    }
}
