//! When each kind of refresh falls due: one burst timer per source the engine re-reads, and the
//! tick for what no file event announces.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::debounce::{Debounce, Pending};

/// Claude Code rewrites its session file once per status change, and appends to a subagent's
/// transcript at each step of its turn, so a subagent starting or finishing shows within a second.
const CLAUDE_SESSIONS: Debounce = debounce(300, 1_000);
/// Codex writes its database and rollout continuously during a turn.
const CODEX: Debounce = debounce(2_000, 5_000);
const ENVIRONMENTS: Debounce = debounce(300, 1_000);
/// A commit or checkout touches several files in the git dir within milliseconds.
const GIT: Debounce = debounce(500, 2_000);
/// Rediscovery walks every root, so it waits for a `git worktree add` or a clone to settle.
const REDISCOVER: Debounce = debounce(2_000, 10_000);
/// After lost events the engine rediscovers every root and reads the top of every tree, about
/// 1.5 CPU-seconds on Daniel's home (830 worktrees, 2026-11-04). A heavy build makes FSEvents drop
/// events every few seconds for as long as it runs, so after one recovery the next waits a minute.
/// The first waits no longer than a rediscovery does, so an isolated loss heals at once.
const LOST_EVENTS: Debounce = Debounce {
    min_interval: Duration::from_secs(60),
    ..REDISCOVER
};
/// A folder moved into a root may hold repositories, and nothing else announces them, so the map is
/// rediscovered. Anything in a home can rename a folder, so after one the next waits a minute.
const MOVED_IN: Debounce = Debounce {
    min_interval: Duration::from_secs(60),
    ..REDISCOVER
};
/// A build churns its `target/` for minutes; re-measure when it pauses, and every 20 s while not.
const ARTIFACT: Debounce = debounce(3_000, 20_000);
/// Folders appearing or vanishing in a project, as when a build recreates `target/`.
const LAYOUT: Debounce = debounce(2_000, 10_000);
/// The same in a scratch folder, which is searched through for its build folders: the largest of
/// Daniel's sessions took 0.25 s and a CPU-second (27,000 folders, 2026-11-06). Its agents make
/// folders there all through their work, so the search waits for ten quiet seconds, and comes at
/// most once a minute.
const SCRATCH_LAYOUT: Debounce = Debounce {
    min_interval: Duration::from_secs(60),
    ..debounce(10_000, 60_000)
};
/// A folder coming, going or moving at the top of a session's scratch: the session's scratch
/// folders are listed again, two listings, and only a new one is searched through.
const SCRATCH_FOLDERS: Debounce = debounce(2_000, 10_000);
/// Edits barely change a folder's size, and every walk of a large project costs seconds, so a
/// project's own files are walked again only once its edits pause for a minute, or every ten
/// minutes while an agent keeps writing: a burst of saves costs one walk.
const OWN_FILES: Debounce = debounce(60_000, 600_000);
/// What no file event announces: agent status that ages, an environment's process dying, a watched
/// folder outside the roots going with the folder around it, the scratch root coming or going.
pub(super) const TICK: Duration = Duration::from_secs(2);
/// How often the processes are read for the ports they listen on, which no event announces either.
/// A read costs about 3 ms on Daniel's Mac (see `processes`), so this costs under a millisecond a
/// second, and a server shows within this long of listening.
pub(super) const PROCESSES: Duration = Duration::from_secs(5);
/// How long after events go missing every tree they may have hidden a change in is walked again. A
/// tree's top shows most changes at once (see `Engine::remeasure_changed_under`), and the tree's
/// next event shows the rest, so this bounds how long a size can stay wrong when neither comes.
/// Walking every tree costs about 90 CPU-seconds on Daniel's home (2026-11-04), and a heavy build
/// loses events for as long as it runs, so it cannot come often.
pub(super) const WALK_AFTER_LOST_EVENTS: Duration = Duration::from_secs(6 * 60 * 60);

const fn debounce(quiet_ms: u64, max_wait_ms: u64) -> Debounce {
    Debounce {
        quiet: Duration::from_millis(quiet_ms),
        max_wait: Duration::from_millis(max_wait_ms),
        min_interval: Duration::ZERO,
    }
}

/// The bursts of events not yet acted on, one set per kind of refresh.
pub(super) struct Timers {
    pub(super) claude_sessions: Pending<()>,
    pub(super) codex: Pending<()>,
    pub(super) environments: Pending<()>,
    pub(super) rediscover: Pending<()>,
    pub(super) moved_in: Pending<()>,
    /// Folders where FSEvents dropped events, so the map is rediscovered and the trees under them
    /// whose top changed are measured again.
    pub(super) rescan: Pending<PathBuf>,
    pub(super) git: Pending<PathBuf>,
    pub(super) artifacts: Pending<PathBuf>,
    pub(super) layout: Pending<PathBuf>,
    pub(super) scratch_layout: Pending<PathBuf>,
    /// The folders of the sessions whose scratch folders may have come, gone or moved.
    pub(super) scratch_folders: Pending<PathBuf>,
    /// Projects whose own files changed, or whose nested projects or build folders did.
    pub(super) own_files: Pending<PathBuf>,
    pub(super) next_tick: Instant,
    /// When the processes are next read for the ports they listen on; `None` in the store edition,
    /// whose sandbox tells it nothing of other processes, so they are never read.
    pub(super) next_processes: Option<Instant>,
}

impl Timers {
    /// The timers from `now`, reading the processes when `reads_processes`.
    pub(super) fn new(now: Instant, reads_processes: bool) -> Self {
        Timers {
            claude_sessions: Pending::new(CLAUDE_SESSIONS),
            codex: Pending::new(CODEX),
            environments: Pending::new(ENVIRONMENTS),
            rediscover: Pending::new(REDISCOVER),
            moved_in: Pending::new(MOVED_IN),
            rescan: Pending::new(LOST_EVENTS),
            git: Pending::new(GIT),
            artifacts: Pending::new(ARTIFACT),
            layout: Pending::new(LAYOUT),
            scratch_layout: Pending::new(SCRATCH_LAYOUT),
            scratch_folders: Pending::new(SCRATCH_FOLDERS),
            own_files: Pending::new(OWN_FILES),
            next_tick: now + TICK,
            next_processes: reads_processes.then_some(now),
        }
    }

    pub(super) fn next_due(&self) -> Instant {
        [
            self.claude_sessions.next_due(),
            self.codex.next_due(),
            self.environments.next_due(),
            self.rediscover.next_due(),
            self.moved_in.next_due(),
            self.rescan.next_due(),
            self.git.next_due(),
            self.artifacts.next_due(),
            self.layout.next_due(),
            self.scratch_layout.next_due(),
            self.scratch_folders.next_due(),
            self.own_files.next_due(),
        ]
        .into_iter()
        .chain([self.next_processes])
        .flatten()
        .fold(self.next_tick, Instant::min)
    }
}
