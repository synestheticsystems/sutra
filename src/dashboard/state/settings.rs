//! What the user set that outlasts a run: the free space to keep, which of the folders macOS
//! guards to watch, which lines on the list the user opened or folded shut again, and how long
//! each group's build folders may sit idle before Sutra cleans them.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dashboard::cleaning::CleanWhenIdle;
use crate::dashboard::listing::Folds;
use crate::home;
use crate::map::{PrivateFolder, ScanOptions};

const SETTINGS_FILE: &str = "settings.json";

/// The settings as saved. A setting missing from the file, as one saved before the setting
/// existed, takes its default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SavedSettings {
    /// Below this much free space Sutra cleans by itself. `None` leaves cleaning to the user.
    pub(super) keep_free_bytes: Option<u64>,
    /// The private folders to watch (see `ScanOptions`).
    pub(super) watched_private_folders: BTreeSet<PrivateFolder>,
    /// The lines the user opened or folded shut again, none at first.
    pub(super) folds: Folds,
    /// How long build folders may sit idle before Sutra cleans them, for every group and for each
    /// group that chose otherwise; never, at first.
    pub(super) clean_when_idle: CleanWhenIdle,
}

impl Default for SavedSettings {
    /// Documents is watched from the start, because Codex Desktop keeps each task's folder there,
    /// with the worktree the task works in.
    fn default() -> Self {
        SavedSettings {
            keep_free_bytes: None,
            watched_private_folders: BTreeSet::from([PrivateFolder::Documents]),
            folds: Folds::default(),
            clean_when_idle: CleanWhenIdle::default(),
        }
    }
}

impl SavedSettings {
    /// What the scanner may enter.
    pub(super) fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            private_folders: self.watched_private_folders.clone(),
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
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::dashboard::cleaning::{IdleTime, WhenIdle};
    use crate::dashboard::listing::fixtures::snapshot;
    use crate::dashboard::{GroupKind, LineKey};
    use crate::map::test_support::TempDir;
    use crate::map::{Ancestry, GroupKey, Snapshot};

    /// The line of the clones of `github.com/me/app`.
    fn app() -> LineKey {
        LineKey::Group {
            kind: GroupKind::Upstream {
                remote: "github.com/me/app".to_string(),
            },
        }
    }

    /// A map whose engine knows the clones of `github.com/me/app`.
    fn map() -> Snapshot {
        Snapshot {
            ancestry: Arc::new(Ancestry {
                groups: BTreeMap::from([(
                    GroupKey::Upstream("github.com/me/app".to_string()),
                    BTreeSet::from([PathBuf::from("/code/app")]),
                )]),
                made_from: BTreeSet::from([PathBuf::from("/code/app")]),
            }),
            ..snapshot(vec![], vec![])
        }
    }

    #[test]
    fn settings_saved_are_read_back() {
        let fixture = TempDir::new("settings-round-trip");
        let path = fixture.path("Sutra/settings.json");
        let mut settings = SavedSettings {
            keep_free_bytes: Some(100_000_000_000),
            watched_private_folders: BTreeSet::from([
                PrivateFolder::Desktop,
                PrivateFolder::Downloads,
            ]),
            folds: Folds::default(),
            clean_when_idle: CleanWhenIdle::default(),
        };
        settings.folds.toggle(&app(), &map());
        let app_group = GroupKey::Upstream("github.com/me/app".to_string());
        let four_hours = WhenIdle::CleanAfter {
            idle: IdleTime::FourHours,
        };
        settings.clean_when_idle.choose_for_every_group(four_hours);
        (settings.clean_when_idle).choose_own(&app_group, Some(WhenIdle::Never), &map());

        settings.save(Some(&path)).unwrap();

        let read_back = SavedSettings::load(Some(&path)).unwrap();
        assert_eq!(read_back, settings);
        assert_eq!(read_back.clean_when_idle.for_every_group(), four_hours);
        assert_eq!(
            read_back.clean_when_idle.own_choice_of(&app_group),
            Some(WhenIdle::Never)
        );
        assert!(!read_back.folds.is_folded(&app()), "opened");
    }

    #[test]
    fn a_file_saved_before_a_setting_existed_gives_it_its_default() {
        let fixture = TempDir::new("settings-older");
        let older = fixture.write("settings.json", r#"{"keep_free_bytes": 25000000000}"#);
        let settings = SavedSettings::load(Some(&older)).unwrap();
        assert_eq!(settings.keep_free_bytes, Some(25_000_000_000));
        assert_eq!(
            settings.scan_options().private_folders,
            BTreeSet::from([PrivateFolder::Documents]),
            "Documents alone is watched"
        );
        assert_eq!(settings.folds, Folds::default(), "every line starts folded");
    }

    #[test]
    fn the_folds_a_file_kept_by_repository_are_let_go_and_the_rest_of_it_reads() {
        let fixture = TempDir::new("settings-folded-repositories");
        let older = fixture.write(
            "settings.json",
            r#"{"keep_free_bytes": 25000000000, "folded_repositories": ["/code/app/.git"]}"#,
        );
        let settings = SavedSettings::load(Some(&older)).unwrap();
        assert_eq!(settings.keep_free_bytes, Some(25_000_000_000));
        assert_eq!(
            settings.folds,
            Folds::default(),
            "no line is known by a git dir"
        );
    }

    #[test]
    fn watching_no_private_folder_is_kept_rather_than_taken_for_the_default() {
        let fixture = TempDir::new("settings-none-watched");
        let path = fixture.path("settings.json");
        let settings = SavedSettings {
            watched_private_folders: BTreeSet::new(),
            ..SavedSettings::default()
        };

        settings.save(Some(&path)).unwrap();

        assert!(
            SavedSettings::load(Some(&path))
                .unwrap()
                .watched_private_folders
                .is_empty()
        );
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
