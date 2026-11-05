//! Disk use of directory trees (build folders and whole project folders), with hard links counted
//! once; where a tree's measurement stands; and a cheap stamp of a tree's top that tells whether it
//! may have changed.
//!
//! Each regular file contributes its allocated bytes divided by its link count. Rustc's incremental
//! cache hard-links most of the files in a `target/`, and every link stays inside that tree, so the
//! shares add up to the deduplicated total without remembering a single inode. The walk is one
//! rayon task per directory and keeps no per-file state, so memory stays flat on a 700k-file tree.

#[cfg(target_os = "macos")]
mod bulk;
#[cfg(not(target_os = "macos"))]
mod portable;

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rayon::prelude::*;

#[cfg(target_os = "macos")]
use bulk::list;
#[cfg(not(target_os = "macos"))]
use portable::list;

/// Disk use of one directory tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirUsage {
    /// Allocated bytes with hard links counted once (each link contributes `allocated / nlink`), so
    /// rustc's hard-linked incremental cache is not double-counted.
    pub bytes: u64,
    pub files: u64,
    /// Subdirectories that could not be listed and were skipped, so `bytes` is a lower bound when
    /// this is non-zero.
    pub unreadable_dirs: u64,
    pub measured_at: SystemTime,
    pub elapsed: Duration,
}

/// Where the size of a directory tree stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measurement {
    /// Found and not yet measured, with no measurement queued.
    Pending,
    /// The first measurement is queued or running.
    Measuring,
    /// The last measurement that finished. A directory measured again keeps this, with the old
    /// size, until the new measurement lands.
    Measured(DirUsage),
    /// The last measurement failed (for example, the directory turned unreadable). The next change
    /// to the directory measures it again.
    Failed(io::ErrorKind),
}

impl Measurement {
    /// The size, once one measurement has finished.
    pub fn usage(&self) -> Option<&DirUsage> {
        match self {
            Measurement::Measured(usage) => Some(usage),
            Measurement::Pending | Measurement::Measuring | Measurement::Failed(_) => None,
        }
    }

    /// Whether a measurement has finished, successfully or not.
    pub fn is_settled(&self) -> bool {
        matches!(self, Measurement::Measured(_) | Measurement::Failed(_))
    }
}

/// Whether a walk leaves out a directory, with everything in it.
type Pruned<'a> = &'a (dyn Fn(&Path) -> bool + Sync);

/// Measure one directory tree in parallel. Symlinks are never followed, the root included, so a
/// root that is missing, a file or a symlink is an error. A directory below it that cannot be
/// listed in full is counted in `unreadable_dirs`, and whatever it could not list is skipped.
pub fn measure(dir: &Path) -> io::Result<DirUsage> {
    measure_pruned(dir, &|_| false)
}

/// `measure`, leaving out every directory below `dir` for which `pruned` holds, with everything in
/// it. A project folder is sized this way without its build folders and the projects nested in
/// it, which are sized on their own.
pub fn measure_pruned(dir: &Path, pruned: Pruned) -> io::Result<DirUsage> {
    // Taken before the walk, so a change made while walking is never older than the measurement.
    let measured_at = SystemTime::now();
    let started = Instant::now();
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a directory", dir.display()),
        ));
    }
    let tally = tally_below(list(dir, pruned)?, pruned);
    Ok(DirUsage {
        bytes: tally.bytes,
        files: tally.files,
        unreadable_dirs: tally.unreadable_dirs,
        measured_at,
        elapsed: started.elapsed(),
    })
}

/// A cheap fingerprint of the top of a tree: the modification times of its folder and of each
/// entry directly in it. Creating, removing or renaming anything in that folder or in a folder
/// directly in it changes it, and so does writing a file directly in it; a change further down
/// does not, which keeps it to one listing. After lost file events it tells, without a walk,
/// whether a tree plausibly changed: equal stamps say only that its top did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TopStamp(u64);

