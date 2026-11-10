//! What the user chose of how long build folders may sit idle before Sutra cleans them: one choice
//! for every group, made in Settings, and the choice of its own a group may make from its line,
//! kept by the line; with the choices offered, and the mark on a line whose own choice differs.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::dashboard::cleaning::words;
use crate::dashboard::listing::{self, ByLine};
use crate::dashboard::{LineKey, WhenIdleChoice, WhenIdleMark};
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
    fn duration(self) -> Duration {
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

/// What Sutra does with build folders once they sit idle.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "snake_case")]
pub enum WhenIdle {
    /// Leaves them to the user, and to keeping free space.
    #[default]
    Never,
    /// Cleans them once they have sat idle for `idle`.
    CleanAfter { idle: IdleTime },
}

impl WhenIdle {
    /// How long build folders may sit idle; `None` for never.
    fn duration(self) -> Option<Duration> {
        match self {
            WhenIdle::Never => None,
            WhenIdle::CleanAfter { idle } => Some(idle.duration()),
        }
    }

    /// Never, then each time offered, shortest first.
    fn offered() -> impl Iterator<Item = WhenIdle> {
        let times = IdleTime::OFFERED.map(|idle| WhenIdle::CleanAfter { idle });
        std::iter::once(WhenIdle::Never).chain(times)
    }
}

/// How long build folders may sit idle before Sutra cleans them, as the user chose: once for every
/// group, and for any group from its line, which the line keeps (see `ByLine`). A group's own
/// choice, a time or never, applies to it; and to every other group, the choice for every group.
/// Docker's group has no build folders, and no choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(in crate::dashboard) struct CleanWhenIdle {
    /// What applies to each group that made no choice of its own: never, at first.
    every_group: WhenIdle,
    /// The choice of its own each group made from its line.
    own: ByLine<WhenIdle>,
}

impl CleanWhenIdle {
    /// What applies to each group that made no choice of its own.
    pub(in crate::dashboard) fn for_every_group(&self) -> WhenIdle {
        self.every_group
    }

    /// The choice of its own `group` made from its line, if it made one.
    pub(in crate::dashboard) fn own_choice_of(&self, group: &GroupKey) -> Option<WhenIdle> {
        self.own.get(&LineKey::group(group)).copied()
    }

    /// What applies to `group`: its own choice, else the one for every group.
    fn applying_to(&self, group: &GroupKey) -> WhenIdle {
        self.own_choice_of(group).unwrap_or(self.every_group)
    }

    /// The user chose `choice` for every group that has none of its own. Returns whether that
    /// changed.
    pub(in crate::dashboard) fn choose_for_every_group(&mut self, choice: WhenIdle) -> bool {
        let changed = self.every_group != choice;
        self.every_group = choice;
        changed
    }

    /// The user chose `own` from the line of `group` as its own, or with `None`, to follow the
    /// choice for every group again. A group `map` no longer knows changes nothing. Returns
    /// whether the choice changed.
    pub(in crate::dashboard) fn choose_own(
        &mut self,
        group: &GroupKey,
        own: Option<WhenIdle>,
        map: &Snapshot,
    ) -> bool {
        self.own.set(&LineKey::group(group), own, map)
    }

    /// A live map arrived: carry the choice of each group that `moves` says moved to its new key,
    /// and forget the choice of every group `map` no longer knows (see `ByLine::keep_up_with`).
    /// Returns whether the choices changed.
    pub(in crate::dashboard) fn keep_up_with(
        &mut self,
        map: &Snapshot,
        moves: &BTreeMap<GroupKey, GroupKey>,
    ) -> bool {
        self.own.keep_up_with(map, moves)
    }

    /// How long the build folders of each group on `map` may sit idle, of those a time applies to.
    pub(super) fn times_on(&self, map: &Snapshot) -> IdleTimes {
        let timed = listing::groups(map)
            .filter_map(|group| Some((group.clone(), self.applying_to(group).duration()?)));
        IdleTimes(timed.collect())
    }

    /// The mark on the line of `group` while its own choice differs from the one for every group,
    /// its tip saying what applies to it; `None` while it follows that one, or chose the same.
    /// Once every group cleans itself, a mark on every line would say nothing.
    pub(in crate::dashboard) fn mark_of(&self, group: &GroupKey) -> Option<WhenIdleMark> {
        let own = self
            .own_choice_of(group)
            .filter(|own| *own != self.every_group)?;
        Some(match own.duration() {
            Some(idle) => WhenIdleMark::OwnTime {
                tip: words::cleans_after(idle),
            },
            None => WhenIdleMark::Never {
                tip: words::never_cleans().to_string(),
            },
        })
    }
}

/// How long the build folders of each group on a map may sit idle before Sutra cleans them, for
/// the groups a time applies to, as they stood when Sutra looked. A project in any other group
/// never goes for being idle, nor does a folder it shares: "Never" for a group is a promise that
/// no other group's time breaks.
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

