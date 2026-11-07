//! A card's identity: its name, the name's tooltip (the folder and its last activity), and the
//! chip beside the name and its tooltip: a worktree's branch (the branch in full, and which
//! worktree this is), or a scratch folder's place in its session's scratch (whose session it is).
//! A card nested under its repository's header goes by its branch instead, as the header names the
//! repository.

use std::path::Path;

use super::Naming;
use crate::dashboard::{Chip, ScratchMark, format};
use crate::map::{ActivitySource, Project, RepoInfo, ScratchSpace};

/// What a card goes by.
pub(super) struct Identity {
    pub(super) name: String,
    pub(super) name_tooltip: String,
    pub(super) chip: Option<Chip>,
}

/// What `project`'s card goes by, as `naming` says, its last activity read at `last_activity`, the
/// moment on the user's clock (see `format::timestamp`). Going by its branch, the card needs no
/// chip, and the name's tooltip gives the branch in full, which the name may cut short, before the
/// folder. A folder in no repository has no branch, and goes by its name whatever `naming` says;
/// so does a worktree on no branch, as two detached at one commit would read the same, and its
/// chip says where its head is.
pub(super) fn identity(
    project: &Project,
    naming: Naming,
    home: &Path,
    last_activity: Option<&str>,
) -> Identity {
    let folder = name_tooltip(project, home, last_activity);
    match (naming, &project.repo) {
        (Naming::Branch, Some(repo)) if repo.branch.is_some() => {
            let branch = branch(repo);
            Identity {
                name_tooltip: format!("{branch}\n{folder}"),
                name: branch,
                chip: None,
            }
        }
        (Naming::Folder | Naming::Branch, repo) => Identity {
            name: project.name.clone(),
            name_tooltip: folder,
            chip: match (repo, &project.scratch) {
                (Some(repo), _) => Some(Chip::Branch {
                    text: branch(repo),
                    tooltip: branch_tooltip(repo, home),
                }),
                (None, Some(space)) => Some(Chip::Scratch {
                    mark: scratch_mark(space, &project.path),
                }),
                (None, None) => None,
            },
        },
    }
}

/// The mark of `space`, one session's scratch, on its scratch folder at `folder`: where the folder
/// lies in the session's folder, and whose session it is, by its whole id, as the card goes by the
/// session's name. The mark fades once the session has ended, which says whether the session may
/// still build in the folder.
fn scratch_mark(space: &ScratchSpace, folder: &Path) -> ScratchMark {
    ScratchMark {
        place: space.place_of(folder).display().to_string(),
        tooltip: format!("Claude Code's scratch space\nSession {}", space.session_id),
        session_runs: space.session.is_some(),
    }
}

