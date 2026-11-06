//! What the list shows, in order: the projects in focus in the user's sort, the agents working
//! outside every project, and one row standing for the quiet projects, which opens to list them.
//! A typed filter lists every project it admits instead. In the Activity sort, the worktrees of a
//! repository with two or more cards above the quiet row stand together in its group, under the
//! header that names it. Judged at one instant, so the cards and the counts always agree.

mod arrangement;
mod filter;
mod focus;
mod folds;
mod hold;
mod repository;
mod sort;

pub(super) use arrangement::Arrangement;
pub(super) use focus::{FOCUS_LARGE_BYTES, FOCUS_RECENT};
pub(super) use folds::Folds;
pub(super) use repository::Group;

use std::path::Path;
use std::time::SystemTime;

use self::focus::in_focus;
use self::hold::{Hold, Order, Section};
use crate::map::{AgentSession, Project, Snapshot, SubagentWithParent};

/// One row of the list.
pub(super) enum Row<'a> {
    Project(&'a Project),
    /// One repository's worktrees, under the header that names it: the Activity sort's group.
    Repository(Group<'a>),
    /// The agents working outside every project: all of them, or those the filter admits.
    Elsewhere(Elsewhere<'a>),
    /// "▸ 764 quiet": the projects outside the focus, behind one row.
    QuietProjects {
        count: usize,
        shown: bool,
    },
}

/// One row of a section of the list, as the sort lays the section out: a card standing alone, or
/// one repository's group of cards.
pub(super) enum SectionRow<'a> {
    Project(&'a Project),
    Repository(Group<'a>),
}

impl<'a> From<SectionRow<'a>> for Row<'a> {
    fn from(row: SectionRow<'a>) -> Row<'a> {
        match row {
            SectionRow::Project(project) => Row::Project(project),
            SectionRow::Repository(group) => Row::Repository(group),
        }
    }
}

/// The agents working outside every project that have something to show: the sessions, but for
/// jobs that have finished, and the subagents at work there.
pub(super) struct Elsewhere<'a> {
    pub(super) sessions: Vec<&'a AgentSession>,
    pub(super) subagents: Vec<&'a SubagentWithParent>,
}

impl<'a> Elsewhere<'a> {
    /// Those on `snapshot` working in a folder `admits` admits, `None` when that is none.
    fn of(snapshot: &'a Snapshot, admits: impl Fn(&Path) -> bool) -> Option<Elsewhere<'a>> {
        let sessions = snapshot.unplaced_agents.iter();
        let sessions = sessions.filter(|agent| !agent.is_finished_job() && admits(&agent.cwd));
        let subagents = snapshot.unplaced_subagents.iter();
        let subagents = subagents.filter(|working| admits(&working.subagent.cwd));
        let row = Elsewhere {
            sessions: sessions.collect(),
            subagents: subagents.collect(),
        };
        (!row.sessions.is_empty() || !row.subagents.is_empty()).then_some(row)
    }
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
            let sorted = match section {
                Section::Main => sort.lay_out(projects, snapshot),
                // No header stands below the quiet row, so no repository has two headers, nor two
                // groups that one click on a header would fold: one with worktrees on both sides
                // of the row has its group above it, or none.
                Section::Quiet => {
                    let arranged = sort.arrange(projects).into_iter();
                    arranged.map(SectionRow::Project).collect()
                }
            };
            hold.arrange(section, sorted).into_iter().map(Row::from)
        };
        let mut rows = Vec::new();
        match arrangement.query() {
            Some(query) => {
                let admitted = snapshot.projects.iter().filter(|p| query.admits(p));
                rows.extend(cards(Section::Main, admitted.collect()));
                let elsewhere = Elsewhere::of(snapshot, |folder| query.admits_folder(folder));
                rows.extend(elsewhere.map(Row::Elsewhere));
            }
            None => {
                let (focused, quiet): (Vec<&Project>, Vec<&Project>) = snapshot
                    .projects
                    .iter()
                    .partition(|project| in_focus(project, now));
                rows.extend(cards(Section::Main, focused));
                rows.extend(Elsewhere::of(snapshot, |_| true).map(Row::Elsewhere));
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

    /// The cards of each section, in the order on screen, with the cards of each group folded shut
    /// where its header stands: the listing lays a group out whether the window shows its cards or
    /// not, so a folded group holds its place as an open one does.
    fn order(&self) -> Order {
        let mut order = Order::default();
        let mut past_quiet_row = false;
        for row in &self.rows {
            let section = if past_quiet_row {
                &mut order.quiet
            } else {
                &mut order.main
            };
            match row {
                Row::Project(project) => section.push(project.path.clone()),
                Row::Repository(group) => {
                    section.extend(group.cards.iter().map(|card| card.path.clone()));
                }
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

    use crate::map::test_support::{agent_session, at, project_at, snapshot_of, subagent_at_work};
    use crate::map::{
        AgentSession, AgentStatus, Project, RepoInfo, Snapshot, Subagent, SubagentWithParent,
    };

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

    /// `project` as the main worktree of the repository whose git dir is `common_dir`, on
    /// `branch`.
    pub(in crate::dashboard) fn main_worktree(
        common_dir: &str,
        branch: &str,
        project: Project,
    ) -> Project {
        let common_dir = PathBuf::from(common_dir);
        Project {
            repo: Some(RepoInfo {
                git_dir: common_dir.clone(),
                common_dir,
                worktree_name: None,
                branch: Some(branch.to_string()),
                head: None,
            }),
            ..project
        }
    }

    /// `project` as a worktree linked to the repository whose git dir is `common_dir`, under its
    /// folder's name, on `branch`.
    pub(in crate::dashboard) fn linked_worktree(
        common_dir: &str,
        branch: &str,
        project: Project,
    ) -> Project {
        let common_dir = PathBuf::from(common_dir);
        Project {
            repo: Some(RepoInfo {
                git_dir: common_dir.join("worktrees").join(&project.name),
                common_dir,
                worktree_name: Some(project.name.clone()),
                branch: Some(branch.to_string()),
                head: None,
            }),
            ..project
        }
    }

    pub(in crate::dashboard) fn agent(cwd: &str, status: AgentStatus) -> AgentSession {
        AgentSession {
            cwd: PathBuf::from(cwd),
            ..agent_session(cwd, status)
        }
    }

    /// A subagent at work in `cwd` for an idle session in `/code/app`.
    pub(in crate::dashboard) fn subagent_in(cwd: &str) -> SubagentWithParent {
        let subagent = Subagent {
            cwd: PathBuf::from(cwd),
            ..subagent_at_work(cwd)
        };
        SubagentWithParent::new(subagent, &agent("/code/app", AgentStatus::Idle))
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

    /// What `rows` show, a few words each: a project's name, a repository's group as
    /// `name: card, card`, `elsewhere`, or `quiet:<count>`.
    pub(in crate::dashboard) fn shown(rows: &[super::Row]) -> Vec<String> {
        let names = |projects: &[&Project]| {
            let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
            names.join(", ")
        };
        rows.iter()
            .map(|row| match row {
                super::Row::Project(project) => project.name.clone(),
                super::Row::Repository(group) => format!("{}: {}", group.name, names(&group.cards)),
                super::Row::Elsewhere(_) => "elsewhere".to_string(),
                super::Row::QuietProjects { count, .. } => format!("quiet:{count}"),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{
        agent, linked_worktree, main_worktree, now, project, quiet, shown, snapshot, subagent_in,
    };
    use super::*;
    use crate::map::{AgentStatus, Harness};

    #[test]
    fn the_activity_sort_groups_two_or_more_worktrees_above_the_quiet_row_and_none_below_it() {
        let (app, site, old) = ("/code/app/.git", "/code/site/.git", "/code/old/.git");
        let map = snapshot(
            vec![
                linked_worktree(app, "fix", project("app-fix", 1)),
                main_worktree(site, "main", project("site", 1)),
                main_worktree(app, "main", project("app", 30)),
                linked_worktree(site, "wip", quiet("site-wip")),
                main_worktree(old, "main", quiet("old")),
                linked_worktree(old, "wip", quiet("old-wip")),
            ],
            vec![agent("/tmp/scratch", AgentStatus::Busy)],
        );
        let mut arrangement = Arrangement::default();
        arrangement.toggle_quiet(Some(&map), now());
        let listing = Listing::new(&map, &arrangement, now());
        assert_eq!(
            shown(&listing.rows),
            [
                "app: app, app-fix",
                "site",
                "elsewhere",
                "quiet:3",
                "old",
                "old-wip",
                "site-wip"
            ],
            "site has one card on each side of the row, and old's two stand alone below it"
        );
    }

    #[test]
    fn under_a_filter_the_activity_sort_groups_every_worktree_it_admits() {
        let app = "/code/app/.git";
        let map = snapshot(
            vec![
                main_worktree(app, "main", quiet("app")),
                linked_worktree(app, "fix", project("app-fix", 1)),
                project("site", 1),
            ],
            vec![],
        );
        let mut arrangement = Arrangement::default();
        arrangement.set_filter_text("app".to_string(), Some(&map), now());
        let listing = Listing::new(&map, &arrangement, now());
        assert_eq!(shown(&listing.rows), ["app: app, app-fix"]);
    }

    #[test]
    fn the_list_shows_the_focused_projects_then_the_agents_elsewhere_then_the_quiet_row() {
        let map = snapshot(
            vec![project("recent", 1), quiet("old"), project("older", 60)],
            vec![agent("/tmp/scratch", AgentStatus::Busy)],
        );
        let listing = Listing::new(&map, &Arrangement::default(), now());
        assert_eq!(
            shown(&listing.rows),
            ["recent", "older", "elsewhere", "quiet:1"]
        );
    }

    #[test]
    fn the_quiet_row_opens_to_list_the_quiet_projects_below_it() {
        let map = snapshot(vec![quiet("b"), project("recent", 1), quiet("a")], vec![]);
        let mut arrangement = Arrangement::default();
        arrangement.toggle_quiet(Some(&map), now());
        let listing = Listing::new(&map, &arrangement, now());
        assert_eq!(shown(&listing.rows), ["recent", "quiet:2", "a", "b"]);
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
        assert_eq!(shown(&listing.rows), ["app", "app-old", "elsewhere"]);
        let Row::Elsewhere(agents) = &listing.rows[2] else {
            panic!("the agents elsewhere come last");
        };
        assert_eq!(
            agents.sessions.len(),
            1,
            "only the agent whose folder matches"
        );
    }

    #[test]
    fn a_subagent_at_work_in_no_project_makes_the_elsewhere_row_and_a_filter_finds_its_folder() {
        let map = Snapshot {
            unplaced_subagents: vec![subagent_in("/tmp/build"), subagent_in("/tmp/other")],
            ..snapshot(vec![project("app", 1)], vec![])
        };
        let listing = Listing::new(&map, &Arrangement::default(), now());
        assert_eq!(shown(&listing.rows), ["app", "elsewhere"]);

        let mut filtered = Arrangement::default();
        filtered.set_filter_text("build".to_string(), Some(&map), now());
        let listing = Listing::new(&map, &filtered, now());
        let [Row::Elsewhere(working)] = listing.rows.as_slice() else {
            panic!("only the agents elsewhere match");
        };
        let subagents = working.subagents.iter();
        let folders: Vec<&Path> = subagents.map(|one| one.subagent.cwd.as_path()).collect();
        assert_eq!(folders, [Path::new("/tmp/build")]);
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