/// What Settings and a group's line offer: never to clean build folders by themselves, then each
/// time, in the words both show.
pub(in crate::dashboard) fn when_idle_choices() -> Vec<WhenIdleChoice> {
    let choice = |choice: WhenIdle| WhenIdleChoice {
        choice,
        label: words::when_idle_choice(choice.duration()),
    };
    WhenIdle::offered().map(choice).collect()
}

/// The first choice on a group's line, of following `every_group`, the choice for every group,
/// which it names as Settings words it.
pub(in crate::dashboard) fn as_in_settings(every_group: WhenIdle) -> String {
    words::as_in_settings(&words::when_idle_choice(every_group.duration()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Provenance;
    use crate::map::test_support::{project_at, snapshot_of};

    const HOUR: Duration = Duration::from_secs(60 * 60);

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

    fn after(idle: IdleTime) -> WhenIdle {
        WhenIdle::CleanAfter { idle }
    }

    fn app_site_and_loose() -> Snapshot {
        snapshot_of(vec![
            in_group(upstream("app"), "app"),
            in_group(upstream("site"), "site"),
            project_at("/code/loose"),
        ])
    }

    #[test]
    fn a_groups_own_choice_applies_to_it_and_the_one_for_every_group_to_the_rest() {
        let map = app_site_and_loose();
        let mut chosen = CleanWhenIdle::default();
        assert!(
            chosen.times_on(&map).shortest().is_none(),
            "never, at first"
        );
        assert!(chosen.choose_for_every_group(after(IdleTime::OneDay)));
        assert!(chosen.choose_own(&upstream("app"), Some(after(IdleTime::TwoHours)), &map));
        assert!(chosen.choose_own(&upstream("site"), Some(WhenIdle::Never), &map));

        let times = chosen.times_on(&map);
        assert_eq!(
            times,
            IdleTimes(HashMap::from([
                (upstream("app"), HOUR * 2),
                (GroupKey::Other, HOUR * 24)
            ])),
            "site never, and Other as every group"
        );
        assert_eq!(times.shortest(), Some(HOUR * 2));

        assert!(chosen.choose_own(&upstream("app"), None, &map));
        assert_eq!(chosen.own_choice_of(&upstream("app")), None);
        assert_eq!(
            chosen.times_on(&map).of(&map.projects[0]),
            Some(HOUR * 24),
            "app follows every group again"
        );
    }

    #[test]
    fn only_a_line_whose_own_choice_differs_from_every_groups_is_marked_and_says_what_applies() {
        let map = app_site_and_loose();
        let mut chosen = CleanWhenIdle::default();
        chosen.choose_own(&upstream("app"), Some(after(IdleTime::FourHours)), &map);
        chosen.choose_own(&upstream("site"), Some(WhenIdle::Never), &map);
        let marks = |chosen: &CleanWhenIdle| {
            [upstream("app"), upstream("site"), GroupKey::Other].map(|group| chosen.mark_of(&group))
        };
        let four_hours = WhenIdleMark::OwnTime {
            tip: "Cleans its build folders once they've sat idle for 4 hours".to_string(),
        };
        assert_eq!(
            marks(&chosen),
            [Some(four_hours.clone()), None, None],
            "site chose never, as every group is"
        );

        chosen.choose_for_every_group(after(IdleTime::FourHours));
        let never = WhenIdleMark::Never {
            tip: "Never cleans its build folders for sitting idle".to_string(),
        };
        assert_eq!(
            marks(&chosen),
            [None, Some(never), None],
            "app chose what every group has"
        );
    }

    #[test]
    fn never_then_each_time_is_offered_and_the_times_run_from_an_hour_to_two_weeks() {
        let offered: Vec<Option<u64>> = WhenIdle::offered()
            .map(|choice| choice.duration().map(|idle| idle.as_secs() / 3600))
            .collect();
        assert_eq!(
            offered,
            [
                None,
                Some(1),
                Some(2),
                Some(4),
                Some(8),
                Some(24),
                Some(72),
                Some(168),
                Some(336)
            ]
        );
    }

    #[test]
    fn the_choices_are_saved_by_name_and_a_file_with_none_reads_as_never_for_every_group() {
        let map = app_site_and_loose();
        let mut chosen = CleanWhenIdle::default();
        chosen.choose_for_every_group(after(IdleTime::EightHours));
        chosen.choose_own(&upstream("site"), Some(WhenIdle::Never), &map);
        let saved = serde_json::to_value(&chosen).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({
                "every_group": {"clean_after": {"idle": "eight_hours"}},
                "own": {"upstream github.com/me/site": "never"},
            })
        );
        assert_eq!(
            serde_json::from_value::<CleanWhenIdle>(saved).unwrap(),
            chosen
        );
        assert_eq!(
            serde_json::from_value::<CleanWhenIdle>(serde_json::json!({})).unwrap(),
            CleanWhenIdle::default()
        );
    }
}
