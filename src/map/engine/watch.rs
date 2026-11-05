//! The file watchers whose events the engine thread routes: one over the roots, and one over each
//! folder outside them that the map needs file events from.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use super::live::Message;
use crate::map::{Source, SourceProblem};

/// The most folders outside the roots to watch. FSEvents gives a process 512 streams, and the next
/// one starts without complaint and reports nothing (macOS 26.7, 2026-11-04), which `notify` cannot
/// tell, so a folder past this counts as one that could not be watched. What is left of the 512 is
/// for the stream over the roots and any that a framework starts.
const MAX_WATCHED_OUTSIDE: usize = 500;

/// Every file watcher the engine keeps, each with a stream of its own, so that a change to the map
/// never restarts a running stream. A stream takes a new path only by stopping and starting again
/// from now (see `FsEventWatcher::watch_inner`), which drops what it has not delivered yet and
/// gives no notice of the loss: in eight runs of writing 20,000 files, adding one path lost the
/// events of 5 to 281 of them, and in four runs adding none lost none (2026-11-04). A stream costs
/// a thread and about 25 KiB.
pub(super) struct Watches {
    /// Where each watcher sends its events.
    messages: Sender<Message>,
    /// Kept for its stream to run. `None` when it could not start.
    _over_roots: Option<RecommendedWatcher>,
    root_problems: Vec<SourceProblem>,
    /// The watcher over each folder outside the roots that the map needs file events from.
    pub(super) outside: BTreeMap<PathBuf, RecommendedWatcher>,
    outside_problems: Vec<SourceProblem>,
}

impl Watches {
    /// Watches over `roots`, and over nothing outside them yet.
    pub(super) fn over_roots(roots: &[PathBuf], messages: Sender<Message>) -> Watches {
        let (over_roots, root_problems) = watch_roots(roots, messages.clone());
        Watches {
            messages,
            _over_roots: over_roots,
            root_problems,
            outside: BTreeMap::new(),
            outside_problems: Vec::new(),
        }
    }

    /// Watch each of `folders` and no other folder outside the roots, leaving the stream of one
    /// already watched alone. Returns whether discovery should read again: a watch hears only what
    /// changes after it begins, and discovery read its folder before that, or read a folder that
    /// has gone since.
    pub(super) fn watch_outside_the_roots(&mut self, folders: BTreeSet<PathBuf>) -> bool {
        self.outside.retain(|folder, _| folders.contains(folder));
        let mut read_again = false;
        let mut failures = Vec::new();
        for folder in folders {
            if self.outside.contains_key(&folder) {
                continue;
            }
            match self.begin_watch(&folder) {
                Ok(watcher) => {
                    self.outside.insert(folder, watcher);
                    read_again = true;
                }
                Err(err) => {
                    let kind = failure_kind(&err);
                    read_again |= kind == io::ErrorKind::NotFound;
                    failures.push((folder, kind));
                }
            }
        }
        self.outside_problems = SourceProblem::per_kind(Source::Watcher, failures);
        read_again
    }

    fn begin_watch(&self, folder: &Path) -> notify::Result<RecommendedWatcher> {
        if self.outside.len() >= MAX_WATCHED_OUTSIDE {
            return Err(notify::Error::new(notify::ErrorKind::MaxFilesWatch));
        }
        let mut watcher = forwarding_watcher(self.messages.clone())?;
        watcher.watch(folder, RecursiveMode::Recursive)?;
        Ok(watcher)
    }

    /// Stop watching the folders outside the roots that are gone, and say whether any was. A stream
    /// reports nothing when the folder around its own moves or is removed (macOS 26.7, 2026-11-04),
    /// so no event tells.
    pub(super) fn stop_watching_gone_folders(&mut self) -> bool {
        let before = self.outside.len();
        self.outside
            .retain(|folder, _| fs::symlink_metadata(folder).is_ok());
        self.outside.len() != before
    }

    /// What is not watched and why: the roots first, then the folders outside them.
    pub(super) fn problems(&self) -> Vec<SourceProblem> {
        self.root_problems
            .iter()
            .chain(&self.outside_problems)
            .cloned()
            .collect()
    }
}

