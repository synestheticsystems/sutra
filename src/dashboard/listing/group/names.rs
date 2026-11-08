//! What the rows of one line go by, which are also the names they are ordered by: what sets each
//! apart from the rows beside it. A card goes by its branch where its line stands for a checkout of
//! its own repository, as git gives each of a repository's worktrees a branch of its own; every
//! other card, and a checkout's line, goes by its folder's name. Where two rows beside each other
//! would read alike, beside the same chip, each takes on the folders above it until they differ. A
//! group's line goes by its upstream's own name, lengthened the same way where another group's
//! would read alike.

use std::collections::HashMap;
use std::path::{Component, Path};

use crate::map::{GroupKey, Project};

/// What a card goes by among the rows beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::dashboard) enum Naming {
    /// Its branch, which needs no chip beside it.
    Branch,
    /// Its folder's name, made longer where a row beside it would read alike, beside its chip: its
    /// branch, or its session's scratch mark.
    Folder(String),
}

impl Naming {
    /// The name the card shows, as it is ordered by.
    pub(in crate::dashboard) fn shown<'a>(&'a self, project: &'a Project) -> &'a str {
        match self {
            Naming::Branch => {
                let branch = project
                    .repo
                    .as_ref()
                    .and_then(|repo| repo.branch.as_deref());
                branch.unwrap_or(&project.name)
            }
            Naming::Folder(name) => name,
        }
    }
}

/// One row of a line, as its name is worked out.
pub(super) struct Row<'a> {
    pub(super) project: &'a Project,
    /// The row is a checkout's line, holding the copies made from it, which wears no chip.
    pub(super) is_line: bool,
}

/// What each of `rows`, the rows of one line, goes by, in their order. `line` is the checkout the
/// line stands for, when it stands for one: the cards of its repository on a branch go by it. A
/// checkout's line goes by its folder's name, as `Naming::Folder` gives it.
pub(super) fn of_rows(rows: &[Row], line: Option<&Project>) -> Vec<Naming> {
    let by_branch = |row: &Row| {
        let on_a_branch = row
            .project
            .repo
            .as_ref()
            .is_some_and(|repo| repo.branch.is_some());
        !row.is_line
            && on_a_branch
            && line.is_some_and(|line| repository(line) == repository(row.project))
    };
    let candidates: Vec<(Vec<String>, Chip)> = rows
        .iter()
        .map(|row| match by_branch(row) {
            true => (
                vec![Naming::Branch.shown(row.project).to_string()],
                Chip::None,
            ),
            false if row.is_line => (folder_names(row.project), Chip::None),
            false => (folder_names(row.project), Chip::of(row.project)),
        })
        .collect();
    let distinct = distinct(&candidates);
    rows.iter()
        .zip(distinct)
        .map(|(row, name)| match by_branch(row) {
            true => Naming::Branch,
            false => Naming::Folder(name),
        })
        .collect()
}

/// The git dir of the repository `project` is a worktree of, if it is one.
fn repository(project: &Project) -> Option<&Path> {
    project.repo.as_ref().map(|repo| repo.common_dir.as_path())
}

/// What each group's line goes by: the name of what its key names (see `key_names`), made longer
/// where another group's would read alike, or a line keyed by no group, as Docker's, already reads
/// so: `taken` holds what those lines read.
pub(in crate::dashboard::listing) fn of_groups<'k>(
    keys: impl IntoIterator<Item = &'k GroupKey>,
    taken: &[&str],
) -> HashMap<&'k GroupKey, String> {
    let keys: Vec<&GroupKey> = keys.into_iter().collect();
    let groups = keys.iter().map(|key| (key_names(key), Chip::None));
    let fixed = taken
        .iter()
        .map(|name| (vec![name.to_string()], Chip::None));
    let candidates: Vec<(Vec<String>, Chip)> = groups.chain(fixed).collect();
    keys.into_iter().zip(distinct(&candidates)).collect()
}

/// What a group's line may go by, shortest first: an upstream by its repository's name, then with
/// its owner and its host; a local repository by its folder's name, less `.git`, then with the
/// folders above it; a package by its name.
fn key_names(key: &GroupKey) -> Vec<String> {
    match key {
        GroupKey::Upstream(remote) => {
            let parts: Vec<&str> = remote.split('/').filter(|part| !part.is_empty()).collect();
            (1..=parts.len().max(1))
                .map(|taken| parts[parts.len().saturating_sub(taken)..].join("/"))
                .collect()
        }
        GroupKey::LocalRepository(folder) => {
            let names = suffixes(folder, None);
            let less_git = |name: String| match name.strip_suffix(".git") {
                Some(stripped) if !stripped.is_empty() && !stripped.ends_with('/') => {
                    stripped.to_string()
                }
                _ => name,
            };
            names.into_iter().map(less_git).collect()
        }
        GroupKey::Package(name) => vec![name.clone()],
        GroupKey::Other => vec!["Other".to_string()],
    }
}

