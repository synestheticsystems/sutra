//! The Activity sort's groups. A repository is the worktrees that share one git dir
//! (`RepoInfo::common_dir`): a main worktree and the worktrees linked to it. Two clones of one
//! remote are two repositories, and a bare repository's linked worktrees are a group of their own.
//! A repository has a group only where two or more of its worktrees are listed together, so the
//! list gains no header for a project on its own.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::Path;

use super::SectionRow;
use super::sort::{activity_rank, activity_slice};
use crate::map::{Project, RepoInfo, Snapshot};

/// One repository's worktrees on the list, under the header that names it.
pub(in crate::dashboard) struct Group<'a> {
    /// The git dir the repository's worktrees share, which the group is known by.
    pub(in crate::dashboard) common_dir: &'a Path,
    /// What the header calls the repository (see `name`).
    pub(in crate::dashboard) name: String,
    /// The folder the repository goes by (`RepoInfo::folder`), which the header's tooltip names.
    pub(in crate::dashboard) folder: &'a Path,
    /// The worktrees the list shows, two or more, whether the header holds them folded shut or
    /// not: the main worktree first, then the linked ones by their five-minute slice, each slice
    /// in the order of the names they go by on the list.
    pub(in crate::dashboard) cards: Vec<&'a Project>,
    /// Every worktree of the repository on the map, listed or not, whose sizes the header adds up.
    pub(in crate::dashboard) worktrees: Vec<&'a Project>,
}

