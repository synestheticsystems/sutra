//! The gauge of the disk: read at each tick of the clock, unavailable while the volume holding the
//! home folder cannot be read, and read again once it can.

use super::*;
use crate::dashboard::DiskGauge;

/// The gauge the window shows after a tick of the clock.
fn gauge_after_a_tick(state: &mut State) -> Option<DiskGauge> {
    state.tick(Instant::now(), now());
    state.contents(now()).toolbar.disk
}

#[test]
fn a_home_folder_whose_volume_cannot_be_read_shows_why_until_it_can_be_read_again() {
    let fixture = TempDir::new("state-gauge");
    let home = fixture.path("home");
    let (mut state, _inbox) = dashboard_at(&home, fixture.root(), None);
    assert_eq!(state.contents(now()).toolbar.disk, None, "not read yet");

    let unread = DiskGauge::Unavailable {
        tooltip: "Couldn't read the disk: entity not found".to_string(),
    };
    assert_eq!(gauge_after_a_tick(&mut state), Some(unread.clone()));
    assert_eq!(
        gauge_after_a_tick(&mut state),
        Some(unread),
        "the same failure again changes nothing"
    );

    std::fs::create_dir(&home).unwrap();
    assert!(matches!(
        gauge_after_a_tick(&mut state),
        Some(DiskGauge::Read { .. })
    ));
}
