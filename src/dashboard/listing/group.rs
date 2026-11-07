//! The Activity sort's groups: the cards that share a parent, under the line that names it. A
//! parent is what decides its cards' fate. A repository is the parent of its worktrees, as they
//! share its history: the worktrees that share one git dir (`RepoInfo::common_dir`), a main
//! worktree and the worktrees linked to it. Two clones of one remote are two repositories, and a
//! bare repository's linked worktrees are a group of their own. A Claude Code session is the parent
//! of its scratch folders, which share no history: they share the session that made them, lie in
//! its folder, and lose their use when it ends. A parent has a group only where two or more of its
//! cards are listed together, so the list gains no line for a card on its own.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;

use super::SectionRow;
use super::sort::{activity_rank, activity_slice};
use crate::map::{Project, RepoInfo, ScratchSpace, Snapshot};

/// The cards that share one parent on the list, under the line that names it.
pub(in crate::dashboard) struct Group<'a> {
    pub(in crate::dashboard) parent: Parent<'a>,
    /// What the line calls the parent (see `name`).
    pub(in crate::dashboard) name: String,
    /// The cards the list shows, two or more, whether the line holds them folded or not: a
    /// repository's main worktree first, then the cards by their five-minute slice, each slice in
    /// the order of the names they go by nested (see `nested_name`).
    pub(in crate::dashboard) cards: Vec<&'a Project>,
    /// Every project of the parent on the map, listed or not, whose sizes the line adds up.
    pub(in crate::dashboard) members: Vec<&'a Project>,
}

/// What a group's cards share.
#[derive(Debug, Clone, Copy)]
pub(in crate::dashboard) enum Parent<'a> {
    /// The repository whose worktrees they are.
    Repository(&'a RepoInfo),
    /// The session whose scratch folders they are, in the space of the first card gathered, which
    /// every card of the session shares.
    Session(&'a ScratchSpace),
}

impl<'a> Parent<'a> {
    /// The parent of `project`, `None` for a plain folder, which has none.
    pub(super) fn of(project: &'a Project) -> Option<Parent<'a>> {
        match (&project.repo, &project.scratch) {
            (Some(repo), _) => Some(Parent::Repository(repo)),
            (None, Some(space)) => Some(Parent::Session(space)),
            (None, None) => None,
        }
    }

    /// The folder the parent is known by: a repository's shared git dir, or a session's folder in
    /// the scratch root.
    pub(in crate::dashboard) fn folder(&self) -> &'a Path {
        match self {
            Parent::Repository(repo) => &repo.common_dir,
            Parent::Session(space) => &space.session_folder,
        }
    }
}

/// `projects`, one section's cards, in the Activity order, with the cards of each parent that has
/// two or more of them gathered into its group. The groups and the cards standing alone interleave,
/// each group in the place its newest card would take, so a group moves up only when one of its
/// cards' activity enters a newer slice. `map` gives each group every project of its parent.
pub(super) fn gather<'a>(projects: Vec<&'a Project>, map: &'a Snapshot) -> Vec<SectionRow<'a>> {
    let mut rows = Vec::new();
    let mut gatherings: HashMap<&Path, Gathering> = HashMap::new();
    for project in projects {
        match Parent::of(project) {
            Some(parent) => gatherings
                .entry(parent.folder())
                .or_insert_with(|| Gathering::new(parent))
                .cards
                .push(project),
            None => rows.push(SectionRow::Project(project)),
        }
    }
    // A line over one card would only take room, so that card stands alone.
    gatherings.retain(|_, gathering| match gathering.cards.as_slice() {
        [alone] => {
            rows.push(SectionRow::Project(alone));
            false
        }
        _ => true,
    });
    for project in &map.projects {
        let gathering = Parent::of(project).and_then(|parent| gatherings.get_mut(parent.folder()));
        if let Some(gathering) = gathering {
            gathering.members.push(project);
        }
    }
    let groups = gatherings.into_values().map(Gathering::into_group);
    rows.extend(groups.map(SectionRow::Group));
    rows.sort_by_cached_key(|row| match row {
        SectionRow::Project(project) => {
            activity_rank(activity_slice(project), &project.name, &project.path)
        }
        SectionRow::Group(group) => {
            let newest = group.cards.iter().map(|card| activity_slice(card)).max();
            activity_rank(newest.flatten(), &group.name, group.parent.folder())
        }
    });
    rows
}

/// A parent's cards as `gather` gathers them, before the group can be named.
struct Gathering<'a> {
    /// The parent of the first card gathered, which all of them share.
    parent: Parent<'a>,
    cards: Vec<&'a Project>,
    members: Vec<&'a Project>,
}

impl<'a> Gathering<'a> {
    fn new(parent: Parent<'a>) -> Gathering<'a> {
        Gathering {
            parent,
            cards: Vec::new(),
            members: Vec::new(),
        }
    }

    fn into_group(self) -> Group<'a> {
        let mut cards = self.cards;
        // As the Activity sort orders them, except that a repository's main worktree comes first,
        // and a nested card goes by another name (see `nested_name`), so within a slice the cards
        // stand in the order of the names they show.
        cards.sort_by_cached_key(|card| {
            (
                !is_main_worktree(card),
                Reverse(activity_slice(card)),
                nested_name(card).to_lowercase(),
                card.path.clone(),
            )
        });
        Group {
            name: name(self.parent, &self.members),
            parent: self.parent,
            cards,
            members: self.members,
        }
    }
}

