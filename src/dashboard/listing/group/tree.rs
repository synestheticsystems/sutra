//! The tree of provenance in a group. Each card hangs from the nearest project listed beside it
//! that it was made from (see `Provenance::made_from`): a linked worktree from its repository's
//! main worktree, a clone from what it was cloned from, a session's scratch from the checkout the
//! session started in. A checkout with copies hanging from it stands under a line of its own, which
//! holds its own card first, then its copies. Where the group's top would hold one such checkout
//! alone, its rows hang from the group's line itself, which then stands for the checkout too: a
//! line of its own would only say again what the group's says.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::names::{self, Naming, Row};
use crate::dashboard::SortOrder;
use crate::dashboard::listing::Elsewhere;
use crate::map::Project;

/// One row hanging from a line.
pub(in crate::dashboard) enum Node<'a> {
    /// A project's card, going by `naming`.
    Card {
        project: &'a Project,
        naming: Naming,
    },
    /// A checkout with copies made from it, under a line of its own.
    Checkout(Checkout<'a>),
    /// The agents working outside every project, which Other holds.
    Elsewhere(Elsewhere<'a>),
}

/// A checkout and the copies made from it, under its own line.
pub(in crate::dashboard) struct Checkout<'a> {
    pub(in crate::dashboard) project: &'a Project,
    /// What its line calls it, which no row beside it reads.
    pub(in crate::dashboard) name: String,
    /// Its own card first, then the copies made from it, in the sort's order.
    pub(in crate::dashboard) children: Vec<Node<'a>>,
    /// It and every project of its group on the map made from it, listed or not, whose sizes its
    /// line adds up.
    pub(in crate::dashboard) members: Vec<&'a Project>,
}

impl<'a> Node<'a> {
    /// The cards of this row and of what it holds, in the order on screen, its line folded shut or
    /// not.
    pub(in crate::dashboard) fn cards(&self) -> Vec<&'a Project> {
        match self {
            Node::Card { project, .. } => vec![project],
            Node::Checkout(checkout) => checkout.children.iter().flat_map(Node::cards).collect(),
            Node::Elsewhere(_) => Vec::new(),
        }
    }
}

/// The tree of `listed`, the projects of one group listed in one section, with the agents working
/// `elsewhere` atop it, for Other's. `members` are every project of the group on the map, listed or
/// not. Returns the checkout the group's line stands for too, when its top holds that checkout and
/// its copies alone, and the rows that hang from the line, each line's rows in `sort`'s order.
pub(super) fn grow<'a>(
    listed: Vec<&'a Project>,
    elsewhere: Option<Elsewhere<'a>>,
    members: &[&'a Project],
    sort: SortOrder,
) -> (Option<&'a Project>, Vec<Node<'a>>) {
    let parents = parents(&listed);
    let mut copies: HashMap<&Path, Vec<&'a Project>> = HashMap::new();
    let mut top = Vec::new();
    for project in listed {
        match parents.get(project.path.as_path()) {
            Some(parent) => copies.entry(parent).or_default().push(project),
            None => top.push(project),
        }
    }
    let sprouts: Vec<Sprout> = top
        .into_iter()
        .map(|project| Sprout::of(project, &mut copies))
        .collect();
    let lone_checkout = match (sprouts.as_slice(), &elsewhere) {
        ([only], None) if !only.copies.is_empty() => Some(only.project),
        _ => None,
    };
    let mut nodes = match lone_checkout {
        Some(_) => {
            let only = sprouts.into_iter().next().expect("the one checkout");
            only.into_line_rows(members, sort)
        }
        None => grow_rows(sprouts, None, members, sort),
    };
    if let Some(elsewhere) = elsewhere {
        nodes.insert(0, Node::Elsewhere(elsewhere));
    }
    (lone_checkout, nodes)
}

/// A project and the copies hanging from it, before the rows are named and ordered.
struct Sprout<'a> {
    project: &'a Project,
    copies: Vec<Sprout<'a>>,
}

