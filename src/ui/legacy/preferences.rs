//! Preserve the existing eframe storage key and location at the renderer boundary.
use crate::settings::AppSettings;

const STORAGE_KEY: &str = "apollo.settings.v1";

pub(super) fn load(storage: Option<&dyn eframe::Storage>) -> AppSettings {
    let serialized = storage.and_then(|storage| storage.get_string(STORAGE_KEY));
    AppSettings::from_serialized(serialized.as_deref())
}

pub(super) fn save(settings: &AppSettings, storage: &mut dyn eframe::Storage) {
    // Eframe saves periodically; do not rewrite unchanged (or untouched invalid) preferences.
    if load(Some(storage)) != *settings
        && let Ok(serialized) = settings.to_serialized()
    {
        storage.set_string(STORAGE_KEY, serialized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, path::Path};

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

    #[test]
    fn storage_preserves_existing_values_until_preferences_change() {
        let mut storage = MemoryStorage::default();
        let existing =
            r#"{ "last_source_directory": "old source", "last_output_directory": "old output" }"#;
        storage
            .0
            .insert("apollo.settings.v1".to_owned(), existing.to_owned());
        let mut settings = load(Some(&storage));
        save(&settings, &mut storage);
        assert_eq!(storage.0.get("apollo.settings.v1").unwrap(), existing);
        settings.remember_output(Path::new("new output").join("clip.mkv").as_path());
        save(&settings, &mut storage);
        assert_eq!(load(Some(&storage)), settings);
        let value: serde_json::Value =
            serde_json::from_str(storage.0.get("apollo.settings.v1").unwrap()).unwrap();
        assert_eq!(value["last_source_directory"], "old source");
        assert_eq!(value["last_output_directory"], "new output");
        assert_eq!(storage.0.len(), 1);
        storage
            .0
            .insert("apollo.settings.v1".to_owned(), "invalid".to_owned());
        let defaults = load(Some(&storage));
        assert_eq!(defaults, AppSettings::default());
        save(&defaults, &mut storage);
        assert_eq!(storage.0.get("apollo.settings.v1").unwrap(), "invalid");
    }
}
