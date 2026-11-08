//! The calls into libproc that tell of the user's processes: which there are, the folder each runs
//! in, its name, its command and the TCP sockets it listens on, as `lsof -p` reads them. `libc`
//! binds most of it; the socket record it lacks is laid out here, as far as the fields read, as the
//! macOS SDK's `<sys/proc_info.h>` lays it out. Every field is a fixed size, so the layout is the
//! same on arm64 and x86_64. On 2026-11-07 a C probe compiled against the SDK (Xcode 26.6) printed
//! the sizes and offsets checked below, and a pass over the processes of Daniel's Mac found the 39
//! listening sockets `lsof -nP -iTCP -sTCP:LISTEN` listed, at the same addresses and ports.

use std::collections::BTreeSet;
use std::ffi::{CStr, OsStr, c_char, c_int};
use std::io;
use std::mem::{MaybeUninit, offset_of, size_of};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::OnceLock;

/// `proc_listpids` lists the processes whose effective user is the one given.
const PROC_UID_ONLY: u32 = 4;
/// The `proc_pidfdinfo` flavor that describes a socket, as a `socket_fdinfo`.
const PROC_PIDFDSOCKETINFO: c_int = 3;
/// `soi_kind` of a TCP socket, whose `soi_proto` is a `tcp_sockinfo`.
const SOCKINFO_TCP: c_int = 2;
/// `tcpsi_state` of a socket that listens for connections.
const TSI_S_LISTEN: c_int = 1;
/// The `insi_vflag` bit of an IPv6 socket, whose address is an `in6_addr`; without it the address
/// is IPv4, in the last four bytes. An IPv6 socket that takes IPv4 as well has both bits.
const INI_IPV6: u8 = 0x2;

/// `struct socket_fdinfo`, 792 bytes, read only as far as a TCP socket's state and local address.
#[repr(C)]
struct SocketFdInfo {
    /// `pfi`, a `proc_fileinfo`, and `psi` up to `soi_kind`.
    _before_kind: [u8; 256],
    /// `psi.soi_kind`: what `soi_proto` holds.
    kind: c_int,
    _rfu: u32,
    /// `psi.soi_proto`, a union of 528 bytes, as the `tcp_sockinfo` of a TCP socket.
    tcp: TcpSockInfo,
    _rest_of_proto: [u8; 528 - size_of::<TcpSockInfo>()],
}

/// `struct tcp_sockinfo`, 120 bytes: an `in_sockinfo` of 80, then the connection's state.
#[repr(C)]
struct TcpSockInfo {
    /// `insi_fport`.
    _foreign_port: c_int,
    /// `insi_lport`, the local port in network byte order.
    local_port: c_int,
    _before_vflag: [u8; 16],
    /// `insi_vflag`.
    vflag: u8,
    _before_laddr: [u8; 23],
    /// `insi_laddr`: an `in6_addr`, or an IPv4 address after twelve bytes of padding.
    local_address: [u8; 16],
    _after_laddr: [u8; 16],
    /// `tcpsi_state`.
    state: c_int,
    _rest: [u8; 36],
}

impl TcpSockInfo {
    fn local_port(&self) -> u16 {
        u16::from_be(self.local_port as u16)
    }

    fn local_ip(&self) -> IpAddr {
        if self.vflag & INI_IPV6 != 0 {
            IpAddr::V6(Ipv6Addr::from(self.local_address))
        } else {
            let [.., a, b, c, d] = self.local_address;
            IpAddr::V4(Ipv4Addr::new(a, b, c, d))
        }
    }
}

const _: () = assert!(size_of::<SocketFdInfo>() == 792);
const _: () = assert!(offset_of!(SocketFdInfo, kind) == 24 + 232);
const _: () = assert!(offset_of!(SocketFdInfo, tcp) == 24 + 240);
const _: () = assert!(size_of::<TcpSockInfo>() == 120);
const _: () = assert!(offset_of!(TcpSockInfo, local_port) == 4);
const _: () = assert!(offset_of!(TcpSockInfo, vflag) == 24);
const _: () = assert!(offset_of!(TcpSockInfo, local_address) == 48);
const _: () = assert!(offset_of!(TcpSockInfo, state) == 80);

/// A process as long as it runs: its pid, and when it started, which tells it from a process
/// given the same pid after it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ProcessId {
    pid: u32,
    started: (u64, u64),
}

impl ProcessId {
    pub(super) fn pid(self) -> u32 {
        self.pid
    }
}

