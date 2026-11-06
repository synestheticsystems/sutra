//! A card's agents, one lozenge per session, as `(Claude) (Codex) 4m` reads, and an outlined one
//! for each subagent at work in the project for a session elsewhere. The look says what an agent
//! is doing: waiting on the user, at work, or quiet mid-turn or at rest. Beside a lozenge stands
//! only how long it has waited or been quiet. A session the Claude app runs opens there on a
//! click, and a subagent's lozenge opens the session it works for.

use std::time::SystemTime;

use crate::dashboard::{AgentLook, AgentLozenge, format};
use crate::map::{AgentSession, AgentStatus, AwaySubagent, SessionHost};

/// The lozenges of the sessions that earn one, then those of the subagents at work here for
/// sessions elsewhere, each in the order the map lists them. An agent keeps its place for as long
/// as it lasts, so a lozenge never moves when its status changes: the look already tells which one
/// waits.
pub(super) fn lozenges(
    agents: &[AgentSession],
    subagents: &[AwaySubagent],
    now: SystemTime,
) -> Vec<AgentLozenge> {
    let sessions = shown(agents).into_iter().map(|agent| lozenge(agent, now));
    let subagents = subagents.iter().map(|away| subagent_lozenge(away, now));
    sessions.chain(subagents).collect()
}

/// Whether any of `agents` waits on the user, which makes the card or the row they are on throb.
pub(in crate::dashboard::contents) fn any_waiting<'a>(
    agents: impl IntoIterator<Item = &'a AgentSession>,
) -> bool {
    agents
        .into_iter()
        .any(|agent| agent.status == AgentStatus::Waiting)
}

/// The sessions that earn a lozenge. A job that has finished earns none: it still counts as the
/// project's activity, and nothing comes back to it.
fn shown(agents: &[AgentSession]) -> Vec<&AgentSession> {
    agents
        .iter()
        .filter(|agent| !agent.is_finished_job())
        .collect()
}

/// One session's lozenge at `now`.
pub(in crate::dashboard::contents) fn lozenge(
    agent: &AgentSession,
    now: SystemTime,
) -> AgentLozenge {
    AgentLozenge {
        harness: agent.harness.to_string(),
        look: look(agent.status),
        outlined: false,
        beside: how_long(agent.status, agent.since, now),
        tooltip: agent_tooltip(agent, now),
        link: agent.host.as_ref().and_then(SessionHost::link),
    }
}

/// The lozenge at `now` of a subagent at work away from its session, coloured as any agent's.
/// The outline, not a word on the card, says it is a subagent; a click opens the session it works
/// for.
pub(in crate::dashboard::contents) fn subagent_lozenge(
    away: &AwaySubagent,
    now: SystemTime,
) -> AgentLozenge {
    let subagent = &away.subagent;
    AgentLozenge {
        harness: away.parent.harness.to_string(),
        look: look(subagent.status),
        outlined: true,
        beside: how_long(subagent.status, subagent.since, now),
        tooltip: subagent_tooltip(away, now),
        link: away.parent.host.as_ref().and_then(SessionHost::link),
    }
}

/// Only what is live moves; the rest is still.
fn look(status: AgentStatus) -> AgentLook {
    match status {
        AgentStatus::Waiting => AgentLook::Waiting,
        AgentStatus::Busy => AgentLook::Working,
        AgentStatus::Quiet | AgentStatus::Idle | AgentStatus::Unknown => AgentLook::Still,
    }
}

