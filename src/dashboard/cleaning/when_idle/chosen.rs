//! What the user chose from each group's line: how long its build folders may sit idle before
//! Sutra cleans them, kept by the line, and the choices and the mark the line shows.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::dashboard::LineKey;
use crate::dashboard::cleaning::words;
use crate::dashboard::listing::{self, ByLine};
use crate::map::{GroupKey, Project, Snapshot};

/// How long the user can let a group's build folders sit idle before Sutra cleans them: from an
/// hour, for the agents whose builds fill a disk in an afternoon, to two weeks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum IdleTime {
    OneHour,
    TwoHours,
    FourHours,
    EightHours,
    OneDay,
    ThreeDays,
    OneWeek,
    TwoWeeks,
}

impl IdleTime {
    /// Every time offered, shortest first.
    const OFFERED: [IdleTime; 8] = [
        IdleTime::OneHour,
        IdleTime::TwoHours,
        IdleTime::FourHours,
        IdleTime::EightHours,
        IdleTime::OneDay,
        IdleTime::ThreeDays,
        IdleTime::OneWeek,
        IdleTime::TwoWeeks,
    ];

    /// How long that is.
    pub(in crate::dashboard) fn duration(self) -> Duration {
        let hours = match self {
            IdleTime::OneHour => 1,
            IdleTime::TwoHours => 2,
            IdleTime::FourHours => 4,
            IdleTime::EightHours => 8,
            IdleTime::OneDay => 24,
            IdleTime::ThreeDays => 3 * 24,
            IdleTime::OneWeek => 7 * 24,
            IdleTime::TwoWeeks => 14 * 24,
        };
        Duration::from_secs(hours * 60 * 60)
    }
}

/// How long each group's build folders may sit idle before Sutra cleans them, as the user chose
/// from the group's line, kept by the line (see `ByLine`). A group not here is cleaned only when
/// the user asks, or to keep free space.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::dashboard) struct CleanWhenIdle(ByLine<IdleTime>);

impl CleanWhenIdle {
    /// How long `group`'s build folders may sit idle, if the user chose.
    pub(in crate::dashboard) fn idle_time_of(&self, group: &GroupKey) -> Option<IdleTime> {
        self.0.get(&LineKey::group(group)).copied()
    }

    /// The user chose to clean `group`'s build folders once idle for `idle`, or with `None`, not
    /// to. A group `map` no longer knows changes nothing. Returns whether the choice changed.
    pub(in crate::dashboard) fn choose(
        &mut self,
        group: &GroupKey,
        idle: Option<IdleTime>,
        map: &Snapshot,
    ) -> bool {
        self.0.set(&LineKey::group(group), idle, map)
    }

    /// A live map arrived: carry the choice of each group that `moves` says moved to its new key,
    /// and forget the choice for every group `map` no longer knows (see `ByLine::keep_up_with`).
    /// Returns whether the choices changed.
    pub(in crate::dashboard) fn keep_up_with(
        &mut self,
        map: &Snapshot,
        moves: &BTreeMap<GroupKey, GroupKey>,
    ) -> bool {
        self.0.keep_up_with(map, moves)
    }

    /// How long the build folders of each group on `map` the user chose a time for may sit idle.
    pub(super) fn times_on(&self, map: &Snapshot) -> IdleTimes {
        let chosen = listing::groups(map)
            .filter_map(|group| Some((group.clone(), self.idle_time_of(group)?.duration())));
        IdleTimes(chosen.collect())
    }
}

/// How long the build folders of each group on a map may sit idle before Sutra cleans them, for
/// the groups the user chose a time for, as they stood when Sutra looked. A project in any other
/// group never goes for being idle, nor does a folder it shares: "Never" on a group's line is a
/// promise that no other group's time breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::dashboard) struct IdleTimes(HashMap<GroupKey, Duration>);

impl IdleTimes {
    /// How long `project` must have been quiet for its build folders to go, by its group's time;
    /// `None` for never.
    pub(in crate::dashboard) fn of(&self, project: &Project) -> Option<Duration> {
        self.0.get(&project.provenance.group).copied()
    }

    /// The shortest time of any group on the map, which says how often to look; `None` while no
    /// group has one.
    pub(super) fn shortest(&self) -> Option<Duration> {
        self.0.values().min().copied()
    }
}

/// A time a group's line offers, with the words its menu shows.
pub(in crate::dashboard) struct WhenIdleChoice {
    /// How long the group's build folders may sit idle; `None` for never.
    pub(in crate::dashboard) idle: Option<IdleTime>,
    pub(in crate::dashboard) label: String,
}

/// What a group's line offers: never to clean its build folders by itself, then each time.
pub(in crate::dashboard) fn when_idle_choices() -> Vec<WhenIdleChoice> {
    std::iter::once(None)
        .chain(IdleTime::OFFERED.map(Some))
        .map(|idle| WhenIdleChoice {
            idle,
            label: words::when_idle_choice(idle.map(IdleTime::duration)),
        })
        .collect()
}

/// What the mark on the line of a group says, whose build folders are cleaned once idle for `idle`.
pub(in crate::dashboard) fn when_idle_mark(idle: IdleTime) -> String {
    words::when_idle_mark(idle.duration())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Provenance;
    use crate::map::test_support::{project_at, snapshot_of};

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
    fn each_group_with_a_time_has_it_on_the_map_the_shortest_says_how_often_to_look() {
        let map = snapshot_of(vec![
            in_group(upstream("app"), "app"),
            in_group(upstream("site"), "site"),
            project_at("/code/loose"),
        ]);
        let mut chosen = CleanWhenIdle::default();
        assert!(chosen.choose(&upstream("app"), Some(IdleTime::OneWeek), &map));
        assert!(chosen.choose(&GroupKey::Other, Some(IdleTime::TwoHours), &map));
        let times = chosen.times_on(&map);
        assert_eq!(
            times,
            IdleTimes(HashMap::from([
                (upstream("app"), Duration::from_secs(7 * 24 * 60 * 60)),
                (GroupKey::Other, Duration::from_secs(2 * 60 * 60))
            ]))
        );
        assert_eq!(times.shortest(), Some(Duration::from_secs(2 * 60 * 60)));

        assert!(chosen.choose(&upstream("app"), None, &map));
        assert!(chosen.choose(&GroupKey::Other, None, &map));
        assert_eq!(chosen.idle_time_of(&upstream("app")), None);
        assert_eq!(
            chosen.times_on(&map).shortest(),
            None,
            "no group has a time"
        );
    }

    #[test]
    fn the_times_offered_run_from_an_hour_to_two_weeks_each_longer_than_the_last() {
        let hours: Vec<u64> = (IdleTime::OFFERED.iter())
            .map(|idle| idle.duration().as_secs() / (60 * 60))
            .collect();
        assert_eq!(hours, [1, 2, 4, 8, 24, 72, 168, 336]);
    }
}
