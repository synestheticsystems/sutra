//! The file watchers whose events the engine thread routes: one over the roots, and one or more
//! over the folders outside them that the map needs file events from.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use notify::event::Flag;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::live::Message;
use crate::map::{Source, SourceProblem};

/// The most streams over folders outside the roots. FSEvents gives a process 512 streams, and the
/// next one starts without complaint and reports nothing (macOS 26.7, 2026-11-04), which `notify`
/// cannot tell. What is left of the 512 is for the stream over the roots and any that a framework
/// starts.
const STREAMS_OUTSIDE: usize = 500;

/// Every file watcher the engine keeps. Each folder outside the roots has a stream of its own
/// while there are streams to spare, so that a change to the map restarts no running stream. A
/// stream takes a new path only by stopping and starting again from now (see
/// `FsEventWatcher::watch_inner`), which drops what it has not delivered yet and gives no notice
/// of the loss: in eight runs of writing 20,000 files, adding one path lost the events of 5 to 281
/// of them, and in four runs adding none lost none (2026-11-04). A stream costs a thread and about
/// 25 KiB. Past the last stream to spare, a folder joins the stream that watches the fewest, which
/// starts again and has its other folders rescanned (see `Watches::start_stream`).
pub(super) struct Watches {
    /// Where each watcher sends its events.
    messages: Sender<Message>,
    /// Kept for its stream to run. `None` when it could not start.
    _over_roots: Option<RecommendedWatcher>,
    root_problems: Vec<SourceProblem>,
    /// The streams over the folders outside the roots that the map needs file events from.
    outside: Vec<Stream>,
    outside_problems: Vec<SourceProblem>,
}

/// A stream over one or more folders outside the roots.
struct Stream {
    folders: BTreeSet<PathBuf>,
    /// Kept for its stream to run.
    _watcher: RecommendedWatcher,
}

/// What starting a stream came to.
struct Started {
    /// `None` when it watches none of its folders.
    stream: Option<Stream>,
    /// Whether discovery should read again (see `Watches::watch_outside_the_roots`).
    read_again: bool,
    /// The folders it could not watch, and why.
    failures: Vec<(PathBuf, io::ErrorKind)>,
}

impl Watches {
    /// Watches over `roots`, and over nothing outside them yet.
    pub(super) fn over_roots(roots: &[PathBuf], messages: Sender<Message>) -> Watches {
        let (over_roots, root_problems) = watch_roots(roots, messages.clone());
        Watches {
            messages,
            _over_roots: over_roots,
            root_problems,
            outside: Vec::new(),
            outside_problems: Vec::new(),
        }
    }

    /// Watch each of `folders` and no other folder outside the roots, leaving alone the stream of
    /// each one already watched, unless that stream watches others that leave or join it. Returns
    /// whether discovery should read again: a watch hears only what changes after it begins, and
    /// discovery read its folder before that, or read a folder that has gone since.
    pub(super) fn watch_outside_the_roots(&mut self, mut folders: BTreeSet<PathBuf>) -> bool {
        // Each stream is to watch the folders it watches that are still wanted. One left with none
        // stops here, which frees its stream for a folder that joins.
        let mut plan: Vec<(Option<Stream>, BTreeSet<PathBuf>)> = Vec::new();
        for stream in mem::take(&mut self.outside) {
            let kept: BTreeSet<PathBuf> = stream
                .folders
                .iter()
                .filter(|folder| folders.remove(*folder))
                .cloned()
                .collect();
            if !kept.is_empty() {
                plan.push((Some(stream), kept));
            }
        }
        // The folders left are new. Each has a stream of its own while there are streams to spare,
        // and past them joins the stream that is to watch the fewest.
        for folder in folders {
            let fewest = if plan.len() < STREAMS_OUTSIDE {
                None
            } else {
                let wanted = plan.iter_mut().map(|(_, wanted)| wanted);
                wanted.min_by_key(|wanted| wanted.len())
            };
            match fewest {
                Some(wanted) => {
                    wanted.insert(folder);
                }
                None => plan.push((None, BTreeSet::from([folder]))),
            }
        }
        let mut read_again = false;
        let mut failures = Vec::new();
        for (stream, wanted) in plan {
            match stream {
                Some(stream) if stream.folders == wanted => self.outside.push(stream),
                replaced => {
                    let started = self.start_stream(wanted, replaced);
                    read_again |= started.read_again;
                    failures.extend(started.failures);
                    self.outside.extend(started.stream);
                }
            }
        }
        self.outside_problems = SourceProblem::per_kind(Source::Watcher, failures);
        read_again
    }

