//! How the user has arranged the list (its sort, the filter field, the quiet projects shown or
//! not) and what holds its order still. Every rule for when cards change places lives here and in
//! `hold`.

use std::time::SystemTime;

use super::Listing;
use super::filter::Query;
use super::hold::Hold;
use crate::dashboard::SortOrder;
use crate::dashboard::cleaning::Cleaning;
use crate::map::Snapshot;

/// Kept for the session only: every launch opens newest activity first, unfiltered, with the
/// quiet projects behind their row.
#[derive(Debug, Default)]
pub(in crate::dashboard) struct Arrangement {
    sort: SortOrder,
    /// The filter field's text; `None` while the field is hidden.
    filter: Option<String>,
    quiet_shown: bool,
    hold: Hold,
    /// The first sizing of the map's folders has finished. Only the first holds the Disk sort,
    /// because only then do sizes land by the hundred.
    sized_once: bool,
}

impl Arrangement {
    pub(in crate::dashboard) fn sort(&self) -> SortOrder {
        self.sort
    }

    /// The filter field's text, `None` while the field is hidden.
    pub(in crate::dashboard) fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    pub(super) fn query(&self) -> Option<Query> {
        self.filter.as_deref().and_then(Query::new)
    }

    pub(super) fn quiet_shown(&self) -> bool {
        self.quiet_shown
    }

    pub(super) fn hold(&self) -> &Hold {
        &self.hold
    }

    /// A new map arrived, with `cleaning` at work. Cards keep their places where the list is held,
    /// and the first sizing to finish lets go of the Disk sort.
    pub(in crate::dashboard) fn observe(
        &mut self,
        snapshot: &Snapshot,
        cleaning: &Cleaning,
        now: SystemTime,
    ) {
        if !self.sized_once && snapshot.sizing().is_finished() {
            self.sized_once = true;
            self.hold.stop_waiting_for_sizes();
        }
        self.keep_on_screen(Some(snapshot), cleaning, now);
    }

    /// The user chose `sort`, which orders the list now. Chosen before the first sizing finished,
    /// the Disk sort keeps this order until it does.
    pub(in crate::dashboard) fn set_sort(
        &mut self,
        sort: SortOrder,
        snapshot: &Snapshot,
        cleaning: &Cleaning,
        now: SystemTime,
    ) {
        self.sort = sort;
        let sorted = Listing::sorted(snapshot, self, cleaning, now).order();
        if sort == SortOrder::Disk && !self.sized_once {
            self.hold.hold_until_sized(sorted);
        } else {
            self.hold.stop_waiting_for_sizes();
            self.hold.keep(sorted);
        }
    }

    pub(in crate::dashboard) fn show_filter(&mut self) {
        self.filter.get_or_insert_with(String::new);
    }

    pub(in crate::dashboard) fn set_filter_text(
        &mut self,
        text: String,
        snapshot: Option<&Snapshot>,
        cleaning: &Cleaning,
        now: SystemTime,
    ) {
        self.filter = Some(text);
        self.keep_on_screen(snapshot, cleaning, now);
    }

    /// Clear the filter and hide its field.
    pub(in crate::dashboard) fn hide_filter(
        &mut self,
        snapshot: Option<&Snapshot>,
        cleaning: &Cleaning,
        now: SystemTime,
    ) {
        self.filter = None;
        self.keep_on_screen(snapshot, cleaning, now);
    }

    /// Open or close the quiet projects' row.
    pub(in crate::dashboard) fn toggle_quiet(
        &mut self,
        snapshot: Option<&Snapshot>,
        cleaning: &Cleaning,
        now: SystemTime,
    ) {
        self.quiet_shown = !self.quiet_shown;
        self.keep_on_screen(snapshot, cleaning, now);
    }

    /// The pointer came over the list, which shows `snapshot`, or no map yet while the first scan
    /// runs.
    pub(in crate::dashboard) fn pointer_entered(
        &mut self,
        snapshot: Option<&Snapshot>,
        cleaning: &Cleaning,
        now: SystemTime,
    ) {
        self.hold.pointer_entered();
        self.keep_on_screen(snapshot, cleaning, now);
    }

    /// The pointer left the list, or the window no longer shows it.
    pub(in crate::dashboard) fn pointer_left(&mut self) {
        self.hold.pointer_left();
    }

    /// A new engine started: its map is sorted from scratch and sized from the start. A pointer
    /// over the list stays there, and holds the new map's cards from the first that arrives.
    pub(in crate::dashboard) fn forget_map(&mut self) {
        self.hold.forget_map();
        self.sized_once = false;
    }

