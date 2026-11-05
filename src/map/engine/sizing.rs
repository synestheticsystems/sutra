//! Measuring sizes off the engine thread. A large `target/` takes seconds to walk, so the engine
//! queues measurements here and keeps routing events; each result comes back as a message. The
//! engine decides the order: a build folder a build just changed goes ahead of the launch
//! backlog, build folders go ahead of whole project folders, and nothing is measured twice at
//! once. A tree that keeps changing rests between walks for as long as its size says, so a build
//! that runs for minutes has its `target/` walked about once a minute, not at every pause.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::map::DirUsage;
use crate::map::disk::{self, TopStamp};

/// Measurements running at once. Each is itself parallel across the pool, so two keep the pool
/// busy while letting a small directory finish beside a large one.
const MAX_RUNNING: usize = 2;

/// Threads of the sizing pool. Like discovery, sizing is metadata I/O, which contends in the
/// kernel past a few threads (see `scan::DISCOVERY_THREADS`): a first sizing of Daniel's home
/// (5.9 million files, 2026-11-04) cost 175 CPU-seconds on 8 threads and 110 on 4, for 18 s more.
const SIZING_THREADS: usize = 4;

/// How long a tree rests after a walk before a change has it walked again, per file the walk
/// counted. A walk costs about a CPU-second per 100,000 files (Daniel's M4 Max, 2026-11-04), so
/// a 100,000-file `target/` is walked about once a minute while its build runs, and a folder of
/// a few thousand files as soon as its debounce allows.
const REST_PER_FILE: Duration = Duration::from_micros(600);

/// What one measurement sizes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::map::engine) enum Job {
    /// A build folder, walked whole.
    Artifact(PathBuf),
    /// A project folder's own files: the folder walked without its build folders and the
    /// projects nested inside it, which are measured on their own.
    OwnFiles(PathBuf),
}

impl Job {
    /// The folder the walk starts at.
    pub(in crate::map::engine) fn root(&self) -> &Path {
        match self {
            Job::Artifact(path) | Job::OwnFiles(path) => path,
        }
    }
}

/// How soon a requested measurement should run, least urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Urgency {
    /// The first measurement.
    Backlog,
    /// What it measures just changed.
    Changed,
    /// A clean just deleted from it. The rest after its last walk went by the files it held then,
    /// and the card is waiting to show what is left, so this does not wait for that rest to end.
    AfterClean,
}

/// One finished measurement.
pub(super) struct Measured {
    pub(super) job: Job,
    pub(super) usage: io::Result<DirUsage>,
    pub(super) began: Instant,
    /// The stamp of the tree's top when the walk began, `None` when it could not be read.
    pub(super) stamp: Option<TopStamp>,
}

/// Receives each finished measurement, on a pool thread.
type Report = Arc<dyn Fn(Measured) + Send + Sync>;

/// What a tree's last walk tells about the next one.
#[derive(Debug, Clone, Copy)]
struct LastWalk {
    began: Instant,
    /// When its result reached the engine.
    ended: Instant,
    /// The files it counted, none when it failed.
    files: u64,
    /// The stamp of the tree's top when it began, `None` when it could not be read.
    stamp: Option<TopStamp>,
}

impl LastWalk {
    /// When a change may have the tree walked again.
    fn rest_ends(&self) -> Instant {
        let files = u32::try_from(self.files).unwrap_or(u32::MAX);
        self.ended + REST_PER_FILE.saturating_mul(files)
    }
}

pub(super) struct Sizing {
    /// `None` when the pool could not be built: jobs then share rayon's global pool.
    pool: Option<rayon::ThreadPool>,
    report: Report,
    /// Build folders waiting, those a build just changed first.
    artifacts: VecDeque<PathBuf>,
    /// Project folders waiting. One runs only when no build folder waits, and never two at once,
    /// so a walk of a large project never holds up the size of a folder a build just changed.
    folders: VecDeque<PathBuf>,
    queued: HashSet<Job>,
    running: HashSet<Job>,
    /// Running measurements that a later change made stale; each is asked for again when it
    /// reports, this urgently.
    rerun: HashMap<Job, Urgency>,
    /// The last walk of each tree on the map.
    last_walks: HashMap<Job, LastWalk>,
    /// Changes that came while their tree rested, and when each rest ends.
    resting: HashMap<Job, Instant>,
}

