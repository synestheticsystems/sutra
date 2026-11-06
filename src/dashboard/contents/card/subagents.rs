//! The subagents at work in a folder, a small icon each: at the end of the card's tray, or of a
//! line of the agents working elsewhere. The icon says the subagent's harness and its colour what
//! the subagent is doing, as a session's lozenge does; standing apart from the sessions' lozenges
//! says it is a subagent. Its tooltip names it, its task and the session it works for, and a click
//! opens that session.

use std::time::SystemTime;

use super::agents::{look, status_words};
use crate::dashboard::{AgentHarness, SubagentIcon};
use crate::map::{Harness, SessionHost, SubagentWithParent};

/// The icons at `now` of `subagents`, in the order given: the map's, by session and then each
/// session's own, which holds whatever their status, so no icon moves when its colour changes.
pub(in crate::dashboard::contents) fn icons<'a>(
    subagents: impl IntoIterator<Item = &'a SubagentWithParent>,
    now: SystemTime,
) -> Vec<SubagentIcon> {
    let subagents = subagents.into_iter();
    subagents.map(|working| icon(working, now)).collect()
}

/// The icon at `now` of a subagent at work.
fn icon(working: &SubagentWithParent, now: SystemTime) -> SubagentIcon {
    let SubagentWithParent { subagent, parent } = working;
    SubagentIcon {
        harness: match parent.harness {
            Harness::ClaudeCode => AgentHarness::ClaudeCode,
            Harness::Codex => AgentHarness::Codex,
        },
        look: look(subagent.status),
        tooltip: tooltip(working, now),
        link: parent.host.as_ref().and_then(SessionHost::link),
    }
}

/// The icon's tooltip: the subagent's status in words, as every agent's tooltip begins, then a line
/// with its name and its task, when its harness records them, and the session it works for, by
/// name or else by id.
fn tooltip(working: &SubagentWithParent, now: SystemTime) -> String {
    let SubagentWithParent { subagent, parent } = working;
    let status = status_words(subagent.status, subagent.since, now);
    let works_for = format!(
        "for {}",
        parent.label.as_deref().unwrap_or(&parent.session_id)
    );
    let names = [subagent.name.as_deref(), subagent.description.as_deref()];
    let names = names.into_iter().flatten().chain([works_for.as_str()]);
    format!("{status}\n{}", names.collect::<Vec<_>>().join(" \u{b7} "))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::dashboard::AgentLook;
    use crate::map::test_support::{agent_session, at, subagent_at_work};
    use crate::map::{AgentSession, AgentStatus, Subagent};

    /// The idle session `abc-123`, as Claude Code records it, with no name.
    fn parent() -> AgentSession {
        agent_session("abc-123", AgentStatus::Idle)
    }

    #[test]
    fn a_subagents_icon_shows_its_sessions_harness_and_its_own_look_and_opens_its_session() {
        let now = at(10_000);
        let host = SessionHost::ClaudeApp {
            app_session_id: Some("local_2b5d1f5a-66c8".to_string()),
        };
        let claude = AgentSession {
            host: Some(host),
            ..parent()
        };
        let busy = icon(
            &SubagentWithParent::new(subagent_at_work("a1"), &claude),
            now,
        );
        assert_eq!(
            (busy.harness, busy.look),
            (AgentHarness::ClaudeCode, AgentLook::Working)
        );
        assert_eq!(
            busy.link.as_deref(),
            Some("claude://code/continue?session=local_2b5d1f5a-66c8")
        );

        let codex = AgentSession {
            harness: Harness::Codex,
            host: Some(SessionHost::Terminal),
            ..parent()
        };
        let quiet = Subagent {
            status: AgentStatus::Quiet,
            ..subagent_at_work("t1")
        };
        let quiet = icon(&SubagentWithParent::new(quiet, &codex), now);
        assert_eq!(
            (quiet.harness, quiet.look),
            (AgentHarness::Codex, AgentLook::Still)
        );
        assert_eq!(quiet.link, None, "a session in a terminal has no link");
    }

    #[test]
    fn a_subagents_tooltip_says_its_status_then_names_it_its_task_and_the_session_it_works_for() {
        let now = at(10_000);
        let launch = AgentSession {
            label: Some("Sutra launch".to_string()),
            ..parent()
        };
        let tooltip = |subagent: Subagent, parent: &AgentSession| {
            tooltip(&SubagentWithParent::new(subagent, parent), now)
        };
        let fixes = Subagent {
            name: Some("dashboard-fixes".to_string()),
            description: Some("Fix dashboard review findings".to_string()),
            ..subagent_at_work("a1")
        };
        assert_eq!(
            tooltip(fixes, &launch),
            "Busy\ndashboard-fixes \u{b7} Fix dashboard review findings \u{b7} for Sutra launch"
        );
        let nicknamed = Subagent {
            name: Some("Boyle".to_string()),
            status: AgentStatus::Quiet,
            since: Some(now - Duration::from_secs(4 * 60 + 10)),
            ..subagent_at_work("t1")
        };
        assert_eq!(
            tooltip(nicknamed, &launch),
            "Mid-turn, silent for 4m\nBoyle \u{b7} for Sutra launch"
        );
        assert_eq!(
            tooltip(subagent_at_work("a3"), &parent()),
            "Busy\nfor abc-123",
            "a subagent its harness says nothing more of, for a session without a name"
        );
    }
}
