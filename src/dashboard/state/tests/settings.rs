//! The settings: keeping free space only after a yes, a setting that cannot be saved, and the
//! private folders, whose change makes the map afresh.

use super::*;
use crate::map::test_support::snapshot_of;

#[test]
fn turning_on_keep_free_asks_first_and_a_yes_keeps_it_for_the_next_run() {
    let fixture = TempDir::new("state-keep-free");
    let (mut state, _inbox) = dashboard(&fixture);
    let keep_free = |state: &mut State, bytes| state.perform(Action::KeepFree { bytes }, now());
    let kept = |state: &State| state.contents(now()).settings.keep_free;
    let saved = || SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    const FLOOR: u64 = 100_000_000_000;

    keep_free(&mut state, Some(FLOOR));
    let question = state.contents(now()).dialog.expect("a question first");
    assert_eq!(question.title, "Keep 100 GB free?");
    assert!(!question.destructive, "its yes only changes a setting");
    assert_eq!(kept(&state), None, "not before the yes");
    answer(&mut state, &question, false);
    assert_eq!(kept(&state), None, "a no changes nothing");

    keep_free(&mut state, Some(FLOOR));
    let question = state.contents(now()).dialog.unwrap();
    answer(&mut state, &question, true);
    assert_eq!(kept(&state), Some(FLOOR));
    assert_eq!(saved().keep_free_bytes, Some(FLOOR));

    keep_free(&mut state, None);
    let asked = state.contents(now()).dialog;
    assert_eq!(asked, None, "turning it off asks nothing");
    assert_eq!(saved().keep_free_bytes, None);
}

#[test]
fn a_setting_that_cannot_be_saved_is_kept_for_this_run_and_said_to_be() {
    let fixture = TempDir::new("state-settings-unsaved");
    fixture.write("blocker", "a file where the folder would go");
    let (mut state, _inbox) = dashboard_at(
        fixture.root(),
        fixture.root(),
        Some(fixture.path("blocker/settings.json")),
    );

    state.perform(Action::KeepFree { bytes: None }, now());

    let notice = state.contents(now()).dialog.expect("a notice");
    assert_eq!(notice.title, "Sutra will forget this when it quits");
    assert_eq!(notice.action, None);
    answer(&mut state, &notice, false);
    assert_eq!(state.contents(now()).dialog, None);
}

#[test]
fn including_private_folders_is_kept_for_the_next_run_and_the_map_is_made_afresh() {
    let fixture = TempDir::new("state-private-folders");
    let (mut state, _inbox) = dashboard(&fixture);
    state.show(Arc::new(snapshot_of(Vec::new())), now());

    state.perform(Action::IncludePrivateFolders { include: false }, now());
    assert!(
        state.snapshot.is_some(),
        "the same choice again changes nothing"
    );

    state.perform(Action::IncludePrivateFolders { include: true }, now());
    let contents = state.contents(now());
    assert!(contents.settings.include_private_folders);
    assert_eq!(contents.list, List::Scanning, "the map is made afresh");
    let saved = SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    assert!(saved.include_private_folders);
    state.stop();
}
