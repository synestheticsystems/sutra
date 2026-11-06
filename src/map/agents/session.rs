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
    /// it lies in a private folder Sutra stays out of, outside the sandbox grant, or no longer
    /// exists.
    pub cwd: PathBuf,
    pub status: AgentStatus,
    /// What the agent is waiting for, in the harness's own words (for example "permission_prompt").
    pub waiting_for: Option<String>,
    /// The harness's own session kind (for example "interactive", "bg", "exec").
    pub kind: Option<String>,
    /// The session's name or title, if it has one.
    pub label: Option<String>,
    /// The model the session works with, by the id its harness records, as `claude-opus-5-5` or
    /// `gpt-6-astra` reads: for Claude Code, the model of its newest reply; for Codex, the one its
    /// thread runs on.
    pub model: Option<String>,
    /// When the current status was set; for a quiet agent, when it last wrote.
    pub since: Option<SystemTime>,
    pub last_activity: Option<SystemTime>,
    /// Where the session runs, when the harness records it.
    pub host: Option<SessionHost>,
    /// The session's subagents at work: Claude Code's subagents, or the threads a Codex thread
    /// spawned, by id. While one is, the session counts as busy unless it waits on the user (see
    /// `with_subagent_work`). The map also shows each where it works (see `SubagentWithParent`).
    pub subagents_at_work: Vec<Subagent>,
}

/// One of a session's subagents at work, as its harness's files tell it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subagent {
    /// Claude Code's id for the subagent, or the Codex thread's.
    pub id: String,
    /// The folder it works in, resolved as a session's is (see `resolve_cwd`).
    pub cwd: PathBuf,
    /// Its name, and the task it was given in a few words, when its harness records them.
    pub name: Option<String>,
    pub description: Option<String>,
    /// The kind of agent it is, when its harness records it: Claude Code's agent type, as
    /// `general-purpose` or `Explore` reads, or the role Codex spawned it in, as `explorer` reads.
    pub agent_type: Option<String>,
    /// The model it works with, recorded as a session's is (see `AgentSession::model`).
    pub model: Option<String>,
    /// Busy, or for a Codex thread silent past a minute, quiet: no harness writes down that a
    /// subagent waits on the user, and one at rest is not at work.
    pub status: AgentStatus,
    /// When a quiet subagent last wrote; the others record no start.
    pub since: Option<SystemTime>,
    pub last_write: Option<SystemTime>,
}

/// One of a session's subagents at work, with the session it works for, as the map shows it apart
/// from that session: on the card of the project its folder lies in, or, outside every project,
/// among the agents working elsewhere (see `State::refresh_agents`). One at work beside its session
/// is shown there too, rather than counting on its session alone, so that a card's tray holds all
/// that is at work in its folder, whichever session it works for. The session still counts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentWithParent {
    pub subagent: Subagent,
    /// The session it works for.
    pub parent: ParentSession,
}

/// The session a subagent works for: its harness, its id and name, and where it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentSession {
    pub harness: Harness,
    pub session_id: String,
    pub label: Option<String>,
    pub host: Option<SessionHost>,
}

impl SubagentWithParent {
    /// `subagent`, at work for `session`.
    pub(crate) fn new(subagent: Subagent, session: &AgentSession) -> SubagentWithParent {
        SubagentWithParent {
            subagent,
            parent: ParentSession {
                harness: session.harness,
                session_id: session.session_id.clone(),
                label: session.label.clone(),
                host: session.host.clone(),
            },
        }
    }
}

/// What a session's subagents did, as their harness's files tell it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct SubagentWork {
    pub(super) at_work: Vec<Subagent>,
    /// When one of them last wrote.
    pub(super) last_write: Option<SystemTime>,
}

impl SubagentWork {
    /// Count one subagent, whose files say whether its turn goes on (`mid_turn`) and when it last
    /// wrote, as of `now`. It is at work while mid-turn and not silent past
    /// `SUBAGENT_SILENCE_LIMIT`, and only then does `describe` say what it is, as reading that can
    /// take more of its files.
    pub(super) fn count(
        &mut self,
        mid_turn: bool,
        last_write: Option<SystemTime>,
        now: SystemTime,
        describe: impl FnOnce() -> Subagent,
    ) {
        // `duration_since` fails for a write dated after `now`, which is as fresh as writes come.
        let silent_too_long = |at: SystemTime| {
            now.duration_since(at)
                .is_ok_and(|silent| silent > SUBAGENT_SILENCE_LIMIT)
        };
        if mid_turn && last_write.is_some_and(|at| !silent_too_long(at)) {
            self.at_work.push(describe());
        }
        self.last_write = self.last_write.max(last_write);
    }
}

