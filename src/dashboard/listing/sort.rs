//! The orders the project list can take, as the toolbar names them.

use std::cmp::Reverse;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use super::focus::disk_bytes;
use super::{SectionRow, group};
use crate::dashboard::SortOrder;
use crate::map::{Project, Snapshot};

/// How finely the Activity sort tells activity apart. Projects whose last activity falls in the
/// same five minutes of the clock rank alike and stand in name order, so that agents at work in
/// several projects at once do not shuffle the cards each time one of them writes. The slices are
/// fixed to the clock, not counted back from now, so a card moves up only when its activity enters
/// a newer slice, never because time passes.
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

    /// `projects`, the cards above the quiet row or every card a typed filter admits, as this sort
    /// lays them out: the Activity sort gathers the cards of each parent with two or more of them
    /// into its group (see `group`), and the others list every card alone. `map` gives each group
    /// every project of its parent.
    pub(super) fn lay_out<'a>(
        self,
        projects: Vec<&'a Project>,
        map: &'a Snapshot,
    ) -> Vec<SectionRow<'a>> {
        match self {
            SortOrder::Activity => group::gather(projects, map),
            SortOrder::Name | SortOrder::Disk => {
                let arranged = self.arrange(projects).into_iter();
                arranged.map(SectionRow::Project).collect()
            }
        }
    }

    /// `projects` in this order, every card alone, as the quiet section lists them in every sort.
    /// They arrive newest activity first, and the Disk sort is stable, so projects of the same size
    /// keep that order; it weighs `focus::disk_bytes`.
    pub(super) fn arrange<'a>(
        self,
        projects: impl IntoIterator<Item = &'a Project>,
    ) -> Vec<&'a Project> {
        let mut arranged: Vec<&Project> = projects.into_iter().collect();
        match self {
            SortOrder::Activity => arranged.sort_by_cached_key(|project| {
                activity_rank(activity_slice(project), &project.name, &project.path)
            }),
            SortOrder::Name => arranged.sort_by_cached_key(|project| project.name.to_lowercase()),
            SortOrder::Disk => arranged.sort_by_key(|project| Reverse(disk_bytes(project))),
        }
        arranged
    }
}

/// The slice of the clock (see `ACTIVITY_SLICE`) holding `project`'s last activity; `None`, which
/// the Activity sort puts last, for a project with no activity known.
pub(super) fn activity_slice(project: &Project) -> Option<u64> {
    let since_epoch = project.last_activity?.duration_since(UNIX_EPOCH).ok()?;
    Some(since_epoch.as_secs() / ACTIVITY_SLICE.as_secs())
}

/// Where the Activity sort stands a card, or a group, that goes by `name`, its newest activity in
/// `slice`: the newest slice first, each slice in name order, ignoring case, and two of one name
/// by `path`, the card's folder or the folder its group's parent is known by, so that no two tie.
pub(super) fn activity_rank<'p>(
    slice: Option<u64>,
    name: &str,
    path: &'p Path,
) -> (Reverse<Option<u64>>, String, &'p Path) {
    (Reverse(slice), name.to_lowercase(), path)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::Row;
    use super::super::fixtures::{self, linked_worktree, main_worktree, shown, snapshot};
    use super::*;
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

    fn names(arranged: Vec<&Project>) -> Vec<&str> {
        arranged
            .iter()
            .map(|project| project.name.as_str())
            .collect()
    }

    /// A project last active `secs` after the epoch, or never.
    fn active(name: &str, secs: Option<u64>) -> Project {
        Project {
            last_activity: secs.map(at),
            ..project(name, &[], None)
        }
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
            names(SortOrder::Activity.arrange(&projects)),
            ["newer-a", "newer-b", "older-a", "older-b", "never"]
        );
    }

    #[test]
    fn a_card_moves_up_only_when_its_activity_enters_a_newer_slice() {
        let order = |b_at| {
            let projects = [active("a", Some(1_250)), active("b", Some(b_at))];
            let arranged = SortOrder::Activity.arrange(&projects);
            arranged
                .iter()
                .map(|project| project.name.clone())
                .collect::<Vec<_>>()
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
    fn only_the_activity_sort_nests_a_repositorys_worktrees_and_a_sessions_scratch_folders() {
        let app = "/code/app/.git";
        let foundry = |folder| {
            let scratch = fixtures::scratch_folder("e26ca359", folder, fixtures::project("", 1));
            fixtures::running_as("Foundry", scratch)
        };
        let map = snapshot(
            vec![
                main_worktree(app, "main", fixtures::project("app", 1)),
                linked_worktree(app, "fix", fixtures::project("app-fix", 1)),
                foundry("r567"),
                foundry("r593"),
            ],
            vec![],
        );
        let laid_out = |sort: SortOrder| {
            let rows = sort.lay_out(map.projects.iter().collect(), &map);
            let rows: Vec<Row> = rows.into_iter().map(Row::from).collect();
            shown(&rows)
        };
        assert_eq!(
            laid_out(SortOrder::Activity),
            ["app: app, app-fix", "Foundry: r567, r593"]
        );
        assert_eq!(
            laid_out(SortOrder::Name),
            ["app", "app-fix", "Foundry", "Foundry"]
        );
        assert_eq!(
            laid_out(SortOrder::Disk),
            ["app", "app-fix", "Foundry", "Foundry"]
        );
    }

    #[test]
    fn name_ignores_case() {
        let projects = [
            project("beta", &[], None),
            project("Alpha", &[], None),
            project("gamma", &[], None),
        ];
        assert_eq!(
            names(SortOrder::Name.arrange(&projects)),
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
            names(SortOrder::Disk.arrange(&projects)),
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
            names(SortOrder::Disk.arrange(&projects)),
            ["large-folder", "large-target"]
        );
    }
}