    /// A stream over `folders`, which takes the place of `replaced`, the stream that watched some
    /// of them before, if there was one. That stream stops first, so that the streams never number
    /// more than `STREAMS_OUTSIDE`, and it drops what it had not delivered, while the new one hears
    /// only what changes from now. So a rescan notice, the one `notify` gives when FSEvents drops
    /// events, names the folders that both watch, and the engine reads them again as it does after
    /// any lost events. `notify` adds a path to a stream only by starting it again, so a stream
    /// over k folders starts k times: 80 ms for 100 folders (2026-11-05). Spreading the folders
    /// over every stream keeps each one's share, and each start, small.
    fn start_stream(&self, folders: BTreeSet<PathBuf>, replaced: Option<Stream>) -> Started {
        // Taking the folders drops the rest of the replaced stream, its watcher, which stops it.
        let before = replaced.map_or_else(BTreeSet::new, |stream| stream.folders);
        let mut watcher = match forwarding_watcher(self.messages.clone()) {
            Ok(watcher) => watcher,
            Err(err) => {
                let kind = failure_kind(&err);
                return Started {
                    stream: None,
                    read_again: kind == io::ErrorKind::NotFound,
                    failures: folders.into_iter().map(|folder| (folder, kind)).collect(),
                };
            }
        };
        let mut watched = BTreeSet::new();
        let mut read_again = false;
        let mut failures = Vec::new();
        for folder in folders {
            match watcher.watch(&folder, RecursiveMode::Recursive) {
                Ok(()) => {
                    read_again |= !before.contains(&folder);
                    watched.insert(folder);
                }
                Err(err) => {
                    let kind = failure_kind(&err);
                    read_again |= kind == io::ErrorKind::NotFound;
                    failures.push((folder, kind));
                }
            }
        }
        let watched_before = watched.iter().filter(|folder| before.contains(*folder));
        let rescan = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        let notice = watched_before.cloned().fold(rescan, Event::add_path);
        if !notice.paths.is_empty() {
            // Fails only while the engine shuts down, when no one is left to read them again.
            let _ = self.messages.send(Message::Fs(Ok(notice)));
        }
        let stream = (!watched.is_empty()).then(|| Stream {
            folders: watched,
            _watcher: watcher,
        });
        Started {
            stream,
            read_again,
            failures,
        }
    }

    /// Stop watching the folders outside the roots that are gone, and say whether any was. A stream
    /// reports nothing when the folder around its own moves or is removed (macOS 26.7, 2026-11-04),
    /// so no event tells. A stream that watches other folders too starts again over them.
    pub(super) fn stop_watching_gone_folders(&mut self) -> bool {
        let is_there = |folder: &PathBuf| fs::symlink_metadata(folder).is_ok();
        // Asked on every tick, when almost always every folder is there.
        if self.outside.iter().all(|s| s.folders.iter().all(is_there)) {
            return false;
        }
        for stream in mem::take(&mut self.outside) {
            let there: BTreeSet<PathBuf> = stream
                .folders
                .iter()
                .filter(|f| is_there(f))
                .cloned()
                .collect();
            if there.len() == stream.folders.len() {
                self.outside.push(stream);
                continue;
            }
            // A folder that cannot be watched again is as good as gone: the rediscovery that this
            // asks for has `watch_outside_the_roots` try it again, and report it if it fails.
            let started = self.start_stream(there, Some(stream));
            self.outside.extend(started.stream);
        }
        true
    }

