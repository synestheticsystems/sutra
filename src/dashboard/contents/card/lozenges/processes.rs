//! The ports a project's processes listen on, a lozenge each, as `(node :5177)` reads: up, as the
//! port listens, and named by the first process listening on it. Its tooltip says first who can
//! reach the port, this Mac alone or the network, as a server one thought private may not be; then
//! each process listening on it: its name and pid, its command, and the folder it runs in.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::Path;

use super::{Lozenge, LozengeLook, link, reach};
use crate::dashboard::format;
use crate::map::processes::ListeningProcess;

/// How much of a command a tooltip shows: enough to tell one server from another, where a
/// browser's command runs to thousands of characters.
const COMMAND_SHOWN: usize = 200;

/// A lozenge for each port `listening` listen on but those in `shown`, in port order.
pub(super) fn lozenges(
    listening: &[ListeningProcess],
    shown: &BTreeSet<u16>,
    home: &Path,
) -> Vec<Lozenge> {
    let ports: BTreeSet<u16> = listening.iter().flat_map(ListeningProcess::ports).collect();
    let ports = ports.difference(shown);
    ports.map(|&port| lozenge(port, listening, home)).collect()
}

/// The lozenge of `port`, which some of `listening` listen on.
fn lozenge(port: u16, listening: &[ListeningProcess], home: &Path) -> Lozenge {
    let on_port = |process: &&ListeningProcess| process.listens_on(port);
    let processes: Vec<&ListeningProcess> = listening.iter().filter(on_port).collect();
    let addresses: BTreeSet<IpAddr> = processes
        .iter()
        .flat_map(|process| &process.sockets)
        .filter(|socket| socket.port() == port)
        .map(|socket| socket.ip())
        .collect();
    let mut tooltip = vec![reach(&addresses)];
    for process in &processes {
        tooltip.push(format!("{}, pid {}", process.name, process.pid));
        tooltip.extend(process.command.as_deref().map(|run| shortened(run, home)));
        tooltip.push(format::tilde(&process.folder, home));
    }
    Lozenge {
        name: processes[0].name.clone(),
        port: Some(format!(":{port}")),
        look: LozengeLook::Up,
        beside: None,
        muted: false,
        notifications_off: false,
        tooltip: tooltip.join("\n"),
        link: Some(link(port)),
    }
}

/// `command` as a tooltip shows it: the home folder written `~`, and cut short past
/// `COMMAND_SHOWN` characters.
fn shortened(command: &str, home: &Path) -> String {
    let command = command.replace(&format!("{}/", home.display()), "~/");
    match command.char_indices().nth(COMMAND_SHOWN) {
        Some((cut, _)) => format!("{}\u{2026}", &command[..cut]),
        None => command,
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv6Addr, SocketAddr};

    use super::*;
    use crate::map::test_support::listening_in;

    const HOME: &str = "/Users/dev";

    /// `process`, listening at `addresses` on `port` in place of where it listened.
    fn at(addresses: &[IpAddr], port: u16, process: ListeningProcess) -> ListeningProcess {
        let sockets = addresses.iter().map(|&ip| SocketAddr::new(ip, port));
        ListeningProcess {
            sockets: sockets.collect(),
            ..process
        }
    }

    #[test]
    fn a_port_shows_as_up_by_its_process_name_opens_in_the_browser_and_says_who_listens() {
        let vite = ListeningProcess {
            command: Some("node /Users/dev/site/node_modules/vite/bin/vite.js --port 5177".into()),
            ..listening_in("/Users/dev/site", "node", &[5177])
        };

        let [lozenge] = lozenges(&[vite], &BTreeSet::new(), Path::new(HOME))
            .try_into()
            .unwrap();

        assert_eq!(lozenge.name, "node");
        assert_eq!(lozenge.port.as_deref(), Some(":5177"));
        assert_eq!(lozenge.look, LozengeLook::Up);
        assert_eq!(lozenge.link.as_deref(), Some("http://localhost:5177"));
        assert_eq!(
            lozenge.tooltip,
            "Listening on this Mac only, at 127.0.0.1\n\
             node, pid 4242\n\
             node ~/site/node_modules/vite/bin/vite.js --port 5177\n\
             ~/site"
        );
    }

    #[test]
    fn processes_on_one_port_share_its_lozenge_and_a_port_shown_already_has_none() {
        let pids = |pid| ListeningProcess {
            pid,
            ..listening_in("/code/app", "gunicorn", &[8000, 9000])
        };
        let workers = [
            pids(100),
            at(&[Ipv6Addr::LOCALHOST.into()], 8000, pids(101)),
        ];
        let shown = BTreeSet::from([9000]);

        let lozenges = lozenges(&workers, &shown, Path::new(HOME));

        let [lozenge] = lozenges.as_slice() else {
            panic!("one lozenge for :8000 alone: {lozenges:?}");
        };
        assert_eq!(lozenge.port.as_deref(), Some(":8000"));
        let tooltip: Vec<&str> = lozenge.tooltip.lines().collect();
        assert_eq!(
            tooltip[0],
            "Listening on this Mac only, at 127.0.0.1 and ::1"
        );
        assert_eq!(
            tooltip
                .iter()
                .filter(|line| line.contains(", pid 10"))
                .count(),
            2
        );
    }

    #[test]
    fn a_long_command_is_cut_short_and_writes_the_home_folder_as_a_tilde() {
        let long = format!(
            "/Users/dev/chrome --user-data-dir=/Users/dev/p {}",
            "x".repeat(300)
        );
        let shown = shortened(&long, Path::new(HOME));
        assert!(
            shown.starts_with("~/chrome --user-data-dir=~/p xx"),
            "{shown}"
        );
        assert_eq!(shown.chars().count(), COMMAND_SHOWN + 1);
        assert!(shown.ends_with('\u{2026}'));
        assert_eq!(shortened("cargo run", Path::new(HOME)), "cargo run");
    }
}
