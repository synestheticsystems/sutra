//! Where the map places an agent: on the card of the project its folder lies in, or, outside every
//! project, among the agents working elsewhere; and what one load places in each place.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::owner;
use super::tracked::Tracked;
use crate::map::{AgentSession, SubagentWithParent};

/// Where an agent goes on the map: on the card of the project its folder lies in (see `owner`), or,
/// when that is none, among the agents working elsewhere, beside the folder.
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
        let tracked = |path: &&str| Tracked::plain(PathBuf::from(path), &boundary);
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