    /// The folders outside the roots that a stream watches.
    #[cfg(test)]
    pub(super) fn watched_outside(&self) -> Vec<&PathBuf> {
        let mut watched: Vec<&PathBuf> = self.outside.iter().flat_map(|s| &s.folders).collect();
        watched.sort();
        watched
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
        assert_eq!(watches.outside.len(), 2, "each on a stream of its own");
        assert!(!watches.watch_outside_the_roots(BTreeSet::from([second.clone()])));
        assert_eq!(watches.watched_outside(), [&second]);
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

    /// Use up every stream to spare on folders of `fixture`, one each, and return those folders. A
    /// watcher watching nothing has no stream, so these take none of the 512 from the tests
    /// running beside this one.
    fn use_up_the_streams(fixture: &TempDir, watches: &mut Watches) -> Vec<PathBuf> {
        let held: Vec<PathBuf> = (0..STREAMS_OUTSIDE)
            .map(|n| fixture.dir(&format!("held/{n}")))
            .collect();
        for folder in &held {
            watches.outside.push(Stream {
                folders: BTreeSet::from([folder.clone()]),
                _watcher: forwarding_watcher(watches.messages.clone()).unwrap(),
            });
        }
        held
    }

    /// The one other folder on the stream that watches `folder`.
    fn partner_of(watches: &Watches, folder: &Path) -> PathBuf {
        let stream = watches.outside.iter().find(|s| s.folders.contains(folder));
        let folders = &stream.expect("a stream watches it").folders;
        let others: Vec<&PathBuf> = folders.iter().filter(|other| *other != folder).collect();
        assert_eq!(others.len(), 1, "{folders:?}");
        others[0].clone()
    }

    /// The folders that the rescan notices waiting in `inbox` name.
    fn rescanned(inbox: &Receiver<Message>) -> Vec<PathBuf> {
        let notices = inbox.try_iter().filter_map(|message| match message {
            Message::Fs(Ok(event)) if event.need_rescan() => Some(event.paths),
            _ => None,
        });
        notices.flatten().collect()
    }

    #[test]
    fn past_the_last_stream_to_spare_folders_join_the_streams_watching_fewest_and_all_report() {
        let fixture = TempDir::new("watch-join");
        let (messages, inbox) = mpsc::channel();
        let mut watches = Watches::over_roots(&[], messages);
        let held = use_up_the_streams(&fixture, &mut watches);
        let joining = [fixture.dir("joining-a"), fixture.dir("joining-b")];

        let wanted = held.iter().chain(&joining).cloned().collect();
        assert!(
            watches.watch_outside_the_roots(wanted),
            "what joined is read again"
        );
        assert_eq!(watches.problems(), []);
        assert_eq!(
            watches.outside.len(),
            STREAMS_OUTSIDE,
            "no stream past the last"
        );
        let partners = joining
            .each_ref()
            .map(|folder| partner_of(&watches, folder));
        assert_ne!(partners[0], partners[1], "each joined a stream of its own");

        for folder in joining.iter().chain(&partners) {
            assert_reported(&inbox, &folder.join("written.txt"));
        }
    }

    #[test]
    fn a_folder_joining_or_leaving_a_running_stream_has_the_stream_rescan_its_other_folders() {
        let fixture = TempDir::new("watch-rescan");
        let (messages, inbox) = mpsc::channel();
        let mut watches = Watches::over_roots(&[], messages);
        let held = use_up_the_streams(&fixture, &mut watches);
        let joining = fixture.dir("joining");
        let with_joining: BTreeSet<PathBuf> = held.iter().chain([&joining]).cloned().collect();
        let without: BTreeSet<PathBuf> = held.iter().cloned().collect();

        assert!(watches.watch_outside_the_roots(with_joining));
        let partner = partner_of(&watches, &joining);
        let partner_alone = std::slice::from_ref(&partner);
        assert_eq!(rescanned(&inbox), partner_alone, "the stream it joined");

        assert!(
            !watches.watch_outside_the_roots(without.clone()),
            "nothing new to read"
        );
        assert_eq!(rescanned(&inbox), partner_alone, "the stream it left");

        assert!(!watches.watch_outside_the_roots(without));
        let none = rescanned(&inbox);
        assert!(
            none.is_empty(),
            "the same folders again start nothing: {none:?}"
        );
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
        assert_eq!(watches.watched_outside(), [&beside]);
    }

    #[test]
    fn a_gone_folder_leaves_the_stream_it_shares_which_rescans_the_folders_left_on_it() {
        let fixture = TempDir::new("watch-forget-gone-shared");
        let (messages, inbox) = mpsc::channel();
        let mut watches = Watches::over_roots(&[], messages);
        let held = use_up_the_streams(&fixture, &mut watches);
        let joining = fixture.dir("joining");
        watches.watch_outside_the_roots(held.iter().chain([&joining]).cloned().collect());
        let partner = partner_of(&watches, &joining);
        rescanned(&inbox);

        fs::remove_dir(&joining).unwrap();

        assert!(watches.stop_watching_gone_folders());
        assert!(!watches.stop_watching_gone_folders(), "told once");
        assert!(!watches.watched_outside().contains(&&joining));
        assert_eq!(
            rescanned(&inbox),
            [partner],
            "the folder left on its stream"
        );
    }
}
