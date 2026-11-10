//! Cleaning without being asked, once the user chose how much free space to keep: when less than that
//! is free, the build folder quiet the longest is cleaned, then free space is read again, so no more
//! goes than it takes. Nothing with an agent or a running environment is touched, and nothing on
//! another volume, where emptying a folder gives none of the room back.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use super::{By, Cleaning, Report, words};
use crate::dashboard::KeepFreeChoice;
use crate::dashboard::disk_space::DiskSpace;
use crate::map::Snapshot;
use crate::map::clean::{self, CargoEnv, CleanError, IdleFolder, PrepareError, Prepared};

/// How much free space the user can ask to keep, in the decimal gigabytes Finder counts in.
const KEEP_FREE_CHOICES: [u64; 4] = [
    25_000_000_000,
    50_000_000_000,
    100_000_000_000,
    200_000_000_000,
];

/// How often free space is read while nothing is being cleaned.
const CHECK_EVERY: Duration = Duration::from_secs(60);

/// A project this recently active is left alone even when the disk runs low: someone may be in the
/// middle of it, with no agent or environment to say so. A first guess.
pub(super) const QUIET_AT_LEAST: Duration = Duration::from_secs(60 * 60);

/// How long a folder whose clean was refused or failed is left out before it gets another try: what
/// stopped it, such as a build using it, may well pass.
const PASS_OVER_FOR: Duration = Duration::from_secs(60 * 60);

/// What cleaning by itself needs to remember.
#[derive(Debug, Default)]
pub(super) struct KeepingFree {
    /// `None` until the user chooses how much to keep.
    pub(super) floor: Option<u64>,
    next_check: Option<Instant>,
    /// Folders whose clean was refused or failed, and when each may be tried again.
    passed_over: HashMap<PathBuf, SystemTime>,
}

impl KeepingFree {
    /// Keeping `floor` free, starting afresh: a folder passed over before gets another chance.
    pub(super) fn with_floor(floor: Option<u64>) -> KeepingFree {
        KeepingFree {
            floor,
            ..KeepingFree::default()
        }
    }

    /// Read free space again at the next tick: a clean just finished.
    pub(super) fn check_soon(&mut self) {
        self.next_check = None;
    }

    pub(super) fn pass_over(&mut self, folder: PathBuf, now: SystemTime) {
        self.passed_over.insert(folder, now + PASS_OVER_FOR);
    }

    #[cfg(test)]
    pub(super) fn has_passed_over(&self, folder: &Path) -> bool {
        self.passed_over.contains_key(folder)
    }

    /// Whether the user keeps free space and less than that is free on the volume holding `path`.
    /// A volume that does not answer gives no reason to clean.
    pub(super) fn is_short_of_room(&self, path: &Path) -> bool {
        self.floor
            .is_some_and(|floor| free_bytes(path).is_ok_and(|free| free < floor))
    }
}

/// What the user can choose Sutra to keep free: nothing, so it never cleans by itself, then each
/// amount it offers.
pub(in crate::dashboard) fn keep_free_choices() -> Vec<KeepFreeChoice> {
    let amounts = KEEP_FREE_CHOICES.map(Some);
    std::iter::once(None)
        .chain(amounts)
        .map(|bytes| KeepFreeChoice {
            bytes,
            label: words::keep_free_choice(bytes),
        })
        .collect()
}

/// How many bytes are free on the volume holding `path` (see `DiskSpace`).
pub(in crate::dashboard) fn free_bytes(path: &Path) -> io::Result<u64> {
    DiskSpace::of(path).map(|space| space.free)
}

/// The volume holding `path`.
fn volume_of(path: &Path) -> io::Result<u64> {
    Ok(fs::metadata(path)?.dev())
}

/// Whether every folder `prepared` deletes is on the volume holding `path`. A folder that does not
/// answer counts as elsewhere.
pub(super) fn is_all_on_volume_of(prepared: &Prepared, path: &Path) -> bool {
    let Ok(volume) = volume_of(path) else {
        return false;
    };
    prepared
        .plan()
        .folders()
        .into_iter()
        .all(|folder| volume_of(folder).is_ok_and(|its_volume| its_volume == volume))
}

/// The folder to clean next: none while at least `floor` is `free`, else the one quiet the longest
/// that `may_clean` allows.
fn next_folder<'a>(
    map: &'a Snapshot,
    now: SystemTime,
    floor: u64,
    free: u64,
    may_clean: impl Fn(&IdleFolder) -> bool,
) -> Option<IdleFolder<'a>> {
    if free >= floor {
        return None;
    }
    clean::idle_folders(map, now, QUIET_AT_LEAST)
        .into_iter()
        .find(may_clean)
}

