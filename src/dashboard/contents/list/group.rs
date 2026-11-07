//! A group on the list: a line naming the parent the cards share, with the disk its cards take,
//! then the cards, each going by what sets it apart, as the line names what they share. A
//! repository's line goes by its name; a session's by the session's name, with the mark of its
//! scratch, and the whole line steps back once the session has ended, as whether it runs is what
//! says whether its scratch may still be in use. Folded shut, the line stands alone and still says
//! what its cards would of their agents: their robots stand at its end, and it throbs while one of
//! them waits on the user.

use crate::dashboard::contents::card::{self, CardContext, Naming, agents, subagents};
use crate::dashboard::listing::{self, Folds, Group};
use crate::dashboard::{GroupFold, GroupLine, ListRow, Parent, format};
use crate::map::{FolderSize, Project, ScratchSpace, Snapshot};

/// The row of `group`, on `map`, folded shut as `folds` says.
pub(super) fn row(group: &Group, map: &Snapshot, folds: &Folds, cards: &CardContext) -> ListRow {
    let parent = parent(group.parent);
    let fold = if folds.is_folded(&parent) {
        folded(&group.cards, cards)
    } else {
        let nested = group.cards.iter();
        GroupFold::Open {
            cards: nested
                .map(|project| card::card(project, Naming::Nested, map, cards))
                .collect(),
        }
    };
    let (tooltip, scratch) = match group.parent {
        listing::Parent::Repository(repo) => (format::tilde(repo.folder(), cards.home), None),
        listing::Parent::Session(space) => (
            session_tooltip(space, &group.name, cards),
            Some(card::scratch_mark(space, None)),
        ),
    };
    ListRow::Group {
        line: GroupLine {
            parent,
            name: group.name.clone(),
            tooltip,
            scratch,
            total: total_of(&group.members, cards),
        },
        fold,
    }
}

/// The parent as the window knows it, by the text of the folder it is known by.
fn parent(parent: listing::Parent) -> Parent {
    let folder = format::path_text(parent.folder());
    match parent {
        listing::Parent::Repository(_) => Parent::Repository { common_dir: folder },
        listing::Parent::Session(_) => Parent::Session { folder },
    }
}

/// The tooltip of the line of `space`'s session, which goes by `name`: the session and whether it
/// still runs, then its folder.
fn session_tooltip(space: &ScratchSpace, name: &str, cards: &CardContext) -> String {
    let runs = if space.session.is_some() {
        "running"
    } else {
        "ended"
    };
    let folder = format::tilde(&space.session_folder, cards.home);
    format!("{name}, {runs}\n{folder}")
}

/// What the line of a folded group shows of `cards`, the cards it holds: a robot for each agent in
/// them, every session before every subagent, each in the order their cards would stand them, and
/// whether one of the sessions waits on the user, as would make its card throb.
fn folded(cards: &[&Project], ctx: &CardContext) -> GroupFold {
    let sessions = cards
        .iter()
        .flat_map(|card| agents::badges(&card.agents, ctx.now));
    let at_work = cards.iter().flat_map(|card| &card.subagents);
    let waiting = agents::any_waiting(cards.iter().flat_map(|card| &card.agents));
    GroupFold::Folded {
        sessions: sessions.collect(),
        subagents: subagents::badges(at_work, ctx.now),
        waiting,
    }
}

