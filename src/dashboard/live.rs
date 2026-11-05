//! The dashboard thread: it owns the dashboard's state, handles the events that reach it a batch at
//! a time, keeps the clock, and after each batch tells the window when what it shows has changed.
//! The clock runs only what has something to do: the tick always, the check of waits while one
//! is pending, and the beat while a clean deletes.

use std::iter;
use std::ops::ControlFlow;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use super::cleaning::Report;
use super::state::{EngineId, State};
use super::{Action, DashboardWindow, WindowContents, lock};
use crate::map::Snapshot;

/// What reaches the dashboard thread.
pub(super) enum Event {
    /// The user asked for something (`Dashboard::perform`).
    Action(Action),
    /// A map from the engine started as `EngineId`.
    Snapshot(EngineId, Arc<Snapshot>),
    /// That engine's channel closed, so its map no longer changes.
    EngineStopped(EngineId),
    /// What a cleaning thread found.
    Cleaning(Report),
    /// The app quits (`Dashboard::stop`).
    Stop,
}

/// How often the dashboard looks again without an event: the ages the window shows grow, and
/// cleaning by itself checks free space.
const TICK: Duration = Duration::from_secs(2);

/// How often a wait not yet announced is checked against `notifications`' delay.
const WAIT_CHECK: Duration = Duration::from_secs(1);

/// Five times a second while a clean deletes, so its card's bar moves: the bar is the clean's own
/// count of what it freed, which only the dashboard reads.
const BEAT: Duration = Duration::from_millis(200);

/// Handle `state`'s events from `inbox` until `Event::Stop`, publishing into `published` what the
/// window shows and telling `window` each time it changes.
pub(super) fn run(
    mut state: State,
    inbox: &Receiver<Event>,
    window: &dyn DashboardWindow,
    published: &Mutex<WindowContents>,
) {
    let mut clock = Clock::new(Instant::now());
    'running: loop {
        let wake = clock.next_wake(state.is_deleting(), state.has_pending_waits());
        let first = match inbox.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            // The state holds a sender of its own, so the channel closes only once it is gone.
            Err(RecvTimeoutError::Disconnected) => break 'running,
        };
        for event in first
            .into_iter()
            .chain(iter::from_fn(|| inbox.try_recv().ok()))
        {
            if handle(&mut state, event).is_break() {
                break 'running;
            }
        }
        let at = Instant::now();
        if clock.tick_due(at) {
            state.tick(at, SystemTime::now());
        }
        if state.has_pending_waits() && clock.wait_check_due(at) {
            state.announce_lasting_waits();
        }
        publish(state.contents(SystemTime::now()), window, published);
        clock.worked_out(at);
    }
    state.stop();
}

/// Hand `event` to the method of `state` that takes it; `Break` once the app quits.
pub(super) fn handle(state: &mut State, event: Event) -> ControlFlow<()> {
    let now = SystemTime::now();
    match event {
        Event::Action(action) => state.perform(action, now),
        Event::Snapshot(engine, snapshot) => state.on_snapshot(engine, snapshot, now),
        Event::EngineStopped(engine) => state.on_engine_stopped(engine),
        Event::Cleaning(report) => state.on_report(report, now),
        Event::Stop => return ControlFlow::Break(()),
    }
    ControlFlow::Continue(())
}

/// Keep `contents` as what the window shows and tell `window`, unless the window shows that
/// already.
fn publish(
    contents: WindowContents,
    window: &dyn DashboardWindow,
    published: &Mutex<WindowContents>,
) {
    {
        let mut shown = lock(published);
        if *shown == contents {
            return;
        }
        *shown = contents;
    }
    // Outside the lock, as the window reads the contents again from here.
    window.changed();
}

/// When the dashboard next looks again without an event.
struct Clock {
    tick: Instant,
    wait_check: Instant,
    beat: Instant,
}

impl Clock {
    /// A clock started at `start`, whose first tick is a whole `TICK` away and whose other looks
    /// are due at once when they come to matter.
    fn new(start: Instant) -> Clock {
        Clock {
            tick: start + TICK,
            wait_check: start,
            beat: start,
        }
    }

    /// The next time to look again: the tick, the check of waits while one is `waits_pending`,
    /// and the beat while a clean is `deleting`.
    fn next_wake(&self, deleting: bool, waits_pending: bool) -> Instant {
        let mut wake = self.tick;
        if waits_pending {
            wake = wake.min(self.wait_check);
        }
        if deleting {
            wake = wake.min(self.beat);
        }
        wake
    }

    /// Whether the tick is due `at`, setting the next one when it is.
    fn tick_due(&mut self, at: Instant) -> bool {
        due(&mut self.tick, at, TICK)
    }

    /// Whether the check of waits is due `at`, setting the next one when it is.
    fn wait_check_due(&mut self, at: Instant) -> bool {
        due(&mut self.wait_check, at, WAIT_CHECK)
    }

    /// The window's contents were worked out `at`, so a clean's bar needs no other look before
    /// the next beat.
    fn worked_out(&mut self, at: Instant) {
        self.beat = at + BEAT;
    }
}

/// Whether `next` has come by `at`, moving it a whole `every` past `at` when it has.
fn due(next: &mut Instant, at: Instant, every: Duration) -> bool {
    if at < *next {
        return false;
    }
    *next = at + every;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tick_always_runs_and_the_other_timers_only_while_they_have_something_to_do() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        assert_eq!(
            clock.next_wake(false, false),
            start + TICK,
            "nothing else to do"
        );
        assert_eq!(clock.next_wake(true, false), start, "a clean deleting");
        assert_eq!(clock.next_wake(false, true), start, "a wait pending");

        clock.worked_out(start);
        assert!(clock.wait_check_due(start));
        assert_eq!(clock.next_wake(true, true), start + BEAT);
        assert_eq!(clock.next_wake(false, true), start + WAIT_CHECK);
        assert!(!clock.wait_check_due(start + BEAT), "checked a moment ago");
    }

    #[test]
    fn the_tick_keeps_its_pace_whatever_events_come_between() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        assert!(!clock.tick_due(start + TICK / 2));
        assert!(clock.tick_due(start + TICK));
        assert_eq!(clock.next_wake(false, false), start + TICK * 2);
    }
}
