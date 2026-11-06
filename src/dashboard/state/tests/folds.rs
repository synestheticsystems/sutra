//! The repositories' groups folded shut from their headers: the fold kept for the next run,
//! forgotten once a live map no longer lists the repository, and a folded group held in its place
//! while the pointer is over the list.

use super::*;
use crate::dashboard::RepositoryFold;
use crate::dashboard::listing::fixtures::{linked_worktree, main_worktree};
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

/// Click the header of the group of the repository whose git dir is `common_dir`.
fn click_header(state: &mut State, common_dir: &str) {
    let common_dir = common_dir.to_string();
    state.perform(Action::ToggleRepositoryFold { common_dir }, now());
}

/// The groups the window shows, in order, each as `name: open` or `name: folded`.
fn groups(state: &State) -> Vec<String> {
    let List::Rows { rows } = state.contents(now()).list else {
        panic!("no rows");
    };
    let groups = rows.into_iter().filter_map(|row| match row {
        ListRow::Repository { name, fold, .. } => Some(match fold {
            RepositoryFold::Open { .. } => format!("{name}: open"),
            RepositoryFold::Folded { .. } => format!("{name}: folded"),
        }),
        _ => None,
    });
    groups.collect()
}

#[test]
fn a_group_folded_from_its_header_is_kept_folded_for_the_next_run_and_a_click_opens_it() {
    let fixture = TempDir::new("state-fold-kept");
    let (mut state, _inbox) = dashboard(&fixture);
    let map = [repository(APP, "app", 1), repository(SITE, "site", 1)].concat();
    state.show(Arc::new(snapshot_of(map)), now());
    let saved = || SavedSettings::load(Some(&settings_in(&fixture))).unwrap();

    click_header(&mut state, APP);
    assert_eq!(groups(&state), ["app: folded", "site: open"]);
    assert!(saved().folded_repositories.is_folded(APP));

    click_header(&mut state, APP);
    assert_eq!(groups(&state), ["app: open", "site: open"]);
    assert!(!saved().folded_repositories.is_folded(APP));
}

#[test]
fn a_fold_is_forgotten_and_the_settings_saved_once_a_live_map_no_longer_lists_its_repository() {
    let fixture = TempDir::new("state-fold-forgotten");
    let (mut state, _inbox) = dashboard(&fixture);
    let both = [repository(APP, "app", 1), repository(SITE, "site", 1)].concat();
    state.show(Arc::new(snapshot_of(both)), now());
    click_header(&mut state, APP);
    click_header(&mut state, SITE);

    let app_gone = repository(SITE, "site", 1).to_vec();
    state.show(Arc::new(snapshot_of(app_gone)), now());

    let saved = SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    assert!(!saved.folded_repositories.is_folded(APP));
    assert!(
        saved.folded_repositories.is_folded(SITE),
        "site is still listed"
    );
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
    click_header(&mut state, SITE);
    assert_eq!(groups(&state), ["app: open", "site: folded"]);

    // Site's worktrees at work now, which puts its group first in the sort.
    state.show(map(0), now());
    assert_eq!(groups(&state), ["app: open", "site: folded"], "held");

    state.perform(Action::PointerLeftList, now());
    assert_eq!(groups(&state), ["site: folded", "app: open"], "re-sorted");
}
