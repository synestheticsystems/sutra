//! The folds the user chose from the lines of the Activity sort's groups. A group starts as its
//! parent does: a repository's open, as its worktrees are the user's projects, and a session's
//! folded, as its scratch is what the list should keep out of the way of those projects. A click
//! on a line turns its fold, and the choice outlasts the run, kept with the saved settings; so that
//! what is kept never outgrows the map, a choice goes once a live map no longer lists its parent.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::group;
use crate::dashboard::{Parent, format};
use crate::map::Snapshot;

/// The fold the user left each line in, by the text the window knows its parent by
/// (`format::path_text` of its folder), which names no other folder. A line never clicked is not
/// here, and starts as its parent does (see `Parent::starts_folded`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SavedFolds")]
pub(in crate::dashboard) struct Folds(BTreeMap<String, Fold>);

/// Whether a group's cards show under its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Fold {
    Open,
    Folded,
}

/// The folds as a settings file keeps them: as now, or as runs kept them before a session had a
/// line, which folded only repositories, and kept the git dir of each one folded shut.
#[derive(Deserialize)]
#[serde(untagged)]
enum SavedFolds {
    Chosen(BTreeMap<String, Fold>),
    FoldedRepositories(BTreeSet<String>),
}

impl From<SavedFolds> for Folds {
    fn from(saved: SavedFolds) -> Folds {
        match saved {
            SavedFolds::Chosen(chosen) => Folds(chosen),
            SavedFolds::FoldedRepositories(folded) => {
                let folded = folded.into_iter().map(|key| (key, Fold::Folded));
                Folds(folded.collect())
            }
        }
    }
}

impl Parent {
    /// The text the window knows the parent by: the folder it is known by.
    fn key(&self) -> &str {
        match self {
            Parent::Repository { common_dir } => common_dir,
            Parent::Session { folder } => folder,
        }
    }

    /// Whether the parent's group starts folded: a session's does, and a repository's does not.
    fn starts_folded(&self) -> bool {
        matches!(self, Parent::Session { .. })
    }
}

impl Folds {
    /// Whether the group of `parent` is folded shut.
    pub(in crate::dashboard) fn is_folded(&self, parent: &Parent) -> bool {
        match self.0.get(parent.key()) {
            Some(fold) => *fold == Fold::Folded,
            None => parent.starts_folded(),
        }
    }

    /// The user clicked the line of `parent`: fold its group shut, or open it again. A parent `map`
    /// no longer lists changes nothing, as the line belonged to an older map, and the next map
    /// would forget its fold anyway. Returns whether the folds changed.
    pub(in crate::dashboard) fn toggle(&mut self, parent: &Parent, map: &Snapshot) -> bool {
        if !lists(map, parent.key()) {
            return false;
        }
        let turned = if self.is_folded(parent) {
            Fold::Open
        } else {
            Fold::Folded
        };
        self.0.insert(parent.key().to_string(), turned);
        true
    }

    /// A live map arrived: forget the fold of every parent `map` no longer lists. Returns whether
    /// any went.
    pub(in crate::dashboard) fn forget_gone(&mut self, map: &Snapshot) -> bool {
        let before = self.0.len();
        self.0.retain(|key, _| lists(map, key));
        self.0.len() < before
    }
}

/// Whether a project on `map` has a parent known by the text `key`.
fn lists(map: &Snapshot, key: &str) -> bool {
    let parents = map.projects.iter().filter_map(group::Parent::of);
    parents
        .map(|parent| parent.folder())
        .any(|folder| format::is_path_text(folder, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::listing::fixtures::{
        linked_worktree, main_worktree, project, scratch_folder, snapshot,
    };

    const APP: &str = "/code/app/.git";
    const SESSION: &str = "/private/tmp/claude-501/-code-app/e26ca359";

    fn app() -> Parent {
        Parent::Repository {
            common_dir: APP.to_string(),
        }
    }

    fn session() -> Parent {
        Parent::Session {
            folder: SESSION.to_string(),
        }
    }

    /// A map listing the repository `APP` by its linked worktree alone, a folder in none, and a
    /// scratch folder of the session whose folder is `SESSION`.
    fn map() -> Snapshot {
        snapshot(
            vec![
                linked_worktree(APP, "fix", project("app-fix", 1)),
                project("notes", 1),
                scratch_folder("e26ca359", "r567", project("Foundry", 1)),
            ],
            vec![],
        )
    }

    #[test]
    fn a_repositorys_group_starts_open_and_a_sessions_folded_and_a_click_turns_each() {
        let mut folds = Folds::default();
        assert!(!folds.is_folded(&app()));
        assert!(folds.is_folded(&session()));

        assert!(folds.toggle(&app(), &map()));
        assert!(folds.toggle(&session(), &map()));
        assert!(folds.is_folded(&app()));
        assert!(!folds.is_folded(&session()));

        assert!(folds.toggle(&app(), &map()));
        assert!(folds.toggle(&session(), &map()));
        assert!(!folds.is_folded(&app()));
        assert!(folds.is_folded(&session()));
    }

    #[test]
    fn a_click_on_the_line_of_a_parent_the_map_no_longer_lists_changes_nothing() {
        let mut folds = Folds::default();
        let gone = Parent::Repository {
            common_dir: "/code/gone/.git".to_string(),
        };
        assert!(!folds.toggle(&gone, &map()));
        let notes = Parent::Session {
            folder: "/code/notes".to_string(),
        };
        assert!(!folds.toggle(&notes, &map()), "a folder in no session");
        assert_eq!(folds, Folds::default());
    }

    #[test]
    fn a_fold_lasts_while_the_map_lists_its_parent_and_goes_with_it() {
        let mut folds = Folds::default();
        folds.toggle(&app(), &map());
        folds.toggle(&session(), &map());
        let site = "/code/site/.git";
        let with_site = snapshot(
            vec![
                linked_worktree(APP, "fix", project("app-fix", 1)),
                main_worktree(site, "main", project("site", 1)),
                scratch_folder("e26ca359", "r567", project("Foundry", 1)),
            ],
            vec![],
        );
        let site = Parent::Repository {
            common_dir: site.to_string(),
        };
        folds.toggle(&site, &with_site);

        assert!(!folds.forget_gone(&with_site), "all three still listed");
        let only_site = snapshot(
            vec![main_worktree("/code/site/.git", "main", project("site", 1))],
            vec![],
        );
        assert!(folds.forget_gone(&only_site));
        assert!(!folds.is_folded(&app()));
        assert!(folds.is_folded(&session()), "folded, as it starts again");
        assert!(folds.is_folded(&site), "site's fold stays");
    }

    #[test]
    fn folds_are_saved_as_each_lines_choice_and_read_back_from_the_repositories_once_folded() {
        let mut folds = Folds::default();
        folds.toggle(&app(), &map());
        folds.toggle(&session(), &map());
        let saved = serde_json::to_value(&folds).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({ APP: "folded", SESSION: "open" }),
            "each choice in words"
        );
        assert_eq!(serde_json::from_value::<Folds>(saved).unwrap(), folds);

        let before_sessions = serde_json::json!([APP]);
        let read = serde_json::from_value::<Folds>(before_sessions).unwrap();
        assert!(read.is_folded(&app()), "a repository folded then stays so");
        assert!(read.is_folded(&session()), "and a session starts folded");
    }
}
