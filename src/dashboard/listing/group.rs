//! The list's groups. Every card stands in the group of the upstream its project came from (see
//! `map::Provenance`), under the line that names it: the cards in a tree of provenance (`tree`),
//! each going by what sets it apart from the rows beside it (`names`). Other holds what nothing
//! else groups, and the agents working outside every project.

mod names;
mod tree;

pub(in crate::dashboard) use names::Naming;
pub(in crate::dashboard) use tree::{Checkout, Node};

use std::collections::{BTreeMap, HashMap};

use super::{DOCKER, Elsewhere};
use crate::dashboard::SortOrder;
use crate::map::{GroupKey, Project, Snapshot};

/// Other's key, which the group of the agents working elsewhere has with no project in it.
static OTHER: GroupKey = GroupKey::Other;

/// A group on the list: the cards of the projects that came from one upstream, under its line.
pub(in crate::dashboard) struct Group<'a> {
    pub(in crate::dashboard) key: &'a GroupKey,
    /// What its line calls it, which no other group's line on the list reads.
    pub(in crate::dashboard) name: String,
    /// The checkout its line stands for too, when its top would hold that checkout and the copies
    /// made from it alone, which then hang from the group's line itself (see `tree`).
    pub(in crate::dashboard) checkout: Option<&'a Project>,
    /// What hangs from its line, in the sort's order.
    pub(in crate::dashboard) children: Vec<Node<'a>>,
    /// Every project of the group on the map, listed or not, whose sizes its line adds up.
    pub(in crate::dashboard) members: Vec<&'a Project>,
}

impl<'a> Group<'a> {
    /// Every card in the group, in the order on screen, its lines folded shut or not.
    pub(in crate::dashboard) fn cards(&self) -> Vec<&'a Project> {
        self.children.iter().flat_map(Node::cards).collect()
    }
}

/// The projects on the map by the group each stands in, listed or not.
pub(super) struct Members<'a>(HashMap<&'a GroupKey, Vec<&'a Project>>);

impl<'a> Members<'a> {
    pub(super) fn of(map: &'a Snapshot) -> Members<'a> {
        let mut members: HashMap<&GroupKey, Vec<&Project>> = HashMap::new();
        for project in &map.projects {
            let group = &project.provenance.group;
            members.entry(group).or_default().push(project);
        }
        Members(members)
    }

    fn of_group(&self, key: &GroupKey) -> &[&'a Project] {
        self.0.get(key).map_or(&[], Vec::as_slice)
    }
}

/// The groups of `listed`, one section's projects, with the agents working `elsewhere` in Other,
/// each with its members among `members`, going by `names`, its rows and the groups themselves in
/// `sort`'s order.
pub(super) fn gather<'a>(
    listed: Vec<&'a Project>,
    elsewhere: Option<Elsewhere<'a>>,
    members: &Members<'a>,
    names: &HashMap<&GroupKey, String>,
    sort: SortOrder,
) -> Vec<Group<'a>> {
    let mut by_group: BTreeMap<&'a GroupKey, Vec<&'a Project>> = BTreeMap::new();
    for project in listed {
        let group = &project.provenance.group;
        by_group.entry(group).or_default().push(project);
    }
    if elsewhere.is_some() {
        by_group.entry(&OTHER).or_default();
    }
    let mut elsewhere = elsewhere;
    let mut groups: Vec<Group> = by_group
        .into_iter()
        .map(|(key, listed)| {
            let group_members = members.of_group(key);
            let agents = elsewhere.take_if(|_| *key == GroupKey::Other);
            let (checkout, children) = tree::grow(listed, agents, group_members, sort);
            Group {
                key,
                name: names.get(key).cloned().unwrap_or_default(),
                checkout,
                children,
                members: group_members.to_vec(),
            }
        })
        .collect();
    sort.order_groups(&mut groups);
    groups
}

/// What each group's line goes by, unique among the groups of `listed`, every project with a row
/// on the list, and Other, which may hold the agents working elsewhere alone; and apart from
/// Docker's line, while `docker` says Docker is here.
pub(super) fn names_of<'a>(listed: &[&'a Project], docker: bool) -> HashMap<&'a GroupKey, String> {
    let mut keys: Vec<&GroupKey> = listed
        .iter()
        .map(|project| &project.provenance.group)
        .chain([&OTHER])
        .collect();
    keys.sort();
    keys.dedup();
    let taken: &[&str] = if docker { &[DOCKER] } else { &[] };
    names::of_groups(keys, taken)
}

#[cfg(test)]
mod tests;
