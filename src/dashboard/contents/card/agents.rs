//! A card's sessions, a robot each on its tray, before the subagents (see `subagents`). The robot
//! says which AI the session is (see `robots`), and the light of its face what the session is
//! doing: waiting on the user, at work, or quiet mid-turn or at rest. Its tooltip says the rest in
//! words, how long it has waited among them, and a session the Claude app runs opens there on a
//! click.

use std::time::SystemTime;

use super::robots::robot;
use crate::dashboard::{AgentBadge, AgentLook, format};
use crate::map::{AgentSession, AgentStatus, SessionHost};

/// The badges of the sessions that earn one, in the order the map lists the sessions. A session
/// keeps its place for as long as it lasts, so a robot never moves when its status changes: its
/// face already tells which one waits.
pub(super) fn badges(agents: &[AgentSession], now: SystemTime) -> Vec<AgentBadge> {
    shown(agents)
        .into_iter()
        .map(|agent| badge(agent, now))
        .collect()
}

/// Whether any of `agents` waits on the user, which makes the card or the row they are on throb.
pub(in crate::dashboard::contents) fn any_waiting<'a>(
    agents: impl IntoIterator<Item = &'a AgentSession>,
) -> bool {
    agents
        .into_iter()
        .any(|agent| agent.status == AgentStatus::Waiting)
}

/// The sessions that earn a badge. A job that has finished earns none: it still counts as the
/// project's activity, and nothing comes back to it.
fn shown(agents: &[AgentSession]) -> Vec<&AgentSession> {
    agents
        .iter()
        .filter(|agent| !agent.is_finished_job())
        .collect()
}

/// One session's badge at `now`.
pub(in crate::dashboard::contents) fn badge(agent: &AgentSession, now: SystemTime) -> AgentBadge {
    AgentBadge {
        robot: robot(agent.harness, agent.model.as_deref()),
        look: look(agent.status),
        tooltip: agent_tooltip(agent, now),
        link: agent.host.as_ref().and_then(SessionHost::link),
    }
}

/// The look of an agent at `status`, a session's or a subagent's: only what is live is coloured,
/// and the rest is grey.
pub(super) fn look(status: AgentStatus) -> AgentLook {
    match status {
        AgentStatus::Waiting => AgentLook::Waiting,
        AgentStatus::Busy => AgentLook::Working,
        AgentStatus::Quiet | AgentStatus::Idle | AgentStatus::Unknown => AgentLook::Still,
    }
}

/// How long a session has waited, from its first whole minute.
fn how_long_waiting(agent: &AgentSession, now: SystemTime) -> Option<String> {
    agent
        .since
        .filter(|_| agent.status == AgentStatus::Waiting)
        .and_then(|since| format::whole_units(since, now))
}

/// What a waiting session waits for, when its harness says: Claude Code's permission prompt, the
/// wait people meet most, reads as a sentence; any other reason keeps the harness's own words.
fn waiting_for(agent: &AgentSession) -> Option<String> {
    match agent.waiting_for.as_deref()? {
        "permission_prompt" => Some("waiting for permission".to_string()),
        _ => agent
            .waiting_reason()
            .map(|reason| format!("waiting: {reason}")),
    }
}

/// A status in words, which a robot says only in the light of its face, at `now` for an agent at
/// `status` since `since`.
pub(super) fn status_words(
    status: AgentStatus,
    since: Option<SystemTime>,
    now: SystemTime,
) -> String {
    match status {
        AgentStatus::Waiting => "waiting".to_string(),
        AgentStatus::Busy => "Busy".to_string(),
        AgentStatus::Quiet => match since {
            Some(since) => format!("Mid-turn, silent for {}", format::age(since, now)),
            None => "Mid-turn, silent".to_string(),
        },
        AgentStatus::Idle => "Idle".to_string(),
        AgentStatus::Unknown => "Status unknown".to_string(),
    }
}