/// `projects`, one section's cards, in the Activity order, with the cards of each repository that
/// has two or more of them gathered into its group. The groups and the cards standing alone
/// interleave, each group in the place its newest card would take, so a group moves up only when
/// one of its cards' activity enters a newer slice. `map` gives each group every worktree of its
/// repository.
pub(super) fn group<'a>(projects: Vec<&'a Project>, map: &'a Snapshot) -> Vec<SectionRow<'a>> {
    let mut rows = Vec::new();
    let mut gatherings: HashMap<&Path, Gathering> = HashMap::new();
    for project in projects {
        match &project.repo {
            Some(repo) => gatherings
                .entry(&repo.common_dir)
                .or_insert_with(|| Gathering::new(repo))
                .cards
                .push(project),
            None => rows.push(SectionRow::Project(project)),
        }
    }
    // A header over one card would only take room, so that card stands alone.
    gatherings.retain(|_, gathering| match gathering.cards.as_slice() {
        [alone] => {
            rows.push(SectionRow::Project(alone));
            false
        }
        _ => true,
    });
    for project in &map.projects {
        let gathering = project
            .repo
            .as_ref()
            .and_then(|repo| gatherings.get_mut(repo.common_dir.as_path()));
        if let Some(gathering) = gathering {
            gathering.worktrees.push(project);
        }
    }
    let groups = gatherings.into_values().map(Gathering::into_group);
    rows.extend(groups.map(SectionRow::Repository));
    rows.sort_by_cached_key(|row| match row {
        SectionRow::Project(project) => {
            activity_rank(activity_slice(project), &project.name, &project.path)
        }
        SectionRow::Repository(group) => {
            let newest = group.cards.iter().map(|card| activity_slice(card)).max();
            activity_rank(newest.flatten(), &group.name, group.common_dir)
        }
    });
    rows
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
        // As the Activity sort orders them, except that a nested card goes by its branch, or on
        // no branch by its folder's name (see `card::header`), so within a slice the cards stand
        // in the order of the names they show.
        cards.sort_by_cached_key(|card| {
            let branch = card.repo.as_ref().and_then(|repo| repo.branch.as_deref());
            (
                !is_main_worktree(card),
                Reverse(activity_slice(card)),
                branch.unwrap_or(&card.name).to_lowercase(),
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

    /// What `group` makes of `projects` on `map`, as the rows show it.
    fn grouped_of(projects: Vec<&Project>, map: &Snapshot) -> Vec<String> {
        let rows: Vec<Row> = group(projects, map).into_iter().map(Row::from).collect();
        shown(&rows)
    }

    /// What `group` makes of every project on `map`, as the rows show it.
    fn grouped(map: &Snapshot) -> Vec<String> {
        grouped_of(map.projects.iter().collect(), map)
    }

    /// The groups `group` makes of every project on `map`, failing on any card left alone.
    fn groups(map: &Snapshot) -> Vec<Group<'_>> {
        let rows = group(map.projects.iter().collect(), map).into_iter();
        rows.map(|row| match row {
            SectionRow::Repository(group) => group,
            SectionRow::Project(alone) => panic!("{} stands alone", alone.name),
        })
        .collect()
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
    fn linked_worktrees_in_one_slice_stand_in_the_order_of_the_names_they_go_by() {
        let app = "/code/app/.git";
        // On no branch, it goes by its folder's name, not by the commit its head is at.
        let linked = linked_worktree(app, "", project("app-m", 1));
        let detached = Project {
            repo: linked.repo.map(|repo| RepoInfo {
                branch: None,
                head: Some("01fb326".to_string()),
                ..repo
            }),
            ..linked
        };
        let map = snapshot(
            vec![
                linked_worktree(app, "zeta", project("app-a", 1)),
                detached,
                linked_worktree(app, "alpha", project("app-b", 1)),
            ],
            vec![],
        );
        assert_eq!(grouped(&map), ["app: app-b, app-m, app-a"]);
    }

    #[test]
    fn a_repository_with_one_card_among_them_has_no_group_and_its_card_stands_alone() {
        let app = "/code/app/.git";
        let map = snapshot(
            vec![
                linked_worktree(app, "fix", project("app-fix", 1)),
                project("notes", 1),
                main_worktree(app, "main", quiet("app")),
            ],
            vec![],
        );
        let listed = vec![&map.projects[0], &map.projects[1]];
        assert_eq!(grouped_of(listed, &map), ["app-fix", "notes"]);
    }

    #[test]
    fn groups_and_cards_alone_interleave_by_their_newest_slice_then_by_name() {
        let (alpha, beta, gamma) = ("/code/alpha/.git", "/code/beta/.git", "/code/gamma/.git");
        let map = snapshot(
            vec![
                project("notes", 0),
                main_worktree(beta, "main", project("beta", 1)),
                project("draft", 1),
                main_worktree(alpha, "main", project("alpha", 60)),
                linked_worktree(alpha, "fix", project("alpha-fix", 0)),
                main_worktree(gamma, "main", project("gamma", 30)),
                linked_worktree(gamma, "old", project("gamma-old", 40)),
                project("zine", 30),
            ],
            vec![],
        );
        assert_eq!(
            grouped(&map),
            [
                "alpha: alpha, alpha-fix",
                "beta",
                "draft",
                "notes",
                "gamma: gamma, gamma-old",
                "zine"
            ],
            "alpha's newest card is its linked worktree, and beta's one card stands alone"
        );
    }

    #[test]
    fn two_clones_of_one_remote_are_two_repositories() {
        let clone = |parent: &str| {
            let common_dir = format!("{parent}/app/.git");
            let worktree = |name: &str| Project {
                path: PathBuf::from(parent).join(name),
                ..project(name, 1)
            };
            [
                main_worktree(&common_dir, "main", worktree("app")),
                linked_worktree(&common_dir, "fix", worktree("app-fix")),
            ]
        };
        let map = snapshot([clone("/work"), clone("/code")].concat(), vec![]);
        let dirs: Vec<&Path> = groups(&map).iter().map(|group| group.common_dir).collect();
        assert_eq!(
            dirs,
            [Path::new("/code/app/.git"), Path::new("/work/app/.git")],
            "named alike, so in the order of their git dirs"
        );
    }

    #[test]
    fn a_repository_goes_by_its_main_worktrees_name_listed_or_not_whatever_its_git_dir() {
        // Kept the way some keep their worktrees, its git dir is `.bare` in the main worktree,
        // which is quiet, so only the linked worktrees are listed.
        let bare = "/code/app/.bare";
        let map = snapshot(
            vec![
                linked_worktree(bare, "fix", project("app-fix", 1)),
                linked_worktree(bare, "docs", project("app-docs", 1)),
                main_worktree(bare, "main", quiet("app")),
            ],
            vec![],
        );
        let rows = group(vec![&map.projects[0], &map.projects[1]], &map);
        let [SectionRow::Repository(group)] = rows.as_slice() else {
            panic!("one group, not {}", rows.len());
        };
        assert_eq!(group.name, "app");
        let worktrees: Vec<&str> = group.worktrees.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(
            worktrees,
            ["app-fix", "app-docs", "app"],
            "every worktree on the map"
        );
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
        let site = "/Volumes/work/site/.git";
        let outside_the_roots = snapshot(
            vec![
                linked_worktree(site, "fix", project("site-fix", 1)),
                linked_worktree(site, "docs", project("site-docs", 1)),
            ],
            vec![],
        );
        assert_eq!(grouped(&outside_the_roots), ["site: site-docs, site-fix"]);
    }
}
