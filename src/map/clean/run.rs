//! Deleting a plan's folders under cargo's build locks.
//!
//! `cargo clean` takes no lock (`src/ops/cargo_clean.rs:125-139`) and deletes a running build's
//! files from under it. Sutra refuses while a build runs, and makes a build that starts during
//! the delete wait for it. In order:
//!  1. Open each folder, refusing one that is no longer the folder its checks passed, and take
//!     every lock in the folders, or refuse when a build holds one (`lock`). Each folder stays
//!     open from here on, and whatever follows reaches into it through that folder alone.
//!  2. Measure the folders, while every link of a file is still there to share its size.
//!  3. Find the locks again and take any that appeared while measuring took its seconds, still
//!     refusing when a build holds one, and refuse a folder whose path no longer leads to it.
//!     Nothing is deleted before this.
//!  4. Delete everything but the lock files, the folders holding them and the folder's
//!     `CACHEDIR.TAG` (`removal::clear`). A build starting now waits on Sutra's locks.
//!  5. Let the locks go, and a build that waited has the emptied folder.
//!
//! Those few empty files stay for good, where `cargo clean` would remove them. Deleting a lock file
//! a build waits on would wake that build holding a lock on a file with no name. Checked with cargo
//! 1.99.1 on 2026-11-04: the build then made the folder again without its tag or its Time Machine
//! and iCloud exclusions, and a `cargo check` started during it ran alongside it instead of waiting.
//! No wait can rule that out, since nothing says when a waiting build has woken.
//!
//! The files are deleted for real, not moved to the Trash, which would free nothing until the
//! Trash is emptied. Sutra never takes cargo's package cache lock
//! (`$CARGO_HOME/.package-cache`): `cargo clean` does not, and holding it would stall every cargo
//! command on the machine.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::locks::HeldLocks;
use super::open_folder::{Identity, OpenFolder, is_not_a_folder};
use super::{Plan, removal};
use crate::map::disk;

/// A plan's folders, each held open as the folder its checks passed, with every build lock in them
/// held, ready to delete.
#[derive(Debug)]
pub struct LockedPlan {
    folders: Vec<(PathBuf, OpenFolder)>,
    locks: HeldLocks,
}

/// Open each folder `plan` deletes, refusing one that is no longer the folder `identities` names
/// for it, in the plan's order, and take every build lock in them without waiting, or refuse
/// because a build holds one.
pub(super) fn lock(plan: &Plan, identities: &[Identity]) -> Result<LockedPlan, CleanError> {
    let mut folders = Vec::new();
    for (index, path) in plan.folders().into_iter().enumerate() {
        // A folder with no identity never passed its checks.
        let identity = identities.get(index).ok_or_else(|| changed(path))?;
        folders.push((path.to_path_buf(), open_as(path, *identity)?));
    }
    let mut locks = HeldLocks::default();
    for (path, folder) in &folders {
        locks.take_found(folder, path)?;
    }
    Ok(LockedPlan { folders, locks })
}

/// The folder at `path`, held open, when it is the folder `identity` names.
fn open_as(path: &Path, identity: Identity) -> Result<OpenFolder, CleanError> {
    let folder = OpenFolder::at(path).map_err(|err| {
        let gone = err.kind() == io::ErrorKind::NotFound;
        if gone || is_not_a_folder(&err) {
            changed(path)
        } else {
            io_error(path, &err)
        }
    })?;
    if folder.identity().map_err(|err| io_error(path, &err))? != identity {
        return Err(changed(path));
    }
    Ok(folder)
}

fn changed(folder: &Path) -> CleanError {
    CleanError::Changed {
        folder: folder.to_path_buf(),
    }
}

fn io_error(path: &Path, err: &io::Error) -> CleanError {
    CleanError::Io {
        path: path.to_path_buf(),
        kind: err.kind(),
    }
}

/// How far a clean has come, readable while it runs, so the card can show a folder shrinking.
#[derive(Debug, Default)]
pub struct Progress {
    /// Each folder the clean has reached.
    reached: Mutex<Vec<(PathBuf, Arc<FolderProgress>)>>,
}

impl Progress {
    /// The files and links deleted so far.
    pub fn files_deleted(&self) -> u64 {
        self.reached()
            .iter()
            .map(|(_, folder)| folder.files_deleted.load(Ordering::Relaxed))
            .sum()
    }

    /// The space `folder` has given back so far, nothing until the clean reaches it. A running
    /// count for the card: what the clean freed in the end is measured (see `Freed`).
    pub fn bytes_freed_in(&self, folder: &Path) -> u64 {
        self.reached()
            .iter()
            .find(|(reached, _)| reached == folder)
            .map_or(0, |(_, progress)| {
                progress.bytes_freed.load(Ordering::Relaxed)
            })
    }

