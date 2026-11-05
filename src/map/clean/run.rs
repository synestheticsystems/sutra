//! Deleting a plan's folders under cargo's build locks.
//!
//! `cargo clean` takes no lock (`src/ops/cargo_clean.rs:125-139`) and deletes a running build's
//! files from under it. Sutra refuses while a build runs, and makes a build that starts during
//! the delete wait for it. In order:
//!  1. Take every lock in the folders, or refuse when a build holds one (`lock`).
//!  2. Measure the folders, while every link of a file is still there to share its size.
//!  3. Find the locks again and take any that appeared while measuring took its seconds, still
//!     refusing when a build holds one, and refuse a folder that is no longer the one measured.
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

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::locks::HeldLocks;
use super::{Plan, removal};
use crate::map::disk;

/// A plan's folders with every build lock in them held, ready to delete.
#[derive(Debug)]
pub struct LockedPlan {
    /// Each folder, and which folder its path led to when the locks were taken.
    folders: Vec<(PathBuf, FolderIdentity)>,
    locks: HeldLocks,
}

/// Which folder a path leads to, so that a clean deletes the folder it was asked about and not one
/// put in its place since, whether at that path or by a link anywhere above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FolderIdentity {
    device: u64,
    inode: u64,
}

impl FolderIdentity {
    /// Of the folder at `path`, which must be a folder and not a link to one.
    fn of(path: &Path) -> Result<FolderIdentity, CleanError> {
        let changed = || CleanError::Changed {
            folder: path.to_path_buf(),
        };
        let meta = fs::symlink_metadata(path).map_err(|_| changed())?;
        if !meta.is_dir() {
            return Err(changed());
        }
        Ok(FolderIdentity {
            device: meta.dev(),
            inode: meta.ino(),
        })
    }
}

/// Take every build lock in the folders `plan` deletes, without waiting, or refuse because a
/// build holds one.
pub fn lock(plan: &Plan) -> Result<LockedPlan, CleanError> {
    let folders = plan.folders();
    let mut locks = HeldLocks::default();
    locks.take_found(&folders)?;
    let folders = folders
        .into_iter()
        .map(|folder| Ok((folder.to_path_buf(), FolderIdentity::of(folder)?)))
        .collect::<Result<_, CleanError>>()?;
    Ok(LockedPlan { folders, locks })
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
    /// Which folder each path led to when the locks were taken, in the plan's order.
    pub(super) fn identities(&self) -> Vec<FolderIdentity> {
        self.folders.iter().map(|(_, identity)| *identity).collect()
    }

    /// Refuse unless each path leads where `identities` says it did.
    pub(super) fn check_still(&self, identities: &[FolderIdentity]) -> Result<(), CleanError> {
        for ((folder, now), then) in self.folders.iter().zip(identities) {
            if now != then {
                return Err(CleanError::Changed {
                    folder: folder.clone(),
                });
            }
        }
        Ok(())
    }

    /// Delete the folders, counting each file in `progress` as it goes.
    pub fn delete(self, progress: &Progress) -> Result<Cleaned, CleanError> {
        let LockedPlan { folders, mut locks } = self;
        let paths: Vec<&Path> = folders.iter().map(|(path, _)| path.as_path()).collect();
        let before = paths
            .iter()
            .map(|folder| size_of(folder))
            .collect::<Result<Vec<u64>, CleanError>>()?;
        locks.take_found(&paths)?;
        for (folder, identity) in &folders {
            if FolderIdentity::of(folder)? != *identity {
                return Err(CleanError::Changed {
                    folder: folder.clone(),
                });
            }
            removal::clear(folder, &mut locks, &progress.reach(folder))?;
        }
        let freed = paths
            .iter()
            .zip(before)
            .map(|(folder, before)| {
                let left = size_of(folder)?;
                Ok(Freed {
                    folder: folder.to_path_buf(),
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
    use super::*;
    use crate::map::clean::locks::tests::{built, hold_like_cargo};
    use crate::map::clean::{CleanPlan, Origin, PlannedFolder};
    use crate::map::test_support::TempDir;

    fn plan_of(folders: &[&Path]) -> Plan {
        Plan::CargoClean(CleanPlan {
            workspace: folders[0].parent().unwrap().to_path_buf(),
            folders: folders
                .iter()
                .map(|path| PlannedFolder {
                    path: path.to_path_buf(),
                    origin: Origin::Default,
                })
                .collect(),
        })
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

        let cleaned = lock(&plan_of(&[&target, &build_dir]))
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

        let cleaned = lock(&plan_of(&[&target]))
            .unwrap()
            .delete(&progress)
            .unwrap();

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
        let locked = lock(&plan_of(&[&target])).unwrap();
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
        let locked = lock(&plan_of(&[&target])).unwrap();
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
