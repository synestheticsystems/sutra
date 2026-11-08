//! What the list shows, in order: the groups in focus in the user's sort (see `focus`), each under
//! its line, then one row standing for the quiet groups, which opens to list them. Every card stands
//! in the group of the upstream its project came from, in a tree of provenance (see `group`), and
//! the agents working outside every project stand in Other. Docker's card stands in a group of its
//! own, which the sort places among the others: it came from no upstream, and what it holds, the
//! containers for no project and the disk Docker takes, is no project's. A group is in focus while
//! any card in it is, so each stands wholly on one side of the quiet row, and no line stands twice.
//! Of a session's scratch folders, those with something to show have a card (see `scratch_rows`). A
//! typed filter lists every project it admits instead. Judged at one instant, so the cards and the
//! counts always agree.

mod arrangement;
mod by_line;
mod filter;
#[cfg(test)]
pub(super) mod fixtures;
mod focus;
mod folds;
mod group;
mod hold;
mod scratch_rows;
mod sort;

pub(super) use arrangement::Arrangement;
pub(super) use by_line::ByLine;
pub(super) use focus::FOCUS_RECENT;
pub(super) use folds::Folds;
pub(super) use group::{Checkout, Group, Naming, Node};

use std::path::Path;
use std::time::SystemTime;

use self::focus::docker_in_focus;
use self::group::Members;
use self::hold::{Hold, Order, Section};
use self::scratch_rows::has_a_row;
use crate::dashboard::cleaning::Cleaning;
use crate::dashboard::{LineKey, SortOrder};
use crate::map::{AgentSession, Docker, Project, Snapshot, SubagentWithParent};

/// What Docker's card and its group's line go by, as the Name sort and the filter read it too.
pub(super) const DOCKER: &str = "Docker";