impl Sizing {
    pub(super) fn new(report: impl Fn(Measured) + Send + Sync + 'static) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(SIZING_THREADS)
            .thread_name(|index| format!("sutra-size-{index}"))
            .build()
            .ok();
        Sizing {
            pool,
            report: Arc::new(report),
            artifacts: VecDeque::new(),
            folders: VecDeque::new(),
            queued: HashSet::new(),
            running: HashSet::new(),
            rerun: HashMap::new(),
            last_walks: HashMap::new(),
            resting: HashMap::new(),
        }
    }

    /// Ask at `now` for `job` to run. One already queued keeps one place in its queue (moved to
    /// the front when what it measures changed); one running runs again afterwards; and a change
    /// to a tree that rests after its last walk waits for the rest to end. A first measurement
    /// never waits: the tree has no size to show meanwhile.
    pub(super) fn request(&mut self, job: &Job, urgency: Urgency, now: Instant) {
        if self.running.contains(job) {
            let again = self.rerun.entry(job.clone()).or_insert(Urgency::Changed);
            *again = urgency.max(*again);
            return;
        }
        if urgency == Urgency::Changed && !self.queued.contains(job) {
            let rest_ends = self.last_walks.get(job).map(LastWalk::rest_ends);
            if let Some(rest_ends) = rest_ends.filter(|rest_ends| *rest_ends > now) {
                self.resting.insert(job.clone(), rest_ends);
                return;
            }
        }
        self.enqueue(job, urgency);
    }

    fn enqueue(&mut self, job: &Job, urgency: Urgency) {
        let (queue, path) = match job {
            Job::Artifact(path) => (&mut self.artifacts, path),
            Job::OwnFiles(path) => (&mut self.folders, path),
        };
        if self.queued.contains(job) {
            if urgency > Urgency::Backlog {
                queue.retain(|queued| queued != path);
                queue.push_front(path.clone());
            }
            return;
        }
        self.queued.insert(job.clone());
        match urgency {
            Urgency::Changed | Urgency::AfterClean => queue.push_front(path.clone()),
            Urgency::Backlog => queue.push_back(path.clone()),
        }
    }

    /// Record that the walk `measured` reports reached the engine at `now`. Returns whether its
    /// tree was asked for again, because what it measures changed while it ran.
    pub(super) fn finished(&mut self, measured: &Measured, now: Instant) -> bool {
        let job = &measured.job;
        self.running.remove(job);
        let walk = LastWalk {
            began: measured.began,
            ended: now,
            files: measured.usage.as_ref().map_or(0, |usage| usage.files),
            stamp: measured.stamp,
        };
        self.last_walks.insert(job.clone(), walk);
        let again = self.rerun.remove(job);
        if let Some(urgency) = again {
            self.request(job, urgency, now);
        }
        again.is_some()
    }

    /// Whether the top of `job`'s tree changed since its last walk began, judged by its stamp
    /// now, which leaves out what the walk leaves out (`left_out` names that for a project). A
    /// tree not walked yet counts as unchanged: its first walk is queued or running. A stamp
    /// that cannot be read, then or now, counts as a change, so the walk that follows reports
    /// what happened to the tree.
    pub(super) fn top_changed(
        &self,
        job: &Job,
        left_out: impl FnOnce(&Path) -> HashSet<PathBuf>,
    ) -> bool {
        let Some(last) = self.last_walks.get(job) else {
            return false;
        };
        let skipped = skipped(job, left_out);
        let now = disk::top_stamp(job.root(), &|dir| skipped.contains(dir));
        !matches!((last.stamp, now), (Some(then), Ok(now)) if then == now)
    }

    /// Whether the last walk of `job`'s tree began at `when` or later, so it saw every change made
    /// before `when`. One that began earlier may have passed a folder before a change there,
    /// however late it ended.
    pub(super) fn walk_began_since(&self, job: &Job, when: Instant) -> bool {
        self.last_walks
            .get(job)
            .is_some_and(|walk| walk.began >= when)
    }

