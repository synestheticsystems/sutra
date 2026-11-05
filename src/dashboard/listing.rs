//! What the list shows, in order: the projects in focus in the user's sort, the agents working
//! outside every project, and one row standing for the quiet projects, which opens to list them.
//! A typed filter lists every project it admits instead. Judged at one instant, so the cards and
//! the counts always agree.

mod arrangement;
mod filter;
mod focus;
mod hold;
mod sort;

pub(super) use arrangement::Arrangement;
pub(super) use focus::{FOCUS_LARGE_BYTES, FOCUS_RECENT};

use std::time::SystemTime;

use self::focus::in_focus;
use self::hold::{Hold, Order, Section};
use crate::map::{AgentSession, Project, Snapshot};

/// One row of the list.
pub(super) enum Row<'a> {
    Project(&'a Project),
    /// The agents working outside every project: all of them, or those the filter admits.
    Elsewhere(Vec<&'a AgentSession>),
    /// "▸ 764 quiet": the projects outside the focus, behind one row.
    QuietProjects {
        count: usize,
        shown: bool,
    },
}

/// The agents working outside every project that have something to show.
fn elsewhere(snapshot: &Snapshot) -> impl Iterator<Item = &AgentSession> {
    snapshot
        .unplaced_agents
        .iter()
        .filter(|agent| !agent.is_finished_job())
}

pub(super) struct Listing<'a> {
    pub(super) snapshot: &'a Snapshot,
    pub(super) rows: Vec<Row<'a>>,
}

impl<'a> Listing<'a> {
    /// The list as it shows now, held still where the arrangement holds it.
    pub(super) fn new(
        snapshot: &'a Snapshot,
        arrangement: &Arrangement,
        now: SystemTime,
    ) -> Listing<'a> {
        Listing::arranged(snapshot, arrangement, now, arrangement.hold())
    }

    /// The list as the sort alone orders it, with nothing held.
    fn sorted(snapshot: &'a Snapshot, arrangement: &Arrangement, now: SystemTime) -> Listing<'a> {
        Listing::arranged(snapshot, arrangement, now, &Hold::default())
    }

    fn arranged(
        snapshot: &'a Snapshot,
        arrangement: &Arrangement,
        now: SystemTime,
        hold: &Hold,
    ) -> Listing<'a> {
        let sort = arrangement.sort();
        let cards = |section, projects: Vec<&'a Project>| {
            hold.arrange(section, sort.arrange(projects))
                .into_iter()
                .map(Row::Project)
        };
        let mut rows = Vec::new();
        match arrangement.query() {
            Some(query) => {
                let admitted = snapshot.projects.iter().filter(|p| query.admits(p));
                rows.extend(cards(Section::Main, admitted.collect()));
                let elsewhere: Vec<&AgentSession> = elsewhere(snapshot)
                    .filter(|agent| query.admits_folder(&agent.cwd))
                    .collect();
                if !elsewhere.is_empty() {
                    rows.push(Row::Elsewhere(elsewhere));
                }
            }
            None => {
                let (focused, quiet): (Vec<&Project>, Vec<&Project>) = snapshot
                    .projects
                    .iter()
                    .partition(|project| in_focus(project, now));
                rows.extend(cards(Section::Main, focused));
                let elsewhere: Vec<&AgentSession> = elsewhere(snapshot).collect();
                if !elsewhere.is_empty() {
                    rows.push(Row::Elsewhere(elsewhere));
                }
                if !quiet.is_empty() {
                    let shown = arrangement.quiet_shown();
                    rows.push(Row::QuietProjects {
                        count: quiet.len(),
                        shown,
                    });
                    if shown {
                        rows.extend(cards(Section::Quiet, quiet));
                    }
                }
            }
        }
        Listing { snapshot, rows }
    }

    /// The cards of each section, in the order on screen.
    fn order(&self) -> Order {
        let mut order = Order::default();
        let mut past_quiet_row = false;
        for row in &self.rows {
            match row {
                Row::Project(project) if past_quiet_row => order.quiet.push(project.path.clone()),
                Row::Project(project) => order.main.push(project.path.clone()),
                Row::QuietProjects { .. } => past_quiet_row = true,
                Row::Elsewhere(_) => {}
            }
        }
        order
    }
}

/// Snapshots and projects for the listing's tests.
#[cfg(test)]
pub(super) mod fixtures {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use crate::map::test_support::{agent_session, at, project_at, snapshot_of};
    use crate::map::{AgentSession, AgentStatus, Project, Snapshot};