/// A session's tooltip: its status in words, what it waits for when it says and for how long from
/// the first whole minute, with how many of its subagents are at work; the session's name, or its
/// id when it has none; and its harness, with the model it works with when the harness records
/// it, as the robot shows only which AI it is. The count explains a session busy here that its own
/// app shows at rest, having handed its work to subagents.
fn agent_tooltip(agent: &AgentSession, now: SystemTime) -> String {
    let waits_for = waiting_for(agent).filter(|_| agent.status == AgentStatus::Waiting);
    let status = waits_for.unwrap_or_else(|| status_words(agent.status, agent.since, now));
    let status = match how_long_waiting(agent, now) {
        Some(how_long) => format!("{status} ({how_long})"),
        None => status,
    };
    let status = match agent.subagents_at_work.len() {
        0 => status,
        n => format!("{status}, {} at work", format::plural(n as u64, "subagent")),
    };
    let name = agent.label.as_deref().unwrap_or(&agent.session_id);
    let harness = match &agent.model {
        Some(model) => format!("{} \u{b7} {model}", agent.harness),
        None => agent.harness.to_string(),
    };
    format!("{status}\n{name}\n{harness}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::dashboard::Robot;
    use crate::map::Harness;
    use crate::map::test_support::{agent_session, at, subagent_at_work};

    fn session(status: AgentStatus) -> AgentSession {
        agent_session("abc-123", status)
    }

    #[test]
    fn a_finished_job_earns_no_badge_and_an_interactive_thread_at_rest_does() {
        let codex = |id: &str, kind: &str, status| AgentSession {
            harness: Harness::Codex,
            kind: Some(kind.to_string()),
            ..agent_session(id, status)
        };
        let agents = [
            codex("job", "exec", AgentStatus::Idle),
            codex("running", "exec", AgentStatus::Busy),
            codex("resting", "cli", AgentStatus::Idle),
        ];
        let ids: Vec<&str> = shown(&agents)
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect();
        assert_eq!(ids, ["running", "resting"]);
    }

    #[test]
    fn each_session_keeps_its_place_whatever_its_status() {
        let agents = [
            agent_session("idle", AgentStatus::Idle),
            agent_session("busy", AgentStatus::Busy),
            agent_session("unknown", AgentStatus::Unknown),
            agent_session("waiting", AgentStatus::Waiting),
            agent_session("quiet", AgentStatus::Quiet),
        ];
        let ids: Vec<&str> = shown(&agents)
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect();
        assert_eq!(ids, ["idle", "busy", "unknown", "waiting", "quiet"]);
    }

    #[test]
    fn a_session_waiting_looks_it_one_at_work_looks_busy_and_the_rest_are_still() {
        let looks = [
            AgentStatus::Waiting,
            AgentStatus::Busy,
            AgentStatus::Quiet,
            AgentStatus::Idle,
            AgentStatus::Unknown,
        ]
        .map(look);
        assert_eq!(
            looks,
            [
                AgentLook::Waiting,
                AgentLook::Working,
                AgentLook::Still,
                AgentLook::Still,
                AgentLook::Still
            ]
        );
    }

    #[test]
    fn a_sessions_robot_is_the_one_of_the_model_it_works_with_or_else_of_its_harness() {
        let now = at(0);
        let mut codex = session(AgentStatus::Busy);
        codex.harness = Harness::Codex;
        assert_eq!(badge(&codex, now).robot, Robot::Codex);
        codex.model = Some("glm-4.6".to_string());
        assert_eq!(badge(&codex, now).robot, Robot::Plain);
        assert_eq!(badge(&session(AgentStatus::Idle), now).robot, Robot::Claude);
    }

    #[test]
    fn a_waiting_sessions_tooltip_says_how_long_from_its_first_whole_minute() {
        let now = at(10_000);
        let mut waiting = session(AgentStatus::Waiting);
        waiting.waiting_for = Some("permission_prompt".to_string());
        assert!(agent_tooltip(&waiting, now).starts_with("waiting for permission\n"));
        waiting.since = Some(now - Duration::from_secs(30));
        assert!(agent_tooltip(&waiting, now).starts_with("waiting for permission\n"));
        waiting.since = Some(now - Duration::from_secs(4 * 60 + 10));
        assert!(agent_tooltip(&waiting, now).starts_with("waiting for permission (4m)\n"));
    }

    #[test]
    fn a_waiting_agents_tooltip_says_what_it_waits_for() {
        let now = at(10_000);
        let mut waiting = session(AgentStatus::Waiting);
        waiting.waiting_for = Some("permission_prompt".to_string());
        assert!(agent_tooltip(&waiting, now).starts_with("waiting for permission\n"));
        waiting.waiting_for = Some("user_question".to_string());
        assert!(agent_tooltip(&waiting, now).starts_with("waiting: user question\n"));
    }

    #[test]
    fn an_agents_tooltip_says_in_words_what_its_robot_says_in_its_light() {
        let now = at(10_000);
        let mut busy = session(AgentStatus::Busy);
        busy.label = Some("Sutra launch".to_string());
        assert_eq!(agent_tooltip(&busy, now), "Busy\nSutra launch\nClaude");
        let mut quiet = session(AgentStatus::Quiet);
        quiet.since = Some(now - Duration::from_secs(4 * 60));
        assert!(agent_tooltip(&quiet, now).starts_with("Mid-turn, silent for 4m\n"));
        assert!(agent_tooltip(&session(AgentStatus::Unknown), now).starts_with("Status unknown\n"));
    }

    #[test]
    fn an_agents_tooltip_counts_its_subagents_at_work() {
        let now = at(0);
        let mut busy = session(AgentStatus::Busy);
        busy.subagents_at_work = vec![subagent_at_work("a1")];
        assert_eq!(
            agent_tooltip(&busy, now),
            "Busy, 1 subagent at work\nabc-123\nClaude"
        );
        busy.subagents_at_work = ["a1", "a2", "a3"].map(subagent_at_work).to_vec();
        assert_eq!(
            agent_tooltip(&busy, now),
            "Busy, 3 subagents at work\nabc-123\nClaude"
        );
    }

    #[test]
    fn agent_tooltip_prefers_the_session_name_over_its_id() {
        let now = at(0);
        let mut named = session(AgentStatus::Busy);
        assert_eq!(agent_tooltip(&named, now), "Busy\nabc-123\nClaude");
        named.label = Some("fix the parser".to_string());
        assert_eq!(agent_tooltip(&named, now), "Busy\nfix the parser\nClaude");
    }

    #[test]
    fn an_agents_tooltip_ends_with_its_harness_and_the_model_it_works_with_when_recorded() {
        let now = at(0);
        let mut busy = AgentSession {
            label: Some("Sutra launch".to_string()),
            model: Some("claude-opus-5-5".to_string()),
            subagents_at_work: vec![subagent_at_work("a1")],
            ..session(AgentStatus::Busy)
        };
        assert_eq!(
            agent_tooltip(&busy, now),
            "Busy, 1 subagent at work\nSutra launch\nClaude \u{b7} claude-opus-5-5"
        );
        busy.model = None;
        assert_eq!(
            agent_tooltip(&busy, now),
            "Busy, 1 subagent at work\nSutra launch\nClaude"
        );
    }
}