impl Cleaning {
    /// The clock ticked: if the user keeps free space, it is time to look, nothing is being cleaned
    /// and too little is free, work out the next folder's clean on a thread of its own.
    pub(in crate::dashboard) fn keep_free_space(
        &mut self,
        map: &Arc<Snapshot>,
        at: Instant,
        now: SystemTime,
        home: &Path,
    ) {
        let Some(floor) = self.keeping_free.floor else {
            return;
        };
        let due = self.keeping_free.next_check.is_none_or(|next| at >= next);
        if !due || self.is_busy() {
            return;
        }
        self.keeping_free.next_check = Some(at + CHECK_EVERY);
        self.keeping_free
            .passed_over
            .retain(|_, until| *until > now);
        // Nothing can be said about a volume that does not answer, and no reason to clean follows.
        let (Ok(free), Ok(volume)) = (free_bytes(home), volume_of(home)) else {
            return;
        };
        let chosen = next_folder(map, now, floor, free, |folder| {
            self.may_clean_unasked(folder, volume)
        })
        .map(|folder| (folder.dir.path.clone(), folder.project.path.clone()));
        if let Some((clicked, project)) = chosen {
            self.prepare_unasked(clicked, project, Arc::clone(map), home, now);
        }
    }

    /// Whether looking for room may pick `folder`: no clean is at work on it, it was not refused
    /// lately, and it is on `volume`, the one short of room, as emptying another gives none back.
    fn may_clean_unasked(&self, folder: &IdleFolder, volume: u64) -> bool {
        self.state_of(folder.dir).is_none()
            && !self.keeping_free.passed_over.contains_key(&folder.dir.path)
            && volume_of(&folder.dir.path).is_ok_and(|its_volume| its_volume == volume)
    }

    fn prepare_unasked(
        &mut self,
        clicked: PathBuf,
        project: PathBuf,
        map: Arc<Snapshot>,
        home: &Path,
        now: SystemTime,
    ) {
        let preparing = {
            let (folder, in_project) = (clicked.clone(), project.clone());
            let reported = (clicked.clone(), project);
            let env = CargoEnv::of_this_process(home);
            self.threads.run(
                "sutra-clean-prepare",
                move || clean::prepare(&folder, &in_project, &map, &env),
                move |prepared| Report::UnaskedPrepared {
                    clicked: reported.0,
                    project: reported.1,
                    prepared,
                },
            )
        };
        if let Err(err) = preparing {
            let title = words::stopped(&clicked, None, 0, home).0;
            self.banners.push((title, err.to_string()));
            self.keeping_free.pass_over(clicked, now);
            return;
        }
        self.preparing_unasked = true;
    }

    /// The next folder's clean has been worked out, or its thread ended with no word (`None`). It
    /// goes unless Sutra will not clean it or a build is using it, which nobody needs telling, or
    /// trying it failed, which a banner tells; either way the next look passes it over.
    pub(super) fn clean_unasked(
        &mut self,
        clicked: PathBuf,
        project: PathBuf,
        prepared: Option<Result<Prepared, PrepareError>>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        self.preparing_unasked = false;
        self.keeping_free.check_soon();
        let failure = match prepared {
            Some(Ok(prepared)) => {
                self.queue(clicked, prepared, By::Sutra { project });
                self.start_next(map, now, home);
                return;
            }
            Some(Err(PrepareError::Locks(err @ CleanError::Io { .. }))) => Some(Some(err)),
            Some(Err(_)) => None,
            None => Some(None),
        };
        if let Some(failure) = failure {
            self.banners
                .push(words::stopped(&clicked, failure.as_ref(), 0, home));
        }
        self.keeping_free.pass_over(clicked, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::cleaning::tests::{
        ALWAYS_SHORT, cleaning, idle_clean, offered, prepared_at,
    };
    use crate::map::Project;
    use crate::map::test_support::{
        TempDir, agent_session, at, cargo_target, measured, project_at, snapshot_of,
    };
    use crate::map::{AgentSession, AgentStatus};

    const DAY: u64 = 86_400;
    const LARGE: u64 = 100 * clean::WORTH_CLEANING_BYTES;

    fn now() -> SystemTime {
        at(1_000 * DAY)
    }

    fn map() -> Snapshot {
        let project = |name: &str, days: u64| Project {
            last_activity: Some(at((1_000 - days) * DAY)),
            artifacts: vec![cargo_target(
                format!("/code/{name}/target"),
                measured(LARGE),
            )],
            ..project_at(format!("/code/{name}"))
        };
        snapshot_of(vec![project("newer", 2), project("older", 9)])
    }

    #[test]
    fn with_enough_free_nothing_is_cleaned() {
        let map = map();
        assert!(next_folder(&map, now(), 100, 100, |_| true).is_none());
    }

    #[test]
    fn with_too_little_free_the_longest_quiet_goes_first_unless_passed_over() {
        let map = map();
        let name = |folder: Option<IdleFolder>| folder.map(|folder| folder.project.name.clone());
        assert_eq!(
            name(next_folder(&map, now(), 100, 99, |_| true)).as_deref(),
            Some("older")
        );
        assert_eq!(
            name(next_folder(&map, now(), 100, 99, |folder| {
                folder.project.name != "older"
            }))
            .as_deref(),
            Some("newer")
        );
        assert!(next_folder(&map, now(), 100, 99, |_| false).is_none());
    }

    #[test]
    fn the_choices_offer_never_cleaning_by_itself_then_each_amount() {
        let choices = keep_free_choices();
        let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Never Clean by Itself",
                "Keep 25 GB Free",
                "Keep 50 GB Free",
                "Keep 100 GB Free",
                "Keep 200 GB Free",
            ]
        );
        assert_eq!(choices[0].bytes, None);
        assert_eq!(choices[3].bytes, Some(100_000_000_000));
    }

