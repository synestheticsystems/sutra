//! The engine thread: it owns the map, turns file events and finished measurements into
//! refreshes, and publishes a snapshot at most every `PUBLISH_INTERVAL` while anything changed.

mod worked;

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::time::{Duration, Instant, SystemTime};

use notify::event::{CreateKind, ModifyKind, RemoveKind};
use notify::{Event, EventKind};

use self::worked::worked_at;
use super::probe::{self, Probe};
use super::route::Route;
use super::sizing::{Job, Measured, Sizing, Urgency};
use super::state::State;
use super::timers::{TICK, Timers, WALK_AFTER_LOST_EVENTS};
use super::watch::Watches;
use crate::map::grant::Roots;
use crate::map::scan::MacOsAnswer;
use crate::map::{PrivateFolder, ScanOptions, Snapshot, Source};

const PUBLISH_INTERVAL: Duration = Duration::from_millis(250);

/// How long the first walk waits for macOS to answer about the private folders it would enter.
/// macOS answers at once for a folder the user answered about on an earlier launch. On 2026-11-06
/// a new process's first listing of a folder macOS guards, refused with no prompt, took 3 to 10 ms
/// in 23 runs and 31 ms once, and tccd answered requests it had answered before in 10 to 35 ms. So
/// a folder answered on an earlier launch goes into the first walk, and needs no second. While a
/// prompt waits for the user, the first map goes out this much later, without its folder.
const WAIT_FOR_MACOS: Duration = Duration::from_millis(100);

/// What reaches the engine thread.
pub(super) enum Message {
    Fs(notify::Result<Event>),
    Measured(Measured),
    /// A clean just deleted from these folders.
    Cleaned(Vec<PathBuf>),
    /// macOS answered whether Sutra may read this private folder (see `probe`).
    MacOsAnswered(PrivateFolder, MacOsAnswer),
    /// The dashboard is about to start another engine, or is stopping, so this one ends.
    Stop,
}

/// Run the engine until it is told to stop or the receiving end of `snapshots` is dropped.
/// `messages` feeds `inbox`, for the watcher, the sizing pool and the threads that ask macOS about
/// the private folders, listing each with `probe`, to report on.
pub(super) fn run(
    roots: &Roots,
    opts: &ScanOptions,
    probe: &Probe,
    snapshots: &SyncSender<Arc<Snapshot>>,
    messages: Sender<Message>,
    inbox: &Receiver<Message>,
) {
    let mut state = State::new(roots, opts);
    if state.roots().is_empty() {
        // Nothing to scan or watch, so this one snapshot is the last; a dropped receiver has
        // nothing left to miss.
        let _ = snapshots.send(Arc::new(state.snapshot()));
        return;
    }
    let asked = state.awaiting_macos();
    let unanswered = asked.len();
    probe::ask(asked, probe, &messages);
    if wait_for_macos(&mut state, unanswered, inbox).is_break() {
        return;
    }
    // Watching starts before the scan, so a change made while it runs is not missed.
    let watches = Watches::over_roots(&state.watched_roots(), messages.clone());
    state.report_problems(&[Source::Watcher], watches.problems());
    state.rediscover(None);
    let mut engine = Engine::new(state, watches, messages, snapshots);
    engine.queue_unmeasured(Instant::now());
    engine.dispatch();
    engine.run(inbox);
}

/// Wait up to `WAIT_FOR_MACOS` for macOS to answer about the `unanswered` private folders `state`
/// keeps out until it does, so the first walk enters each one answered by then. `Break` when the
/// engine is told to stop meanwhile.
fn wait_for_macos(
    state: &mut State,
    mut unanswered: usize,
    inbox: &Receiver<Message>,
) -> ControlFlow<()> {
    let deadline = Instant::now() + WAIT_FOR_MACOS;
    while unanswered > 0 {
        match inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Message::MacOsAnswered(folder, answer)) => {
                state.macos_answered(folder, answer);
                unanswered -= 1;
            }
            Ok(Message::Stop) => return ControlFlow::Break(()),
            // The first walk measures every folder a clean left. Nothing else runs yet to send.
            Ok(Message::Cleaned(_) | Message::Fs(_) | Message::Measured(_)) => {}
            // `run` holds a sender, so the inbox stays open until the wait is up.
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    ControlFlow::Continue(())
}

