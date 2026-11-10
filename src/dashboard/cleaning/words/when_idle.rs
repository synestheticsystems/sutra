//! What Sutra says about cleaning a group's build folders by itself once they sit idle: each choice
//! in Settings and on a group's line, the mark of a line's own choice, and the banners after a look
//! at the groups.

use std::io;
use std::time::Duration;

use crate::dashboard::format;

/// `idle` in whole hours, as its count and its unit, in the largest unit it fills whole: `an
/// hour`, `8 hours`, `a day`, `3 days`, `a week`, `2 weeks`.
fn count_and_unit(idle: Duration) -> (String, &'static str) {
    const DAY: u64 = 24;
    const WEEK: u64 = 7 * DAY;
    match idle.as_secs() / (60 * 60) {
        1 => ("an".to_string(), "hour"),
        hours if hours % DAY != 0 => (hours.to_string(), "hours"),
        DAY => ("a".to_string(), "day"),
        WEEK => ("a".to_string(), "week"),
        hours if hours % WEEK == 0 => ((hours / WEEK).to_string(), "weeks"),
        hours => ((hours / DAY).to_string(), "days"),
    }
}

/// `idle` as a sentence says it: `an hour`, `8 hours`, `a day`, `3 days`, `a week`, `2 weeks`.
fn idle_time(idle: Duration) -> String {
    let (count, unit) = count_and_unit(idle);
    format!("{count} {unit}")
}

/// The choice of cleaning build folders once idle for `idle`, as a menu's title case writes it,
/// or with `None`, of not cleaning them.
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

/// The first choice on a group's line, of following the choice for every group, which Settings
/// words as `choice`.
pub(in crate::dashboard::cleaning) fn as_in_settings(choice: &str) -> String {
    format!("As in Settings ({choice})")
}

/// What the mark on the line of a group whose build folders are cleaned once idle for `idle`, a
/// time of its own, says.
pub(in crate::dashboard::cleaning) fn cleans_after(idle: Duration) -> String {
    format!(
        "Cleans its build folders once they've sat idle for {}",
        idle_time(idle)
    )
}

/// What the mark on the line of a group whose build folders are never cleaned for sitting idle,
/// where every other group's are, says.
pub(in crate::dashboard::cleaning) fn never_cleans() -> &'static str {
    "Never cleans its build folders for sitting idle"
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
        format!("{what} Nothing was deleted, and Sutra will look again."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(60 * 60);
    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    #[test]
    fn idle_times_read_in_the_largest_unit_they_fill_whole() {
        let said: Vec<String> = [1, 2, 8, 24, 36, 72, 168, 240, 336]
            .into_iter()
            .map(|hours| idle_time(HOUR * hours))
            .collect();
        assert_eq!(
            said,
            [
                "an hour", "2 hours", "8 hours", "a day", "36 hours", "3 days", "a week",
                "10 days", "2 weeks"
            ]
        );
    }

    #[test]
    fn the_choices_read_in_title_case_after_never_and_a_lines_mark_in_a_sentence() {
        let choices: Vec<String> = [None, Some(HOUR), Some(HOUR * 4), Some(DAY), Some(DAY * 7)]
            .into_iter()
            .map(when_idle_choice)
            .collect();
        assert_eq!(
            choices,
            [
                "Never",
                "For an Hour",
                "For 4 Hours",
                "For a Day",
                "For a Week"
            ]
        );
        assert_eq!(as_in_settings("For a Day"), "As in Settings (For a Day)");
        assert_eq!(
            cleans_after(HOUR * 2),
            "Cleans its build folders once they've sat idle for 2 hours"
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
            cleaned_when_idle("ankurah", 1, 2_100_000_000, HOUR).1,
            "1 folder idle for an hour or more: 2.1 GB back."
        );
    }
}
