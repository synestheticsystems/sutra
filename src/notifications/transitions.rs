//! Which transitions happened between two loads, and what to say about each.

use std::collections::HashMap;
use std::mem;
use std::time::{Duration, SystemTime};

use super::{Sound, unit_key};
use crate::map::{AgentSession, AgentStatus, Harness};
use crate::model::{Environment, State};

/// How long an agent must wait on the user before the wait is announced. A permission prompt the
/// user answers sooner, at the terminal they are already looking at, makes no sound at all.
pub(super) const WAIT_DELAY: Duration = Duration::from_secs(10);

/// One transition to tell the user about.
#[derive(Debug, PartialEq)]
pub(super) struct Announcement {
    pub(super) sound: Sound,
    pub(super) speech: String,
    pub(super) banner_title: String,
    pub(super) banner_body: String,
    /// The unit whose own mute and notification toggles apply. Agents answer to the global
    /// toggles only.
    pub(super) unit_key: Option<String>,
}

/// The unit states and agent statuses of the previous load, so the next load can be diffed
/// against them, and the waits that have not lasted long enough to announce yet.
#[derive(Default)]
pub(super) struct Transitions {
    units: HashMap<(String, String), State>,
    agents: HashMap<(Harness, String), AgentStatus>,
    waits: HashMap<(Harness, String), Wait>,
    /// A load has been recorded. The first load only records, because everything in it would read
    /// as a change.
    primed: bool,
}

/// An agent waiting on the user, not announced yet.
#[derive(Debug)]
struct Wait {
    /// When the wait began: the agent's own record of it, or else when Sutra first saw it.
    began: SystemTime,
    /// What to say once the wait has lasted `WAIT_DELAY`, from the agent's latest load.
    announcement: Announcement,
}

impl Transitions {
    /// Record this load and return what to announce at `now`: the units that changed since the
    /// previous load, and the waits that have now lasted `WAIT_DELAY`. Each agent comes with the
    /// name of the project it works in.
    pub(super) fn advance<'a>(
        &mut self,
        envs: impl IntoIterator<Item = &'a Environment>,
        agents: impl IntoIterator<Item = (&'a str, &'a AgentSession)>,
        now: SystemTime,
    ) -> Vec<Announcement> {
        let mut announcements = Vec::new();

        let mut units = HashMap::new();
        for env in envs {
            for unit in &env.units {
                let key = (env.id.clone(), unit.name.clone());
                // A unit that was not there before counts as changed.
                let changed = self
                    .units
                    .get(&key)
                    .is_none_or(|old| !state_variant_eq(old, &unit.state));
                if self.primed && changed {
                    announcements.extend(unit_announcement(&env.id, &unit.name, &unit.state));
                }
                units.insert(key, unit.state.clone());
            }
        }

        let mut statuses = HashMap::new();
        // Only agents still waiting keep their wait: one answered, or gone, is never announced.
        let mut waits = HashMap::new();
        for (project, agent) in agents {
            let key = (agent.harness, agent.session_id.clone());
            if agent.status == AgentStatus::Waiting {
                let began = match self.waits.remove(&key) {
                    Some(wait) => Some(wait.began),
                    None if self.primed && self.agents.get(&key) != Some(&AgentStatus::Waiting) => {
                        Some(agent.since.unwrap_or(now))
                    }
                    // Waiting already at the previous load, and announced or recorded then.
                    None => None,
                };
                if let Some(began) = began {
                    let announcement = agent_announcement(project, agent);
                    waits.insert(
                        key.clone(),
                        Wait {
                            began,
                            announcement,
                        },
                    );
                }
            }
            statuses.insert(key, agent.status);
        }

        self.units = units;
        self.agents = statuses;
        self.waits = waits;
        self.primed = true;
        announcements.extend(self.lasting_waits(now));
        announcements
    }

    /// Take the waits that have lasted `WAIT_DELAY` by `now`, longest first, so each is announced
    /// once.
    pub(super) fn lasting_waits(&mut self, now: SystemTime) -> Vec<Announcement> {
        let lasted = |wait: &Wait| {
            now.duration_since(wait.began)
                .is_ok_and(|waited| waited >= WAIT_DELAY)
        };
        let (mut due, pending): (Vec<_>, Vec<_>) = mem::take(&mut self.waits)
            .into_iter()
            .partition(|(_, wait)| lasted(wait));
        self.waits = pending.into_iter().collect();
        due.sort_by_key(|(_, wait)| wait.began);
        due.into_iter().map(|(_, wait)| wait.announcement).collect()
    }

    /// Whether a wait is still to be announced, should it last.
    pub(super) fn has_pending_waits(&self) -> bool {
        !self.waits.is_empty()
    }

    /// Drop the waits not yet announced, keeping what the last load recorded of each agent.
    pub(super) fn forget_pending_waits(&mut self) {
        self.waits.clear();
    }
}

