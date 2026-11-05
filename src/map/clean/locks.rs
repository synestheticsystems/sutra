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
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use rayon::prelude::*;

use super::CleanError;
use crate::map::scan;

/// The files cargo locks in a profile folder.
const LOCK_NAMES: [&str; 3] = [".cargo-build-lock", ".cargo-lock", ".cargo-artifact-lock"];

/// Cargo's locks that Sutra holds, each released when the set is dropped.
#[derive(Debug, Default)]
pub(super) struct HeldLocks {
    held: Vec<HeldLock>,
}

#[derive(Debug)]
struct HeldLock {
    path: PathBuf,
    _flock: Flock<File>,
}

impl HeldLocks {
    /// Find the lock files in `folders` and take each not held yet.
    pub(super) fn take_found(&mut self, folders: &[&Path]) -> Result<(), CleanError> {
        for folder in folders {
            self.take_all(find(folder)?)?;
        }
        Ok(())
    }

    /// Take each of `paths` not held yet, a file that is gone skipped (its folder went with it).
    /// When a build holds one, those this call took are let go again and the build's lock is
    /// named.
    pub(super) fn take_all(&mut self, paths: Vec<PathBuf>) -> Result<(), CleanError> {
        let before = self.held.len();
        for path in paths {
            if self.holds(&path) {
                continue;
            }
            match take(path) {
                Ok(Some(lock)) => self.held.push(lock),
                Ok(None) => {}
                Err(err) => {
                    self.held.truncate(before);
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    pub(super) fn holds(&self, path: &Path) -> bool {
        self.held.iter().any(|lock| lock.path == path)
    }
}

/// Lock `path` exclusively without waiting; `None` when the file is gone.
fn take(path: PathBuf) -> Result<Option<HeldLock>, CleanError> {
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            let kind = err.kind();
            return Err(CleanError::Io { path, kind });
        }
    };
    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(flock) => Ok(Some(HeldLock {
            path,
            _flock: flock,
        })),
        Err((_, Errno::EWOULDBLOCK)) => Err(CleanError::BuildRunning { lock: path }),
        // A volume that cannot lock files at all (`Unsupported`) cannot tell a running build
        // from an idle folder either, so the clean stops there too.
        Err((_, errno)) => {
            let kind = io::Error::from(errno).kind();
            Err(CleanError::Io { path, kind })
        }
    }
}

/// Every lock file in `folder`, at any depth. A build's profile folders lie wherever its target
/// folder does, and that can be anywhere in a build folder Sutra deletes, even inside another
/// build's profile folder, so no folder is passed over. Only folder entries are read, never a
/// file's metadata or contents: a `target` of 8,200 folders and 115,000 entries took 60 ms on
/// discovery's threads and 145 ms on one (M4 Max, 2026-11-04). A folder that cannot be listed could
/// hide a lock, so it fails the search.
pub(super) fn find(folder: &Path) -> Result<Vec<PathBuf>, CleanError> {
    scan::on_discovery_threads(|| find_in(folder))
}

fn find_in(dir: &Path) -> Result<Vec<PathBuf>, CleanError> {
    let io_error = |err: io::Error| CleanError::Io {
        path: dir.to_path_buf(),
        kind: err.kind(),
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // Removed since its parent was listed.
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(io_error(err)),
    };
    let mut locks = Vec::new();
    let mut subfolders = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        // `file_type` does not follow symlinks, so a linked folder is never entered.
        let kind = entry.file_type().map_err(io_error)?;
        if kind.is_file() && is_lock_name(&entry.file_name()) {
            locks.push(entry.path());
        } else if kind.is_dir() {
            subfolders.push(entry.path());
        }
    }
    let below = subfolders
        .par_iter()
        .map(|subfolder| find_in(subfolder))
        .collect::<Result<Vec<_>, _>>()?;
    locks.extend(below.into_iter().flatten());
    Ok(locks)
}

fn is_lock_name(name: &OsStr) -> bool {
    LOCK_NAMES.iter().any(|lock| name == *lock)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

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

        let mut found = find(&target).unwrap();
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
            find(&target),
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
            HeldLocks::default().take_found(&[&target]),
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
            HeldLocks::default().take_found(&[&target]),
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
                locks.take_found(&[&target]),
                Err(CleanError::BuildRunning { lock: busy }),
                "{lock}"
            );
            assert!(locks.held.is_empty(), "{lock}");
        }
        let mut locks = HeldLocks::default();
        assert_eq!(locks.take_found(&[&target]), Ok(()), "every build finished");
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
            HeldLocks::default().take_found(&[&target]),
            Err(CleanError::BuildRunning { lock })
        );
    }

    #[test]
    fn held_locks_keep_a_starting_build_out_and_are_taken_once() {
        let fixture = TempDir::new("locks-held");
        let target = built(&fixture, &["debug"]);
        let mut locks = HeldLocks::default();
        locks.take_found(&[&target]).unwrap();
        locks.take_found(&[&target]).unwrap();
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
}
