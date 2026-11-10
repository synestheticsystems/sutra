//! Where a project on the map came from, which places its card on the list: the group it stands in,
//! keyed by the upstream its work comes from, and what it was made from, which it hangs from within
//! that group. The engine works it out for every project it tracks, from the files git keeps for a
//! checkout or the manifests of a folder with no git, and from the other projects on the map.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Where a project came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The group its card stands in.
    pub group: GroupKey,
    /// The projects the engine tracks that it was made from, nearest first: the checkout a linked
    /// worktree or a clone was made from, and what that one was made from in turn, which for a
    /// scratch folder holding one checkout are that checkout's; or for any other scratch folder
    /// whose git, if any, ends in the scratch, the checkout its session started in when that is in
    /// its group. Its card hangs from the nearest of them listed beside it.
    pub made_from: Vec<PathBuf>,
}

/// What a group on the list is keyed by: the first of these a project has.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GroupKey {
    /// The remote a checkout's chain of origins ends at (see `scan::Lineage`), as
    /// `github.com/owner/repo` reads.
    Upstream(String),
    /// A repository with no remote anywhere in its chain, by the folder at the chain's root: a
    /// checkout, or a bare repository, outside a session's scratch, as its copies in the scratch
    /// stand under it. A root in the scratch keys none, as a repository there is the session's own
    /// work and stands with the session's checkout.
    LocalRepository(PathBuf),
    /// A folder with no git, by the package it builds, where no one group's checkouts build a
    /// package of that name.
    Package(String),
    /// None of those: a folder with no git and no package, which is no scratch folder, or whose
    /// session started where no one project holds. A scratch folder whose chain ends in the
    /// scratch counts as one with no git.
    Other,
}

/// What the projects the engine tracks came from, cards or not: every group they stand in, and every
/// project something was made from. A line on the list stands for one of them, so what is kept of a
/// line lasts while the engine knows what it stands for, even while a folder waits to be sized for
/// its card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ancestry {
    /// Every group they stand in, with the folder of each project standing in it.
    pub groups: BTreeMap<GroupKey, BTreeSet<PathBuf>>,
    pub made_from: BTreeSet<PathBuf>,
}

impl Ancestry {
    /// The groups `before` knew that are gone, each by the one group that every project it held
    /// stands in now, which `before` did not know: a group whose key changed under all its projects
    /// at once, as an upstream's does when its checkouts' remote is given a new address, so that
    /// what the user chose for its line can follow it. A group with a project gone from the map or
    /// standing apart from the rest moved nowhere, as did one that joined a group known before, or
    /// two that went into the same new one, as which of them it is cannot be told. Other, which
    /// keys a project by nothing it has, moves neither from nor to anywhere.
    pub fn moves_since(&self, before: &Ancestry) -> BTreeMap<GroupKey, GroupKey> {
        let group_now: HashMap<&Path, &GroupKey> = (self.groups.iter())
            .flat_map(|(group, folders)| {
                folders.iter().map(move |folder| (folder.as_path(), group))
            })
            .collect();
        let gone = (before.groups.iter())
            .filter(|(group, _)| **group != GroupKey::Other && !self.groups.contains_key(*group));
        let moved: Vec<(&GroupKey, &GroupKey)> = gone
            .filter_map(|(group, folders)| {
                let mut now_in = folders.iter().map(|folder| group_now.get(folder.as_path()));
                let to = *now_in.next()??;
                let new = *to != GroupKey::Other && !before.groups.contains_key(to);
                (new && now_in.all(|now| now == Some(&to))).then_some((group, to))
            })
            .collect();
        let moved_into = |to| moved.iter().filter(|(_, into)| *into == to).count();
        (moved.iter())
            .filter(|(_, to)| moved_into(*to) == 1)
            .map(|(from, to)| ((*from).clone(), (*to).clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upstream(remote: &str) -> GroupKey {
        GroupKey::Upstream(format!("github.com/me/{remote}"))
    }

    /// The ancestry of a map whose projects stand in `groups`, each with its projects' folders,
    /// and from which nothing was made.
    fn standing<const N: usize>(groups: [(GroupKey, &[&str]); N]) -> Ancestry {
        Ancestry {
            groups: (groups.into_iter())
                .map(|(group, folders)| (group, folders.iter().map(PathBuf::from).collect()))
                .collect(),
            made_from: BTreeSet::new(),
        }
    }

    #[test]
    fn a_group_whose_every_project_stands_in_one_new_group_moved_there() {
        let before = standing([
            (upstream("ankurah-old"), &["/code/ankurah", "/code/wt/fix"]),
            (upstream("site"), &["/code/site"]),
        ]);
        let now = standing([
            (
                upstream("ankurah"),
                &["/code/ankurah", "/code/wt/fix", "/code/wt/new"],
            ),
            (upstream("site"), &["/code/site"]),
        ]);
        assert_eq!(
            now.moves_since(&before),
            BTreeMap::from([(upstream("ankurah-old"), upstream("ankurah"))])
        );
        assert_eq!(before.moves_since(&before), BTreeMap::new(), "nothing went");
    }

    #[test]
    fn a_group_that_parted_lost_a_project_or_joined_one_known_before_moved_nowhere() {
        let before = standing([
            (upstream("app"), &["/code/app", "/code/wt/app-fix"]),
            (upstream("site"), &["/code/site"]),
        ]);
        let parted = standing([
            (upstream("app-renamed"), &["/code/app"]),
            (upstream("app-fork"), &["/code/wt/app-fix"]),
            (upstream("site"), &["/code/site"]),
        ]);
        assert_eq!(parted.moves_since(&before), BTreeMap::new(), "parted");
        let one_gone = standing([
            (upstream("app-renamed"), &["/code/app"]),
            (upstream("site"), &["/code/site"]),
        ]);
        assert_eq!(
            one_gone.moves_since(&before),
            BTreeMap::new(),
            "a project left"
        );
        let joined = standing([(
            upstream("site"),
            &["/code/app", "/code/wt/app-fix", "/code/site"],
        )]);
        assert_eq!(
            joined.moves_since(&before),
            BTreeMap::new(),
            "site was known"
        );
    }

    #[test]
    fn two_groups_gone_into_one_new_group_and_other_on_either_side_moved_nowhere() {
        let before = standing([
            (upstream("app"), &["/code/app"]),
            (upstream("site"), &["/code/site"]),
            (GroupKey::Other, &["/code/notes"]),
        ]);
        let merged = standing([
            (upstream("monorepo"), &["/code/app", "/code/site"]),
            (upstream("notes"), &["/code/notes"]),
        ]);
        assert_eq!(merged.moves_since(&before), BTreeMap::new());
        let into_other = standing([
            (GroupKey::Other, &["/code/app"]),
            (upstream("site"), &["/code/site"]),
            (upstream("notes"), &["/code/notes"]),
        ]);
        let before_without_other = standing([
            (upstream("app"), &["/code/app"]),
            (upstream("site"), &["/code/site"]),
        ]);
        assert_eq!(
            into_other.moves_since(&before_without_other),
            BTreeMap::new()
        );
    }
}
