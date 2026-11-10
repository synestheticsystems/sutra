//! Where each project on the map came from (see `Provenance`). Each project's own files are read
//! with it, a checkout's git files for its lineage and the packages it builds, a folder's manifests
//! for its package (`Origin`); then the map as a whole decides what they alone cannot. In a
//! session's scratch only a chain of origins that ends outside the scratch counts, a scratch
//! folder's own or that of the one checkout directly inside it, and a scratch folder with neither
//! is taken for one with no git (see `scan::lineage_of_scratch_folder`). In order, a project's
//! group is keyed by:
//!   - the remote its checkout's chain of origins ends at;
//!   - for a repository with no remote anywhere in that chain, the folder at the chain's root,
//!     which lies outside the scratch: a repository kept there is a project of its own, and its
//!     copies in the scratch stand under it;
//!   - for a folder with no git, the package it builds: in the group of the checkouts that build a
//!     package of that name when they all stand in one, else in a group of the package's own;
//!   - for a scratch folder with none of those, the group of the checkout its session started in,
//!     which it hangs from;
//!   - and otherwise, Other.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::tracked::Tracked;
use super::{State, owner};
use crate::map::scan::{self, Lineage};
use crate::map::{Ancestry, FolderAccess, GroupKey, Provenance, RepoInfo};

/// A file that could not be read, and why.
type Unread = (PathBuf, io::ErrorKind);

/// What a project's own files say of where it came from, read with the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Origin {
    /// A git checkout, or a scratch folder taking the lineage of the one it holds: its lineage, and
    /// the packages it builds, by which a folder with no git may join its group. A linked worktree
    /// of a repository with a main worktree builds none here, as the main worktree speaks for the
    /// repository.
    Checkout {
        lineage: Lineage,
        builds: Vec<String>,
    },
    /// A folder with no git, or a scratch folder whose chain ends in the scratch: the package it
    /// builds, when it builds one and roots no workspace. A folder Sutra cannot read says nothing,
    /// and is taken for one with neither.
    Folder { package: Option<String> },
}

impl Origin {
    /// Where the worktree `repo`, checked out at `folder`, came from: its lineage, read from the git
    /// dir it shares, so even a worktree Sutra cannot read has one, and what it builds, read from
    /// the folder where it can.
    pub(super) fn of_worktree(
        folder: &Path,
        repo: &RepoInfo,
        access: FolderAccess,
        may_follow: &(dyn Fn(&Path) -> bool + Sync),
    ) -> (Origin, Vec<Unread>) {
        let (lineage, mut unread) = scan::lineage(repo, may_follow);
        let speaks_for_its_repository =
            repo.worktree_name.is_none() || repo.folder() == repo.common_dir;
        let builds = if speaks_for_its_repository && access == FolderAccess::Readable {
            let (builds, failures) = scan::packages_built(folder);
            unread.extend(failures);
            builds
        } else {
            Vec::new()
        };
        (Origin::Checkout { lineage, builds }, unread)
    }

    /// Where `folder`, a folder only a dev-runner environment names, came from: it may be a checkout
    /// git made, or a folder with no git.
    pub(super) fn of_folder(
        folder: &Path,
        access: FolderAccess,
        may_follow: &(dyn Fn(&Path) -> bool + Sync),
    ) -> (Origin, Vec<Unread>) {
        if access != FolderAccess::Readable {
            return (Origin::Folder { package: None }, Vec::new());
        }
        let (lineage, unread) = scan::lineage_of_folder(folder, may_follow);
        Origin::of_lineage_or_package(folder, lineage, unread)
    }

    /// Where `folder`, a session's scratch folder in `scratch_root`, came from: a checkout when its
    /// chain of origins, or that of the one checkout it holds, ends outside the scratch (see
    /// `scan::lineage_of_scratch_folder`), and otherwise a folder with no git.
    pub(super) fn of_scratch_folder(
        folder: &Path,
        access: FolderAccess,
        scratch_root: &Path,
        may_follow: &(dyn Fn(&Path) -> bool + Sync),
    ) -> (Origin, Vec<Unread>) {
        if access != FolderAccess::Readable {
            return (Origin::Folder { package: None }, Vec::new());
        }
        let (lineage, unread) = scan::lineage_of_scratch_folder(folder, scratch_root, may_follow);
        Origin::of_lineage_or_package(folder, lineage, unread)
    }

    /// Where `folder` came from, by `lineage`, what git's files say of it, with `unread`, what
    /// reading them could not read: a checkout, with the packages the folder builds, or else a
    /// folder with no git, by its package.
    fn of_lineage_or_package(
        folder: &Path,
        lineage: Option<Lineage>,
        mut unread: Vec<Unread>,
    ) -> (Origin, Vec<Unread>) {
        let origin = match lineage {
            Some(lineage) => {
                let (builds, failures) = scan::packages_built(folder);
                unread.extend(failures);
                Origin::Checkout { lineage, builds }
            }
            None => Origin::Folder {
                package: scan::own_package(folder).unwrap_or_else(|failure| {
                    unread.push(failure);
                    None
                }),
            },
        };
        (origin, unread)
    }

