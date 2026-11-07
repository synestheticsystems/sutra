//! The parallel folder walk that finds repository roots and the folders publishing dev-runner state
//! of their own.
//!
//! Discovery is bound by metadata I/O, not CPU, so the walk lists folders on every thread of the
//! current rayon pool at once (2 to 6 times faster than one thread on Daniel's M4 Max, 2026-11-03).
//! It uses `read_dir`, whose entries carry their type, so classifying a child costs no `stat`.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::Failure;
use super::boundary::Boundary;
use super::cargo_target::{CACHEDIR_TAG_FILE, holds_cargo_tag};
use crate::map::devrunner::PROJECT_STATE_DIR;

/// What the walk found.
pub(super) struct Walk {
    /// Folders holding a `.git` entry (a directory for a main worktree, a file for a linked one).
    pub(super) repo_roots: Vec<PathBuf>,
    /// Folders holding a state folder of their own, which makes each a project, a repository or
    /// not. The state folder in the home folder is the registry every project shares.
    pub(super) state_folders: Vec<PathBuf>,
    pub(super) dirs_visited: usize,
    /// Folders that could not be listed, skipped with everything below them.
    pub(super) unreadable_dirs: Vec<Failure>,
}

/// Walk every root, stopping at each folder below a root that holds a `.git` entry. A repository's
/// own tree holds nearly all the directories under a code folder and no other worktree worth
/// listing: the worktrees nested inside it are in its registry, and the clones nested inside it are
/// dependencies. A root that is itself a repository (a home folder kept in git for its dotfiles)
/// is listed and still walked, because the user's projects live below it. Symlinks are never
/// followed.
pub(super) fn walk(boundary: &Boundary) -> Walk {
    let walker = Walker {
        boundary,
        repo_roots: Mutex::new(Vec::new()),
        state_folders: Mutex::new(Vec::new()),
        dirs_visited: AtomicUsize::new(0),
        unreadable_dirs: Mutex::new(Vec::new()),
    };
    rayon::scope(|scope| {
        for root in boundary.roots() {
            let walker = &walker;
            let root = root.clone();
            scope.spawn(move |scope| walker.visit(scope, root, Depth::Root));
        }
    });
    Walk {
        repo_roots: walker
            .repo_roots
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        state_folders: walker
            .state_folders
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        dirs_visited: walker.dirs_visited.into_inner(),
        unreadable_dirs: walker
            .unreadable_dirs
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    }
}

struct Walker<'a> {
    boundary: &'a Boundary,
    repo_roots: Mutex<Vec<PathBuf>>,
    state_folders: Mutex<Vec<PathBuf>>,
    dirs_visited: AtomicUsize,
    unreadable_dirs: Mutex<Vec<Failure>>,
}

impl<'a> Walker<'a> {
    fn visit<'s>(&'s self, scope: &rayon::Scope<'s>, dir: PathBuf, depth: Depth)
    where
        'a: 's,
    {
        let listing = match self.list(&dir, depth) {
            Ok(listing) => listing,
            Err(err) => {
                self.unreadable_dirs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push((dir, err.kind()));
                return;
            }
        };
        self.dirs_visited.fetch_add(1, Ordering::Relaxed);
        for name in listing.subdirs {
            let child = dir.join(name);
            scope.spawn(move |scope| self.visit(scope, child, Depth::BelowRoot));
        }
        if listing.has_state_folder {
            self.state_folders
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(dir.clone());
        }
        if listing.is_repo {
            self.repo_roots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(dir);
        }
    }

    /// What `dir` holds, or why it cannot be fully listed. A folder half-listed could hide its
    /// `.git` entry, so it is skipped whole. A folder cargo tagged as its cache holds no project,
    /// whatever its name, so the walk goes no further into it.
    fn list(&self, dir: &Path, depth: Depth) -> io::Result<Listing> {
        let skipped = self.boundary.skipped_children_of(dir);
        let mut listing = Listing {
            is_repo: false,
            has_state_folder: false,
            subdirs: Vec::new(),
        };
        let mut holds_tag = false;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            holds_tag |= name == CACHEDIR_TAG_FILE;
            if name == ".git" {
                if depth == Depth::BelowRoot {
                    // A repository is a project whatever it holds.
                    return Ok(Listing {
                        is_repo: true,
                        has_state_folder: false,
                        subdirs: Vec::new(),
                    });
                }
                listing.is_repo = true;
                continue;
            }
            // `DirEntry::file_type` does not follow symlinks, so a symlinked folder is never entered.
            if !entry.file_type()?.is_dir()
                || Boundary::is_build_output(&name)
                || skipped.is_some_and(|names| names.contains(&name))
            {
                continue;
            }
            if name == PROJECT_STATE_DIR {
                listing.has_state_folder = !self.boundary.is_home(dir);
                continue;
            }
            listing.subdirs.push(name);
        }
        if holds_tag && depth == Depth::BelowRoot && holds_cargo_tag(dir) {
            return Ok(Listing {
                is_repo: false,
                has_state_folder: false,
                subdirs: Vec::new(),
            });
        }
        Ok(listing)
    }
}

/// Whether a folder is one of the roots, which the walk descends even when it is a repository.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Depth {
    Root,
    BelowRoot,
}

struct Listing {
    /// The folder holds a `.git` entry.
    is_repo: bool,
    /// The folder holds a state folder of its own.
    has_state_folder: bool,
    /// The children to walk next: none for a repository below a root.
    subdirs: Vec<OsString>,
}
