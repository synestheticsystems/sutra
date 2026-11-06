//! An agent session as the map holds it, whichever harness recorded it.

use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// How long a subagent mid-turn may go without writing and still count as at work. Its files cannot
/// tell a long model call or tool run from a subagent that died without closing its turn, so past
/// this it is taken for gone. In 1,242 Claude Code subagent transcripts of the last 30 days on
/// Daniel's Mac (2026-11-05), 1 silence mid-turn in 2,500 lasted longer; in Codex's spawned
/// threads, 1 in 100,000.
const SUBAGENT_SILENCE_LIMIT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub harness: Harness,
    pub session_id: String,
    pub pid: Option<u32>,
    /// The working directory, canonical where it may be resolved; as the harness recorded it when
    /// it lies in a private folder Settings keeps Sutra out of, outside the sandbox grant, or no
    /// longer exists.
    pub cwd: PathBuf,
    pub status: AgentStatus,
    /// What the agent is waiting for, in the harness's own words (for example "permission_prompt").
    pub waiting_for: Option<String>,
    /// The harness's own session kind (for example "interactive", "bg", "exec").
    pub kind: Option<String>,
    /// The session's name or title, if it has one.
    pub label: Option<String>,
    /// When the current status was set; for a quiet agent, when it last wrote.
    pub since: Option<SystemTime>,
    pub last_activity: Option<SystemTime>,
    /// Where the session runs, when the harness records it.
    pub host: Option<SessionHost>,
    /// How many of the session's subagents are at work: Claude Code's subagents, or the threads a
    /// Codex thread spawned. While one is, the session counts as busy unless it waits on the user
    /// (see `with_subagent_work`).
    pub subagents_at_work: usize,
}

/// What a session's subagents did, as their harness's files tell it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SubagentWork {
    pub(super) at_work: usize,
    /// When one of them last wrote.
    pub(super) last_write: Option<SystemTime>,
}

impl SubagentWork {
    /// Count one subagent, whose files say whether its turn goes on (`mid_turn`) and when it last
    /// wrote, as of `now`. It is at work while mid-turn and not silent past
    /// `SUBAGENT_SILENCE_LIMIT`.
    pub(super) fn count(
        &mut self,
        mid_turn: bool,
        last_write: Option<SystemTime>,
        now: SystemTime,
    ) {
        // `duration_since` fails for a write dated after `now`, which is as fresh as writes come.
        let silent_too_long = |at: SystemTime| {
            now.duration_since(at)
                .is_ok_and(|silent| silent > SUBAGENT_SILENCE_LIMIT)
        };
        let at_work = mid_turn && last_write.is_some_and(|at| !silent_too_long(at));
        self.at_work += usize::from(at_work);
        self.last_write = self.last_write.max(last_write);
    }
}

impl AgentSession {
    /// This session, counting what its subagents did as its own. Their writes are its activity,
    /// and while one is at work the session is busy, whatever its harness says of the session
    /// itself: a session that handed its work to subagents and went idle is still at it. A session
    /// waiting on the user stays waiting, because the user is wanted either way.
    pub(super) fn with_subagent_work(mut self, work: SubagentWork) -> AgentSession {
        self.subagents_at_work = work.at_work;
        self.last_activity = self.last_activity.max(work.last_write);
        let promoted =
            work.at_work > 0 && !matches!(self.status, AgentStatus::Busy | AgentStatus::Waiting);
        if promoted {
            self.status = AgentStatus::Busy;
            // No file records when the subagents' work made the session busy.
            self.since = None;
        }
        self
    }

    /// What the agent waits for, in reader words: the harness's underscores become spaces.
    pub fn waiting_reason(&self) -> Option<String> {
        self.waiting_for
            .as_deref()
            .map(|reason| reason.replace('_', " "))
    }

