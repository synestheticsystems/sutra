//! Cleaning a group's build folders by itself once they have sat idle as long as the user chose
//! from the group's line (see `chosen`). Every hour, and soon after a choice, the build folders of
//! each idle project in such a group are cleaned with no question asked, as the user's choice is
//! their say so: idle as the toolbar's offer tells it (see `clean::prepare_idle`), only for the
//! group's own time, and never where the clean would take the builds of a project quiet for less.
//! A group may hold busy and idle checkouts at once, and only the idle ones' folders go. Each
//! group's cleans end in one banner saying what went. Keeping free space (see `auto`) stays the
//! rule for a disk running short, beside this one.

mod chosen;
#[cfg(test)]
mod tests;

pub(in crate::dashboard) use chosen::CleanWhenIdle;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use super::queue::Waiting;
use super::{By, Cleaning, Report, words};
use crate::dashboard::listing;
use crate::map::clean::{self, CargoEnv, CleanError, Cleaned, IdleCleans, PrepareError, Why};
use crate::map::{GroupKey, Project, Snapshot};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// How often the groups are looked at. A folder becomes idle over days, so an hour late is
/// nothing, and each look reads what every project's build deletes.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// How long a folder whose clean failed is left alone, so that what stopped it, which may well
/// stop it again, is told once a day at most rather than at every look.
const PASS_OVER_FOR: Duration = DAY;

/// What cleaning the groups' idle build folders needs to remember between looks.
#[derive(Debug, Default)]
pub(super) struct WhenIdle {
    /// When to look next; `None` to look at the next tick.
    next_check: Option<Instant>,
    /// The cleans a look found are being worked out.
    pub(super) preparing: bool,
    /// The user changed a group's time while they were, so what comes back is out of date.
    outdated: bool,
    /// Each group's cleans from the last look, until the last of them ends.
    rounds: HashMap<GroupKey, Round>,
    /// Folders whose clean failed, and when each may be tried again.
    passed_over: HashMap<PathBuf, SystemTime>,
}

/// One group's cleans from one look, and what they have given back so far, for the banner once
/// they have all ended.
#[derive(Debug)]
struct Round {
    /// What the banner calls the group.
    name: String,
    /// How long its folders had sat idle at least.
    idle: Duration,
    cleaned: usize,
    freed: u64,
}

impl Cleaning {
    /// The clock ticked: if the user chose a time for any group, it is time to look, and nothing
    /// is being cleaned, work out on a thread of its own the clean of each build folder of those
    /// groups idle that long, as `map` shows them, but for those a clean or a question already
    /// has, and those passed over.
    pub(in crate::dashboard) fn clean_when_idle(
        &mut self,
        map: &Arc<Snapshot>,
        chosen: &CleanWhenIdle,
        at: Instant,
        now: SystemTime,
        home: &Path,
    ) {
        let due = self.when_idle.next_check.is_none_or(|next| at >= next);
        if chosen.is_empty() || !due || self.is_busy() {
            return;
        }
        self.when_idle.next_check = Some(at + CHECK_EVERY);
        self.when_idle.passed_over.retain(|_, until| *until > now);
        let times = chosen.times_on(map);
        if times.is_empty() {
            return;
        }
        let mut left_alone = self.being_cleaned();
        left_alone.extend(self.when_idle.passed_over.keys().cloned());
        let map = Arc::clone(map);
        let env = CargoEnv::of_this_process(home);
        let looking = self.threads.run(
            "sutra-clean-prepare",
            move || {
                let time_of = |project: &Project| times.get(&project.provenance.group).copied();
                clean::prepare_idle(&map, now, time_of, &left_alone, &env)
            },
            Report::WhenIdlePrepared,
        );
        match looking {
            Ok(()) => self.when_idle.preparing = true,
            Err(err) => self.banners.push(words::look_stopped(Some(&err))),
        }
    }

