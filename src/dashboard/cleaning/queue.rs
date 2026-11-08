//! The cleans wanted, a few at a time: whether each may still go when its turn comes, the thread
//! that does the deleting, and what happens when it ends.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::when_idle::IdleTimes;
use super::{CleanOutcome, Cleaning, OFFER_QUIET_AT_LEAST, Report, Stage, auto, overlap, words};
use crate::dashboard::state::Alert;
use crate::map::clean::{self, Prepared, Progress};
use crate::map::{GroupKey, Project, Snapshot};

/// Who wanted a clean, which decides how its end is told and what must still hold when its turn
/// comes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum By {
    /// The user asked for this folder, which holds whatever comes next: a failure is told in an
    /// alert, and success shows on the card they are looking at.
    User,
    /// The user said yes to every idle folder at once, so this one goes only if the project at
    /// `project` is still idle when its turn comes. Its end is told as `User`.
    UserForIdle { project: PathBuf },
    /// Sutra chose it to keep free space, so it goes only if room is still short on the volume
    /// holding the home folder, every folder it deletes is on that volume and clear of those a
    /// question waiting for the user asks about, and the project at `project` is still idle when
    /// its turn comes. Nobody is waiting, so a banner tells of its end.
    Sutra { project: PathBuf },
    /// Sutra chose it as the user chose for the groups, by their `times` when it looked, so it
    /// goes only if the project at `project`, in `group`, and every project sharing its folders
    /// are still idle by their own groups' times, and it keeps clear of the folders a question
    /// waiting for the user asks about. A banner tells of a failure, and another of what the
    /// group's cleans gave back once all have ended.
    WhenIdle {
        project: PathBuf,
        group: GroupKey,
        times: Arc<IdleTimes>,
    },
}

/// How many cleans may delete at once: those the user starts one after another each show moving,
/// and each deletes on every core already, so more at once would only take turns in the kernel. A
/// first guess.
const CLEANS_AT_ONCE: usize = 3;

/// A clean wanted, waiting its turn.
#[derive(Debug)]
pub(super) struct Waiting {
    pub(super) clicked: PathBuf,
    pub(super) prepared: Prepared,
    pub(super) by: By,
}

/// A clean under way.
#[derive(Debug)]
pub(super) struct Running {
    /// The folder on the map that led to it.
    pub(super) clicked: PathBuf,
    /// Every folder it deletes.
    pub(super) folders: Vec<PathBuf>,
    pub(super) by: By,
    /// What it has deleted so far, which is all there is to say should it stop early.
    pub(super) progress: Arc<Progress>,
}

impl Cleaning {
    /// Show `prepared`'s folders as being cleaned, and line it up behind what is running.
    pub(super) fn queue(&mut self, clicked: PathBuf, prepared: Prepared, by: By) {
        for (folder, size) in prepared.plan().folders().into_iter().zip(&prepared.sizes) {
            let before = size.map(|usage| usage.bytes);
            self.folders
                .insert(folder.to_path_buf(), Stage::Deleting { before });
        }
        self.waiting.push_back(Waiting {
            clicked,
            prepared,
            by,
        });
    }

    /// Whether `waiting` may start now, as `map` shows things, with `None` when there is no running
    /// engine to go by. What the user asked for by name goes, but one that rests on nobody using
    /// the folder must look again, since the look it was queued on can be many minutes old.
    fn may_start(
        &mut self,
        waiting: &Waiting,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) -> bool {
        // How long the project and each sharer must have been quiet, as `quiet_at_least` says.
        let still_idle = |project: &Path, quiet_at_least: &dyn Fn(&Project) -> Option<Duration>| {
            map.is_some_and(|map| {
                clean::is_still_idle(&waiting.prepared, project, map, now, quiet_at_least)
            })
        };
        match &waiting.by {
            By::User => true,
            By::UserForIdle { project } => still_idle(project, &|_| Some(OFFER_QUIET_AT_LEAST)),
            By::Sutra { project } => {
                if !self.keeping_free.is_short_of_room(home) {
                    return false;
                }
                // The folder was picked on its own, and its plan can reach further: to a
                // build-dir set on another disk, where emptying it gives none of the room back. A
                // question may also have asked about one of its folders since, and the user's
                // answer decides that folder.
                let may_go = still_idle(project, &|_| Some(auto::QUIET_AT_LEAST))
                    && auto::is_all_on_volume_of(&waiting.prepared, home)
                    && self
                        .questions
                        .asked_about_in(&waiting.clicked, &waiting.prepared)
                        .is_none();
                if !may_go {
                    self.keeping_free.pass_over(waiting.clicked.clone(), now);
                }
                may_go
            }
            // Should it not go, the next look finds it again while it stays idle.
            By::WhenIdle { project, times, .. } => {
                still_idle(project, &|its| times.of(its))
                    && (self.questions)
                        .asked_about_in(&waiting.clicked, &waiting.prepared)
                        .is_none()
            }
        }
    }

