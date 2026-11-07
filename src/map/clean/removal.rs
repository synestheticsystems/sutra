//! Deleting what a build folder holds, around cargo's lock files and the folder's tag, which stay
//! (see `run`). Each folder is reached through the one holding it, from the build folder held open
//! (see `open_folder`), so nothing outside that folder is ever deleted, whatever is swapped for a
//! link on the way while the delete runs.

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use rayon::prelude::*;

use super::CleanError;
use super::locks::HeldLocks;
use super::open_folder::{Entry, Kind, OpenFolder, Stat};
use super::run::FolderProgress;
use crate::map::scan::{self, CACHEDIR_TAG_FILE};

/// Delete everything in `folder`, at `path`, but what cargo's locks need: the lock files held, the
/// folders holding them, and `folder`'s `CACHEDIR.TAG` (cargo writes the tag, and excludes the
/// folder from Time Machine and iCloud, only when it creates the folder, so a build that took the
/// folder over would otherwise lose both). A folder that appeared with lock files of its own is
/// taken over when its locks are free and left to its build when they are not.
pub(super) fn clear(
    folder: &OpenFolder,
    path: &Path,
    locks: &mut HeldLocks,
    progress: &FolderProgress,
) -> Result<(), CleanError> {
    let tag = OsStr::new(CACHEDIR_TAG_FILE);
    clear_around(folder, path, Some(tag), locks, progress)
}

/// `clear` of `dir`, at `path`, which is the build folder or a folder in it. `tag` names the build
/// folder's own tag, in the build folder alone.
fn clear_around(
    dir: &OpenFolder,
    path: &Path,
    tag: Option<&OsStr>,
    locks: &mut HeldLocks,
    progress: &FolderProgress,
) -> Result<(), CleanError> {
    let mut whole = Vec::new();
    for Entry { name, kind } in listing(dir, path)? {
        let at = path.join(&name);
        if locks.holds(&at) || Some(name.as_os_str()) == tag {
            continue;
        }
        // A link is deleted, never entered.
        if kind != Kind::Folder {
            remove_file(dir, path, &name, progress)?;
            continue;
        }
        let subfolder = match dir.folder(&name) {
            Ok(subfolder) => subfolder,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(io_error(&at, err)),
        };
        match locks.take_found(&subfolder, &at) {
            Ok(false) => whole.push(name),
            Ok(true) => clear_around(&subfolder, &at, None, locks, progress)?,
            // A build started in it after the locks were taken: it is the build's now.
            Err(CleanError::BuildRunning { .. }) => {}
            Err(err) => return Err(err),
        }
    }
    // Deleting is metadata I/O, as discovery's walk is, and more threads only contend in the
    // kernel: 42,000 files in 900 folders went in 1.2 to 1.6 s on discovery's four threads, and in
    // 3.2 to 3.4 s on sixteen (M4 Max, 2026-11-06). Each folder being deleted is held open, and
    // fewer threads hold fewer at once, some 20 there, far inside the 256 files an app may open.
    scan::on_discovery_threads(|| {
        whole
            .par_iter()
            .try_for_each(|name| remove_tree(dir, path, name, progress))
    })
}

/// Delete the folder `name` in `parent`, at `parent_path`, and everything in it, its subfolders in
/// parallel: a large build folder holds hundreds of thousands of files.
fn remove_tree(
    parent: &OpenFolder,
    parent_path: &Path,
    name: &OsStr,
    progress: &FolderProgress,
) -> Result<(), CleanError> {
    let path = parent_path.join(name);
    let folder = match parent.folder(name) {
        Ok(folder) => folder,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(io_error(&path, err)),
    };
    let mut subfolders = Vec::new();
    for Entry { name, kind } in listing(&folder, &path)? {
        if kind == Kind::Folder {
            subfolders.push(name);
        } else {
            remove_file(&folder, &path, &name, progress)?;
        }
    }
    subfolders
        .par_iter()
        .try_for_each(|subfolder| remove_tree(&folder, &path, subfolder, progress))?;
    drop(folder);
    match parent.remove_folder(name) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(io_error(&path, err)),
        _ => Ok(()),
    }
}

/// What `dir`, at `path`, holds (see `OpenFolder::entries`).
fn listing(dir: &OpenFolder, path: &Path) -> Result<Vec<Entry>, CleanError> {
    dir.entries().map_err(|err| io_error(path, err))
}

