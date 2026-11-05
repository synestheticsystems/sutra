//! One project's card: its identity (name, branch, age), what is happening in it (why its folder
//! cannot be read, its agents, its environment's units), and, on a tray along the bottom, its disk
//! use. A right click offers what can be done to it.

pub(super) mod agents;
mod header;
mod menu;
mod tray;
mod units;

pub(in crate::dashboard) use menu::offers_terminate;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::dashboard::cleaning::Cleaning;
use crate::dashboard::{Card, Chip, format};
use crate::map::{FolderAccess, Project, Snapshot};
use crate::notifications::Notifier;

/// What every card reads besides its own project and the map.
pub(in crate::dashboard) struct CardContext<'a> {
    pub(in crate::dashboard) notifier: &'a Notifier,
    pub(in crate::dashboard) cleaning: &'a Cleaning,
    /// The real home folder, written `~` in the paths shown.
    pub(in crate::dashboard) home: &'a Path,
    /// One instant for every age on screen, so cards worked out together agree.
    pub(in crate::dashboard) now: SystemTime,
}

/// The card of `project`, on `map`.
pub(super) fn card(project: &Project, map: &Snapshot, ctx: &CardContext) -> Card {
    let last_activity = project.last_activity.and_then(format::timestamp);
    Card {
        path: format::path_text(&project.path),
        location: format::tilde(&project.path, ctx.home),
        name: project.name.clone(),
        name_tooltip: header::name_tooltip(project, ctx.home, last_activity.as_deref()),
        branch: project.repo.as_ref().map(|repo| Chip {
            text: header::branch(repo),
            tooltip: header::worktree(repo, ctx.home),
        }),
        age: project.last_activity.map(|then| format::age(then, ctx.now)),
        unreadable: unreadable_reason(project, &map.roots, map.sandboxed, ctx.home),
        agents: agents::lozenges(&project.agents, ctx.now),
        units: project
            .environment
            .as_ref()
            .map_or_else(Vec::new, |env| units::lozenges(env, ctx.notifier)),
        tray: tray::tray(project, |dir| ctx.cleaning.state_of(dir), ctx.home),
        // Only an agent makes a card throb: a failed unit can stand for a long time without
        // needing the user now.
        waiting: agents::any_waiting(&project.agents),
        menu: menu::entries(project, ctx.cleaning, ctx.notifier, map.sandboxed),
    }
}

/// Why the card shows nothing from inside its folder: `folder missing`, `can't read this folder`,
/// `in Documents, not watched`, or, in the store edition for a folder outside every root,
/// `outside the granted folder`. `None` for a folder Sutra reads.
fn unreadable_reason(
    project: &Project,
    roots: &[PathBuf],
    sandboxed: bool,
    home: &Path,
) -> Option<String> {
    let reason = match project.access {
        FolderAccess::Readable => return None,
        FolderAccess::Missing => "folder missing".to_string(),
        FolderAccess::Denied
            if sandboxed && !roots.iter().any(|root| project.path.starts_with(root)) =>
        {
            "outside the granted folder".to_string()
        }
        FolderAccess::Denied => "can't read this folder".to_string(),
        FolderAccess::PrivateFolder => {
            let private_folder = project
                .path
                .strip_prefix(home)
                .ok()
                .and_then(|inside| inside.components().next());
            match private_folder {
                Some(folder) => format!("in {}, not watched", folder.as_os_str().to_string_lossy()),
                None => "in a private folder, not watched".to_string(),
            }
        }
    };
    Some(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::{agent, now, project, snapshot};
    use crate::map::test_support::{environment_at, project_at};
    use crate::model::{Environment, UnitStatus};

    const HOME: &str = "/Users/dev";

    #[test]
    fn a_folder_that_reads_needs_no_label() {
        let home = Path::new(HOME);
        let readable = project_at("/Users/dev/code/app");
        assert_eq!(unreadable_reason(&readable, &[], true, home), None);
    }

    #[test]
    fn an_unreadable_folder_says_why_in_true_words() {
        let home = Path::new(HOME);
        let grant = [PathBuf::from("/Users/dev/code")];
        let label = |path, access, sandboxed| {
            let project = Project {
                access,
                ..project_at(path)
            };
            unreadable_reason(&project, &grant, sandboxed, home).unwrap()
        };
        assert_eq!(
            label("/Users/dev/code/gone", FolderAccess::Missing, false),
            "folder missing"
        );
        assert_eq!(
            label("/private/tmp/wt", FolderAccess::Denied, true),
            "outside the granted folder"
        );
        assert_eq!(
            label("/private/tmp/wt", FolderAccess::Denied, false),
            "can't read this folder",
            "the direct edition has no grant"
        );
        assert_eq!(
            label("/Users/dev/code/sealed", FolderAccess::Denied, true),
            "can't read this folder",
            "inside the grant, the grant is not why"
        );
        assert_eq!(
            label(
                "/Users/dev/Documents/app",
                FolderAccess::PrivateFolder,
                false
            ),
            "in Documents, not watched"
        );
    }

    #[test]
    fn a_card_waits_while_an_agent_in_it_waits_and_not_for_a_busy_agent_or_a_failed_unit() {
        let mut waiting = project("waiting", 1);
        waiting
            .agents
            .push(agent("/code/waiting", crate::map::AgentStatus::Waiting));
        let mut busy = project("busy", 1);
        busy.agents
            .push(agent("/code/busy", crate::map::AgentStatus::Busy));
        let mut failed = project("failed", 1);
        failed.environment = Some(Environment {
            units: vec![UnitStatus::parse("server", "failed: E0308")],
            ..environment_at(&failed.path)
        });
        let map = snapshot(vec![waiting, busy, failed], vec![]);
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new(HOME),
            now: now(),
        };
        let waiting: Vec<bool> = map
            .projects
            .iter()
            .map(|project| card(project, &map, &ctx).waiting)
            .collect();
        assert_eq!(waiting, [true, false, false]);
    }

    #[test]
    fn a_card_names_its_folder_by_path_and_shows_it_with_a_tilde() {
        let map = snapshot(vec![project_at("/Users/dev/code/app")], vec![]);
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new(HOME),
            now: now(),
        };
        let card = card(&map.projects[0], &map, &ctx);
        assert_eq!(card.path, "/Users/dev/code/app");
        assert_eq!(card.location, "~/code/app");
        assert_eq!(card.name, "app");
        assert_eq!(card.age, None, "no activity known");
    }
}