/// File events went missing, and not every tree has been walked since.
#[derive(Debug, Clone, Copy)]
struct LostEvents {
    first: Instant,
    latest: Instant,
}

struct Engine<'a> {
    state: State,
    watches: Watches,
    snapshots: &'a SyncSender<Arc<Snapshot>>,
    sizing: Sizing,
    timers: Timers,
    lost_events: Option<LostEvents>,
    /// A change not yet published.
    dirty: bool,
    /// `None` until the first snapshot is offered, which goes out at once.
    last_publish_attempt: Option<Instant>,
}

impl<'a> Engine<'a> {
    fn new(
        state: State,
        watches: Watches,
        messages: Sender<Message>,
        snapshots: &'a SyncSender<Arc<Snapshot>>,
    ) -> Self {
        let sizing = Sizing::new(move |measured| {
            // Fails only while the engine shuts down, when the result has no map to land in.
            let _ = messages.send(Message::Measured(measured));
        });
        let now = Instant::now();
        let mut engine = Engine {
            state,
            watches,
            snapshots,
            sizing,
            timers: Timers::new(now),
            lost_events: None,
            dirty: true,
            last_publish_attempt: None,
        };
        engine.keep_watches_current(now);
        engine
    }

    fn run(&mut self, inbox: &Receiver<Message>) {
        loop {
            let now = Instant::now();
            if !self.publish_if_due(now) {
                return;
            }
            let wake = self.next_wake(now);
            match inbox.recv_timeout(wake.saturating_duration_since(now)) {
                Ok(Message::Fs(event)) => self.on_fs_event(event),
                Ok(Message::Measured(measured)) => self.on_measured(measured),
                Ok(Message::Cleaned(folders)) => self.on_cleaned(folders, Instant::now()),
                Ok(Message::MacOsAnswered(folder, answer)) => {
                    self.on_macos_answer(folder, answer, Instant::now());
                }
                Ok(Message::Stop) => return,
                Err(RecvTimeoutError::Timeout) => {}
                // The sizing pool holds a sender for as long as the engine runs.
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.run_due(Instant::now());
        }
    }

    /// When the next snapshot may go out: at once for the first, then `PUBLISH_INTERVAL` after
    /// the last attempt. `None` while nothing changed.
    fn publish_due_at(&self, now: Instant) -> Option<Instant> {
        if !self.dirty {
            return None;
        }
        Some(
            self.last_publish_attempt
                .map_or(now, |attempt| attempt + PUBLISH_INTERVAL),
        )
    }

    /// The earliest of the next timer, the next snapshot and the end of the next tree's rest.
    fn next_wake(&self, now: Instant) -> Instant {
        [self.publish_due_at(now), self.sizing.next_release()]
            .into_iter()
            .flatten()
            .fold(self.timers.next_due(), Instant::min)
    }

    /// Returns `false` once no one receives snapshots any more.
    fn publish_if_due(&mut self, now: Instant) -> bool {
        if self.publish_due_at(now).is_none_or(|due| now < due) {
            return true;
        }
        self.last_publish_attempt = Some(now);
        match self.snapshots.try_send(Arc::new(self.state.snapshot())) {
            Ok(()) => self.dirty = false,
            // The consumer has not taken the last snapshot yet; stay dirty and offer a fresher
            // one at the next interval.
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return false,
        }
        true
    }

    fn on_fs_event(&mut self, event: notify::Result<Event>) {
        let now = Instant::now();
        let event = match event {
            Ok(event) => event,
            // The watcher lost track of the file system: the remedy is the one for dropped events.
            Err(err) => {
                let lost = if err.paths.is_empty() {
                    self.watches.roots().to_vec()
                } else {
                    err.paths
                };
                for path in lost {
                    self.timers.rescan.mark(path, now);
                }
                return;
            }
        };
        if event.need_rescan() {
            for path in event.paths {
                self.timers.rescan.mark(path, now);
            }
            return;
        }
        let folder_appeared_or_vanished = matches!(
            event.kind,
            EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder)
        );
        // FSEvents reports a rename at each end, with nothing to tell a file from a folder.
        let renamed = matches!(event.kind, EventKind::Modify(ModifyKind::Name(_)));
        for path in &event.paths {
            // FSEvents names the folder that moved and nothing in it, and a move out of the roots
            // has no other end to report, so this alone tells that the projects inside went too.
            if renamed && self.state.holds_projects(path) {
                self.timers.rediscover.mark((), now);
            }
            match self.state.index().route(path) {
                Route::Ignore => {}
                Route::ClaudeSessions => self.timers.claude_sessions.mark((), now),
                Route::Codex => self.timers.codex.mark((), now),
                Route::Environments => self.timers.environments.mark((), now),
                Route::Git(worktrees) => {
                    for worktree in worktrees {
                        self.timers.git.mark(worktree, now);
                    }
                }
                Route::Rediscover => self.timers.rediscover.mark((), now),
                Route::ScratchFolders(session) => {
                    // A file written at the top of the scratch, or in a folder not on the map
                    // yet, is no folder coming or going.
                    if folder_appeared_or_vanished || (renamed && !path.is_file()) {
                        self.timers.scratch_folders.mark(session, now);
                    }
                }
                Route::Artifact(artifact) => self.timers.artifacts.mark(artifact, now),
                Route::FileChange(project) => {
                    let readable = self.state.is_readable(&project);
                    if let Some(at) = worked_at(&event.kind, path, readable, SystemTime::now()) {
                        self.dirty |= self.state.touch(&project, at);
                    }
                    // A folder moved in or out may be or hold a build folder. Saving a file also
                    // renames, so a path now holding a file is no reason to look. Looking in a
                    // private folder would raise the privacy prompt.
                    let moved_folder = renamed && readable && !path.is_file();
                    if folder_appeared_or_vanished || moved_folder {
                        self.layout_changed(project.clone(), now);
                    }
                    self.timers.own_files.mark(project, now);
                }
                Route::OutsideProjects => {
                    let moved_folder = renamed
                        && self.state.may_read(path)
                        && fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
                    if moved_folder {
                        self.timers.moved_in.mark((), now);
                    }
                }
            }
        }
    }

