//! What serves in a project, a lozenge each, as `(web :3000) (node :5177)` reads: its dev-runner
//! units, as its environment lists them (see `units`), then the ports its processes listen on (see
//! `processes`). Each port shows once: a port a unit shows is not shown again, as the unit says
//! more. The look says the state, and a lozenge with a port opens it in the browser on a click. A
//! group's folded line shows the lozenges of the cards it holds, so whatever serves in a card
//! shows there too.

mod processes;
mod units;

use std::collections::BTreeSet;

use super::CardContext;
use crate::map::Project;

/// One lozenge, as `(web :3000)` reads: a dev-runner unit, or a port a process listens on. Its name
/// keeps the word from when every lozenge was a unit's.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UnitLozenge {
    /// The unit's name, or the process's.
    pub name: String,
    /// The port as the window draws it after the name, colon included: `:3000`.
    pub port: Option<String>,
    pub look: UnitLook,
    /// What a unit under way or in trouble says of itself.
    pub beside: Option<String>,
    /// The unit's announcements are muted. A process makes none.
    pub muted: bool,
    /// The unit's banners are off. A process makes none.
    pub notifications_off: bool,
    pub tooltip: String,
    /// Opens the port in the browser, on a click.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnitLook {
    /// Up: green, as a unit that is up and a port that is listening are.
    Up,
    /// On the way: blue, so amber stays for what needs the user.
    Starting,
    /// Failed: red.
    Failed,
    /// Stopped, unknown, or its environment's process has gone: grey.
    Still,
}

/// The lozenges of `project`: its units, then the ports its processes listen on, in port order,
/// each port once.
pub(in crate::dashboard::contents) fn lozenges(
    project: &Project,
    ctx: &CardContext,
) -> Vec<UnitLozenge> {
    let mut shown = BTreeSet::new();
    let mut lozenges = Vec::new();
    if let Some(env) = &project.environment {
        shown.extend(env.units.iter().filter_map(|unit| env.port_for(&unit.name)));
        lozenges.extend(units::lozenges(env, ctx.notifier));
    }
    lozenges.extend(processes::lozenges(&project.listening, &shown, ctx.home));
    lozenges
}

/// What opens `port` in the browser.
fn link(port: u16) -> String {
    format!("http://localhost:{port}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::now;
    use crate::map::test_support::{environment_at, listening_in, project_at};
    use crate::model::{Environment, UnitStatus};
    use crate::notifications::Notifier;

    #[test]
    fn a_card_shows_its_units_then_its_processes_ports_and_a_port_a_unit_shows_only_once() {
        let app = Project {
            environment: Some(Environment {
                units: vec![UnitStatus::parse("web", "ready")],
                ports: HashMap::from([("web".to_string(), 5173), ("api".to_string(), 8787)]),
                ..environment_at("/code/app")
            }),
            listening: vec![
                listening_in("/code/app", "node", &[5173, 24678]),
                listening_in("/code/app/api", "node", &[8787]),
            ],
            ..project_at("/code/app")
        };
        let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
        let ctx = CardContext {
            notifier: &notifier,
            cleaning: &cleaning,
            home: Path::new("/Users/dev"),
            now: now(),
        };

        let shown: Vec<(String, Option<String>)> = lozenges(&app, &ctx)
            .into_iter()
            .map(|lozenge| (lozenge.name, lozenge.port))
            .collect();

        let lozenge = |name: &str, port: &str| (name.to_string(), Some(port.to_string()));
        assert_eq!(
            shown,
            [
                lozenge("web", ":5173"),
                lozenge("node", ":8787"),
                lozenge("node", ":24678")
            ],
            "web's unit says more than node does of :5173; api has a port but no unit to show it"
        );
    }
}
