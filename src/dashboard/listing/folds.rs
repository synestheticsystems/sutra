//! The repositories whose groups the user folded shut from their headers. Folding is the user's
//! choice, so it outlasts the run, kept with the saved settings; and so that what is kept never
//! outgrows the map, a repository's fold goes once a live map no longer lists the repository.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::dashboard::format;
use crate::map::Snapshot;

/// The repositories whose groups are folded shut, each by its git dir (`RepoInfo::common_dir`) in
/// the text the window knows its group by (`format::path_text`), which names no other folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(in crate::dashboard) struct Folds(BTreeSet<String>);

impl Folds {
    /// Whether the group of the repository whose git dir reads `common_dir` is folded shut.
    pub(in crate::dashboard) fn is_folded(&self, common_dir: &str) -> bool {
        self.0.contains(common_dir)
    }

    /// The user clicked the header of the repository whose git dir reads `common_dir`: fold its
    /// group shut, or open it again. A repository `map` no longer lists changes nothing, as the
    /// header belonged to an older map, and the next map would forget its fold anyway. Returns
    /// whether the folds changed.
    pub(in crate::dashboard) fn toggle(&mut self, common_dir: &str, map: &Snapshot) -> bool {
        if !lists(map, common_dir) {
            return false;
        }
        if !self.0.remove(common_dir) {
            self.0.insert(common_dir.to_string());
        }
        true
    }

    /// A live map arrived: forget the fold of every repository `map` no longer lists. Returns
    /// whether any went.
    pub(in crate::dashboard) fn forget_gone(&mut self, map: &Snapshot) -> bool {
        let before = self.0.len();
        self.0.retain(|common_dir| lists(map, common_dir));
        self.0.len() < before
    }
}

/// Whether a worktree on `map` belongs to the repository whose git dir reads `common_dir`.
fn lists(map: &Snapshot, common_dir: &str) -> bool {
    map.projects
        .iter()
        .filter_map(|project| project.repo.as_ref())
        .any(|repo| format::is_path_text(&repo.common_dir, common_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::listing::fixtures::{linked_worktree, main_worktree, project, snapshot};

    const APP: &str = "/code/app/.git";

    /// A map listing the repository `APP` by its linked worktree alone, and a folder in none.
    fn map() -> Snapshot {
        snapshot(
            vec![
                linked_worktree(APP, "fix", project("app-fix", 1)),
                project("notes", 1),
            ],
            vec![],
        )
    }

    #[test]
    fn a_click_on_a_header_folds_its_group_and_another_opens_it() {
        let mut folds = Folds::default();
        assert!(folds.toggle(APP, &map()));
        assert!(folds.is_folded(APP));
        assert!(folds.toggle(APP, &map()));
        assert!(!folds.is_folded(APP));
    }

    #[test]
    fn a_click_on_the_header_of_a_repository_the_map_no_longer_lists_changes_nothing() {
        let mut folds = Folds::default();
        assert!(!folds.toggle("/code/gone/.git", &map()));
        assert!(
            !folds.toggle("/code/notes", &map()),
            "a folder in no repository"
        );
        assert_eq!(folds, Folds::default());
    }

    #[test]
    fn a_fold_lasts_while_the_map_lists_its_repository_and_goes_with_it() {
        let mut folds = Folds::default();
        folds.toggle(APP, &map());
        let site = "/code/site/.git";
        let both = snapshot(
            vec![
                linked_worktree(APP, "fix", project("app-fix", 1)),
                main_worktree(site, "main", project("site", 1)),
            ],
            vec![],
        );
        folds.toggle(site, &both);

        assert!(!folds.forget_gone(&both), "both still listed");
        let without_app = snapshot(
            vec![main_worktree(site, "main", project("site", 1))],
            vec![],
        );
        assert!(folds.forget_gone(&without_app));
        assert!(!folds.is_folded(APP));
        assert!(folds.is_folded(site), "site's fold stays");
    }
}
