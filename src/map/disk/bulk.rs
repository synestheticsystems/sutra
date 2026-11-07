//! Listing a directory on macOS with `getattrlistbulk`, which returns the type, modification time,
//! link count and allocated size of every entry along with its name. The alternative, `read_dir`
//! and an `lstat` of each file, has the kernel look up each file, and build it a vnode once a walk
//! has met more files than its cache holds (`kern.maxvnodes` is 263,168 on Daniel's M4 Max; his
//! home holds millions). On 613,000 of those files (2026-11-04) this took 3 to 4.5 CPU-seconds
//! where that took 6 to 12, in less wall time, and found the same bytes to the digit.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nix::libc;

use super::{Listing, Pruned};

/// From `<sys/attr.h>`, which the `libc` crate leaves out.
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
/// From `enum vtype` in `<sys/vnode.h>`.
const VREG: u32 = 1;
const VDIR: u32 = 2;

/// Room for several hundred entries per call.
const BUFFER_BYTES: usize = 64 * 1024;

thread_local! {
    /// One per pool thread, because allocating it per directory would cost more than the call.
    /// Each listing borrows it only until the listing ends, before the walk goes below.
    static BUFFER: RefCell<Vec<u8>> = RefCell::new(vec![0; BUFFER_BYTES]);
}

/// What the kernel packed for one entry. Each field is there only when the set it returns first
/// says so: a failed entry may hold nothing but its error.
#[derive(Debug, Default, PartialEq, Eq)]
struct Entry<'a> {
    error: u32,
    name: Option<&'a [u8]>,
    kind: Option<u32>,
    modified: Option<SystemTime>,
    links: Option<u32>,
    allocated: Option<u64>,
}

/// List `dir`, leaving out the subfolders `pruned` holds. Fails only when `dir` cannot be opened;
/// what it cannot read past that counts in `unreadable_dirs`. A symlink is never followed.
pub(super) fn list(dir: &Path, pruned: Pruned) -> io::Result<Listing> {
    let folder = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)?;
    let mut listing = Listing::default();
    let mut incomplete = false;
    let mut attributes = requested_attributes();
    BUFFER.with_borrow_mut(|buffer| {
        loop {
            // SAFETY: `folder` stays open for the call, `attributes` is a valid `attrlist`, and
            // the kernel writes at most `buffer.len()` bytes into `buffer`.
            let entries = unsafe {
                libc::getattrlistbulk(
                    folder.as_raw_fd(),
                    (&raw mut attributes).cast(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    0,
                )
            };
            match usize::try_from(entries) {
                Ok(0) => break,
                Ok(entries) => {
                    incomplete |= !add_entries(buffer, entries, dir, pruned, &mut listing)
                }
                Err(_) => {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    // Deleted while being listed, as a running build does: it holds nothing now.
                    incomplete |= err.kind() != io::ErrorKind::NotFound;
                    break;
                }
            }
        }
    });
    if incomplete {
        listing.tally.unreadable_dirs += 1;
    }
    Ok(listing)
}

fn requested_attributes() -> libc::attrlist {
    libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_NAME
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_MODTIME,
        volattr: 0,
        dirattr: 0,
        fileattr: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE,
        forkattr: 0,
    }
}

/// Add the first `entries` entries packed in `buffer` to `listing`. Returns whether every one of
/// them could be read.
fn add_entries(
    buffer: &[u8],
    entries: usize,
    dir: &Path,
    pruned: Pruned,
    listing: &mut Listing,
) -> bool {
    let mut complete = true;
    let mut rest = buffer;
    for _ in 0..entries {
        let Some(length) = rest
            .first_chunk()
            .map(|bytes| u32::from_ne_bytes(*bytes) as usize)
        else {
            return false;
        };
        let Some((packed, after)) = rest.split_at_checked(length) else {
            return false;
        };
        rest = after;
        complete &= read_entry(packed).is_some_and(|entry| add_entry(&entry, dir, pruned, listing));
    }
    complete
}

/// Returns whether the entry could be read. One deleted since the directory was listed, as a
/// running build does, holds nothing now and reads as nothing.
fn add_entry(entry: &Entry, dir: &Path, pruned: Pruned, listing: &mut Listing) -> bool {
    if entry.error != 0 {
        return entry.error == libc::ENOENT as u32;
    }
    match (entry.kind, entry.name, entry.links, entry.allocated) {
        (Some(VDIR), Some(name), ..) => {
            let subdir = dir.join(OsStr::from_bytes(name));
            if !pruned(&subdir) {
                listing.subdirs.push(subdir);
            }
            true
        }
        (Some(VREG), _, Some(links), Some(allocated)) => {
            // A file that was just listed has at least one link; `max` only keeps the division
            // total.
            let share = allocated / u64::from(links.max(1));
            listing.tally.add_file(share, entry.modified);
            true
        }
        // A file without the sizes it was asked for.
        (Some(VREG), ..) | (None, ..) | (Some(VDIR), None, ..) => false,
        // Symlinks and the like allocate no blocks on APFS, and a symlink is not followed.
        _ => true,
    }
}

