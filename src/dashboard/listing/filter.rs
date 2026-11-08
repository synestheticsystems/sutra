//! The filter field (⌘F): which projects a typed text admits, among every project on the map and
//! not only those in focus.

use std::path::Path;

use super::DOCKER;
use crate::map::{Docker, GroupKey, Project, ScratchSpace};

/// Typed text matched against a project's name, the name its card's chip shows beside it (a
/// worktree's branch, a scratch folder's session), what its group is keyed by, which its group's
/// line names, and its path, ignoring case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Query {
    /// Lowercase, without the spaces around it.
    needle: String,
}

impl Query {
    /// The query `text` makes, or `None` when it is blank and filters nothing.
    pub(super) fn new(text: &str) -> Option<Query> {
        let needle = text.trim().to_lowercase();
        (!needle.is_empty()).then_some(Query { needle })
    }

    pub(super) fn admits(&self, project: &Project) -> bool {
        let branch = project
            .repo
            .as_ref()
            .and_then(|repo| repo.branch.as_deref());
        let session = project.scratch.as_ref().map(ScratchSpace::name);
        let group = || match &project.provenance.group {
            GroupKey::Upstream(remote) => self.matches(remote),
            GroupKey::LocalRepository(folder) => self.admits_folder(folder),
            GroupKey::Package(name) => self.matches(name),
            GroupKey::Other => self.matches("Other"),
        };
        self.matches(&project.name)
            || branch.is_some_and(|branch| self.matches(branch))
            || session.is_some_and(|session| self.matches(&session))
            || group()
            || self.admits_folder(&project.path)
    }

    /// Whether the query matches Docker's card: its name, or a container's on it, or its image.
    pub(super) fn admits_docker(&self, docker: &Docker) -> bool {
        let mut containers = docker.containers.iter();
        self.matches(DOCKER)
            || containers
                .any(|container| self.matches(&container.name) || self.matches(&container.image))
    }

    /// Whether the query matches a folder's path, as it does for the folder an agent outside every
    /// project works in.
    pub(super) fn admits_folder(&self, path: &Path) -> bool {
        self.matches(&path.to_string_lossy())
    }

    fn matches(&self, text: &str) -> bool {
        text.to_lowercase().contains(&self.needle)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::RepoInfo;
    use crate::map::test_support::project_at;

    fn project(path: &str, branch: Option<&str>) -> Project {
        let path = PathBuf::from(path);
        Project {
            repo: branch.map(|branch| RepoInfo {
                common_dir: path.join(".git"),
                git_dir: path.join(".git"),
                worktree_name: None,
                branch: Some(branch.to_string()),
                head: None,
            }),
            ..project_at(path)
        }
    }

    fn admits(text: &str, project: &Project) -> bool {
        Query::new(text).unwrap().admits(project)
    }

    #[test]
    fn a_query_matches_the_name_the_branch_or_the_path_ignoring_case() {
        let app = project("/Users/me/code/Sutra", Some("poc-worktree-map"));
        assert!(admits("sutra", &app));
        assert!(admits("WORKTREE-map", &app));
        assert!(admits("me/code", &app));
        assert!(!admits("namingspaces", &app));
    }

    #[test]
    fn a_project_without_a_branch_is_matched_by_name_and_path() {
        let site = project("/Users/me/sites/blog", None);
        assert!(admits("blog", &site));
        assert!(!admits("main", &site));
    }

    #[test]
    fn a_scratch_folder_is_matched_by_its_sessions_name_and_its_path_in_the_scratch() {
        use crate::dashboard::listing::fixtures::{running_as, scratch_folder};
        let scratch = scratch_folder("e26ca359", "r567guards", project("/x", None));
        let scratch = running_as("Foundry Agent Studio architecture", scratch);
        assert!(admits("agent studio", &scratch));
        assert!(admits("scratchpad", &scratch));
        assert!(admits("r567", &scratch));
        assert!(!admits("main", &scratch));
    }

    #[test]
    fn blank_text_is_no_query_and_surrounding_spaces_are_ignored() {
        assert_eq!(Query::new(""), None);
        assert_eq!(Query::new("   "), None);
        assert!(admits("  sutra ", &project("/code/sutra", None)));
    }

    #[test]
    fn a_query_matches_the_upstream_a_project_came_from_as_its_groups_line_names_it() {
        use crate::dashboard::listing::fixtures::{came_from, upstream};
        let review = came_from(
            upstream("github.com/synestheticsystems/sutra"),
            &[],
            project("/tmp/s/review", None),
        );
        assert!(admits("synesthetic", &review), "its owner's name");
        assert!(admits("SUTRA", &review), "its repository's name");
        assert!(!admits("alice", &review));
        assert!(
            admits("other", &project("/code/notes", None)),
            "Other's line"
        );
    }
}
