//! The list, or what stands in its place: what the store edition needs before it can read
//! anything, the wait for the first map, or why nothing is listed. Otherwise the rows: a card per
//! project, or in the Activity sort a repository's group, its cards under a header with its name
//! and disk, then the agents working outside every project, and the row standing for the quiet
//! projects.

mod elsewhere;
mod repository;

use std::path::PathBuf;

use super::card::{self, CardContext, Naming, agents};
use crate::dashboard::listing::{FOCUS_RECENT, Folds, Listing, Row};
use crate::dashboard::{List, ListRow, format};
use crate::map::grant::GrantError;
use crate::map::{LARGE_FOLDER_BYTES, Snapshot};

/// The list, from `listing`, `None` until the first map arrives, and the filter field's text.
/// `roots` are the folders watched, empty while the store edition has none granted, and
/// `lost_grant` why the one granted on an earlier launch could not be reopened. `folds` says which
/// repositories' groups are folded shut.
pub(in crate::dashboard) fn list(
    listing: Option<&Listing>,
    filter: Option<&str>,
    roots: &[PathBuf],
    lost_grant: Option<&GrantError>,
    engine_stopped: bool,
    folds: &Folds,
    cards: &CardContext,
) -> List {
    if roots.is_empty() {
        return no_folder(lost_grant);
    }
    match listing {
        None if engine_stopped => List::Empty {
            message: "The scanner stopped before its first scan. Quit and relaunch Sutra."
                .to_string(),
        },
        None => List::Scanning,
        Some(listing) if listing.rows.is_empty() => List::Empty {
            message: nothing_shown(filter, roots, cards),
        },
        Some(listing) => List::Rows {
            rows: listing
                .rows
                .iter()
                .map(|row| list_row(row, listing.snapshot, folds, cards))
                .collect(),
        },
    }
}

fn list_row(row: &Row, map: &Snapshot, folds: &Folds, cards: &CardContext) -> ListRow {
    match row {
        Row::Project(project) => ListRow::Card {
            card: Box::new(card::card(project, Naming::Folder, map, cards)),
        },
        Row::Repository(group) => repository::row(group, map, folds, cards),
        Row::Elsewhere(working) => ListRow::Elsewhere {
            heading: "Elsewhere".to_string(),
            lines: elsewhere::lines(working, cards),
            waiting: agents::any_waiting(working.sessions.iter().copied()),
        },
        Row::QuietProjects { count, shown } => ListRow::QuietProjects {
            count: format::count(*count),
            open: *shown,
            label: how_many(*count),
            tooltip: what_makes_one_quiet(),
        },
    }
}

/// Why the list is empty: the filter admits no project, or the scan found nothing.
fn nothing_shown(filter: Option<&str>, roots: &[PathBuf], cards: &CardContext) -> String {
    let typed = filter.map(str::trim).filter(|text| !text.is_empty());
    match typed {
        Some(typed) => format!("No project matches \u{201c}{typed}\u{201d}."),
        None => format!(
            "No worktrees or dev environments found under {}.",
            format::roots(roots, cards.home)
        ),
    }
}

/// The store edition with no folder granted yet: why the home folder matters, and why the one
/// granted before could not be reopened.
fn no_folder(lost_grant: Option<&GrantError>) -> List {
    List::NoFolder {
        message:
            "Sutra finds your worktrees, dev environments and build folders there. Your agents \
                  keep their sessions in ~/.claude and ~/.codex, so a smaller folder hides them."
                .to_string(),
        lost: lost_grant
            .map(|lost| format!("The folder chosen before could not be reopened: {lost}.")),
    }
}

/// `764 quiet`, beside the chevron.
fn how_many(count: usize) -> String {
    format!("{count} quiet")
}

