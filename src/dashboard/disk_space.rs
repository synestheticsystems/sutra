//! How much the disk holding the home folder holds and has free, as the volume counts them, and
//! what macOS calls it. Finder also counts what macOS could purge if it had to, and that is not
//! there for a build to use until then. The scope bar's gauge shows it, and cleaning by itself
//! keeps a floor of it free.

use std::fmt;
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

/// The volume holding a folder, as the scope bar's gauge shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Volume {
    /// What macOS calls it, as Finder shows it: `Macintosh HD` for the home folder's, whose data
    /// lives on a volume of its own that Finder shows as part of it. `None` when macOS gives it no
    /// name.
    pub(super) name: Option<String>,
    pub(super) space: DiskSpace,
}

/// Why the volume holding a folder could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VolumeUnread {
    /// Its size and free space, for this reason.
    Space(io::ErrorKind),
    /// Its name, as macOS said why.
    Name(String),
}

impl fmt::Display for VolumeUnread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VolumeUnread::Space(kind) => write!(f, "{kind}"),
            VolumeUnread::Name(why) => f.write_str(why),
        }
    }
}

impl Volume {
    /// The volume holding `path`.
    pub(super) fn of(path: &Path) -> Result<Volume, VolumeUnread> {
        let space = DiskSpace::of(path).map_err(|err| VolumeUnread::Space(err.kind()))?;
        let name = volume_name(path).map_err(VolumeUnread::Name)?;
        Ok(Volume { name, space })
    }
}

/// What macOS calls the volume holding `path`, as Finder shows it, or why it would not say.
#[cfg(target_os = "macos")]
fn volume_name(path: &Path) -> Result<Option<String>, String> {
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::AnyObject;
    use objc2::{ClassType, msg_send};
    use objc2_foundation::{NSString, NSURL, NSURLVolumeLocalizedNameKey};

    // The pool releases what Foundation hands back on the dashboard's thread, which has none.
    autoreleasepool(|_| {
        let url = unsafe { NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())) };
        let mut value: Option<Retained<AnyObject>> = None;
        unsafe { url.getResourceValue_forKey_error(&mut value, NSURLVolumeLocalizedNameKey) }
            .map_err(|err| err.localizedDescription().to_string())?;
        let Some(value) = value else {
            return Ok(None);
        };
        let is_text: bool = unsafe { msg_send![&*value, isKindOfClass: NSString::class()] };
        // Foundation documents the name as a string; anything else is no name to show.
        if !is_text {
            return Ok(None);
        }
        // SAFETY: the value was just found to be a string.
        let name = unsafe { Retained::cast::<NSString>(value) };
        Ok(Some(name.to_string()))
    })
}

/// Outside macOS nothing names a volume as Finder does.
#[cfg(not(target_os = "macos"))]
fn volume_name(_: &Path) -> Result<Option<String>, String> {
    Ok(None)
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
        assert_eq!(
            Volume::of(&fixture.path("gone")),
            Err(VolumeUnread::Space(io::ErrorKind::NotFound))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_volume_holding_a_folder_goes_by_the_name_finder_shows_and_has_its_space() {
        let fixture = TempDir::new("disk-space-volume");
        let volume = Volume::of(fixture.root()).unwrap();
        assert!(volume.space.total > 0);
        let root = Volume::of(Path::new("/")).unwrap();
        assert!(
            volume.name.is_some() && volume.name == root.name,
            "the temporary folders live on the startup disk, which Finder names: {:?}",
            volume.name
        );
    }
}
