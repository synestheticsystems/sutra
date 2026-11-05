//! Coalescing a burst of file events into one refresh. A build writes thousands of files and an
//! agent rewrites its session file on every status change; re-reading a source once per burst
//! keeps the engine's work proportional to what changed, not to how many writes it took.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// When a burst of events is acted on.
#[derive(Debug, Clone, Copy)]
pub(super) struct Debounce {
    /// Act this long after the burst's last event, so one refresh sees the whole burst.
    pub(super) quiet: Duration,
    /// But no later than this after its first event, so a source that never goes quiet (a long
    /// build, a busy agent) is still refreshed while it churns.
    pub(super) max_wait: Duration,
    /// And no sooner than this after the last time any of its keys was acted on, for a refresh
    /// that costs too much to repeat as often as its events can come.
    pub(super) min_interval: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Burst {
    first: Instant,
    last: Instant,
}

/// The keys with a burst of events not yet acted on: one per project, artifact or source.
#[derive(Debug)]
pub(super) struct Pending<K> {
    debounce: Debounce,
    bursts: HashMap<K, Burst>,
    /// `None` until something falls due.
    last_acted: Option<Instant>,
}

impl<K: Eq + Hash + Clone> Pending<K> {
    pub(super) fn new(debounce: Debounce) -> Self {
        Pending {
            debounce,
            bursts: HashMap::new(),
            last_acted: None,
        }
    }

    /// Record an event for `key` at `now`.
    pub(super) fn mark(&mut self, key: K, now: Instant) {
        self.bursts
            .entry(key)
            .or_insert(Burst {
                first: now,
                last: now,
            })
            .last = now;
    }

    /// The earliest moment a burst falls due, if any is pending.
    pub(super) fn next_due(&self) -> Option<Instant> {
        let due = self
            .bursts
            .values()
            .map(|burst| due_at(self.debounce, burst))
            .min()?;
        Some(self.next_allowed().map_or(due, |allowed| due.max(allowed)))
    }

    /// The earliest the next burst may be acted on, if one was acted on before.
    fn next_allowed(&self) -> Option<Instant> {
        self.last_acted.map(|at| at + self.debounce.min_interval)
    }

    /// Drop every pending burst, because a broader refresh is about to cover them.
    pub(super) fn clear(&mut self) {
        self.bursts.clear();
    }

    /// Remove and return every key whose burst is due at `now`.
    pub(super) fn take_due(&mut self, now: Instant) -> Vec<K> {
        if self.next_allowed().is_some_and(|allowed| now < allowed) {
            return Vec::new();
        }
        let debounce = self.debounce;
        let mut due = Vec::new();
        self.bursts.retain(|key, burst| {
            let keep = due_at(debounce, burst) > now;
            if !keep {
                due.push(key.clone());
            }
            keep
        });
        if !due.is_empty() {
            self.last_acted = Some(now);
        }
        due
    }
}

impl Pending<()> {
    /// For a source with a single key: whether its burst fell due (and is now cleared).
    pub(super) fn fire(&mut self, now: Instant) -> bool {
        !self.take_due(now).is_empty()
    }
}

fn due_at(debounce: Debounce, burst: &Burst) -> Instant {
    (burst.last + debounce.quiet).min(burst.first + debounce.max_wait)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBOUNCE: Debounce = Debounce {
        quiet: Duration::from_secs(3),
        max_wait: Duration::from_secs(20),
        min_interval: Duration::ZERO,
    };

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_burst_falls_due_after_its_last_event_goes_quiet() {
        let t0 = Instant::now();
        let mut pending = Pending::new(DEBOUNCE);
        pending.mark("target", t0);
        pending.mark("target", t0 + secs(2));
        assert_eq!(pending.next_due(), Some(t0 + secs(5)));
        assert!(pending.take_due(t0 + secs(4)).is_empty());
        assert_eq!(pending.take_due(t0 + secs(5)), ["target"]);
        assert_eq!(pending.next_due(), None);
    }

    #[test]
    fn a_burst_that_never_goes_quiet_falls_due_at_max_wait() {
        let t0 = Instant::now();
        let mut pending = Pending::new(DEBOUNCE);
        for second in 0..=19 {
            pending.mark("target", t0 + secs(second));
        }
        assert!(pending.take_due(t0 + secs(19)).is_empty());
        // Events every second never leave 3 s of quiet; the first was at t0, so the burst is cut
        // off at t0 + 20 s.
        assert_eq!(pending.next_due(), Some(t0 + secs(20)));
        assert_eq!(pending.take_due(t0 + secs(20)), ["target"]);
    }

    #[test]
    fn keys_fall_due_independently() {
        let t0 = Instant::now();
        let mut pending = Pending::new(DEBOUNCE);
        pending.mark("a", t0);
        pending.mark("b", t0 + secs(2));
        assert_eq!(pending.take_due(t0 + secs(3)), ["a"]);
        assert_eq!(pending.take_due(t0 + secs(5)), ["b"]);
    }

    #[test]
    fn a_minimum_interval_holds_back_every_burst_after_the_first_and_loses_none() {
        let t0 = Instant::now();
        let mut pending = Pending::new(Debounce {
            min_interval: secs(60),
            ..DEBOUNCE
        });
        pending.mark("a", t0);
        assert_eq!(
            pending.take_due(t0 + secs(3)),
            ["a"],
            "the first is not held"
        );

        pending.mark("a", t0 + secs(10));
        pending.mark("b", t0 + secs(50));
        assert_eq!(pending.next_due(), Some(t0 + secs(63)));
        assert!(pending.take_due(t0 + secs(62)).is_empty());
        let mut due = pending.take_due(t0 + secs(63));
        due.sort_unstable();
        assert_eq!(due, ["a", "b"]);
        assert_eq!(pending.next_due(), None);
    }

    #[test]
    fn fire_reports_and_clears_a_single_key_source() {
        let t0 = Instant::now();
        let mut pending = Pending::new(DEBOUNCE);
        assert!(!pending.fire(t0));
        pending.mark((), t0);
        assert!(pending.fire(t0 + secs(3)));
        assert!(!pending.fire(t0 + secs(30)));
    }
}
