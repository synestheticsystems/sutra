//! A group on the list: a line naming the parent the cards share, with the disk its cards take,
//! then the cards, each going by what sets it apart, as the line names what they share. A
//! repository's line goes by its name; a session's by the session's name, with the mark of its
//! scratch, which steps back once the session has ended, as whether it runs is what says whether
//! its scratch may still be in use. A right click on a session's line cleans the build folders in
//! its scratch. Folded shut (see `fold`), the line stands alone and still says what its cards would
//! of their agents and their dev servers: their robots stand at its end, it throbs while one of
//! them waits on the user, and the servers running in them stand after its total.

mod fold;

use crate::dashboard::cleaning::Cleaning;
use crate::dashboard::contents::card::{self, CardContext, Naming};
use crate::dashboard::listing::{self, Folds, Group};
use crate::dashboard::{
    Action, GroupFold, GroupLine, ListRow, MenuAction, MenuEntry, Parent, format,
};
use crate::map::{ArtifactDir, DirUsage, FolderSize, Project, ScratchSpace, Snapshot};
use fold::folded;

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
    let (tooltip, scratch, menu) = match group.parent {
        listing::Parent::Repository(repo) => {
            (format::tilde(repo.folder(), cards.home), None, Vec::new())
        }
        listing::Parent::Session(space) => (
            session_tooltip(space, &group.name, cards),
            Some(card::scratch_mark(space)),
            clean_build_folders(space, &group.members, cards.cleaning),
        ),
    };
    ListRow::Group {
        line: GroupLine {
            parent,
            name: group.name.clone(),
            tooltip,
            scratch,
            total: total_of(&group.members, cards),
            menu,
        },
        fold: Box::new(fold),
    }
}

