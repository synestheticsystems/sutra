//! The lines opened or folded shut again: the fold kept for the next run, kept while the engine
//! knows what the line stands for, card or not, and forgotten once it does not; and a line held in
//! its place while the pointer is over the list.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::dashboard::listing::fixtures::{linked_worktree, main_worktree};
use crate::dashboard::{GroupKind, LineFold};
use crate::map::test_support::snapshot_of;
use crate::map::{Ancestry, GroupKey};

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

/// The line of the repository whose git dir is `common_dir`, which has no remote.
fn repo(common_dir: &str) -> LineKey {
    let folder = Path::new(common_dir).parent().expect("a .git in a folder");
    LineKey::Group {
        kind: GroupKind::LocalRepository {
            folder: format::path_text(folder),
        },
    }
}

/// Click the line of `line`.
fn click_line(state: &mut State, line: LineKey) {
    state.perform(Action::ToggleFold { line }, now());
}

/// The lines the window shows, in order, each as `name: open` or `name: folded`.
fn lines(state: &State) -> Vec<String> {
    let List::Rows { rows } = state.contents(now()).list else {
        panic!("no rows");
    };
    let lines = rows.into_iter().filter_map(|row| match row {
        ListRow::Line { line, .. } => Some(match line.fold {
            LineFold::Open => format!("{}: open", line.name),
            LineFold::Folded { .. } => format!("{}: folded", line.name),
        }),
        _ => None,
    });
    lines.collect()
}

#[test]
fn a_line_opened_is_kept_open_for_the_next_run_and_a_click_folds_it_again() {
    let fixture = TempDir::new("state-fold-kept");
    let (mut state, _inbox) = dashboard(&fixture);
    let map = [repository(APP, "app", 1), repository(SITE, "site", 1)].concat();
    state.show(Arc::new(snapshot_of(map)), now());
    let saved = || SavedSettings::load(Some(&settings_in(&fixture))).unwrap();
    assert_eq!(
        lines(&state),
        ["app: folded", "site: folded"],
        "as they start"
    );

    click_line(&mut state, repo(APP));
    assert_eq!(lines(&state), ["app: open", "site: folded"]);
    assert!(!saved().folds.is_folded(&repo(APP)));

    click_line(&mut state, repo(APP));
    assert_eq!(lines(&state), ["app: folded", "site: folded"]);
    assert!(saved().folds.is_folded(&repo(APP)));
}

#[test]
fn a_fold_lasts_while_the_engine_knows_its_line_card_or_not_and_goes_once_it_does_not() {
    let fixture = TempDir::new("state-fold-forgotten");
    let (mut state, _inbox) = dashboard(&fixture);
    let both = [repository(APP, "app", 1), repository(SITE, "site", 1)].concat();
    state.show(Arc::new(snapshot_of(both)), now());
    click_line(&mut state, repo(APP));
    click_line(&mut state, repo(SITE));
    let saved = || SavedSettings::load(Some(&settings_in(&fixture))).unwrap();

    // App's folders wait to be sized for their cards: the engine still knows its group.
    let app_waiting = Snapshot {
        ancestry: Arc::new(Ancestry {
            groups: BTreeMap::from([
                (
                    GroupKey::LocalRepository(PathBuf::from("/code/app")),
                    BTreeSet::from([PathBuf::from("/code/app")]),
                ),
                (
                    GroupKey::LocalRepository(PathBuf::from("/code/site")),
                    BTreeSet::from([PathBuf::from("/code/site")]),
                ),
            ]),
            made_from: BTreeSet::new(),
        }),
        ..snapshot_of(repository(SITE, "site", 1).to_vec())
    };
    state.show(Arc::new(app_waiting), now());
    assert!(
        !saved().folds.is_folded(&repo(APP)),
        "app's opening is kept"
    );

    let app_gone = repository(SITE, "site", 1).to_vec();
    state.show(Arc::new(snapshot_of(app_gone)), now());
    assert!(
        saved().folds.is_folded(&repo(APP)),
        "forgotten, so folded as it starts"
    );
    assert!(!saved().folds.is_folded(&repo(SITE)), "site is still known");
}

#[test]
fn an_open_line_keeps_its_place_while_the_pointer_is_over_the_list() {
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
    assert_eq!(lines(&state), ["app: folded", "site: open"]);

    // Site's worktrees at work now, which puts its group first in the sort.
    state.show(map(0), now());
    assert_eq!(lines(&state), ["app: folded", "site: open"], "held");

    state.perform(Action::PointerLeftList, now());
    assert_eq!(lines(&state), ["site: open", "app: folded"], "re-sorted");
}
