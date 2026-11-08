//! A card's identity: its name, the name's tooltip (the folder and its last activity), and the
//! chip beside the name and its tooltip: a worktree's branch (the branch in full, and which
//! worktree this is), or a scratch folder's session, in the mark of scratch (its whole id). The
//! listing decides what a card goes by (see `listing::Naming`), and one going by its branch needs
//! no chip, as its name says what the chip would.

use std::path::Path;

use crate::dashboard::format;
use crate::dashboard::listing::Naming;
use crate::map::{ActivitySource, Project, RepoInfo, ScratchSpace};

/// The chip beside a card's name, whose look says what it marks before its words do.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum Chip {
    /// A worktree's branch, a filled label: the branch, `detached abc1234` or `no commits`, and in
    /// its tooltip the branch in full and which worktree of which repository this is.
    Branch { text: String, tooltip: String },
    /// A scratch folder's session, by its name, in the mark of scratch. The session's name is the
    /// mark's own words, so it steps back with the mark once the session has ended.
    Scratch { session: String, mark: ScratchMark },
}

/// The mark of Claude Code's scratch space: an outline of dashes around a scratch folder's
/// session's name on its card, drawn the way a placeholder is, so that nothing about it reads as a
/// branch, which is kept, where scratch is thrown away. It fades, as a still robot does, once its
/// session has ended and will build there no more.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ScratchMark {
    /// `Claude Code's scratch space`, and the session's whole id.
    pub tooltip: String,
    /// The session runs, and may build in its scratch again.
    pub session_runs: bool,
}

/// What a card goes by.
pub(super) struct Identity {
    pub(super) name: String,
    pub(super) name_tooltip: String,
    pub(super) chip: Option<Chip>,
}

/// What `project`'s card goes by, as `naming` says, its last activity read at `last_activity`, the
/// moment on the user's clock (see `format::timestamp`). Going by its branch, the card needs no
/// chip, and its name tooltip gives the branch in full, which the name may cut short, before the
/// folder. Going by its folder, it wears its chip: a worktree's branch, or a scratch folder's
/// session in the mark of scratch.
pub(super) fn identity(
    project: &Project,
    naming: &Naming,
    home: &Path,
    last_activity: Option<&str>,
) -> Identity {
    let folder = name_tooltip(project, home, last_activity);
    match (naming, &project.repo) {
        (Naming::Branch, Some(repo)) => {
            let branch = branch(repo);
            Identity {
                name_tooltip: format!("{branch}\n{folder}"),
                name: branch,
                chip: None,
            }
        }
        (naming, repo) => Identity {
            name: naming.shown(project).to_string(),
            name_tooltip: folder,
            chip: match (repo, &project.scratch) {
                (Some(repo), _) => Some(Chip::Branch {
                    text: branch(repo),
                    tooltip: branch_tooltip(repo, home),
                }),
                (None, Some(space)) => Some(Chip::Scratch {
                    session: space.name(),
                    mark: scratch_mark(space),
                }),
                (None, None) => None,
            },
        },
    }
}

/// The mark of `space`, one session's scratch, around the session's name on its scratch folders'
/// cards. Its tooltip says whose session it is, by its whole id. The mark fades once the session
/// has ended, which says whether the session may still build there.
fn scratch_mark(space: &ScratchSpace) -> ScratchMark {
    ScratchMark {
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

    /// What a card goes by under its folder's own name.
    fn by_folder(project: &Project) -> Naming {
        Naming::Folder(project.name.clone())
    }

    #[test]
    fn a_worktree_going_by_its_folder_wears_its_head_on_its_chip_when_on_no_branch() {
        let home = Path::new(HOME);
        let detached = Project {
            repo: Some(repo(Some("app-pr1-astra"), None, Some("01fb326"))),
            ..project_at("/Users/dev/code/app-pr1-astra")
        };
        let astra = identity(&detached, &by_folder(&detached), home, None);
        assert_eq!(astra.name, "app-pr1-astra");
        assert_eq!(branch_text(astra.chip).as_deref(), Some("detached 01fb326"));
    }

    #[test]
    fn a_card_going_by_its_branch_gives_it_in_full_above_its_folder_and_needs_no_chip() {
        let home = Path::new(HOME);
        let long = "claude/worktree-map-subagents-and-the-five-minute-slices";
        let worktree = Project {
            repo: Some(repo(Some("app-wt"), Some(long), None)),
            ..project_at("/Users/dev/code/app-wt")
        };
        let at = Some("2026-11-04 10:23");

        let by_branch = identity(&worktree, &Naming::Branch, home, at);
        assert_eq!(by_branch.name, long);
        assert_eq!(
            by_branch.name_tooltip,
            format!("{long}\n~/code/app-wt\nLast activity 2026-11-04 10:23")
        );
        assert_eq!(by_branch.chip, None);

        let lengthened = Naming::Folder("code/app-wt".to_string());
        let by_folder = identity(&worktree, &lengthened, home, at);
        assert_eq!(
            by_folder.name, "code/app-wt",
            "the name the listing gave it"
        );
        assert_eq!(
            by_folder.name_tooltip,
            "~/code/app-wt\nLast activity 2026-11-04 10:23"
        );
        assert_eq!(branch_text(by_folder.chip).as_deref(), Some(long));
    }

    #[test]
    fn a_folder_in_no_repository_goes_by_its_name_and_wears_no_chip() {
        let notes = project_at("/Users/dev/code/notes");
        let identity = identity(&notes, &by_folder(&notes), Path::new(HOME), None);
        assert_eq!(identity.name, "notes");
        assert_eq!(identity.name_tooltip, "~/code/notes");
        assert_eq!(identity.chip, None);
    }

    #[test]
    fn a_scratch_folder_goes_by_its_own_name_and_wears_its_sessions_in_the_mark() {
        let session = "/private/tmp/claude-501/-Users-dev-sk/d5fa0639-441e-4722-b044-080d70c0b04f";
        let path = format!("{session}/scratchpad/r567fixture");
        let scratch = |running: Option<crate::map::AgentSession>| Project {
            scratch: Some(ScratchSpace {
                session_folder: PathBuf::from(session),
                session_id: "d5fa0639-441e-4722-b044-080d70c0b04f".to_string(),
                started_in: "sk".to_string(),
                started_in_folder: None,
                session: running,
                session_name: Some("Simulator review".to_string()),
            }),
            ..project_at(&path)
        };
        let mark = |session_runs| {
            Some(Chip::Scratch {
                session: "Simulator review".to_string(),
                mark: ScratchMark {
                    tooltip: "Claude Code's scratch space\n\
                              Session d5fa0639-441e-4722-b044-080d70c0b04f"
                        .to_string(),
                    session_runs,
                },
            })
        };

        let ended = scratch(None);
        let ended = identity(&ended, &by_folder(&ended), Path::new(HOME), None);

        assert_eq!(ended.name, "r567fixture");
        assert_eq!(ended.name_tooltip, path, "the folder in full");
        assert_eq!(ended.chip, mark(false));
        let running = crate::map::test_support::agent_session(
            "d5fa0639-441e-4722-b044-080d70c0b04f",
            crate::map::AgentStatus::Busy,
        );
        let running = scratch(Some(running));
        let running = identity(&running, &by_folder(&running), Path::new(HOME), None);
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
