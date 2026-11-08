//! Cleaning build folders: the threads that work out what a clean deletes and do the deleting,
//! several cleans at once when their folders lie apart, the questions when a clean does more than
//! its click says, and what the card shows about each folder until the map has measured it again.
//! Sutra cleans by itself too: to keep free space (`auto`), and a group's folders once idle as long
//! as the user chose (`when_idle`).

mod asking;
mod auto;
mod queue;
mod threads;
mod when_idle;
mod words;

pub(super) use auto::{free_bytes, keep_free_choices};
pub(super) use threads::{CleanOutcome, Report, Threads};
pub(super) use when_idle::CleanWhenIdle;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::state::{Alert, Question};
use crate::map::clean;
use crate::map::{ArtifactDir, Project, Snapshot};
use asking::{Clicked, OpenQuestions};
use auto::KeepingFree;
use queue::{By, Running, Waiting};
use when_idle::WhenIdle;

/// How long a project must have been quiet for the toolbar to offer its build folders. A folder is
/// rebuilt when its project is next used, and on Daniel's Mac (2026-11-04) three days separates what
/// has been left from what is in use: 282 GB idle at three days, 532 GB at one, 4 GB at seven.
const OFFER_QUIET_AT_LEAST: Duration = Duration::from_secs(3 * 24 * 60 * 60);

/// The build folders being cleaned, or cleaned and not yet measured again.
#[derive(Debug)]
pub(super) struct Cleaning {
    /// Starts the threads that work out and do the cleans.
    threads: Threads,
    folders: HashMap<PathBuf, Stage>,
    /// Each click on Clean still being worked out, by the folder clicked.
    preparing: HashMap<PathBuf, Clicked>,
    /// The questions waiting for an answer, and the folders each would clean.
    questions: OpenQuestions,
    /// The cleans of every idle folder are being worked out.
    preparing_idle: bool,
    /// A clean Sutra chose by itself is being worked out.
    preparing_unasked: bool,
    /// Cleans waiting their turn, behind `CLEANS_AT_ONCE` running or a running clean of the same
    /// folders.
    waiting: VecDeque<Waiting>,
    running: Vec<Running>,
    /// What stopped and has not been told yet: told once nothing is left to run, so that a burst of
    /// failures makes one alert.
    stopped: Vec<(String, String)>,
    /// Alerts for the user, for the dashboard to show in turn.
    alerts: Vec<Alert>,
    /// Banners about cleans Sutra made by itself, for the notifier to send.
    banners: Vec<(String, String)>,
    /// Folders a clean just deleted from, for the engine to measure again.
    measure_again: Vec<PathBuf>,
    keeping_free: KeepingFree,
    when_idle: WhenIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Being deleted or waiting its turn. It held `before` when the clean was worked out, if that
    /// was known.
    Deleting { before: Option<u64> },
    /// Deleted at `at`, leaving `left`. The map shows the old size until it measures the folder
    /// again.
    Deleted { at: SystemTime, left: u64 },
}

/// What a clean means for a build folder's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CleanState {
    /// Being worked out, asked about, waiting its turn or being deleted. It held `before`, if that
    /// was known, and has given back `freed` by the clean's own count.
    Deleting { before: Option<u64>, freed: u64 },
    /// Deleted and not yet measured again, holding `left`.
    Deleted { left: u64 },
}

impl CleanState {
    /// What the folder holds now, in place of the map's size, which is out of date.
    pub(super) fn left(self) -> Option<u64> {
        match self {
            CleanState::Deleting { before, freed } => {
                before.map(|before| before.saturating_sub(freed))
            }
            CleanState::Deleted { left } => Some(left),
        }
    }
}

/// What the toolbar offers to clean: how many folders, and how much they hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IdleOffer {
    pub(super) folders: usize,
    pub(super) bytes: u64,
}

