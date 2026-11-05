//! What Sutra says about cleaning by itself: each choice of how much to keep free, the question
//! before it starts, and the banner after.

use std::path::Path;
use std::time::Duration;

use crate::dashboard::format;
use crate::dashboard::state::Question;
use crate::map::clean::Cleaned;

/// The choice of keeping `floor` free, or with `None` of never cleaning by itself.
pub(in crate::dashboard::cleaning) fn keep_free_choice(floor: Option<u64>) -> String {
    match floor {
        Some(floor) => format!("Keep {} Free", format::size(floor, false)),
        None => "Never Clean by Itself".to_string(),
    }
}

/// The banner after Sutra cleaned `target` by itself, to keep `floor` free.
pub(in crate::dashboard::cleaning) fn made_room(
    target: &Path,
    cleaned: &Cleaned,
    floor: u64,
    home: &Path,
) -> (String, String) {
    let freed: u64 = cleaned.folders.iter().map(|folder| folder.bytes).sum();
    (
        "Made some room".to_string(),
        format!(
            "Cleaned {}: {} back. You asked to keep {} free.",
            format::tilde(target, home),
            format::size(freed, false),
            format::size(floor, false)
        ),
    )
}

/// The question before Sutra starts keeping `floor` free, with `free` free now.
pub(in crate::dashboard::cleaning) fn keep_free_question(
    floor: u64,
    free: Option<u64>,
    quiet_at_least: Duration,
) -> Question {
    let floor_words = format::size(floor, false);
    let minutes = quiet_at_least.as_secs() / 60;
    let quiet = if minutes == 60 {
        "the last hour".to_string()
    } else {
        format!("the last {minutes} minutes")
    };
    let mut detail = format!(
        "When free space drops below {floor_words}, Sutra cleans the build folders quiet the \
         longest, one at a time, without asking. Anything with an agent, a running environment or \
         activity in {quiet} stays put."
    );
    if let Some(free) = free {
        detail.push_str(&format!(
            "\n\n{} is free right now{}.",
            format::size(free, false),
            if free < floor {
                ", so it starts straight away"
            } else {
                ""
            }
        ));
    }
    Question {
        title: format!("Keep {floor_words} free?"),
        detail,
        action: format!("Keep {floor_words} Free"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const HOME: &str = "/Users/dev";

    #[test]
    fn keeping_free_space_says_what_will_happen_and_whether_it_starts_now() {
        let an_hour = Duration::from_secs(3_600);
        let question = keep_free_question(100_000_000_000, Some(168_000_000_000), an_hour);
        assert_eq!(question.title, "Keep 100 GB free?");
        assert_eq!(question.action, "Keep 100 GB Free");
        assert_eq!(
            question.detail,
            "When free space drops below 100 GB, Sutra cleans the build folders quiet the longest, \
             one at a time, without asking. Anything with an agent, a running environment or \
             activity in the last hour stays put.\n\n168 GB is free right now."
        );
        let low = keep_free_question(200_000_000_000, Some(168_000_000_000), an_hour);
        assert!(
            low.detail
                .ends_with("168 GB is free right now, so it starts straight away.")
        );
    }

    #[test]
    fn a_clean_nobody_asked_for_says_where_the_room_came_from() {
        let cleaned = Cleaned {
            folders: vec![crate::map::clean::Freed {
                folder: PathBuf::from("/Users/dev/code/app/target"),
                bytes: 46_000_000_000,
                left: 4096,
            }],
        };
        assert_eq!(
            made_room(
                Path::new("/Users/dev/code/app/target"),
                &cleaned,
                100_000_000_000,
                Path::new(HOME)
            ),
            (
                "Made some room".to_string(),
                "Cleaned ~/code/app/target: 46 GB back. You asked to keep 100 GB free.".to_string()
            )
        );
    }
}