    pub(in crate::dashboard) fn now() -> SystemTime {
        at(1_000_000_000)
    }

    /// A project last active `minutes_ago`, in focus for that alone.
    pub(in crate::dashboard) fn project(name: &str, minutes_ago: u64) -> Project {
        Project {
            last_activity: Some(now() - Duration::from_secs(minutes_ago * 60)),
            ..quiet(name)
        }
    }

    /// A project out of focus: no activity known, and nothing in it.
    pub(in crate::dashboard) fn quiet(name: &str) -> Project {
        project_at(PathBuf::from("/code").join(name))
    }

    pub(in crate::dashboard) fn agent(cwd: &str, status: AgentStatus) -> AgentSession {
        AgentSession {
            cwd: PathBuf::from(cwd),
            ..agent_session(cwd, status)
        }
    }

    /// A map of `projects`, in the order given, which the engine delivers newest activity first.
    pub(in crate::dashboard) fn snapshot(
        projects: Vec<Project>,
        unplaced_agents: Vec<AgentSession>,
    ) -> Snapshot {
        Snapshot {
            unplaced_agents,
            ..snapshot_of(projects)
        }
    }

    /// What the rows show, one word each: a project's name, `elsewhere`, or `quiet:<count>`.
    pub(in crate::dashboard) fn shown(listing: &super::Listing) -> Vec<String> {
        listing
            .rows
            .iter()
            .map(|row| match row {
                super::Row::Project(project) => project.name.clone(),
                super::Row::Elsewhere(_) => "elsewhere".to_string(),
                super::Row::QuietProjects { count, .. } => format!("quiet:{count}"),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{agent, now, project, quiet, shown, snapshot};
    use super::*;
    use crate::map::{AgentStatus, Harness};

    #[test]
    fn the_list_shows_the_focused_projects_then_the_agents_elsewhere_then_the_quiet_row() {
        let map = snapshot(
            vec![project("recent", 1), quiet("old"), project("older", 60)],
            vec![agent("/tmp/scratch", AgentStatus::Busy)],
        );
        let listing = Listing::new(&map, &Arrangement::default(), now());
        assert_eq!(shown(&listing), ["recent", "older", "elsewhere", "quiet:1"]);
    }

    #[test]
    fn the_quiet_row_opens_to_list_the_quiet_projects_below_it() {
        let map = snapshot(vec![quiet("b"), project("recent", 1), quiet("a")], vec![]);
        let mut arrangement = Arrangement::default();
        arrangement.toggle_quiet(Some(&map), now());
        let listing = Listing::new(&map, &arrangement, now());
        assert_eq!(shown(&listing), ["recent", "quiet:2", "a", "b"]);
    }

    #[test]
    fn a_typed_filter_lists_every_project_it_admits_in_focus_or_not() {
        let map = snapshot(
            vec![project("app", 1), quiet("app-old"), project("site", 1)],
            vec![
                agent("/tmp/app-scratch", AgentStatus::Busy),
                agent("/tmp/other", AgentStatus::Busy),
            ],
        );
        let mut arrangement = Arrangement::default();
        arrangement.set_filter_text("APP".to_string(), Some(&map), now());
        let listing = Listing::new(&map, &arrangement, now());
        assert_eq!(shown(&listing), ["app", "app-old", "elsewhere"]);
        let Row::Elsewhere(agents) = &listing.rows[2] else {
            panic!("the agents elsewhere come last");
        };
        assert_eq!(agents.len(), 1, "only the agent whose folder matches");
    }

    #[test]
    fn a_finished_job_elsewhere_makes_no_row_and_a_running_one_does() {
        let job = |status| AgentSession {
            kind: Some("exec".to_string()),
            harness: Harness::Codex,
            ..agent("/tmp/review", status)
        };
        let rows = |map: &Snapshot, arrangement: &Arrangement| {
            Listing::new(map, arrangement, now()).rows.len()
        };
        let finished = snapshot(vec![], vec![job(AgentStatus::Idle)]);
        assert_eq!(rows(&finished, &Arrangement::default()), 0);
        let mut filtered = Arrangement::default();
        filtered.set_filter_text("review".to_string(), Some(&finished), now());
        assert_eq!(rows(&finished, &filtered), 0, "nor under a filter");
        let running = snapshot(vec![], vec![job(AgentStatus::Busy)]);
        assert_eq!(rows(&running, &Arrangement::default()), 1);
    }
}
