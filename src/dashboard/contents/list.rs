//! The list, or what stands in its place: what the store edition needs before it can read
//! anything, the wait for the first map, or why nothing is listed. Otherwise the rows (see `rows`):
//! each group's line, which names the upstream its cards came from, then what hangs from it in a
//! tree of provenance, each checkout with copies under a line of its own (see `line`); Docker's
//! group, its line over a card for each container and the disk Docker takes (see `docker`); and the
//! row standing for the quiet groups.

mod docker;
mod elsewhere;
mod line;
mod rows;

pub use docker::{AccountedFor, ContainerCard};
pub use elsewhere::ElsewhereLine;
pub use line::{GroupKind, Line, LineFold, LineKey, MoreLozenges, WhenIdleMark};
pub use rows::{ListRow, Place};

pub(in crate::dashboard) use line::offered_cleans;

use std::path::PathBuf;

use super::card::CardContext;
use crate::dashboard::cleaning::CleanWhenIdle;
use crate::dashboard::listing::{FOCUS_RECENT, Folds, Listing};
use crate::dashboard::{SortOrder, format};
use crate::map::LARGE_FOLDER_BYTES;
use crate::map::grant::GrantError;

/// What the user chose for the list's lines that outlasts the run: which are folded shut, and how
/// long each group's build folders may sit idle before Sutra cleans them.
#[derive(Clone, Copy)]
pub(in crate::dashboard) struct LineChoices<'a> {
    pub(in crate::dashboard) folds: &'a Folds,
    pub(in crate::dashboard) clean_when_idle: &'a CleanWhenIdle,
}

/// The list, or what stands in its place.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum List {
    /// The store edition has no folder to read yet: what Sutra needs and why, with the button that
    /// opens the panel (`MenuAction::ChooseFolder`).
    NoFolder {
        message: String,
        /// Why the folder granted on an earlier launch could not be reopened.
        lost: Option<String>,
    },
    /// The first map is still being made.
    Scanning,
    /// Nothing to list, or nothing the filter admits, or no map ever came because the scanner
    /// stopped, and why, in a sentence or two.
    Empty {
        message: String,
    },
    Rows {
        rows: Vec<ListRow>,
    },
}

