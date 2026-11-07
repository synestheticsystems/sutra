//! Deleting what a build folder holds, around cargo's lock files and the folder's tag, which stay
//! (see `run`).

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rayon::prelude::*;

use super::CleanError;
use super::locks::{self, HeldLocks};
use super::run::FolderProgress;
use crate::map::scan::CACHEDIR_TAG_FILE;

/// Delete everything in `folder` but what cargo's locks need: the lock files held, the folders
/// holding them, and `folder`'s `CACHEDIR.TAG` (cargo writes the tag, and excludes the folder
/// from Time Machine and iCloud, only when it creates the folder, so a build that took the
/// folder over would otherwise lose both). A folder that appeared with lock files of its own is
/// taken over when its locks are free and left to its build when they are not.
pub(super) fn clear(
    folder: &Path,
    locks: &mut HeldLocks,
    progress: &FolderProgress,
) -> Result<(), CleanError> {
    clear_around(folder, &folder.join(CACHEDIR_TAG_FILE), locks, progress)
}

/// `clear` of `dir`, which is the build folder or a folder in it. `tag` is the build folder's own.
fn clear_around(
    dir: &Path,
    tag: &Path,
    locks: &mut HeldLocks,
    progress: &FolderProgress,
) -> Result<(), CleanError> {
    let io_error = |err: io::Error| io_error(dir, err);
    let mut whole = Vec::new();
    for entry in listing(dir)? {
        let path = entry.path();
        if locks.holds(&path) || path == tag {
            continue;
        }
        // `file_type` does not follow symlinks: a link is deleted, never entered.
        if !entry.file_type().map_err(io_error)?.is_dir() {
            remove_file(&path, progress)?;
            continue;
        }
        let found = locks::find(&path)?;
        if found.is_empty() {
            whole.push(path);
            continue;
        }
        match locks.take_all(found) {
            Ok(()) => clear_around(&path, tag, locks, progress)?,
            // A build started in it after the locks were taken: it is the build's now.
            Err(CleanError::BuildRunning { .. }) => {}
            Err(err) => return Err(err),
        }
    }
    whole
        .par_iter()
        .try_for_each(|subfolder| remove_tree(subfolder, progress))
}

/// Delete `folder` and everything in it, its subfolders in parallel: a large build folder holds
/// hundreds of thousands of files.
fn remove_tree(folder: &Path, progress: &FolderProgress) -> Result<(), CleanError> {
    let mut subfolders = Vec::new();
    for entry in listing(folder)? {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            subfolders.push(entry.path());
        } else {
            remove_file(&entry.path(), progress)?;
        }
    }
    subfolders
        .par_iter()
        .try_for_each(|subfolder| remove_tree(subfolder, progress))?;
    match fs::remove_dir(folder) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(io_error(folder, err)),
        _ => Ok(()),
    }
}

/// What `dir` holds, read in full before any of it is deleted, as a folder read while entries
/// go from it may skip some; nothing when it is gone.
fn listing(dir: &Path) -> Result<Vec<fs::DirEntry>, CleanError> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .collect::<io::Result<Vec<_>>>()
            .map_err(|err| io_error(dir, err)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(io_error(dir, err)),
    }
}