/// How long an agent at `status` since `since` has waited, or been quiet, from its first whole
/// minute.
fn how_long(status: AgentStatus, since: Option<SystemTime>, now: SystemTime) -> Option<String> {
    since
        .filter(|_| matches!(status, AgentStatus::Waiting | AgentStatus::Quiet))
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

/// A status in words, which a lozenge says only in its look, at `now` for an agent at `status`
/// since `since`.
fn status_words(status: AgentStatus, since: Option<SystemTime>, now: SystemTime) -> String {
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

/// An agent's tooltip: its status in words, what it waits for when it says, with how many of its
/// subagents are at work, and the session's name, or its id when it has none. The count explains a
/// session busy here that its own app shows at rest, having handed its work to subagents.
fn agent_tooltip(agent: &AgentSession, now: SystemTime) -> String {
    let waits_for = waiting_for(agent).filter(|_| agent.status == AgentStatus::Waiting);
    let status = waits_for.unwrap_or_else(|| status_words(agent.status, agent.since, now));
    let status = match agent.subagents_at_work.len() {
        0 => status,
        n => format!("{status}, {} at work", format::plural(n as u64, "subagent")),
    };
    format!(
        "{status}\n{}",
        agent.label.as_deref().unwrap_or(&agent.session_id)
    )
}

/// A subagent's tooltip: its status in words, then in words what its outline says, that it is a
/// subagent, with its name and task when its harness records them, and the session it works for,
/// by name or else by id.
fn subagent_tooltip(away: &AwaySubagent, now: SystemTime) -> String {
    let subagent = &away.subagent;
    let status = status_words(subagent.status, subagent.since, now);
    let names = [&subagent.name, &subagent.description].into_iter();
    let quoted = names
        .flatten()
        .map(|name| format!("\u{201c}{name}\u{201d}, "));
    let quoted: String = quoted.collect();
    let parent = &away.parent;
    let works_for = parent.label.as_deref().unwrap_or(&parent.session_id);
    format!("{status}\nSubagent {quoted}working for {works_for}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::map::test_support::{agent_session, at, subagent_at_work};
    use crate::map::{Harness, Subagent};

    fn session(status: AgentStatus) -> AgentSession {
        agent_session("abc-123", status)
    }

    #[test]
    fn a_finished_job_earns_no_lozenge_and_an_interactive_thread_at_rest_does() {
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

    /// What stands on and beside `agent`'s lozenge at `now`.
    fn words(agent: &AgentSession, now: SystemTime) -> (String, Option<String>) {
        let lozenge = lozenge(agent, now);
        (lozenge.harness, lozenge.beside)
    }

    #[test]
    fn a_waiting_or_quiet_agent_shows_how_long_and_the_others_only_their_name() {
        let now = at(10_000);
        let four_minutes = Some(now - Duration::from_secs(4 * 60 + 10));
        let mut waiting = session(AgentStatus::Waiting);
        waiting.since = four_minutes;
        let named =
            |harness: &str, beside: Option<&str>| (harness.to_string(), beside.map(str::to_string));
        assert_eq!(words(&waiting, now), named("Claude", Some("4m")));
        let mut codex = session(AgentStatus::Busy);
        codex.harness = Harness::Codex;
        codex.since = four_minutes;
        assert_eq!(words(&codex, now), named("Codex", None));
        codex.status = AgentStatus::Quiet;
        assert_eq!(words(&codex, now), named("Codex", Some("4m")));
        assert_eq!(
            words(&session(AgentStatus::Idle), now),
            named("Claude", None)
        );
    }

    #[test]
    fn a_wait_under_a_minute_or_without_a_start_shows_no_duration() {
        let now = at(10_000);
        let mut waiting = session(AgentStatus::Waiting);
        assert_eq!(words(&waiting, now).1, None);
        waiting.since = Some(now - Duration::from_secs(30));
        assert_eq!(words(&waiting, now).1, None);
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
    fn an_agents_tooltip_says_in_words_what_the_lozenge_says_in_its_look() {
        let now = at(10_000);
        let mut busy = session(AgentStatus::Busy);
        busy.label = Some("Sutra launch".to_string());
        assert_eq!(agent_tooltip(&busy, now), "Busy\nSutra launch");
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
            "Busy, 1 subagent at work\nabc-123"
        );
        busy.subagents_at_work = ["a1", "a2", "a3"].map(subagent_at_work).to_vec();
        assert_eq!(
            agent_tooltip(&busy, now),
            "Busy, 3 subagents at work\nabc-123"
        );
    }

    #[test]
    fn a_cards_lozenges_are_its_sessions_then_the_subagents_at_work_for_sessions_elsewhere() {
        let here = [agent_session("here", AgentStatus::Busy)];
        let elsewhere = agent_session("elsewhere", AgentStatus::Idle);
        let subagents = [AwaySubagent::new(subagent_at_work("a1"), &elsewhere)];

        let lozenges = lozenges(&here, &subagents, at(0));

        let outlined: Vec<bool> = lozenges.iter().map(|lozenge| lozenge.outlined).collect();
        assert_eq!(outlined, [false, true]);
    }

    #[test]
    fn a_subagents_lozenge_is_outlined_coloured_as_any_agents_and_opens_its_sessions_link() {
        let now = at(10_000);
        let host = SessionHost::ClaudeApp {
            app_session_id: Some("local_2b5d1f5a-66c8".to_string()),
        };
        let parent = AgentSession {
            host: Some(host),
            ..session(AgentStatus::Idle)
        };
        let busy = subagent_lozenge(&AwaySubagent::new(subagent_at_work("a1"), &parent), now);
        assert!(busy.outlined);
        assert_eq!(
            (busy.harness.as_str(), busy.look),
            ("Claude", AgentLook::Working)
        );
        assert_eq!(busy.beside, None);
        assert_eq!(
            busy.link.as_deref(),
            Some("claude://code/continue?session=local_2b5d1f5a-66c8")
        );

        let quiet = Subagent {
            status: AgentStatus::Quiet,
            since: Some(now - Duration::from_secs(4 * 60 + 10)),
            ..subagent_at_work("a2")
        };
        let quiet = subagent_lozenge(&AwaySubagent::new(quiet, &parent), now);
        assert_eq!(
            (quiet.look, quiet.beside.as_deref()),
            (AgentLook::Still, Some("4m"))
        );
        assert!(!lozenge(&parent, now).outlined, "a session's is filled");
    }

    #[test]
    fn a_subagents_tooltip_says_its_status_names_it_and_its_task_and_the_session_it_works_for() {
        let now = at(0);
        let launch = AgentSession {
            label: Some("Sutra launch".to_string()),
            ..session(AgentStatus::Idle)
        };
        let tooltip = |subagent: Subagent, parent: &AgentSession| {
            subagent_tooltip(&AwaySubagent::new(subagent, parent), now)
        };
        let fixes = Subagent {
            name: Some("dashboard-fixes".to_string()),
            description: Some("Fix dashboard review findings".to_string()),
            ..subagent_at_work("a1")
        };
        assert_eq!(
            tooltip(fixes, &launch),
            "Busy\nSubagent \u{201c}dashboard-fixes\u{201d}, \u{201c}Fix dashboard review \
             findings\u{201d}, working for Sutra launch"
        );
        let nicknamed = Subagent {
            name: Some("Boyle".to_string()),
            ..subagent_at_work("a2")
        };
        assert_eq!(
            tooltip(nicknamed, &launch),
            "Busy\nSubagent \u{201c}Boyle\u{201d}, working for Sutra launch"
        );
        let unnamed = subagent_at_work("a3");
        let untitled = session(AgentStatus::Idle);
        assert_eq!(
            tooltip(unnamed, &untitled),
            "Busy\nSubagent working for abc-123",
            "a session without a name goes by its id"
        );
    }

    #[test]
    fn agent_tooltip_prefers_the_session_name_over_its_id() {
        let now = at(0);
        let mut named = session(AgentStatus::Busy);
        assert_eq!(agent_tooltip(&named, now), "Busy\nabc-123");
        named.label = Some("fix the parser".to_string());
        assert_eq!(agent_tooltip(&named, now), "Busy\nfix the parser");
    }
}
