//! One project's card: its identity (name, branch, age), what is happening in it (why its folder
//! cannot be read, its environment's units), and, on a tray along the bottom, its disk use and its
//! agents, sessions before subagents. A right click offers what can be done to it.

pub(super) mod agents;
mod header;
mod menu;
mod robots;
pub(super) mod subagents;
mod tray;
mod units;

pub(in crate::dashboard) use menu::offers_terminate;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::dashboard::cleaning::Cleaning;
use crate::dashboard::{Card, Unreadable, UnreadableLook, format};
use crate::map::{FolderAccess, FolderSize, KeptOutBy, Project, Snapshot};
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

/// What a card goes by, which its place on the list decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Naming {
    /// Its folder's name, beside its branch on a chip.
    Folder,
    /// Its branch, nested under the header that names its repository.
    Branch,
}

/// The card of `project`, on `map`, going by what `naming` says.
pub(super) fn card(project: &Project, naming: Naming, map: &Snapshot, ctx: &CardContext) -> Card {
    let last_activity = project.last_activity.and_then(format::timestamp);
    let identity = header::identity(project, naming, ctx.home, last_activity.as_deref());
    // A session's scratch folder shows the session first, which works elsewhere.
    let owner = project
        .scratch
        .as_ref()
        .and_then(|space| space.session.as_ref());
    let owner = owner.map(|session| agents::badge(session, ctx.now));
    let sessions = owner
        .into_iter()
        .chain(agents::badges(&project.agents, ctx.now))
        .collect();
    let subagents = subagents::badges(&project.subagents, ctx.now);
    Card {
        path: format::path_text(&project.path),
        location: format::tilde(&project.path, ctx.home),
        name: identity.name,
        name_tooltip: identity.name_tooltip,
        branch: identity.branch,
        age: project.last_activity.map(|then| format::age(then, ctx.now)),
        unreadable: unreadable(project, &map.roots, map.sandboxed),
        units: project
            .environment
            .as_ref()
            .map_or_else(Vec::new, |env| units::lozenges(env, ctx.notifier)),
        tray: tray::tray(
            project,
            sessions,
            subagents,
            |dir| ctx.cleaning.state_of(dir),
            ctx.home,
        ),
        // Only an agent makes a card throb: a failed unit can stand for a long time without
        // needing the user now.
        waiting: agents::any_waiting(&project.agents),
        menu: menu::entries(project, ctx.cleaning, ctx.notifier, map.sandboxed),
    }
}

/// The total on `project`'s tray: its folder's whole size, with what a clean has left of the
/// build folders it is at work on; `None` until the folder is sized.
pub(super) fn tray_total(project: &Project, ctx: &CardContext) -> Option<FolderSize> {
    tray::total_on_card(project, |dir| ctx.cleaning.state_of(dir))
}