impl AgentSession {
    /// This session, counting what its subagents did as its own. Their writes are its activity,
    /// and while one is at work the session is busy, whatever its harness says of the session
    /// itself: a session that handed its work to subagents and went idle is still at it. A session
    /// waiting on the user stays waiting, because the user is wanted either way. The subagents
    /// keep the order of their ids, so a reload that finds the same ones compares equal.
    pub(super) fn with_subagent_work(mut self, mut work: SubagentWork) -> AgentSession {
        self.last_activity = self.last_activity.max(work.last_write);
        let promoted = !work.at_work.is_empty()
            && !matches!(self.status, AgentStatus::Busy | AgentStatus::Waiting);
        if promoted {
            self.status = AgentStatus::Busy;
            // No file records when the subagents' work made the session busy.
            self.since = None;
        }
        work.at_work.sort_by(|a, b| a.id.cmp(&b.id));
        self.subagents_at_work = work.at_work;
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
    use crate::map::test_support::{agent_session, at, subagent_at_work};

    const MINUTE: Duration = Duration::from_secs(60);

    /// The work of one subagent `a1`, as of `now`.
    fn one(mid_turn: bool, last_write: Option<SystemTime>, now: SystemTime) -> SubagentWork {
        let mut work = SubagentWork::default();
        work.count(mid_turn, last_write, now, || subagent_at_work("a1"));
        work
    }

    #[test]
    fn a_subagent_is_at_work_while_mid_turn_until_silent_past_the_limit() {
        let now = at(100_000);
        let at_work = |mid_turn, last_write| one(mid_turn, last_write, now).at_work.len() == 1;

        assert!(at_work(true, Some(now - 9 * MINUTE)));
        assert!(at_work(true, Some(now + MINUTE)), "dated after now");
        assert!(!at_work(true, Some(now - 11 * MINUTE)));
        assert!(!at_work(true, None), "undated, so its silence is unknown");
        assert!(!at_work(false, Some(now)), "its turn has closed");
    }

    #[test]
    fn subagent_work_describes_only_those_at_work_and_keeps_the_newest_write() {
        let now = at(100_000);
        let mut work = SubagentWork::default();
        work.count(true, Some(now - MINUTE), now, || subagent_at_work("a1"));
        work.count(false, Some(now - 2 * MINUTE), now, || {
            panic!("a subagent whose turn has closed is not described")
        });
        work.count(true, Some(now - 3 * MINUTE), now, || subagent_at_work("a3"));

        let subagents = work.at_work.iter();
        let ids: Vec<&str> = subagents.map(|subagent| subagent.id.as_str()).collect();
        assert_eq!(ids, ["a1", "a3"]);
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
            .with_subagent_work(working.clone());

            assert_eq!(session.status, expected, "{status:?}");
            assert_eq!(session.subagents_at_work, working.at_work, "{status:?}");
            let since = (status == expected).then_some(at(5));
            assert_eq!(session.since, since, "{status:?} keeps its own start");
        }
    }

    #[test]
    fn a_sessions_subagents_at_work_keep_the_order_of_their_ids_whatever_order_they_were_read_in() {
        let now = at(100_000);
        let mut work = SubagentWork::default();
        for id in ["c3", "a1", "b2"] {
            work.count(true, Some(now), now, || subagent_at_work(id));
        }

        let session = agent_session("s", AgentStatus::Idle).with_subagent_work(work);

        let subagents = session.subagents_at_work.iter();
        let ids: Vec<&str> = subagents.map(|subagent| subagent.id.as_str()).collect();
        assert_eq!(ids, ["a1", "b2", "c3"]);
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
        assert_eq!(newer.subagents_at_work, []);
        assert_eq!(newer.last_activity, Some(at(60_000)));

        let older = session.with_subagent_work(one(false, Some(at(40_000)), now));
        assert_eq!(older.last_activity, Some(at(50_000)), "an older write");
    }

    #[test]
    fn a_subagent_with_its_parent_carries_the_sessions_name_and_host() {
        let host = SessionHost::ClaudeApp {
            app_session_id: Some("local_2b5d".to_string()),
        };
        let session = AgentSession {
            label: Some("Sutra launch".to_string()),
            host: Some(host.clone()),
            ..agent_session("0b1d", AgentStatus::Idle)
        };

        let working = SubagentWithParent::new(subagent_at_work("a1"), &session);

        assert_eq!(working.subagent, subagent_at_work("a1"));
        assert_eq!(
            working.parent,
            ParentSession {
                harness: Harness::ClaudeCode,
                session_id: "0b1d".to_string(),
                label: Some("Sutra launch".to_string()),
                host: Some(host),
            }
        );
    }
}