    fn keeping(floor: Option<u64>) -> Cleaning {
        Cleaning {
            keeping_free: KeepingFree::with_floor(floor),
            ..cleaning()
        }
    }

    /// A map of one idle project in `fixture`, with a large build folder.
    fn idle_project_in(fixture: &TempDir) -> (PathBuf, PathBuf, Arc<Snapshot>) {
        let target = fixture.dir("app/target");
        let project = fixture.path("app");
        let map = Arc::new(snapshot_of(vec![Project {
            artifacts: vec![cargo_target(target.clone(), measured(LARGE))],
            ..project_at(project.clone())
        }]));
        (target, project, map)
    }

    #[test]
    fn a_folder_on_another_volume_gives_no_room_back_and_is_not_picked() {
        let fixture = TempDir::new("auto-volume");
        let (_, _, map) = idle_project_in(&fixture);
        let cleaning = cleaning();
        let idle = clean::idle_folders(&map, now(), QUIET_AT_LEAST);
        let volume = volume_of(fixture.root()).unwrap();
        assert!(cleaning.may_clean_unasked(&idle[0], volume));
        assert!(!cleaning.may_clean_unasked(&idle[0], volume + 1));
    }

    #[test]
    fn a_folder_passed_over_gets_another_try_once_its_time_is_up_and_is_then_forgotten() {
        let fixture = TempDir::new("auto-pass-over");
        let (target, _, map) = idle_project_in(&fixture);
        let mut cleaning = keeping(ALWAYS_SHORT);
        cleaning.keeping_free.pass_over(target.clone(), now());

        cleaning.keep_free_space(&map, Instant::now(), now(), fixture.root());
        assert!(!cleaning.is_busy(), "left out for now");

        cleaning.keeping_free.check_soon();
        let later = now() + PASS_OVER_FOR;
        cleaning.keep_free_space(&map, Instant::now(), later, fixture.root());
        assert!(cleaning.is_busy(), "being worked out again");
        assert!(!cleaning.keeping_free.has_passed_over(&target));
    }

    #[test]
    fn a_question_waiting_for_the_user_does_not_hold_up_keeping_free_space() {
        let fixture = TempDir::new("auto-while-asking");
        let (_, _, map) = idle_project_in(&fixture);
        let mut cleaning = keeping(ALWAYS_SHORT);
        // A card's question and the toolbar's, each about a folder of its own.
        let card = fixture.dir("card/target");
        let dir = cargo_target(card.clone(), measured(LARGE));
        let home = fixture.root();
        cleaning.start(
            &project_at(fixture.path("card")),
            &dir,
            Arc::clone(&map),
            home,
        );
        let mut reaching = prepared_at(&card);
        reaching.sharing.projects = vec!["fwd".to_string()];
        cleaning.asking(&card, Some(Ok(reaching)), None, now(), home);
        let other = fixture.dir("other/target");
        cleaning.asking_idle(offered(vec![idle_clean(&[&other])], Vec::new()));
        assert_eq!(cleaning.take_alerts().len(), 2, "both questions are open");

        cleaning.keep_free_space(&map, Instant::now(), now(), home);
        assert!(cleaning.preparing_unasked);
    }

