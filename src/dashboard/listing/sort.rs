//! The orders the list can take, as the toolbar names them. Every sort lays the list out in groups
//! (see `group`), and orders both the groups and the rows beside each other under each line: by
//! their newest work, by name, or by the disk they take. Docker's group stands among the others
//! by when the latest container on its card started, by its name, and by the disk Docker takes.

use std::cmp::Reverse;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::focus::{disk_bytes, docker_bytes};
use super::{DOCKER, Elsewhere, Group, Node, SectionRow};
use crate::dashboard::SortOrder;
use crate::map::{Docker, GroupKey, Project};

/// How finely the Activity sort tells activity apart. Rows whose last activity falls in the same
/// five minutes of the clock rank alike and stand in name order, so that agents at work in several
/// projects at once do not shuffle the cards each time one of them writes. The slices are fixed to
/// the clock, not counted back from now, so a row moves up only when its activity enters a newer
/// slice, never because time passes.
const ACTIVITY_SLICE: Duration = Duration::from_secs(5 * 60);

impl SortOrder {
    pub(in crate::dashboard) const ALL: [SortOrder; 3] =
        [SortOrder::Activity, SortOrder::Name, SortOrder::Disk];

    pub(in crate::dashboard) fn label(self) -> &'static str {
        match self {
            SortOrder::Activity => "Activity",
            SortOrder::Name => "Name",
            SortOrder::Disk => "Disk",
        }
    }

    pub(in crate::dashboard) fn tooltip(self) -> &'static str {
        match self {
            SortOrder::Activity => "Newest activity first",
            SortOrder::Name => "By name",
            SortOrder::Disk => "Largest folders first",
        }
    }

    /// `groups` in this order: by their newest work, the newest slice first and each slice by name;
    /// by name, with Other, which names no upstream, last; or the most disk first, groups that take
    /// alike keeping the Activity order. Two of one name stand by their keys, so no two tie.
    pub(super) fn order_groups(self, groups: &mut [Group]) {
        let by_activity = |group: &Group| (activity_rank(group), group.key.clone());
        match self {
            SortOrder::Activity => groups.sort_by_cached_key(by_activity),
            SortOrder::Name => {
                groups.sort_by_cached_key(|group| (name_rank(group), group.key.clone()))
            }
            SortOrder::Disk => {
                groups.sort_by_cached_key(by_activity);
                groups.sort_by_cached_key(|group| Reverse(bytes_of(&group.members)));
            }
        }
    }

    /// One section's rows: `groups`, as `order_groups` ordered them, with Docker's group, while
    /// `docker` is here, after every group this order ranks before it or alike.
    pub(super) fn lay_out<'a>(
        self,
        groups: Vec<Group<'a>>,
        docker: Option<&'a Docker>,
    ) -> Vec<SectionRow<'a>> {
        let at = docker.map(|docker| match self {
            SortOrder::Activity => {
                let rank = docker_activity_rank(docker);
                groups.partition_point(|group| activity_rank(group) <= rank)
            }
            SortOrder::Name => {
                groups.partition_point(|group| name_rank(group) <= docker_name_rank())
            }
            SortOrder::Disk => {
                let bytes = docker_bytes(docker);
                groups.partition_point(|group| bytes_of(&group.members) >= bytes)
            }
        });
        let mut rows: Vec<SectionRow> = groups.into_iter().map(SectionRow::Group).collect();
        if let (Some(at), Some(docker)) = (at, docker) {
            rows.insert(at, SectionRow::Docker(docker));
        }
        rows
    }

    /// `rows`, the rows of one line beside each other, in this order: by their newest work, the
    /// newest slice first and each slice in the order of the names they go by, ignoring case; by
    /// those names; or the most disk first. They arrive in the map's order, newest activity first,
    /// and the Disk sort is stable, so rows that take alike keep it. The agents working elsewhere
    /// come first in every order, as what they show is all at work.
    pub(super) fn order_rows(self, rows: &mut [Node]) {
        match self {
            SortOrder::Activity => rows.sort_by_cached_key(|row| {
                let (name, path) = name_of(row);
                let path = path.map(Path::to_path_buf);
                (rank(row), Reverse(slice_of(row)), name.to_lowercase(), path)
            }),
            SortOrder::Name => rows.sort_by_cached_key(|row| {
                let (name, path) = name_of(row);
                (rank(row), name.to_lowercase(), path.map(Path::to_path_buf))
            }),
            SortOrder::Disk => rows.sort_by_cached_key(|row| (rank(row), Reverse(bytes(row)))),
        }
    }
}