/// The stamp of the top of the tree at `dir` now, leaving out the entries `pruned` holds, as a
/// walk with it would. An entry deleted since the folder was listed is left out too: the folder's
/// own time shows the deletion.
pub(crate) fn top_stamp(dir: &Path, pruned: &dyn Fn(&Path) -> bool) -> io::Result<TopStamp> {
    let mut hasher = DefaultHasher::new();
    fs::symlink_metadata(dir)?.modified()?.hash(&mut hasher);
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if pruned(&entry.path()) {
            continue;
        }
        match entry.metadata() {
            Ok(metadata) => entries.push((entry.file_name(), metadata.modified()?)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    // A listing comes in no set order.
    entries.sort();
    entries.hash(&mut hasher);
    Ok(TopStamp(hasher.finish()))
}

/// Running totals for one subtree, merged pairwise as the parallel walk returns.
#[derive(Debug, Default)]
struct Tally {
    bytes: u64,
    files: u64,
    unreadable_dirs: u64,
}

impl Tally {
    fn merge(self, other: Tally) -> Tally {
        Tally {
            bytes: self.bytes + other.bytes,
            files: self.files + other.files,
            unreadable_dirs: self.unreadable_dirs + other.unreadable_dirs,
        }
    }

    /// Count one regular file holding `share` of the disk.
    fn add_file(&mut self, share: u64) {
        self.bytes += share;
        self.files += 1;
    }
}

/// What one directory holds: its files added up, and the subfolders the walk goes on to.
#[derive(Debug, Default)]
struct Listing {
    tally: Tally,
    subdirs: Vec<PathBuf>,
}

/// `listing`'s files, then the trees of its subfolders, walked in parallel.
fn tally_below(listing: Listing, pruned: Pruned) -> Tally {
    let below = listing
        .subdirs
        .into_par_iter()
        .map(|subdir| match list(&subdir, pruned) {
            Ok(listing) => tally_below(listing, pruned),
            // Removed after its parent was listed.
            Err(err) if err.kind() == io::ErrorKind::NotFound => Tally::default(),
            Err(_) => Tally {
                unreadable_dirs: 1,
                ..Tally::default()
            },
        })
        .reduce(Tally::default, Tally::merge);
    listing.tally.merge(below)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;
    use std::path::PathBuf;

    /// APFS allocates whole 4 KiB blocks, so a file's allocated size exceeds its length by less
    /// than this.
    const BLOCK: u64 = 4096;

    /// Write `len` non-zero bytes (zeros could be stored sparse) at `rel`.
    fn file(fixture: &TempDir, rel: &str, len: usize) -> PathBuf {
        fixture.write(rel, vec![0xA5; len])
    }

    fn assert_within_block_rounding(usage: &DirUsage, lengths: &[u64]) {
        let logical: u64 = lengths.iter().sum();
        let rounded: u64 = lengths.iter().map(|len| len.div_ceil(BLOCK) * BLOCK).sum();
        assert!(
            (logical..=rounded).contains(&usage.bytes),
            "{} bytes, expected {logical}..={rounded}",
            usage.bytes
        );
    }

    #[test]
    fn nested_plain_files_sum_to_their_allocated_size() {
        let fixture = TempDir::new("disk-plain");
        file(&fixture, "one.bin", 1);
        file(&fixture, "a/five-thousand.bin", 5_000);
        file(&fixture, "a/b/hundred-k.bin", 100_000);
        let usage = measure(fixture.root()).unwrap();
        assert_eq!(usage.files, 3);
        assert_eq!(usage.unreadable_dirs, 0);
        assert_within_block_rounding(&usage, &[1, 5_000, 100_000]);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_with_two_hard_links_is_counted_once() {
        use std::os::unix::fs::MetadataExt;
        let fixture = TempDir::new("disk-hardlink");
        let original = file(&fixture, "a/original.bin", 100_000);
        fs::create_dir(fixture.path("b")).unwrap();
        fs::hard_link(&original, fixture.path("b/link.bin")).unwrap();
        let allocated = fs::metadata(&original).unwrap().blocks() * 512;
        let usage = measure(fixture.root()).unwrap();
        assert_eq!(usage.files, 2);
        assert_eq!(usage.bytes, allocated);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_to_a_large_file_and_a_directory_are_not_followed() {
        let outside = TempDir::new("disk-symlink-targets");
        let large = file(&outside, "dir/large.bin", 1 << 20);
        let fixture = TempDir::new("disk-symlinks");
        file(&fixture, "small.bin", 10);
        std::os::unix::fs::symlink(&large, fixture.path("to-file")).unwrap();
        std::os::unix::fs::symlink(large.parent().unwrap(), fixture.path("to-dir")).unwrap();
        let usage = measure(fixture.root()).unwrap();
        assert_eq!(usage.files, 1);
        assert_within_block_rounding(&usage, &[10]);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_subdirectory_is_skipped_and_counted() {
        let fixture = TempDir::new("disk-unreadable");
        file(&fixture, "readable/kept.bin", 5_000);
        file(&fixture, "locked/hidden.bin", 100_000);
        fixture.restrict(&fixture.path("locked"), 0o000);
        let usage = measure(fixture.root()).unwrap();
        assert_eq!(usage.files, 1);
        assert_eq!(usage.unreadable_dirs, 1);
        assert_within_block_rounding(&usage, &[5_000]);
    }

    #[test]
    fn a_missing_root_or_a_file_root_is_an_error() {
        let fixture = TempDir::new("disk-bad-root");
        let missing = measure(&fixture.path("missing")).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        let file = file(&fixture, "file.bin", 10);
        let not_dir = measure(&file).unwrap_err();
        assert_eq!(not_dir.kind(), io::ErrorKind::NotADirectory);
    }

    #[test]
    fn a_pruned_walk_leaves_out_the_pruned_folders_and_everything_in_them() {
        let fixture = TempDir::new("disk-pruned");
        file(&fixture, "src/kept.bin", 5_000);
        file(&fixture, "target/debug/big.bin", 100_000);
        file(&fixture, "nested/wt/src/other.bin", 100_000);
        let left_out = [fixture.path("target"), fixture.path("nested/wt")];
        let usage =
            measure_pruned(fixture.root(), &|dir| left_out.iter().any(|p| p == dir)).unwrap();
        assert_eq!(usage.files, 1);
        assert_within_block_rounding(&usage, &[5_000]);
    }

    #[test]
    fn a_directory_too_long_for_one_listing_call_is_counted_whole() {
        let fixture = TempDir::new("disk-long-listing");
        // Each entry takes over 200 bytes, so 1,000 of them outgrow one call's 64 KiB.
        let long_name = "n".repeat(200);
        for n in 0..1_000 {
            file(&fixture, &format!("{long_name}{n}"), 1);
        }
        let usage = measure(fixture.root()).unwrap();
        assert_eq!(usage.files, 1_000);
        assert_eq!(usage.unreadable_dirs, 0);
    }

    #[test]
    fn a_stamp_changes_with_the_top_of_the_tree() {
        let fixture = TempDir::new("disk-stamp-top");
        file(&fixture, "src/a/lib.rs", 10);
        let unpruned = |_: &Path| false;
        let before = top_stamp(fixture.root(), &unpruned).unwrap();
        assert_eq!(
            top_stamp(fixture.root(), &unpruned).unwrap(),
            before,
            "read twice"
        );

        file(&fixture, "README.md", 10);
        let after = top_stamp(fixture.root(), &unpruned).unwrap();
        assert_ne!(after, before, "a new file at the top");

        file(&fixture, "src/main.rs", 10);
        assert_ne!(
            top_stamp(fixture.root(), &unpruned).unwrap(),
            after,
            "a new file in a folder at the top"
        );
    }

    #[test]
    fn a_stamp_ignores_pruned_folders_and_changes_further_down() {
        let fixture = TempDir::new("disk-stamp-pruned");
        file(&fixture, "src/a/lib.rs", 10);
        fs::create_dir(fixture.path("target")).unwrap();
        let target = fixture.path("target");
        let pruned = |dir: &Path| dir == target;
        let before = top_stamp(fixture.root(), &pruned).unwrap();

        file(&fixture, "target/debug/app", 10);
        file(&fixture, "src/a/new.rs", 10);
        assert_eq!(
            top_stamp(fixture.root(), &pruned).unwrap(),
            before,
            "the pruned build folder and a folder two levels down changed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_root_is_an_error() {
        let fixture = TempDir::new("disk-symlink-root");
        file(&fixture, "target/large.bin", 100_000);
        let link = fixture.path("link");
        std::os::unix::fs::symlink(fixture.path("target"), &link).unwrap();
        let err = measure(&link).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotADirectory);
    }
}