/// `packed` is one whole entry, its length first. `None` when it is shorter than it says.
fn read_entry(packed: &[u8]) -> Option<Entry<'_>> {
    let mut fields = packed.get(size_of::<u32>()..)?;
    let common = next_u32(&mut fields)?;
    let _volume = next_u32(&mut fields)?;
    let _folder = next_u32(&mut fields)?;
    let file = next_u32(&mut fields)?;
    let _fork = next_u32(&mut fields)?;
    let mut entry = Entry::default();
    if common & ATTR_CMN_ERROR != 0 {
        entry.error = next_u32(&mut fields)?;
    }
    if common & libc::ATTR_CMN_NAME != 0 {
        // An `attrreference_t`: where the name lies from here, and its length with its NUL.
        let reference_at = packed.len() - fields.len();
        let offset = next_u32(&mut fields)? as usize;
        let length = next_u32(&mut fields)? as usize;
        let name = packed
            .get(reference_at + offset..)?
            .get(..length.checked_sub(1)?)?;
        entry.name = Some(name);
    }
    if common & libc::ATTR_CMN_OBJTYPE != 0 {
        entry.kind = Some(next_u32(&mut fields)?);
    }
    if common & libc::ATTR_CMN_MODTIME != 0 {
        // A `struct timespec`: whole seconds since the epoch, then nanoseconds.
        let seconds = i64::from_ne_bytes(next_chunk(&mut fields)?);
        let nanoseconds = i64::from_ne_bytes(next_chunk(&mut fields)?);
        entry.modified = time_of(seconds, nanoseconds);
    }
    if file & libc::ATTR_FILE_LINKCOUNT != 0 {
        entry.links = Some(next_u32(&mut fields)?);
    }
    if file & libc::ATTR_FILE_ALLOCSIZE != 0 {
        let (bytes, _) = fields.split_first_chunk()?;
        entry.allocated = Some(u64::from_ne_bytes(*bytes));
    }
    Some(entry)
}

fn next_u32(fields: &mut &[u8]) -> Option<u32> {
    next_chunk(fields).map(u32::from_ne_bytes)
}

fn next_chunk<const N: usize>(fields: &mut &[u8]) -> Option<[u8; N]> {
    let (bytes, rest) = fields.split_first_chunk()?;
    *fields = rest;
    Some(*bytes)
}

/// The moment a `struct timespec` names, or `None` for nanoseconds out of their range, which no
/// file system writes.
fn time_of(seconds: i64, nanoseconds: i64) -> Option<SystemTime> {
    let nanoseconds = Duration::from_nanos(
        u64::try_from(nanoseconds)
            .ok()
            .filter(|n| *n < 1_000_000_000)?,
    );
    let whole = Duration::from_secs(seconds.unsigned_abs());
    let at = if seconds >= 0 {
        UNIX_EPOCH.checked_add(whole)
    } else {
        UNIX_EPOCH.checked_sub(whole)
    };
    at?.checked_add(nanoseconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(value: u32) -> [u8; 4] {
        value.to_ne_bytes()
    }

    /// One packed entry holding only the fields `common` and `file` name.
    fn packed(common: u32, file: u32, fields: &[&[u8]]) -> Vec<u8> {
        let body: Vec<u8> = fields.concat();
        let length = 4 + 20 + body.len();
        let sets = [common, 0, 0, file, 0];
        let mut out = field(length as u32).to_vec();
        out.extend(sets.iter().flat_map(|set| field(*set)));
        out.extend(body);
        out
    }

    #[test]
    fn an_entry_reads_only_the_fields_its_returned_set_names() {
        let common =
            ATTR_CMN_ERROR | libc::ATTR_CMN_NAME | libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_MODTIME;
        let file = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE;
        // The name follows every fixed field, 40 bytes after its reference starts.
        let whole = packed(
            common,
            file,
            &[
                &field(0),
                &field(40),
                &field(4),
                &field(VREG),
                &1_762_400_000i64.to_ne_bytes(),
                &250_000_000i64.to_ne_bytes(),
                &field(2),
                &8192u64.to_ne_bytes(),
                b"abc\0",
            ],
        );
        assert_eq!(
            read_entry(&whole),
            Some(Entry {
                error: 0,
                name: Some(b"abc".as_slice()),
                kind: Some(VREG),
                modified: Some(UNIX_EPOCH + Duration::from_millis(1_762_400_000_250)),
                links: Some(2),
                allocated: Some(8192),
            })
        );

        let failed = packed(ATTR_CMN_ERROR, 0, &[&field(libc::EACCES as u32)]);
        assert_eq!(
            read_entry(&failed),
            Some(Entry {
                error: libc::EACCES as u32,
                ..Entry::default()
            })
        );
    }

    #[test]
    fn an_entry_shorter_than_it_says_cannot_be_read() {
        let truncated = packed(libc::ATTR_CMN_OBJTYPE, 0, &[]);
        assert_eq!(read_entry(&truncated), None);
        let name_past_the_end = packed(libc::ATTR_CMN_NAME, 0, &[&field(100), &field(4)]);
        assert_eq!(read_entry(&name_past_the_end), None);
    }

    #[test]
    fn a_file_with_no_time_or_one_out_of_range_still_counts_its_bytes() {
        let file = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE;
        let sizes: [&[u8]; 3] = [&field(VREG), &field(1), &4096u64.to_ne_bytes()];
        let undated = packed(libc::ATTR_CMN_OBJTYPE, file, &sizes);
        let past_a_second = 1_000_000_000i64.to_ne_bytes();
        let out_of_range = packed(
            libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_MODTIME,
            file,
            &[
                sizes[0],
                &0i64.to_ne_bytes(),
                &past_a_second,
                sizes[1],
                sizes[2],
            ],
        );

        for packed in [undated, out_of_range] {
            let entry = read_entry(&packed).expect("the entry reads");
            assert_eq!(entry.modified, None);
            let mut listing = Listing::default();
            assert!(add_entry(&entry, Path::new("/"), &|_| false, &mut listing));
            assert_eq!(listing.tally.bytes, 4096);
            assert_eq!(listing.tally.newest_write, None);
        }
    }
}