/// What `members` take on disk between them, as their trays write it: the totals the trays show,
/// added up, at least that when part of one could not be read. A member still being sized adds
/// nothing, and the sum says nothing of it; `None` while none is sized.
fn total_of(members: &[&Project], cards: &CardContext) -> Option<String> {
    let total = members
        .iter()
        .filter_map(|member| card::tray_total(member, cards))
        .reduce(|total, size| FolderSize {
            bytes: total.bytes + size.bytes,
            partial: total.partial || size.partial,
        })?;
    Some(format::size(total.bytes, total.partial))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::dashboard::cleaning::Cleaning;
    use crate::dashboard::cleaning::tests::{cleaning, deleted};
    use crate::dashboard::listing::fixtures::{
        agent, linked_worktree, main_worktree, now, project, quiet, running_as, scratch_folder,
        snapshot, subagent_in,
    };
    use crate::dashboard::listing::{Arrangement, Listing, Row};
    use crate::dashboard::{AgentLook, Chip, ScratchMark};
    use crate::map::test_support::{cargo_target, measured, project_at, usage};
    use crate::map::{AgentStatus, DirUsage, Measurement};
    use crate::notifications::Notifier;

    /// What the cards read, with `cleaning` at work.
    fn context<'a>(cleaning: &'a Cleaning, notifier: &'a Notifier) -> CardContext<'a> {
        CardContext {
            notifier,
            cleaning,
            home: Path::new("/Users/dev"),
            now: now(),
        }
    }

    /// The row of the first group the Activity sort lists for `map`, with `folds`.
    fn first_group(map: &Snapshot, folds: &Folds) -> ListRow {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let listing = Listing::new(map, &Arrangement::default(), now());
        let group = listing.rows.iter().find_map(|row| match row {
            Row::Group(group) => Some(group),
            _ => None,
        });
        row(
            group.expect("a group"),
            map,
            folds,
            &context(&cleaning, &notifier),
        )
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
        let ListRow::Group {
            line,
            fold: GroupFold::Open { cards },
        } = first_group(&map, &Folds::default())
        else {
            panic!("the group, open");
        };
        assert_eq!(
            line.parent,
            Parent::Repository {
                common_dir: app.to_string()
            }
        );
        assert_eq!(line.name, "app");
        assert_eq!(line.tooltip, "/code/app", "where the repository is");
        assert_eq!(line.scratch, None);
        assert_eq!(
            line.total.as_deref(),
            Some("2.5 GB"),
            "the quiet worktree's size too, and nothing yet of the one being sized"
        );
        let shown: Vec<(&str, &str, bool)> = cards
            .iter()
            .map(|card| (card.path.as_str(), card.name.as_str(), card.chip.is_some()))
            .collect();
        assert_eq!(
            shown,
            [
                ("/code/app", "main", false),
                ("/code/app-fix", "fix", false)
            ]
        );
    }

    /// A map whose session `e26ca359` has the scratch folders `r567`, holding 100 GB, and
    /// `target-567`, the session running as `Foundry` while `running` says.
    fn foundry(running: bool) -> Snapshot {
        let folder = |name, bytes| {
            let mut folder = scratch_folder("e26ca359", name, project("", 1));
            folder.own_files = measured(bytes);
            if running {
                folder = running_as("Foundry", folder);
            }
            folder
        };
        snapshot(
            vec![folder("r567", 100_000_000_000), folder("target-567", 0)],
            vec![],
        )
    }

    #[test]
    fn a_sessions_group_starts_folded_under_its_name_with_its_scratch_mark_and_disk() {
        let ListRow::Group { line, fold } = first_group(&foundry(true), &Folds::default()) else {
            panic!("a group's row");
        };
        let session = "/private/tmp/claude-501/-code-app/e26ca359";
        assert_eq!(
            line.parent,
            Parent::Session {
                folder: session.to_string()
            }
        );
        assert_eq!(line.name, "Foundry", "the session's own name while it runs");
        assert_eq!(line.tooltip, format!("Foundry, running\n{session}"));
        assert_eq!(
            line.scratch,
            Some(ScratchMark {
                place: None,
                tooltip: "Claude Code's scratch space\nSession e26ca359".to_string(),
                session_runs: true,
            }),
            "the mark of the whole space"
        );
        assert_eq!(line.total.as_deref(), Some("100 GB"));
        assert!(matches!(fold, GroupFold::Folded { .. }), "{fold:?}");
    }

    #[test]
    fn a_sessions_open_group_holds_its_folders_by_their_own_names_and_says_when_it_has_ended() {
        let mut folds = Folds::default();
        let session = Parent::Session {
            folder: "/private/tmp/claude-501/-code-app/e26ca359".to_string(),
        };
        folds.toggle(&session, &foundry(false));
        let ListRow::Group {
            line,
            fold: GroupFold::Open { cards },
        } = first_group(&foundry(false), &folds)
        else {
            panic!("the group, open");
        };
        assert_eq!(line.name, "app e26ca359", "where it started, and its id");
        assert!(line.tooltip.starts_with("app e26ca359, ended\n"));
        assert_eq!(line.scratch.map(|mark| mark.session_runs), Some(false));
        let names: Vec<(&str, Option<&Chip>)> = cards
            .iter()
            .map(|card| (card.name.as_str(), card.chip.as_ref()))
            .collect();
        assert_eq!(names, [("r567", None), ("target-567", None)]);
    }

    #[test]
    fn a_folded_group_keeps_its_name_and_disk_and_shows_its_cards_robots_sessions_first() {
        let app = "/code/app/.git";
        let map = |fix_status| {
            let mut main = main_worktree(app, "main", project("app", 1));
            main.own_files = measured(2_000_000_000);
            main.agents.push(agent("/code/app", AgentStatus::Busy));
            main.subagents.push(subagent_in("/code/app"));
            let mut fix = linked_worktree(app, "fix", project("app-fix", 1));
            fix.agents.push(agent("/code/app-fix", fix_status));
            snapshot(vec![main, fix], vec![])
        };
        let mut folds = Folds::default();
        let parent = Parent::Repository {
            common_dir: app.to_string(),
        };
        folds.toggle(&parent, &map(AgentStatus::Waiting));
        let folded = |fix_status| {
            let ListRow::Group { line, fold } = first_group(&map(fix_status), &folds) else {
                panic!("a group's row");
            };
            let GroupFold::Folded {
                sessions,
                subagents,
                waiting,
            } = fold
            else {
                panic!("folded: {fold:?}");
            };
            let looks: Vec<AgentLook> = sessions.iter().map(|badge| badge.look).collect();
            (line.name, line.total, looks, subagents.len(), waiting)
        };

        assert_eq!(
            folded(AgentStatus::Waiting),
            (
                "app".to_string(),
                Some("2 GB".to_string()),
                vec![AgentLook::Working, AgentLook::Waiting],
                1,
                true
            ),
            "app's session before app-fix's, then the subagent, throbbing for app-fix's"
        );
        let (.., waiting) = folded(AgentStatus::Busy);
        assert!(!waiting, "no throb while no session waits");
    }

    #[test]
    fn a_groups_total_waits_for_a_first_size_and_says_when_it_is_a_lower_bound() {
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
    fn a_groups_total_falls_with_what_a_clean_leaves_as_its_cards_trays_do() {
        let (cleaning, notifier) = (deleted(), Notifier::new(|_, _| {}));
        let cards = context(&cleaning, &notifier);
        let app = Project {
            own_files: measured(1_000_000_000),
            artifacts: vec![cargo_target("/code/app/target", measured(12_300_000_000))],
            ..project_at("/code/app")
        };
        assert_eq!(total_of(&[&app], &cards).as_deref(), Some("1 GB"));
    }
}
