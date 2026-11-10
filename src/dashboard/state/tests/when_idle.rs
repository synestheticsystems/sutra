//! The groups whose build folders Sutra cleans once idle: the time chosen from a group's line, kept
//! for the next run while the engine knows the group, and the cleans it makes at the tick.

use super::*;
use crate::dashboard::GroupKind;
use crate::dashboard::cleaning::{CleanWhenIdle, IdleTime};
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

fn choose(state: &mut State, group: GroupKind, idle: Option<IdleTime>) {
    state.perform(Action::CleanWhenIdle { group, idle }, now());
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

    choose(&mut state, app(), Some(IdleTime::ThreeDays));
    choose(&mut state, GroupKind::Docker, Some(IdleTime::ThreeDays));

    let mut app_after_three_days = CleanWhenIdle::default();
    app_after_three_days.choose(&app_key(), Some(IdleTime::ThreeDays), &map);
    assert_eq!(saved(), app_after_three_days, "Docker has none");

    let site = active_at("/code/site".into(), Vec::new());
    state.show(Arc::new(snapshot_of(vec![site])), now());
    assert_eq!(
        saved().idle_time_of(&app_key()),
        None,
        "forgotten with its group"
    );
}

#[test]
fn a_groups_time_and_fold_follow_its_checkouts_when_their_remote_gets_a_new_address() {
    let fixture = TempDir::new("state-when-idle-moved");
    let (mut state, _inbox) = dashboard(&fixture);
    let saved = || SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    let checkouts = |group: &GroupKey| {
        let clones = [
            clone_of_app("/code/app".into(), 0, Vec::new()),
            clone_of_app("/code/wt/app-fix".into(), 0, Vec::new()),
        ];
        clones.map(|clone| Project {
            provenance: Provenance {
                group: group.clone(),
                made_from: Vec::new(),
            },
            ..clone
        })
    };
    state.show(Arc::new(snapshot_of(checkouts(&app_key()).to_vec())), now());
    choose(&mut state, app(), Some(IdleTime::ThreeDays));
    let line = |group: &GroupKey| LineKey::group(group);
    state.perform(
        Action::ToggleFold {
            line: line(&app_key()),
        },
        now(),
    );

    let renamed = GroupKey::Upstream("github.com/me/app-renamed".to_string());
    state.show(Arc::new(snapshot_of(checkouts(&renamed).to_vec())), now());

    assert_eq!(
        saved().clean_when_idle.idle_time_of(&renamed),
        Some(IdleTime::ThreeDays)
    );
    assert!(!saved().folds.is_folded(&line(&renamed)), "still open");
    assert_eq!(saved().clean_when_idle.idle_time_of(&app_key()), None);
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

    choose(&mut state, app(), Some(IdleTime::ThreeDays));
    assert!(built.exists(), "nothing goes before the tick");
    state.tick(Instant::now(), now());

    drive_until(&mut state, &inbox, "the idle folder's clean", |_| {
        !built.exists()
    });
}
