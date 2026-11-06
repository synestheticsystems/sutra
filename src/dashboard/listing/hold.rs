//! Holding the list still. While the pointer is over the list, or while the Disk sort waits for
//! the first sizing to finish, the cards that stay listed keep their places. Holding only pauses
//! re-sorting: a card that arrives appears at once, in the place the sort gives it, and a card
//! that leaves is gone at once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// The cards of each section of the list, in the order on screen.
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

    /// `sorted`, one section's cards in the sort's order, with the cards the held order lists put
    /// back in that order. Each card new to the section keeps the place the sort gives it, and a
    /// card gone from it leaves no gap. Unheld, `sorted` as it is.
    pub(super) fn arrange<'a>(
        &self,
        section: Section,
        sorted: Vec<&'a Project>,
    ) -> Vec<&'a Project> {
        let Some(order) = &self.order else {
            return sorted;
        };
        let held = match section {
            Section::Main => &order.main,
            Section::Quiet => &order.quiet,
        };
        keep_places(sorted, held)
    }
}

/// Fill the places `sorted` gives the cards `held` lists with those cards in `held`'s order.
fn keep_places<'a>(sorted: Vec<&'a Project>, held: &[PathBuf]) -> Vec<&'a Project> {
    let place: HashMap<&Path, usize> = held
        .iter()
        .enumerate()
        .map(|(place, path)| (path.as_path(), place))
        .collect();
    let mut kept: Vec<&Project> = sorted
        .iter()
        .copied()
        .filter(|project| place.contains_key(project.path.as_path()))
        .collect();
    kept.sort_by_key(|project| place[project.path.as_path()]);
    let mut kept = kept.into_iter();
    sorted
        .into_iter()
        .map(|project| {
            if place.contains_key(project.path.as_path()) {
                kept.next().expect("one held card for each held place")
            } else {
                project
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn names<'a>(projects: &[&'a Project]) -> Vec<&'a str> {
        projects
            .iter()
            .map(|project| project.name.as_str())
            .collect()
    }

    #[test]
    fn held_cards_keep_their_order_however_the_sort_now_ranks_them() {
        let [a, b, c] = ["a", "b", "c"].map(project);
        assert_eq!(
            names(&keep_places(vec![&c, &a, &b], &paths(&["a", "b", "c"]))),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn a_new_card_takes_the_place_the_sort_gives_it_among_held_ones() {
        let [a, b, new] = ["a", "b", "new"].map(project);
        assert_eq!(
            names(&keep_places(vec![&new, &b, &a], &paths(&["a", "b"]))),
            ["new", "a", "b"]
        );
        assert_eq!(
            names(&keep_places(vec![&b, &new, &a], &paths(&["a", "b"]))),
            ["a", "new", "b"]
        );
    }

    #[test]
    fn a_card_gone_from_the_section_leaves_no_gap() {
        let [a, c] = ["a", "c"].map(project);
        assert_eq!(
            names(&keep_places(vec![&c, &a], &paths(&["a", "b", "c"]))),
            ["a", "c"]
        );
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
        let hold = Hold::default();
        assert_eq!(
            names(&hold.arrange(Section::Main, vec![&b, &a])),
            ["b", "a"]
        );
    }
}