    #[test]
    fn a_refusal_passes_a_folder_over_quietly_and_a_failure_to_try_says_so() {
        let home = Path::new("/Users/dev");
        let target = PathBuf::from("/Users/dev/code/app/target");
        let project = PathBuf::from("/Users/dev/code/app");
        let outcome_of = |prepared| {
            let mut cleaning = Cleaning {
                preparing_unasked: true,
                ..cleaning()
            };
            cleaning.clean_unasked(target.clone(), project.clone(), prepared, None, now(), home);
            assert!(!cleaning.is_busy());
            assert!(cleaning.keeping_free.has_passed_over(&target));
            assert!(cleaning.take_alerts().is_empty(), "nobody asked for it");
            cleaning.take_banners().len()
        };
        let build_running = PrepareError::Locks(CleanError::BuildRunning {
            lock: target.join("debug/.cargo-lock"),
        });
        assert_eq!(outcome_of(Some(Err(build_running))), 0);
        let cannot_lock = PrepareError::Locks(CleanError::Io {
            path: target.join("debug/.cargo-lock"),
            kind: io::ErrorKind::PermissionDenied,
        });
        assert_eq!(outcome_of(Some(Err(cannot_lock))), 1);
        assert_eq!(outcome_of(None), 1, "its thread ended with no word");
    }

    #[test]
    fn a_clean_worked_out_while_the_map_was_replaced_does_not_go_and_later_looks_still_run() {
        let fixture = TempDir::new("auto-map-replaced");
        let target = fixture.dir("app/target");
        let mut cleaning = Cleaning {
            preparing_unasked: true,
            ..keeping(ALWAYS_SHORT)
        };

        cleaning.clean_unasked(
            target.clone(),
            fixture.path("app"),
            Some(Ok(prepared_at(&target))),
            None,
            now(),
            fixture.root(),
        );

        assert!(cleaning.running.is_empty());
        assert!(!cleaning.is_busy(), "the next look is free to run");
        assert_eq!(cleaning.folders.len(), 0, "its card shows no clean");
        assert!(target.exists());
    }

    #[test]
    fn a_clean_sutra_chose_goes_only_if_room_is_still_short_and_the_project_still_idle() {
        let fixture = TempDir::new("auto-still-wanted");
        let (target, project, idle_map) = idle_project_in(&fixture);
        let at_work = snapshot_of(vec![Project {
            agents: vec![agent_session("s1", AgentStatus::Busy)],
            ..idle_map.projects[0].clone()
        }]);
        let home = fixture.root();
        let start = |floor, map: &Snapshot| {
            let mut cleaning = keeping(floor);
            let by = By::Sutra {
                project: project.clone(),
            };
            cleaning.queue(target.clone(), prepared_at(&target), by);
            cleaning.start_next(Some(map), now(), home);
            cleaning
        };

        let stopped_keeping = start(None, &idle_map);
        assert!(stopped_keeping.running.is_empty(), "the user turned it off");
        assert!(stopped_keeping.folders.is_empty());
        assert!(!stopped_keeping.keeping_free.has_passed_over(&target));

        let enough_free = start(Some(1), &idle_map);
        assert!(enough_free.running.is_empty(), "room came back");
        assert!(!enough_free.keeping_free.has_passed_over(&target));

        let agent_started = start(ALWAYS_SHORT, &at_work);
        assert!(agent_started.running.is_empty());
        assert!(agent_started.keeping_free.has_passed_over(&target));

        assert!(!start(ALWAYS_SHORT, &idle_map).running.is_empty());
    }

    #[test]
    fn a_session_at_rest_however_long_keeps_its_folders_from_keeping_free_space() {
        let fixture = TempDir::new("auto-at-rest");
        let (target, project, idle_map) = idle_project_in(&fixture);
        // At rest for a month, in a project quiet as long, which the toolbar asks about apart.
        let month_ago = now() - Duration::from_secs(30 * DAY);
        let resting = snapshot_of(vec![Project {
            agents: vec![AgentSession {
                since: Some(month_ago),
                ..agent_session("s1", AgentStatus::Idle)
            }],
            last_activity: Some(month_ago),
            ..idle_map.projects[0].clone()
        }]);
        assert_eq!(
            clean::at_rest_folders(&resting, now(), Duration::from_secs(DAY)).len(),
            1
        );

        assert!(
            next_folder(&resting, now(), 100, 99, |_| true).is_none(),
            "never picked"
        );
        let mut cleaning = keeping(ALWAYS_SHORT);
        let by = By::Sutra { project };
        cleaning.queue(target.clone(), prepared_at(&target), by);
        cleaning.start_next(Some(&resting), now(), fixture.root());
        assert!(
            cleaning.running.is_empty(),
            "nor cleaned at its turn, should one have been picked before the session came"
        );
    }

    #[test]
    fn free_space_comes_from_the_volume_holding_the_path() {
        let fixture = TempDir::new("auto-free-bytes");
        assert!(free_bytes(fixture.root()).unwrap() > 0);
        assert_eq!(
            free_bytes(&fixture.path("nothing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
