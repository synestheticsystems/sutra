//! The Repository sort's groups. A repository is the worktrees that share one git dir
//! (`RepoInfo::common_dir`): a main worktree and the worktrees linked to it. Two clones of one
//! remote are two repositories, and a bare repository's linked worktrees are a group of their own.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::Path;

use super::Cards;
use super::sort::activity_slice;
use crate::dashboard::SortOrder;
use crate::map::{Project, RepoInfo, Snapshot};

/// One repository's worktrees on the list, under the header that names it.
pub(in crate::dashboard) struct Group<'a> {
    /// The git dir the repository's worktrees share, which the group is known by.
    pub(in crate::dashboard) common_dir: &'a Path,
    /// What the header calls the repository (see `name`).
    pub(in crate::dashboard) name: String,
    /// The folder the repository goes by (`RepoInfo::folder`), which the header's tooltip names.
    pub(in crate::dashboard) folder: &'a Path,
    /// The worktrees the list shows: the main worktree first, then the linked ones by their
    /// five-minute slice, each slice in the order of the branches they go by on the list.
    pub(in crate::dashboard) cards: Vec<&'a Project>,
    /// Every worktree of the repository on the map, listed or not, whose sizes the header adds up.
    pub(in crate::dashboard) worktrees: Vec<&'a Project>,
}

/// `projects`, one section's cards, gathered into their repositories' groups, with the folders in
/// no repository after them in the Activity order. The groups stand by their newest activity
/// slice, then by name, so a group moves up only when one of its cards' activity enters a newer
/// slice. `map` gives each group every worktree of its repository.
pub(super) fn group<'a>(projects: Vec<&'a Project>, map: &'a Snapshot) -> Cards<'a> {
    let mut gatherings: HashMap<&Path, Gathering> = HashMap::new();
    let mut ungrouped = Vec::new();
    for project in projects {
        match &project.repo {
            Some(repo) => gatherings
                .entry(&repo.common_dir)
                .or_insert_with(|| Gathering::new(repo))
                .cards
                .push(project),
            None => ungrouped.push(project),
        }
    }
    for project in &map.projects {
        let gathering = project
            .repo
            .as_ref()
            .and_then(|repo| gatherings.get_mut(repo.common_dir.as_path()));
        if let Some(gathering) = gathering {
            gathering.worktrees.push(project);
        }
    }
    let mut groups: Vec<Group> = gatherings
        .into_values()
        .map(Gathering::into_group)
        .collect();
    groups.sort_by_cached_key(|group| {
        let newest = group.cards.iter().map(|card| activity_slice(card)).max();
        (
            Reverse(newest.flatten()),
            group.name.to_lowercase(),
            group.common_dir,
        )
    });
    Cards {
        groups,
        ungrouped: SortOrder::Activity.arrange(ungrouped),
    }
}

/// A repository's worktrees as `group` gathers them, before the group can be named.
struct Gathering<'a> {
    /// The git state of the first worktree gathered, whose common dir all of them share.
    repo: &'a RepoInfo,
    cards: Vec<&'a Project>,
    worktrees: Vec<&'a Project>,
}

impl<'a> Gathering<'a> {
    fn new(repo: &'a RepoInfo) -> Gathering<'a> {
        Gathering {
            repo,
            cards: Vec::new(),
            worktrees: Vec::new(),
        }
    }

    fn into_group(self) -> Group<'a> {
        let main = self
            .worktrees
            .iter()
            .copied()
            .find(|worktree| is_main_worktree(worktree));
        let mut cards = self.cards;
        // As the Activity sort orders them, except that a nested card goes by its branch, so
        // within a slice the cards stand in the order of the branches they show.
        cards.sort_by_cached_key(|card| {
            let branch = card
                .repo
                .as_ref()
                .and_then(|repo| repo.branch.as_deref().or(repo.head.as_deref()));
            (
                !is_main_worktree(card),
                Reverse(activity_slice(card)),
                branch.unwrap_or_default().to_lowercase(),
                card.path.clone(),
            )
        });
        Group {
            common_dir: &self.repo.common_dir,
            name: name(main, self.repo),
            folder: self.repo.folder(),
            cards,
            worktrees: self.worktrees,
        }
    }
}

fn is_main_worktree(project: &Project) -> bool {
    project
        .repo
        .as_ref()
        .is_some_and(|repo| repo.worktree_name.is_none())
}

