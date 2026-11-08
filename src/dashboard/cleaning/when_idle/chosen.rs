//! What the user chose from each group's line: how many days its build folders may sit idle before
//! Sutra cleans them, kept by the line, and the choices and the mark the line shows.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::DAY;
use crate::dashboard::LineKey;
use crate::dashboard::cleaning::words;
use crate::dashboard::listing::{self, ByLine};
use crate::map::{GroupKey, Snapshot};

/// How many days the user can let a group's build folders sit idle before Sutra cleans them.
const DAYS_CHOICES: [u32; 4] = [1, 3, 7, 14];

/// How many days each group's build folders may sit idle before Sutra cleans them, as the user
/// chose from the group's line, kept by the line (see `ByLine`). A group not here is cleaned only
/// when the user asks, or to keep free space.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::dashboard) struct CleanWhenIdle(ByLine<u32>);

impl CleanWhenIdle {
    /// How many days `group`'s build folders may sit idle, if the user chose.
    pub(in crate::dashboard) fn days_for(&self, group: &GroupKey) -> Option<u32> {
        self.0.get(&LineKey::group(group)).copied()
    }

    /// The user chose to clean `group`'s build folders once idle for `days`, or with `None`, not
    /// to. A time the line does not offer, or a group `map` no longer knows, changes nothing.
    /// Returns whether the choice changed.
    pub(in crate::dashboard) fn choose(
        &mut self,
        group: &GroupKey,
        days: Option<u32>,
        map: &Snapshot,
    ) -> bool {
        let offered = days.is_none_or(|days| DAYS_CHOICES.contains(&days));
        offered && self.0.set(&LineKey::group(group), days, map)
    }

    /// A live map arrived: forget the choice for every group `map` no longer knows. Returns
    /// whether any went.
    pub(in crate::dashboard) fn forget_gone(&mut self, map: &Snapshot) -> bool {
        self.0.forget_gone(map)
    }

    /// Whether the user chose a time for no group.
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How long the build folders of each group on `map` the user chose a time for may sit idle.
    pub(super) fn times_on(&self, map: &Snapshot) -> HashMap<GroupKey, Duration> {
        listing::groups(map)
            .filter_map(|group| Some((group.clone(), DAY * self.days_for(group)?)))
            .collect()
    }
}

/// A time a group's line offers, with the words its menu shows.
pub(in crate::dashboard) struct WhenIdleChoice {
    /// How many days the group's build folders may sit idle; `None` for never.
    pub(in crate::dashboard) days: Option<u32>,
    pub(in crate::dashboard) label: String,
}

/// What a group's line offers: never to clean its build folders by itself, then each time.
pub(in crate::dashboard) fn when_idle_choices() -> Vec<WhenIdleChoice> {
    std::iter::once(None)
        .chain(DAYS_CHOICES.map(Some))
        .map(|days| WhenIdleChoice {
            days,
            label: words::when_idle_choice(days.map(|days| DAY * days)),
        })
        .collect()
}

/// What the mark on the line of a group says, whose build folders are cleaned once idle `days`.
pub(in crate::dashboard) fn when_idle_mark(days: u32) -> String {
    words::when_idle_mark(DAY * days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{project_at, snapshot_of};
    use crate::map::{Project, Provenance};

    fn upstream(name: &str) -> GroupKey {
        GroupKey::Upstream(format!("github.com/me/{name}"))
    }

    /// A project at `/code/<name>` standing in `group`.
    fn in_group(group: GroupKey, name: &str) -> Project {
        Project {
            provenance: Provenance {
                group,
                made_from: Vec::new(),
            },
            ..project_at(format!("/code/{name}"))
        }
    }

    #[test]
    fn a_group_takes_only_a_time_its_line_offers_and_the_groups_without_one_have_none() {
        let map = snapshot_of(vec![
            in_group(upstream("app"), "app"),
            in_group(upstream("site"), "site"),
            project_at("/code/loose"),
        ]);
        let mut chosen = CleanWhenIdle::default();
        assert!(
            !chosen.choose(&upstream("app"), Some(5), &map),
            "not offered"
        );
        assert!(chosen.choose(&upstream("app"), Some(7), &map));
        assert!(chosen.choose(&GroupKey::Other, Some(1), &map));
        assert_eq!(
            chosen.times_on(&map),
            HashMap::from([(upstream("app"), DAY * 7), (GroupKey::Other, DAY)])
        );

        assert!(chosen.choose(&upstream("app"), None, &map));
        assert_eq!(chosen.days_for(&upstream("app")), None);
    }
}