/// One row of the list.
pub(super) enum Row<'a> {
    /// A group, under its line.
    Group(Group<'a>),
    /// Docker's group, under its line: its card alone, with the containers it runs for no
    /// project, and its disk use.
    Docker(&'a Docker),
    /// "▸ 764 quiet": the projects of the groups outside the focus, behind one row.
    QuietProjects { count: usize, shown: bool },
}

/// One row of a section of the list, as the sort lays the section out: a group, or Docker's.
pub(super) enum SectionRow<'a> {
    Group(Group<'a>),
    Docker(&'a Docker),
}

impl<'a> From<SectionRow<'a>> for Row<'a> {
    fn from(row: SectionRow<'a>) -> Row<'a> {
        match row {
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
        let with_rows: Vec<&Project> = snapshot
            .projects
            .iter()
            .filter(|project| has_a_row(project, cleaning))
            .collect();
        let members = Members::of(snapshot);
        let docker = snapshot.docker.as_ref();
        let names = group::names_of(&with_rows, docker.is_some());
        let section = |section, projects, elsewhere, docker| {
            let gathered = group::gather(projects, elsewhere, &members, &names, sort);
            let laid_out = sort.lay_out(gathered, docker);
            hold.arrange(section, laid_out).into_iter().map(Row::from)
        };
        let mut rows = Vec::new();
        match arrangement.query() {
            Some(query) => {
                let admitted = with_rows.into_iter().filter(|p| query.admits(p)).collect();
                let elsewhere = Elsewhere::of(snapshot, |folder| query.admits_folder(folder));
                let docker = docker.filter(|docker| query.admits_docker(docker));
                rows.extend(section(Section::Main, admitted, elsewhere, docker));
            }
            None => {
                let elsewhere = Elsewhere::of(snapshot, |_| true);
                let (focused, quiet) =
                    focus::split_by_group(with_rows, sort, now, elsewhere.is_some());
                let (docker_focused, docker_quiet) = match docker {
                    Some(docker) if docker_in_focus(docker, sort) => (Some(docker), None),
                    docker => (None, docker),
                };
                rows.extend(section(Section::Main, focused, elsewhere, docker_focused));
                let count = quiet.len() + usize::from(docker_quiet.is_some());
                if count > 0 {
                    let shown = arrangement.quiet_shown();
                    rows.push(Row::QuietProjects { count, shown });
                    if shown {
                        rows.extend(section(Section::Quiet, quiet, None, docker_quiet));
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

    /// What the line of `line` holds on the map, listed or not, while the list shows that line: a
    /// group's members, or a checkout's.
    pub(super) fn members_of(&self, line: &LineKey) -> Option<&[&'a Project]> {
        fn in_rows<'r, 'a>(rows: &'r [Node<'a>], line: &LineKey) -> Option<&'r [&'a Project]> {
            rows.iter().find_map(|row| match row {
                Node::Checkout(checkout) if LineKey::checkout(&checkout.project.path) == *line => {
                    Some(checkout.members.as_slice())
                }
                Node::Checkout(checkout) => in_rows(&checkout.children, line),
                Node::Card { .. } | Node::Elsewhere(_) => None,
            })
        }
        self.rows.iter().find_map(|row| match row {
            Row::Group(group) if LineKey::group(group.key) == *line => {
                Some(group.members.as_slice())
            }
            Row::Group(group) => in_rows(&group.children, line),
            Row::Docker(_) | Row::QuietProjects { .. } => None,
        })
    }

    /// The cards of each section, in the order on screen, with the cards under each line folded
    /// shut where it stands: the listing lays a group out whether the window shows its cards or
    /// not, so a folded line holds its place as an open one does.
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
                Row::Group(group) => {
                    let cards = group.cards().into_iter();
                    section.extend(cards.map(|card| card.path.clone()));
                }
                // Held by the socket it is read through, which is no project's folder.
                Row::Docker(docker) => section.push(docker.socket.clone()),
                Row::QuietProjects { .. } => past_quiet_row = true,
            }
        }
        order
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::fixtures::{
        agent, came_from, docker, linked_worktree, main_worktree, now, project, quiet, running_as,
        scratch_folder, shown, snapshot, subagent_in, upstream,
    };
    use super::*;
    use crate::dashboard::cleaning::tests::{cleaning, deleted};
    use crate::map::test_support::measured;
    use crate::map::{AgentStatus, Harness, LARGE_FOLDER_BYTES};

    /// The rows `map` lists as `arrangement` arranges it.
    fn listed(map: &Snapshot, arrangement: &Arrangement) -> Vec<String> {
        shown(&Listing::new(map, arrangement, &cleaning(), now()).rows)
    }

    #[test]
    fn a_group_stands_wholly_above_the_quiet_row_while_any_card_in_it_is_in_focus() {
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
        assert_eq!(
            listed(&map, &arrangement),
            [
                "app: app, app-fix",
                "site: site, site-wip",
                "Other: elsewhere",
                "quiet:2",
                "old: old, old-wip",
            ],
            "site's quiet worktree stands in its group, and old's two in theirs below the row"
        );
    }

    #[test]
    fn a_typed_filter_lists_every_project_it_admits_in_its_group_in_focus_or_not() {
        let app = "/code/app/.git";
        let map = snapshot(
            vec![
                main_worktree(app, "main", quiet("app")),
                linked_worktree(app, "fix", project("app-fix", 1)),
                project("site", 1),
                linked_worktree(app, "old", quiet("app-old")),
            ],
            vec![
                agent("/tmp/app-scratch", AgentStatus::Busy),
                agent("/tmp/other", AgentStatus::Busy),
            ],
        );
        let mut arrangement = Arrangement::default();
        arrangement.set_filter_text("APP".to_string(), Some(&map), &cleaning(), now());
        let listing = Listing::new(&map, &arrangement, &cleaning(), now());
        assert_eq!(
            shown(&listing.rows),
            ["app: app, app-fix, app-old", "Other: elsewhere"]
        );
        let Row::Group(other) = &listing.rows[1] else {
            panic!("Other's group");
        };
        let [Node::Elsewhere(agents)] = other.children.as_slice() else {
            panic!("the agents elsewhere alone in Other");
        };
        assert_eq!(
            agents.sessions.len(),
            1,
            "only the one whose folder matches"
        );
    }

    #[test]
    fn the_agents_elsewhere_stand_first_in_other_with_the_projects_nothing_else_groups() {
        let map = snapshot(
            vec![project("recent", 1), quiet("old"), project("older", 60)],
            vec![agent("/tmp/scratch", AgentStatus::Busy)],
        );
        assert_eq!(
            listed(&map, &Arrangement::default()),
            ["Other: elsewhere, recent, older, old"],
            "one card in focus brings Other's quiet one with it"
        );
    }

    #[test]
    fn the_scratch_of_a_session_that_ended_waits_behind_the_quiet_row_in_the_activity_sort_alone() {
        let sized = |gigabytes: u64, project: Project| Project {
            own_files: measured(gigabytes * LARGE_FOLDER_BYTES),
            ..project
        };
        let ended = scratch_folder("d5fa0639", "sim-target", project("", 1));
        let running = running_as(
            "Foundry",
            scratch_folder("e26ca359", "r567", project("", 1)),
        );
        let map = snapshot(
            vec![
                sized(12, came_from(upstream("github.com/me/sim"), &[], ended)),
                sized(
                    2,
                    came_from(upstream("github.com/me/foundry"), &[], running),
                ),
                sized(3, quiet("forgotten")),
            ],
            vec![],
        );
        let listed = |sort| {
            let mut arrangement = Arrangement::default();
            arrangement.set_sort(sort, &map, &cleaning(), now());
            arrangement.toggle_quiet(Some(&map), &cleaning(), now());
            listed(&map, &arrangement)
        };
        assert_eq!(
            listed(SortOrder::Activity),
            [
                "foundry: r567",
                "quiet:2",
                "sim: sim-target",
                "Other: forgotten"
            ],
            "its activity a minute ago, and its size, are disk news"
        );
        assert_eq!(
            listed(SortOrder::Disk),
            ["sim: sim-target", "Other: forgotten", "foundry: r567"],
            "in focus where the sort is for disk, the largest group first"
        );
    }

    #[test]
    fn a_scratch_folder_with_nothing_to_show_has_no_row_and_its_groups_line_counts_it_still() {
        let foundry = |folder, own_files| Project {
            own_files: measured(own_files),
            ..came_from(
                upstream("github.com/vt/foundry"),
                &[],
                running_as(
                    "Foundry",
                    scratch_folder("e26ca359", folder, project("", 1)),
                ),
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
        assert_eq!(
            shown(&listing.rows),
            ["foundry: r567guards", "Other: notes"]
        );
        let Row::Group(group) = &listing.rows[0] else {
            panic!("foundry's group");
        };
        assert_eq!(group.members.len(), 2, "both folders add to its total");

        let mut filtered = Arrangement::default();
        filtered.set_filter_text("r567".to_string(), Some(&map), &cleaning(), now());
        assert_eq!(
            listed(&map, &filtered),
            ["foundry: r567guards"],
            "a filter finds no row the list does not have"
        );

        // The folder `deleted()` has just cleaned, `/code/app/target`, is r567fixture's.
        let mut cleaned = map.clone();
        let target = crate::map::test_support::cargo_target("/code/app/target", measured(10));
        cleaned.projects[1].artifacts.push(target);
        let listing = Listing::new(&cleaned, &Arrangement::default(), &deleted(), now());
        assert_eq!(
            shown(&listing.rows),
            ["foundry: r567fixture, r567guards", "Other: notes"],
            "a clean's bar shows on its row until the map measures the folder again"
        );
    }

    #[test]
    fn the_quiet_row_opens_to_list_the_quiet_groups_below_it() {
        let grouped = |remote: &str, project| came_from(upstream(remote), &[], project);
        let map = snapshot(
            vec![
                grouped("github.com/me/b", quiet("b")),
                grouped("github.com/me/recent", project("recent", 1)),
                grouped("github.com/me/a", quiet("a")),
            ],
            vec![],
        );
        let mut arrangement = Arrangement::default();
        assert_eq!(listed(&map, &arrangement), ["recent: recent", "quiet:2"]);
        arrangement.toggle_quiet(Some(&map), &cleaning(), now());
        assert_eq!(
            listed(&map, &arrangement),
            ["recent: recent", "quiet:2", "a: a", "b: b"]
        );
    }

    #[test]
    fn a_subagent_at_work_in_no_project_stands_in_other_and_a_filter_finds_its_folder() {
        let mut builder = subagent_in("/tmp/scratch");
        builder.subagent.acted_in = vec![PathBuf::from("/tmp/build")];
        let map = Snapshot {
            unplaced_subagents: vec![builder, subagent_in("/tmp/other")],
            ..snapshot(vec![project("app", 1)], vec![])
        };
        assert_eq!(
            listed(&map, &Arrangement::default()),
            ["Other: elsewhere, app"]
        );

        let filtered = |text: &str| {
            let mut filtered = Arrangement::default();
            filtered.set_filter_text(text.to_string(), Some(&map), &cleaning(), now());
            filtered
        };
        let listing = Listing::new(&map, &filtered("build"), &cleaning(), now());
        let [Row::Group(other)] = listing.rows.as_slice() else {
            panic!("only Other");
        };
        let [Node::Elsewhere(working)] = other.children.as_slice() else {
            panic!("only the agents elsewhere match");
        };
        let subagents = working.subagents.iter();
        let folders: Vec<&Path> = subagents.map(|one| one.subagent.works_in()).collect();
        assert_eq!(folders, [Path::new("/tmp/build")], "the folder it acted in");
        assert_eq!(
            listed(&map, &filtered("scratch")),
            [""; 0],
            "not the one it runs in"
        );
    }

    /// The rows `map` lists in `sort`, the quiet row open.
    fn listed_in(map: &Snapshot, sort: SortOrder) -> Vec<String> {
        let mut arrangement = Arrangement::default();
        arrangement.set_sort(sort, map, &cleaning(), now());
        arrangement.toggle_quiet(Some(map), &cleaning(), now());
        shown(&Listing::new(map, &arrangement, &cleaning(), now()).rows)
    }

    /// `project`, cloned from an upstream named after it, so it stands in a group of its own.
    fn alone(project: Project) -> Project {
        let remote = format!("github.com/me/{}", project.name);
        came_from(upstream(&remote), &[], project)
    }

    #[test]
    fn dockers_group_stands_among_the_groups_where_each_sort_puts_it() {
        let map = Snapshot {
            docker: Some(docker(&[("pg596", 12)], 10 * LARGE_FOLDER_BYTES)),
            ..snapshot(
                vec![
                    alone(Project {
                        own_files: measured(20 * LARGE_FOLDER_BYTES),
                        ..project("api", 1)
                    }),
                    alone(Project {
                        own_files: measured(LARGE_FOLDER_BYTES),
                        ..project("site", 30)
                    }),
                ],
                vec![],
            )
        };
        let listed = ["api: api", "docker", "site: site"];
        assert_eq!(
            listed_in(&map, SortOrder::Activity),
            listed,
            "its newest container, made 12 minutes ago, between them"
        );
        assert_eq!(listed_in(&map, SortOrder::Name), listed);
        assert_eq!(listed_in(&map, SortOrder::Disk), listed);
    }

    #[test]
    fn docker_with_no_container_on_its_card_is_quiet_in_the_activity_sort_and_disk_news_elsewhere()
    {
        let map = Snapshot {
            docker: Some(docker(&[], 10 * LARGE_FOLDER_BYTES)),
            ..snapshot(vec![alone(project("api", 1))], vec![])
        };
        assert_eq!(
            listed_in(&map, SortOrder::Activity),
            ["api: api", "quiet:1", "docker"]
        );
        assert_eq!(listed_in(&map, SortOrder::Disk), ["docker", "api: api"]);
        let small = Snapshot {
            docker: Some(docker(&[], LARGE_FOLDER_BYTES - 1)),
            ..map.clone()
        };
        assert_eq!(
            listed_in(&small, SortOrder::Disk),
            ["api: api", "quiet:1", "docker"]
        );
        let away = Snapshot {
            docker: None,
            ..map
        };
        assert_eq!(
            listed_in(&away, SortOrder::Activity),
            ["api: api"],
            "Docker not here"
        );
    }

    #[test]
    fn a_typed_filter_lists_dockers_group_by_its_name_or_a_containers_name_or_image() {
        let map = Snapshot {
            docker: Some(docker(&[("pg596", 12)], 0)),
            ..snapshot(vec![alone(project("api", 1))], vec![])
        };
        let filtered = |text: &str| {
            let mut arrangement = Arrangement::default();
            arrangement.set_filter_text(text.to_string(), Some(&map), &cleaning(), now());
            shown(&Listing::new(&map, &arrangement, &cleaning(), now()).rows)
        };
        assert_eq!(filtered("dock"), ["docker"]);
        assert_eq!(filtered("pg59"), ["docker"]);
        assert_eq!(filtered("postgres"), ["docker"], "its image");
        assert_eq!(filtered("api"), ["api: api"]);
    }

    #[test]
    fn a_finished_job_elsewhere_makes_no_row_and_a_running_one_does() {
        let job = |status| AgentSession {
            kind: Some("exec".to_string()),
            harness: Harness::Codex,
            ..agent("/tmp/review", status)
        };
        let finished = snapshot(vec![], vec![job(AgentStatus::Idle)]);
        assert_eq!(listed(&finished, &Arrangement::default()), [""; 0]);
        let mut filtered = Arrangement::default();
        filtered.set_filter_text("review".to_string(), Some(&finished), &cleaning(), now());
        assert_eq!(listed(&finished, &filtered), [""; 0], "nor under a filter");
        let running = snapshot(vec![], vec![job(AgentStatus::Busy)]);
        assert_eq!(
            listed(&running, &Arrangement::default()),
            ["Other: elsewhere"]
        );
    }
}