    /// Whether this is a batch job that has ended: a `codex exec` whose last turn closed. Unlike an
    /// interactive thread at rest, nothing comes back to it.
    pub fn is_finished_job(&self) -> bool {
        self.harness == Harness::Codex
            && self.kind.as_deref() == Some("exec")
            && self.status == AgentStatus::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Harness {
    ClaudeCode,
    Codex,
}

/// The name people call the harness, for chips, speech and banners.
impl fmt::Display for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Harness::ClaudeCode => "Claude",
            Harness::Codex => "Codex",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentStatus {
    Busy,
    /// Blocked on the user: a permission prompt, a question, a dialog.
    Waiting,
    Idle,
    /// In the middle of a turn, but silent for a while: working on something long, blocked on the
    /// user, or dead. Codex never writes an approval request to disk, so its files cannot tell
    /// which.
    Quiet,
    /// The harness records no status.
    Unknown,
}

/// Where an agent session runs, as its harness records it. The link that opens one is Claude's own
/// protocol, in `claude::app_link`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionHost {
    /// The Claude desktop app. `app_session_id` is the app's own id for the session, which Claude
    /// Code records as `hostSessionId`, when the session file holds one.
    ClaudeApp { app_session_id: Option<String> },
    /// A terminal: the harness was started from a shell.
    Terminal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{agent_session, at};

    const MINUTE: Duration = Duration::from_secs(60);

    /// The work of one subagent, as of `now`.
    fn one(mid_turn: bool, last_write: Option<SystemTime>, now: SystemTime) -> SubagentWork {
        let mut work = SubagentWork::default();
        work.count(mid_turn, last_write, now);
        work
    }

    #[test]
    fn a_subagent_is_at_work_while_mid_turn_until_silent_past_the_limit() {
        let now = at(100_000);
        let at_work = |mid_turn, last_write| one(mid_turn, last_write, now).at_work == 1;

        assert!(at_work(true, Some(now - 9 * MINUTE)));
        assert!(at_work(true, Some(now + MINUTE)), "dated after now");
        assert!(!at_work(true, Some(now - 11 * MINUTE)));
        assert!(!at_work(true, None), "undated, so its silence is unknown");
        assert!(!at_work(false, Some(now)), "its turn has closed");
    }

    #[test]
    fn subagent_work_counts_those_at_work_and_keeps_the_newest_write() {
        let now = at(100_000);
        let mut work = SubagentWork::default();
        work.count(true, Some(now - MINUTE), now);
        work.count(false, Some(now - 2 * MINUTE), now);
        work.count(true, Some(now - 3 * MINUTE), now);

        assert_eq!(work.at_work, 2);
        assert_eq!(work.last_write, Some(now - MINUTE));
    }

    #[test]
    fn a_session_with_a_subagent_at_work_is_busy_unless_it_waits_on_the_user() {
        let now = at(100_000);
        let working = one(true, Some(now), now);
        for (status, expected) in [
            (AgentStatus::Idle, AgentStatus::Busy),
            (AgentStatus::Unknown, AgentStatus::Busy),
            (AgentStatus::Quiet, AgentStatus::Busy),
            (AgentStatus::Busy, AgentStatus::Busy),
            (AgentStatus::Waiting, AgentStatus::Waiting),
        ] {
            let session = AgentSession {
                since: Some(at(5)),
                ..agent_session("s", status)
            }
            .with_subagent_work(working);

            assert_eq!(session.status, expected, "{status:?}");
            assert_eq!(session.subagents_at_work, 1, "{status:?}");
            let since = (status == expected).then_some(at(5));
            assert_eq!(session.since, since, "{status:?} keeps its own start");
        }
    }

    #[test]
    fn a_session_whose_subagents_rest_keeps_its_status_and_takes_their_newer_writes() {
        let now = at(100_000);
        let session = AgentSession {
            last_activity: Some(at(50_000)),
            ..agent_session("s", AgentStatus::Idle)
        };

        let newer = session
            .clone()
            .with_subagent_work(one(false, Some(at(60_000)), now));
        assert_eq!(newer.status, AgentStatus::Idle);
        assert_eq!(newer.subagents_at_work, 0);
        assert_eq!(newer.last_activity, Some(at(60_000)));

        let older = session.with_subagent_work(one(false, Some(at(40_000)), now));
        assert_eq!(older.last_activity, Some(at(50_000)), "an older write");
    }
}