/// A watcher over every root, and a problem for each root it does not watch. Neither failure is
/// fatal: the map still refreshes agents and liveness on every tick, and misses file changes.
fn watch_roots(
    roots: &[PathBuf],
    messages: Sender<Message>,
) -> (Option<RecommendedWatcher>, Vec<SourceProblem>) {
    let mut watcher = match forwarding_watcher(messages) {
        Ok(watcher) => watcher,
        Err(err) => {
            let problems = roots.iter().map(|root| watch_problem(root, &err)).collect();
            return (None, problems);
        }
    };
    let problems = roots
        .iter()
        .filter_map(|root| {
            let err = watcher.watch(root, RecursiveMode::Recursive).err()?;
            Some(watch_problem(root, &err))
        })
        .collect();
    (Some(watcher), problems)
}

/// A watcher whose callback only forwards events; routing happens on the engine thread.
fn forwarding_watcher(messages: Sender<Message>) -> notify::Result<RecommendedWatcher> {
    notify::recommended_watcher(move |event| {
        // Fails only while the engine shuts down, when no one is left to route the event.
        let _ = messages.send(Message::Fs(event));
    })
}

fn watch_problem(root: &Path, err: &notify::Error) -> SourceProblem {
    SourceProblem::new(Source::Watcher, root.to_path_buf(), failure_kind(err))
}

