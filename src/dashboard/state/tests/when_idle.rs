//! The groups whose build folders Sutra cleans once idle: the time chosen from a group's line, kept
//! for the next run while the engine knows the group, and the cleans it makes at the tick.

use super::*;
use crate::dashboard::GroupKind;
use crate::dashboard::cleaning::CleanWhenIdle;
use crate::map::test_support::{cargo_target, measured, snapshot_of};
use crate::map::{GroupKey, Provenance};

const APP: &str = "github.com/me/app";

fn app() -> GroupKind {
    GroupKind::Upstream {
        remote: APP.to_string(),
    }
}

fn app_key() -> GroupKey {
    GroupKey::Upstream(APP.to_string())
}

/// A clone of `APP` at `path`, last active `days_ago`, holding `artifacts`.
fn clone_of_app(path: PathBuf, days_ago: u64, artifacts: Vec<crate::map::ArtifactDir>) -> Project {
    Project {
        provenance: Provenance {
            group: app_key(),
            made_from: Vec::new(),
        },
        last_activity: Some(now() - Duration::from_secs(days_ago * 24 * 60 * 60)),
        ..active_at(path, artifacts)
    }
}

fn choose(state: &mut State, group: GroupKind, days: Option<u32>) {
    state.perform(Action::CleanWhenIdle { group, days }, now());
}

#[test]
fn a_time_chosen_from_a_groups_line_is_kept_for_the_next_run_until_the_group_goes() {
    let fixture = TempDir::new("state-when-idle-kept");
    let (mut state, _inbox) = dashboard(&fixture);
    let saved = || {
        SavedSettings::load(Some(&settings_in(&fixture)))
            .unwrap()
            .clean_when_idle
    };
    let map = snapshot_of(vec![clone_of_app("/code/app".into(), 0, Vec::new())]);
    state.show(Arc::new(map.clone()), now());

    choose(&mut state, app(), Some(3));
    choose(&mut state, GroupKind::Docker, Some(3));
    choose(&mut state, app(), Some(5));

    let mut app_after_three_days = CleanWhenIdle::default();
    app_after_three_days.choose(&app_key(), Some(3), &map);
    assert_eq!(
        saved(),
        app_after_three_days,
        "Docker has none, and five days is not offered"
    );

    let site = active_at("/code/site".into(), Vec::new());
    state.show(Arc::new(snapshot_of(vec![site])), now());
    assert_eq!(
        saved().days_for(&app_key()),
        None,
        "forgotten with its group"
    );
}

#[test]
fn a_tick_cleans_the_idle_folders_of_a_group_chosen_to_be_cleaned_once_idle() {
    let fixture = TempDir::new("state-when-idle-tick");
    let (mut state, inbox) = dashboard(&fixture);
    fixture.package("app");
    let built = fixture.write("app/target/debug/deps/lib", [1; 64 * 1024]);
    let target = cargo_target(fixture.path("app/target"), measured(1 << 30));
    let idle = clone_of_app(fixture.path("app"), 9, vec![target]);
    state.show(Arc::new(snapshot_of(vec![idle])), now());

    choose(&mut state, app(), Some(3));
    assert!(built.exists(), "nothing goes before the tick");
    state.tick(Instant::now(), now());

    drive_until(&mut state, &inbox, "the idle folder's clean", |_| {
        !built.exists()
    });
}
