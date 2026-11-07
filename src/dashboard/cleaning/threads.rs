//! The threads that work out and do the cleans, and what each reports as it ends: one of the
//! dashboard's events, sent from whichever thread did the work.

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use crate::dashboard::live::Event;
use crate::map::clean::{CleanError, Cleaned, IdleCleans, PrepareError, Prepared};

/// How a clean ended, as its thread reports it.
#[derive(Debug, Clone)]
pub(in crate::dashboard) enum CleanOutcome {
    Cleaned(Cleaned),
    Failed(CleanError),
    /// The thread ended without a word, which only a panic makes it do.
    Lost,
}

impl From<Option<Result<Cleaned, CleanError>>> for CleanOutcome {
    fn from(deleted: Option<Result<Cleaned, CleanError>>) -> CleanOutcome {
        match deleted {
            Some(Ok(cleaned)) => CleanOutcome::Cleaned(cleaned),
            Some(Err(err)) => CleanOutcome::Failed(err),
            None => CleanOutcome::Lost,
        }
    }
}

/// What a cleaning thread found, reported as it ends. `None` stands for a thread that ended with no
/// word.
#[derive(Debug)]
pub(in crate::dashboard) enum Report {
    /// What a click on Clean means for each folder it cleans, `clicked`: a card's one, or every
    /// build folder of a session's scratch. In the order of `clicked`.
    Prepared {
        clicked: Vec<PathBuf>,
        prepared: Option<Vec<Result<Prepared, PrepareError>>>,
    },
    /// The cleans of every idle folder.
    IdlePrepared(Option<IdleCleans>),
    /// The clean Sutra chose by itself, of `clicked` in the project at `project`.
    UnaskedPrepared {
        clicked: PathBuf,
        project: PathBuf,
        prepared: Option<Result<Prepared, PrepareError>>,
    },
    /// The clean started from `target` ended.
    Cleaned {
        target: PathBuf,
        outcome: CleanOutcome,
    },
}

/// Starts the threads that work out and do the cleans, which report into the dashboard's events.
#[derive(Debug, Clone)]
pub(in crate::dashboard) struct Threads(Sender<Event>);

impl Threads {
    pub(in crate::dashboard) fn new(events: Sender<Event>) -> Threads {
        Threads(events)
    }

    /// Run `work` on a thread of its own named `name`, and report what it returns as `report` puts
    /// it, with `None` should `work` panic, so the user still hears that it stopped. Fails when no
    /// thread can start.
    pub(super) fn run<T>(
        &self,
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
        report: impl FnOnce(Option<T>) -> Report + Send + 'static,
    ) -> io::Result<()> {
        let events = self.0.clone();
        thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                // Of what `work` holds only a clean's progress outlives the thread, and its counts
                // stay whole through a panic (see `Progress`), so nothing half-done is left to read.
                let found = panic::catch_unwind(AssertUnwindSafe(work)).ok();
                // Fails only once the dashboard has stopped, and no one is left to tell.
                let _ = events.send(Event::Cleaning(report(found)));
            })
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_thread_whose_work_panics_reports_that_it_ended_with_no_word() {
        let (events, inbox) = mpsc::channel();
        let threads = Threads::new(events);
        threads
            .run(
                "sutra-test-panics",
                || -> IdleCleans { panic!("the work broke") },
                Report::IdlePrepared,
            )
            .unwrap();
        let reported = inbox.recv_timeout(Duration::from_secs(30));
        assert!(matches!(
            reported,
            Ok(Event::Cleaning(Report::IdlePrepared(None)))
        ));
    }
}
