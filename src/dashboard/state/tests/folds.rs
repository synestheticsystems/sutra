//! The groups folded shut or opened from their lines: the fold kept for the next run, forgotten
//! once a live map no longer lists the group's parent, and a folded group held in its place while
//! the pointer is over the list.

use super::*;
use crate::dashboard::listing::fixtures::{
    linked_worktree, main_worktree, running_as, scratch_folder,
};
use crate::dashboard::{GroupFold, Parent};
use crate::map::test_support::snapshot_of;

const APP: &str = "/code/app/.git";
const SITE: &str = "/code/site/.git";

/// The project at `/code/<name>`, last active `minutes_ago` by the clock the dashboard reads.
fn active(name: &str, minutes_ago: u64) -> Project {
    Project {
        last_activity: Some(now() - Duration::from_secs(minutes_ago * 60)),
        ..project_at(PathBuf::from("/code").join(name))
    }
}

/// The main and a linked worktree of the repository whose git dir is `common_dir`, named after
/// `name`, both last active `minutes_ago`.
fn repository(common_dir: &str, name: &str, minutes_ago: u64) -> [Project; 2] {
    [
        main_worktree(common_dir, "main", active(name, minutes_ago)),
        linked_worktree(
            common_dir,
            "fix",
            active(&format!("{name}-fix"), minutes_ago),
        ),
    ]
}

/// The repository whose git dir is `common_dir`, as its line names it.
fn repo(common_dir: &str) -> Parent {
    Parent::Repository {
        common_dir: common_dir.to_string(),
    }
}

/// Click the line of the group of `parent`.
fn click_line(state: &mut State, parent: Parent) {
    state.perform(Action::ToggleFold { parent }, now());
}

/// The groups the window shows, in order, each as `name: open` or `name: folded`.
fn groups(state: &State) -> Vec<String> {
    let List::Rows { rows } = state.contents(now()).list else {
        panic!("no rows");
    };
    let groups = rows.into_iter().filter_map(|row| match row {
        ListRow::Group { line, fold } => Some(match *fold {
            GroupFold::Open { .. } => format!("{}: open", line.name),
            GroupFold::Folded { .. } => format!("{}: folded", line.name),
        }),
        _ => None,
    });
    groups.collect()
}

#[test]
fn a_group_folded_from_its_line_is_kept_folded_for_the_next_run_and_a_click_opens_it() {
    let fixture = TempDir::new("state-fold-kept");
    let (mut state, _inbox) = dashboard(&fixture);
    let map = [repository(APP, "app", 1), repository(SITE, "site", 1)].concat();
    state.show(Arc::new(snapshot_of(map)), now());
    let saved = || SavedSettings::load(Some(&settings_in(&fixture))).unwrap();

    click_line(&mut state, repo(APP));
    assert_eq!(groups(&state), ["app: folded", "site: open"]);
    assert!(saved().folds.is_folded(&repo(APP)));

    click_line(&mut state, repo(APP));
    assert_eq!(groups(&state), ["app: open", "site: open"]);
    assert!(!saved().folds.is_folded(&repo(APP)));
}

#[test]
fn a_sessions_scratch_starts_folded_and_an_opened_one_is_kept_open_for_the_next_run() {
    let fixture = TempDir::new("state-fold-scratch");
    let (mut state, _inbox) = dashboard(&fixture);
    let scratch = |folder| {
        let large = Project {
            own_files: crate::map::test_support::measured(crate::map::LARGE_FOLDER_BYTES),
            ..active("", 1)
        };
        running_as("Foundry", scratch_folder("e26ca359", folder, large))
    };
    let map = vec![scratch("r567"), scratch("r593")];
    state.show(Arc::new(snapshot_of(map)), now());
    assert_eq!(groups(&state), ["Foundry: folded"]);

    let session = Parent::Session {
        folder: "/private/tmp/claude-501/-code-app/e26ca359".to_string(),
    };
    click_line(&mut state, session.clone());

    assert_eq!(groups(&state), ["Foundry: open"]);
    let saved = SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    assert!(!saved.folds.is_folded(&session));
}

#[test]
fn a_fold_is_forgotten_and_the_settings_saved_once_a_live_map_no_longer_lists_its_parent() {
    let fixture = TempDir::new("state-fold-forgotten");
    let (mut state, _inbox) = dashboard(&fixture);
    let both = [repository(APP, "app", 1), repository(SITE, "site", 1)].concat();
    state.show(Arc::new(snapshot_of(both)), now());
    click_line(&mut state, repo(APP));
    click_line(&mut state, repo(SITE));

    let app_gone = repository(SITE, "site", 1).to_vec();
    state.show(Arc::new(snapshot_of(app_gone)), now());

    let saved = SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    assert!(!saved.folds.is_folded(&repo(APP)));
    assert!(saved.folds.is_folded(&repo(SITE)), "site is still listed");
    let app_back = [repository(APP, "app", 1), repository(SITE, "site", 1)].concat();
    state.show(Arc::new(snapshot_of(app_back)), now());
    assert_eq!(
        groups(&state),
        ["app: open", "site: folded"],
        "app returns open"
    );
}

#[test]
fn a_folded_group_keeps_its_place_while_the_pointer_is_over_the_list() {
    let fixture = TempDir::new("state-fold-held");
    let (mut state, _inbox) = dashboard(&fixture);
    let map = |site_minutes_ago| {
        let worktrees = [
            repository(APP, "app", 10),
            repository(SITE, "site", site_minutes_ago),
        ];
        Arc::new(snapshot_of(worktrees.concat()))
    };
    state.show(map(30), now());
    state.perform(Action::PointerEnteredList, now());
    click_line(&mut state, repo(SITE));
    assert_eq!(groups(&state), ["app: open", "site: folded"]);

    // Site's worktrees at work now, which puts its group first in the sort.
    state.show(map(0), now());
    assert_eq!(groups(&state), ["app: open", "site: folded"], "held");

    state.perform(Action::PointerLeftList, now());
    assert_eq!(groups(&state), ["site: folded", "app: open"], "re-sorted");
}
