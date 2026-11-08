//! What the list shows, in order: the projects in focus in the user's sort (see `focus`), with
//! Docker's card among them where the sort puts it, the agents working outside every project, and
//! one row standing for the quiet projects, which opens to list them; of a session's scratch
//! folders, those with something to show (see `scratch_rows`). A typed filter lists every project
//! it admits instead. In the Activity sort, the cards of a parent above the quiet row, a
//! repository's worktrees, two or more, or a session's scratch folders, however few, stand
//! together in its group, under the line that names it (see `group`). Judged at one instant, so
//! the cards and the counts always agree.

mod arrangement;
mod filter;
mod focus;
mod folds;
mod group;
mod hold;
mod scratch_rows;
mod sort;

pub(super) use arrangement::Arrangement;
pub(super) use focus::FOCUS_RECENT;
pub(super) use folds::Folds;
pub(super) use group::{Group, Parent, nested_name};

use std::path::Path;
use std::time::SystemTime;

use self::focus::{docker_in_focus, in_focus};
use self::hold::{Hold, Order, Section};
use self::scratch_rows::has_a_row;
use crate::dashboard::SortOrder;
use crate::dashboard::cleaning::Cleaning;
use crate::map::{AgentSession, Docker, Project, Snapshot, SubagentWithParent};

/// What Docker's card goes by, as the Name sort and the filter read it too.
pub(super) const DOCKER: &str = "Docker";

