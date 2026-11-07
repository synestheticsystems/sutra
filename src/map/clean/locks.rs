//! Cargo's build locks: the files cargo `flock`s in each profile folder while it builds there.
//! Sutra takes them exclusively and without waiting, so that it never deletes a folder a build
//! is using, and so that a build starting meanwhile waits until Sutra is done.
//!
//! Since cargo 1.96 a build holds three locks in each profile folder it uses, that is
//! `<folder>/[<triple>/]<profile>`, and as deep as anything else with a target folder of its own
//! inside (rust-analyzer's `target/rust-analyzer`, or any `--target-dir`): `.cargo-build-lock`
//! exclusively, `.cargo-lock` shared, so that older cargos, which hold it exclusively, still keep
//! out, and `.cargo-artifact-lock` exclusively, which `cargo check` does not take
//! (`src/compiler/layout.rs:233-335`). An exclusive try conflicts with each of those holds. These
//! are `flock(2)` locks, the call cargo makes through std's `File::lock` on macOS.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use rayon::prelude::*;

use super::CleanError;
use super::open_folder::{Identity, Kind, OpenFolder};
use crate::map::scan;

/// The files cargo locks in a profile folder.
const LOCK_NAMES: [&str; 3] = [".cargo-build-lock", ".cargo-lock", ".cargo-artifact-lock"];

/// Cargo's locks that Sutra holds, each released when the set is dropped. A lock is known by the
/// file locked, its device and inode, and never by its path: a script can remove a profile folder
/// whose locks Sutra holds, and a build make it again, with new lock files at the same paths that
/// the build holds.
#[derive(Debug, Default)]
pub(super) struct HeldLocks {
    held: Vec<HeldLock>,
}

#[derive(Debug)]
struct HeldLock {
    file: Identity,
    _flock: Flock<File>,
}

impl HeldLocks {
    /// Find the lock files in `folder`, at `path`, and take each not held yet. Returns whether the
    /// folder holds any lock file, held before or taken now. When a build holds one, those this
    /// call took are let go again and the build's lock is named.
    pub(super) fn take_found(
        &mut self,
        folder: &OpenFolder,
        path: &Path,
    ) -> Result<bool, CleanError> {
        let found = find(folder, path)?;
        let holds_any = !found.is_empty();
        let before = self.held.len();
        for lock in found {
            if self.holds(lock.identity) {
                continue;
            }
            match take(lock) {
                Ok(held) => self.held.push(held),
                Err(err) => {
                    self.held.truncate(before);
                    return Err(err);
                }
            }
        }
        Ok(holds_any)
    }

    /// Whether Sutra holds the lock of the file `file` names.
    pub(super) fn holds(&self, file: Identity) -> bool {
        self.held.iter().any(|lock| lock.file == file)
    }
}

/// A lock file found, opened as the search found it, so that its lock is taken on that file and
/// not on whatever stands at its path by then.
#[derive(Debug)]
struct LockFile {
    path: PathBuf,
    file: File,
    identity: Identity,
}

/// Lock `lock` exclusively without waiting.
fn take(lock: LockFile) -> Result<HeldLock, CleanError> {
    let LockFile {
        path,
        file,
        identity,
    } = lock;
    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(flock) => Ok(HeldLock {
            file: identity,
            _flock: flock,
        }),
        Err((_, Errno::EWOULDBLOCK)) => Err(CleanError::BuildRunning { lock: path }),
        // A volume that cannot lock files at all (`Unsupported`) cannot tell a running build
        // from an idle folder either, so the clean stops there too.
        Err((_, errno)) => {
            let kind = io::Error::from(errno).kind();
            Err(CleanError::Io { path, kind })
        }
    }
}

/// Every lock file in `folder`, at `path`, at any depth, each opened. A build's profile folders lie
/// wherever its target folder does, and that can be anywhere in a build folder Sutra deletes, even
/// inside another build's profile folder, so no folder is passed over. Each folder is entered
/// through the one holding it, never along a path, and only folder entries are read and lock files
/// opened, never another file's metadata or contents: a `target` of 8,200 folders and 115,000
/// entries took 60 ms on discovery's threads and 145 ms on one (M4 Max, 2026-11-04). A folder that
/// cannot be listed could hide a lock, so it fails the search.
fn find(folder: &OpenFolder, path: &Path) -> Result<Vec<LockFile>, CleanError> {
    scan::on_discovery_threads(|| find_in(folder, path))
}