    /// Start the cleans waiting that may still go, each on a thread of its own, as many as may run
    /// at once. One that may not goes back to showing its size: its card shows why. One whose
    /// folders a running clean has too waits its turn, as the two would refuse each other at the
    /// folder's locks. A group whose cleans have all ended then says what went.
    pub(super) fn start_next(&mut self, map: Option<&Snapshot>, now: SystemTime, home: &Path) {
        let mut held_back = VecDeque::new();
        while self.running.len() < CLEANS_AT_ONCE {
            let Some(waiting) = self.waiting.pop_front() else {
                break;
            };
            if self.shares_a_running_clean(&waiting) {
                held_back.push_back(waiting);
                continue;
            }
            if !self.may_start(&waiting, map, now, home) {
                self.show_sizes_again(waiting.prepared.plan().folders());
                continue;
            }
            self.begin(waiting, home);
        }
        // Back at the front, in their order, so they go first once their folders are free.
        while let Some(waiting) = held_back.pop_back() {
            self.waiting.push_front(waiting);
        }
        if self.running.is_empty() && self.waiting.is_empty() {
            self.tell_what_stopped();
        }
        self.end_rounds();
    }

    /// Whether `waiting` deletes a folder a running clean deletes, or one inside or around it.
    fn shares_a_running_clean(&self, waiting: &Waiting) -> bool {
        let running_folders: Vec<&PathBuf> = self
            .running
            .iter()
            .flat_map(|running| &running.folders)
            .collect();
        waiting.prepared.plan().folders().into_iter().any(|folder| {
            running_folders
                .iter()
                .any(|running| overlap(folder, running))
        })
    }

    /// Start deleting what `waiting` deletes, on a thread of its own. A thread that cannot start
    /// is told once nothing is left to run.
    fn begin(&mut self, waiting: Waiting, home: &Path) {
        let Waiting {
            clicked,
            prepared,
            by,
        } = waiting;
        let folders: Vec<PathBuf> = prepared
            .plan()
            .folders()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        let progress = Arc::new(Progress::default());
        let deleting = {
            let progress = Arc::clone(&progress);
            let target = clicked.clone();
            self.threads.run(
                "sutra-clean",
                move || prepared.lock().and_then(|locked| locked.delete(&progress)),
                move |deleted| Report::Cleaned {
                    target,
                    outcome: CleanOutcome::from(deleted),
                },
            )
        };
        if let Err(err) = deleting {
            self.show_sizes_again(folders.iter().map(PathBuf::as_path));
            let title = words::stopped(&clicked, None, 0, home).0;
            self.stopped.push((title, err.to_string()));
            return;
        }
        self.running.push(Running {
            clicked,
            folders,
            by,
            progress,
        });
    }