impl Cleaning {
    /// Cleaning that starts its threads with `threads`, keeping `keep_free` free as the user chose
    /// in an earlier run.
    pub(super) fn new(threads: Threads, keep_free: Option<u64>) -> Cleaning {
        Cleaning {
            threads,
            folders: HashMap::new(),
            preparing: HashMap::new(),
            questions: OpenQuestions::default(),
            preparing_idle: false,
            preparing_unasked: false,
            waiting: VecDeque::new(),
            running: Vec::new(),
            stopped: Vec::new(),
            alerts: Vec::new(),
            banners: Vec::new(),
            measure_again: Vec::new(),
            keeping_free: KeepingFree::with_floor(keep_free),
            when_idle: WhenIdle::default(),
        }
    }

    /// The free space Sutra keeps by itself, if the user chose one.
    pub(super) fn keep_free_floor(&self) -> Option<u64> {
        self.keeping_free.floor
    }

    /// Keep this much free from now on.
    pub(super) fn set_keep_free_floor(&mut self, floor: Option<u64>) {
        self.keeping_free = KeepingFree::with_floor(floor);
    }

    /// What is on offer for the toolbar's icon, at `now`: `None` when nothing is.
    pub(super) fn idle_offer(&self, map: &Snapshot, now: SystemTime) -> Option<IdleOffer> {
        let idle = clean::idle_folders(map, now, OFFER_QUIET_AT_LEAST);
        let idle = idle
            .iter()
            .filter(|folder| self.state_of(folder.dir).is_none());
        let offer = idle.fold(
            IdleOffer {
                folders: 0,
                bytes: 0,
            },
            |offer, folder| IdleOffer {
                folders: offer.folders + 1,
                bytes: offer.bytes + folder.bytes,
            },
        );
        (offer.folders > 0).then_some(offer)
    }

    /// Whether a click on the toolbar's clean is still being worked out, which takes seconds when
    /// many large folders are searched for locks.
    pub(super) fn is_preparing_idle(&self) -> bool {
        self.preparing_idle
    }

    /// Whether a clean is being worked out, waits or runs. A question waiting for the user is no
    /// work in progress: cleaning by itself goes on meanwhile, clear of the folders it asks about.
    fn is_busy(&self) -> bool {
        !self.running.is_empty()
            || !self.waiting.is_empty()
            || !self.preparing.is_empty()
            || self.preparing_idle
            || self.preparing_unasked
            || self.when_idle.preparing
    }

    /// The alerts waiting to be shown.
    pub(super) fn take_alerts(&mut self) -> Vec<Alert> {
        std::mem::take(&mut self.alerts)
    }

