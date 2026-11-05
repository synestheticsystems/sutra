//! What the user set that outlasts a run: the free space to keep, and whether to look in the
//! folders macOS guards.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::home;
use crate::map::ScanOptions;

const SETTINGS_FILE: &str = "settings.json";

/// The settings as saved. A setting missing from the file, as one saved before the setting
/// existed, takes its default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SavedSettings {
    /// Below this much free space Sutra cleans by itself. `None` leaves cleaning to the user.
    pub(super) keep_free_bytes: Option<u64>,
    /// Look in Desktop, Documents and Downloads too (see `ScanOptions`).
    pub(super) include_private_folders: bool,
}

impl SavedSettings {
    /// What the scanner may enter.
    pub(super) fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            include_private_folders: self.include_private_folders,
        }
    }
}

/// Why the settings could not be read or saved.
#[derive(Debug)]
pub(super) enum SettingsError {
    Io(io::Error),
    /// The file is not what Sutra wrote.
    Invalid(serde_json::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Io(err) => err.fmt(f),
            SettingsError::Invalid(err) => write!(f, "the file isn't valid: {err}"),
        }
    }
}

impl SavedSettings {
    /// The file that keeps the settings between runs, where the system has a place for it.
    pub(super) fn file() -> Option<PathBuf> {
        home::app_data_file(SETTINGS_FILE)
    }

    /// The settings `file` keeps; the defaults when no run saved any there, or there is no `file`.
    pub(super) fn load(file: Option<&Path>) -> Result<SavedSettings, SettingsError> {
        let Some(path) = file else {
            return Ok(SavedSettings::default());
        };
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(SettingsError::Invalid),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(SavedSettings::default()),
            Err(err) => Err(SettingsError::Io(err)),
        }
    }

    /// Keep these in `file` for the next run. With no `file` there is nowhere to keep them.
    pub(super) fn save(&self, file: Option<&Path>) -> Result<(), SettingsError> {
        let Some(path) = file else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(self).map_err(SettingsError::Invalid)?;
        home::write_atomically(path, &bytes).map_err(SettingsError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn settings_saved_are_read_back() {
        let fixture = TempDir::new("settings-round-trip");
        let path = fixture.path("Sutra/settings.json");
        let settings = SavedSettings {
            keep_free_bytes: Some(100_000_000_000),
            include_private_folders: true,
        };

        settings.save(Some(&path)).unwrap();

        assert_eq!(SavedSettings::load(Some(&path)).unwrap(), settings);
    }

    #[test]
    fn a_file_saved_before_a_setting_existed_gives_it_its_default_which_keeps_out_of_private_folders()
     {
        let fixture = TempDir::new("settings-older");
        let older = fixture.write("settings.json", r#"{"keep_free_bytes": 25000000000}"#);
        let settings = SavedSettings::load(Some(&older)).unwrap();
        assert_eq!(settings.keep_free_bytes, Some(25_000_000_000));
        assert!(!settings.scan_options().include_private_folders);
    }

    #[test]
    fn no_file_means_the_defaults_and_a_broken_one_says_so() {
        let fixture = TempDir::new("settings-broken");
        let missing = fixture.path("nothing.json");
        assert_eq!(
            SavedSettings::load(Some(&missing)).unwrap(),
            SavedSettings::default()
        );
        assert_eq!(SavedSettings::load(None).unwrap(), SavedSettings::default());
        let broken = fixture.write("broken.json", "{ not json");
        assert!(matches!(
            SavedSettings::load(Some(&broken)),
            Err(SettingsError::Invalid(_))
        ));
    }
}