fn find_in(dir: &OpenFolder, path: &Path) -> Result<Vec<LockFile>, CleanError> {
    let io_error = |at: PathBuf, err: io::Error| CleanError::Io {
        path: at,
        kind: err.kind(),
    };
    let entries = dir
        .entries()
        .map_err(|err| io_error(path.to_path_buf(), err))?;
    let mut locks = Vec::new();
    let mut subfolders = Vec::new();
    for entry in entries {
        match entry.kind {
            Kind::File if is_lock_name(&entry.name) => {
                let at = path.join(&entry.name);
                match dir.file(&entry.name) {
                    Ok(file) => {
                        let identity =
                            Identity::of_file(&file).map_err(|err| io_error(at.clone(), err))?;
                        locks.push(LockFile {
                            path: at,
                            file,
                            identity,
                        });
                    }
                    // Gone since the listing, with the folder that held it.
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(io_error(at, err)),
                }
            }
            Kind::Folder => subfolders.push(entry.name),
            Kind::File | Kind::Other => {}
        }
    }
    let below = subfolders
        .par_iter()
        .map(|name| {
            let at = path.join(name);
            match dir.folder(name) {
                Ok(subfolder) => find_in(&subfolder, &at),
                // Removed since its parent was listed.
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(err) => Err(io_error(at, err)),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    locks.extend(below.into_iter().flatten());
    Ok(locks)
}

fn is_lock_name(name: &OsStr) -> bool {
    LOCK_NAMES.iter().any(|lock| name == *lock)
}

#[cfg(test)]
pub(super) mod tests {
    use std::fs;

    use super::*;
    use crate::map::test_support::TempDir;

    impl HeldLocks {
        /// Whether Sutra holds the lock of the file at `path` now.
        pub(in crate::map::clean) fn holds_file_at(&self, path: &Path) -> bool {
            self.holds(Identity::of_file(&File::open(path).unwrap()).unwrap())
        }

        /// `take_found` in the folder at `path`, opened for it.
        pub(in crate::map::clean) fn take_found_at(
            &mut self,
            path: &Path,
        ) -> Result<bool, CleanError> {
            self.take_found(&OpenFolder::at(path).unwrap(), path)
        }
    }

    /// The lock files `find` finds in the folder at `path`, by path.
    fn found_in(path: &Path) -> Result<Vec<PathBuf>, CleanError> {
        let found = find(&OpenFolder::at(path).unwrap(), path)?;
        Ok(found.into_iter().map(|lock| lock.path).collect())
    }

    /// Hold `lock` as a running cargo 1.99 does: `.cargo-lock` shared, the others exclusively.
    pub(in crate::map::clean) fn hold_like_cargo(lock: &Path) -> Flock<File> {
        let shared = lock.file_name().is_some_and(|name| name == ".cargo-lock");
        let mode = if shared {
            FlockArg::LockSharedNonblock
        } else {
            FlockArg::LockExclusiveNonblock
        };
        Flock::lock(File::open(lock).unwrap(), mode)
            .map_err(|(_, errno)| errno)
            .unwrap()
    }

    /// A `target` with a build's three lock files in each of `profiles`, and some output.
    pub(in crate::map::clean) fn built(fixture: &TempDir, profiles: &[&str]) -> PathBuf {
        fixture.write(
            "app/target/CACHEDIR.TAG",
            "Signature: 8a477f597d28d172789f06886806bc55",
        );
        for profile in profiles {
            for lock in LOCK_NAMES {
                fixture.write(&format!("app/target/{profile}/{lock}"), "");
            }
            fixture.write(
                &format!("app/target/{profile}/deps/libapp.rlib"),
                [7; 64 * 1024],
            );
        }
        fixture.path("app/target")
    }

    #[test]
    fn locks_are_found_at_any_depth_inside_profile_folders_too_and_only_by_their_names() {
        let fixture = TempDir::new("locks-find");
        let profiles = [
            "debug",
            "aarch64-apple-darwin/release",
            "rust-analyzer/flycheck0/aarch64-apple-darwin/debug",
            "a/b/c/d/e/f/g/debug",
            "debug/build/app-0123/out/nested/debug",
        ];
        let target = built(&fixture, &profiles);
        let directly_in_it = fixture.write("app/target/.cargo-lock", "");
        fixture.write("app/target/debug/deps/not-a-lock", "");
        let linked = fixture.write("elsewhere/debug/.cargo-lock", "");
        std::os::unix::fs::symlink(linked.parent().unwrap(), target.join("linked")).unwrap();

        let mut found = found_in(&target).unwrap();
        found.sort();

        let mut expected: Vec<PathBuf> = profiles
            .iter()
            .flat_map(|profile| LOCK_NAMES.map(|lock| target.join(profile).join(lock)))
            .chain([directly_in_it])
            .collect();
        expected.sort();
        assert_eq!(found, expected);
    }

    #[test]
    fn a_folder_the_search_cannot_list_fails_it_at_any_depth() {
        let fixture = TempDir::new("locks-sealed");
        let target = built(&fixture, &["debug"]);
        let sealed = fixture.dir("app/target/debug/build/app-0123/out/sealed");
        fixture.restrict(&sealed, 0o000);

        assert_eq!(
            found_in(&target),
            Err(CleanError::Io {
                path: sealed,
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn a_build_in_a_deep_profile_folder_refuses() {
        let fixture = TempDir::new("locks-deep");
        let target = built(&fixture, &["debug", "a/b/c/debug"]);
        let busy = target.join("a/b/c/debug/.cargo-build-lock");
        let _building = hold_like_cargo(&busy);

        assert_eq!(
            HeldLocks::default().take_found_at(&target),
            Err(CleanError::BuildRunning { lock: busy })
        );
    }

    #[test]
    fn a_build_with_its_target_folder_inside_a_free_profile_folder_refuses() {
        let fixture = TempDir::new("locks-inside-profile");
        let target = built(&fixture, &["debug", "debug/own-target/debug"]);
        let busy = target.join("debug/own-target/debug/.cargo-build-lock");
        let _building = hold_like_cargo(&busy);

        assert_eq!(
            HeldLocks::default().take_found_at(&target),
            Err(CleanError::BuildRunning { lock: busy })
        );
    }

    #[test]
    fn any_lock_a_build_holds_refuses_and_lets_go_of_those_just_taken() {
        let fixture = TempDir::new("locks-busy");
        let target = built(&fixture, &["debug", "release"]);
        for lock in LOCK_NAMES {
            let busy = target.join("release").join(lock);
            let _building = hold_like_cargo(&busy);
            let mut locks = HeldLocks::default();
            assert_eq!(
                locks.take_found_at(&target),
                Err(CleanError::BuildRunning { lock: busy }),
                "{lock}"
            );
            assert!(locks.held.is_empty(), "{lock}");
        }
        let mut locks = HeldLocks::default();
        assert_eq!(
            locks.take_found_at(&target),
            Ok(true),
            "every build finished"
        );
        assert_eq!(locks.held.len(), 6);
    }

    #[test]
    fn an_older_cargo_holding_the_cargo_lock_alone_refuses_too() {
        let fixture = TempDir::new("locks-older-cargo");
        let target = built(&fixture, &["debug"]);
        let lock = target.join("debug/.cargo-lock");
        let _old_cargo = Flock::lock(File::open(&lock).unwrap(), FlockArg::LockExclusiveNonblock)
            .map_err(|(_, errno)| errno)
            .unwrap();
        assert_eq!(
            HeldLocks::default().take_found_at(&target),
            Err(CleanError::BuildRunning { lock })
        );
    }

    #[test]
    fn held_locks_keep_a_starting_build_out_and_are_taken_once() {
        let fixture = TempDir::new("locks-held");
        let target = built(&fixture, &["debug"]);
        let mut locks = HeldLocks::default();
        locks.take_found_at(&target).unwrap();
        locks.take_found_at(&target).unwrap();
        assert_eq!(
            locks.held.len(),
            3,
            "a lock already held is not taken twice"
        );

        let starting = File::open(target.join("debug/.cargo-build-lock")).unwrap();
        assert!(matches!(
            Flock::lock(starting, FlockArg::LockExclusiveNonblock),
            Err((_, Errno::EWOULDBLOCK))
        ));
    }

    #[test]
    fn a_lock_file_put_at_a_held_locks_path_is_not_held_and_is_taken_or_refuses() {
        let fixture = TempDir::new("locks-replaced");
        let target = built(&fixture, &["debug", "release"]);
        let mut locks = HeldLocks::default();
        locks.take_found_at(&target).unwrap();
        let replace = |lock: &Path| {
            fs::remove_file(lock).unwrap();
            fs::write(lock, "").unwrap();
        };
        let free = target.join("debug/.cargo-build-lock");
        let busy = target.join("release/.cargo-build-lock");
        replace(&free);
        replace(&busy);
        assert!(!locks.holds_file_at(&free), "its path is no lock held");

        let building = hold_like_cargo(&busy);
        assert_eq!(
            locks.take_found_at(&target),
            Err(CleanError::BuildRunning { lock: busy }),
            "a build holds the one put in its place"
        );
        drop(building);
        locks.take_found_at(&target).unwrap();

        assert!(locks.holds_file_at(&free));
        let starting = File::open(&free).unwrap();
        assert!(matches!(
            Flock::lock(starting, FlockArg::LockExclusiveNonblock),
            Err((_, Errno::EWOULDBLOCK))
        ));
    }
}
