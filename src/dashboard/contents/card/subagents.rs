//! The subagents at work in a folder, a robot each: after the divider on the card's tray, or at the
//! end of a line of the agents working elsewhere. The robot says which AI the subagent is and the
//! light of its face what it is doing, as a session's does; standing after the divider says it is
//! a subagent. Its tooltip names it, its task, what kind of agent it is, the model it works with
//! and the session it works for, and a click opens that session.

use std::time::SystemTime;

use super::agents::{look, status_words};
use super::robots::robot;
use crate::dashboard::AgentBadge;
use crate::map::{SessionHost, SubagentWithParent};

/// The badges at `now` of `subagents`, in the order given: the map's, by session and then each
/// session's own, which holds whatever their status, so no robot moves when its light changes.
pub(in crate::dashboard::contents) fn badges<'a>(
    subagents: impl IntoIterator<Item = &'a SubagentWithParent>,
    now: SystemTime,
) -> Vec<AgentBadge> {
    let subagents = subagents.into_iter();
    subagents.map(|working| badge(working, now)).collect()
}

/// The badge at `now` of a subagent at work: the robot of the model it works with, or else of the
/// harness of the session it works for.
fn badge(working: &SubagentWithParent, now: SystemTime) -> AgentBadge {
    let SubagentWithParent { subagent, parent } = working;
    AgentBadge {
        robot: robot(parent.harness, subagent.model.as_deref()),
        look: look(subagent.status),
        tooltip: tooltip(working, now),
        link: parent.host.as_ref().and_then(SessionHost::link),
    }
}

/// The badge's tooltip: the subagent's status in words, as every agent's tooltip begins; a line with
/// its name and its task, when its harness records either; and a line with what kind of agent it
/// is and the model it works with, when recorded, and the session it works for, by name or else by
/// id.
fn tooltip(working: &SubagentWithParent, now: SystemTime) -> String {
    let SubagentWithParent { subagent, parent } = working;
    let works_for = format!(
        "for {}",
        parent.label.as_deref().unwrap_or(&parent.session_id)
    );
    let lines = [
        Some(status_words(subagent.status, subagent.since, now)),
        line([subagent.name.as_deref(), subagent.description.as_deref()]),
        line([
            subagent.agent_type.as_deref(),
            subagent.model.as_deref(),
            Some(&works_for),
        ]),
    ];
    lines.into_iter().flatten().collect::<Vec<_>>().join("\n")
}

/// A line of a tooltip: the parts of it that are known, between dots, or `None` when none is.
fn line<'a>(parts: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    let known: Vec<&str> = parts.into_iter().flatten().collect();
    (!known.is_empty()).then(|| known.join(" \u{b7} "))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::dashboard::{AgentLook, Robot};
    use crate::map::test_support::{agent_session, at, subagent_at_work};
    use crate::map::{AgentSession, AgentStatus, Harness, Subagent};

    /// The idle session `abc-123`, as Claude Code records it, with no name.
    fn parent() -> AgentSession {
        agent_session("abc-123", AgentStatus::Idle)
    }

    #[test]
    fn a_subagents_badge_shows_its_robot_and_its_own_look_and_opens_its_session() {
        let now = at(10_000);
        let host = SessionHost::ClaudeApp {
            app_session_id: Some("local_2b5d1f5a-66c8".to_string()),
        };
        let claude = AgentSession {
            host: Some(host),
            ..parent()
        };
        let busy = badge(
            &SubagentWithParent::new(subagent_at_work("a1"), &claude),
            now,
        );
        assert_eq!((busy.robot, busy.look), (Robot::Claude, AgentLook::Working));
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
        let quiet = badge(&SubagentWithParent::new(quiet, &codex), now);
        assert_eq!((quiet.robot, quiet.look), (Robot::Codex, AgentLook::Still));
        assert_eq!(quiet.link, None, "a session in a terminal has no link");

        let on_glm = Subagent {
            model: Some("glm-4.6".to_string()),
            ..subagent_at_work("a2")
        };
        let on_glm = badge(&SubagentWithParent::new(on_glm, &claude), now);
        assert_eq!(on_glm.robot, Robot::Plain, "its own model picks its robot");
    }

    #[test]
    fn a_subagents_tooltip_says_its_status_names_it_and_its_task_then_says_what_it_is_and_for_whom()
    {
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
            agent_type: Some("general-purpose".to_string()),
            model: Some("claude-opus-5-5".to_string()),
            ..subagent_at_work("a1")
        };
        assert_eq!(
            tooltip(fixes, &launch),
            "Busy\n\
             dashboard-fixes \u{b7} Fix dashboard review findings\n\
             general-purpose \u{b7} claude-opus-5-5 \u{b7} for Sutra launch"
        );
        let nicknamed = Subagent {
            name: Some("Boyle".to_string()),
            model: Some("gpt-5.5".to_string()),
            status: AgentStatus::Quiet,
            since: Some(now - Duration::from_secs(4 * 60 + 10)),
            ..subagent_at_work("t1")
        };
        assert_eq!(
            tooltip(nicknamed, &launch),
            "Mid-turn, silent for 4m\nBoyle\ngpt-5.5 \u{b7} for Sutra launch",
            "a sub-agent spawned in no role, whose task Codex does not record"
        );
        let unnamed = Subagent {
            agent_type: Some("Explore".to_string()),
            ..subagent_at_work("a2")
        };
        assert_eq!(
            tooltip(unnamed, &launch),
            "Busy\nExplore \u{b7} for Sutra launch",
            "with neither name nor task, no line for them"
        );
        assert_eq!(
            tooltip(subagent_at_work("a3"), &parent()),
            "Busy\nfor abc-123",
            "a subagent its harness says nothing more of, for a session without a name"
        );
    }
}