/// What makes a project quiet, with the focus thresholds spelled out so the reader can tell why a
/// project is behind the row.
fn what_makes_one_quiet() -> String {
    format!(
        "No agent session, no dev-runner environment, no activity in the last {} days, and less \
         than {} on disk",
        FOCUS_RECENT.as_secs() / 86_400,
        format::bytes(LARGE_FOLDER_BYTES),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::Arrangement;
    use crate::dashboard::listing::fixtures::{
        agent, linked_worktree, main_worktree, now, project, quiet, snapshot, subagent_in,
    };
    use crate::dashboard::{AgentLook, ElsewhereLine, RepositoryFold};
    use crate::map::AgentStatus;
    use crate::notifications::Notifier;

    const HOME: &str = "/Users/dev";

    /// What `list` gives for `map` as the user arranged it, under the root `/code`, the groups of
    /// the repositories in `folds` folded shut.
    fn listed_with(map: &Snapshot, arrangement: &Arrangement, folds: &Folds) -> List {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let cards = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new(HOME),
            now: now(),
        };
        let listing = Listing::new(map, arrangement, now());
        let roots = [PathBuf::from("/code")];
        list(
            Some(&listing),
            arrangement.filter(),
            &roots,
            None,
            false,
            folds,
            &cards,
        )
    }

    /// What `list` gives for `map` as the user arranged it, under the root `/code`, no group
    /// folded.
    fn listed(map: &Snapshot, arrangement: &Arrangement) -> List {
        listed_with(map, arrangement, &Folds::default())
    }

    #[test]
    fn the_rows_are_the_cards_the_agents_elsewhere_with_their_folders_and_the_quiet_row() {
        let map = Snapshot {
            unplaced_subagents: vec![subagent_in("/Users/dev/build")],
            ..snapshot(
                vec![project("recent", 1), quiet("old")],
                vec![
                    agent("/tmp/idle", AgentStatus::Idle),
                    agent("/Users/dev/scratch", AgentStatus::Waiting),
                ],
            )
        };
        let List::Rows { rows } = listed(&map, &Arrangement::default()) else {
            panic!("a map with projects lists them");
        };
        let [
            ListRow::Card { card },
            ListRow::Elsewhere {
                heading,
                lines,
                waiting,
            },
            ListRow::QuietProjects {
                count, open, label, ..
            },
        ] = rows.as_slice()
        else {
            panic!("a card, the agents elsewhere, the quiet row: {rows:?}");
        };
        assert_eq!(card.name, "recent");
        assert_eq!(heading, "Elsewhere");
        let looks: Vec<(Option<AgentLook>, &str)> = lines
            .iter()
            .map(|line| match line {
                ElsewhereLine::Session { badge, folder, .. } => (Some(badge.look), folder.as_str()),
                ElsewhereLine::Subagents { folder, .. } => (None, folder.as_str()),
            })
            .collect();
        assert_eq!(
            looks,
            [
                (Some(AgentLook::Still), "/tmp/idle"),
                (Some(AgentLook::Waiting), "~/scratch"),
                (None, "~/build"),
            ],
            "the sessions in the map's order, whatever their status, then the subagent's folder"
        );
        assert!(*waiting, "the row throbs while one waits");
        assert_eq!((*count, *open, label.as_str()), (1, false, "1 quiet"));
    }

    #[test]
    fn a_group_the_user_folded_shows_its_header_alone_and_the_others_their_cards() {
        let (app, site) = ("/code/app/.git", "/code/site/.git");
        let map = snapshot(
            vec![
                main_worktree(app, "main", project("app", 1)),
                linked_worktree(app, "fix", project("app-fix", 1)),
                main_worktree(site, "main", project("site", 1)),
                linked_worktree(site, "fix", project("site-fix", 1)),
            ],
            vec![],
        );
        let mut folds = Folds::default();
        folds.toggle(app, &map);
        let List::Rows { rows } = listed_with(&map, &Arrangement::default(), &folds) else {
            panic!("a map with projects lists them");
        };
        let folded: Vec<(&str, bool)> = rows
            .iter()
            .map(|row| match row {
                ListRow::Repository { name, fold, .. } => {
                    (name.as_str(), matches!(fold, RepositoryFold::Folded { .. }))
                }
                row => panic!("only the two groups: {row:?}"),
            })
            .collect();
        assert_eq!(folded, [("app", true), ("site", false)]);
    }

    #[test]
    fn an_empty_list_says_whether_the_filter_or_the_scan_found_nothing() {
        let empty = snapshot(vec![], vec![]);
        assert_eq!(
            listed(&empty, &Arrangement::default()),
            List::Empty {
                message: "No worktrees or dev environments found under /code.".to_string()
            }
        );
        let map = snapshot(vec![project("app", 1)], vec![]);
        let mut filtered = Arrangement::default();
        filtered.set_filter_text(" site ".to_string(), Some(&map), now());
        assert_eq!(
            listed(&map, &filtered),
            List::Empty {
                message: "No project matches \u{201c}site\u{201d}.".to_string()
            }
        );
    }

    #[test]
    fn before_the_first_map_the_list_waits_and_says_when_it_never_will() {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let cards = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new(HOME),
            now: now(),
        };
        let roots = [PathBuf::from("/code")];
        let none = Folds::default();
        assert_eq!(
            list(None, None, &roots, None, false, &none, &cards),
            List::Scanning
        );
        assert!(matches!(
            list(None, None, &roots, None, true, &none, &cards),
            List::Empty { message } if message.starts_with("The scanner stopped before its first")
        ));
        assert!(matches!(
            list(None, None, &[], None, false, &none, &cards),
            List::NoFolder { lost: None, .. }
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn with_no_folder_granted_the_list_says_why_and_why_the_last_one_was_lost() {
        let no_folder = no_folder(Some(&GrantError::AccessRefused));
        let List::NoFolder { message, lost } = no_folder else {
            panic!("the store edition with no folder");
        };
        assert!(message.starts_with("Sutra finds your worktrees, dev environments"));
        assert_eq!(
            lost.as_deref(),
            Some(
                "The folder chosen before could not be reopened: the sandbox refused access to \
                 the saved folder."
            )
        );
    }

    #[test]
    fn the_quiet_row_counts_its_projects_and_says_what_makes_one_quiet() {
        assert_eq!(how_many(764), "764 quiet");
        assert_eq!(
            what_makes_one_quiet(),
            "No agent session, no dev-runner environment, no activity in the last 7 days, and \
             less than 1.1 GB on disk"
        );
    }
}