/// The processes whose effective user is this process's. Fails only when macOS will not list them.
pub(super) fn processes_of_this_user() -> io::Result<Vec<u32>> {
    // SAFETY: `getuid` cannot fail, and a null buffer asks only for the room the list needs.
    let uid = unsafe { libc::getuid() };
    let needed = unsafe { libc::proc_listpids(PROC_UID_ONLY, uid, ptr::null_mut(), 0) };
    if needed <= 0 {
        return Err(io::Error::last_os_error());
    }
    // Room for a few more, as processes start between the two calls.
    let mut pids: Vec<libc::pid_t> = vec![0; needed as usize / size_of::<libc::pid_t>() + 64];
    let room = (pids.len() * size_of::<libc::pid_t>()) as c_int;
    // SAFETY: `pids` holds `room` bytes.
    let written =
        unsafe { libc::proc_listpids(PROC_UID_ONLY, uid, pids.as_mut_ptr().cast(), room) };
    if written <= 0 {
        return Err(io::Error::last_os_error());
    }
    pids.truncate(written as usize / size_of::<libc::pid_t>());
    Ok(pids
        .into_iter()
        .filter_map(|pid| u32::try_from(pid).ok().filter(|&pid| pid > 0))
        .collect())
}

/// The folder `pid` runs in, its working directory; `None` once it has ended, or when macOS will
/// not say.
pub(super) fn folder_of(pid: u32) -> Option<PathBuf> {
    let mut info = MaybeUninit::<libc::proc_vnodepathinfo>::uninit();
    let size = size_of::<libc::proc_vnodepathinfo>() as c_int;
    // SAFETY: `info` holds `size` bytes, and is read only once libproc has written them all.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    let path = text(info.pvi_cdir.vip_path.as_flattened())?;
    (!path.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(path)))
}

/// Room for the file descriptors of one process at a time, kept through a pass so that it grows to
/// the largest table once rather than being made again for each process.
pub(super) struct Descriptors(Vec<libc::proc_fdinfo>);

impl Default for Descriptors {
    fn default() -> Descriptors {
        Descriptors(Vec::with_capacity(256))
    }
}

/// The addresses and ports `pid` listens on for TCP connections, each once; empty once it has
/// ended, or when macOS will not say what it holds.
pub(super) fn listening_sockets(pid: u32, room: &mut Descriptors) -> BTreeSet<SocketAddr> {
    let Some(descriptors) = descriptors_of(pid, room) else {
        return BTreeSet::new();
    };
    descriptors
        .iter()
        .filter(|descriptor| descriptor.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32)
        .filter_map(|descriptor| listening_address(pid, descriptor.proc_fd))
        .collect()
}

/// The file descriptors `pid` holds, in `room`; `None` once it has ended, or when macOS will not
/// say.
fn descriptors_of(pid: u32, room: &mut Descriptors) -> Option<&[libc::proc_fdinfo]> {
    let fds = &mut room.0;
    loop {
        let bytes = (fds.capacity() * size_of::<libc::proc_fdinfo>()) as c_int;
        // SAFETY: `fds` has room for `bytes`, and holds as many records as libproc wrote.
        let written = unsafe {
            libc::proc_pidinfo(
                pid as c_int,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast(),
                bytes,
            )
        };
        if written <= 0 {
            return None;
        }
        let count = written as usize / size_of::<libc::proc_fdinfo>();
        // A full table may have been cut short, so it is listed again with twice the room.
        if count < fds.capacity() {
            unsafe { fds.set_len(count) };
            return Some(fds);
        }
        fds.reserve(fds.capacity());
    }
}

