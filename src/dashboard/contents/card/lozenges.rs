//! What serves in a project, a lozenge each, as `(web :3000) (node :5177) (postgres :36684)` reads:
//! its dev-runner units, as its environment lists them (see `units`), then the ports its processes
//! listen on (see `processes`), then those its Docker containers publish (see `containers`). Each
//! port shows once, the first time: a port a unit shows is not shown again, as the unit says more.
//! The look says the state, and a lozenge with a port opens it in the browser on a click. A group's
//! folded line shows the lozenges of the cards it holds, so whatever serves in a card shows there
//! too.

mod containers;
mod processes;
mod units;

use std::collections::BTreeSet;
use std::net::IpAddr;

use super::CardContext;
use crate::dashboard::format;
use crate::map::Project;

/// One lozenge, as `(web :3000)` reads: a dev-runner unit, a port a process listens on, or a Docker
/// container, by a port it publishes. Its name keeps the word from when every lozenge was a unit's.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UnitLozenge {
    /// The unit's name, the process's, or the short name of the image the container runs.
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
    /// Up: green, as a unit that is up, a port that is listening and a running container are.
    Up,
    /// On the way: blue, so amber stays for what needs the user.
    Starting,
    /// Failed: red.
    Failed,
    /// Stopped, unknown, or its environment's process has gone: grey.
    Still,
}

/// The lozenges of `project`: its units, then the ports its processes listen on, in port order,
/// then its containers, by name, each port once.
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
    shown.extend(project.listening.iter().flat_map(|process| process.ports()));
    let containers = &project.containers;
    lozenges.extend(containers::lozenges(
        containers,
        &project.path,
        &shown,
        ctx.home,
    ));
    lozenges
}

/// What opens `port` in the browser.
fn link(port: u16) -> String {
    format!("http://localhost:{port}")
}

/// Who can reach a port listened on at `addresses`: this Mac alone, while every one of them is a
/// loopback address, or else the network.
fn reach(addresses: &BTreeSet<IpAddr>) -> String {
    let words: Vec<String> = addresses.iter().map(IpAddr::to_string).collect();
    let at = format::all(&words.iter().map(String::as_str).collect::<Vec<_>>());
    if addresses.iter().all(|ip| ip.to_canonical().is_loopback()) {
        format!("Listening on this Mac only, at {at}")
    } else {
        format!("Listening on the network, at {at}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::dashboard::listing::fixtures::now;
    use crate::map::docker::{Container, PublishedPort};
    use crate::map::test_support::{environment_at, listening_in, project_at};
    use crate::model::{Environment, UnitStatus};
    use crate::notifications::Notifier;

    #[test]
    fn a_port_any_address_beyond_loopback_listens_on_reaches_the_network() {
        let loopback = [Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()];
        let mapped = [Ipv4Addr::LOCALHOST.to_ipv6_mapped().into()];
        let everywhere = [Ipv4Addr::LOCALHOST.into(), Ipv6Addr::UNSPECIFIED.into()];
        let reach = |addresses: &[IpAddr]| {
            let set: BTreeSet<IpAddr> = addresses.iter().copied().collect();
            reach(&set)
        };
        assert_eq!(
            reach(&loopback),
            "Listening on this Mac only, at 127.0.0.1 and ::1"
        );
        assert_eq!(
            reach(&mapped),
            "Listening on this Mac only, at ::ffff:127.0.0.1"
        );
        assert_eq!(
            reach(&everywhere),
            "Listening on the network, at 127.0.0.1 and ::"
        );
    }

    #[test]
    fn a_card_shows_its_units_then_its_processes_then_its_containers_and_each_port_only_once() {
        let published = |port| PublishedPort {
            port,
            to: 5432,
            addresses: BTreeSet::from([IpAddr::from(Ipv4Addr::LOCALHOST)]),
            udp: false,
        };
        let postgres = Container {
            id: "e99d".to_string(),
            name: "app-db-1".to_string(),
            image: "postgres:17".to_string(),
            compose_folder: Some(PathBuf::from("/code/app")),
            bound: Vec::new(),
            ports: vec![published(24678), published(55432)],
            created: SystemTime::UNIX_EPOCH,
        };
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
            containers: vec![postgres],
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
                lozenge("node", ":24678"),
                lozenge("postgres", ":55432"),
            ],
            "web's unit says more than node does of :5173, and node shows :24678 before postgres; \
             api has a port but no unit to show it"
        );
    }
}