/// The name's tooltip: the folder, then when its last activity was and where it came from, read
/// at `last_activity`.
fn name_tooltip(project: &Project, home: &Path, last_activity: Option<&str>) -> String {
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
fn branch(repo: &RepoInfo) -> String {
    match (&repo.branch, &repo.head) {
        (Some(branch), _) => branch.clone(),
        (None, Some(head)) => format!("detached {head}"),
        (None, None) => "no commits".to_string(),
    }
}

/// The branch chip's tooltip: the branch in full, which the chip cuts short when it is long, then
/// which worktree of which repository this is.
fn branch_tooltip(repo: &RepoInfo, home: &Path) -> String {
    format!("{}\n{}", branch(repo), worktree(repo, home))
}

/// Which worktree of which repository this is.
fn worktree(repo: &RepoInfo, home: &Path) -> String {
    let Some(name) = &repo.worktree_name else {
        return "Main worktree".to_string();
    };
    format!(
        "Linked worktree {name} of {}",
        format::tilde(repo.folder(), home)
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::test_support::project_at;

    const HOME: &str = "/Users/dev";

    /// What a branch chip says, `None` for no chip or another kind.
    fn branch_text(chip: Option<Chip>) -> Option<String> {
        match chip? {
            Chip::Branch { text, .. } => Some(text),
            Chip::Scratch { .. } => None,
        }
    }

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
    fn a_nested_worktree_on_no_branch_goes_by_its_folder_with_its_head_on_the_chip() {
        let home = Path::new(HOME);
        let detached = |folder: &str| Project {
            repo: Some(repo(Some(folder), None, Some("01fb326"))),
            ..project_at(format!("/Users/dev/code/{folder}"))
        };
        let astra = identity(&detached("app-pr1-astra"), Naming::Branch, home, None);
        let opus = identity(&detached("app-pr1-opus"), Naming::Branch, home, None);
        assert_eq!(
            (astra.name.as_str(), opus.name.as_str()),
            ("app-pr1-astra", "app-pr1-opus")
        );
        assert_eq!(branch_text(astra.chip).as_deref(), Some("detached 01fb326"));
    }

    #[test]
    fn a_nested_card_goes_by_its_branch_given_in_full_above_its_folder_and_needs_no_chip() {
        let home = Path::new(HOME);
        let long = "claude/worktree-map-subagents-and-the-five-minute-slices";
        let worktree = Project {
            repo: Some(repo(Some("app-wt"), Some(long), None)),
            ..project_at("/Users/dev/code/app-wt")
        };
        let at = Some("2026-11-04 10:23");

        let nested = identity(&worktree, Naming::Branch, home, at);
        assert_eq!(nested.name, long);
        assert_eq!(
            nested.name_tooltip,
            format!("{long}\n~/code/app-wt\nLast activity 2026-11-04 10:23")
        );
        assert_eq!(nested.chip, None);

        let alone = identity(&worktree, Naming::Folder, home, at);
        assert_eq!(alone.name, "app-wt");
        assert_eq!(
            alone.name_tooltip,
            "~/code/app-wt\nLast activity 2026-11-04 10:23"
        );
        assert_eq!(branch_text(alone.chip).as_deref(), Some(long));
    }

    #[test]
    fn a_folder_in_no_repository_goes_by_its_name_whatever_the_naming() {
        let notes = project_at("/Users/dev/code/notes");
        let identity = identity(&notes, Naming::Branch, Path::new(HOME), None);
        assert_eq!(identity.name, "notes");
        assert_eq!(identity.name_tooltip, "~/code/notes");
        assert_eq!(identity.chip, None);
    }

    #[test]
    fn a_scratch_folder_goes_by_its_sessions_name_its_mark_saying_where_it_lies_and_whose_it_is() {
        let session = "/private/tmp/claude-501/-Users-dev-sk/d5fa0639-441e-4722-b044-080d70c0b04f";
        let path = format!("{session}/scratchpad/r567fixture");
        let scratch = |running: Option<crate::map::AgentSession>| Project {
            name: "sk d5fa0639".to_string(),
            scratch: Some(ScratchSpace {
                session_folder: PathBuf::from(session),
                session_id: "d5fa0639-441e-4722-b044-080d70c0b04f".to_string(),
                started_in: "sk".to_string(),
                session: running,
            }),
            ..project_at(&path)
        };
        let mark = |session_runs| {
            Some(Chip::Scratch {
                mark: ScratchMark {
                    place: "scratchpad/r567fixture".to_string(),
                    tooltip: "Claude Code's scratch space\n\
                              Session d5fa0639-441e-4722-b044-080d70c0b04f"
                        .to_string(),
                    session_runs,
                },
            })
        };

        let ended = identity(&scratch(None), Naming::Branch, Path::new(HOME), None);

        assert_eq!(ended.name, "sk d5fa0639");
        assert_eq!(ended.name_tooltip, path, "the folder in full");
        assert_eq!(ended.chip, mark(false));
        let running = crate::map::test_support::agent_session(
            "d5fa0639-441e-4722-b044-080d70c0b04f",
            crate::map::AgentStatus::Busy,
        );
        let running = identity(
            &scratch(Some(running)),
            Naming::Branch,
            Path::new(HOME),
            None,
        );
        assert_eq!(running.chip, mark(true));
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