/// Where the Activity sort stands `group`: by the slice of its newest work, the newest first, then
/// by its name, ignoring case.
fn activity_rank(group: &Group) -> (Reverse<Option<u64>>, String) {
    (
        Reverse(newest_slice(&group.children)),
        group.name.to_lowercase(),
    )
}

/// Where the Activity sort stands Docker's group: by the slice in which the latest container on
/// its card started, as a group by its newest work.
fn docker_activity_rank(docker: &Docker) -> (Reverse<Option<u64>>, String) {
    let started = docker
        .containers
        .iter()
        .filter_map(|container| container.started);
    (Reverse(slice(started.max())), DOCKER.to_lowercase())
}

/// Where the Name sort stands `group`: by its name, ignoring case, with Other last.
fn name_rank(group: &Group) -> (bool, String) {
    let other = *group.key == GroupKey::Other;
    (other, group.name.to_lowercase())
}

/// Where the Name sort stands Docker's group: by its name, as any group but Other.
fn docker_name_rank() -> (bool, String) {
    (false, DOCKER.to_lowercase())
}

/// Where a row stands before any order weighs it: the agents working elsewhere first.
fn rank(row: &Node) -> u8 {
    u8::from(!matches!(row, Node::Elsewhere(_)))
}

/// The name a row goes by, and the folder it is known by, which tells apart two of one name.
fn name_of<'a>(row: &'a Node) -> (&'a str, Option<&'a Path>) {
    match row {
        Node::Card { project, naming } => (naming.shown(project), Some(project.path.as_path())),
        Node::Checkout(checkout) => (&checkout.name, Some(checkout.project.path.as_path())),
        Node::Elsewhere(_) => ("", None),
    }
}

/// The slice of the clock (see `ACTIVITY_SLICE`) holding the newest work in `row`; `None`, which
/// the Activity sort puts last, for a row with no work known.
fn slice_of(row: &Node) -> Option<u64> {
    match row {
        Node::Card { project, .. } => activity_slice(project),
        Node::Checkout(checkout) => newest_slice(&checkout.children),
        Node::Elsewhere(elsewhere) => slice(newest_work_elsewhere(elsewhere)),
    }
}

/// The newest slice of `rows`' work.
fn newest_slice(rows: &[Node]) -> Option<u64> {
    rows.iter().filter_map(slice_of).max()
}

/// The newest work of the agents working elsewhere.
fn newest_work_elsewhere(elsewhere: &Elsewhere) -> Option<SystemTime> {
    let sessions = elsewhere
        .sessions
        .iter()
        .map(|session| session.last_activity);
    let subagents = elsewhere.subagents.iter();
    let subagents = subagents.map(|working| working.subagent.last_write);
    sessions.chain(subagents).flatten().max()
}

/// What a row takes on disk, as the Disk sort weighs it: a card its folder, a checkout's line all
/// it holds on the map, listed or not.
fn bytes(row: &Node) -> u64 {
    match row {
        Node::Card { project, .. } => disk_bytes(project),
        Node::Checkout(checkout) => bytes_of(&checkout.members),
        Node::Elsewhere(_) => 0,
    }
}

/// What `projects` take on disk between them, as `focus::disk_bytes` weighs each.
fn bytes_of(projects: &[&Project]) -> u64 {
    projects.iter().map(|project| disk_bytes(project)).sum()
}

/// The slice of the clock holding `project`'s last activity.
fn activity_slice(project: &Project) -> Option<u64> {
    slice(project.last_activity)
}

