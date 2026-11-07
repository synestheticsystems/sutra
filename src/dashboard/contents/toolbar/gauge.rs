//! The gauge at the end of the scope bar: how full the disk holding the home folder is. It darkens
//! once a tenth of the disk is left and turns red once a twentieth is, which on a disk that builds
//! code is a few build folders away from full; amber stays for an agent that wants the user. When
//! the disk cannot be read, the gauge says so, rather than leaving the bar's place empty.

use crate::dashboard::disk_space::{Volume, VolumeUnread};
use crate::dashboard::{DiskGauge, GaugeLook, format};

/// The share of the disk left free below which the gauge darkens.
const TIGHT_BELOW: f64 = 0.10;
/// The share left free below which it turns red.
const FULL_BELOW: f64 = 0.05;

/// The gauge of `reading`, the volume holding the home folder or why it could not be read, with
/// `floor` the free space Sutra keeps by cleaning by itself, if it keeps any.
pub(in crate::dashboard) fn gauge(
    reading: &Result<Volume, VolumeUnread>,
    floor: Option<u64>,
) -> DiskGauge {
    let volume = match reading {
        Ok(volume) => volume,
        Err(why) => return unavailable(&why.to_string()),
    };
    let space = volume.space;
    if space.total == 0 {
        return unavailable("it says it holds nothing");
    }
    let free = space.free.min(space.total);
    let used = space.total - free;
    // Whole percents rounded down, so the disk reads 100% only once nothing is free.
    let percent = u8::try_from(u128::from(used) * 100 / u128::from(space.total)).unwrap_or(100);
    let free_share = free as f64 / space.total as f64;
    let look = if free_share < FULL_BELOW {
        GaugeLook::Full
    } else if free_share < TIGHT_BELOW {
        GaugeLook::Tight
    } else {
        GaugeLook::Roomy
    };
    let bytes = format!(
        "{} of {} in use, {} free",
        format::size(used, false),
        format::size(space.total, false),
        format::size(free, false)
    );
    let named = match &volume.name {
        Some(name) => format!("{name}: {bytes}"),
        None => bytes,
    };
    let floor = floor.filter(|floor| *floor < space.total);
    DiskGauge::Read {
        used: f32::from(percent) / 100.0,
        label: format!("{percent}%"),
        tooltip: match floor {
            Some(floor) => format!("{named}\nKeeping {} free", format::size(floor, false)),
            None => named,
        },
        look,
        floor: floor.map(|floor| ((space.total - floor) as f64 / space.total as f64) as f32),
    }
}

/// The gauge of a disk that could not be read, for the reason `why`.
fn unavailable(why: &str) -> DiskGauge {
    DiskGauge::Unavailable {
        tooltip: format!("Couldn't read the disk: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::dashboard::disk_space::DiskSpace;

    const GB: u64 = 1_000_000_000;

    /// The gauge of a disk of `total` bytes with `free` of them free, called `Macintosh HD`, with
    /// no free space kept.
    fn gauge_of(total: u64, free: u64) -> DiskGauge {
        gauge(&Ok(volume(total, free)), None)
    }

    fn volume(total: u64, free: u64) -> Volume {
        Volume {
            name: Some("Macintosh HD".to_string()),
            space: DiskSpace { total, free },
        }
    }

    /// What a gauge that read the disk shows: its share, its label, its tooltip and its look.
    fn read(gauge: DiskGauge) -> (f32, String, String, GaugeLook, Option<f32>) {
        match gauge {
            DiskGauge::Read {
                used,
                label,
                tooltip,
                look,
                floor,
            } => (used, label, tooltip, look, floor),
            DiskGauge::Unavailable { tooltip } => panic!("unavailable: {tooltip}"),
        }
    }

    #[test]
    fn the_gauge_fills_with_the_share_in_use_in_whole_percents_rounded_down() {
        let (used, label, ..) = read(gauge_of(2_000 * GB, 1_000 * GB));
        assert_eq!((used, label.as_str()), (0.5, "50%"));
        let (_, label, ..) = read(gauge_of(1_000 * GB, 1));
        assert_eq!(label, "99%", "a byte free is not full");
        let (_, label, ..) = read(gauge_of(1_000 * GB, 0));
        assert_eq!(label, "100%");
    }

    #[test]
    fn its_tooltip_names_the_volume_then_says_how_much_is_in_use_of_how_much_and_what_is_free() {
        let (_, _, tooltip, ..) = read(gauge_of(2_000 * GB, 160 * GB));
        assert_eq!(tooltip, "Macintosh HD: 1.8 TB of 2 TB in use, 160 GB free");
        let unnamed = Volume {
            name: None,
            ..volume(2_000 * GB, 160 * GB)
        };
        let (_, _, tooltip, ..) = read(gauge(&Ok(unnamed), None));
        assert_eq!(tooltip, "1.8 TB of 2 TB in use, 160 GB free");
    }

    #[test]
    fn the_gauge_darkens_with_a_tenth_left_and_turns_red_with_a_twentieth() {
        let look = |free_gb| read(gauge_of(1_000 * GB, free_gb * GB)).3;
        assert_eq!(look(100), GaugeLook::Roomy);
        assert_eq!(look(99), GaugeLook::Tight);
        assert_eq!(look(50), GaugeLook::Tight);
        assert_eq!(look(49), GaugeLook::Full);
    }

    #[test]
    fn the_free_space_sutra_keeps_is_ticked_where_it_begins_and_named_in_the_tooltip() {
        let reading = Ok(volume(2_000 * GB, 160 * GB));
        let (_, _, tooltip, _, floor) = read(gauge(&reading, Some(50 * GB)));
        assert_eq!(floor, Some(0.975));
        assert_eq!(
            tooltip,
            "Macintosh HD: 1.8 TB of 2 TB in use, 160 GB free\nKeeping 50 GB free"
        );
        let (.., floor) = read(gauge(&reading, Some(2_000 * GB)));
        assert_eq!(floor, None, "more than the disk holds marks nowhere");
    }

    #[test]
    fn a_disk_that_cannot_be_read_says_why_in_the_bars_place() {
        let denied = gauge(
            &Err(VolumeUnread::Space(io::ErrorKind::PermissionDenied)),
            None,
        );
        assert_eq!(
            denied,
            DiskGauge::Unavailable {
                tooltip: "Couldn't read the disk: permission denied".to_string()
            }
        );
        assert_eq!(
            gauge_of(0, 0),
            DiskGauge::Unavailable {
                tooltip: "Couldn't read the disk: it says it holds nothing".to_string()
            }
        );
    }
}
