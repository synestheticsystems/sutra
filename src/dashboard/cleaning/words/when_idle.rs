//! What Sutra says about cleaning a group's build folders by itself once they sit idle: how long a
//! choice lets them sit, and the banners after a look at the groups.

use std::io;
use std::time::Duration;

use crate::dashboard::format;

/// `idle` as a sentence says it, in whole days: `a day`, `3 days`, `a week`, `2 weeks`.
fn idle_time(idle: Duration) -> String {
    match idle.as_secs() / (24 * 60 * 60) {
        1 => "a day".to_string(),
        7 => "a week".to_string(),
        days if days > 7 && days % 7 == 0 => format!("{} weeks", days / 7),
        days => format!("{days} days"),
    }
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