    /// The banners waiting to be sent.
    pub(super) fn take_banners(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.banners)
    }

    /// The folders waiting to be measured after a clean.
    pub(super) fn take_measure_again(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.measure_again)
    }

    /// Take what a cleaning thread found, as `map` shows things at `now`; `map` is `None` while no
    /// engine keeps one up to date.
    pub(super) fn receive(
        &mut self,
        report: Report,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        match report {
            Report::Prepared { clicked, prepared } => {
                // A thread that ended with no word has none for any of its folders.
                let mut each = prepared.map(Vec::into_iter);
                for folder in &clicked {
                    let prepared = each.as_mut().and_then(Iterator::next);
                    self.asking(folder, prepared, map, now, home);
                }
            }
            Report::IdlePrepared(found) => self.asking_idle(found),
            Report::UnaskedPrepared {
                clicked,
                project,
                prepared,
            } => self.clean_unasked(clicked, project, prepared, map, now, home),
            Report::WhenIdlePrepared(found) => self.when_idle_prepared(found, map, now, home),
            Report::Cleaned { target, outcome } => self.finished(&target, outcome, map, now, home),
        }
    }

    /// Forget each deleted folder the map has caught up with: gone from it, or measured since the
    /// delete.
    pub(super) fn settle(&mut self, snapshot: &Snapshot) {
        self.folders.retain(|path, stage| match stage {
            Stage::Deleting { .. } => true,
            Stage::Deleted { at, .. } => snapshot
                .artifact(path)
                .is_some_and(|dir| !measured_since(dir, *at)),
        });
    }

    /// Whether a clean is deleting now, which the card's bar shows moving.
    pub(super) fn is_deleting(&self) -> bool {
        !self.running.is_empty()
    }

    /// What a clean means for the card of `dir`, `None` for a folder not being cleaned or measured
    /// since. A folder one clean is deleting while another clean of it is worked out or asked about
    /// shows how far the delete has come.
    pub(super) fn state_of(&self, dir: &ArtifactDir) -> Option<CleanState> {
        let pending =
            self.preparing.contains_key(&dir.path) || self.questions.asks_about(&dir.path);
        match self.folders.get(&dir.path) {
            Some(Stage::Deleting { before }) => {
                let running = self
                    .running
                    .iter()
                    .find(|running| running.folders.contains(&dir.path));
                let freed = running.map_or(0, |running| running.progress.bytes_freed_in(&dir.path));
                Some(CleanState::Deleting {
                    before: *before,
                    freed,
                })
            }
            _ if pending => {
                let before = dir.measurement.usage().map(|usage| usage.bytes);
                Some(CleanState::Deleting { before, freed: 0 })
            }
            Some(Stage::Deleted { at, left }) if !measured_since(dir, *at) => {
                Some(CleanState::Deleted { left: *left })
            }
            Some(Stage::Deleted { .. }) | None => None,
        }
    }

    /// Whether a clean means anything for one of `project`'s build folders now (see `state_of`),
    /// which its card shows.
    pub(super) fn is_at_work_in(&self, project: &Project) -> bool {
        let mut folders = project.artifacts.iter();
        folders.any(|dir| self.state_of(dir).is_some())
    }
}

/// The question before Sutra starts keeping `floor` free, with `free` free now.
pub(super) fn keep_free_question(floor: u64, free: Option<u64>) -> Question {
    words::keep_free_question(floor, free, auto::QUIET_AT_LEAST)
}

/// Whether deleting one of two folders deletes some of the other: they are the same folder, or one
/// holds the other.
fn overlap(one: &Path, other: &Path) -> bool {
    one.starts_with(other) || other.starts_with(one)
}

