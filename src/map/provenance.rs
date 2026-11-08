//! Where a project on the map came from, which places its card on the list: the group it stands in,
//! keyed by the upstream its work comes from, and what it was made from, which it hangs from within
//! that group. The engine works it out for every project it tracks, from the files git keeps for a
//! checkout or the manifests of a folder with no git, and from the other projects on the map.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Where a project came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The group its card stands in.
    pub group: GroupKey,
    /// The projects the engine tracks that it was made from, nearest first: the checkout a linked
    /// worktree or a clone was made from, and what that one was made from in turn, which for a
    /// scratch folder holding one checkout are that checkout's; or for any other scratch folder with
    /// no git, the checkout its session started in when that is in its group. Its card hangs from
    /// the nearest of them listed beside it.
    pub made_from: Vec<PathBuf>,
}

/// What a group on the list is keyed by: the first of these a project has.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GroupKey {
    /// The remote a checkout's chain of origins ends at (see `scan::Lineage`), as
    /// `github.com/owner/repo` reads.
    Upstream(String),
    /// A repository with no remote anywhere in its chain, by the folder at the chain's root: a
    /// checkout, or a bare repository.
    LocalRepository(PathBuf),
    /// A folder with no git, by the package it builds, where no one group's checkouts build a
    /// package of that name.
    Package(String),
    /// None of those: a folder with no git and no package, which is no scratch folder, or whose
    /// session started where no one project holds.
    Other,
}

/// What the projects the engine tracks came from, cards or not: every group they stand in, and every
/// project something was made from. A line on the list stands for one of them, so what is kept of a
/// line lasts while the engine knows what it stands for, even while a folder waits to be sized for
/// its card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ancestry {
    pub groups: BTreeSet<GroupKey>,
    pub made_from: BTreeSet<PathBuf>,
}