impl<'a> Sprout<'a> {
    /// `project`, with the copies hanging from it taken from `copies`, and theirs from them.
    fn of(project: &'a Project, copies: &mut HashMap<&Path, Vec<&'a Project>>) -> Sprout<'a> {
        let made_from_it = copies.remove(project.path.as_path()).unwrap_or_default();
        Sprout {
            project,
            copies: made_from_it
                .into_iter()
                .map(|copy| Sprout::of(copy, copies))
                .collect(),
        }
    }

    /// The rows of this checkout's line: its own card first, then its copies.
    fn into_line_rows(self, members: &[&'a Project], sort: SortOrder) -> Vec<Node<'a>> {
        let own = Sprout {
            project: self.project,
            copies: Vec::new(),
        };
        let rows = std::iter::once(own).chain(self.copies).collect();
        grow_rows(rows, Some(self.project), members, sort)
    }
}

/// The rows of one line, `sprouts`, named among each other and ordered as `sort` says; `line` is
/// the checkout the line stands for, when it stands for one, whose own card comes first.
fn grow_rows<'a>(
    sprouts: Vec<Sprout<'a>>,
    line: Option<&'a Project>,
    members: &[&'a Project],
    sort: SortOrder,
) -> Vec<Node<'a>> {
    let rows: Vec<Row> = sprouts
        .iter()
        .map(|sprout| Row {
            project: sprout.project,
            is_line: !sprout.copies.is_empty(),
        })
        .collect();
    let names = names::of_rows(&rows, line);
    let mut nodes: Vec<Node> = sprouts
        .into_iter()
        .zip(names)
        .map(
            |(sprout, naming)| match (sprout.copies.is_empty(), naming) {
                (true, naming) => Node::Card {
                    project: sprout.project,
                    naming,
                },
                (false, naming) => Node::Checkout(Checkout {
                    project: sprout.project,
                    name: naming.shown(sprout.project).to_string(),
                    members: made_from(sprout.project, members),
                    children: sprout.into_line_rows(members, sort),
                }),
            },
        )
        .collect();
    let first = usize::from(line.is_some());
    sort.order_rows(&mut nodes[first..]);
    nodes
}

/// `checkout` and every project among `members` made from it.
fn made_from<'a>(checkout: &'a Project, members: &[&'a Project]) -> Vec<&'a Project> {
    let copies = members.iter().copied().filter(|member| {
        let made_from = &member.provenance.made_from;
        made_from.contains(&checkout.path)
    });
    std::iter::once(checkout).chain(copies).collect()
}

/// The project each of `listed` hangs from: the nearest it was made from that is listed beside
/// it. One that would hang, through others, from itself, as remotes that run in a loop would have
/// it, hangs from none.
fn parents<'a>(listed: &[&'a Project]) -> HashMap<&'a Path, &'a Path> {
    let on_the_list: HashSet<&Path> = listed
        .iter()
        .map(|project| project.path.as_path())
        .collect();
    let mut parents: HashMap<&Path, &Path> = HashMap::new();
    for project in listed {
        let made_from = project.provenance.made_from.iter();
        let nearest = made_from
            .map(|path| path.as_path())
            .find(|path| on_the_list.contains(path));
        if let Some(parent) = nearest {
            parents.insert(&project.path, parent);
        }
    }
    // Each loop is cut once, at the first of its projects by path, so the same one each time.
    let mut paths: Vec<&Path> = on_the_list.into_iter().collect();
    paths.sort();
    for path in paths {
        if hangs_from_itself(&parents, path) {
            parents.remove(path);
        }
    }
    parents
}

/// Whether following `parents` up from `path` comes back to it.
fn hangs_from_itself(parents: &HashMap<&Path, &Path>, path: &Path) -> bool {
    let mut above = parents.get(path).copied();
    // A loop that does not pass through `path` runs no longer than the map of parents.
    for _ in 0..parents.len() {
        match above {
            Some(next) if next == path => return true,
            Some(next) => above = parents.get(next).copied(),
            None => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests;
