//! Gathering one section's projects into their groups: each group with all its members on the
//! map, Other with the agents working elsewhere, and every group named apart from the rest.

use super::*;
use crate::dashboard::listing::fixtures::{agent, came_from, project, snapshot, upstream};
use crate::map::AgentStatus;

#[test]
fn a_groups_members_are_all_its_projects_on_the_map_with_a_row_or_not() {
    let grouped = |name: &str| came_from(upstream("github.com/me/app"), &[], project(name, 1));
    let map = snapshot(
        vec![grouped("app"), grouped("app-2"), grouped("small")],
        vec![],
    );
    let members = Members::of(&map);
    let listed = vec![&map.projects[0], &map.projects[1]];
    let names = names_of(&listed, false);
    let groups = gather(listed, None, &members, &names, SortOrder::Activity);
    let [group] = groups.as_slice() else {
        panic!("one group");
    };
    assert_eq!(group.name, "app");
    assert_eq!(group.cards().len(), 2, "the listed ones");
    assert_eq!(group.members.len(), 3, "and the one without a row");
}

#[test]
fn the_agents_working_elsewhere_stand_in_other_though_no_project_does() {
    let map = snapshot(vec![], vec![agent("/tmp/review", AgentStatus::Busy)]);
    let members = Members::of(&map);
    let elsewhere = Elsewhere {
        sessions: map.unplaced_agents.iter().collect(),
        subagents: Vec::new(),
    };
    let names = names_of(&[], false);
    let groups = gather(
        Vec::new(),
        Some(elsewhere),
        &members,
        &names,
        SortOrder::Activity,
    );
    let [group] = groups.as_slice() else {
        panic!("Other alone");
    };
    assert_eq!(
        (group.key, group.name.as_str()),
        (&GroupKey::Other, "Other")
    );
    assert!(matches!(group.children.as_slice(), [Node::Elsewhere(_)]));
}

#[test]
fn every_group_with_a_row_and_other_are_named_apart_from_each_other() {
    let map = snapshot(
        vec![
            came_from(upstream("github.com/vt/foundry"), &[], project("a", 1)),
            came_from(upstream("github.com/fork/foundry"), &[], project("b", 1)),
        ],
        vec![],
    );
    let listed: Vec<&Project> = map.projects.iter().collect();
    let names = names_of(&listed, false);
    let mut named: Vec<&str> = names.values().map(String::as_str).collect();
    named.sort();
    assert_eq!(named, ["Other", "fork/foundry", "vt/foundry"]);
}
