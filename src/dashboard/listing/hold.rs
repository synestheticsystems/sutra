//! Holding the list still. While the pointer is over the list, or while the Disk sort waits for
//! the first sizing to finish, the groups that stay listed keep their places, Docker's among them,
//! and so do the rows under each line, folded shut or open. Holding only pauses re-sorting: a card
//! that arrives appears at once, in the place the sort gives it under its line, and a card that
//! leaves is gone at once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Checkout, Group, Node, SectionRow};
use crate::map::Project;

/// What holds the list still, and the order it holds.
#[derive(Debug, Default)]
pub(super) struct Hold {
    /// The pointer is over the list. Up to four snapshots a second arrive, and a card sliding
    /// into place could take a click meant for the card that was there. The window tells only of
    /// the pointer coming and going, over the list and over what stands in its place alike, so
    /// this is kept apart from the map: a pointer that came while the first scan ran, or stayed
    /// while an engine started afresh, is still there when the cards arrive.
    pointer_over: bool,
    /// The Disk sort was chosen before the first sizing finished, when each size that lands would
    /// move the cards.
    until_sized: bool,
    /// The cards on screen, in their order, while anything holds the list: from the first map
    /// that shows them, should the list be held before any map.
    order: Option<Order>,
}

/// The cards of each section of the list, in the order on screen, those under a folded line where
/// it stands, and Docker's by the socket Docker is read through. The cards under a line stand
/// together, so the order of the groups, and of the rows under each line, is the order of their
/// cards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Order {
    /// The cards above the quiet row, or every card while a filter is typed.
    pub(super) main: Vec<PathBuf>,
    /// The quiet cards below the row, while it is open.
    pub(super) quiet: Vec<PathBuf>,
}

/// A section of the list, whose cards are held apart from the other's, so a project that joins
/// the focus or leaves it moves between sections at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Main,
    Quiet,
}

impl Hold {
    /// Whether anything holds the list still: the pointer over it, or the Disk sort waiting for
    /// the first sizing.
    pub(super) fn is_held(&self) -> bool {
        self.pointer_over || self.until_sized
    }

    /// The pointer came over the list.
    pub(super) fn pointer_entered(&mut self) {
        self.pointer_over = true;
    }

    /// The pointer left the list, or the list is no longer drawn.
    pub(super) fn pointer_left(&mut self) {
        self.pointer_over = false;
        self.release_when_free();
    }

    /// Hold `sorted`, the Disk sort's order now, until the first sizing finishes.
    pub(super) fn hold_until_sized(&mut self, sorted: Order) {
        self.until_sized = true;
        self.order = Some(sorted);
    }

    /// Stop waiting for the first sizing: it finished, or the user chose another sort.
    pub(super) fn stop_waiting_for_sizes(&mut self) {
        self.until_sized = false;
        self.release_when_free();
    }

    /// While something holds the list, hold `order` from now on: what the list shows after a
    /// change, so a card that arrived keeps the place it appeared in, the order of a sort the user
    /// just chose, or the first order a map gives a list held before it.
    pub(super) fn keep(&mut self, order: Order) {
        if self.is_held() {
            self.order = Some(order);
        }
    }

    /// A new engine started, whose map is sorted from scratch: hold nothing of the old map's
    /// order, and stop waiting for its sizes. The pointer has not moved, so it still holds the list
    /// once the new map arrives.
    pub(super) fn forget_map(&mut self) {
        self.until_sized = false;
        self.order = None;
    }

    fn release_when_free(&mut self) {
        if !self.is_held() {
            self.order = None;
        }
    }

    /// `sorted`, one section's rows as the sort lays them out, with the cards the held order lists
    /// put back in that order: each group in the place its first held card gives it, and Docker's
    /// in the place of its card, then under each line the rows beside each other, each in the place
    /// of its first held card, the line's own card staying first. Each group or row new to the
    /// section keeps the place the sort gives it, and one gone from it leaves no gap. Unheld,
    /// `sorted` as it is.
    pub(super) fn arrange<'a>(
        &self,
        section: Section,
        sorted: Vec<SectionRow<'a>>,
    ) -> Vec<SectionRow<'a>> {
        let Some(order) = &self.order else {
            return sorted;
        };
        let held = match section {
            Section::Main => &order.main,
            Section::Quiet => &order.quiet,
        };
        let places = Places(
            held.iter()
                .enumerate()
                .map(|(place, path)| (path.as_path(), place))
                .collect(),
        );
        let row_place = |row: &SectionRow| match row {
            SectionRow::Group(group) => places.first_of(&group.cards()),
            SectionRow::Docker(docker) => places.0.get(docker.socket.as_path()).copied(),
        };
        keep_places(sorted, row_place)
            .into_iter()
            .map(|row| match row {
                SectionRow::Group(group) => {
                    let own_card = usize::from(group.checkout.is_some());
                    SectionRow::Group(Group {
                        children: places.arrange_rows(group.children, own_card),
                        ..group
                    })
                }
                docker @ SectionRow::Docker(_) => docker,
            })
            .collect()
    }
}

