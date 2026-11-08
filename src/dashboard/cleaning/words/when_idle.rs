//! What Sutra says about cleaning a group's build folders by itself once they sit idle: each choice
//! on a group's line and the mark of the choice made, and the banners after a look at the groups.

use std::io;
use std::time::Duration;

use crate::dashboard::format;

/// `idle` in whole days, as its count and its unit: `a day`, `3 days`, `a week`, `2 weeks`.
fn count_and_unit(idle: Duration) -> (String, &'static str) {
    match idle.as_secs() / (24 * 60 * 60) {
        1 => ("a".to_string(), "day"),
        7 => ("a".to_string(), "week"),
        days if days > 7 && days % 7 == 0 => ((days / 7).to_string(), "weeks"),
        days => (days.to_string(), "days"),
    }
}

/// `idle` as a sentence says it: `a day`, `3 days`, `a week`, `2 weeks`.
fn idle_time(idle: Duration) -> String {
    let (count, unit) = count_and_unit(idle);
    format!("{count} {unit}")
}

/// The choice on a group's line of cleaning its build folders once idle for `idle`, as a menu's
/// title case writes it, or with `None`, of not cleaning them.
pub(in crate::dashboard::cleaning) fn when_idle_choice(idle: Option<Duration>) -> String {
    let Some(idle) = idle else {
        return "Never".to_string();
    };
    let (count, unit) = count_and_unit(idle);
    let mut letters = unit.chars();
    let unit: String = letters
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .chain(letters)
        .collect();
    format!("For {count} {unit}")
}

/// What the mark on the line of a group whose build folders are cleaned once idle for `idle` says.
pub(in crate::dashboard::cleaning) fn when_idle_mark(idle: Duration) -> String {
    format!(
        "Cleans its build folders once they've sat idle for {}",
        idle_time(idle)
    )
}

/// The banner once Sutra has cleaned `folders` of `group`'s build folders by itself, which gave
/// back `freed`, as the user chose to clean them once idle for `idle`.
pub(in crate::dashboard::cleaning) fn cleaned_when_idle(
    group: &str,
    folders: usize,
    freed: u64,
    idle: Duration,
) -> (String, String) {
    (
        format!("Cleaned {group}'s idle build folders"),
        format!(
            "{} idle for {} or more: {} back.",
            format::plural(folders as u64, "folder"),
            idle_time(idle),
            format::size(freed, false)
        ),
    )
}

/// The banner when the look for the groups' idle build folders stopped: its thread could not
/// start, for `why`, or ended with no word.
pub(in crate::dashboard::cleaning) fn look_stopped(why: Option<&io::Error>) -> (String, String) {
    let what = match why {
        Some(err) => format!("Sutra couldn't start looking: {err}."),
        None => "The look stopped before it finished.".to_string(),
    };
    (
        "Couldn't look for idle build folders".to_string(),
        format!("{what} Nothing was deleted, and Sutra looks again in an hour."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    #[test]
    fn idle_times_read_in_days_and_whole_weeks() {
        let said: Vec<String> = [1, 3, 7, 14, 10]
            .into_iter()
            .map(|days| idle_time(DAY * days))
            .collect();
        assert_eq!(said, ["a day", "3 days", "a week", "2 weeks", "10 days"]);
    }

    #[test]
    fn a_lines_choices_read_in_title_case_after_never_and_its_mark_in_a_sentence() {
        let choices: Vec<String> = [None, Some(1), Some(3), Some(7), Some(14)]
            .into_iter()
            .map(|days| when_idle_choice(days.map(|days| DAY * days)))
            .collect();
        assert_eq!(
            choices,
            [
                "Never",
                "For a Day",
                "For 3 Days",
                "For a Week",
                "For 2 Weeks"
            ]
        );
        assert_eq!(
            when_idle_mark(DAY * 3),
            "Cleans its build folders once they've sat idle for 3 days"
        );
    }

    #[test]
    fn the_banner_after_a_groups_cleans_says_how_many_went_how_idle_and_what_came_back() {
        assert_eq!(
            cleaned_when_idle("ankurah", 4, 48_000_000_000, DAY * 3),
            (
                "Cleaned ankurah's idle build folders".to_string(),
                "4 folders idle for 3 days or more: 48 GB back.".to_string()
            )
        );
        assert_eq!(
            cleaned_when_idle("ankurah", 1, 2_100_000_000, DAY * 7).1,
            "1 folder idle for a week or more: 2.1 GB back."
        );
    }
}