/// Delete one entry that is no folder: a file or a symlink.
fn remove_file(path: &Path, progress: &FolderProgress) -> Result<(), CleanError> {
    let gives_back = bytes_given_back(path);
    match fs::remove_file(path) {
        Ok(()) => {
            progress.count_file(gives_back);
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(io_error(path, err)),
    }
}

/// What deleting the link at `path` gives back: the file's blocks when it is the last link, since
/// rustc hard-links most of a build folder and any other link keeps them. Two threads deleting a
/// file's last two links at once can both see another link and miss it, which only shows on the
/// card until the clean ends and the folder is measured.
fn bytes_given_back(path: &Path) -> u64 {
    fs::symlink_metadata(path).map_or(0, |meta| {
        if meta.nlink() == 1 {
            meta.blocks() * 512
        } else {
            0
        }
    })
}

fn io_error(path: &Path, err: io::Error) -> CleanError {
    CleanError::Io {
        path: path.to_path_buf(),
        kind: err.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::clean::Progress;
    use crate::map::clean::locks::tests::{built, hold_like_cargo};
    use crate::map::test_support::TempDir;

    fn take_all_in(target: &Path) -> HeldLocks {
        let mut locks = HeldLocks::default();
        locks.take_found(&[target]).unwrap();
        locks
    }

    #[test]
    fn only_a_files_last_link_counts_as_space_given_back() {
        let fixture = TempDir::new("removal-last-link");
        let target = built(&fixture, &["debug"]);
        let linked = fixture.write("app/target/debug/deps/liblinked.rlib", [1; 64 * 1024]);
        fs::hard_link(&linked, target.join("debug/liblinked.rlib")).unwrap();
        let kept = fixture.write("app/target/debug/deps/libkept.rlib", [2; 64 * 1024]);
        let outside = fixture.path("app/kept-outside.rlib");
        fs::hard_link(&kept, &outside).unwrap();
        let allocated = |path: &Path| fs::metadata(path).unwrap().blocks() * 512;
        let libapp = allocated(&target.join("debug/deps/libapp.rlib"));
        let liblinked = allocated(&linked);
        let mut locks = take_all_in(&target);
        let progress = Progress::default();

        clear(&target, &mut locks, &progress.reach(&target)).unwrap();

        assert_eq!(
            progress.bytes_freed_in(&target),
            libapp + liblinked,
            "the linked file once, and nothing for one a link outside the folder keeps"
        );
        assert!(outside.exists());
    }

    #[test]
    fn clearing_keeps_the_held_lock_files_their_folders_and_the_tag() {
        let fixture = TempDir::new("removal-clear");
        let target = built(&fixture, &["debug", "x86_64-apple-darwin/debug"]);
        fixture.write("app/target/doc/app/index.html", "<p>docs</p>");
        fixture.write("app/target/.rustc_info.json", "{}");
        let mut locks = take_all_in(&target);
        let progress = Progress::default();

        clear(&target, &mut locks, &progress.reach(&target)).unwrap();

        let mut left: Vec<String> = walk(&target);
        left.sort();
        assert_eq!(
            left,
            [
                "CACHEDIR.TAG",
                "debug",
                "debug/.cargo-artifact-lock",
                "debug/.cargo-build-lock",
                "debug/.cargo-lock",
                "x86_64-apple-darwin",
                "x86_64-apple-darwin/debug",
                "x86_64-apple-darwin/debug/.cargo-artifact-lock",
                "x86_64-apple-darwin/debug/.cargo-build-lock",
                "x86_64-apple-darwin/debug/.cargo-lock",
            ]
        );
        assert_eq!(
            progress.files_deleted(),
            4,
            "two libraries, the docs page, the cache"
        );
    }

    #[test]
    fn clearing_keeps_the_locks_of_a_profile_folder_at_any_depth_and_nothing_else_around_them() {
        let fixture = TempDir::new("removal-clear-deep");
        let target = built(
            &fixture,
            &["a/b/c/debug", "debug", "debug/own-target/debug"],
        );
        let mut locks = take_all_in(&target);

        clear(&target, &mut locks, &FolderProgress::default()).unwrap();

        let mut left = walk(&target);
        left.sort();
        let locks_in = |profile: &str| {
            [".cargo-artifact-lock", ".cargo-build-lock", ".cargo-lock"]
                .map(|lock| format!("{profile}/{lock}"))
        };
        let mut expected: Vec<String> = [
            "CACHEDIR.TAG",
            "a",
            "a/b",
            "a/b/c",
            "a/b/c/debug",
            "debug",
            "debug/own-target",
            "debug/own-target/debug",
        ]
        .map(str::to_string)
        .into_iter()
        .chain(locks_in("a/b/c/debug"))
        .chain(locks_in("debug"))
        .chain(locks_in("debug/own-target/debug"))
        .collect();
        expected.sort();
        assert_eq!(left, expected);
    }

    #[test]
    fn a_build_that_starts_inside_a_held_profile_folder_keeps_that_folder_and_the_rest_goes() {
        let fixture = TempDir::new("removal-new-inside-profile");
        let target = built(&fixture, &["debug", "release"]);
        let mut locks = take_all_in(&target);
        // A build given a target folder inside `debug` started once Sutra held `debug`'s locks.
        built(&fixture, &["debug/own-target/debug"]);
        let _building = hold_like_cargo(&target.join("debug/own-target/debug/.cargo-build-lock"));

        clear(&target, &mut locks, &FolderProgress::default()).unwrap();

        assert!(
            target
                .join("debug/own-target/debug/deps/libapp.rlib")
                .exists()
        );
        assert!(target.join("debug/deps/libapp.rlib").exists());
        assert!(!target.join("release/deps").exists());
    }

    #[test]
    fn a_profile_folder_a_build_took_after_the_locks_is_left_to_it() {
        let fixture = TempDir::new("removal-new-profile");
        let target = built(&fixture, &["debug"]);
        let mut locks = take_all_in(&target);
        // A build of another profile started once Sutra held the first one's locks.
        built(&fixture, &["release"]);
        let _building = hold_like_cargo(&target.join("release/.cargo-build-lock"));

        clear(&target, &mut locks, &FolderProgress::default()).unwrap();

        assert!(target.join("release/deps/libapp.rlib").exists());
        assert!(!target.join("debug/deps").exists());
    }

    #[test]
    fn a_profile_folder_that_appeared_free_is_taken_and_cleared() {
        let fixture = TempDir::new("removal-new-free-profile");
        let target = built(&fixture, &["debug"]);
        let mut locks = take_all_in(&target);
        built(&fixture, &["release"]);

        clear(&target, &mut locks, &FolderProgress::default()).unwrap();

        assert!(!target.join("release/deps").exists());
        assert!(locks.holds(&target.join("release/.cargo-build-lock")));
    }

    /// Every path below `root`, relative to it.
    fn walk(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            found.push(path.strip_prefix(root).unwrap().display().to_string());
            if path.is_dir() {
                found.extend(
                    walk(&path)
                        .into_iter()
                        .map(|below| format!("{}/{below}", path.file_name().unwrap().display())),
                );
            }
        }
        found
    }
}
