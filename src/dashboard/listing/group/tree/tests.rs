//! The tree of one group: what each card hangs from, when a checkout takes on a line of its own,
//! when the group's line stands for its lone checkout, and what each line holds.

use super::*;
use crate::dashboard::listing::fixtures::{
    agent, came_from, linked_worktree, main_worktree, project, quiet, upstream,
};
use crate::map::AgentStatus;

/// What `grow` makes of `listed`, with `members` on the map too, in the Activity sort: whether the
/// group's line stands for a checkout, and its rows, a card by its folder's own name and a
/// checkout's line as `name(row, row)`.
fn grown(listed: &[&Project], members: &[&Project]) -> (Option<String>, String) {
    let (checkout, nodes) = grow(listed.to_vec(), None, members, SortOrder::Activity);
    (checkout.map(|project| project.name.clone()), shown(&nodes))
}

fn shown(nodes: &[Node]) -> String {
    let words: Vec<String> = nodes
        .iter()
        .map(|node| match node {
            Node::Card { project, .. } => project.name.clone(),
            Node::Checkout(checkout) => format!("{}({})", checkout.name, shown(&checkout.children)),
            Node::Elsewhere(_) => "elsewhere".to_string(),
        })
        .collect();
    words.join(", ")
}

const ALICE: &str = "github.com/me/alice";

/// The clone of `ALICE` at `/code/<name>`, made from the projects at `made_from`, nearest first.
fn alice(name: &str, made_from: &[&str], minutes_ago: u64) -> Project {
    came_from(upstream(ALICE), made_from, project(name, minutes_ago))
}

#[test]
fn a_lone_checkout_with_copies_has_them_hang_from_the_groups_line_after_its_own_card() {
    let app = "/code/app/.git";
    let main = main_worktree(app, "main", project("app", 30));
    let fix = linked_worktree(app, "fix", project("app-fix", 1));
    assert_eq!(
        grown(&[&fix, &main], &[&fix, &main]),
        (Some("app".to_string()), "app, app-fix".to_string()),
        "the checkout's own card first, whatever its activity"
    );
}

#[test]
fn a_lone_checkout_with_no_copies_is_a_group_of_one_card() {
    let solo = alice("solo", &[], 1);
    assert_eq!(grown(&[&solo], &[&solo]), (None, "solo".to_string()));
}

#[test]
fn two_clones_of_one_upstream_stand_side_by_side_and_one_with_copies_takes_on_its_line() {
    let first = alice("alice", &[], 5);
    let wt = alice("alice-wt", &["/code/alice"], 1);
    let second = alice("alice-2", &[], 10);
    assert_eq!(
        grown(&[&first, &wt, &second], &[&first, &wt, &second]),
        (None, "alice(alice, alice-wt), alice-2".to_string())
    );
}

#[test]
fn a_copy_of_a_copy_hangs_three_lines_deep() {
    let first = alice("alice", &[], 5);
    let other = alice("alice-2", &[], 10);
    let capture = alice("capture", &["/code/alice"], 4);
    let review = alice("review", &["/code/capture", "/code/alice"], 1);
    let nodes = grow(
        vec![&first, &capture, &review, &other],
        None,
        &[&first, &capture, &review, &other],
        SortOrder::Activity,
    )
    .1;
    assert_eq!(
        shown(&nodes),
        "alice(alice, capture(capture, review)), alice-2"
    );
}

#[test]
fn a_copy_of_something_not_listed_hangs_from_its_nearest_ancestor_that_is() {
    let first = alice("alice", &[], 5);
    let other = alice("alice-2", &[], 10);
    // Made from a scratch clone with no card, itself made from alice.
    let review = alice("review", &["/tmp/s/clone", "/code/alice"], 1);
    // Made from Cargo's git cache, which no card stands for.
    let cached = alice("cached", &["/Users/me/.cargo/git/db/alice-1a2b"], 1);
    assert_eq!(
        grown(
            &[&first, &other, &review, &cached],
            &[&first, &other, &review, &cached]
        ),
        (None, "alice(alice, review), cached, alice-2".to_string())
    );
}

#[test]
fn copies_that_would_hang_from_each_other_in_a_loop_have_it_cut_at_the_first_by_path() {
    let a = alice("a", &["/code/b"], 1);
    let b = alice("b", &["/code/a"], 1);
    assert_eq!(
        grown(&[&b, &a], &[&a, &b]),
        (Some("a".to_string()), "a, b".to_string()),
        "a hangs from nothing, so it stands alone atop the group, b hanging from it"
    );
}

#[test]
fn a_checkouts_line_holds_every_copy_of_it_on_the_map_listed_or_not() {
    let first = alice("alice", &[], 5);
    let wt = alice("alice-wt", &["/code/alice"], 1);
    let unlisted = alice("alice-small", &["/code/alice-wt", "/code/alice"], 1);
    let other = alice("alice-2", &[], 10);
    let (_, nodes) = grow(
        vec![&first, &wt, &other],
        None,
        &[&first, &wt, &unlisted, &other],
        SortOrder::Activity,
    );
    let Some(Node::Checkout(line)) = nodes.first() else {
        panic!("alice's line first: {}", shown(&nodes));
    };
    let members: Vec<&str> = line.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(members, ["alice", "alice-wt", "alice-small"]);
}

#[test]
fn the_agents_working_elsewhere_stand_first_in_other_and_keep_its_line_its_own() {
    let notes = quiet("notes");
    let session = agent("/tmp/review", AgentStatus::Busy);
    let elsewhere = Elsewhere {
        sessions: vec![&session],
        subagents: Vec::new(),
    };
    let (checkout, nodes) = grow(vec![&notes], Some(elsewhere), &[&notes], SortOrder::Name);
    assert_eq!(checkout, None);
    assert_eq!(shown(&nodes), "elsewhere, notes");
}
