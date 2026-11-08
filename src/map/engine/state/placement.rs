//! Where the map places an agent: on the card of the project its folder lies in, or, outside every
//! project, among the agents working elsewhere; and what one load places in each place.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::owner;
use super::tracked::Tracked;
use crate::map::{AgentSession, Subagent, SubagentWithParent};

/// Where an agent goes on the map: on the card of the project its folder lies in (see `owner`), or,
/// when that is none, among the agents working elsewhere, beside the folder. A session's scratch
/// folder counts as such a project whether it has a card yet or not, as an agent standing on it
/// gives it one (see `earns_a_card`): no card decides where an agent stands, so an agent's place
/// cannot turn on the card it gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place<'a> {
    Project(&'a Path),
    Elsewhere,
}

impl<'a> Place<'a> {
    /// The place of an agent working in `folder`.
    pub(super) fn of(projects: &'a BTreeMap<PathBuf, Tracked>, folder: &'a Path) -> Place<'a> {
        owner(projects, folder).map_or(Place::Elsewhere, Place::Project)
    }

    /// The place of `subagent`: the project holding the first of its folders that a project
    /// holds, the most telling first (see `Subagent::folders`). A subagent that acted only outside
    /// every project stays on the card of the project its harness runs it in.
    pub(super) fn of_subagent(
        projects: &'a BTreeMap<PathBuf, Tracked>,
        subagent: &'a Subagent,
    ) -> Place<'a> {
        let mut folders = subagent.folders();
        let project = folders.find_map(|folder| owner(projects, folder));
        project.map_or(Place::Elsewhere, Place::Project)
    }
}

/// What one load places in one place.
#[derive(Default)]
pub(super) struct Placed {
    pub(super) sessions: Vec<AgentSession>,
    pub(super) subagents: Vec<SubagentWithParent>,
}

impl Placed {
    /// A stable order, so a reload that finds the same agents compares equal: the sessions by id,
    /// and the subagents by their session's, each session's keeping the order it gave them.
    pub(super) fn sort(&mut self) {
        let by_session = |a: &SubagentWithParent, b: &SubagentWithParent| {
            a.parent.session_id.cmp(&b.parent.session_id)
        };
        self.sessions
            .sort_by(|a, b| a.session_id.cmp(&b.session_id));
        self.subagents.sort_by(by_session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::scan::Boundary;
    use crate::map::test_support::{agent_session, subagent_at_work};
    use crate::map::{AgentStatus, ScanOptions};

    /// The map's projects at `paths`, none of them on disk.
    fn projects(paths: &[&str]) -> BTreeMap<PathBuf, Tracked> {
        let boundary = Boundary::new(&[], Path::new("/"), &ScanOptions::default());
        let tracked = |path: &&str| Tracked::plain(PathBuf::from(path), &boundary, &|_| true);
        paths
            .iter()
            .map(|path| (PathBuf::from(path), tracked(path)))
            .collect()
    }

    #[test]
    fn an_agent_goes_on_the_card_of_the_project_its_folder_lies_in_or_else_elsewhere() {
        let projects = projects(&["/code/app", "/code/app/.claude/worktrees/wt"]);
        let place = |folder: &'static str| Place::of(&projects, Path::new(folder));
        assert_eq!(
            place("/code/app/src"),
            Place::Project(Path::new("/code/app"))
        );
        assert_eq!(
            place("/code/app/.claude/worktrees/wt/src"),
            Place::Project(Path::new("/code/app/.claude/worktrees/wt"))
        );
        assert_eq!(place("/tmp/x"), Place::Elsewhere);
    }

    #[test]
    fn a_subagent_goes_on_the_card_of_the_first_project_holding_a_folder_it_acted_in_or_runs_in() {
        let projects = projects(&["/code/app", "/code/app-wt/fix", "/code/notes"]);
        // Each subagent runs in its session's folder, `/code/app`, and acted in `acted_in`: the
        // folders its newest `cd`s entered, then those holding the files it last edited.
        let acting = |acted_in: &[&str]| Subagent {
            acted_in: acted_in.iter().map(PathBuf::from).collect(),
            ..subagent_at_work("a1")
        };
        let place = |acted_in: &[&str]| {
            let subagent = acting(acted_in);
            match Place::of_subagent(&projects, &subagent) {
                Place::Project(project) => Some(project.to_path_buf()),
                Place::Elsewhere => None,
            }
        };
        let on = |project: &str| Some(PathBuf::from(project));
        assert_eq!(
            place(&["/code/app-wt/fix/src", "/code/notes"]),
            on("/code/app-wt/fix"),
            "where it went beats where it last edited, its notes"
        );
        assert_eq!(
            place(&["/tmp/scratch", "/code/app-wt/fix", "/code/notes"]),
            on("/code/app-wt/fix"),
            "where it went last lies in no project, so where it went before"
        );
        assert_eq!(
            place(&["/tmp/clone", "/code/notes"]),
            on("/code/notes"),
            "where it went lies in no project, so where it edited"
        );
        assert_eq!(
            place(&["/tmp/clone", "/tmp/clone/src"]),
            on("/code/app"),
            "both lie in no project, so where it runs"
        );
        assert_eq!(place(&[]), on("/code/app"), "it acted nowhere yet");

        let outside = Subagent {
            cwd: PathBuf::from("/tmp/scratch"),
            ..acting(&["/tmp/clone"])
        };
        assert_eq!(Place::of_subagent(&projects, &outside), Place::Elsewhere);
    }

    #[test]
    fn what_one_place_holds_sorts_by_session_each_sessions_subagents_keeping_their_order() {
        let session = |id: &str| agent_session(id, AgentStatus::Idle);
        let working = |id: &str, parent: &str| {
            SubagentWithParent::new(subagent_at_work(id), &session(parent))
        };
        let mut placed = Placed {
            sessions: vec![session("b"), session("a")],
            subagents: vec![working("b2", "b"), working("a1", "a"), working("b1", "b")],
        };

        placed.sort();

        let sessions = placed.sessions.iter();
        let sessions: Vec<&str> = sessions.map(|s| s.session_id.as_str()).collect();
        assert_eq!(sessions, ["a", "b"]);
        let subagents = placed.subagents.iter();
        let subagents: Vec<&str> = subagents.map(|one| one.subagent.id.as_str()).collect();
        assert_eq!(subagents, ["a1", "b2", "b1"]);
    }
}