    /// Measure at once each of `folders` on the map, so its card shows what a clean left. One off
    /// the map, such as a build folder outside every project, has no card to show it.
    fn on_cleaned(&mut self, folders: Vec<PathBuf>, now: Instant) {
        for folder in folders {
            if self.state.is_build_folder(&folder) {
                self.request_measurement(&Job::Artifact(folder), Urgency::AfterClean, now);
            }
        }
        self.dispatch();
    }

    /// macOS answered, after the first walk, whether Sutra may read the private folder `folder`.
    /// One it let in is entered by a rediscovery, which then finds, sizes and watches what it
    /// holds. The projects in one it refused say so at once.
    fn on_macos_answer(&mut self, folder: PrivateFolder, answer: MacOsAnswer, now: Instant) {
        self.state.macos_answered(folder, answer);
        match answer {
            MacOsAnswer::LetIn => {
                // The stream over the roots began before macOS let Sutra in, and FSEvents may
                // not tell a stream about a folder its process could not read when it began. That
                // is unverified, since testing it means touching a real private folder, so the
                // stream begins again, now that Sutra may. The rediscovery this answer starts reads
                // again whatever the restart left undelivered.
                self.watches.restart_over_roots(&self.state.watched_roots());
                self.dirty |= self
                    .state
                    .report_problems(&[Source::Watcher], self.watches.problems());
                self.timers.rediscover.mark((), now);
            }
            MacOsAnswer::Refused => self.dirty = true,
        }
    }

    /// A folder appeared, vanished or moved in `project`, so look for its build folders again once
    /// its folders settle: soon in a worktree, which is searched two levels down, and later in a
    /// scratch folder, which is searched through (see `timers::SCRATCH_LAYOUT`).
    fn layout_changed(&mut self, project: PathBuf, now: Instant) {
        if self.state.is_scratch_folder(&project) {
            self.timers.scratch_layout.mark(project, now);
        } else {
            self.timers.layout.mark(project, now);
        }
    }