/// What to say when a unit enters `state`, or `None` for a state not worth a sound.
fn unit_announcement(env_id: &str, unit_name: &str, state: &State) -> Option<Announcement> {
    let sound = match state {
        State::None | State::Other(_) | State::Stopped => return None,
        State::Building | State::Starting => Sound::Submarine,
        State::Ready | State::Running => Sound::Ping,
        State::Failed => Sound::Basso,
    };
    Some(Announcement {
        sound,
        speech: format!("{unit_name} {state}"),
        banner_title: format!("Sutra · {unit_name}"),
        banner_body: state.to_string(),
        unit_key: Some(unit_key(env_id, unit_name)),
    })
}

/// What to say when an agent in `project` has waited on the user long enough.
fn agent_announcement(project: &str, agent: &AgentSession) -> Announcement {
    let harness = agent.harness;
    let banner_body = match agent.waiting_reason() {
        Some(reason) => format!("{harness} is waiting: {reason}"),
        None => format!("{harness} is waiting"),
    };
    Announcement {
        sound: Sound::Glass,
        speech: format!("{project} {harness} waiting"),
        banner_title: format!("Sutra · {project}"),
        banner_body,
        unit_key: None,
    }
}

/// Compare two State values by variant, also comparing the inner string for Other.
fn state_variant_eq(a: &State, b: &State) -> bool {
    match (a, b) {
        (State::Other(a), State::Other(b)) => a == b,
        _ => mem::discriminant(a) == mem::discriminant(b),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::test_support::{agent_session, at, environment_at};
    use crate::model::UnitStatus;

    fn t0() -> SystemTime {
        at(1_000_000)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn env(id: &str, units: &[(&str, &str)]) -> Environment {
        Environment {
            id: id.to_string(),
            units: units
                .iter()
                .map(|(name, status)| UnitStatus::parse(name, status))
                .collect(),
            ..environment_at(PathBuf::from("/code").join(id))
        }
    }

    /// A Claude session in `status`; a waiting one began waiting at `since`.
    fn agent(session_id: &str, status: AgentStatus, since: Option<SystemTime>) -> AgentSession {
        AgentSession {
            waiting_for: (status == AgentStatus::Waiting).then(|| "permission_prompt".to_string()),
            since,
            ..agent_session(session_id, status)
        }
    }

    fn speeches(announcements: &[Announcement]) -> Vec<&str> {
        announcements.iter().map(|a| a.speech.as_str()).collect()
    }

    /// Transitions that have recorded one load in which `session` was busy.
    fn primed_with_busy(session: &str) -> Transitions {
        let mut transitions = Transitions::default();
        let busy = agent(session, AgentStatus::Busy, None);
        transitions.advance([], [("sutra", &busy)], t0());
        transitions
    }

    #[test]
    fn the_first_load_records_without_announcing() {
        let mut transitions = Transitions::default();
        let waiting = agent("s1", AgentStatus::Waiting, Some(t0() - secs(60)));
        let first = transitions.advance(
            &[env("app", &[("server", "failed")])],
            [("app", &waiting)],
            t0(),
        );
        assert!(first.is_empty());
        assert!(transitions.primed);
        assert!(!transitions.has_pending_waits());
        assert!(transitions.lasting_waits(t0() + secs(60)).is_empty());
    }

    #[test]
    fn a_unit_changing_state_is_announced_at_once_and_an_unchanged_one_is_not() {
        let mut transitions = Transitions::default();
        transitions.advance(
            &[env("app", &[("server", "building"), ("web", "ready")])],
            [],
            t0(),
        );
        let next = transitions.advance(
            &[env("app", &[("server", "ready"), ("web", "ready")])],
            [],
            t0(),
        );
        assert_eq!(speeches(&next), ["server ready"]);
        assert_eq!(next[0].sound, Sound::Ping);
        assert_eq!(next[0].banner_title, "Sutra · server");
        assert_eq!(next[0].unit_key.as_deref(), Some("app\x00server"));
    }

    #[test]
    fn a_unit_that_appears_stopped_makes_no_sound() {
        let mut transitions = Transitions::default();
        transitions.advance(&[env("app", &[])], [], t0());
        let next = transitions.advance(&[env("app", &[("server", "stopped")])], [], t0());
        assert!(next.is_empty());
    }

    #[test]
    fn a_wait_is_announced_once_it_has_lasted_wait_delay_with_its_project_and_reason() {
        let mut transitions = primed_with_busy("s1");
        let waiting = agent("s1", AgentStatus::Waiting, Some(t0()));

        assert!(
            transitions
                .advance([], [("sutra", &waiting)], t0())
                .is_empty()
        );
        assert!(transitions.has_pending_waits());
        assert!(
            transitions
                .lasting_waits(t0() + WAIT_DELAY - secs(1))
                .is_empty()
        );
        assert_eq!(
            transitions.lasting_waits(t0() + WAIT_DELAY),
            [Announcement {
                sound: Sound::Glass,
                speech: "sutra Claude waiting".to_string(),
                banner_title: "Sutra · sutra".to_string(),
                banner_body: "Claude is waiting: permission prompt".to_string(),
                unit_key: None,
            }]
        );
        assert!(!transitions.has_pending_waits());
    }

    #[test]
    fn a_wait_answered_sooner_is_never_announced() {
        let mut transitions = primed_with_busy("s1");
        let waiting = agent("s1", AgentStatus::Waiting, Some(t0()));
        let answered = agent("s1", AgentStatus::Busy, None);

        transitions.advance([], [("sutra", &waiting)], t0());
        assert!(
            transitions
                .advance([], [("sutra", &answered)], t0() + secs(4))
                .is_empty()
        );
        assert!(!transitions.has_pending_waits());
        assert!(transitions.lasting_waits(t0() + secs(60)).is_empty());
    }

    #[test]
    fn a_wait_forgotten_once_the_map_stops_updating_is_never_announced() {
        let mut transitions = primed_with_busy("s1");
        let waiting = agent("s1", AgentStatus::Waiting, Some(t0()));
        assert!(
            transitions
                .advance([], [("sutra", &waiting)], t0())
                .is_empty()
        );
        assert!(transitions.has_pending_waits());

        transitions.forget_pending_waits();
        assert!(!transitions.has_pending_waits());
        assert!(transitions.lasting_waits(t0() + WAIT_DELAY).is_empty());
    }

    #[test]
    fn a_wait_whose_agent_is_gone_is_never_announced() {
        let mut transitions = primed_with_busy("s1");
        let waiting = agent("s1", AgentStatus::Waiting, Some(t0()));
        transitions.advance([], [("sutra", &waiting)], t0());
        transitions.advance([], [], t0() + secs(2));
        assert!(transitions.lasting_waits(t0() + secs(60)).is_empty());
    }

    #[test]
    fn a_wait_seen_after_it_lasted_is_announced_with_the_load_that_shows_it() {
        let mut transitions = primed_with_busy("s1");
        let long_waiting = agent("s1", AgentStatus::Waiting, Some(t0() - secs(30)));
        let next = transitions.advance([], [("sutra", &long_waiting)], t0());
        assert_eq!(speeches(&next), ["sutra Claude waiting"]);
    }

    #[test]
    fn a_wait_with_no_recorded_start_counts_from_the_load_that_first_shows_it() {
        let mut transitions = primed_with_busy("s1");
        let waiting = agent("s1", AgentStatus::Waiting, None);
        transitions.advance([], [("sutra", &waiting)], t0());
        // Still waiting at the next load, which keeps the wait's start.
        transitions.advance([], [("sutra", &waiting)], t0() + secs(5));
        assert!(transitions.lasting_waits(t0() + secs(9)).is_empty());
        assert_eq!(transitions.lasting_waits(t0() + secs(10)).len(), 1);
    }

    #[test]
    fn an_agent_still_waiting_is_not_announced_again() {
        let mut transitions = primed_with_busy("s1");
        let waiting = agent("s1", AgentStatus::Waiting, Some(t0() - secs(30)));
        assert_eq!(
            transitions.advance([], [("sutra", &waiting)], t0()).len(),
            1
        );
        assert!(
            transitions
                .advance([], [("sutra", &waiting)], t0() + secs(2))
                .is_empty()
        );
        assert!(!transitions.has_pending_waits());
    }

    #[test]
    fn waits_that_last_together_are_announced_longest_first() {
        let mut transitions = Transitions::default();
        let busy = [
            agent("short", AgentStatus::Busy, None),
            agent("long", AgentStatus::Busy, None),
        ];
        transitions.advance([], busy.iter().map(|a| ("app", a)), t0());
        let short = agent("short", AgentStatus::Waiting, Some(t0() + secs(3)));
        let long = agent("long", AgentStatus::Waiting, Some(t0()));
        // Loaded shortest first, so only the announcement order can put the long one first.
        transitions.advance([], [("short", &short), ("long", &long)], t0() + secs(3));
        assert_eq!(
            speeches(&transitions.lasting_waits(t0() + secs(20))),
            ["long Claude waiting", "short Claude waiting"]
        );
    }

    #[test]
    fn an_agent_turning_busy_or_idle_is_not_announced() {
        let mut transitions = Transitions::default();
        let waiting = agent("s1", AgentStatus::Waiting, Some(t0()));
        let busy = agent("s1", AgentStatus::Busy, None);
        let idle = agent("s1", AgentStatus::Idle, None);
        transitions.advance([], [("app", &waiting)], t0());
        assert!(transitions.advance([], [("app", &busy)], t0()).is_empty());
        assert!(transitions.advance([], [("app", &idle)], t0()).is_empty());
    }
}
