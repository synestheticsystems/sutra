//! Listing a directory where no call returns its metadata in bulk: `read_dir`, and an `lstat` of
//! each file.

use std::fs;
use std::io;
use std::path::Path;

use super::{Listing, Pruned};

/// List `dir`, leaving out the subfolders `pruned` holds. Fails only when `dir` cannot be opened;
/// what it cannot read past that counts in `unreadable_dirs`. Only regular files carry bytes: on
/// APFS directories and symlinks allocate no blocks.
pub(super) fn list(dir: &Path, pruned: Pruned) -> io::Result<Listing> {
    let mut listing = Listing::default();
    let mut incomplete = false;
    for entry in fs::read_dir(dir)? {
        // `file_type` comes from the directory record and `metadata` is an lstat; neither follows a
        // symlink, so a link to a large file elsewhere adds nothing.
        let outcome = entry.and_then(|entry| {
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                let subdir = entry.path();
                if !pruned(&subdir) {
                    listing.subdirs.push(subdir);
                }
            } else if file_type.is_file() {
                let meta = entry.metadata()?;
                listing
                    .tally
                    .add_file(file_share(&meta), meta.modified().ok());
            }
            Ok(())
        });
        match outcome {
            Ok(()) => {}
            // Deleted since the directory was listed, as a running build does: it holds nothing now.
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(_) => incomplete = true,
        }
    }
    if incomplete {
        listing.tally.unreadable_dirs += 1;
    }
    Ok(listing)
}

/// A regular file's share of the disk: its allocated bytes over its link count, so the links of
/// one file inside a tree add up to its size once. Deleting every link frees exactly the shares.
#[cfg(unix)]
fn file_share(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    // `blocks` counts 512-byte units whatever the volume's block size. A path that was just
    // stat'ed has at least one link; `max` only keeps the division total.
    meta.blocks() * 512 / meta.nlink().max(1)
}

/// Without inode data, logical length is the closest figure and hard links count in full.
#[cfg(not(unix))]
fn file_share(meta: &fs::Metadata) -> u64 {
    meta.len()
}