    /// The provenance its own files give the project at `path`, before the rest of the map is
    /// weighed: a checkout's by its lineage, and a folder's by its package, or Other.
    pub(super) fn own_provenance(&self, path: &Path) -> Provenance {
        let (group, made_from) = match self {
            Origin::Checkout { lineage, .. } => {
                let group = match &lineage.remote {
                    Some(remote) => GroupKey::Upstream(remote.clone()),
                    None => {
                        let root = lineage.made_from.last().map_or(path, PathBuf::as_path);
                        GroupKey::LocalRepository(root.to_path_buf())
                    }
                };
                (group, lineage.made_from.clone())
            }
            Origin::Folder {
                package: Some(name),
            } => (GroupKey::Package(name.clone()), Vec::new()),
            Origin::Folder { package: None } => (GroupKey::Other, Vec::new()),
        };
        Provenance { group, made_from }
    }
}

impl State {
    /// Work out every project's provenance again, as projects joined or left the map, and what
    /// the lines on the list may stand for (see `Ancestry`).
    pub(super) fn trace_provenance(&mut self) {
        let traced = trace(&self.projects);
        let mut ancestry = Ancestry::default();
        for (path, provenance) in traced {
            (ancestry.groups.entry(provenance.group.clone()))
                .or_default()
                .insert(path.clone());
            ancestry
                .made_from
                .extend(provenance.made_from.iter().cloned());
            if let Some(tracked) = self.projects.get_mut(&path) {
                tracked.project.provenance = provenance;
            }
        }
        self.ancestry = Arc::new(ancestry);
    }
}

/// How many projects a chain of scratch folders that hang from their sessions' starting folders
/// may run through, each started inside the one before: one is the rule, and more only stops one
/// whose starting folders run in a loop.
const LONGEST_SESSION_CHAIN: usize = 4;

/// The provenance of each of `projects`, as the module says: what each one's own files give it,
/// then the folders with no git placed by the checkouts on the map. What a project was made from is
/// cut to the projects the map tracks, as only those can stand on the list.
fn trace(projects: &BTreeMap<PathBuf, Tracked>) -> BTreeMap<PathBuf, Provenance> {
    let mut traced: BTreeMap<PathBuf, Provenance> = projects
        .iter()
        .map(|(path, tracked)| (path.clone(), tracked.origin.own_provenance(path)))
        .collect();
    let builders = builders(projects, &traced);
    let mut waiting_on_sessions = Vec::new();
    for (path, tracked) in projects {
        let Origin::Folder { package } = &tracked.origin else {
            continue;
        };
        let started_in = tracked.project.scratch.as_ref();
        let started_in = started_in.and_then(|space| space.started_in_folder.as_deref());
        let session_checkout = started_in
            .and_then(|folder| owner(projects, folder))
            .filter(|checkout| *checkout != path.as_path());
        match package {
            Some(name) => {
                let built_by = builders.get(name.as_str());
                let Some(group) = built_by.and_then(one_of).cloned() else {
                    continue;
                };
                // A scratch folder made while its session worked in a checkout of the group hangs
                // from that checkout.
                let made_from = session_checkout
                    .filter(|checkout| traced[*checkout].group == group)
                    .map(|checkout| hanging_from(checkout, &traced))
                    .unwrap_or_default();
                traced.insert(path.clone(), Provenance { group, made_from });
            }
            None => {
                if let Some(checkout) = session_checkout {
                    waiting_on_sessions.push((path.clone(), checkout.to_path_buf()));
                }
            }
        }
    }
    // A scratch folder placed by its session's checkout takes that checkout's group, and hangs from
    // it; one whose checkout is itself such a folder waits for that one first.
    for _ in 0..LONGEST_SESSION_CHAIN {
        let placed: BTreeSet<&PathBuf> = waiting_on_sessions.iter().map(|(path, _)| path).collect();
        let (ready, waiting): (Vec<_>, Vec<_>) = waiting_on_sessions
            .iter()
            .cloned()
            .partition(|(_, checkout)| !placed.contains(checkout));
        for (path, checkout) in ready {
            let group = traced[&checkout].group.clone();
            let made_from = hanging_from(&checkout, &traced);
            traced.insert(path, Provenance { group, made_from });
        }
        if waiting.is_empty() {
            break;
        }
        waiting_on_sessions = waiting;
    }
    for (path, provenance) in &mut traced {
        let mut seen = BTreeSet::new();
        provenance.made_from.retain(|made_from| {
            made_from != path && projects.contains_key(made_from) && seen.insert(made_from.clone())
        });
    }
    traced
}

/// The groups that build each package, by the packages the checkouts among `projects` build, each
/// in the group `traced` gives it.
fn builders<'a>(
    projects: &'a BTreeMap<PathBuf, Tracked>,
    traced: &BTreeMap<PathBuf, Provenance>,
) -> HashMap<&'a str, BTreeSet<GroupKey>> {
    let mut builders: HashMap<&str, BTreeSet<GroupKey>> = HashMap::new();
    for (path, tracked) in projects {
        if let Origin::Checkout { builds, .. } = &tracked.origin {
            for name in builds {
                let group = traced[path].group.clone();
                builders.entry(name).or_default().insert(group);
            }
        }
    }
    builders
}

/// The one item of `items`, when it has exactly one.
fn one_of<T>(items: &BTreeSet<T>) -> Option<&T> {
    let mut items = items.iter();
    match (items.next(), items.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// What a project made from `checkout` was made from: `checkout`, then what it was made from.
fn hanging_from(checkout: &Path, traced: &BTreeMap<PathBuf, Provenance>) -> Vec<PathBuf> {
    let above = traced.get(checkout).map(|provenance| &provenance.made_from);
    std::iter::once(checkout.to_path_buf())
        .chain(above.into_iter().flatten().cloned())
        .collect()
}

#[cfg(test)]
mod tests;
