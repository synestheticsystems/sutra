//! The gauge at the end of the scope bar: how full the disk holding the home folder is. It turns
//! amber once a tenth of the disk is left and red once a twentieth is, which on a disk that builds
//! code is a few build folders away from full.

use crate::dashboard::disk_space::DiskSpace;
use crate::dashboard::{DiskGauge, GaugeLook, format};

/// The share of the disk left free below which the gauge turns amber.
const TIGHT_BELOW: f64 = 0.10;
/// The share left free below which it turns red.
const FULL_BELOW: f64 = 0.05;

/// The gauge of `space`, or `None` for a volume that says it holds nothing.
pub(super) fn gauge(space: DiskSpace) -> Option<DiskGauge> {
    if space.total == 0 {
        return None;
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
    Some(DiskGauge {
        used: f32::from(percent) / 100.0,
        label: format!("{percent}%"),
        tooltip: format!(
            "{} of {} in use, {} free",
            format::size(used, false),
            format::size(space.total, false),
            format::size(free, false)
        ),
        look,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    fn gauge_of(total: u64, free: u64) -> DiskGauge {
        gauge(DiskSpace { total, free }).expect("a disk that holds something")
    }

    #[test]
    fn the_gauge_fills_with_the_share_in_use_in_whole_percents_rounded_down() {
        let half = gauge_of(2_000 * GB, 1_000 * GB);
        assert_eq!((half.used, half.label.as_str()), (0.5, "50%"));
        assert_eq!(
            gauge_of(1_000 * GB, 1).label,
            "99%",
            "a byte free is not full"
        );
        assert_eq!(gauge_of(1_000 * GB, 0).label, "100%");
    }

    #[test]
    fn its_tooltip_says_how_much_is_in_use_of_how_much_and_what_is_free() {
        assert_eq!(
            gauge_of(2_000 * GB, 160 * GB).tooltip,
            "1.8 TB of 2 TB in use, 160 GB free"
        );
    }

    #[test]
    fn the_gauge_turns_amber_with_a_tenth_left_and_red_with_a_twentieth() {
        let look = |free_gb| gauge_of(1_000 * GB, free_gb * GB).look;
        assert_eq!(look(100), GaugeLook::Roomy);
        assert_eq!(look(99), GaugeLook::Tight);
        assert_eq!(look(50), GaugeLook::Tight);
        assert_eq!(look(49), GaugeLook::Full);
    }

    #[test]
    fn a_volume_that_says_it_holds_nothing_has_no_gauge() {
        assert_eq!(gauge(DiskSpace { total: 0, free: 0 }), None);
    }
}