    /// Watch the scratch root once it is there, and stop when it goes. Claude Code makes it with
    /// the first session after the Mac starts, often after Sutra, and no stream over a folder that
    /// is not there yet can tell of it. Discovery reads again what the new stream did not hear.
    fn follow_the_scratch_root(&mut self, now: Instant) {
        let Some(root) = self.state.scratch_root() else {
            return;
        };
        if root.is_dir() == self.watches.roots().iter().any(|watched| watched == root) {
            return;
        }
        self.watches.restart_over_roots(&self.state.watched_roots());
        self.dirty |= self
            .state
            .report_problems(&[Source::Watcher], self.watches.problems());
        self.timers.rediscover.mark((), now);
    }

    fn on_measured(&mut self, measured: Measured) {
        let again = self.sizing.finished(&measured, Instant::now());
        let Measured { job, usage, .. } = measured;
        self.dirty |= self.state.apply_usage(&job, usage);
        if again {
            self.state.mark_measuring(&job);
        }
        self.dispatch();
    }

    /// Act on every burst that fell due, and on the tick.
    fn run_due(&mut self, now: Instant) {
        // Whether projects or artifact directories may have joined the map.
        let mut reshaped = false;
        let rescanned = self.timers.rescan.take_due(now);
        if !rescanned.is_empty() {
            // The rediscovery a rescan starts serves any pending one too.
            self.timers.rediscover.clear();
            self.timers.moved_in.clear();
            let first = self.lost_events.map_or(now, |lost| lost.first);
            self.lost_events = Some(LostEvents { first, latest: now });
        }
        // Not short-circuited: one rediscovery serves both bursts, so both are cleared.
        let asked = self.timers.rediscover.fire(now) | self.timers.moved_in.fire(now);
        let rediscover = !rescanned.is_empty() || asked;
        let environments = self.timers.environments.fire(now);
        // Both refreshes can add projects to the map or take them off.
        let projects_before = (rediscover || environments).then(|| self.state.project_paths());
        if rediscover {
            self.state.rediscover(Some(SystemTime::now()));
            self.remeasure_changed_under(&rescanned, now);
            // A rediscovery searches a scratch folder only when it is new (see `state::scratch`),
            // and lost events may have hidden a build folder made in one.
            for folder in self.state.scratch_folders_meeting(&rescanned) {
                self.timers.scratch_layout.mark(folder, now);
            }
            self.dirty = true;
            reshaped = true;
        }
        if environments && self.state.refresh_environments(Some(SystemTime::now())) {
            self.dirty = true;
            reshaped = true;
        }
        if let Some(before) = projects_before {
            self.size_again_around_changes(&before, now);
        }
        for session in self.timers.scratch_folders.take_due(now) {
            match self.state.refind_scratch_folders(&session) {
                Some(changed) => {
                    self.dirty |= changed;
                    reshaped |= changed;
                }
                // A rediscovery lists every session's folders, and says what it could not list.
                None => self.timers.rediscover.mark((), now),
            }
        }
        // Not short-circuited: one reload serves both bursts, so both are cleared.
        let agents_due = self.timers.claude_sessions.fire(now) | self.timers.codex.fire(now);
        let tick = now >= self.timers.next_tick;
        if agents_due || tick {
            self.dirty |= self.state.refresh_agents();
        }
        if tick {
            self.dirty |= self.state.refresh_liveness();
            if self.watches.stop_watching_gone_folders() {
                self.timers.rediscover.mark((), now);
            }
            self.follow_the_scratch_root(now);
            self.timers.next_tick = now + TICK;
        }
        let walking_after_loss = tick && self.walk_trees_after_lost_events(now);
        for worktree in self.timers.git.take_due(now) {
            self.dirty |= self.state.refresh_git(&worktree);
        }
        let layouts = self.timers.layout.take_due(now);
        for project in layouts
            .into_iter()
            .chain(self.timers.scratch_layout.take_due(now))
        {
            if self.state.refind_artifacts(&project) {
                self.dirty = true;
                reshaped = true;
                // Its own files were walked around the build folders it had before.
                self.timers.own_files.mark(project, now);
            }
        }
        let artifacts = self.timers.artifacts.take_due(now);
        for artifact in &artifacts {
            self.request_measurement(&Job::Artifact(artifact.clone()), Urgency::Changed, now);
        }
        let folders = self.timers.own_files.take_due(now);
        for project in &folders {
            // A file change in a private folder may be routed here, and walking the folder
            // would raise the privacy prompt.
            if self.state.is_readable(project) {
                self.request_measurement(&Job::OwnFiles(project.clone()), Urgency::Changed, now);
            }
        }
        if reshaped {
            let on_map: HashSet<Job> = self.state.jobs_newest_first().into_iter().collect();
            self.sizing.forget_unless(|job| on_map.contains(job));
            // Before the walks queued below begin, so a new watch hears a change made during one.
            self.keep_watches_current(now);
            self.queue_unmeasured(now);
        }
        let rested = self.sizing.release_rested(now);
        if reshaped || rested || walking_after_loss || !artifacts.is_empty() || !folders.is_empty()
        {
            self.dispatch();
        }
    }

