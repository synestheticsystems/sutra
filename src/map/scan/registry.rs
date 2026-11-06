//! A repository's worktree registry (`<common>/worktrees/*/`): the only record of linked worktrees
//! that live outside the scanned roots, such as agent scratchpads under `/private/tmp`.

use std::fs;
use std::io;
use std::path::Path;

use super::boundary::{Boundary, listing_access};
use super::{Failure, Worktree, git};
use crate::map::FolderAccess;

/// One repository's registrations, classified.
#[derive(Default)]
pub(super) struct Registry {
    pub(super) worktrees: Vec<Worktree>,
    /// The registry folder, or admin directories in it, that could not be read.
    pub(super) unreadable_dirs: Vec<Failure>,
    /// Folders of registrations whose own git state (`HEAD`, `commondir`) cannot be read.
    pub(super) unresolved: Vec<Failure>,
}

enum Registration {
    Listed(Worktree),
    Unresolved(Failure),
    Stale,
    /// The admin directory itself cannot be read.
    Unreadable(Failure),
}

/// Classify every registration of the repository whose shared git dir is `common_dir`.
pub(super) fn read(common_dir: &Path, boundary: &Boundary) -> Registry {
    let mut registry = Registry::default();
    let folder = common_dir.join("worktrees");
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return registry,
        Err(err) => {
            registry.unreadable_dirs.push((folder, err.kind()));
            return registry;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                registry.unreadable_dirs.push((folder, err.kind()));
                break;
            }
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        match classify(&entry.path(), boundary) {
            Registration::Listed(worktree) => registry.worktrees.push(worktree),
            Registration::Unresolved(failure) => registry.unresolved.push(failure),
            Registration::Stale => {}
            Registration::Unreadable(failure) => registry.unreadable_dirs.push(failure),
        }
    }
    registry
}

/// A registration is stale when the `.git` file it names is gone, which is also what
/// `git worktree prune` checks. A locked one is kept (git never prunes it: its folder may be on a
/// volume that is not mounted), and so is one whose folder exists but cannot be read; both are
/// listed, with the reason they cannot be read.
fn classify(admin_dir: &Path, boundary: &Boundary) -> Registration {
    let dot_git = match git::registered_dot_git(admin_dir) {
        Ok(dot_git) => dot_git,
        // Git prunes a registration that has lost its `gitdir` file.
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Registration::Stale,
        Err(err) => return Registration::Unreadable((admin_dir.to_path_buf(), err.kind())),
    };
    let Some(folder) = dot_git.parent() else {
        // A `gitdir` file naming the root: git never writes one.
        return Registration::Unreadable((admin_dir.to_path_buf(), io::ErrorKind::InvalidData));
    };
    let access = match boundary.kept_out_by(folder) {
        Some(private) => FolderAccess::KeptOut(private),
        None => match fs::symlink_metadata(&dot_git) {
            // Inside the sandbox a folder outside the grant can be stat'ed but not listed, so the
            // `.git` file being there says nothing about whether the folder reads.
            Ok(_) => listing_access(folder),
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied => FolderAccess::Denied,
            Err(_) if admin_dir.join("locked").is_file() => FolderAccess::Missing,
            Err(_) => return Registration::Stale,
        },
    };
    // An unreadable folder cannot be resolved either, so it keeps the path git recorded.
    let path = if access == FolderAccess::Readable {
        fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
    } else {
        folder.to_path_buf()
    };
    match git::repo_info_of_git_dir(admin_dir, &|path| boundary.may_read(path)) {
        Ok(repo) => Registration::Listed(Worktree { path, repo, access }),
        Err(err) => Registration::Unresolved((path, err.kind())),
    }
}
