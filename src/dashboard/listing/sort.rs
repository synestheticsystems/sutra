//! The orders the project list can take, as the toolbar names them.

use std::cmp::Reverse;

use super::focus::disk_bytes;
use crate::dashboard::SortOrder;
use crate::map::Project;

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

    /// `projects` in this order. They arrive newest activity first and both sorts are stable, so
    /// projects that tie keep that order. The Disk sort weighs `focus::disk_bytes`.
    pub(super) fn arrange<'a>(
        self,
        projects: impl IntoIterator<Item = &'a Project>,
    ) -> Vec<&'a Project> {
        let mut arranged: Vec<&Project> = projects.into_iter().collect();
        match self {
            SortOrder::Activity => {}
            SortOrder::Name => arranged.sort_by_cached_key(|project| project.name.to_lowercase()),
            SortOrder::Disk => arranged.sort_by_key(|project| Reverse(disk_bytes(project))),
        }
        arranged
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::Measurement;
    use crate::map::test_support::{cargo_target, measured, project_at};

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

    #[test]
    fn activity_keeps_the_engine_order() {
        let projects = [
            project("b", &[], None),
            project("a", &[], None),
            project("c", &[], None),
        ];
        assert_eq!(
            names(SortOrder::Activity.arrange(&projects)),
            ["b", "a", "c"]
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