    /// Queue every change whose tree's rest has ended by `now`. Returns whether any was queued.
    pub(super) fn release_rested(&mut self, now: Instant) -> bool {
        let rested: Vec<Job> = self
            .resting
            .iter()
            .filter(|(_, rest_ends)| **rest_ends <= now)
            .map(|(job, _)| job.clone())
            .collect();
        for job in &rested {
            self.resting.remove(job);
            self.enqueue(job, Urgency::Changed);
        }
        !rested.is_empty()
    }

    /// When the next resting tree's rest ends, if any rests.
    pub(super) fn next_release(&self) -> Option<Instant> {
        self.resting.values().min().copied()
    }

    /// Forget the trees `on_map` rejects, which left the map, so one that comes back is measured
    /// as a tree never walked.
    pub(super) fn forget_unless(&mut self, on_map: impl Fn(&Job) -> bool) {
        self.last_walks.retain(|job, _| on_map(job));
        self.resting.retain(|job, _| on_map(job));
    }

    /// Start queued measurements up to the running limit. `left_out` names, for a project folder,
    /// the folders inside it that its walk skips, read when the walk starts so it sees the map as
    /// it is then.
    pub(super) fn dispatch(&mut self, left_out: impl Fn(&Path) -> HashSet<PathBuf>) {
        while let Some(job) = self.next() {
            let report = Arc::clone(&self.report);
            let skipped = skipped(&job, &left_out);
            let run = move || {
                let pruned = |dir: &Path| skipped.contains(dir);
                let began = Instant::now();
                // Read first, so a change made during the walk differs from it at the next check.
                // A top that cannot be read is no reason to skip the walk, which reports why.
                let stamp = disk::top_stamp(job.root(), &pruned).ok();
                let usage = disk::measure_pruned(job.root(), &pruned);
                report(Measured {
                    job,
                    usage,
                    began,
                    stamp,
                });
            };
            match &self.pool {
                Some(pool) => pool.spawn(run),
                None => rayon::spawn(run),
            }
        }
    }

    /// The next job to run, moved from its queue to the running set, or `None` when nothing may
    /// start yet.
    fn next(&mut self) -> Option<Job> {
        if self.running.len() >= MAX_RUNNING {
            return None;
        }
        let folder_running = self
            .running
            .iter()
            .any(|job| matches!(job, Job::OwnFiles(_)));
        let job = match self.artifacts.pop_front() {
            Some(artifact) => Job::Artifact(artifact),
            None if !folder_running => Job::OwnFiles(self.folders.pop_front()?),
            None => return None,
        };
        self.queued.remove(&job);
        // This walk serves a change that rests for the tree too.
        self.resting.remove(&job);
        self.running.insert(job.clone());
        Some(job)
    }
}