/// Where the socket `fd` of `pid` listens, when it is a TCP socket that listens.
fn listening_address(pid: u32, fd: i32) -> Option<SocketAddr> {
    let mut info = MaybeUninit::<SocketFdInfo>::uninit();
    let size = size_of::<SocketFdInfo>() as c_int;
    // SAFETY: `info` holds `size` bytes, and is read only once libproc has written them all.
    let written = unsafe {
        libc::proc_pidfdinfo(
            pid as c_int,
            fd,
            PROC_PIDFDSOCKETINFO,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    // Fewer bytes: the descriptor closed since it was listed, or holds something else now.
    if written != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    let tcp = &info.tcp;
    (info.kind == SOCKINFO_TCP && tcp.state == TSI_S_LISTEN)
        .then(|| SocketAddr::new(tcp.local_ip(), tcp.local_port()))
}

/// What macOS calls `pid` and when it started; `None` once it has ended.
pub(super) fn name_and_id(pid: u32) -> Option<(String, ProcessId)> {
    let info = bsd_info(pid)?;
    // The full name, which the kernel keeps when it is longer than the short one, or the short.
    let name = [info.pbi_name.as_slice(), info.pbi_comm.as_slice()]
        .into_iter()
        .filter_map(text)
        .find(|name| !name.is_empty())?;
    let id = ProcessId {
        pid,
        started: (info.pbi_start_tvsec, info.pbi_start_tvusec),
    };
    Some((String::from_utf8_lossy(name).into_owned(), id))
}

fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = size_of::<libc::proc_bsdinfo>() as c_int;
    // SAFETY: `info` holds `size` bytes, and is read only once libproc has written them all.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    (written == size).then(|| unsafe { info.assume_init() })
}

/// The command `pid` runs, its arguments joined by spaces as `ps` shows them; `None` once it has
/// ended, or when macOS will not say.
pub(super) fn command_of(pid: u32) -> Option<String> {
    let mut room = vec![0u8; arguments_room()?];
    let mut size = room.len();
    let mut name = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];
    // SAFETY: `room` holds `size` bytes, and `size` comes back as the bytes written.
    let failed = unsafe {
        libc::sysctl(
            name.as_mut_ptr(),
            name.len() as u32,
            room.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    } != 0;
    if failed {
        return None;
    }
    room.truncate(size);
    command_in(&room)
}

/// The room `KERN_PROCARGS2` needs, which a process's arguments and environment never exceed.
fn arguments_room() -> Option<usize> {
    static ROOM: OnceLock<Option<usize>> = OnceLock::new();
    *ROOM.get_or_init(|| {
        let mut room: c_int = 0;
        let mut size = size_of::<c_int>();
        let mut name = [libc::CTL_KERN, libc::KERN_ARGMAX];
        // SAFETY: `room` holds `size` bytes.
        let failed = unsafe {
            libc::sysctl(
                name.as_mut_ptr(),
                name.len() as u32,
                (&mut room as *mut c_int).cast(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        } != 0;
        (!failed).then(|| usize::try_from(room).ok()).flatten()
    })
}

/// The arguments in a `KERN_PROCARGS2` answer, joined by spaces: the answer holds how many there
/// are, the executable's path, the NULs padding it, then each argument ending in a NUL, then the
/// environment, which is left alone.
fn command_in(answer: &[u8]) -> Option<String> {
    let (count, rest) = answer.split_first_chunk::<4>()?;
    let count = usize::try_from(i32::from_ne_bytes(*count)).ok()?;
    let after_path = rest.iter().position(|&byte| byte == 0)?;
    let rest = &rest[after_path..];
    let first = rest.iter().position(|&byte| byte != 0)?;
    let arguments = rest[first..].split(|&byte| byte == 0).take(count);
    let words: Vec<_> = arguments.map(String::from_utf8_lossy).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The text of a NUL-terminated C string in `chars`, as libproc writes names and paths; `None`
/// when no NUL ends it.
fn text(chars: &[c_char]) -> Option<&[u8]> {
    // SAFETY: a `c_char` is one byte, and any byte is a valid `u8`.
    let bytes = unsafe { std::slice::from_raw_parts(chars.as_ptr().cast::<u8>(), chars.len()) };
    CStr::from_bytes_until_nul(bytes).ok().map(CStr::to_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `KERN_PROCARGS2` answer for `arguments`, run from the executable `path`, with `env`.
    fn answer(path: &str, arguments: &[&str], env: &[&str]) -> Vec<u8> {
        let mut bytes = (arguments.len() as i32).to_ne_bytes().to_vec();
        bytes.extend(path.as_bytes());
        // The path is padded with NULs to a word, as the kernel lays the answer out.
        bytes.extend([0, 0, 0]);
        for word in arguments.iter().chain(env) {
            bytes.extend(word.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn a_command_is_its_arguments_joined_by_spaces_without_the_path_or_the_environment() {
        let vite = answer(
            "/opt/homebrew/bin/node",
            &[
                "node",
                "/code/site/node_modules/vite/bin/vite.js",
                "--port",
                "5177",
            ],
            &["HOME=/Users/dev", "PATH=/usr/bin"],
        );
        assert_eq!(
            command_in(&vite).as_deref(),
            Some("node /code/site/node_modules/vite/bin/vite.js --port 5177")
        );
    }

    #[test]
    fn an_answer_cut_short_or_with_no_arguments_has_no_command() {
        assert_eq!(command_in(&[1, 0]), None);
        assert_eq!(command_in(&answer("/bin/sh", &[], &[])), None);
    }

    #[test]
    fn a_name_ends_at_its_nul_and_one_with_none_is_no_name() {
        let chars: Vec<c_char> = b"node\0junk".iter().map(|&byte| byte as c_char).collect();
        assert_eq!(text(&chars), Some(&b"node"[..]));
        let unterminated: Vec<c_char> = b"node".iter().map(|&byte| byte as c_char).collect();
        assert_eq!(text(&unterminated), None);
    }
}
