//! The lines of the agents working outside every project: one for each session, beside the folder
//! it works in, and one for each other folder subagents work in, so that no folder has two lines of
//! subagents. A subagent at work in a session's folder stands at the end of that session's line, as
//! on a card's tray; the others stand before their folder.

use std::path::Path;

use crate::dashboard::contents::card::{CardContext, agents, subagents};
use crate::dashboard::listing::Elsewhere;
use crate::dashboard::{ElsewhereLine, format};
use crate::map::{AgentSession, SubagentWithParent};

/// The lines: the sessions' in the map's order, each with the subagents at work in that same
/// folder, then a line for each folder the other subagents work in, in the order of the first
/// subagent there. The map orders both whatever their status, so no line or robot moves when a
/// status changes.
pub(super) fn lines(elsewhere: &Elsewhere, cards: &CardContext) -> Vec<ElsewhereLine> {
    let sessions = elsewhere.sessions.iter().map(|&session| Line {
        folder: &session.cwd,
        session: Some(session),
        subagents: Vec::new(),
    });
    let mut lines: Vec<Line> = sessions.collect();
    for &working in &elsewhere.subagents {
        let folder = working.subagent.cwd.as_path();
        match lines.iter_mut().find(|line| line.folder == folder) {
            Some(line) => line.subagents.push(working),
            None => lines.push(Line {
                folder,
                session: None,
                subagents: vec![working],
            }),
        }
    }
    lines.into_iter().map(|line| line.shown(cards)).collect()
}

/// A line in the making: its folder, the session working there when it is a session's line, and
/// the subagents at work in that folder.
struct Line<'a> {
    folder: &'a Path,
    session: Option<&'a AgentSession>,
    subagents: Vec<&'a SubagentWithParent>,
}

impl Line<'_> {
    /// The line as the window draws it.
    fn shown(self, cards: &CardContext) -> ElsewhereLine {
        let folder = format::tilde(self.folder, cards.home);
        let subagents = subagents::badges(self.subagents, cards.now);
        match self.session {
            Some(session) => ElsewhereLine::Session {
                badge: agents::badge(session, cards.now),
                folder,
                subagents,
            },
            None => ElsewhereLine::Subagents { subagents, folder },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::{agent, now, subagent_in};
    use crate::dashboard::{AgentBadge, AgentLook};
    use crate::map::AgentStatus;
    use crate::notifications::Notifier;

    /// A line as a test reads it: its session's look when it is a session's, its folder, and its
    /// subagents by name.
    type Shown = (Option<AgentLook>, String, Vec<String>);

    /// The subagent `name`, at work in `cwd`.
    fn subagent(name: &str, cwd: &str) -> SubagentWithParent {
        let mut working = subagent_in(cwd);
        working.subagent.name = Some(name.to_string());
        working
    }

    /// The lines of `sessions` and `subagents`, under the home folder `/Users/dev`.
    fn lines_of(sessions: &[AgentSession], subagents: &[SubagentWithParent]) -> Vec<Shown> {
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let cards = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new("/Users/dev"),
            now: now(),
        };
        let elsewhere = Elsewhere {
            sessions: sessions.iter().collect(),
            subagents: subagents.iter().collect(),
        };
        let names = |badges: Vec<AgentBadge>| {
            let name = |badge: &AgentBadge| {
                let names = badge.tooltip.lines().nth(1).unwrap_or_default();
                names
                    .split(" \u{b7} ")
                    .next()
                    .unwrap_or_default()
                    .to_string()
            };
            badges.iter().map(name).collect()
        };
        let shown = |line| match line {
            ElsewhereLine::Session {
                badge,
                folder,
                subagents,
            } => (Some(badge.look), folder, names(subagents)),
            ElsewhereLine::Subagents { subagents, folder } => (None, folder, names(subagents)),
        };
        lines(&elsewhere, &cards).into_iter().map(shown).collect()
    }

    /// The line `lines_of` gives for a session in `look`, or for none, in `folder` with `subagents`.
    fn line(look: Option<AgentLook>, folder: &str, subagents: &[&str]) -> Shown {
        let names = subagents.iter().map(|name| name.to_string()).collect();
        (look, folder.to_string(), names)
    }

    #[test]
    fn a_subagent_in_a_sessions_folder_ends_its_line_and_the_others_share_a_line_per_folder() {
        let sessions = [
            agent("/tmp/idle", AgentStatus::Idle),
            agent("/Users/dev/scratch", AgentStatus::Waiting),
        ];
        let subagents = [
            subagent("build-1", "/tmp/build"),
            subagent("helper", "/Users/dev/scratch"),
            subagent("deeper", "/tmp/idle/deeper"),
            subagent("build-2", "/tmp/build"),
        ];

        assert_eq!(
            lines_of(&sessions, &subagents),
            [
                line(Some(AgentLook::Still), "/tmp/idle", &[]),
                line(Some(AgentLook::Waiting), "~/scratch", &["helper"]),
                line(None, "/tmp/build", &["build-1", "build-2"]),
                line(None, "/tmp/idle/deeper", &["deeper"]),
            ],
            "a folder inside a session's is a folder of its own"
        );
    }

    #[test]
    fn a_subagent_in_a_folder_two_sessions_share_ends_the_first_ones_line() {
        let sessions = [
            agent("/tmp/pair", AgentStatus::Busy),
            agent("/tmp/pair", AgentStatus::Idle),
        ];
        let subagents = [subagent("helper", "/tmp/pair")];

        assert_eq!(
            lines_of(&sessions, &subagents),
            [
                line(Some(AgentLook::Working), "/tmp/pair", &["helper"]),
                line(Some(AgentLook::Still), "/tmp/pair", &[]),
            ]
        );
    }
}