/// Why the card shows nothing from inside its folder. Kept out by the user's choice, or until
/// macOS asks them: `in Desktop, not watched`, `in Documents, waiting for macOS`, `in Documents,
/// macOS said no`, or, in the store edition for a folder outside every root, `outside the granted
/// folder`. Failed: `folder missing` or `can't read this folder`. `None` for a folder Sutra reads.
fn unreadable(project: &Project, roots: &[PathBuf], sandboxed: bool) -> Option<Unreadable> {
    let (reason, look) = match project.access {
        FolderAccess::Readable => return None,
        FolderAccess::KeptOut(private, by) => {
            let why = match by {
                KeptOutBy::Settings => "not watched",
                KeptOutBy::Prompt => "waiting for macOS",
                KeptOutBy::MacOs => "macOS said no",
            };
            (
                format!("in {}, {why}", private.name()),
                UnreadableLook::KeptOut,
            )
        }
        FolderAccess::Denied
            if sandboxed && !roots.iter().any(|root| project.path.starts_with(root)) =>
        {
            (
                "outside the granted folder".to_string(),
                UnreadableLook::KeptOut,
            )
        }
        FolderAccess::Denied => ("can't read this folder".to_string(), UnreadableLook::Failed),
        FolderAccess::Missing => ("folder missing".to_string(), UnreadableLook::Failed),
    };
    Some(Unreadable { reason, look })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::Robot;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::{agent, now, project, snapshot, subagent_in};
    use crate::map::PrivateFolder;
    use crate::map::test_support::{environment_at, project_at};
    use crate::model::{Environment, UnitStatus};

    const HOME: &str = "/Users/dev";

    #[test]
    fn a_folder_that_reads_needs_no_label() {
        let readable = project_at("/Users/dev/code/app");
        assert_eq!(unreadable(&readable, &[], true), None);
    }

    #[test]
    fn an_unreadable_folder_says_why_in_true_words_and_looks_quiet_when_kept_out_by_choice() {
        let grant = [PathBuf::from("/Users/dev/code")];
        let label = |path, access, sandboxed| {
            let project = Project {
                access,
                ..project_at(path)
            };
            let unreadable = unreadable(&project, &grant, sandboxed).unwrap();
            (unreadable.reason, unreadable.look)
        };
        let failed = |reason: &str| (reason.to_string(), UnreadableLook::Failed);
        let kept_out = |reason: &str| (reason.to_string(), UnreadableLook::KeptOut);
        assert_eq!(
            label("/Users/dev/code/gone", FolderAccess::Missing, false),
            failed("folder missing")
        );
        assert_eq!(
            label("/private/tmp/wt", FolderAccess::Denied, true),
            kept_out("outside the granted folder")
        );
        assert_eq!(
            label("/private/tmp/wt", FolderAccess::Denied, false),
            failed("can't read this folder"),
            "the direct edition has no grant"
        );
        assert_eq!(
            label("/Users/dev/code/sealed", FolderAccess::Denied, true),
            failed("can't read this folder"),
            "inside the grant, the grant is not why"
        );
        assert_eq!(
            label(
                "/Users/dev/Desktop/app",
                FolderAccess::KeptOut(PrivateFolder::Desktop, KeptOutBy::Settings),
                false
            ),
            kept_out("in Desktop, not watched")
        );
    }

    #[test]
    fn a_folder_macos_keeps_out_says_whether_it_is_still_asking_or_said_no_and_looks_quiet() {
        let in_documents = |by| {
            let project = Project {
                access: FolderAccess::KeptOut(PrivateFolder::Documents, by),
                ..project_at("/Users/dev/Documents/Codex/task/work/app")
            };
            let unreadable = unreadable(&project, &[PathBuf::from(HOME)], false).unwrap();
            assert_eq!(unreadable.look, UnreadableLook::KeptOut, "{by:?}");
            unreadable.reason
        };
        assert_eq!(
            in_documents(KeptOutBy::Prompt),
            "in Documents, waiting for macOS"
        );
        assert_eq!(
            in_documents(KeptOutBy::MacOs),
            "in Documents, macOS said no"
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
            .map(|project| card(project, Naming::Folder, &map, &ctx).waiting)
            .collect();
        assert_eq!(waiting, [true, false, false]);
    }

    #[test]
    fn a_cards_sessions_and_every_subagent_at_work_in_it_stand_on_its_tray() {
        let mut app = project("app", 1);
        app.agents
            .push(agent("/code/app", crate::map::AgentStatus::Busy));
        app.subagents = vec![subagent_in("/code/app"), subagent_in("/code/app/src")];
        let map = snapshot(vec![app], vec![]);
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new(HOME),
            now: now(),
        };

        let card = card(&map.projects[0], Naming::Folder, &map, &ctx);

        let tray = card.tray.expect("a tray for the agents");
        assert_eq!(tray.disk, None, "nothing is sized yet");
        let robots: Vec<Robot> = tray.sessions.iter().map(|badge| badge.robot).collect();
        assert_eq!(robots, [Robot::Claude], "the session alone");
        assert_eq!(tray.subagents.len(), 2);
    }

    #[test]
    fn a_scratch_folders_session_stands_first_on_its_tray_while_it_runs_and_waits_on_its_own() {
        let owner = crate::map::AgentSession {
            label: Some("Foundry Agent Studio architecture".to_string()),
            model: Some("gpt-6-astra".to_string()),
            ..agent("/Users/dev/vt/foundry", crate::map::AgentStatus::Waiting)
        };
        let mut scratch = project_at("/private/tmp/claude-501/-Users-dev-vt-foundry/e26ca359");
        scratch.scratch = Some(crate::map::ScratchSpace {
            session_id: "e26ca359".to_string(),
            started_in: "foundry".to_string(),
            session: Some(owner),
        });
        let started_here = "/private/tmp/claude-501/-Users-dev-vt-foundry/e26ca359/scratchpad";
        scratch
            .agents
            .push(agent(started_here, crate::map::AgentStatus::Busy));
        let map = snapshot(vec![scratch], vec![]);
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new(HOME),
            now: now(),
        };

        let card = card(&map.projects[0], Naming::Folder, &map, &ctx);

        let sessions = card.tray.expect("a tray for the agents").sessions;
        let robots: Vec<Robot> = sessions.iter().map(|badge| badge.robot).collect();
        assert_eq!(
            robots,
            [Robot::Codex, Robot::Claude],
            "its own session first"
        );
        assert!(
            sessions[0]
                .tooltip
                .contains("Foundry Agent Studio architecture")
        );
        assert!(
            !card.waiting,
            "its session waits on the card of the folder it works in"
        );
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
        let card = card(&map.projects[0], Naming::Folder, &map, &ctx);
        assert_eq!(card.path, "/Users/dev/code/app");
        assert_eq!(card.location, "~/code/app");
        assert_eq!(card.name, "app");
        assert_eq!(card.age, None, "no activity known");
    }
}