    /// While the list is held, take what it shows now as the order to hold, so a card that just
    /// appeared keeps its place too. Held before any map, the list takes the order of the first.
    fn keep_on_screen(
        &mut self,
        snapshot: Option<&Snapshot>,
        cleaning: &Cleaning,
        now: SystemTime,
    ) {
        if let Some(snapshot) = snapshot.filter(|_| self.hold.is_held()) {
            let on_screen = Listing::new(snapshot, self, cleaning, now).order();
            self.hold.keep(on_screen);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::fixtures::{
        agent, linked_worktree, main_worktree, now, project, quiet, shown, snapshot,
    };
    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::map::test_support::{cargo_target, measured, node_modules};
    use crate::map::{AgentStatus, Measurement, Project};

    fn listed(map: &Snapshot, arrangement: &Arrangement) -> Vec<String> {
        shown(&Listing::new(map, arrangement, &cleaning(), now()).rows)
    }

    /// An arrangement with the pointer over the list `map` shows.
    fn pointed_at(map: &Snapshot) -> Arrangement {
        let mut arrangement = Arrangement::default();
        arrangement.observe(map, &cleaning(), now());
        arrangement.pointer_entered(Some(map), &cleaning(), now());
        arrangement
    }

    #[test]
    fn while_the_pointer_is_over_the_list_new_activity_moves_no_card() {
        let before = snapshot(vec![project("a", 10), project("b", 20)], vec![]);
        let mut arrangement = pointed_at(&before);

        let b_active = snapshot(vec![project("b", 0), project("a", 10)], vec![]);
        arrangement.observe(&b_active, &cleaning(), now());
        assert_eq!(listed(&b_active, &arrangement), ["a", "b"]);

        arrangement.pointer_left();
        assert_eq!(listed(&b_active, &arrangement), ["b", "a"], "re-sorted");
    }

    #[test]
    fn while_the_pointer_is_over_the_list_no_group_moves_and_a_newcomer_joins_its_own() {
        let (a, b) = ("/code/a/.git", "/code/b/.git");
        let worktrees = || {
            vec![
                main_worktree(a, "main", project("a", 10)),
                linked_worktree(a, "fix", project("a-fix", 10)),
                main_worktree(b, "main", project("b", 20)),
                linked_worktree(b, "fix", project("b-fix", 20)),
            ]
        };
        let before = snapshot(worktrees(), vec![]);
        let mut arrangement = pointed_at(&before);
        assert_eq!(
            listed(&before, &arrangement),
            ["a: a, a-fix", "b: b, b-fix"]
        );

        // A new worktree of b's at work now, which puts b's group first in the sort.
        let mut b_active = worktrees();
        b_active.insert(0, linked_worktree(b, "new", project("b-new", 0)));
        let b_active = snapshot(b_active, vec![]);
        arrangement.observe(&b_active, &cleaning(), now());
        assert_eq!(
            listed(&b_active, &arrangement),
            ["a: a, a-fix", "b: b, b-new, b-fix"]
        );

        arrangement.pointer_left();
        assert_eq!(
            listed(&b_active, &arrangement),
            ["b: b, b-new, b-fix", "a: a, a-fix"],
            "re-sorted"
        );
    }

    #[test]
    fn a_pointer_that_came_over_the_list_during_the_first_scan_holds_the_cards_that_arrive() {
        let mut arrangement = Arrangement::default();
        arrangement.pointer_entered(None, &cleaning(), now());

        let first = snapshot(vec![project("a", 10), project("b", 20)], vec![]);
        arrangement.observe(&first, &cleaning(), now());
        assert_eq!(listed(&first, &arrangement), ["a", "b"]);

        let b_active = snapshot(vec![project("b", 0), project("a", 10)], vec![]);
        arrangement.observe(&b_active, &cleaning(), now());
        assert_eq!(listed(&b_active, &arrangement), ["a", "b"], "held");
    }

    #[test]
    fn a_pointer_over_the_list_as_the_engine_restarts_holds_the_new_engines_cards() {
        let before = snapshot(vec![project("a", 10), project("b", 20)], vec![]);
        let mut arrangement = pointed_at(&before);

        arrangement.forget_map();
        let first = snapshot(vec![project("b", 0), project("a", 10)], vec![]);
        arrangement.observe(&first, &cleaning(), now());
        assert_eq!(listed(&first, &arrangement), ["b", "a"], "sorted afresh");

        let a_active = snapshot(vec![project("a", 0), project("b", 10)], vec![]);
        arrangement.observe(&a_active, &cleaning(), now());
        assert_eq!(listed(&a_active, &arrangement), ["b", "a"], "held");
    }

    #[test]
    fn a_card_that_stops_qualifying_leaves_at_once_while_the_pointer_is_over_the_list() {
        let mut working = project("b", 2);
        working.agents.push(agent("/code/b", AgentStatus::Busy));
        let before = snapshot(vec![project("a", 1), working, project("c", 3)], vec![]);
        let mut arrangement = pointed_at(&before);

        // b's agent ended, and its last activity is long past: it leaves the focus.
        let after = snapshot(vec![project("a", 1), project("c", 3), quiet("b")], vec![]);
        arrangement.observe(&after, &cleaning(), now());
        assert_eq!(listed(&after, &arrangement), ["a", "c", "quiet:1"]);
    }

    #[test]
    fn a_newcomer_arrives_at_once_in_its_sorted_place_and_then_holds_it() {
        let before = snapshot(vec![project("a", 20), project("b", 30)], vec![]);
        let mut arrangement = pointed_at(&before);

        let arrived = snapshot(
            vec![project("new", 10), project("b", 15), project("a", 20)],
            vec![],
        );
        arrangement.observe(&arrived, &cleaning(), now());
        assert_eq!(listed(&arrived, &arrangement), ["new", "a", "b"]);

        let later = snapshot(
            vec![project("b", 0), project("a", 0), project("new", 25)],
            vec![],
        );
        arrangement.observe(&later, &cleaning(), now());
        assert_eq!(listed(&later, &arrangement), ["new", "a", "b"]);
    }

    #[test]
    fn opening_the_quiet_row_under_the_pointer_moves_no_card_above_it() {
        let before = snapshot(vec![project("a", 1), project("b", 2), quiet("q")], vec![]);
        let mut arrangement = pointed_at(&before);
        let b_active = snapshot(vec![project("b", 0), project("a", 1), quiet("q")], vec![]);
        arrangement.observe(&b_active, &cleaning(), now());

        arrangement.toggle_quiet(Some(&b_active), &cleaning(), now());
        assert_eq!(listed(&b_active, &arrangement), ["a", "b", "quiet:1", "q"]);
    }

    /// A project in focus whose only build folder is measured as `target`, and whose own files
    /// are `own_files`.
    fn sized(name: &str, target: Measurement, own_files: Measurement) -> Project {
        Project {
            artifacts: vec![cargo_target(
                PathBuf::from("/code").join(name).join("target"),
                target,
            )],
            own_files,
            ..project(name, 1)
        }
    }

    #[test]
    fn the_disk_sort_holds_its_order_until_the_first_sizing_finishes() {
        let pending = Measurement::Measuring;
        let early = snapshot(
            vec![
                sized("small", measured(10), pending),
                sized("large", pending, pending),
            ],
            vec![],
        );
        let mut arrangement = Arrangement::default();
        arrangement.observe(&early, &cleaning(), now());
        arrangement.set_sort(SortOrder::Disk, &early, &cleaning(), now());
        assert_eq!(listed(&early, &arrangement), ["small", "large"]);

        let landing = snapshot(
            vec![
                sized("small", measured(10), pending),
                sized("large", measured(500), pending),
            ],
            vec![],
        );
        arrangement.observe(&landing, &cleaning(), now());
        assert_eq!(listed(&landing, &arrangement), ["small", "large"], "held");

        let done = snapshot(
            vec![
                sized("small", measured(10), measured(5)),
                sized("large", measured(500), measured(5)),
            ],
            vec![],
        );
        arrangement.observe(&done, &cleaning(), now());
        assert_eq!(listed(&done, &arrangement), ["large", "small"]);
    }

    #[test]
    fn after_the_first_sizing_the_disk_sort_follows_every_size_at_once() {
        let done = |large: u64| {
            snapshot(
                vec![
                    sized("small", measured(10), measured(5)),
                    sized("large", measured(large), measured(5)),
                ],
                vec![],
            )
        };
        let mut arrangement = Arrangement::default();
        arrangement.observe(&done(500), &cleaning(), now());
        arrangement.set_sort(SortOrder::Disk, &done(500), &cleaning(), now());
        assert_eq!(listed(&done(500), &arrangement), ["large", "small"]);

        // A build folder found later waits for its first size, as a new `target/` does, and the
        // list follows the sizes that change meanwhile at once.
        let mut later = done(1);
        later.projects[0].artifacts.push(node_modules(
            "/code/small/web/node_modules",
            Measurement::Measuring,
        ));
        assert!(!later.sizing().is_finished());
        arrangement.observe(&later, &cleaning(), now());
        assert_eq!(listed(&later, &arrangement), ["small", "large"]);
    }

    #[test]
    fn choosing_another_sort_lets_go_of_the_disk_sorts_hold() {
        let pending = Measurement::Measuring;
        let early = snapshot(
            vec![
                sized("b", measured(10), pending),
                sized("a", measured(500), pending),
            ],
            vec![],
        );
        let mut arrangement = Arrangement::default();
        arrangement.observe(&early, &cleaning(), now());
        arrangement.set_sort(SortOrder::Disk, &early, &cleaning(), now());
        arrangement.set_sort(SortOrder::Name, &early, &cleaning(), now());
        assert!(!arrangement.hold.is_held());
        assert_eq!(listed(&early, &arrangement), ["a", "b"]);
    }
}