fn is_main_worktree(project: &Project) -> bool {
    project
        .repo
        .as_ref()
        .is_some_and(|repo| repo.worktree_name.is_none())
}

/// What a card goes by nested under its parent's line, which names what it shares with the cards
/// beside it: a worktree its branch, else its folder's own name, as a worktree on no branch and a
/// scratch folder go by (see `card::header`).
pub(in crate::dashboard) fn nested_name(project: &Project) -> String {
    let branch = project.repo.as_ref().and_then(|repo| repo.branch.clone());
    let folder = || {
        let name = project.path.file_name().unwrap_or(OsStr::new(""));
        name.to_string_lossy().into_owned()
    };
    branch.unwrap_or_else(folder)
}

/// What the line calls `parent`, whose projects on the map are `members`. A repository goes by its
/// main worktree's folder name, when that worktree is on the map; one whose main worktree is not, a
/// bare one or one whose main worktree lies outside the roots, goes by the name of its folder
/// (`RepoInfo::folder`), less `.git`. A session goes by its own name (`ScratchSpace::name`).
fn name(parent: Parent, members: &[&Project]) -> String {
    let repo = match parent {
        Parent::Repository(repo) => repo,
        Parent::Session(space) => return space.name(),
    };
    if let Some(main) = members.iter().find(|member| is_main_worktree(member)) {
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
    use super::super::fixtures::{
        linked_worktree, main_worktree, project, quiet, running_as, scratch_folder, shown, snapshot,
    };
    use super::*;

    /// What `gather` makes of `projects` on `map`, as the rows show it.
    fn grouped_of(projects: Vec<&Project>, map: &Snapshot) -> Vec<String> {
        let rows: Vec<Row> = gather(projects, map).into_iter().map(Row::from).collect();
        shown(&rows)
    }

    /// What `gather` makes of every project on `map`, as the rows show it.
    fn grouped(map: &Snapshot) -> Vec<String> {
        grouped_of(map.projects.iter().collect(), map)
    }

    /// The groups `gather` makes of every project on `map`, failing on any card left alone.
    fn groups(map: &Snapshot) -> Vec<Group<'_>> {
        let rows = gather(map.projects.iter().collect(), map).into_iter();
        rows.map(|row| match row {
            SectionRow::Group(group) => group,
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
    fn a_parent_with_one_card_among_them_has_no_group_and_its_card_stands_alone() {
        let app = "/code/app/.git";
        let map = snapshot(
            vec![
                linked_worktree(app, "fix", project("app-fix", 1)),
                project("notes", 1),
                main_worktree(app, "main", quiet("app")),
                scratch_folder("e26ca359", "r567", project("", 1)),
                scratch_folder("e26ca359", "r593", quiet("")),
            ],
            vec![],
        );
        let listed = vec![&map.projects[0], &map.projects[1], &map.projects[3]];
        assert_eq!(
            grouped_of(listed, &map),
            ["app e26ca359", "app-fix", "notes"]
        );
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
        let dirs: Vec<&Path> = groups(&map)
            .iter()
            .map(|group| group.parent.folder())
            .collect();
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
        let rows = gather(vec![&map.projects[0], &map.projects[1]], &map);
        let [SectionRow::Group(group)] = rows.as_slice() else {
            panic!("one group, not {}", rows.len());
        };
        assert_eq!(group.name, "app");
        let members: Vec<&str> = group.members.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(
            members,
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

    #[test]
    fn a_sessions_scratch_folders_gather_under_its_name_each_going_by_its_folders_own_name() {
        let foundry = |folder, minutes_ago| {
            let scratch = scratch_folder("e26ca359", folder, project("", minutes_ago));
            running_as("Foundry Agent Studio architecture", scratch)
        };
        let map = snapshot(
            vec![
                foundry("r567guards", 10),
                project("notes", 5),
                foundry("target-567-fix", 30),
                foundry("cencora-seed", 10),
                scratch_folder("d5fa0639", "probe", project("", 1)),
            ],
            vec![],
        );
        assert_eq!(
            grouped(&map),
            [
                "app d5fa0639",
                "notes",
                "Foundry Agent Studio architecture: cencora-seed, r567guards, target-567-fix",
            ],
            "the lone folder of another session stands alone, by its session's name"
        );
    }

    #[test]
    fn a_card_nested_goes_by_its_branch_else_its_folders_own_name() {
        let app = "/code/app/.git";
        let on_a_branch = linked_worktree(app, "fix/layout", project("app-fix", 1));
        let detached = Project {
            repo: on_a_branch.repo.clone().map(|repo| RepoInfo {
                branch: None,
                ..repo
            }),
            ..on_a_branch.clone()
        };
        assert_eq!(nested_name(&on_a_branch), "fix/layout");
        assert_eq!(nested_name(&detached), "app-fix");
        let scratch = scratch_folder("e26ca359", "r567", project("", 1));
        assert_eq!(nested_name(&scratch), "r567");
    }
}
