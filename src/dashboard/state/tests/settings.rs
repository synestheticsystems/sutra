//! The settings: keeping free space only after a yes, a setting that cannot be saved, and the
//! private folders to watch, whose change makes the map afresh.

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
fn documents_alone_is_watched_until_the_user_chooses_otherwise() {
    let fixture = TempDir::new("state-private-folders-default");
    let (state, _inbox) = dashboard(&fixture);
    let switches: Vec<(String, bool)> = state
        .contents(now())
        .settings
        .private_folders
        .into_iter()
        .map(|switch| (switch.folder, switch.watched))
        .collect();
    assert_eq!(
        switches,
        [
            ("Desktop".to_string(), false),
            ("Documents".to_string(), true),
            ("Downloads".to_string(), false),
        ]
    );
}

#[test]
fn watching_a_private_folder_is_kept_for_the_next_run_and_the_map_is_made_afresh() {
    let fixture = TempDir::new("state-private-folders");
    let (mut state, _inbox) = dashboard(&fixture);
    state.show(Arc::new(snapshot_of(Vec::new())), now());
    let watch = |state: &mut State, folder: &str, watch| {
        let folder = folder.to_string();
        state.perform(Action::WatchPrivateFolder { folder, watch }, now());
    };
    let watched = |state: &State, name: &str| {
        let switches = state.contents(now()).settings.private_folders;
        switches
            .into_iter()
            .any(|switch| switch.folder == name && switch.watched)
    };

    watch(&mut state, "Documents", true);
    assert!(
        state.snapshot.is_some(),
        "watching a folder already watched changes nothing"
    );
    watch(&mut state, "Pictures", true);
    assert!(
        state.snapshot.is_some(),
        "nor does a folder that is not one"
    );

    watch(&mut state, "Desktop", true);
    assert!(watched(&state, "Desktop") && watched(&state, "Documents"));
    assert_eq!(
        state.contents(now()).list,
        List::Scanning,
        "the map is made afresh"
    );
    watch(&mut state, "Documents", false);
    assert!(!watched(&state, "Documents"));
    let saved = SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    assert_eq!(
        saved.watched_private_folders,
        [PrivateFolder::Desktop].into()
    );
    state.stop();
}
