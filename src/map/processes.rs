//! The TCP ports the user's own processes listen on, each with the folder its process runs in, read
//! as `lsof -p` reads them (see `libproc`). The map places each process on the card of the project
//! its folder lies in, so a dev server started in a worktree shows there whether or not a dev
//! script says so.
//!
//! Reading a socket costs about a microsecond: on Daniel's Mac on 2026-11-07, with 1,311 processes
//! holding about 3,000 sockets, a pass over every socket took 3.2 to 4.1 ms, and one over every
//! process's folder 0.9 ms. So a read takes each process's folder first, and the sockets only of
//! the processes whose folder the map wants: there, with 891 worktrees on the map, a read took 3.0
//! to 3.6 ms. Only the direct edition reads them: inside the App Sandbox macOS tells an app nothing
//! of other apps' processes.

#[cfg(target_os = "macos")]
mod libproc;

use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use libproc::ProcessId;

/// One of the user's processes listening for TCP connections, in the folder it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListeningProcess {
    pub pid: u32,
    /// What macOS calls the process, as Activity Monitor names it: `node`, `postgres`.
    pub name: String,
    /// The command it runs, its arguments joined by spaces as `ps` shows them; `None` when macOS
    /// would not say.
    pub command: Option<String>,
    /// The folder it runs in: its working directory.
    pub folder: PathBuf,
    /// Where it listens, each address and port once, by address and then by port.
    pub sockets: BTreeSet<SocketAddr>,
}

impl ListeningProcess {
    /// The ports it listens on, each once, in order.
    pub fn ports(&self) -> BTreeSet<u16> {
        self.sockets.iter().map(SocketAddr::port).collect()
    }

    /// Whether it listens on `port`, at any address.
    pub fn listens_on(&self, port: u16) -> bool {
        self.sockets.iter().any(|socket| socket.port() == port)
    }
}

/// The command of each process the last pass found listening, kept to the next pass, so that a
/// process is asked for its arguments once in its life rather than at every pass: the answer is a
/// copy of all of them, and of its environment.
#[derive(Debug, Default)]
pub struct LastRead {
    #[cfg(target_os = "macos")]
    commands: HashMap<ProcessId, Option<String>>,
}

/// The user's processes that listen on a TCP port, of those whose folder `wanted` accepts, in the
/// order macOS lists them. Fails only when macOS will not list the user's processes; one that
/// ends while it is read, or will not say what it holds, is left out.
#[cfg(target_os = "macos")]
pub fn listening(
    wanted: impl Fn(&Path) -> bool,
    last_read: &mut LastRead,
) -> io::Result<Vec<ListeningProcess>> {
    let mut room = libproc::Descriptors::default();
    let mut commands = HashMap::new();
    let mut found = Vec::new();
    for pid in libproc::processes_of_this_user()? {
        let Some(folder) = libproc::folder_of(pid).filter(|folder| wanted(folder)) else {
            continue;
        };
        let sockets = libproc::listening_sockets(pid, &mut room);
        if sockets.is_empty() {
            continue;
        }
        let Some((name, id)) = libproc::name_and_id(pid) else {
            continue;
        };
        let command = match last_read.commands.remove(&id) {
            Some(command) => command,
            None => libproc::command_of(pid),
        };
        commands.insert(id, command.clone());
        found.push(ListeningProcess {
            pid,
            name,
            command,
            folder,
            sockets,
        });
    }
    // Only this pass's listeners are kept, so the commands never outnumber them.
    last_read.commands = commands;
    Ok(found)
}

/// Outside macOS no libproc tells of other processes' sockets, so none is found listening.
#[cfg(not(target_os = "macos"))]
pub fn listening(
    _wanted: impl Fn(&Path) -> bool,
    _last_read: &mut LastRead,
) -> io::Result<Vec<ListeningProcess>> {
    Ok(Vec::new())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener};

    use super::*;

    /// This test process, as a pass that wants only the folder it runs in finds it.
    fn this_process(last_read: &mut LastRead) -> Option<ListeningProcess> {
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();
        let me = std::process::id();
        let found = listening(|folder| folder == here, last_read).unwrap();
        found.into_iter().find(|process| process.pid == me)
    }

    #[test]
    fn a_process_listening_in_a_wanted_folder_is_found_with_its_addresses_name_and_command() {
        let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let everywhere = TcpListener::bind((Ipv6Addr::UNSPECIFIED, 0)).unwrap();

        let found = this_process(&mut LastRead::default()).expect("this test process listens");

        let local = local.local_addr().unwrap();
        let everywhere = everywhere.local_addr().unwrap();
        assert!(found.sockets.contains(&local), "{:?}", found.sockets);
        assert!(found.sockets.contains(&everywhere), "{:?}", found.sockets);
        assert_eq!(
            found.folder,
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
        let binary = std::env::current_exe().unwrap();
        let stem = binary.file_name().unwrap().to_string_lossy();
        assert!(stem.starts_with(&found.name), "{} names {stem}", found.name);
        let command = found
            .command
            .expect("macOS tells a process's own user its command");
        assert!(command.contains(&*stem), "{command}");
    }

    #[test]
    fn a_process_in_a_folder_not_wanted_is_left_out_and_a_closed_socket_with_it() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let me = std::process::id();

        let elsewhere = listening(|_| false, &mut LastRead::default()).unwrap();
        assert!(elsewhere.iter().all(|process| process.pid != me));

        drop(listener);
        let found = this_process(&mut LastRead::default());
        let still = found.is_some_and(|process| process.sockets.contains(&address));
        assert!(!still, "{address} no longer listens");
    }

    #[test]
    fn a_listeners_command_is_asked_for_once_and_forgotten_once_a_pass_does_not_find_it() {
        let _listener = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
        let (_, me) = libproc::name_and_id(std::process::id()).unwrap();
        let mut last_read = LastRead::default();
        last_read
            .commands
            .insert(me, Some("as the last pass found it".to_string()));

        let found = this_process(&mut last_read).unwrap();

        assert_eq!(found.command.as_deref(), Some("as the last pass found it"));
        listening(|_| false, &mut last_read).unwrap();
        assert!(last_read.commands.is_empty(), "nothing was found listening");
    }
}