/// Where each card stood in the held order.
struct Places<'p>(HashMap<&'p Path, usize>);

impl Places<'_> {
    /// The first held place among `cards`, `None` for cards new to the section.
    fn first_of(&self, cards: &[&Project]) -> Option<usize> {
        let held = cards
            .iter()
            .filter_map(|card| self.0.get(card.path.as_path()));
        held.min().copied()
    }

    /// `rows`, the rows of one line, back in their held places, past the first `pinned`, a line's
    /// own card, which stays first, and the rows under each of them in theirs.
    fn arrange_rows<'a>(&self, rows: Vec<Node<'a>>, pinned: usize) -> Vec<Node<'a>> {
        let mut rows = rows.into_iter();
        let first: Vec<Node> = rows.by_ref().take(pinned).collect();
        let rest = keep_places(rows.collect(), |row: &Node| self.first_of(&row.cards()));
        let arranged = rest.into_iter().map(|row| match row {
            Node::Checkout(checkout) => Node::Checkout(Checkout {
                children: self.arrange_rows(checkout.children, 1),
                ..checkout
            }),
            row => row,
        });
        first.into_iter().chain(arranged).collect()
    }
}

/// `sorted`, with the places it gives the items `held_place` finds a held place for filled by
/// those items in the order of their held places. Each item with none keeps the place the sort
/// gives it.
fn keep_places<T>(sorted: Vec<T>, held_place: impl Fn(&T) -> Option<usize>) -> Vec<T> {
    let mut held = Vec::new();
    // Each place in `sorted`: an item new to the section, or `None` for one of the held items.
    let mut places = Vec::with_capacity(sorted.len());
    for item in sorted {
        match held_place(&item) {
            Some(place) => {
                held.push((place, item));
                places.push(None);
            }
            None => places.push(Some(item)),
        }
    }
    held.sort_by_key(|(place, _)| *place);
    let mut held = held.into_iter().map(|(_, item)| item);
    places
        .into_iter()
        .map(|new| {
            new.or_else(|| held.next())
                .expect("one held item for each held place")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::shown;
    use super::super::{Naming, Row};
    use super::*;
    use crate::map::GroupKey;
    use crate::map::test_support::project_at;

    fn project(name: &str) -> Project {
        project_at(PathBuf::from("/code").join(name))
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| PathBuf::from("/code").join(name))
            .collect()
    }

    /// A list held by the pointer, the main section's cards on screen being `on_screen`.
    fn holding(on_screen: &[&str]) -> Hold {
        let mut hold = Hold::default();
        hold.pointer_entered();
        hold.keep(Order {
            main: paths(on_screen),
            quiet: Vec::new(),
        });
        hold
    }

    /// The main section's `sorted` groups as `hold` arranges them, as the rows show them.
    fn arranged(hold: &Hold, sorted: Vec<Group>) -> Vec<String> {
        let sorted = sorted.into_iter().map(SectionRow::Group).collect();
        let arranged = hold.arrange(Section::Main, sorted).into_iter();
        shown(&arranged.map(Row::from).collect::<Vec<_>>())
    }

    /// The card of `project`, going by its folder's name.
    fn card(project: &Project) -> Node<'_> {
        Node::Card {
            project,
            naming: Naming::Folder(project.name.clone()),
        }
    }

    /// The line of `project`, holding its own card first, then `copies`.
    fn line<'a>(project: &'a Project, copies: Vec<Node<'a>>) -> Node<'a> {
        Node::Checkout(Checkout {
            project,
            name: project.name.clone(),
            children: std::iter::once(card(project)).chain(copies).collect(),
            members: Vec::new(),
        })
    }

    /// The group `name`, whose line holds `children`.
    fn group<'a>(name: &'static str, children: Vec<Node<'a>>) -> Group<'a> {
        // Kept for the rest of the test run, as a group borrows its key from the map.
        let key = Box::leak(Box::new(GroupKey::Upstream(name.to_string())));
        Group {
            key,
            name: name.to_string(),
            checkout: None,
            children,
            members: Vec::new(),
        }
    }

    /// The group of the one card `project`.
    fn alone(project: &Project) -> Group<'_> {
        let name = Box::leak(project.name.clone().into_boxed_str());
        group(name, vec![card(project)])
    }

    #[test]
    fn held_groups_keep_their_order_however_the_sort_now_ranks_them() {
        let [a, b, c] = ["a", "b", "c"].map(project);
        let hold = holding(&["a", "b", "c"]);
        assert_eq!(
            arranged(&hold, vec![alone(&c), alone(&a), alone(&b)]),
            ["a: a", "b: b", "c: c"]
        );
    }

    #[test]
    fn a_new_group_takes_the_place_the_sort_gives_it_among_held_ones() {
        let [a, b, new] = ["a", "b", "new"].map(project);
        let hold = holding(&["a", "b"]);
        assert_eq!(
            arranged(&hold, vec![alone(&new), alone(&b), alone(&a)]),
            ["new: new", "a: a", "b: b"]
        );
        assert_eq!(
            arranged(&hold, vec![alone(&b), alone(&new), alone(&a)]),
            ["a: a", "new: new", "b: b"]
        );
    }

    #[test]
    fn a_group_gone_from_the_section_leaves_no_gap() {
        let [a, c] = ["a", "c"].map(project);
        let hold = holding(&["a", "b", "c"]);
        assert_eq!(
            arranged(&hold, vec![alone(&c), alone(&a)]),
            ["a: a", "c: c"]
        );
    }

    #[test]
    fn held_rows_under_a_line_keep_their_order_and_a_card_new_to_it_arrives_within_it() {
        let [a, a_fix, b, b_new, b_old] = ["a", "a-fix", "b", "b-new", "b-old"].map(project);
        let hold = holding(&["a", "a-fix", "b", "b-old"]);
        // The sort now puts b's group first and its old card before its new one, where a flat
        // hold would have put the new card between a's cards.
        let sorted = vec![
            group("b", vec![card(&b_old), card(&b_new), card(&b)]),
            group("a", vec![card(&a_fix), card(&a)]),
        ];
        assert_eq!(
            arranged(&hold, sorted),
            ["a: a, a-fix", "b: b, b-new, b-old"]
        );
    }

    #[test]
    fn a_lines_own_card_stays_first_wherever_the_held_order_had_it() {
        let [app, copy, other] = ["app", "copy", "other"].map(project);
        // The copy stood before the checkout it was made from until that one took on its line.
        let hold = holding(&["copy", "app", "other"]);
        let sorted = vec![group(
            "app",
            vec![line(&app, vec![card(&copy)]), card(&other)],
        )];
        assert_eq!(arranged(&hold, sorted), ["app: app(app, copy), other"]);
    }

    #[test]
    fn the_rows_under_a_checkouts_line_keep_their_held_places_too() {
        let [app, fix, wip] = ["app", "fix", "wip"].map(project);
        let hold = holding(&["app", "wip", "fix"]);
        let sorted = vec![group("app", vec![line(&app, vec![card(&fix), card(&wip)])])];
        assert_eq!(arranged(&hold, sorted), ["app: app(app, wip, fix)"]);
    }

    #[test]
    fn the_pointer_and_the_disk_sort_each_keep_the_list_held_until_both_let_go() {
        let mut hold = Hold::default();
        hold.hold_until_sized(Order::default());
        hold.pointer_entered();
        hold.stop_waiting_for_sizes();
        assert!(hold.is_held(), "the pointer still holds it");
        hold.pointer_left();
        assert!(!hold.is_held());

        hold.pointer_entered();
        hold.hold_until_sized(Order::default());
        hold.pointer_left();
        assert!(hold.is_held(), "the first sizing still holds it");
        hold.stop_waiting_for_sizes();
        assert!(!hold.is_held());
    }

    #[test]
    fn an_unheld_list_keeps_the_sorts_order() {
        let [a, b] = ["a", "b"].map(project);
        assert_eq!(
            arranged(&Hold::default(), vec![alone(&b), alone(&a)]),
            ["b: b", "a: a"]
        );
    }
}
