//! A folder held open by its descriptor, so that a clean checks, locks and deletes the folder it
//! opened, never whatever its path leads to by then. Each name is looked up in the folder held
//! (`openat`, `fstatat`, `unlinkat`), never along a path again, and no link is followed: a folder
//! swapped for a link, at its own path or anywhere above, leads a clean nowhere new.

use std::ffi::{OsStr, OsString};
use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use nix::dir::{Dir, Type};
use nix::fcntl::{AtFlags, OFlag, openat};
use nix::libc;
use nix::sys::stat::{FileStat, Mode, SFlag, fstatat};
use nix::unistd::{UnlinkatFlags, unlinkat};

/// A folder opened as itself, not through a link.
#[derive(Debug)]
pub(super) struct OpenFolder(File);

/// Which folder or file a path led to: its device and inode, which no other file has while it
/// exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
}

/// One name in a folder, and what it names as the folder's listing tells it.
#[derive(Debug)]
pub(super) struct Entry {
    pub(super) name: OsString,
    pub(super) kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Folder,
    File,
    /// A link, a pipe, a socket or a device.
    Other,
}

/// What `OpenFolder::stat` reads of a name, its link not followed.
pub(super) struct Stat(FileStat);

impl OpenFolder {
    /// The folder at `path`. A link there, or anything else that is no folder, fails it (see
    /// `is_not_a_folder`).
    pub(super) fn at(path: &Path) -> io::Result<OpenFolder> {
        let folder = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        Ok(OpenFolder(folder))
    }

    /// The folder `name` in this one, opened as `at` opens one.
    pub(super) fn folder(&self, name: &OsStr) -> io::Result<OpenFolder> {
        self.open(name, OFlag::O_DIRECTORY).map(OpenFolder)
    }

    /// The file `name` in this one, opened to read. A link is never followed, and opening a pipe
    /// never waits for a writer.
    pub(super) fn file(&self, name: &OsStr) -> io::Result<File> {
        self.open(name, OFlag::O_NONBLOCK)
    }