fn failure_kind(err: &notify::Error) -> io::ErrorKind {
    match &err.kind {
        notify::ErrorKind::Io(err) => err.kind(),
        notify::ErrorKind::PathNotFound => io::ErrorKind::NotFound,
        notify::ErrorKind::MaxFilesWatch => io::ErrorKind::QuotaExceeded,
        // Notify's other errors (a bad configuration) have no I/O counterpart.
        _ => io::ErrorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::map::test_support::TempDir;

    /// How long FSEvents gets to deliver one event. Moments on a quiet machine, but a machine
    /// writing many files holds events back far longer (none at all in 15 s during a docs build
    /// on 2026-11-04), so the engine's own tests do not wait for it, and these, whose subject it
    /// is, wait this long.
    const PATIENCE: Duration = Duration::from_secs(180);

    /// Write `file` until `inbox` reports it by its canonical path, or fail after `PATIENCE`.
    fn assert_reported(inbox: &Receiver<Message>, file: &Path) {
        let deadline = Instant::now() + PATIENCE;
        // Written again and again, because FSEvents may drop any one write's event.
        loop {
            fs::write(file, "x").unwrap();
            match inbox.recv_timeout(Duration::from_millis(500)) {
                Ok(Message::Fs(Ok(event))) if event.paths.iter().any(|path| path == file) => {
                    return;
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => panic!("the watcher stopped"),
            }
            assert!(Instant::now() < deadline, "no event within {PATIENCE:?}");
        }
    }

    #[test]
    fn the_watcher_reports_a_file_written_under_a_root_by_its_canonical_path() {
        let fixture = TempDir::new("watch-reports");
        let (messages, inbox) = mpsc::channel();
        let (watcher, problems) = watch_roots(&[fixture.root().to_path_buf()], messages);
        assert!(watcher.is_some());
        assert_eq!(problems, []);

        assert_reported(&inbox, &fixture.path("written.txt"));
    }

    #[test]
    fn a_folder_outside_the_roots_reports_a_file_written_in_it_beside_one_that_joined_later() {
        let fixture = TempDir::new("watch-reports-outside");
        let (messages, inbox) = mpsc::channel();
        let mut watches = Watches::over_roots(&[], messages);
        let first = fixture.dir("first");
        let second = fixture.dir("second");

        assert!(watches.watch_outside_the_roots(BTreeSet::from([first.clone()])));
        assert_reported(&inbox, &first.join("written.txt"));

        assert!(watches.watch_outside_the_roots(BTreeSet::from([first.clone(), second.clone()])));
        assert_reported(&inbox, &second.join("written.txt"));
        assert_reported(&inbox, &first.join("written-later.txt"));
    }

    #[test]
    fn only_a_folder_newly_outside_begins_a_watch_and_one_no_longer_named_ends_its_own() {
        let fixture = TempDir::new("watch-outside-set");
        let (messages, _inbox) = mpsc::channel();
        let mut watches = Watches::over_roots(&[], messages);
        let first = fixture.dir("first");
        let second = fixture.dir("second");

        assert!(watches.watch_outside_the_roots(BTreeSet::from([first.clone()])));
        assert!(
            !watches.watch_outside_the_roots(BTreeSet::from([first.clone()])),
            "the same folders again begin nothing"
        );
        assert!(watches.watch_outside_the_roots(BTreeSet::from([first.clone(), second.clone()])));
        assert!(!watches.watch_outside_the_roots(BTreeSet::from([second.clone()])));
        let watched: Vec<&PathBuf> = watches.outside.keys().collect();
        assert_eq!(watched, [&second]);
        assert_eq!(watches.problems(), []);
    }

    #[test]
    fn a_root_and_a_folder_outside_it_that_cannot_be_watched_are_both_reported_until_it_can() {
        let fixture = TempDir::new("watch-problems");
        let (messages, _inbox) = mpsc::channel();
        let gone_root = fixture.path("gone-root");
        let mut watches = Watches::over_roots(std::slice::from_ref(&gone_root), messages);
        let gone = fixture.path("gone");
        let not_found = io::ErrorKind::NotFound;

        assert!(
            watches.watch_outside_the_roots(BTreeSet::from([gone.clone()])),
            "a folder gone since discovery read it is a reason to read again"
        );
        assert_eq!(
            watches.problems(),
            [
                SourceProblem::new(Source::Watcher, gone_root.clone(), not_found),
                SourceProblem::new(Source::Watcher, gone.clone(), not_found),
            ]
        );

        fs::create_dir(&gone).unwrap();
        assert!(watches.watch_outside_the_roots(BTreeSet::from([gone])));
        assert_eq!(
            watches.problems(),
            [SourceProblem::new(Source::Watcher, gone_root, not_found)]
        );
    }

    #[test]
    fn past_the_limit_a_folder_is_reported_rather_than_left_on_a_stream_that_says_nothing() {
        let fixture = TempDir::new("watch-limit");
        let (messages, _inbox) = mpsc::channel();
        let mut watches = Watches::over_roots(&[], messages.clone());
        // A watcher watching nothing has no stream, so these take none of the 512 from the tests
        // running beside this one.
        let held: Vec<PathBuf> = (0..MAX_WATCHED_OUTSIDE)
            .map(|n| fixture.path(&n.to_string()))
            .collect();
        for folder in &held {
            let watcher = forwarding_watcher(messages.clone()).unwrap();
            watches.outside.insert(folder.clone(), watcher);
        }
        let first = fixture.dir("first");
        let second = fixture.dir("second");

        let too_many = held.iter().chain([&first, &second]).cloned().collect();
        assert!(!watches.watch_outside_the_roots(too_many));
        assert_eq!(
            watches.problems(),
            [SourceProblem {
                count: 2,
                ..SourceProblem::new(Source::Watcher, first.clone(), io::ErrorKind::QuotaExceeded)
            }]
        );

        let one_left = held[1..].iter().chain([&first]).cloned().collect();
        assert!(watches.watch_outside_the_roots(one_left));
        assert_eq!(watches.problems(), []);
        assert!(watches.outside.contains_key(&first));
    }

    #[test]
    fn a_watched_folder_that_is_gone_is_forgotten_once() {
        let fixture = TempDir::new("watch-forget-gone");
        let (messages, _inbox) = mpsc::channel();
        let mut watches = Watches::over_roots(&[], messages);
        let group = fixture.dir("group");
        let inside = fixture.dir("group/wt");
        let beside = fixture.dir("beside");
        assert!(watches.watch_outside_the_roots(BTreeSet::from([inside.clone(), beside.clone()])));
        assert!(!watches.stop_watching_gone_folders(), "both are there");

        fs::rename(&group, fixture.path("moved")).unwrap();

        assert!(watches.stop_watching_gone_folders());
        assert!(!watches.stop_watching_gone_folders(), "told once");
        let watched: Vec<&PathBuf> = watches.outside.keys().collect();
        assert_eq!(watched, [&beside]);
    }
}
