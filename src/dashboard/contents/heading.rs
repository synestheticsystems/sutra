//! What the window says of itself: its title and the Dock badge, which count the agents waiting
//! on the user while the map is live, and, once the engine has stopped, the line over the list
//! that says the list shows only how things last stood. A count the map can no longer keep up to
//! date would claim to know what Sutra no longer knows.

/// The line over the list once the engine has stopped while a map was shown.
pub(in crate::dashboard) const STOPPED: &str =
    "Updates stopped, so this is how things last stood. Quit and reopen Sutra.";

/// `Sutra · 2 waiting` while agents wait on the user, `Sutra · stopped` once the map no longer
/// updates, else `Sutra`.
pub(in crate::dashboard) fn title(waiting: usize, stopped: bool) -> String {
    match (stopped, waiting) {
        (true, _) => "Sutra \u{00b7} stopped".to_string(),
        (false, 0) => "Sutra".to_string(),
        (false, n) => format!("Sutra \u{00b7} {n} waiting"),
    }
}

/// The Dock badge: how many agents wait on the user, none at 0, or `!` once the map no longer
/// updates, so the Dock still calls the user back to a Sutra that needs reopening.
pub(in crate::dashboard) fn dock_badge(waiting: usize, stopped: bool) -> Option<String> {
    match (stopped, waiting) {
        (true, _) => Some("!".to_string()),
        (false, 0) => None,
        (false, n) => Some(n.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_counts_the_agents_waiting_says_nothing_of_none_and_says_when_updates_stopped() {
        assert_eq!(title(0, false), "Sutra");
        assert_eq!(title(2, false), "Sutra \u{00b7} 2 waiting");
        assert_eq!(title(2, true), "Sutra \u{00b7} stopped");
    }

    #[test]
    fn the_dock_badge_counts_the_agents_waiting_and_turns_to_a_warning_when_updates_stopped() {
        assert_eq!(dock_badge(0, false), None);
        assert_eq!(dock_badge(3, false).as_deref(), Some("3"));
        assert_eq!(dock_badge(0, true).as_deref(), Some("!"));
        assert_eq!(dock_badge(3, true).as_deref(), Some("!"));
    }
}