    fn open(&self, name: &OsStr, flags: OFlag) -> io::Result<File> {
        let flags = flags | OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
        let fd = openat(Some(self.fd()), name, flags, Mode::empty())?;
        // SAFETY: `openat` just returned this descriptor, and nothing else owns it.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    /// Which folder this is.
    pub(super) fn identity(&self) -> io::Result<Identity> {
        Identity::of_file(&self.0)
    }

    /// What the folder holds, listed in full before any of it is deleted, as a folder read while
    /// entries go from it may skip some.
    pub(super) fn entries(&self) -> io::Result<Vec<Entry>> {
        // A listing moves through the folder on a descriptor of its own.
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC;
        let mut listing = Dir::openat(Some(self.fd()), ".", flags, Mode::empty())?;
        let mut entries = Vec::new();
        for entry in listing.iter() {
            let entry = entry?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let kind = match entry.file_type() {
                Some(Type::Directory) => Kind::Folder,
                Some(Type::File) => Kind::File,
                Some(_) => Kind::Other,
                // A volume that leaves the kind out of its listings, which APFS never does.
                None => match self.stat(name) {
                    Ok(stat) => stat.kind(),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                    Err(err) => return Err(err),
                },
            };
            entries.push(Entry {
                name: name.to_os_string(),
                kind,
            });
        }
        Ok(entries)
    }

    /// What `name` in this folder is.
    pub(super) fn stat(&self, name: &OsStr) -> io::Result<Stat> {
        let stat = fstatat(Some(self.fd()), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        Ok(Stat(stat))
    }

    /// Delete `name` in this folder, which is no folder: a file, or a link and never what it
    /// leads to.
    pub(super) fn remove_file(&self, name: &OsStr) -> io::Result<()> {
        Ok(unlinkat(Some(self.fd()), name, UnlinkatFlags::NoRemoveDir)?)
    }

    /// Delete the folder `name` in this folder, which must be empty.
    pub(super) fn remove_folder(&self, name: &OsStr) -> io::Result<()> {
        Ok(unlinkat(Some(self.fd()), name, UnlinkatFlags::RemoveDir)?)
    }

    fn fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl Identity {
    /// Which file `file`, open, is.
    pub(super) fn of_file(file: &File) -> io::Result<Identity> {
        file.metadata().map(|meta| Identity::of(&meta))
    }

    fn of(meta: &Metadata) -> Identity {
        Identity {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }
}

impl Stat {
    /// Which file the name names, as `Identity::of_file` tells it of the file open.
    pub(super) fn identity(&self) -> Identity {
        // As `std`'s `MetadataExt::dev` widens the device to read it.
        Identity {
            device: self.0.st_dev as u64,
            inode: self.0.st_ino,
        }
    }

    pub(super) fn kind(&self) -> Kind {
        let kind = SFlag::from_bits_truncate(self.0.st_mode) & SFlag::S_IFMT;
        if kind == SFlag::S_IFDIR {
            Kind::Folder
        } else if kind == SFlag::S_IFREG {
            Kind::File
        } else {
            Kind::Other
        }
    }

    /// How many names the file has.
    pub(super) fn links(&self) -> u64 {
        u64::from(self.0.st_nlink)
    }

    /// The bytes the file's blocks take on the volume, counted in 512-byte blocks as `std`'s
    /// `MetadataExt::blocks` counts them.
    pub(super) fn allocated_bytes(&self) -> u64 {
        self.0.st_blocks as u64 * 512
    }
}

/// Whether opening a folder failed because something other than a folder stands at its path: a
/// file, or a link, which macOS refuses with `ENOTDIR` and Linux with `ELOOP`.
pub(super) fn is_not_a_folder(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::NotADirectory || err.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn a_folder_opens_as_itself_and_a_link_or_a_file_at_its_path_does_not() {
        let fixture = TempDir::new("open-folder-at");
        let folder = fixture.dir("folder");
        let link = fixture.path("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        let file = fixture.write("file", "");

        let opened = OpenFolder::at(&folder).unwrap();

        assert_eq!(
            opened.identity().unwrap(),
            Identity::of(&std::fs::symlink_metadata(&folder).unwrap())
        );
        assert!(is_not_a_folder(&OpenFolder::at(&link).unwrap_err()));
        assert!(is_not_a_folder(&OpenFolder::at(&file).unwrap_err()));
    }

    #[test]
    fn names_are_found_and_deleted_in_the_folder_held_wherever_its_path_leads_now() {
        let fixture = TempDir::new("open-folder-relative");
        fixture.write("app/target/debug/lib.rlib", "built");
        fixture.write("app/target/CACHEDIR.TAG", "tag");
        let elsewhere = fixture.write("elsewhere/target/debug/lib.rlib", "precious");
        let target = OpenFolder::at(&fixture.path("app/target")).unwrap();
        std::fs::rename(fixture.path("app"), fixture.path("app.old")).unwrap();
        std::os::unix::fs::symlink(fixture.path("elsewhere"), fixture.path("app")).unwrap();

        let debug = target.folder(OsStr::new("debug")).unwrap();
        debug.remove_file(OsStr::new("lib.rlib")).unwrap();
        target.remove_folder(OsStr::new("debug")).unwrap();

        let mut names: Vec<_> = target.entries().unwrap();
        names.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name, "CACHEDIR.TAG");
        assert_eq!(names[0].kind, Kind::File);
        assert!(!fixture.path("app.old/target/debug").exists());
        assert!(elsewhere.exists(), "the link's folder is untouched");
    }

    #[test]
    fn a_file_is_one_identity_whether_its_name_is_read_or_the_file_opened() {
        let fixture = TempDir::new("open-folder-identity");
        let folder = fixture.dir("debug");
        fixture.write("debug/.cargo-lock", "");
        fixture.write("debug/.cargo-build-lock", "");
        let opened = OpenFolder::at(&folder).unwrap();
        let lock = OsStr::new(".cargo-lock");

        let by_name = opened.stat(lock).unwrap().identity();

        assert_eq!(
            Identity::of_file(&opened.file(lock).unwrap()).unwrap(),
            by_name
        );
        assert_ne!(
            opened
                .stat(OsStr::new(".cargo-build-lock"))
                .unwrap()
                .identity(),
            by_name
        );
    }

    #[test]
    fn a_link_in_a_folder_is_named_and_deleted_as_a_link() {
        let fixture = TempDir::new("open-folder-link-entry");
        let kept = fixture.write("elsewhere/kept", "precious");
        let folder = fixture.dir("target");
        std::os::unix::fs::symlink(fixture.path("elsewhere"), folder.join("linked")).unwrap();
        let opened = OpenFolder::at(&folder).unwrap();
        let linked = OsStr::new("linked");

        assert_eq!(opened.entries().unwrap()[0].kind, Kind::Other);
        assert_eq!(opened.stat(linked).unwrap().kind(), Kind::Other);
        assert!(is_not_a_folder(&opened.folder(linked).unwrap_err()));
        assert_eq!(
            opened.file(linked).unwrap_err().raw_os_error(),
            Some(libc::ELOOP),
            "never opened through"
        );
        opened.remove_file(linked).unwrap();

        assert!(!folder.join("linked").exists());
        assert!(kept.exists());
    }
}
