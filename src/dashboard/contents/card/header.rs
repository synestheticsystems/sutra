//! A card's identity: the name's tooltip (the folder and its last activity), and the branch chip
//! and its tooltip (the branch in full, and which worktree this is).

use std::path::Path;

use crate::dashboard::format;
use crate::map::{ActivitySource, Project, RepoInfo};

/// The name's tooltip: the folder, then when its last activity was and where it came from, read
/// at `last_activity`, the moment on the user's clock (see `format::timestamp`).
pub(super) fn name_tooltip(project: &Project, home: &Path, last_activity: Option<&str>) -> String {
    let folder = format::tilde(&project.path, home);
    let source = project.activity_source.map(|source| match source {
        ActivitySource::Git => "from git",
        ActivitySource::Agent => "from an agent session",
        ActivitySource::Environment => "from the dev environment",
        ActivitySource::FileChange => "from a file change",
    });
    let activity = match (last_activity, source) {
        (Some(at), Some(source)) => format!("Last activity {at}, {source}"),
        (Some(at), None) => format!("Last activity {at}"),
        (None, Some(source)) => format!("Last activity {source}"),
        (None, None) => return folder,
    };
    format!("{folder}\n{activity}")
}

/// The branch chip: the branch, `detached abc1234`, or `no commits`. A linked worktree's own name
/// is left to the tooltip (see `worktree`).
pub(super) fn branch(repo: &RepoInfo) -> String {
    match (&repo.branch, &repo.head) {
        (Some(branch), _) => branch.clone(),
        (None, Some(head)) => format!("detached {head}"),
        (None, None) => "no commits".to_string(),
    }
}

/// The branch chip's tooltip: the branch in full, which the chip cuts short when it is long, then
/// which worktree of which repository this is.
pub(super) fn branch_tooltip(repo: &RepoInfo, home: &Path) -> String {
    format!("{}\n{}", branch(repo), worktree(repo, home))
}

/// Which worktree of which repository this is.
fn worktree(repo: &RepoInfo, home: &Path) -> String {
    let Some(name) = &repo.worktree_name else {
        return "Main worktree".to_string();
    };
    // A non-bare repository's common dir is the main worktree's `.git`; its parent names the
    // repository the way the user knows it.
    let repository = match repo.common_dir.file_name() {
        Some(dir) if dir == ".git" => repo.common_dir.parent().unwrap_or(&repo.common_dir),
        _ => &repo.common_dir,
    };
    format!(
        "Linked worktree {name} of {}",
        format::tilde(repository, home)
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::test_support::project_at;

    const HOME: &str = "/Users/dev";

    fn repo(worktree_name: Option<&str>, branch: Option<&str>, head: Option<&str>) -> RepoInfo {
        RepoInfo {
            common_dir: PathBuf::from("/Users/dev/code/app/.git"),
            git_dir: PathBuf::from("/Users/dev/code/app/.git"),
            worktree_name: worktree_name.map(str::to_string),
            branch: branch.map(str::to_string),
            head: head.map(str::to_string),
        }
    }

    #[test]
    fn branch_of_the_main_worktree_is_its_branch() {
        assert_eq!(branch(&repo(None, Some("main"), Some("abc1234"))), "main");
    }

    #[test]
    fn branch_of_a_detached_head_names_the_commit() {
        assert_eq!(
            branch(&repo(None, None, Some("abc1234"))),
            "detached abc1234"
        );
    }

    #[test]
    fn branch_of_a_repository_without_commits_says_so() {
        assert_eq!(branch(&repo(None, None, None)), "no commits");
    }

    #[test]
    fn branch_of_a_linked_worktree_leaves_its_name_to_the_card_and_the_tooltip() {
        let linked = repo(Some("app-wt"), Some("feature-x"), Some("abc1234"));
        assert_eq!(branch(&linked), "feature-x");
        assert!(branch_tooltip(&linked, Path::new(HOME)).contains("app-wt"));
    }

    #[test]
    fn branch_tooltip_gives_the_whole_branch_then_the_worktree_of_the_main_repository_folder() {
        let home = Path::new(HOME);
        assert_eq!(
            branch_tooltip(&repo(None, Some("main"), None), home),
            "main\nMain worktree"
        );
        let long = "claude/worktree-map-subagents-and-the-five-minute-slices";
        assert_eq!(
            branch_tooltip(&repo(Some("map-wt"), Some(long), None), home),
            format!("{long}\nLinked worktree map-wt of ~/code/app")
        );
    }

    #[test]
    fn the_name_tooltip_gives_the_folder_then_when_and_where_activity_came_from() {
        let home = Path::new(HOME);
        let mut project = project_at("/Users/dev/code/app");
        assert_eq!(name_tooltip(&project, home, None), "~/code/app");
        project.activity_source = Some(ActivitySource::Agent);
        assert_eq!(
            name_tooltip(&project, home, Some("2026-11-04 10:23")),
            "~/code/app\nLast activity 2026-11-04 10:23, from an agent session"
        );
        project.activity_source = None;
        assert_eq!(
            name_tooltip(&project, home, Some("2026-11-04 10:23")),
            "~/code/app\nLast activity 2026-11-04 10:23"
        );
    }
}
