//! A repository's group on the list: a header with the repository's name and the disk its
//! worktrees take, then their cards, each going by its branch, as the header names the repository.
//! Folded shut, the header stands alone and still says what its cards would of their agents: their
//! robots stand at its end, and it throbs while one of them waits on the user.

use crate::dashboard::contents::card::{self, CardContext, Naming, agents, subagents};
use crate::dashboard::listing::{Folds, Group};
use crate::dashboard::{ListRow, RepositoryFold, format};
use crate::map::{FolderSize, Project, Snapshot};

/// The row of `group`, on `map`, folded shut when `folds` holds its repository.
pub(super) fn row(group: &Group, map: &Snapshot, folds: &Folds, cards: &CardContext) -> ListRow {
    let common_dir = format::path_text(group.common_dir);
    let fold = if folds.is_folded(&common_dir) {
        folded(&group.cards, cards)
    } else {
        let nested = group.cards.iter();
        RepositoryFold::Open {
            cards: nested
                .map(|project| card::card(project, Naming::Branch, map, cards))
                .collect(),
        }
    };
    ListRow::Repository {
        common_dir,
        name: group.name.clone(),
        tooltip: format::tilde(group.folder, cards.home),
        total: total_of(&group.worktrees, cards),
        fold,
    }
}

/// What the header of a folded group shows of `worktrees`, the cards it holds: a robot for each
/// agent in them, every session before every subagent, each in the order their cards would stand
/// them, and whether one of the sessions waits on the user, as would make its card throb.
fn folded(worktrees: &[&Project], cards: &CardContext) -> RepositoryFold {
    let sessions = worktrees
        .iter()
        .flat_map(|worktree| agents::badges(&worktree.agents, cards.now));
    let at_work = worktrees.iter().flat_map(|worktree| &worktree.subagents);
    let waiting = agents::any_waiting(worktrees.iter().flat_map(|worktree| &worktree.agents));
    RepositoryFold::Folded {
        sessions: sessions.collect(),
        subagents: subagents::badges(at_work, cards.now),
        waiting,
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::dashboard::AgentLook;
    use crate::dashboard::cleaning::Cleaning;
    use crate::dashboard::cleaning::tests::{cleaning, deleted};
    use crate::dashboard::listing::fixtures::{
        agent, linked_worktree, main_worktree, now, project, quiet, snapshot, subagent_in,
    };
    use crate::dashboard::listing::{Arrangement, Listing, Row};
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

    /// The row of the first group the Activity sort lists for `map`, with the groups of the
    /// repositories in `folds` folded shut.
    fn first_group(map: &Snapshot, folds: &Folds) -> ListRow {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let listing = Listing::new(map, &Arrangement::default(), now());
        let group = listing.rows.iter().find_map(|row| match row {
            Row::Repository(group) => Some(group),
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
        let ListRow::Repository {
            common_dir,
            name,
            tooltip,
            total,
            fold: RepositoryFold::Open { cards },
        } = first_group(&map, &Folds::default())
        else {
            panic!("the group, open");
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
    fn a_folded_group_keeps_its_name_and_disk_and_shows_its_worktrees_robots_sessions_first() {
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
        folds.toggle(app, &map(AgentStatus::Waiting));
        let folded = |fix_status| {
            let ListRow::Repository {
                name, total, fold, ..
            } = first_group(&map(fix_status), &folds)
            else {
                panic!("a group's row");
            };
            let RepositoryFold::Folded {
                sessions,
                subagents,
                waiting,
            } = fold
            else {
                panic!("folded: {fold:?}");
            };
            let looks: Vec<AgentLook> = sessions.iter().map(|badge| badge.look).collect();
            (name, total, looks, subagents.len(), waiting)
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
}