/// What a card or a line may go by, shortest first: its folder's name, then with each folder above
/// it. A scratch folder's run out at its session's folder, whose name is the session's long id,
/// which the scratch mark says already.
fn folder_names(project: &Project) -> Vec<String> {
    let below = project
        .scratch
        .as_ref()
        .map(|space| space.session_folder.as_path());
    suffixes(&project.path, below)
}

/// The paths ending `path`, one part, then two and on, to the whole of it below `below` when
/// given, else to the whole path.
fn suffixes(path: &Path, below: Option<&Path>) -> Vec<String> {
    let path = below
        .and_then(|below| path.strip_prefix(below).ok())
        .unwrap_or(path);
    let parts: Vec<String> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return vec![path.to_string_lossy().into_owned()];
    }
    (1..=parts.len())
        .map(|taken| parts[parts.len() - taken..].join("/"))
        .collect()
}

/// The chip beside a card's name, as far as telling two cards apart goes: two that read alike
/// beside chips that differ are told apart by them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Chip<'a> {
    None,
    /// A worktree's: its branch, or the commit its head is at.
    Branch(Option<&'a str>, Option<&'a str>),
    /// A scratch folder's: its session's mark.
    Session(&'a Path),
}

impl<'a> Chip<'a> {
    fn of(project: &'a Project) -> Chip<'a> {
        match (&project.repo, &project.scratch) {
            (Some(repo), _) => Chip::Branch(repo.branch.as_deref(), repo.head.as_deref()),
            (None, Some(space)) => Chip::Session(&space.session_folder),
            (None, None) => Chip::None,
        }
    }
}

/// For each item, the shortest of its names, `candidates` shortest first, that no other item
/// reads beside the same chip; one whose every name another shares keeps its longest.
fn distinct(candidates: &[(Vec<String>, Chip)]) -> Vec<String> {
    let mut taken = vec![0; candidates.len()];
    let name = |item: usize, taken: &[usize]| candidates[item].0[taken[item]].as_str();
    loop {
        let mut read: HashMap<(&str, Chip), Vec<usize>> = HashMap::new();
        for (item, (_, chip)) in candidates.iter().enumerate() {
            read.entry((name(item, &taken), *chip))
                .or_default()
                .push(item);
        }
        let mut lengthened = false;
        for alike in read.values().filter(|alike| alike.len() > 1) {
            for &item in alike {
                if taken[item] + 1 < candidates[item].0.len() {
                    taken[item] += 1;
                    lengthened = true;
                }
            }
        }
        if !lengthened {
            break;
        }
    }
    (0..candidates.len())
        .map(|item| name(item, &taken).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::dashboard::listing::fixtures::{
        linked_worktree, main_worktree, project, scratch_folder,
    };
    use crate::map::test_support::project_at;

    /// The names `of_rows` gives `rows`, each a project and whether it holds copies, under the line
    /// of `line`, as the cards show them.
    fn shown(rows: &[(&Project, bool)], line: Option<&Project>) -> Vec<String> {
        let rows: Vec<Row> = rows
            .iter()
            .map(|(project, is_line)| Row {
                project,
                is_line: *is_line,
            })
            .collect();
        let names = of_rows(&rows, line);
        let shown = rows.iter().zip(&names);
        shown
            .map(|(row, naming)| naming.shown(row.project).to_string())
            .collect()
    }

    #[test]
    fn a_cards_of_its_lines_repository_go_by_their_branches_and_any_other_by_its_folder() {
        let app = "/code/app/.git";
        let main = main_worktree(app, "main", project("app", 1));
        let fix = linked_worktree(app, "fix/layout", project("app-fix", 1));
        let clone = main_worktree("/code/app-2/.git", "main", project("app-2", 1));
        let scratch = scratch_folder("e26ca359", "review", project("", 1));
        let rows = [
            (&main, false),
            (&fix, false),
            (&clone, false),
            (&scratch, false),
        ];
        assert_eq!(
            shown(&rows, Some(&main)),
            ["main", "fix/layout", "app-2", "review"]
        );
        assert_eq!(
            shown(&rows, None),
            ["app", "app-fix", "app-2", "review"],
            "under the group's own line, every card by its folder"
        );
    }

    #[test]
    fn a_worktree_on_no_branch_goes_by_its_folder_with_its_head_on_its_chip() {
        let app = "/code/app/.git";
        let main = main_worktree(app, "main", project("app", 1));
        let mut detached = linked_worktree(app, "", project("app-pr1", 1));
        if let Some(repo) = detached.repo.as_mut() {
            repo.branch = None;
            repo.head = Some("01fb326".to_string());
        }
        assert_eq!(shown(&[(&detached, false)], Some(&main)), ["app-pr1"]);
    }

    #[test]
    fn rows_beside_each_other_that_read_alike_beside_one_chip_take_on_the_folders_above_them() {
        let clone = |path: &str| Project {
            repo: main_worktree(&format!("{path}/.git"), "main", project("x", 1)).repo,
            ..project_at(path)
        };
        let vt = clone("/Users/me/vt/foundry");
        let ss = clone("/Users/me/ss/foundry");
        let other = clone("/Users/me/ss/other");
        assert_eq!(
            shown(&[(&vt, false), (&ss, false), (&other, false)], None),
            ["vt/foundry", "ss/foundry", "other"]
        );
        let on_a_branch = |branch: &str, project: &Project| Project {
            repo: project.repo.clone().map(|repo| crate::map::RepoInfo {
                branch: Some(branch.to_string()),
                ..repo
            }),
            ..project.clone()
        };
        assert_eq!(
            shown(&[(&on_a_branch("fix", &vt), false), (&ss, false)], None),
            ["foundry", "foundry"],
            "told apart by the branches on their chips"
        );
    }

    #[test]
    fn two_scratch_folders_of_one_session_that_read_alike_go_by_their_places_in_its_folder() {
        let session = PathBuf::from("/private/tmp/claude-501/-code-app/e26ca359");
        let tasks = scratch_folder("e26ca359", "tasks", project("", 1));
        let in_scratchpad = Project {
            path: session.join("scratchpad/tasks"),
            ..tasks.clone()
        };
        let beside = Project {
            path: session.join("tasks"),
            ..tasks
        };
        assert_eq!(
            shown(&[(&in_scratchpad, false), (&beside, false)], None),
            ["scratchpad/tasks", "tasks"]
        );
        let other_session = scratch_folder("0b1d4f00", "tasks", project("", 1));
        assert_eq!(
            shown(&[(&beside, false), (&other_session, false)], None),
            ["tasks", "tasks"],
            "told apart by their sessions' marks"
        );
    }

    #[test]
    fn a_checkouts_line_goes_by_its_folder_and_wears_no_chip() {
        let app = main_worktree("/code/app/.git", "main", project("app", 1));
        let card = main_worktree("/elsewhere/app/.git", "main", {
            Project {
                path: PathBuf::from("/elsewhere/app"),
                ..project("app", 1)
            }
        });
        assert_eq!(
            shown(&[(&app, true), (&card, false)], Some(&app)),
            ["app", "app"],
            "the line by its folder; the other card on the same branch, but no chip on the line"
        );
    }

    #[test]
    fn a_group_goes_by_its_upstreams_name_made_longer_where_another_groups_would_read_alike() {
        let keys = [
            GroupKey::Upstream("github.com/vt/foundry".to_string()),
            GroupKey::Upstream("github.com/fork/foundry".to_string()),
            GroupKey::Upstream("github.com/ss/sutra".to_string()),
            GroupKey::LocalRepository(PathBuf::from("/code/notes.git")),
            GroupKey::Package("sutra".to_string()),
            GroupKey::Other,
        ];
        let names = of_groups(&keys, &[]);
        let named: Vec<&str> = keys.iter().map(|key| names[key].as_str()).collect();
        assert_eq!(
            named,
            [
                "vt/foundry",
                "fork/foundry",
                "ss/sutra",
                "notes",
                "sutra",
                "Other"
            ],
            "the package's name cannot grow, so the upstream's does"
        );
    }

    #[test]
    fn a_group_that_would_read_as_a_line_keyed_by_no_group_goes_by_a_longer_name() {
        let keys = [GroupKey::Upstream("github.com/me/Docker".to_string())];
        assert_eq!(of_groups(&keys, &["Docker"])[&keys[0]], "me/Docker");
        assert_eq!(
            of_groups(&keys, &[])[&keys[0]],
            "Docker",
            "with Docker's line away"
        );
    }
}