/// One row of the list.
pub(super) enum Row<'a> {
    Project(&'a Project),
    /// The cards that share a parent, under the line that names it: the Activity sort's group.
    Group(Group<'a>),
    /// Docker, on a card of its own: the containers it runs for no project, and its disk use.
    Docker(&'a Docker),
    /// The agents working outside every project: all of them, or those the filter admits.
    Elsewhere(Elsewhere<'a>),
    /// "▸ 764 quiet": the projects outside the focus, behind one row.
    QuietProjects {
        count: usize,
        shown: bool,
    },
}

/// One row of a section of the list, as the sort lays the section out: a card standing alone, the
/// group of the cards that share a parent, or Docker's card.
pub(super) enum SectionRow<'a> {
    Project(&'a Project),
    Group(Group<'a>),
    Docker(&'a Docker),
}

impl<'a> From<SectionRow<'a>> for Row<'a> {
    fn from(row: SectionRow<'a>) -> Row<'a> {
        match row {
            SectionRow::Project(project) => Row::Project(project),
            SectionRow::Group(group) => Row::Group(group),
            SectionRow::Docker(docker) => Row::Docker(docker),
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
        let subagents = subagents.filter(|working| admits(working.subagent.works_in()));
        let row = Elsewhere {
            sessions: sessions.collect(),
            subagents: subagents.collect(),
        };
        (!row.sessions.is_empty() || !row.subagents.is_empty()).then_some(row)
    }
}

pub(super) struct Listing<'a> {
    pub(super) snapshot: &'a Snapshot,
    /// The sort the rows are laid out in, which also decides what is in focus.
    pub(super) sort: SortOrder,
    pub(super) rows: Vec<Row<'a>>,
}

impl<'a> Listing<'a> {
    /// The list as it shows now, held still where the arrangement holds it, with `cleaning` at
    /// work.
    pub(super) fn new(
        snapshot: &'a Snapshot,
        arrangement: &Arrangement,
        cleaning: &Cleaning,
        now: SystemTime,
    ) -> Listing<'a> {
        Listing::arranged(snapshot, arrangement, cleaning, now, arrangement.hold())
    }

    /// The list as the sort alone orders it, with nothing held.
    fn sorted(
        snapshot: &'a Snapshot,
        arrangement: &Arrangement,
        cleaning: &Cleaning,
        now: SystemTime,
    ) -> Listing<'a> {
        Listing::arranged(snapshot, arrangement, cleaning, now, &Hold::default())
    }

    fn arranged(
        snapshot: &'a Snapshot,
        arrangement: &Arrangement,
        cleaning: &Cleaning,
        now: SystemTime,
        hold: &Hold,
    ) -> Listing<'a> {
        let sort = arrangement.sort();
        let with_rows = snapshot.projects.iter();
        let with_rows = with_rows.filter(|project| has_a_row(project, cleaning));
        let cards = |section, projects: Vec<&'a Project>, docker: Option<&'a Docker>| {
            let mut sorted = match section {
                Section::Main => sort.lay_out(projects, snapshot),
                // No line stands below the quiet row, so no parent has two lines, nor two groups
                // that one click on a line would fold: one with cards on both sides of the row
                // has its group above it, or none.
                Section::Quiet => {
                    let arranged = sort.arrange(projects).into_iter();
                    arranged.map(SectionRow::Project).collect()
                }
            };
            if let Some(docker) = docker {
                sort.place_docker(&mut sorted, docker);
            }
            hold.arrange(section, sorted).into_iter().map(Row::from)
        };
        let docker = snapshot.docker.as_ref();
        let mut rows = Vec::new();
        match arrangement.query() {
            Some(query) => {
                let admitted = with_rows.filter(|p| query.admits(p));
                let docker = docker.filter(|docker| query.admits_docker(docker));
                rows.extend(cards(Section::Main, admitted.collect(), docker));
                let elsewhere = Elsewhere::of(snapshot, |folder| query.admits_folder(folder));
                rows.extend(elsewhere.map(Row::Elsewhere));
            }
            None => {
                let (focused, quiet): (Vec<&Project>, Vec<&Project>) =
                    with_rows.partition(|project| in_focus(project, sort, now));
                let (docker_focused, docker_quiet) = match docker {
                    Some(docker) if docker_in_focus(docker, sort) => (Some(docker), None),
                    docker => (None, docker),
                };
                rows.extend(cards(Section::Main, focused, docker_focused));
                rows.extend(Elsewhere::of(snapshot, |_| true).map(Row::Elsewhere));
                let count = quiet.len() + usize::from(docker_quiet.is_some());
                if count > 0 {
                    let shown = arrangement.quiet_shown();
                    rows.push(Row::QuietProjects { count, shown });
                    if shown {
                        rows.extend(cards(Section::Quiet, quiet, docker_quiet));
                    }
                }
            }
        }
        Listing {
            snapshot,
            sort,
            rows,
        }
    }

    /// The cards of each section, in the order on screen, with the cards of each group folded shut
    /// where its line stands: the listing lays a group out whether the window shows its cards or
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
                Row::Group(group) => {
                    section.extend(group.cards.iter().map(|card| card.path.clone()));
                }
                // Held by the socket it is read through, which is no project's folder.
                Row::Docker(docker) => section.push(docker.socket.clone()),
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

    use crate::map::docker::{Container, DockerDisk, Store};
    use crate::map::test_support::{agent_session, at, project_at, snapshot_of, subagent_at_work};
    use crate::map::{
        AgentSession, AgentStatus, Docker, Project, RepoInfo, ScratchSpace, Snapshot, Subagent,
        SubagentWithParent,
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

    /// `project` as the scratch folder `folder` of the session `session_id`, which started in
    /// `/code/app` and has ended, never named, so it goes by the name the engine gives it, as
    /// `app e26ca359` reads.
    pub(in crate::dashboard) fn scratch_folder(
        session_id: &str,
        folder: &str,
        project: Project,
    ) -> Project {
        let session_folder = PathBuf::from("/private/tmp/claude-501/-code-app").join(session_id);
        let space = ScratchSpace {
            session_folder,
            session_id: session_id.to_string(),
            started_in: "app".to_string(),
            session: None,
            session_name: None,
        };
        Project {
            path: space.session_folder.join("scratchpad").join(folder),
            name: folder.to_string(),
            scratch: Some(space),
            ..project
        }
    }

    /// `scratch`, a scratch folder, while its session runs under the name `label`.
    pub(in crate::dashboard) fn running_as(label: &str, scratch: Project) -> Project {
        let mut space = scratch.scratch.expect("a scratch folder");
        space.session = Some(AgentSession {
            label: Some(label.to_string()),
            ..agent_session(&space.session_id, AgentStatus::Busy)
        });
        space.session_name = Some(label.to_string());
        Project {
            scratch: Some(space),
            ..scratch
        }
    }

    pub(in crate::dashboard) fn agent(cwd: &str, status: AgentStatus) -> AgentSession {
        AgentSession {
            cwd: PathBuf::from(cwd),
            ..agent_session(cwd, status)
        }
    }

    /// A subagent at work in `cwd`, run there and acting nowhere else, for an idle session in
    /// `/code/app`.
    pub(in crate::dashboard) fn subagent_in(cwd: &str) -> SubagentWithParent {
        let subagent = Subagent {
            cwd: PathBuf::from(cwd),
            ..subagent_at_work(cwd)
        };
        SubagentWithParent::new(subagent, &agent("/code/app", AgentStatus::Idle))
    }

    /// Docker read at `/run/docker.sock`, running `containers` for no project, made `minutes_ago`
    /// each, and taking `bytes` on disk.
    pub(in crate::dashboard) fn docker(containers: &[(&str, u64)], bytes: u64) -> Docker {
        let store = |bytes| Store {
            bytes,
            count: 1,
            in_use: 1,
            reclaimable: 0,
        };
        let container = |&(name, minutes_ago): &(&str, u64)| Container {
            id: name.to_string(),
            name: name.to_string(),
            image: "postgres:17".to_string(),
            compose_folder: None,
            bound: Vec::new(),
            ports: Vec::new(),
            created: now() - Duration::from_secs(minutes_ago * 60),
        };
        Docker {
            socket: PathBuf::from("/run/docker.sock"),
            containers: containers.iter().map(container).collect(),
            disk: Some(DockerDisk {
                images: store(bytes),
                containers: store(0),
                volumes: store(0),
                build_cache: store(0),
                read_at: now(),
            }),
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

    /// What `rows` show, a few words each: a project's name, a group as `name: card, card`, each
    /// card by its folder's own name, `docker`, `elsewhere`, or `quiet:<count>`.
    pub(in crate::dashboard) fn shown(rows: &[super::Row]) -> Vec<String> {
        let names = |projects: &[&Project]| {
            let folder = |project: &&Project| {
                let name = project.path.file_name().unwrap();
                name.to_string_lossy().into_owned()
            };
            let names: Vec<String> = projects.iter().map(folder).collect();
            names.join(", ")
        };
        rows.iter()
            .map(|row| match row {
                super::Row::Project(project) => project.name.clone(),
                super::Row::Group(group) => format!("{}: {}", group.name, names(&group.cards)),
                super::Row::Docker(_) => "docker".to_string(),
                super::Row::Elsewhere(_) => "elsewhere".to_string(),
                super::Row::QuietProjects { count, .. } => format!("quiet:{count}"),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::fixtures::{
        agent, docker, linked_worktree, main_worktree, now, project, quiet, shown, snapshot,
        subagent_in,
    };
    use super::*;
    use crate::dashboard::cleaning::tests::{cleaning, deleted};
    use crate::map::test_support::measured;
    use crate::map::{AgentStatus, Harness, LARGE_FOLDER_BYTES};

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
        arrangement.toggle_quiet(Some(&map), &cleaning(), now());
        let listing = Listing::new(&map, &arrangement, &cleaning(), now());
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
        arrangement.set_filter_text("app".to_string(), Some(&map), &cleaning(), now());
        let listing = Listing::new(&map, &arrangement, &cleaning(), now());
        assert_eq!(shown(&listing.rows), ["app: app, app-fix"]);
    }

    #[test]
    fn the_list_shows_the_focused_projects_then_the_agents_elsewhere_then_the_quiet_row() {
        let map = snapshot(
            vec![project("recent", 1), quiet("old"), project("older", 60)],
            vec![agent("/tmp/scratch", AgentStatus::Busy)],
        );
        let listing = Listing::new(&map, &Arrangement::default(), &cleaning(), now());
        assert_eq!(
            shown(&listing.rows),
            ["recent", "older", "elsewhere", "quiet:1"]
        );
    }

    #[test]
    fn the_scratch_of_a_session_that_ended_waits_behind_the_quiet_row_in_the_activity_sort_alone() {
        use super::fixtures::{running_as, scratch_folder};
        let sized = |gigabytes: u64, project: Project| Project {
            own_files: measured(gigabytes * LARGE_FOLDER_BYTES),
            ..project
        };
        let map = snapshot(
            vec![
                sized(12, scratch_folder("d5fa0639", "sim-target", project("", 1))),
                sized(
                    2,
                    running_as(
                        "Foundry",
                        scratch_folder("e26ca359", "r567", project("", 1)),
                    ),
                ),
                sized(3, quiet("forgotten")),
            ],
            vec![],
        );
        let listed = |sort| {
            let mut arrangement = Arrangement::default();
            arrangement.set_sort(sort, &map, &cleaning(), now());
            arrangement.toggle_quiet(Some(&map), &cleaning(), now());
            shown(&Listing::new(&map, &arrangement, &cleaning(), now()).rows)
        };
        assert_eq!(
            listed(SortOrder::Activity),
            ["Foundry: r567", "quiet:2", "sim-target", "forgotten"],
            "its activity a minute ago, and its size, are disk news"
        );
        assert_eq!(
            listed(SortOrder::Disk),
            ["sim-target", "forgotten", "r567"],
            "in focus where the sort is for disk"
        );
    }

    #[test]
    fn a_scratch_folder_with_nothing_to_show_has_no_row_and_its_sessions_line_counts_it_still() {
        use super::fixtures::{running_as, scratch_folder};
        let foundry = |folder, own_files| Project {
            own_files: measured(own_files),
            ..running_as(
                "Foundry",
                scratch_folder("e26ca359", folder, project("", 1)),
            )
        };
        let map = snapshot(
            vec![
                foundry("r567guards", 2 * LARGE_FOLDER_BYTES),
                foundry("r567fixture", 28_000_000),
                project("notes", 1),
            ],
            vec![],
        );
        let listing = Listing::new(&map, &Arrangement::default(), &cleaning(), now());
        assert_eq!(shown(&listing.rows), ["Foundry: r567guards", "notes"]);
        let Row::Group(group) = &listing.rows[0] else {
            panic!("the session's line");
        };
        assert_eq!(group.members.len(), 2, "both folders add to its total");

        let mut filtered = Arrangement::default();
        filtered.set_filter_text("r567".to_string(), Some(&map), &cleaning(), now());
        let listing = Listing::new(&map, &filtered, &cleaning(), now());
        assert_eq!(
            shown(&listing.rows),
            ["Foundry: r567guards"],
            "a filter finds no row the list does not have"
        );

        // The folder `deleted()` has just cleaned, `/code/app/target`, is r567fixture's.
        let mut cleaned = map.clone();
        let target = crate::map::test_support::cargo_target("/code/app/target", measured(10));
        cleaned.projects[1].artifacts.push(target);
        let listing = Listing::new(&cleaned, &Arrangement::default(), &deleted(), now());
        assert_eq!(
            shown(&listing.rows),
            ["Foundry: r567fixture, r567guards", "notes"],
            "a clean's bar shows on its row until the map measures the folder again"
        );
    }

    #[test]
    fn the_quiet_row_opens_to_list_the_quiet_projects_below_it() {
        let map = snapshot(vec![quiet("b"), project("recent", 1), quiet("a")], vec![]);
        let mut arrangement = Arrangement::default();
        arrangement.toggle_quiet(Some(&map), &cleaning(), now());
        let listing = Listing::new(&map, &arrangement, &cleaning(), now());
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
        arrangement.set_filter_text("APP".to_string(), Some(&map), &cleaning(), now());
        let listing = Listing::new(&map, &arrangement, &cleaning(), now());
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
        let mut builder = subagent_in("/tmp/scratch");
        builder.subagent.acted_in = vec![PathBuf::from("/tmp/build")];
        let map = Snapshot {
            unplaced_subagents: vec![builder, subagent_in("/tmp/other")],
            ..snapshot(vec![project("app", 1)], vec![])
        };
        let listing = Listing::new(&map, &Arrangement::default(), &cleaning(), now());
        assert_eq!(shown(&listing.rows), ["app", "elsewhere"]);

        let filtered = |text: &str| {
            let mut filtered = Arrangement::default();
            filtered.set_filter_text(text.to_string(), Some(&map), &cleaning(), now());
            filtered
        };
        let listing = Listing::new(&map, &filtered("build"), &cleaning(), now());
        let [Row::Elsewhere(working)] = listing.rows.as_slice() else {
            panic!("only the agents elsewhere match");
        };
        let subagents = working.subagents.iter();
        let folders: Vec<&Path> = subagents.map(|one| one.subagent.works_in()).collect();
        assert_eq!(folders, [Path::new("/tmp/build")], "the folder it acted in");
        let listing = Listing::new(&map, &filtered("scratch"), &cleaning(), now());
        assert_eq!(shown(&listing.rows), [""; 0], "not the one it runs in");
    }

    /// The rows `map` lists in `sort`, the quiet row open.
    fn listed_in(map: &Snapshot, sort: SortOrder) -> Vec<String> {
        let mut arrangement = Arrangement::default();
        arrangement.set_sort(sort, map, &cleaning(), now());
        arrangement.toggle_quiet(Some(map), &cleaning(), now());
        shown(&Listing::new(map, &arrangement, &cleaning(), now()).rows)
    }

    #[test]
    fn dockers_card_stands_among_the_cards_where_each_sort_puts_it() {
        let map = Snapshot {
            docker: Some(docker(&[("pg596", 12)], 10 * LARGE_FOLDER_BYTES)),
            ..snapshot(
                vec![
                    Project {
                        own_files: measured(20 * LARGE_FOLDER_BYTES),
                        ..project("api", 1)
                    },
                    Project {
                        own_files: measured(LARGE_FOLDER_BYTES),
                        ..project("site", 30)
                    },
                ],
                vec![],
            )
        };
        assert_eq!(
            listed_in(&map, SortOrder::Activity),
            ["api", "docker", "site"],
            "its newest container, made 12 minutes ago, between them"
        );
        assert_eq!(listed_in(&map, SortOrder::Name), ["api", "docker", "site"]);
        assert_eq!(listed_in(&map, SortOrder::Disk), ["api", "docker", "site"]);
    }

    #[test]
    fn docker_with_no_container_on_its_card_is_quiet_in_the_activity_sort_and_disk_news_elsewhere()
    {
        let map = Snapshot {
            docker: Some(docker(&[], 10 * LARGE_FOLDER_BYTES)),
            ..snapshot(vec![project("api", 1)], vec![])
        };
        assert_eq!(
            listed_in(&map, SortOrder::Activity),
            ["api", "quiet:1", "docker"]
        );
        assert_eq!(listed_in(&map, SortOrder::Disk), ["docker", "api"]);
        let small = Snapshot {
            docker: Some(docker(&[], LARGE_FOLDER_BYTES - 1)),
            ..map.clone()
        };
        assert_eq!(
            listed_in(&small, SortOrder::Disk),
            ["api", "quiet:1", "docker"]
        );
        let away = Snapshot {
            docker: None,
            ..map
        };
        assert_eq!(
            listed_in(&away, SortOrder::Activity),
            ["api"],
            "Docker not here"
        );
    }

    #[test]
    fn a_typed_filter_lists_dockers_card_by_its_name_or_a_containers_name_or_image() {
        let map = Snapshot {
            docker: Some(docker(&[("pg596", 12)], 0)),
            ..snapshot(vec![project("api", 1)], vec![])
        };
        let filtered = |text: &str| {
            let mut arrangement = Arrangement::default();
            arrangement.set_filter_text(text.to_string(), Some(&map), &cleaning(), now());
            shown(&Listing::new(&map, &arrangement, &cleaning(), now()).rows)
        };
        assert_eq!(filtered("dock"), ["docker"]);
        assert_eq!(filtered("pg59"), ["docker"]);
        assert_eq!(filtered("postgres"), ["docker"], "its image");
        assert_eq!(filtered("api"), ["api"]);
    }

    #[test]
    fn a_finished_job_elsewhere_makes_no_row_and_a_running_one_does() {
        let job = |status| AgentSession {
            kind: Some("exec".to_string()),
            harness: Harness::Codex,
            ..agent("/tmp/review", status)
        };
        let rows = |map: &Snapshot, arrangement: &Arrangement| {
            Listing::new(map, arrangement, &cleaning(), now())
                .rows
                .len()
        };
        let finished = snapshot(vec![], vec![job(AgentStatus::Idle)]);
        assert_eq!(rows(&finished, &Arrangement::default()), 0);
        let mut filtered = Arrangement::default();
        filtered.set_filter_text("review".to_string(), Some(&finished), &cleaning(), now());
        assert_eq!(rows(&finished, &filtered), 0, "nor under a filter");
        let running = snapshot(vec![], vec![job(AgentStatus::Busy)]);
        assert_eq!(rows(&running, &Arrangement::default()), 1);
    }
}
