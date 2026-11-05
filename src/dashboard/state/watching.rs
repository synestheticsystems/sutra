//! The engine the dashboard runs: started on the roots, and replaced by an engine started afresh
//! when the user grants another folder or changes what the scanner may enter. The old one stops
//! first: an engine left running would go on walking and watching for a map no one shows.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use crate::dashboard::live::Event;
use crate::map::grant::Roots;
use crate::map::{ScanOptions, Snapshot, engine};

/// Tells apart the engines the dashboard has started, so a snapshot that a stopped engine had
/// already sent is dropped rather than shown over the new map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::dashboard) struct EngineId(u64);

/// The running engine.
pub(super) struct Watching {
    id: EngineId,
    engine: engine::Handle,
}

impl Watching {
    /// Start an engine on `roots`, entering what `options` allow, once the engine of `replaced`
    /// has stopped. Its snapshots arrive in `events` as `Event::Snapshot`, then
    /// `Event::EngineStopped` once its channel closes.
    pub(super) fn start(
        roots: Roots,
        options: ScanOptions,
        replaced: Option<Watching>,
        events: &Sender<Event>,
    ) -> Watching {
        let id = EngineId(replaced.as_ref().map_or(0, |old| old.id.0 + 1));
        if let Some(old) = replaced {
            old.stop();
        }
        let (engine, snapshots) = engine::start(roots, options);
        pass_on(id, snapshots, events.clone());
        Watching { id, engine }
    }

    /// Whether an event from engine `id` is from this engine.
    pub(super) fn started(&self, id: EngineId) -> bool {
        self.id == id
    }

    /// Have the engine measure at once the folders a clean just deleted from.
    pub(super) fn measure_after_clean(&self, folders: Vec<PathBuf>) {
        self.engine.measure_after_clean(folders);
    }

    /// Stop the engine and wait for it to end. What it sent before, and the word that its channel
    /// closed, still arrive, and are dropped as not from the running engine.
    pub(super) fn stop(self) {
        self.engine.stop();
    }
}

/// Pass engine `id`'s snapshots on into `events` as they come, then say that its channel closed.
/// The dashboard handles every event waiting before it works out the window's contents once, so
/// snapshots that arrive together cost one look.
fn pass_on(id: EngineId, snapshots: Receiver<Arc<Snapshot>>, events: Sender<Event>) {
    thread::Builder::new()
        .name("sutra-snapshots".into())
        .spawn(move || {
            for snapshot in snapshots {
                // Fails only once the dashboard has stopped, and no one is left to show it.
                if events.send(Event::Snapshot(id, snapshot)).is_err() {
                    return;
                }
            }
            // Fails only once the dashboard has stopped, and no one is left to tell.
            let _ = events.send(Event::EngineStopped(id));
        })
        .expect("spawn the thread that passes the engine's snapshots on");
}