/// What the walk of `job` leaves out: nothing of a build folder, and what `left_out` names for a
/// project. Its stamp leaves out the same, so a change only there is no change to the tree.
fn skipped(job: &Job, left_out: impl FnOnce(&Path) -> HashSet<PathBuf>) -> HashSet<PathBuf> {
    match job {
        Job::Artifact(_) => HashSet::new(),
        Job::OwnFiles(project) => left_out(project),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{TempDir, usage};

    fn target(name: &str) -> Job {
        Job::Artifact(PathBuf::from(format!("/code/{name}/target")))
    }

    fn folder(name: &str) -> Job {
        Job::OwnFiles(PathBuf::from(format!("/code/{name}")))
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// What a walk of `job` reports when it counted `files` and began just now with `stamp`.
    fn walked(job: &Job, files: u64, stamp: Option<TopStamp>) -> Measured {
        Measured {
            job: job.clone(),
            usage: Ok(DirUsage {
                files,
                ..usage(files * 4096)
            }),
            began: Instant::now(),
            stamp,
        }
    }

    /// Walk `job` once, from `start` until it reports at `end`, having counted `files`.
    fn walk(sizing: &mut Sizing, job: &Job, start: Instant, end: Instant, files: u64) {
        sizing.request(job, Urgency::Backlog, start);
        assert_eq!(sizing.next().as_ref(), Some(job));
        let measured = Measured {
            began: start,
            ..walked(job, files, None)
        };
        assert!(!sizing.finished(&measured, end));
    }

    #[test]
    fn a_changed_directory_runs_before_the_backlog() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        sizing.request(&target("a"), Urgency::Backlog, t0);
        sizing.request(&target("b"), Urgency::Backlog, t0);
        sizing.request(&target("c"), Urgency::Changed, t0);
        sizing.request(&target("b"), Urgency::Changed, t0);
        assert_eq!(sizing.next(), Some(target("b")));
        assert_eq!(sizing.next(), Some(target("c")));
    }

    #[test]
    fn no_more_than_the_limit_run_at_once() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        for name in ["a", "b", "c"] {
            sizing.request(&target(name), Urgency::Backlog, t0);
        }
        assert_eq!(sizing.next(), Some(target("a")));
        assert_eq!(sizing.next(), Some(target("b")));
        assert_eq!(sizing.next(), None);
        assert!(!sizing.finished(&walked(&target("a"), 1, None), t0));
        assert_eq!(sizing.next(), Some(target("c")));
    }

    #[test]
    fn a_job_queued_twice_runs_once() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        sizing.request(&target("a"), Urgency::Backlog, t0);
        sizing.request(&target("a"), Urgency::Backlog, t0);
        assert_eq!(sizing.next(), Some(target("a")));
        assert_eq!(sizing.next(), None);
    }

    #[test]
    fn project_folders_wait_for_every_build_folder_queued_before_or_after_them() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        sizing.request(&folder("a"), Urgency::Changed, t0);
        sizing.request(&target("a"), Urgency::Backlog, t0);
        assert_eq!(sizing.next(), Some(target("a")));
        sizing.request(&target("b"), Urgency::Backlog, t0);
        assert_eq!(sizing.next(), Some(target("b")));
        assert!(!sizing.finished(&walked(&target("a"), 1, None), t0));
        assert_eq!(sizing.next(), Some(folder("a")));
    }

    #[test]
    fn one_project_folder_runs_at_a_time_leaving_a_slot_for_build_folders() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        sizing.request(&folder("a"), Urgency::Backlog, t0);
        sizing.request(&folder("b"), Urgency::Backlog, t0);
        assert_eq!(sizing.next(), Some(folder("a")));
        assert_eq!(sizing.next(), None, "the second folder waits");
        sizing.request(&target("c"), Urgency::Changed, t0);
        assert_eq!(sizing.next(), Some(target("c")));
        assert!(!sizing.finished(&walked(&folder("a"), 1, None), t0));
        assert!(!sizing.finished(&walked(&target("c"), 1, None), t0));
        assert_eq!(sizing.next(), Some(folder("b")));
    }

    #[test]
    fn a_tree_that_changes_while_walked_is_walked_again_once_its_rest_ends() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        sizing.request(&target("a"), Urgency::Backlog, t0);
        assert_eq!(sizing.next(), Some(target("a")));
        sizing.request(&target("a"), Urgency::Changed, t0 + secs(1));
        assert_eq!(sizing.next(), None, "not twice at once");

        assert!(sizing.finished(&walked(&target("a"), 100_000, None), t0 + secs(2)));
        assert_eq!(sizing.next(), None, "a 100,000-file walk rests a minute");
        assert_eq!(sizing.next_release(), Some(t0 + secs(62)));
        assert!(!sizing.release_rested(t0 + secs(61)));
        assert!(sizing.release_rested(t0 + secs(62)));
        assert_eq!(sizing.next(), Some(target("a")));
        assert_eq!(sizing.next_release(), None);
    }

    #[test]
    fn a_tree_rests_in_proportion_to_its_files_and_a_first_measurement_never_rests() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        walk(&mut sizing, &target("big"), t0, t0 + secs(1), 100_000);
        walk(&mut sizing, &target("small"), t0, t0 + secs(1), 1_000);

        sizing.request(&target("big"), Urgency::Changed, t0 + secs(5));
        sizing.request(&target("small"), Urgency::Changed, t0 + secs(5));
        assert_eq!(sizing.next(), Some(target("small")), "its 0.6 s rest ended");
        assert_eq!(
            sizing.next(),
            None,
            "the big one rests until a minute after its walk"
        );
        assert_eq!(sizing.next_release(), Some(t0 + secs(61)));

        sizing.request(&target("big"), Urgency::Backlog, t0 + secs(5));
        assert_eq!(
            sizing.next(),
            Some(target("big")),
            "measured again from the backlog, as after its size was lost"
        );
        assert_eq!(
            sizing.next_release(),
            None,
            "that walk serves the resting change"
        );
    }

    #[test]
    fn a_tree_a_clean_just_deleted_from_is_walked_at_once_whatever_its_last_walk_counted() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        walk(&mut sizing, &target("a"), t0, t0 + secs(1), 1_000_000);
        sizing.request(&target("a"), Urgency::Changed, t0 + secs(2));
        assert_eq!(sizing.next(), None, "a million files rest ten minutes");

        sizing.request(&target("a"), Urgency::AfterClean, t0 + secs(3));
        assert_eq!(sizing.next(), Some(target("a")));
        assert_eq!(
            sizing.next_release(),
            None,
            "that walk serves the resting change"
        );

        // A walk running as the clean ends saw files that are gone, and a change after that does
        // not make the next walk wait for the rest this one earns.
        sizing.request(&target("a"), Urgency::AfterClean, t0 + secs(4));
        sizing.request(&target("a"), Urgency::Changed, t0 + secs(4));
        assert!(sizing.finished(&walked(&target("a"), 500_000, None), t0 + secs(5)));
        assert_eq!(sizing.next(), Some(target("a")));
    }

    #[test]
    fn a_tree_that_left_the_map_is_forgotten() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        walk(&mut sizing, &target("a"), t0, t0 + secs(1), 100_000);
        sizing.request(&target("a"), Urgency::Changed, t0 + secs(2));
        sizing.forget_unless(|_| false);
        assert_eq!(
            sizing.next_release(),
            None,
            "nothing rests for a tree that is gone"
        );
        sizing.request(&target("a"), Urgency::Changed, t0 + secs(3));
        assert_eq!(
            sizing.next(),
            Some(target("a")),
            "back on the map, it is a tree never walked"
        );
    }

    #[test]
    fn only_a_walk_that_began_after_a_moment_counts_as_seeing_every_change_before_it() {
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        assert!(!sizing.walk_began_since(&target("a"), t0), "never walked");

        walk(&mut sizing, &target("a"), t0, t0 + secs(10), 1);
        assert!(sizing.walk_began_since(&target("a"), t0));
        assert!(
            !sizing.walk_began_since(&target("a"), t0 + secs(5)),
            "it may have passed a folder before a change at that moment, though it ended later"
        );
    }

    #[test]
    fn a_tree_counts_as_changed_when_its_top_differs_from_when_its_last_walk_began() {
        let fixture = TempDir::new("sizing-top-changed");
        fixture.write("src/lib.rs", "");
        fixture.write("target/debug/app", "");
        let target = fixture.path("target");
        let left_out = |_: &Path| HashSet::from([target.clone()]);
        let job = Job::OwnFiles(fixture.root().to_path_buf());
        let t0 = Instant::now();
        let mut sizing = Sizing::new(|_| {});
        assert!(
            !sizing.top_changed(&job, left_out),
            "never walked: its first walk is queued"
        );

        let stamp = disk::top_stamp(fixture.root(), &|dir| dir == target).ok();
        walk(&mut sizing, &job, t0, t0, 1);
        sizing.finished(&walked(&job, 1, stamp), t0);
        assert!(!sizing.top_changed(&job, left_out), "nothing changed");

        fixture.write("target/new", "");
        assert!(
            !sizing.top_changed(&job, left_out),
            "only what its walk leaves out changed"
        );
        fixture.write("README.md", "");
        assert!(sizing.top_changed(&job, left_out), "a new file at the top");

        sizing.finished(&walked(&job, 1, None), t0);
        assert!(
            sizing.top_changed(&job, left_out),
            "its top could not be read when the walk began"
        );

        let gone = Job::Artifact(fixture.path("gone"));
        walk(&mut sizing, &gone, t0, t0, 1);
        sizing.finished(&walked(&gone, 1, stamp), t0);
        assert!(
            sizing.top_changed(&gone, left_out),
            "its top cannot be read now"
        );
    }
}
