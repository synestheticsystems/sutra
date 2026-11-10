//! A project's dev-runner units, one lozenge each, as `(web :3000) (api :8787) Compiling` reads.
//! The look says the state; a unit with a port opens in the browser on a click. Beside a lozenge
//! stands what a unit in trouble or under way says of itself, and a mark for each announcement
//! switched off; the words for the rest are in its tooltip.

use super::{Lozenge, LozengeLook, link};
use crate::model::{Environment, State, UnitStatus};
use crate::notifications::Notifier;

/// The lozenges of `env`'s units, in the order it lists them.
pub(super) fn lozenges(env: &Environment, notifier: &Notifier) -> Vec<Lozenge> {
    let stopped = env.has_stopped();
    env.units
        .iter()
        .map(|unit| {
            let port = env.port_for(&unit.name);
            Lozenge {
                name: Some(unit.name.clone()),
                port: port.map(|port| format!(":{port}")),
                look: look(&unit.state, stopped),
                beside: unit.detail.clone().filter(|_| worth_saying(&unit.state)),
                muted: notifier.is_unit_muted(&env.id, &unit.name),
                notifications_off: notifier.is_unit_notifications_off(&env.id, &unit.name),
                tooltip: unit_tooltip(unit, stopped.then_some(env.pid)),
                link: port.map(link),
            }
        })
        .collect()
}

/// Whether what a unit says of itself belongs on the card: it is under way or in trouble. A unit
/// that is up has nothing the user needs at a glance.
fn worth_saying(state: &State) -> bool {
    matches!(state, State::Building | State::Starting | State::Failed)
}

/// Up, on the way, failed, or still for the rest. A stopped environment's process is gone, so what
/// it last published is no longer news, and only a failure, which may say why it stopped, keeps
/// its look.
fn look(state: &State, environment_stopped: bool) -> LozengeLook {
    match state {
        State::Failed => LozengeLook::Failed,
        _ if environment_stopped => LozengeLook::Still,
        State::Running | State::Ready => LozengeLook::Up,
        State::Building | State::Starting => LozengeLook::Starting,
        State::Stopped | State::None | State::Other(_) => LozengeLook::Still,
    }
}

/// A unit's state word: what its status file says, or `no status` for an empty one.
fn unit_state(state: &State) -> String {
    match state {
        State::None => "no status".to_string(),
        state => state.to_string(),
    }
}

/// A unit's tooltip: its state and detail in words, which the lozenge says in its look. When the
/// process running its environment has gone, the state is only what it last said.
fn unit_tooltip(unit: &UnitStatus, stopped_process: Option<u32>) -> String {
    let state = unit_state(&unit.state);
    let mut words = match stopped_process {
        Some(pid) => format!("Last said {state}; process {pid} is not running"),
        None => state,
    };
    if let Some(detail) = &unit.detail {
        words.push('\n');
        words.push_str(detail);
    }
    words
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::map::test_support::{DEAD_PID, environment_at};

    #[test]
    fn a_stopped_environments_last_states_look_still_except_a_failure() {
        assert_eq!(look(&State::Ready, false), LozengeLook::Up);
        assert_eq!(look(&State::Ready, true), LozengeLook::Still);
        assert_eq!(look(&State::Building, false), LozengeLook::Starting);
        assert_eq!(look(&State::Building, true), LozengeLook::Still);
        assert_eq!(look(&State::Failed, true), LozengeLook::Failed);
    }

    #[test]
    fn only_a_unit_under_way_or_in_trouble_says_more_on_the_card() {
        assert!(worth_saying(&State::Building));
        assert!(worth_saying(&State::Failed));
        assert!(!worth_saying(&State::Ready));
        assert!(!worth_saying(&State::Stopped));
    }

    #[test]
    fn a_units_tooltip_says_its_state_and_detail_and_that_a_stopped_one_is_old_news() {
        let unit = UnitStatus {
            name: "daemon".to_string(),
            state: State::Ready,
            detail: Some("9 projects".to_string()),
        };
        assert_eq!(unit_tooltip(&unit, None), "ready\n9 projects");
        assert_eq!(
            unit_tooltip(&unit, Some(4242)),
            "Last said ready; process 4242 is not running\n9 projects"
        );
    }

    #[test]
    fn an_empty_unit_state_reads_no_status() {
        assert_eq!(unit_state(&State::None), "no status");
        assert_eq!(unit_state(&State::Ready), "ready");
        assert_eq!(unit_state(&State::Other("paused".to_string())), "paused");
    }

    #[test]
    fn a_unit_with_a_port_names_it_and_opens_it_and_one_switched_off_says_so() {
        let env = Environment {
            id: "env".to_string(),
            pid: DEAD_PID,
            units: vec![
                UnitStatus::parse("web", "building: Compiling"),
                UnitStatus::parse("db", "ready: 9 tables"),
            ],
            ports: HashMap::from([("web".to_string(), 3000)]),
            ..environment_at("/code/app")
        };
        let mut notifier = Notifier::new(|_, _| {});
        notifier.toggle_unit_mute("env", "db");

        let lozenges = lozenges(&env, &notifier);

        assert_eq!(lozenges[0].port.as_deref(), Some(":3000"));
        assert_eq!(lozenges[0].link.as_deref(), Some("http://localhost:3000"));
        assert_eq!(lozenges[0].look, LozengeLook::Still, "its process is gone");
        assert_eq!(lozenges[0].beside.as_deref(), Some("Compiling"));
        assert_eq!(lozenges[1].port, None);
        assert_eq!(lozenges[1].beside, None, "a unit that is up says no more");
        assert!(lozenges[1].muted && !lozenges[1].notifications_off);
    }
}
