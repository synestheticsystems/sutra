//! The list, or what stands in its place: what the store edition needs before it can read
//! anything, the wait for the first map, or why nothing is listed. Otherwise the rows: a card per
//! project, or in the Repository sort each repository's cards under a header with its name and
//! disk, then the agents working outside every project, and the row standing for the quiet
//! projects.

use std::path::PathBuf;

use super::card::{self, CardContext, Naming, agents};
use crate::dashboard::listing::{Elsewhere, FOCUS_LARGE_BYTES, FOCUS_RECENT, Listing, Row};
use crate::dashboard::{ElsewhereAgent, List, ListRow, format};
use crate::map::grant::GrantError;
use crate::map::{FolderSize, Project, Snapshot};

/// The list, from `listing`, `None` until the first map arrives, and the filter field's text.
/// `roots` are the folders watched, empty while the store edition has none granted, and
/// `lost_grant` why the one granted on an earlier launch could not be reopened.
pub(in crate::dashboard) fn list(
    listing: Option<&Listing>,
    filter: Option<&str>,
    roots: &[PathBuf],
    lost_grant: Option<&GrantError>,
    engine_stopped: bool,
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
                .map(|row| list_row(row, listing.snapshot, cards))
                .collect(),
        },
    }
}

fn list_row(row: &Row, map: &Snapshot, cards: &CardContext) -> ListRow {
    match row {
        Row::Project(project) => ListRow::Card {
            card: Box::new(card::card(project, Naming::Folder, map, cards)),
        },
        Row::Repository(group) => ListRow::Repository {
            common_dir: format::path_text(group.common_dir),
            name: group.name.clone(),
            tooltip: format::tilde(group.folder, cards.home),
            total: total_of(&group.worktrees, cards),
            cards: group
                .cards
                .iter()
                .map(|project| card::card(project, Naming::Branch, map, cards))
                .collect(),
        },
        Row::Elsewhere(working) => ListRow::Elsewhere {
            heading: "Elsewhere".to_string(),
            agents: elsewhere(working, cards),
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

/// What `worktrees` take on disk between them, as their trays write it: the totals the trays show,
/// added up, at least that when part of one could not be read. A worktree still being sized adds
/// nothing, and the sum says nothing of it; `None` while none is sized.
fn total_of(worktrees: &[&Project], cards: &CardContext) -> Option<String> {
    let total = worktrees
        .iter()
        .filter_map(|worktree| card::tray_total(worktree, cards))
        .reduce(|total, size| FolderSize {
            bytes: total.bytes + size.bytes,
            partial: total.partial || size.partial,
        })?;
    Some(format::size(total.bytes, total.partial))
}

/// Agents whose working folder lies in no project, each with that folder: the sessions, then the
/// subagents at work away from theirs, each in the order the map lists them, which holds whatever
/// their status.
fn elsewhere(working: &Elsewhere, cards: &CardContext) -> Vec<ElsewhereAgent> {
    let sessions = working.sessions.iter().map(|agent| ElsewhereAgent {
        lozenge: agents::lozenge(agent, cards.now),
        folder: format::tilde(&agent.cwd, cards.home),
    });
    let subagents = working.subagents.iter().map(|away| ElsewhereAgent {
        lozenge: agents::subagent_lozenge(away, cards.now),
        folder: format::tilde(&away.subagent.cwd, cards.home),
    });
    sessions.chain(subagents).collect()
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
        format::bytes(FOCUS_LARGE_BYTES),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::dashboard::cleaning::Cleaning;
    use crate::dashboard::cleaning::tests::{cleaning, deleted};
    use crate::dashboard::listing::Arrangement;
    use crate::dashboard::listing::fixtures::{
        agent, away_subagent, linked_worktree, main_worktree, now, project, quiet, snapshot,
    };
    use crate::dashboard::{AgentLook, SortOrder};
    use crate::map::test_support::{cargo_target, measured, project_at, usage};
    use crate::map::{AgentStatus, DirUsage, Measurement};
    use crate::notifications::Notifier;

    const HOME: &str = "/Users/dev";

    /// What the cards read, with `cleaning` at work.
    fn context<'a>(cleaning: &'a Cleaning, notifier: &'a Notifier) -> CardContext<'a> {
        CardContext {
            notifier,
            cleaning,
            home: Path::new(HOME),
            now: now(),
        }
    }

    /// What `list` gives for `map` as the user arranged it, under the root `/code`.
    fn listed(map: &Snapshot, arrangement: &Arrangement) -> List {
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
            &cards,
        )
    }

    #[test]
    fn the_rows_are_the_cards_the_agents_elsewhere_with_their_folders_and_the_quiet_row() {
        let map = Snapshot {
            unplaced_subagents: vec![away_subagent("/Users/dev/build")],
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
                agents,
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
        let folders: Vec<&str> = agents.iter().map(|agent| agent.folder.as_str()).collect();
        assert_eq!(
            folders,
            ["/tmp/idle", "~/scratch", "~/build"],
            "the sessions in the map's order, whatever their status, then the subagent"
        );
        assert_eq!(agents[1].lozenge.look, AgentLook::Waiting);
        let outlined: Vec<bool> = agents.iter().map(|agent| agent.lozenge.outlined).collect();
        assert_eq!(outlined, [false, false, true]);
        assert!(*waiting, "the row throbs while one waits");
        assert_eq!((*count, *open, label.as_str()), (1, false, "1 quiet"));
    }

    #[test]
    fn a_repositorys_group_is_its_name_its_disk_and_its_cards_going_by_their_branches() {
        let app = "/code/app/.git";
        let sized = |bytes, project| Project {
            own_files: measured(bytes),
            ..project
        };
        let map = snapshot(
            vec![
                sized(2_000_000_000, main_worktree(app, "main", project("app", 1))),
                linked_worktree(app, "fix", project("app-fix", 1)),
                sized(500_000_000, linked_worktree(app, "old", quiet("app-old"))),
            ],
            vec![],
        );
        let mut arrangement = Arrangement::default();
        arrangement.set_sort(SortOrder::Repository, &map, now());
        let List::Rows { rows } = listed(&map, &arrangement) else {
            panic!("a map with projects lists them");
        };
        let [
            ListRow::Repository {
                common_dir,
                name,
                tooltip,
                total,
                cards,
            },
            ListRow::QuietProjects { .. },
        ] = rows.as_slice()
        else {
            panic!("the group, then the quiet row: {rows:?}");
        };
        assert_eq!((common_dir.as_str(), name.as_str()), (app, "app"));
        assert_eq!(tooltip, "/code/app", "where the repository is");
        assert_eq!(
            total.as_deref(),
            Some("2.5 GB"),
            "the quiet worktree's size too, and nothing yet of the one being sized"
        );
        let shown: Vec<(&str, &str, bool)> = cards
            .iter()
            .map(|card| {
                (
                    card.path.as_str(),
                    card.name.as_str(),
                    card.branch.is_some(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("/code/app", "main", false),
                ("/code/app-fix", "fix", false)
            ]
        );
    }

    #[test]
    fn a_repositorys_total_waits_for_a_first_size_and_says_when_it_is_a_lower_bound() {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let cards = context(&cleaning, &notifier);
        let being_sized = project_at("/code/app");
        let read_in_part = Project {
            own_files: Measurement::Measured(DirUsage {
                unreadable_dirs: 1,
                ..usage(5_000_000)
            }),
            ..project_at("/code/app-fix")
        };
        assert_eq!(total_of(&[&being_sized], &cards), None);
        assert_eq!(
            total_of(&[&being_sized, &read_in_part], &cards).as_deref(),
            Some("\u{2265}5 MB")
        );
    }

    #[test]
    fn a_repositorys_total_falls_with_what_a_clean_leaves_as_its_worktrees_trays_do() {
        let (cleaning, notifier) = (deleted(), Notifier::new(|_, _| {}));
        let cards = context(&cleaning, &notifier);
        let app = Project {
            own_files: measured(1_000_000_000),
            artifacts: vec![cargo_target("/code/app/target", measured(12_300_000_000))],
            ..project_at("/code/app")
        };
        assert_eq!(total_of(&[&app], &cards).as_deref(), Some("1 GB"));
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
        assert_eq!(
            list(None, None, &roots, None, false, &cards),
            List::Scanning
        );
        assert!(matches!(
            list(None, None, &roots, None, true, &cards),
            List::Empty { message } if message.starts_with("The scanner stopped before its first")
        ));
        assert!(matches!(
            list(None, None, &[], None, false, &cards),
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