    /// Show `folders`' sizes again on their cards, all but those a clean still waiting or running
    /// will delete: two cards can ask for a folder two projects build in, and the clean that goes
    /// on keeps its folder showing as being cleaned.
    pub(super) fn show_sizes_again<'a>(&mut self, folders: impl IntoIterator<Item = &'a Path>) {
        for folder in folders {
            let waits = self
                .waiting
                .iter()
                .any(|waiting| waiting.prepared.plan().folders().contains(&folder));
            let runs = self
                .running
                .iter()
                .any(|running| running.folders.iter().any(|its| its == folder));
            if !waits && !runs {
                self.folders.remove(folder);
            }
        }
    }

    /// Everything queued has ended: one alert for all that stopped.
    fn tell_what_stopped(&mut self) {
        if self.stopped.is_empty() {
            return;
        }
        let (title, detail) = words::all_stopped(&self.stopped);
        self.stopped.clear();
        self.alerts.push(Alert::notice(title, detail));
    }

    /// The clean started from `target` ended at `now`. A failure is told to the user, and its
    /// folders show their sizes again. However it ended its folders hold less than the map says, so
    /// they are to be measured. Then what waits may start.
    pub(super) fn finished(
        &mut self,
        target: &Path,
        outcome: CleanOutcome,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        let Some(index) = self
            .running
            .iter()
            .position(|running| running.clicked == target)
        else {
            return;
        };
        let running = self.running.swap_remove(index);
        self.measure_again.extend(running.folders.iter().cloned());
        let failure = match outcome {
            CleanOutcome::Cleaned(cleaned) => {
                match &running.by {
                    By::User | By::UserForIdle { .. } => {}
                    By::Sutra { .. } => {
                        let floor = self.keeping_free.floor.unwrap_or_default();
                        self.banners
                            .push(words::made_room(target, &cleaned, floor, home));
                        self.keeping_free.check_soon();
                    }
                    By::WhenIdle { group, .. } => self.cleaned_when_idle(group, &cleaned),
                }
                for freed in cleaned.folders {
                    let stage = Stage::Deleted {
                        at: now,
                        left: freed.left,
                    };
                    self.folders.insert(freed.folder, stage);
                }
                self.start_next(map, now, home);
                return;
            }
            CleanOutcome::Failed(err) => Some(err),
            CleanOutcome::Lost => None,
        };
        self.show_sizes_again(running.folders.iter().map(PathBuf::as_path));
        let stopped = words::stopped(
            target,
            failure.as_ref(),
            running.progress.files_deleted(),
            home,
        );
        match running.by {
            By::User | By::UserForIdle { .. } => self.stopped.push(stopped),
            By::Sutra { .. } => {
                self.banners.push(stopped);
                self.keeping_free.pass_over(target.to_path_buf(), now);
                self.keeping_free.check_soon();
            }
            By::WhenIdle { .. } => self.failed_when_idle(target, stopped, now),
        }
        self.start_next(map, now, home);
    }

    #[cfg(test)]
    fn keeping_free_passed_over(&self, folder: &Path) -> bool {
        self.keeping_free.has_passed_over(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::cleaning::CleanState;
    use crate::dashboard::cleaning::tests::{
        ALWAYS_SHORT, cleaning, deleting_two, idle_clean, next_report, prepared_at, prepared_clean,
        prepared_of, reporting, target,
    };
    use crate::map::Project;
    use crate::map::clean::{CleanError, Cleaned, Freed, IdleCleans};
    use crate::map::test_support::{at, cargo_target, project_at, snapshot_of};

    #[test]
    fn a_clean_deletes_on_a_thread_of_its_own_whose_report_its_card_then_shows() {
        let fixture = crate::map::test_support::TempDir::new("cleaning-thread");
        fixture.write("app/target/debug/big", [1; 64 * 1024]);
        let folder = fixture.path("app/target");
        let (mut cleaning, inbox) = reporting();
        cleaning.queue(folder.clone(), prepared_at(&folder), By::User);
        cleaning.start_next(None, at(100), fixture.root());

        let Report::Cleaned { target, outcome } = next_report(&inbox) else {
            panic!("the clean's thread reports how it ended");
        };
        assert_eq!(target, folder);
        assert!(matches!(outcome, CleanOutcome::Cleaned(_)), "{outcome:?}");
        assert!(!fixture.path("app/target/debug/big").exists());

        cleaning.finished(&target, outcome, None, at(200), fixture.root());
        assert!(cleaning.running.is_empty());
        let dir = cargo_target(folder, crate::map::test_support::measured(64 * 1024));
        assert!(matches!(
            cleaning.state_of(&dir),
            Some(CleanState::Deleted { .. })
        ));
    }

    #[test]
    fn a_finished_clean_shows_what_it_left_in_each_folder_and_has_them_measured() {
        let mut cleaning = deleting_two();
        let cleaned = Cleaned {
            folders: vec![
                Freed {
                    folder: PathBuf::from("/code/app/target"),
                    bytes: 42,
                    left: 3,
                },
                Freed {
                    folder: PathBuf::from("/cache/app"),
                    bytes: 7,
                    left: 1,
                },
            ],
        };
        cleaning.finished(
            Path::new("/code/app/target"),
            CleanOutcome::Cleaned(cleaned),
            None,
            at(100),
            Path::new("/Users/dev"),
        );
        assert_eq!(
            cleaning.state_of(&target(Some(at(50)))),
            Some(CleanState::Deleted { left: 3 })
        );
        assert_eq!(
            cleaning.folders.get(Path::new("/cache/app")),
            Some(&Stage::Deleted {
                at: at(100),
                left: 1
            })
        );
        assert!(cleaning.running.is_empty());
        assert_eq!(
            cleaning.take_measure_again(),
            [
                PathBuf::from("/code/app/target"),
                PathBuf::from("/cache/app")
            ]
        );
    }

    #[test]
    fn a_clean_that_stopped_early_has_its_folders_measured_too_and_is_told_in_a_notice() {
        let mut cleaning = deleting_two();
        let failed = CleanOutcome::Failed(CleanError::Io {
            path: PathBuf::from("/code/app/target/debug/deps"),
            kind: std::io::ErrorKind::PermissionDenied,
        });
        cleaning.finished(
            Path::new("/code/app/target"),
            failed,
            None,
            at(100),
            Path::new("/Users/dev"),
        );
        assert_eq!(cleaning.take_measure_again().len(), 2);
        assert_eq!(cleaning.state_of(&target(Some(at(50)))), None);
        assert!(matches!(
            cleaning.take_alerts().as_slice(),
            [Alert::Notice { title, .. }] if title == "Couldn't finish cleaning /code/app/target"
        ));
    }

    #[test]
    fn cleans_of_separate_folders_run_together_up_to_a_limit_and_one_sharing_a_folder_waits() {
        let fixture = crate::map::test_support::TempDir::new("cleaning-together");
        let home = fixture.root().to_path_buf();
        let folders: Vec<PathBuf> = ["a", "b", "c", "d"]
            .map(|name| fixture.dir(&format!("{name}/target")))
            .to_vec();
        let mut cleaning = cleaning();
        cleaning.queue(folders[0].clone(), prepared_at(&folders[0]), By::User);
        // The same folder again, as a second card would ask for one two projects build in.
        let again = folders[0].join("debug");
        cleaning.queue(again.clone(), prepared_at(&again), By::User);
        for folder in &folders[1..] {
            cleaning.queue(folder.clone(), prepared_at(folder), By::User);
        }

        cleaning.start_next(None, at(100), &home);
        let running: Vec<&PathBuf> = cleaning.running.iter().map(|r| &r.clicked).collect();
        assert_eq!(running, [&folders[0], &folders[1], &folders[2]]);
        let waiting: Vec<&PathBuf> = cleaning.waiting.iter().map(|w| &w.clicked).collect();
        assert_eq!(waiting, [&again, &folders[3]], "each keeps its place");

        let outcome = CleanOutcome::Cleaned(Cleaned {
            folders: Vec::new(),
        });
        cleaning.finished(&folders[0], outcome, None, at(100), &home);

        let running: Vec<&PathBuf> = cleaning.running.iter().map(|r| &r.clicked).collect();
        assert_eq!(running, [&folders[2], &folders[1], &again]);
        let waiting: Vec<&PathBuf> = cleaning.waiting.iter().map(|w| &w.clicked).collect();
        assert_eq!(waiting, [&folders[3]]);
    }

    #[test]
    fn a_clean_of_idle_folders_goes_only_if_its_project_is_still_idle_when_its_turn_comes() {
        let dir = cargo_target(
            "/code/app/target",
            crate::map::test_support::measured(1 << 30),
        );
        let idle = Project {
            artifacts: vec![dir.clone()],
            ..project_at("/code/app")
        };
        let at_work = Project {
            agents: vec![crate::map::test_support::agent_session(
                "s1",
                crate::map::AgentStatus::Busy,
            )],
            ..idle.clone()
        };
        let may_start = |by: By, map: Option<&Snapshot>| {
            let waiting = Waiting {
                clicked: dir.path.clone(),
                prepared: prepared_clean("/code/app/target"),
                by,
            };
            cleaning().may_start(&waiting, map, at(1_000), Path::new("/Users/dev"))
        };
        let for_idle = || By::UserForIdle {
            project: PathBuf::from("/code/app"),
        };

        assert!(may_start(
            for_idle(),
            Some(&snapshot_of(vec![idle.clone()]))
        ));
        assert!(!may_start(
            for_idle(),
            Some(&snapshot_of(vec![at_work.clone()]))
        ));
        assert!(
            !may_start(for_idle(), Some(&snapshot_of(Vec::new()))),
            "it left the map"
        );
        assert!(!may_start(for_idle(), None), "the engine stopped");
        assert!(
            may_start(By::User, Some(&snapshot_of(vec![at_work]))),
            "the user asked for this folder, agent and all"
        );
    }

    #[test]
    fn a_clean_that_may_not_go_shows_its_size_again_and_the_next_one_starts() {
        let fixture = crate::map::test_support::TempDir::new("cleaning-dropped");
        let target = fixture.dir("app/target");
        let map = snapshot_of(Vec::new());
        let mut cleaning = cleaning();
        let gone = By::UserForIdle {
            project: PathBuf::from("/code/gone"),
        };
        cleaning.queue(target.clone(), prepared_at(&target), gone);
        let next = fixture.dir("other/target");
        cleaning.queue(next.clone(), prepared_at(&next), By::User);

        cleaning.start_next(Some(&map), at(100), fixture.root());

        assert_eq!(cleaning.running[0].clicked, next);
        assert!(!cleaning.folders.contains_key(&target));
        assert!(target.exists());
    }

    #[test]
    fn a_folder_another_clean_still_deletes_goes_on_showing_as_being_cleaned_when_one_is_dropped() {
        let fixture = crate::map::test_support::TempDir::new("cleaning-shared-dropped");
        let shared = fixture.dir("shared/target");
        let map = snapshot_of(Vec::new());
        let mut cleaning = cleaning();
        let gone = By::UserForIdle {
            project: PathBuf::from("/code/gone"),
        };
        cleaning.queue(shared.clone(), prepared_at(&shared), gone);
        // The same folder again, as a second card would ask for one two projects build in.
        cleaning.queue(shared.clone(), prepared_at(&shared), By::User);

        cleaning.start_next(Some(&map), at(100), fixture.root());

        assert_eq!(cleaning.running.len(), 1, "the second clean goes");
        assert!(
            cleaning.folders.contains_key(&shared),
            "its card goes on showing the clean that goes"
        );
    }

    #[test]
    fn a_clean_sutra_chose_does_not_go_when_a_folder_it_deletes_is_on_another_volume() {
        let fixture = crate::map::test_support::TempDir::new("cleaning-other-volume");
        let target = fixture.dir("app/target");
        let project = fixture.path("app");
        let map = snapshot_of(vec![project_at(project.clone())]);
        let mut cleaning = cleaning();
        cleaning.set_keep_free_floor(ALWAYS_SHORT);
        let may_start = |cleaning: &mut Cleaning, build_dir: &Path| {
            let waiting = Waiting {
                clicked: target.clone(),
                prepared: prepared_of(&[&target, build_dir]),
                by: By::Sutra {
                    project: project.clone(),
                },
            };
            cleaning.may_start(&waiting, Some(&map), at(1_000), fixture.root())
        };

        assert!(
            may_start(&mut cleaning, &fixture.dir("cache/app")),
            "a build-dir on the same volume goes with it"
        );
        // `/dev` is a volume of its own, standing in for a build-dir set on an external disk. Only
        // whether the clean may start is asked, so nothing is deleted.
        assert!(
            !may_start(&mut cleaning, Path::new("/dev")),
            "emptying it frees nothing on the volume short of room"
        );
        assert!(cleaning.keeping_free_passed_over(&target));
    }

    #[test]
    fn a_clean_sutra_chose_keeps_out_of_a_folder_a_question_asks_about_and_goes_elsewhere() {
        let fixture = crate::map::test_support::TempDir::new("cleaning-asked-about");
        let target = fixture.dir("app/target");
        let project = fixture.path("app");
        let asked = fixture.dir("cache/app/target");
        let map = snapshot_of(vec![project_at(project.clone())]);
        let mut cleaning = cleaning();
        cleaning.set_keep_free_floor(ALWAYS_SHORT);
        cleaning.asking_idle(Some(IdleCleans {
            cleans: vec![idle_clean(&[&asked])],
            left_out: Vec::new(),
        }));
        let may_start = |cleaning: &mut Cleaning, build_dir: &Path| {
            let waiting = Waiting {
                clicked: target.clone(),
                prepared: prepared_of(&[&target, build_dir]),
                by: By::Sutra {
                    project: project.clone(),
                },
            };
            cleaning.may_start(&waiting, Some(&map), at(1_000), fixture.root())
        };

        assert!(
            may_start(&mut cleaning, &fixture.dir("elsewhere")),
            "a build-dir apart from it goes"
        );
        let inside = fixture.dir("cache/app/target/debug");
        let around = fixture.path("cache/app");
        for build_dir in [&asked, &inside, &around] {
            assert!(
                !may_start(&mut cleaning, build_dir),
                "{} waits for the answer",
                build_dir.display()
            );
        }
        assert!(cleaning.keeping_free_passed_over(&target));
    }

    #[test]
    fn a_clean_sutra_chose_ends_in_a_banner_and_is_passed_over_when_it_fails() {
        let mut cleaning = cleaning();
        cleaning.set_keep_free_floor(Some(100_000_000_000));
        let home = Path::new("/Users/dev");
        let target = Path::new("/Users/dev/code/app/target");
        let running = || Running {
            clicked: target.to_path_buf(),
            folders: vec![target.to_path_buf()],
            by: By::Sutra {
                project: PathBuf::from("/Users/dev/code/app"),
            },
            progress: Arc::default(),
        };

        cleaning.running = vec![running()];
        let cleaned = Cleaned {
            folders: vec![Freed {
                folder: target.to_path_buf(),
                bytes: 46_000_000_000,
                left: 4096,
            }],
        };
        cleaning.finished(target, CleanOutcome::Cleaned(cleaned), None, at(100), home);
        assert_eq!(
            cleaning.take_banners(),
            [(
                "Made some room".to_string(),
                "Cleaned ~/code/app/target: 46 GB back. You asked to keep 100 GB free.".to_string()
            )]
        );

        cleaning.running = vec![running()];
        let failed = CleanOutcome::Failed(CleanError::BuildRunning {
            lock: target.join("debug/.cargo-lock"),
        });
        cleaning.finished(target, failed, None, at(200), home);
        assert_eq!(cleaning.take_banners().len(), 1);
        assert!(
            cleaning.stopped.is_empty() && cleaning.take_alerts().is_empty(),
            "no alert for a clean nobody asked for"
        );
        assert!(cleaning.keeping_free_passed_over(target));
    }

    #[test]
    fn what_stopped_a_clean_the_user_asked_for_waits_for_the_others_to_end() {
        let mut cleaning = cleaning();
        let home = Path::new("/Users/dev");
        let target = Path::new("/Users/dev/code/app/target");
        cleaning.running = vec![Running {
            clicked: target.to_path_buf(),
            folders: vec![target.to_path_buf()],
            by: By::User,
            progress: Arc::default(),
        }];
        let fixture = crate::map::test_support::TempDir::new("cleaning-stopped");
        let next = fixture.path("gone-long-ago/target");
        cleaning.waiting.push_back(Waiting {
            clicked: next.clone(),
            prepared: prepared_at(&next),
            by: By::User,
        });

        let failed = CleanOutcome::Failed(CleanError::BuildRunning {
            lock: target.join("debug/.cargo-lock"),
        });
        cleaning.finished(target, failed, None, at(100), home);

        assert_eq!(
            cleaning.stopped.len(),
            1,
            "told once nothing is left to run"
        );
        assert!(cleaning.take_alerts().is_empty());
        assert!(!cleaning.running.is_empty());
    }
}
