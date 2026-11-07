//! How much the disk holding the home folder holds and has free, as the volume counts them. Finder
//! also counts what macOS could purge if it had to, and that is not there for a build to use until
//! then. The scope bar's gauge shows it, and cleaning by itself keeps a floor of it free.

use std::io;
use std::path::Path;

use nix::sys::statvfs::statvfs;

/// A volume's size and the part of it free for anything to use now, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DiskSpace {
    pub(super) total: u64,
    pub(super) free: u64,
}

impl DiskSpace {
    /// The space of the volume holding `path`.
    pub(super) fn of(path: &Path) -> io::Result<DiskSpace> {
        let volume = statvfs(path).map_err(io::Error::from)?;
        let fragment = volume.fragment_size();
        Ok(DiskSpace {
            total: u64::from(volume.blocks()) * fragment,
            free: u64::from(volume.blocks_available()) * fragment,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn a_volume_has_room_and_no_more_free_than_it_holds() {
        let fixture = TempDir::new("disk-space");
        let space = DiskSpace::of(fixture.root()).unwrap();
        assert!(space.total > 0);
        assert!(space.free <= space.total);
    }

    #[test]
    fn a_path_that_is_not_there_has_no_volume_to_read() {
        let fixture = TempDir::new("disk-space-missing");
        assert!(DiskSpace::of(&fixture.path("gone")).is_err());
    }
}