/// The list, from `listing`, `None` until the first map arrives, and the filter field's text.
/// `roots` are the folders watched, empty while the store edition has none granted, and
/// `lost_grant` why the one granted on an earlier launch could not be reopened. `chosen` says
/// which lines are folded shut, and which groups Sutra cleans once idle.
pub(in crate::dashboard) fn list(
    listing: Option<&Listing>,
    filter: Option<&str>,
    roots: &[PathBuf],
    lost_grant: Option<&GrantError>,
    engine_stopped: bool,
    chosen: LineChoices,
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
                .flat_map(|row| rows::of(row, listing, chosen, cards))
                .collect(),
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

/// What makes a group quiet in `sort`, with the focus thresholds spelled out so the reader can
/// tell why a group is behind the row (see `listing::focus`).
fn what_makes_one_quiet(sort: SortOrder) -> String {
    let days = FOCUS_RECENT.as_secs() / 86_400;
    match sort {
        SortOrder::Activity => format!(
            "Groups with no agent session, no dev-runner environment, nothing listening on a port, \
             no container running, and no activity in the last {days} days. Scratch from Claude \
             Code sessions that have ended waits here too, as does Docker while no container runs."
        ),
        SortOrder::Name | SortOrder::Disk => format!(
            "Groups with no agent session, no dev-runner environment, nothing listening on a port, \
             no container running, no activity in the last {days} days, and no folder of {} or \
             more",
            format::bytes(LARGE_FOLDER_BYTES),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::dashboard::AgentLook;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::Arrangement;
    use crate::dashboard::listing::fixtures::{
        agent, came_from, linked_worktree, main_worktree, now, project, quiet, snapshot,
        subagent_in, upstream,
    };
    use crate::map::{AgentStatus, GroupKey, Snapshot};
    use crate::notifications::Notifier;

    const HOME: &str = "/Users/dev";

    /// What `list` gives for `map` as the user arranged it, under the root `/code`, its lines
    /// folded or open as `folds` says.
    fn listed_with(map: &Snapshot, arrangement: &Arrangement, folds: &Folds) -> List {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let cards = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new(HOME),
            now: now(),
        };
        let listing = Listing::new(map, arrangement, &cleaning, now());
        let roots = [PathBuf::from("/code")];
        let chosen = LineChoices {
            folds,
            clean_when_idle: &CleanWhenIdle::default(),
        };
        list(
            Some(&listing),
            arrangement.filter(),
            &roots,
            None,
            false,
            chosen,
            &cards,
        )
    }

    /// What `list` gives for `map` as the user arranged it, under the root `/code`, every line
    /// folded as it starts.
    fn listed(map: &Snapshot, arrangement: &Arrangement) -> List {
        listed_with(map, arrangement, &Folds::default())
    }

    #[test]
    fn the_rows_are_each_open_groups_line_then_its_rows_and_last_the_quiet_row() {
        let map = Snapshot {
            unplaced_subagents: vec![subagent_in("/Users/dev/build")],
            ..snapshot(
                vec![
                    project("recent", 1),
                    came_from(upstream("github.com/me/old"), &[], quiet("old")),
                ],
                vec![
                    agent("/tmp/idle", AgentStatus::Idle),
                    agent("/Users/dev/scratch", AgentStatus::Waiting),
                ],
            )
        };
        let mut folds = Folds::default();
        folds.toggle(&LineKey::group(&GroupKey::Other), &map);
        let List::Rows { rows } = listed_with(&map, &Arrangement::default(), &folds) else {
            panic!("a map with projects lists them");
        };
        let [
            ListRow::Line { line, .. },
            ListRow::Elsewhere {
                heading,
                lines,
                waiting,
                place,
            },
            ListRow::Card { card, .. },
            ListRow::QuietProjects {
                count, open, label, ..
            },
        ] = rows.as_slice()
        else {
            panic!("Other's line, the agents elsewhere, a card, the quiet row: {rows:?}");
        };
        assert_eq!(line.name, "Other");
        assert_eq!(
            *place,
            Place {
                depth: 1,
                first: true
            },
            "first under Other's line"
        );
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
    fn a_folded_line_stands_alone_and_an_open_ones_rows_follow_it() {
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
        let app = LineKey::group(&GroupKey::LocalRepository(PathBuf::from("/code/app")));
        folds.toggle(&app, &map);
        let List::Rows { rows } = listed_with(&map, &Arrangement::default(), &folds) else {
            panic!("a map with projects lists them");
        };
        let shown: Vec<String> = rows
            .iter()
            .map(|row| match row {
                ListRow::Line { line, .. } => match line.fold {
                    LineFold::Open => format!("{} open", line.name),
                    LineFold::Folded { .. } => format!("{} folded", line.name),
                },
                ListRow::Card { card, place } => format!("{} at {}", card.name, place.depth),
                row => panic!("only lines and cards: {row:?}"),
            })
            .collect();
        assert_eq!(
            shown,
            ["app open", "main at 1", "fix at 1", "site folded"],
            "app opened by the user, its cards by their branches, and site as it starts"
        );
    }

    #[test]
    fn dockers_group_is_its_line_then_while_it_is_open_a_card_for_each_container_then_its_disk() {
        use crate::dashboard::listing::fixtures::docker;
        let map = Snapshot {
            docker: Some(docker(&[("pg596", 12)], 0)),
            ..snapshot(vec![], vec![])
        };
        let rows = |folds: &Folds| match listed_with(&map, &Arrangement::default(), folds) {
            List::Rows { rows } => rows,
            other => panic!("rows: {other:?}"),
        };
        let folded = rows(&Folds::default());
        let [ListRow::Line { line, place }] = &folded[..] else {
            panic!("Docker's line alone: {folded:?}");
        };
        assert_eq!((line.key.clone(), place.depth), (LineKey::DOCKER, 0));

        let mut folds = Folds::default();
        assert!(folds.toggle(&LineKey::DOCKER, &map), "Docker is here");
        let open = rows(&folds);
        let [
            ListRow::Line { .. },
            ListRow::Container { card, place },
            ListRow::DockerDisk { place: under, .. },
        ] = &open[..]
        else {
            panic!("Docker's line, then pg596's card, then its disk: {open:?}");
        };
        assert_eq!(card.name, "pg596");
        let hanging = |first| Place { depth: 1, first };
        assert_eq!((*place, *under), (hanging(true), hanging(false)));
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
        filtered.set_filter_text(" site ".to_string(), Some(&map), &cleaning(), now());
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
        let none = LineChoices {
            folds: &Folds::default(),
            clean_when_idle: &CleanWhenIdle::default(),
        };
        assert_eq!(
            list(None, None, &roots, None, false, none, &cards),
            List::Scanning
        );
        assert!(matches!(
            list(None, None, &roots, None, true, none, &cards),
            List::Empty { message } if message.starts_with("The scanner stopped before its first")
        ));
        assert!(matches!(
            list(None, None, &[], None, false, none, &cards),
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
    fn the_quiet_row_counts_its_projects_and_says_what_makes_a_group_quiet_in_its_sort() {
        assert_eq!(how_many(764), "764 quiet");
        assert_eq!(
            what_makes_one_quiet(SortOrder::Activity),
            "Groups with no agent session, no dev-runner environment, nothing listening on a port, \
             no container running, and no activity in the last 7 days. Scratch from Claude Code \
             sessions that have ended waits here too, as does Docker while no container runs."
        );
        let by_size = "Groups with no agent session, no dev-runner environment, nothing listening \
                       on a port, no container running, no activity in the last 7 days, and no \
                       folder of 1.1 GB or more";
        assert_eq!(what_makes_one_quiet(SortOrder::Name), by_size);
        assert_eq!(what_makes_one_quiet(SortOrder::Disk), by_size);
    }

    #[test]
    fn the_quiet_row_says_what_makes_one_quiet_in_the_sort_the_list_is_in() {
        let old = came_from(upstream("github.com/me/old"), &[], quiet("old"));
        let map = snapshot(vec![project("recent", 1), old], vec![]);
        let tooltip = |sort| {
            let mut arrangement = Arrangement::default();
            arrangement.set_sort(sort, &map, &cleaning(), now());
            let List::Rows { rows } = listed(&map, &arrangement) else {
                panic!("a map with projects lists them");
            };
            rows.into_iter().find_map(|row| match row {
                ListRow::QuietProjects { tooltip, .. } => Some(tooltip),
                _ => None,
            })
        };
        assert_eq!(
            tooltip(SortOrder::Activity),
            Some(what_makes_one_quiet(SortOrder::Activity))
        );
        assert_eq!(
            tooltip(SortOrder::Disk),
            Some(what_makes_one_quiet(SortOrder::Disk))
        );
    }
}
