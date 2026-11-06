//! The engine owns the map: it scans at start, watches the roots and the projects outside them,
//! and publishes a fresh `Snapshot` whenever something changed, so the UI only renders.
//!
//! One thread owns the map (`state`). File events (`watch`) are routed by path (`route`) to the
//! source they concern, each source is refreshed once per burst of events (`debounce`, `timers`),
//! and sizes are measured on a pool of their own (`sizing`), so a 3 s walk of a large `target/`
//! never holds up the map. A 2 s tick re-reads what no file event announces: agents whose status
//! ages, environments whose process died, and watched folders outside the roots that went. Each
//! private folder the walk would enter is asked about on a thread of its own (`probe`), so a
//! privacy prompt holds up nothing but that folder.

mod debounce;
mod live;
mod probe;
mod route;
mod sizing;
mod state;
#[cfg(test)]
mod tests;
mod timers;
mod watch;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use super::grant::Roots;
use super::{ScanOptions, Snapshot};
use probe::Probe;

/// Start the engine on its own threads. The first snapshot arrives after the initial scan, with
/// sizes filled in as they are measured. The scan waits a moment for macOS to answer about the
/// private folders it would enter, and goes without those still unanswered, which join the map
/// when the user says yes to macOS's prompt. With no roots to scan (the user has granted no folder
/// yet) one empty snapshot arrives and the channel then closes, because nothing could ever change
/// it.
pub fn start(roots: Roots, opts: ScanOptions) -> (Handle, Receiver<Arc<Snapshot>>) {
    start_asking(roots, opts, Arc::new(probe::list))
}

/// `start`, asking macOS about each private folder by listing it with `probe`.
fn start_asking(
    roots: Roots,
    opts: ScanOptions,
    probe: Probe,
) -> (Handle, Receiver<Arc<Snapshot>>) {
    // One slot: a consumer that falls behind holds at most one stale snapshot, and the engine
    // retries with a fresh one rather than queueing every intermediate state.
    let (snapshots, receiver) = mpsc::sync_channel(1);
    let (messages, inbox) = mpsc::channel();
    let engine_messages = messages.clone();
    let thread = thread::Builder::new()
        .name("sutra-engine".into())
        .spawn(move || live::run(&roots, &opts, &probe, &snapshots, engine_messages, &inbox))
        .expect("spawn the engine thread");
    (Handle { messages, thread }, receiver)
}

/// A running engine, kept so it can be stopped before an engine on other roots starts: two would
/// publish over each other.
pub struct Handle {
    messages: Sender<live::Message>,
    thread: JoinHandle<()>,
}

impl Handle {
    /// A clean just deleted from `folders`, so measure them at once.
    pub fn measure_after_clean(&self, folders: Vec<PathBuf>) {
        // Fails only when the engine has ended, and its map shows nothing more.
        let _ = self.messages.send(live::Message::Cleaned(folders));
    }

    /// Stop the engine and wait for its thread to end, which drops its file watchers and closes its
    /// snapshot channel. Measurements already running on its sizing pool finish on their own, and
    /// their results are dropped, as is the answer of a privacy prompt still waiting for the user:
    /// the engine thread never waits on one.
    pub fn stop(self) {
        // Fails only when the engine has ended already, as one with no roots does after its one
        // snapshot.
        let _ = self.messages.send(live::Message::Stop);
        // A panic on the engine thread was printed when it happened, and it closed the snapshot
        // channel, which the window reports as the scanner stopping, so the join adds nothing.
        let _ = self.thread.join();
    }
}