fn measured_since(dir: &ArtifactDir, at: SystemTime) -> bool {
    dir.measurement
        .usage()
        .is_some_and(|usage| usage.measured_at >= at)
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::{self, Receiver};

    use super::*;
    use crate::dashboard::live::Event;
    use crate::map::Project;
    use crate::map::clean::{CleanPlan, Origin, Plan, PlannedFolder, Prepared, Progress};
    use crate::map::test_support::{at, cargo_target, project_at, snapshot_of, usage};
    use crate::map::{DirUsage, Measurement};

    /// A floor no volume reaches, so keeping it free always wants a clean.
    pub(in crate::dashboard) const ALWAYS_SHORT: Option<u64> = Some(u64::MAX);

    /// Cleaning keeping no free space, whose threads report to no one: a test that starts one has
    /// it do its work, and drops what it finds.
    pub(in crate::dashboard) fn cleaning() -> Cleaning {
        Cleaning::new(Threads::new(mpsc::channel().0), None)
    }

    /// Cleaning keeping no free space, and where its threads report.
    pub(in crate::dashboard) fn reporting() -> (Cleaning, Receiver<Event>) {
        let (events, inbox) = mpsc::channel();
        (Cleaning::new(Threads::new(events), None), inbox)
    }

    /// The next report a cleaning thread sends to `inbox`, waiting for it a while.
    pub(in crate::dashboard) fn next_report(inbox: &Receiver<Event>) -> Report {
        match inbox.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Event::Cleaning(report)) => report,
            Ok(_) => panic!("only cleaning threads report here"),
            Err(err) => panic!("no report came: {err}"),
        }
    }

    pub(super) fn target(measured_at: Option<SystemTime>) -> ArtifactDir {
        let measurement = measured_at.map_or(Measurement::Measuring, |measured_at| {
            Measurement::Measured(DirUsage {
                measured_at,
                ..usage(12_300_000_000)
            })
        });
        cargo_target("/code/app/target", measurement)
    }

    /// A map whose only project, `/code/app`, holds `artifacts`.
    pub(super) fn map_with(artifacts: Vec<ArtifactDir>) -> Arc<Snapshot> {
        Arc::new(snapshot_of(vec![Project {
            artifacts,
            ..project_at("/code/app")
        }]))
    }

    /// Cleaning that deleted `/code/app/target` at second 100, leaving 42 bytes.
    pub(in crate::dashboard) fn deleted() -> Cleaning {
        let mut cleaning = cleaning();
        cleaning.folders.insert(
            PathBuf::from("/code/app/target"),
            Stage::Deleted {
                at: at(100),
                left: 42,
            },
        );
        cleaning
    }

    /// Cleaning with one clean under way, started from `/code/app/target`, that deletes it and
    /// `/cache/app`. No thread is at work on them.
    pub(super) fn deleting_two() -> Cleaning {
        let mut cleaning = cleaning();
        let folders = vec![
            PathBuf::from("/code/app/target"),
            PathBuf::from("/cache/app"),
        ];
        for folder in &folders {
            let stage = Stage::Deleting {
                before: Some(1_000),
            };
            cleaning.folders.insert(folder.clone(), stage);
        }
        cleaning.running = vec![Running {
            clicked: PathBuf::from("/code/app/target"),
            folders,
            by: By::User,
            progress: Arc::default(),
        }];
        cleaning
    }

    #[test]
    fn a_folder_being_deleted_shows_as_such_whatever_the_map_says() {
        let mut cleaning = deleting_two();
        cleaning.settle(&map_with(vec![]));
        assert_eq!(
            cleaning.state_of(&target(Some(at(500)))),
            Some(CleanState::Deleting {
                before: Some(1_000),
                freed: 0
            })
        );
    }

    #[test]
    fn a_folder_being_deleted_shows_how_far_the_delete_has_come_while_a_question_asks_about_it() {
        let mut cleaning = deleting_two();
        let shared = Path::new("/cache/app");
        cleaning.asking_idle(Some(clean::IdleCleans {
            cleans: vec![idle_clean(&[shared])],
            left_out: Vec::new(),
        }));
        let dir = cargo_target(shared, crate::map::test_support::measured(5_000));
        assert_eq!(
            cleaning.state_of(&dir),
            Some(CleanState::Deleting {
                before: Some(1_000),
                freed: 0
            }),
            "the size the running clean started from, not the map's"
        );
    }

    #[test]
    fn a_folder_being_cleaned_shows_what_is_left_of_it_as_the_clean_goes() {
        let fixture = crate::map::test_support::TempDir::new("cleaning-left");
        fixture.write("app/Cargo.toml", "[package]\nname = \"p\"\n");
        fixture.write("app/target/debug/big", [1; 64 * 1024]);
        let folder = fixture.path("app/target");
        let checked = clean::plan(
            &folder,
            clean::ItsProject::At(&fixture.path("app")),
            &fixture.cargo_env(),
            &clean::Reach::Anywhere,
        )
        .unwrap();
        let started_from = usage(1_000_000);
        let prepared = Prepared::for_test(
            checked.plan.clone(),
            vec![Some(started_from)],
            clean::Sharing::default(),
        );
        let dir = cargo_target(folder.clone(), Measurement::Measured(started_from));
        let mut cleaning = cleaning();
        cleaning.queue(folder.clone(), prepared, By::User);
        let left = |cleaning: &Cleaning| cleaning.state_of(&dir).and_then(CleanState::left);
        assert_eq!(left(&cleaning), Some(1_000_000), "waiting its turn");

        let progress = Arc::new(Progress::default());
        cleaning.running = vec![Running {
            clicked: folder.clone(),
            folders: vec![folder.clone()],
            by: By::User,
            progress: Arc::clone(&progress),
        }];
        checked.lock().unwrap().delete(&progress).unwrap();

        let freed = progress.bytes_freed_in(&folder);
        assert!(freed >= 64 * 1024);
        assert_eq!(left(&cleaning), Some(1_000_000 - freed));
    }

    #[test]
    fn a_deleted_folder_shows_what_the_clean_left_until_the_map_catches_up() {
        let mut cleaning = deleted();
        let stale = target(Some(at(50)));
        cleaning.settle(&map_with(vec![stale.clone()]));
        assert_eq!(
            cleaning.state_of(&stale),
            Some(CleanState::Deleted { left: 42 })
        );

        cleaning.settle(&map_with(vec![]));
        assert!(
            cleaning.folders.is_empty(),
            "cargo clean took it off the map"
        );
    }

    #[test]
    fn a_deleted_folder_a_build_filled_again_shows_its_new_size() {
        let mut cleaning = deleted();
        let refilled = target(Some(at(150)));
        assert_eq!(cleaning.state_of(&refilled), None);
        cleaning.settle(&map_with(vec![refilled]));
        assert!(cleaning.folders.is_empty());
    }

    #[test]
    fn a_folder_not_being_cleaned_shows_its_size() {
        assert_eq!(cleaning().state_of(&target(None)), None);
    }

    /// A clean of `folder`. Starting it runs a real delete, so a test that starts it has `folder`
    /// in a fixture of its own.
    pub(in crate::dashboard) fn prepared_at(folder: &Path) -> Prepared {
        prepared_of(&[folder])
    }

    /// A clean of `folders`, as a target-dir and a build-dir set elsewhere make one. Starting it
    /// runs a real delete, so a test that starts it has each folder in a fixture of its own.
    pub(in crate::dashboard) fn prepared_of(folders: &[&Path]) -> Prepared {
        let folders = folders
            .iter()
            .map(|folder| PlannedFolder {
                path: folder.to_path_buf(),
                origin: Origin::Default,
            })
            .collect::<Vec<_>>();
        let sizes = vec![None; folders.len()];
        Prepared::for_test(
            Plan::CargoClean(CleanPlan {
                workspace: PathBuf::from("/code/app"),
                folders,
            }),
            sizes,
            clean::Sharing::default(),
        )
    }

    /// The clean of `folders` as the toolbar's search finds one, started from the first of them in
    /// the project holding it.
    pub(in crate::dashboard) fn idle_clean(folders: &[&Path]) -> clean::IdleClean {
        let clicked = folders[0];
        let project = clicked
            .parent()
            .expect("a build folder lies in its project");
        clean::IdleClean {
            clicked: clicked.to_path_buf(),
            project_name: project_at(project).name,
            project_path: project.to_path_buf(),
            quiet_at_least: OFFER_QUIET_AT_LEAST,
            prepared: prepared_of(folders),
        }
    }

    pub(super) fn prepared_clean(folder: &str) -> Prepared {
        prepared_at(Path::new(folder))
    }

    #[test]
    fn a_folder_queued_shows_as_being_cleaned_and_keeps_the_offer_from_counting_it() {
        let mut cleaning = cleaning();
        let dir = cargo_target(
            "/code/app/target",
            crate::map::test_support::measured(1 << 30),
        );
        let map = snapshot_of(vec![Project {
            artifacts: vec![dir.clone()],
            ..project_at("/code/app")
        }]);
        let offered = cleaning.idle_offer(&map, at(1_000));
        assert_eq!(
            offered,
            Some(IdleOffer {
                folders: 1,
                bytes: 1 << 30
            })
        );

        cleaning.queue(
            dir.path.clone(),
            prepared_clean("/code/app/target"),
            By::User,
        );

        assert_eq!(
            cleaning.state_of(&dir),
            Some(CleanState::Deleting {
                before: None,
                freed: 0
            })
        );
        assert_eq!(cleaning.idle_offer(&map, at(1_000)), None);
    }
}
