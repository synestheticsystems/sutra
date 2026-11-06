//! Holding the list still. While the pointer is over the list, or while the Disk sort waits for
//! the first sizing to finish, the cards that stay listed keep their places, and the Activity
//! sort's groups keep theirs, folded shut or open. Holding only pauses re-sorting: a card that
//! arrives appears at once, in the place the sort gives it, within its group if it has one, and a
//! card that leaves is gone at once, a group it leaves with one card giving way to that card.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::SectionRow;
use super::repository::Group;
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

/// The cards of each section of the list, in the order on screen, a folded group's where its
/// header stands. A group's cards stand together, so the order of the groups and the cards
/// standing alone is the order of their cards.
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
    /// put back in that order: each card alone in its held place, each group in the place its first
    /// held card gives it, then the cards within each group. Each card or group new to the section
    /// keeps the place the sort gives it, and one gone from it leaves no gap. Unheld, `sorted` as
    /// it is.
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
        let place: HashMap<&Path, usize> = held
            .iter()
            .enumerate()
            .map(|(place, path)| (path.as_path(), place))
            .collect();
        let card_place = |card: &&Project| place.get(card.path.as_path()).copied();
        let row_place = |row: &SectionRow| match row {
            SectionRow::Project(card) => card_place(card),
            SectionRow::Repository(group) => group.cards.iter().filter_map(card_place).min(),
        };
        keep_places(sorted, row_place)
            .into_iter()
            .map(|row| match row {
                SectionRow::Repository(group) => SectionRow::Repository(Group {
                    cards: keep_places(group.cards, card_place),
                    ..group
                }),
                SectionRow::Project(card) => SectionRow::Project(card),
            })
            .collect()
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
    use super::super::Row;
    use super::super::fixtures::shown;
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

    /// The main section's `sorted` rows as `hold` arranges them, as the rows show them.
    fn arranged_rows(hold: &Hold, sorted: Vec<SectionRow>) -> Vec<String> {
        let arranged = hold.arrange(Section::Main, sorted).into_iter();
        shown(&arranged.map(Row::from).collect::<Vec<_>>())
    }

    /// The main section's `sorted` cards, each alone, as `hold` arranges them.
    fn arranged(hold: &Hold, sorted: Vec<&Project>) -> Vec<String> {
        arranged_rows(hold, sorted.into_iter().map(SectionRow::Project).collect())
    }

    /// The group of the repository `name`, listing `cards`, known by its name in place of a git
    /// dir.
    fn group<'a>(name: &'static str, cards: Vec<&'a Project>) -> SectionRow<'a> {
        SectionRow::Repository(Group {
            common_dir: Path::new(name),
            name: name.to_string(),
            folder: Path::new(name),
            cards,
            worktrees: Vec::new(),
        })
    }

    #[test]
    fn held_cards_keep_their_order_however_the_sort_now_ranks_them() {
        let [a, b, c] = ["a", "b", "c"].map(project);
        let hold = holding(&["a", "b", "c"]);
        assert_eq!(arranged(&hold, vec![&c, &a, &b]), ["a", "b", "c"]);
    }

    #[test]
    fn a_new_card_takes_the_place_the_sort_gives_it_among_held_ones() {
        let [a, b, new] = ["a", "b", "new"].map(project);
        let hold = holding(&["a", "b"]);
        assert_eq!(arranged(&hold, vec![&new, &b, &a]), ["new", "a", "b"]);
        assert_eq!(arranged(&hold, vec![&b, &new, &a]), ["a", "new", "b"]);
    }

    #[test]
    fn a_card_gone_from_the_section_leaves_no_gap() {
        let [a, c] = ["a", "c"].map(project);
        let hold = holding(&["a", "b", "c"]);
        assert_eq!(arranged(&hold, vec![&c, &a]), ["a", "c"]);
    }

    #[test]
    fn held_groups_keep_their_order_and_a_card_new_to_one_arrives_within_it() {
        let [a, a_fix, b, b_new] = ["a", "a-fix", "b", "b-new"].map(project);
        let hold = holding(&["a", "a-fix", "b"]);
        // The sort now puts b's group first, its new card second, where a flat hold would have
        // put it between a's cards.
        let sorted = vec![group("b", vec![&b, &b_new]), group("a", vec![&a_fix, &a])];
        assert_eq!(arranged_rows(&hold, sorted), ["a: a, a-fix", "b: b, b-new"]);
    }

    #[test]
    fn held_groups_and_cards_alone_keep_their_order_among_each_other() {
        let [a, a_fix, notes, b, b_fix] = ["a", "a-fix", "notes", "b", "b-fix"].map(project);
        let hold = holding(&["a", "a-fix", "notes", "b", "b-fix"]);
        let sorted = vec![
            group("b", vec![&b, &b_fix]),
            SectionRow::Project(&notes),
            group("a", vec![&a_fix, &a]),
        ];
        assert_eq!(
            arranged_rows(&hold, sorted),
            ["a: a, a-fix", "notes", "b: b, b-fix"]
        );
    }

    #[test]
    fn a_new_group_takes_the_place_the_sort_gives_it_among_held_ones() {
        let [a, a_fix, b, c, c_fix] = ["a", "a-fix", "b", "c", "c-fix"].map(project);
        let hold = holding(&["a", "a-fix", "b"]);
        let sorted = vec![
            group("c", vec![&c, &c_fix]),
            SectionRow::Project(&b),
            group("a", vec![&a, &a_fix]),
        ];
        assert_eq!(
            arranged_rows(&hold, sorted),
            ["c: c, c-fix", "a: a, a-fix", "b"]
        );
    }

    #[test]
    fn a_group_made_or_unmade_under_the_pointer_stays_where_its_cards_stood() {
        let [notes, a, a_fix, site] = ["notes", "a", "a-fix", "site"].map(project);
        // a stood alone until a worktree of its repository joined the section.
        let alone = holding(&["notes", "a", "site"]);
        let sorted = vec![
            group("a", vec![&a, &a_fix]),
            SectionRow::Project(&notes),
            SectionRow::Project(&site),
        ];
        assert_eq!(
            arranged_rows(&alone, sorted),
            ["notes", "a: a, a-fix", "site"]
        );
        // a-fix left the section, which leaves a alone where its group stood.
        let grouped = holding(&["notes", "a", "a-fix", "site"]);
        let sorted = vec![
            SectionRow::Project(&site),
            SectionRow::Project(&a),
            SectionRow::Project(&notes),
        ];
        assert_eq!(arranged_rows(&grouped, sorted), ["notes", "a", "site"]);
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
        assert_eq!(arranged(&Hold::default(), vec![&b, &a]), ["b", "a"]);
    }
}