/// What a right click on the line of `space`'s session offers: one clean of every build folder in
/// `members`, the session's scratch folders on the map, that a card would offer to clean, with what
/// they hold between them, which is what the clean can give back, rather than the line's total;
/// nothing while none is on offer.
fn clean_build_folders(
    space: &ScratchSpace,
    members: &[&Project],
    cleaning: &Cleaning,
) -> Vec<MenuEntry> {
    let offered: Vec<&ArtifactDir> = members
        .iter()
        .flat_map(|member| &member.artifacts)
        .filter(|dir| card::offers_clean(dir, cleaning))
        .collect();
    if offered.is_empty() {
        return Vec::new();
    }
    let sized: Vec<&DirUsage> = offered
        .iter()
        .filter_map(|dir| dir.measurement.usage())
        .collect();
    let title = if sized.is_empty() {
        "Clean Build Folders".to_string()
    } else {
        let bytes = sized.iter().map(|usage| usage.bytes).sum();
        // What a folder not sized yet, or not read in full, holds adds to the sum unseen.
        let partial = sized.len() < offered.len() || sized.iter().any(|u| u.unreadable_dirs > 0);
        format!("Clean Build Folders ({})", format::size(bytes, partial))
    };
    let session_folder = format::path_text(&space.session_folder);
    vec![MenuEntry::Item {
        title,
        checked: false,
        action: MenuAction::Dashboard {
            action: Action::CleanScratch { session_folder },
        },
    }]
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
        linked_worktree, main_worktree, now, project, quiet, running_as, scratch_folder, snapshot,
    };
    use crate::dashboard::listing::{Arrangement, Listing, Row};
    use crate::dashboard::{Chip, ScratchMark};
    use crate::map::test_support::{cargo_target, measured, node_modules, project_at, usage};
    use crate::map::{DirUsage, Measurement};
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

    /// The line and the fold of the first group the Activity sort lists for `map`, with `folds`.
    pub(super) fn first_group(map: &Snapshot, folds: &Folds) -> (GroupLine, GroupFold) {
        first_group_in(map, &Arrangement::default(), folds)
    }

    /// The line and the fold of the first group the Activity sort lists for `map` as `arrangement`
    /// arranges it, with `folds`.
    fn first_group_in(
        map: &Snapshot,
        arrangement: &Arrangement,
        folds: &Folds,
    ) -> (GroupLine, GroupFold) {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let listing = Listing::new(map, arrangement, &cleaning, now());
        let group = listing.rows.iter().find_map(|row| match row {
            Row::Group(group) => Some(group),
            _ => None,
        });
        let group = group.expect("a group");
        let ListRow::Group { line, fold } = row(group, map, folds, &context(&cleaning, &notifier))
        else {
            panic!("a group's row is a group");
        };
        (line, *fold)
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
        let (line, GroupFold::Open { cards }) = first_group(&map, &Folds::default()) else {
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

    /// A map whose session `e26ca359` has the scratch folders `r567`, holding 100 GB,
    /// `target-567`, holding 2 GB, and `notes`, too small for a row, the session running as
    /// `Foundry` while `running` says.
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
            vec![
                folder("r567", 100_000_000_000),
                folder("target-567", 2_000_000_000),
                folder("notes", 400_000_000),
            ],
            vec![],
        )
    }

    #[test]
    fn a_sessions_group_starts_folded_under_its_name_with_its_scratch_mark_and_disk() {
        let (line, fold) = first_group(&foundry(true), &Folds::default());
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
                tooltip: "Claude Code's scratch space\nSession e26ca359".to_string(),
                session_runs: true,
            }),
            "the mark of the whole space, which the window draws around the total"
        );
        assert_eq!(
            line.total.as_deref(),
            Some("102 GB"),
            "every folder, listed or not"
        );
        assert!(matches!(fold, GroupFold::Folded { .. }), "{fold:?}");
    }

    #[test]
    fn a_sessions_open_group_holds_its_folders_by_their_own_names_and_says_when_it_has_ended() {
        let mut folds = Folds::default();
        let session = Parent::Session {
            folder: "/private/tmp/claude-501/-code-app/e26ca359".to_string(),
        };
        folds.toggle(&session, &foundry(false));
        // Quiet once its session has ended, the scratch has its line in the Activity sort only
        // when a typed filter lists it.
        let mut filtered = Arrangement::default();
        filtered.set_filter_text(
            "scratchpad".to_string(),
            Some(&foundry(false)),
            &cleaning(),
            now(),
        );
        let (line, GroupFold::Open { cards }) = first_group_in(&foundry(false), &filtered, &folds)
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
        assert_eq!(
            names,
            [("r567", None), ("target-567", None)],
            "notes, too small, has no row"
        );
    }

    #[test]
    fn a_sessions_line_offers_to_clean_its_scratchs_build_folders_and_a_repositorys_nothing() {
        let with = |folder: &str, artifacts: Vec<ArtifactDir>| {
            let scratch = Project {
                artifacts,
                ..scratch_folder("e26ca359", folder, project("", 1))
            };
            running_as("Foundry", scratch)
        };
        let scratchpad = "/private/tmp/claude-501/-code-app/e26ca359/scratchpad";
        let r567 = format!("{scratchpad}/r567");
        let fix = format!("{scratchpad}/target-567-fix");
        let map = |fix_measurement| {
            snapshot(
                vec![
                    with(
                        "r567",
                        vec![
                            cargo_target(format!("{r567}/target"), measured(60_000_000_000)),
                            node_modules(format!("{r567}/ui/node_modules"), measured(1 << 30)),
                        ],
                    ),
                    with("target-567-fix", vec![cargo_target(&fix, fix_measurement)]),
                ],
                vec![],
            )
        };
        let menu = |map: &Snapshot| {
            let (line, _) = first_group(map, &Folds::default());
            line.menu
        };

        let [MenuEntry::Item { title, action, .. }] = &menu(&map(measured(30_000_000_000)))[..]
        else {
            panic!("one line");
        };
        assert_eq!(
            title, "Clean Build Folders (90 GB)",
            "the two Cargo build folders, not the scratch's whole size"
        );
        assert_eq!(
            *action,
            MenuAction::Dashboard {
                action: Action::CleanScratch {
                    session_folder: "/private/tmp/claude-501/-code-app/e26ca359".to_string()
                }
            }
        );
        let [MenuEntry::Item { title, .. }] = &menu(&map(Measurement::Measuring))[..] else {
            panic!("one line");
        };
        assert_eq!(
            title, "Clean Build Folders (\u{2265}60 GB)",
            "one not sized yet"
        );

        let app = "/code/app/.git";
        let repository = snapshot(
            vec![
                main_worktree(app, "main", project("app", 1)),
                linked_worktree(app, "fix", project("app-fix", 1)),
            ],
            vec![],
        );
        assert_eq!(menu(&repository), []);
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