/// What the header calls a repository: its main worktree's folder name, from `main`, the main
/// worktree on the map. A repository whose main worktree is not on the map, a bare one or one
/// whose main worktree lies outside the roots, goes by the name of its folder
/// (`RepoInfo::folder`), less `.git`.
fn name(main: Option<&Project>, repo: &RepoInfo) -> String {
    if let Some(main) = main {
        return main.name.clone();
    }
    let folder = repo.folder();
    let name = folder
        .file_name()
        .unwrap_or(folder.as_os_str())
        .to_string_lossy();
    name.strip_suffix(".git").unwrap_or(&name).to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::Row;
    use super::super::fixtures::{linked_worktree, main_worktree, project, quiet, shown, snapshot};
    use super::*;

    /// What `group` makes of every project on `map`, as the rows show it.
    fn grouped(map: &Snapshot) -> Vec<String> {
        let rows: Vec<Row> = group(map.projects.iter().collect(), map)
            .into_rows()
            .collect();
        shown(&rows)
    }

    #[test]
    fn a_group_lists_its_main_worktree_first_then_its_linked_ones_in_the_activity_order() {
        // A minute ago falls in the slice of the clock now falls in; ten minutes ago does not.
        let app = "/code/app/.git";
        let map = snapshot(
            vec![
                linked_worktree(app, "fix", project("app-fix", 0)),
                linked_worktree(app, "docs", project("app-docs", 1)),
                linked_worktree(app, "old", project("app-old", 10)),
                main_worktree(app, "main", project("app", 30)),
            ],
            vec![],
        );
        assert_eq!(grouped(&map), ["app: app, app-docs, app-fix, app-old"]);
    }

    #[test]
    fn linked_worktrees_in_one_slice_stand_in_the_order_of_the_branches_they_show() {
        let app = "/code/app/.git";
        let map = snapshot(
            vec![
                linked_worktree(app, "zeta", project("app-a", 1)),
                linked_worktree(app, "alpha", project("app-b", 1)),
            ],
            vec![],
        );
        assert_eq!(grouped(&map), ["app: app-b, app-a"]);
    }

    #[test]
    fn the_groups_stand_by_their_newest_slice_then_by_name_and_the_folders_in_none_follow() {
        let map = snapshot(
            vec![
                project("notes", 0),
                main_worktree("/code/beta/.git", "main", project("beta", 1)),
                project("draft", 1),
                main_worktree("/code/alpha/.git", "main", project("alpha", 60)),
                linked_worktree("/code/alpha/.git", "fix", project("alpha-fix", 0)),
                main_worktree("/code/gamma/.git", "main", project("gamma", 30)),
            ],
            vec![],
        );
        assert_eq!(
            grouped(&map),
            [
                "alpha: alpha, alpha-fix",
                "beta: beta",
                "gamma: gamma",
                "draft",
                "notes"
            ],
            "alpha's newest slice is its linked worktree's"
        );
    }

    #[test]
    fn two_clones_of_one_remote_are_two_repositories() {
        let clone = |parent: &str| {
            let path = PathBuf::from(parent).join("app");
            let common_dir = path.join(".git");
            let app = Project {
                path,
                ..project("app", 1)
            };
            main_worktree(&common_dir.to_string_lossy(), "main", app)
        };
        let map = snapshot(vec![clone("/work"), clone("/code")], vec![]);
        let cards = group(map.projects.iter().collect(), &map);
        let dirs: Vec<&Path> = cards.groups.iter().map(|group| group.common_dir).collect();
        assert_eq!(
            dirs,
            [Path::new("/code/app/.git"), Path::new("/work/app/.git")],
            "named alike, so in the order of their git dirs"
        );
    }

    #[test]
    fn a_repository_goes_by_its_main_worktrees_name_listed_or_not_whatever_its_git_dir() {
        // Kept the way some keep their worktrees, its git dir is `.bare` in the main worktree,
        // which is quiet, so only the linked worktree is listed.
        let bare = "/code/app/.bare";
        let map = snapshot(
            vec![
                linked_worktree(bare, "fix", project("app-fix", 1)),
                main_worktree(bare, "main", quiet("app")),
            ],
            vec![],
        );
        let cards = group(vec![&map.projects[0]], &map);
        let [group] = cards.groups.as_slice() else {
            panic!("one group, not {}", cards.groups.len());
        };
        assert_eq!(group.name, "app");
        let worktrees: Vec<&str> = group.worktrees.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(worktrees, ["app-fix", "app"], "every worktree on the map");
    }

    #[test]
    fn a_repository_whose_main_worktree_is_off_the_map_goes_by_its_folder_less_git() {
        let bare = snapshot(
            vec![
                linked_worktree("/code/tool.git", "main", project("tool-main", 1)),
                linked_worktree("/code/tool.git", "fix", project("tool-fix", 1)),
            ],
            vec![],
        );
        assert_eq!(grouped(&bare), ["tool: tool-fix, tool-main"]);
        let outside_the_roots = snapshot(
            vec![linked_worktree(
                "/Volumes/work/site/.git",
                "fix",
                project("site-fix", 1),
            )],
            vec![],
        );
        assert_eq!(grouped(&outside_the_roots), ["site: site-fix"]);
    }
}