    /// Start counting in `folder`, which the clean has reached.
    pub(super) fn reach(&self, folder: &Path) -> Arc<FolderProgress> {
        let progress = Arc::<FolderProgress>::default();
        self.reached()
            .push((folder.to_path_buf(), Arc::clone(&progress)));
        progress
    }

    fn reached(&self) -> MutexGuard<'_, Vec<(PathBuf, Arc<FolderProgress>)>> {
        // The lock is held only to read the list or push to it, which a panic cannot leave half done.
        self.reached.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// How far a clean has come in one folder.
#[derive(Debug, Default)]
pub(super) struct FolderProgress {
    files_deleted: AtomicU64,
    bytes_freed: AtomicU64,
}

impl FolderProgress {
    /// One more file or link went, giving back `bytes`.
    pub(super) fn count_file(&self, bytes: u64) {
        self.files_deleted.fetch_add(1, Ordering::Relaxed);
        self.bytes_freed.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// What a clean freed, folder by folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub folders: Vec<Freed>,
}

/// What deleting one folder freed: its allocated size with each hard link counting its share, as
/// `disk::measure` counts it, less what stays and whatever a build kept. Cargo's own total counts every hard link
/// in full and so claims more than it frees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Freed {
    pub folder: PathBuf,
    pub bytes: u64,
    /// What the folder holds now, measured the same way.
    pub left: u64,
}

/// Why a clean stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanError {
    /// A build holds this lock, so the folder holding it is in use.
    BuildRunning { lock: PathBuf },
    /// This is no longer the folder the clean was worked out for: it is gone, or something else
    /// stands at its path.
    Changed { folder: PathBuf },
    /// Reading, locking or deleting failed at `path`.
    Io { path: PathBuf, kind: io::ErrorKind },
}

impl LockedPlan {
    /// Delete the folders, counting each file in `progress` as it goes. A folder moved away
    /// since its locks were taken, with another put at its path, is left as it is, and so is the
    /// other.
    pub fn delete(self, progress: &Progress) -> Result<Cleaned, CleanError> {
        let LockedPlan { folders, mut locks } = self;
        let before = folders
            .iter()
            .map(|(path, _)| size_of(path))
            .collect::<Result<Vec<u64>, CleanError>>()?;
        for (path, folder) in &folders {
            locks.take_found(folder, path)?;
        }
        // Each path must still lead to the folder held open, so that one put in its place is
        // refused as it would be before the locks.
        for (path, folder) in &folders {
            let identity = folder.identity().map_err(|err| io_error(path, &err))?;
            open_as(path, identity)?;
        }
        for (path, folder) in &folders {
            removal::clear(folder, path, &mut locks, &progress.reach(path))?;
        }
        let freed = folders
            .iter()
            .zip(before)
            .map(|((path, _), before)| {
                let left = size_of(path)?;
                Ok(Freed {
                    folder: path.clone(),
                    bytes: before.saturating_sub(left),
                    left,
                })
            })
            .collect::<Result<Vec<Freed>, CleanError>>()?;
        Ok(Cleaned { folders: freed })
    }
}