    /// Keep a watch on each folder outside the roots that the map needs file events from, and on no
    /// other, and have discovery read again what a new watch cannot have heard.
    fn keep_watches_current(&mut self, now: Instant) {
        let folders = self.state.folders_outside_the_roots();
        if self.watches.watch_outside_the_roots(folders) {
            self.timers.rediscover.mark((), now);
        }
        self.dirty |= self
            .state
            .report_problems(&[Source::Watcher], self.watches.problems());
    }

    /// Queue every measurement with no size yet, newest activity first.
    fn queue_unmeasured(&mut self, now: Instant) {
        for job in self.state.unmeasured() {
            self.request_measurement(&job, Urgency::Backlog, now);
        }
    }

    /// After dropped events, treat each tree under (or holding) a folder whose events were lost
    /// as though its events had arrived, if its top changed since its last walk began. FSEvents
    /// names the root when it cannot keep up, as under a heavy build, and walking every tree each
    /// time would keep several cores busy for as long as the build runs. Marking its timer gives
    /// a changed tree the pace delivered events give it, so nothing here can walk a tree more
    /// often than they do: a project's own files at most every ten minutes while they keep
    /// changing, a build folder when it pauses and its rest allows. A change below the top of a
    /// tree whose events were all lost shows at the tree's next event (see `disk::TopStamp`), or
    /// at the latest in `walk_trees_after_lost_events`.
    fn remeasure_changed_under(&mut self, folders: &[PathBuf], now: Instant) {
        let lost = |tree: &Path| {
            folders
                .iter()
                .any(|folder| tree.starts_with(folder) || folder.starts_with(tree))
        };
        for job in self.state.jobs_newest_first() {
            let state = &self.state;
            let changed = lost(job.root())
                && self
                    .sizing
                    .top_changed(&job, |project| state.own_files_left_out(project));
            if changed {
                match job {
                    Job::Artifact(artifact) => self.timers.artifacts.mark(artifact, now),
                    Job::OwnFiles(project) => self.timers.own_files.mark(project, now),
                }
            }
        }
    }

    /// Once `WALK_AFTER_LOST_EVENTS` has passed since events first went missing, walk every tree
    /// whose last walk began before they last did, behind whatever else waits. Returns whether it
    /// did.
    fn walk_trees_after_lost_events(&mut self, now: Instant) -> bool {
        let Some(lost) = self
            .lost_events
            .filter(|lost| now >= lost.first + WALK_AFTER_LOST_EVENTS)
        else {
            return false;
        };
        self.lost_events = None;
        for job in self.state.jobs_newest_first() {
            if !self.sizing.walk_began_since(&job, lost.latest) {
                self.request_measurement(&job, Urgency::Backlog, now);
            }
        }
        true
    }

    /// Size again the own files of every project holding one that joined or left the map since
    /// `before`, because they were walked around the projects nested in them then, and look for its
    /// build folders again, because a nested project that left no longer holds its own.
    fn size_again_around_changes(&mut self, before: &BTreeSet<PathBuf>, now: Instant) {
        let after = self.state.project_paths();
        for changed in before.symmetric_difference(&after) {
            for enclosing in self.state.enclosing_projects(changed) {
                self.layout_changed(enclosing.clone(), now);
                self.timers.own_files.mark(enclosing, now);
            }
        }
    }

    fn request_measurement(&mut self, job: &Job, urgency: Urgency, now: Instant) {
        self.sizing.request(job, urgency, now);
        self.state.mark_measuring(job);
        self.dirty = true;
    }

    fn dispatch(&mut self) {
        let state = &self.state;
        self.sizing
            .dispatch(|project| state.own_files_left_out(project));
    }
}

#[cfg(test)]
mod tests;