    /// The cleans a look found have been worked out, or their thread ended with no word (`None`).
    /// Each joins its group's round and is queued, no question asked, unless a clean or a
    /// question took its folder meanwhile; one reaching into a folder a question asks about waits
    /// for the answer at its turn (see `may_start`). Worked out before the user changed a group's
    /// time, they all wait for the next look, at the next tick. What stopped a clean being tried
    /// is told, and its folder passed over; what Sutra will not clean, or a build or a busy sharer
    /// uses, waits for the next look. `map` is the map now, `None` with no engine to keep it.
    pub(super) fn when_idle_prepared(
        &mut self,
        found: Option<IdleCleans>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        self.when_idle.preparing = false;
        if std::mem::take(&mut self.when_idle.outdated) {
            self.when_idle.next_check = None;
            return;
        }
        let Some(IdleCleans { cleans, left_out }) = found else {
            self.banners.push(words::look_stopped(None));
            return;
        };
        for left in left_out {
            if let Why::Refused(PrepareError::Locks(err @ CleanError::Io { .. })) = &left.why {
                self.banners
                    .push(words::stopped(&left.folder, Some(err), 0, home));
                self.when_idle.pass_over(left.folder, now);
            }
        }
        let Some(map) = map else {
            return;
        };
        let names = listing::group_names(map);
        let taken = self.being_cleaned();
        for clean in cleans {
            if taken.contains(&clean.clicked) {
                continue;
            }
            // A project gone from the map since the look has nothing left to clean, and every
            // group on the map goes by a name.
            let project = map
                .projects
                .iter()
                .find(|its| its.path == clean.project_path);
            let Some(group) = project.map(|project| &project.provenance.group) else {
                continue;
            };
            let Some(name) = names.get(group) else {
                continue;
            };
            self.when_idle.rounds.entry(group.clone()).or_insert(Round {
                name: name.clone(),
                idle: clean.quiet_at_least,
                cleaned: 0,
                freed: 0,
            });
            let by = By::WhenIdle {
                project: clean.project_path,
                group: group.clone(),
                quiet_at_least: clean.quiet_at_least,
            };
            self.queue(clean.clicked, clean.prepared, by);
        }
        self.start_next(Some(map), now, home);
    }

    /// The user chose another time for `group`, or none: its cleans still waiting were worked out
    /// for the old one, so they go back, while those deleting finish, and what a look is working
    /// out is dropped when it comes back. The groups are looked at again at the next tick.
    pub(in crate::dashboard) fn when_idle_changed(&mut self, group: &GroupKey) {
        let of_group = |waiting: &Waiting| matches!(&waiting.by, By::WhenIdle { group: its, .. } if its == group);
        let (dropped, waiting): (VecDeque<Waiting>, VecDeque<Waiting>) =
            std::mem::take(&mut self.waiting)
                .into_iter()
                .partition(of_group);
        self.waiting = waiting;
        for clean in &dropped {
            self.show_sizes_again(clean.prepared.plan().folders());
        }
        self.when_idle.outdated |= self.when_idle.preparing;
        self.when_idle.next_check = None;
        self.end_rounds();
    }

    /// The clean Sutra made of one of `group`'s idle folders deleted what `cleaned` says.
    pub(super) fn cleaned_when_idle(&mut self, group: &GroupKey, cleaned: &Cleaned) {
        if let Some(round) = self.when_idle.rounds.get_mut(group) {
            round.cleaned += 1;
            round.freed += cleaned
                .folders
                .iter()
                .map(|folder| folder.bytes)
                .sum::<u64>();
        }
    }

    /// The clean Sutra made of the idle folder `target` stopped at `now`, as `stopped` tells in a
    /// banner. The looks pass it over for a while.
    pub(super) fn failed_when_idle(
        &mut self,
        target: &Path,
        stopped: (String, String),
        now: SystemTime,
    ) {
        self.banners.push(stopped);
        self.when_idle.pass_over(target.to_path_buf(), now);
    }

    /// Each group whose cleans have all ended gets its banner, saying what went, if anything did.
    pub(super) fn end_rounds(&mut self) {
        if self.when_idle.rounds.is_empty() {
            return;
        }
        let under_way: HashSet<&GroupKey> = (self.waiting.iter().map(|waiting| &waiting.by))
            .chain(self.running.iter().map(|running| &running.by))
            .filter_map(|by| match by {
                By::WhenIdle { group, .. } => Some(group),
                _ => None,
            })
            .collect();
        let banners = &mut self.banners;
        self.when_idle.rounds.retain(|group, round| {
            let ended = !under_way.contains(group);
            if ended && round.cleaned > 0 {
                banners.push(words::cleaned_when_idle(
                    &round.name,
                    round.cleaned,
                    round.freed,
                    round.idle,
                ));
            }
            !ended
        });
    }
}

impl WhenIdle {
    fn pass_over(&mut self, folder: PathBuf, now: SystemTime) {
        self.passed_over.insert(folder, now + PASS_OVER_FOR);
    }
}