/// The slice of the clock holding `at`.
fn slice(at: Option<SystemTime>) -> Option<u64> {
    let since_epoch = at?.duration_since(UNIX_EPOCH).ok()?;
    Some(since_epoch.as_secs() / ACTIVITY_SLICE.as_secs())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::Naming;
    use super::super::fixtures::{self, came_from, shown, snapshot, upstream};
    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::{Arrangement, Listing};
    use crate::map::Measurement;
    use crate::map::test_support::{at, cargo_target, measured, project_at};

    /// A project whose build folders hold `sizes` (`None` is a folder not yet measured) and whose
    /// own files hold `own_files`, when they are measured.
    fn project(name: &str, sizes: &[Option<u64>], own_files: Option<u64>) -> Project {
        let path = PathBuf::from("/code").join(name);
        Project {
            artifacts: sizes
                .iter()
                .map(|size| {
                    cargo_target(
                        path.join("target"),
                        size.map_or(Measurement::Measuring, measured),
                    )
                })
                .collect(),
            own_files: own_files.map_or(Measurement::Measuring, measured),
            ..project_at(path)
        }
    }

    /// A project last active `secs` after the epoch, or never.
    fn active(name: &str, secs: Option<u64>) -> Project {
        Project {
            last_activity: secs.map(at),
            ..project(name, &[], None)
        }
    }

    /// The names of `projects`, as cards beside each other going by their folders, in `sort`'s
    /// order.
    fn ordered(sort: SortOrder, projects: &[Project]) -> Vec<String> {
        let mut rows: Vec<Node> = projects
            .iter()
            .map(|project| Node::Card {
                project,
                naming: Naming::Folder(project.name.clone()),
            })
            .collect();
        sort.order_rows(&mut rows);
        rows.iter().map(|row| name_of(row).0.to_string()).collect()
    }

    #[test]
    fn activity_puts_the_newest_five_minutes_first_and_keeps_each_in_name_order() {
        // 1_200 s after the epoch begins a slice; 1_199 s ends the one before.
        let projects = [
            active("never", None),
            active("older-b", Some(1_000)),
            active("older-a", Some(1_199)),
            active("newer-b", Some(1_200)),
            active("newer-a", Some(1_499)),
        ];
        assert_eq!(
            ordered(SortOrder::Activity, &projects),
            ["newer-a", "newer-b", "older-a", "older-b", "never"]
        );
    }

    #[test]
    fn a_row_moves_up_only_when_its_activity_enters_a_newer_slice() {
        let order = |b_at| {
            let projects = [active("a", Some(1_250)), active("b", Some(b_at))];
            ordered(SortOrder::Activity, &projects)
        };
        assert_eq!(order(1_210), ["a", "b"]);
        assert_eq!(
            order(1_490),
            ["a", "b"],
            "newer by four minutes, same slice"
        );
        assert_eq!(order(1_500), ["b", "a"], "into the next slice");
    }

    #[test]
    fn name_ignores_case() {
        let projects = [
            project("beta", &[], None),
            project("Alpha", &[], None),
            project("gamma", &[], None),
        ];
        assert_eq!(
            ordered(SortOrder::Name, &projects),
            ["Alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn disk_puts_the_largest_first_and_ties_keep_activity_order() {
        let projects = [
            project("none", &[], None),
            project("small", &[Some(10)], None),
            project("unmeasured", &[None], None),
            project("large", &[Some(400), Some(200)], None),
        ];
        assert_eq!(
            ordered(SortOrder::Disk, &projects),
            ["large", "small", "none", "unmeasured"]
        );
    }

    #[test]
    fn disk_weighs_a_whole_folder_once_it_is_measured() {
        let projects = [
            project("large-target", &[Some(500)], None),
            project("large-folder", &[Some(100)], Some(1_000)),
        ];
        assert_eq!(
            ordered(SortOrder::Disk, &projects),
            ["large-folder", "large-target"]
        );
    }

    #[test]
    fn every_sort_groups_the_cards_and_orders_the_groups_by_what_it_weighs() {
        let sized = |gigabytes: u64, project: Project| Project {
            own_files: measured(gigabytes << 30),
            ..project
        };
        let grouped = |remote: &str, project| came_from(upstream(remote), &[], project);
        let map = snapshot(
            vec![
                sized(
                    1,
                    grouped("github.com/me/zeta", fixtures::project("zeta", 1)),
                ),
                sized(2, fixtures::project("loose", 1)),
                sized(
                    9,
                    grouped("github.com/me/alpha", fixtures::project("alpha", 30)),
                ),
                sized(
                    3,
                    grouped("github.com/me/zeta", fixtures::project("zeta-2", 40)),
                ),
            ],
            vec![],
        );
        let laid_out = |sort: SortOrder| {
            let mut arrangement = Arrangement::default();
            arrangement.set_sort(sort, &map, &cleaning(), fixtures::now());
            shown(&Listing::new(&map, &arrangement, &cleaning(), fixtures::now()).rows)
        };
        assert_eq!(
            laid_out(SortOrder::Activity),
            ["Other: loose", "zeta: zeta, zeta-2", "alpha: alpha"],
            "a minute ago, Other's name before zeta's, then thirty minutes ago"
        );
        assert_eq!(
            laid_out(SortOrder::Name),
            ["alpha: alpha", "zeta: zeta, zeta-2", "Other: loose"],
            "Other last"
        );
        assert_eq!(
            laid_out(SortOrder::Disk),
            ["alpha: alpha", "zeta: zeta-2, zeta", "Other: loose"],
            "9 GB, then 4 GB in all, the larger of zeta's first"
        );
    }
}