/// Delete `name` in `folder`, at `path`, which is no folder: a file or a link.
fn remove_file(
    folder: &OpenFolder,
    path: &Path,
    name: &OsStr,
    progress: &FolderProgress,
) -> Result<(), CleanError> {
    let gives_back = folder.stat(name).map_or(0, |stat| bytes_given_back(&stat));
    match folder.remove_file(name) {
        Ok(()) => {
            progress.count_file(gives_back);
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(io_error(&path.join(name), err)),
    }
}

/// What deleting the name `stat` read gives back: the file's blocks when it is the last link, since
/// rustc hard-links most of a build folder and any other link keeps them. Two threads deleting a
/// file's last two links at once can both see another link and miss it, which only shows on the
/// card until the clean ends and the folder is measured.
fn bytes_given_back(stat: &Stat) -> u64 {
    if stat.links() == 1 {
        stat.allocated_bytes()
    } else {
        0
    }
}

fn io_error(path: &Path, err: io::Error) -> CleanError {
    CleanError::Io {
        path: path.to_path_buf(),
        kind: err.kind(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    use super::*;
    use crate::map::clean::Progress;
    use crate::map::clean::locks::tests::{built, hold_like_cargo};
    use crate::map::test_support::TempDir;

    fn take_all_in(target: &Path) -> HeldLocks {
        let mut locks = HeldLocks::default();
        locks.take_found_at(target).unwrap();
        locks
    }

    /// `clear` of the folder at `target`, opened for it.
    fn clear_at(
        target: &Path,
        locks: &mut HeldLocks,
        progress: &FolderProgress,
    ) -> Result<(), CleanError> {
        clear(&OpenFolder::at(target).unwrap(), target, locks, progress)
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

        clear_at(&target, &mut locks, &progress.reach(&target)).unwrap();

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

        clear_at(&target, &mut locks, &progress.reach(&target)).unwrap();

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

        clear_at(&target, &mut locks, &FolderProgress::default()).unwrap();

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
    fn a_folder_swapped_for_a_link_above_the_build_folder_once_it_is_open_leads_nowhere_else() {
        let fixture = TempDir::new("removal-parent-link");
        let target = built(&fixture, &["debug"]);
        let elsewhere = fixture.write("elsewhere/target/debug/deps/libapp.rlib", "precious");
        let mut locks = take_all_in(&target);
        let opened = OpenFolder::at(&target).unwrap();
        // The project moved away and a link to another put at its path, after the last check
        // that the build folder's path still leads to it.
        fs::rename(fixture.path("app"), fixture.path("app.old")).unwrap();
        std::os::unix::fs::symlink(fixture.path("elsewhere"), fixture.path("app")).unwrap();

        clear(&opened, &target, &mut locks, &FolderProgress::default()).unwrap();

        assert!(elsewhere.exists(), "the link's folder is untouched");
        assert!(!fixture.path("app.old/target/debug/deps").exists());
        assert!(fixture.path("app.old/target/CACHEDIR.TAG").exists());
    }

    #[test]
    fn a_build_that_starts_inside_a_held_profile_folder_keeps_that_folder_and_the_rest_goes() {
        let fixture = TempDir::new("removal-new-inside-profile");
        let target = built(&fixture, &["debug", "release"]);
        let mut locks = take_all_in(&target);
        // A build given a target folder inside `debug` started once Sutra held `debug`'s locks.
        built(&fixture, &["debug/own-target/debug"]);
        let _building = hold_like_cargo(&target.join("debug/own-target/debug/.cargo-build-lock"));

        clear_at(&target, &mut locks, &FolderProgress::default()).unwrap();

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

        clear_at(&target, &mut locks, &FolderProgress::default()).unwrap();

        assert!(target.join("release/deps/libapp.rlib").exists());
        assert!(!target.join("debug/deps").exists());
    }

    #[test]
    fn a_profile_folder_that_appeared_free_is_taken_and_cleared() {
        let fixture = TempDir::new("removal-new-free-profile");
        let target = built(&fixture, &["debug"]);
        let mut locks = take_all_in(&target);
        built(&fixture, &["release"]);

        clear_at(&target, &mut locks, &FolderProgress::default()).unwrap();

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