/// `folder`'s size as `disk::measure` counts it, nothing when it is gone.
fn size_of(folder: &Path) -> Result<u64, CleanError> {
    match disk::measure(folder) {
        Ok(usage) => Ok(usage.bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(CleanError::Io {
            path: folder.to_path_buf(),
            kind: err.kind(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::map::clean::locks::tests::{built, hold_like_cargo};
    use crate::map::clean::{CleanPlan, ItsProject, Origin, PlannedFolder, Reach, plan};
    use crate::map::test_support::{CACHEDIR_TAG, TempDir};

    /// Lock the clean of `folders`, each checked as the folder at its path now.
    fn lock_folders(folders: &[&Path]) -> Result<LockedPlan, CleanError> {
        let plan = Plan::CargoClean(CleanPlan {
            workspace: folders[0].parent().unwrap().to_path_buf(),
            folders: folders
                .iter()
                .map(|path| PlannedFolder {
                    path: path.to_path_buf(),
                    origin: Origin::Default,
                })
                .collect(),
        });
        let identities: Vec<Identity> = folders
            .iter()
            .map(|folder| OpenFolder::at(folder).unwrap().identity().unwrap())
            .collect();
        lock(&plan, &identities)
    }

    #[test]
    fn a_cache_swapped_for_another_folder_after_its_checks_is_refused_before_its_locks() {
        let fixture = TempDir::new("run-swapped-after-checks");
        let cache = fixture.write("loose/out/CACHEDIR.TAG", CACHEDIR_TAG);
        let cache = cache.parent().unwrap().to_path_buf();
        let env = fixture.cargo_env();
        let project = fixture.path("loose");
        let checked = plan(&cache, ItsProject::At(&project), &env, &Reach::Anywhere).unwrap();
        // An untagged folder put in its place, as preparing the clean goes on to its locks.
        fs::rename(&cache, fixture.path("loose/out.old")).unwrap();
        let precious = fixture.write("loose/out/precious.txt", "keep");

        assert_eq!(
            checked.lock().map(drop),
            Err(CleanError::Changed { folder: cache })
        );
        assert!(precious.exists());
    }

    #[test]
    fn deleting_frees_each_folders_size_and_leaves_only_its_lock_files_and_tag() {
        let fixture = TempDir::new("run-delete");
        let target = built(&fixture, &["debug", "release"]);
        let build_dir = fixture.write("app/cache/debug/deps/libdep.rlib", [9; 32 * 1024]);
        let build_dir = build_dir.ancestors().nth(3).unwrap().to_path_buf();
        fixture.write("app/src/main.rs", "fn main() {}");
        let size_of = |folder: &Path| disk::measure(folder).unwrap().bytes;
        let before = [size_of(&target), size_of(&build_dir)];
        let progress = Progress::default();

        let cleaned = lock_folders(&[&target, &build_dir])
            .unwrap()
            .delete(&progress)
            .unwrap();

        assert_eq!(
            cleaned.folders,
            vec![
                Freed {
                    folder: target.clone(),
                    bytes: before[0] - size_of(&target),
                    left: size_of(&target),
                },
                Freed {
                    folder: build_dir.clone(),
                    bytes: before[1] - size_of(&build_dir),
                    left: size_of(&build_dir),
                },
            ]
        );
        assert!(cleaned.folders[0].bytes >= 2 * 64 * 1024);
        assert!(cleaned.folders[1].bytes >= 32 * 1024);
        assert!(target.join("CACHEDIR.TAG").exists());
        for profile in ["debug", "release"] {
            assert!(target.join(profile).join(".cargo-build-lock").exists());
            assert!(!target.join(profile).join("deps").exists());
        }
        assert!(!build_dir.join("debug").exists());
        assert!(fixture.path("app/src/main.rs").exists());
        assert_eq!(progress.files_deleted(), 3, "the libraries");
        assert_eq!(
            progress.bytes_freed_in(&target),
            cleaned.folders[0].bytes,
            "the running count ends where the measurement does"
        );
        assert_eq!(
            progress.bytes_freed_in(&build_dir),
            cleaned.folders[1].bytes
        );
        assert_eq!(progress.bytes_freed_in(&fixture.path("app/src")), 0);
    }

    #[test]
    fn a_hard_linked_file_is_freed_once() {
        let fixture = TempDir::new("run-hardlink");
        let original = fixture.write("app/target/debug/deps/libapp.rlib", [7; 64 * 1024]);
        let target = fixture.path("app/target");
        fs::hard_link(&original, target.join("debug/libapp.rlib")).unwrap();
        let progress = Progress::default();

        let cleaned = lock_folders(&[&target]).unwrap().delete(&progress).unwrap();

        let freed = cleaned.folders[0].bytes;
        assert!(
            (64 * 1024..2 * 64 * 1024).contains(&freed),
            "{freed}: the file's blocks once, not once per link"
        );
        assert_eq!(progress.bytes_freed_in(&target), freed);
    }

    #[test]
    fn a_folder_swapped_after_the_locks_stops_the_delete_with_both_untouched() {
        let fixture = TempDir::new("run-swapped");
        let target = built(&fixture, &["debug"]);
        let locked = lock_folders(&[&target]).unwrap();
        let moved = fixture.path("app/target.old");
        fs::rename(&target, &moved).unwrap();
        let other = fixture.write("app/target/other", "keep");

        assert_eq!(
            locked.delete(&Progress::default()),
            Err(CleanError::Changed { folder: target })
        );
        assert!(other.exists());
        assert!(moved.join("debug/deps/libapp.rlib").exists());
    }

    #[test]
    fn a_build_that_started_in_a_new_profile_folder_since_the_locks_stops_the_delete_untouched() {
        let fixture = TempDir::new("run-new-profile");
        let target = built(&fixture, &["debug"]);
        let locked = lock_folders(&[&target]).unwrap();
        built(&fixture, &["release"]);
        let busy = target.join("release/.cargo-build-lock");
        let _building = hold_like_cargo(&busy);

        assert_eq!(
            locked.delete(&Progress::default()),
            Err(CleanError::BuildRunning { lock: busy })
        );
        assert!(target.join("debug/deps/libapp.rlib").exists());
    }
}
