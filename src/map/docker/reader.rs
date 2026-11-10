//! The thread that reads Docker, and the rule for when it asks the engine anything.
//!
//! Docker Desktop's Resource Saver stops the virtual machine its engine runs in once no container
//! has run for five minutes, and Docker documents that a request to the engine may start it again.
//! Whether a GET does could not be tried on Daniel's Mac without stopping the containers it runs,
//! so the reader takes it that one does, and asks the engine only while containers run: while its
//! last answer listed some, or once the process serving Docker's socket listens on a port it did
//! not listen on when the engine last said none ran, as Docker Desktop's does for each port a
//! container publishes. Watching that process asks the engine nothing: it is found once, by the pid
//! its socket's connection names, with no request sent, and libproc tells where it listens (see
//! `processes`). So a container that starts publishing no port while none runs goes unseen until
//! one that publishes a port starts, and so does the disk use of a Docker found with none running.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use super::http::{self, GetError};
use super::{Container, DockerDisk, Reading, Seen, engine_api};
use crate::map::processes::{self, ProcessId};
use crate::map::{Source, SourceProblem};

/// How often the reader looks: at where the server listens while no container runs, and at what
/// the engine lists while some do, so a container starting or stopping shows within this long.
const LOOK_EVERY: Duration = Duration::from_secs(5);
/// How often the engine is asked for its disk use while containers run, beside each time one
/// starts or stops: it took 0.42 s on Daniel's Mac, and disk use changes slowly.
const DISK_EVERY: Duration = Duration::from_secs(5 * 60);
/// The longest a request that keeps failing is put off: each failure in a row doubles the wait
/// after `LOOK_EVERY`, up to this.
const LONGEST_RETRY: Duration = Duration::from_secs(5 * 60);

/// The thread that reads Docker, which ends when this is dropped.
pub(crate) struct Reader {
    _stop: Sender<()>,
}

impl Reader {
    /// Start reading the Docker served on the first of `sockets` anything serves, telling `tell`
    /// what Docker is each time that changes, until `tell` returns `false`, as no one hears any
    /// more.
    pub(crate) fn start(
        sockets: Vec<PathBuf>,
        tell: impl Fn(Seen) -> bool + Send + 'static,
    ) -> Reader {
        let watch = Watch::new(sockets, ThisMac);
        let (stop, stopped) = mpsc::channel();
        thread::Builder::new()
            .name("sutra-docker".into())
            .spawn(move || run(watch, &tell, &stopped))
            .expect("spawn the thread that reads Docker");
        Reader { _stop: stop }
    }
}

/// Look at Docker whenever `watch` says, telling `tell` each change, until `stopped` says to stop
/// or `tell` finds no one to hear.
fn run<H: Host>(mut watch: Watch<H>, tell: &impl Fn(Seen) -> bool, stopped: &Receiver<()>) {
    let mut told: Option<Seen> = None;
    loop {
        let seen = watch.look(Instant::now(), SystemTime::now());
        if told.as_ref() != Some(&seen) {
            if !tell(seen.clone()) {
                return;
            }
            told = Some(seen);
        }
        let wait = watch.next_look.saturating_duration_since(Instant::now());
        match stopped.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// What the reader learns of the process serving Docker's socket, which asks the engine nothing.
trait Host {
    type Server: Copy + PartialEq + Debug;
    /// The process serving `socket`.
    fn server(&self, socket: &Path) -> io::Result<Self::Server>;
    /// Where `server` listens for TCP connections; `None` once it has ended.
    fn listening(&self, server: Self::Server) -> Option<BTreeSet<SocketAddr>>;
}

/// This Mac, whose processes libproc tells of.
struct ThisMac;

impl Host for ThisMac {
    type Server = ProcessId;

    fn server(&self, socket: &Path) -> io::Result<ProcessId> {
        processes::server_of(socket)
    }

    fn listening(&self, server: ProcessId) -> Option<BTreeSet<SocketAddr>> {
        processes::sockets_of(server)
    }
}

/// What the reader knows of Docker between looks, and when it looks next.
struct Watch<H: Host> {
    host: H,
    /// The sockets Docker may be served on, in the order tried.
    candidates: Vec<PathBuf>,
    /// The socket in use, and the process serving it, while it runs.
    server: Option<(PathBuf, H::Server)>,
    /// Where the server listened when the engine last said no container ran, or when it was found
    /// listening nowhere, and so not asked: one more port is a container publishing it.
    quiet: Option<BTreeSet<SocketAddr>>,
    /// The containers the engine last listed; `None` until it has, and while asking fails.
    listed: Option<Vec<Container>>,
    /// The last list the engine gave held a container, so containers run, as far as Sutra knows.
    running: bool,
    /// The disk use the engine last gave.
    disk: Option<DockerDisk>,
    /// When the engine was last asked for its disk use, whether it answered or not.
    disk_asked: Option<Instant>,
    /// Why the socket could not be connected to, or the engine would not list its containers.
    list_problem: Option<SourceProblem>,
    /// Why the engine would not give its disk use, until it does or no longer runs a container.
    disk_problem: Option<SourceProblem>,
    /// Lists asked for in a row that failed.
    failures: u32,
    next_look: Instant,
}

impl<H: Host> Watch<H> {
    fn new(candidates: Vec<PathBuf>, host: H) -> Watch<H> {
        Watch {
            host,
            candidates,
            server: None,
            quiet: None,
            listed: None,
            running: false,
            disk: None,
            disk_asked: None,
            list_problem: None,
            disk_problem: None,
            failures: 0,
            next_look: Instant::now(),
        }
    }

    /// Look at Docker at `now`, `at` on the clock, asking the engine only while containers run.
    fn look(&mut self, now: Instant, at: SystemTime) -> Seen {
        self.next_look = now + LOOK_EVERY;
        if self.server.is_none() {
            self.server = self.find();
        }
        if let Some((socket, server)) = self.server.clone() {
            match self.host.listening(server) {
                Some(ports) => self.ask_if_running(&socket, ports, now, at),
                // Its server has ended: Docker quit, or is starting again.
                None => self.forget(),
            }
        }
        self.seen()
    }

    /// The first socket a process serves, and that process. A socket missing or served by none is
    /// Docker not running, which is no problem; one that refuses Sutra is.
    fn find(&mut self) -> Option<(PathBuf, H::Server)> {
        let mut refused = None;
        for socket in &self.candidates {
            match self.host.server(socket) {
                Ok(server) => {
                    self.list_problem = None;
                    return Some((socket.clone(), server));
                }
                Err(err) if is_nothing_there(&err) => {}
                Err(err) => {
                    let problem = SourceProblem::new(Source::Docker, socket.clone(), err.kind());
                    refused.get_or_insert(problem);
                }
            }
        }
        self.list_problem = refused;
        None
    }

    /// Ask the engine what runs when containers run, as far as the server listening at `ports`
    /// tells, or its last answer did.
    fn ask_if_running(
        &mut self,
        socket: &Path,
        ports: BTreeSet<SocketAddr>,
        now: Instant,
        at: SystemTime,
    ) {
        let published = match &self.quiet {
            Some(quiet) => *quiet != ports,
            None => !ports.is_empty(),
        };
        if !self.running && !published {
            self.quiet.get_or_insert(ports);
            return;
        }
        let listed = http::get(socket, "/containers/json")
            .and_then(|body| engine_api::containers(&body).map_err(|_| GetError::Malformed));
        let containers = match listed {
            Ok(containers) => self.with_starts(socket, containers),
            Err(err) => return self.failed_to_list(socket, &err, now),
        };
        let ids = |containers: &[Container]| -> Vec<String> {
            containers
                .iter()
                .map(|container| container.id.clone())
                .collect()
        };
        let changed = self.listed.as_deref().map(ids) != Some(ids(&containers));
        self.running = !containers.is_empty();
        if !self.running {
            self.quiet = Some(ports);
            // Nothing more is asked of the engine, so what it last said of its disk stands.
            self.disk_problem = None;
        }
        self.listed = Some(containers);
        self.list_problem = None;
        self.failures = 0;
        let disk_due = self
            .disk_asked
            .is_none_or(|asked| now >= asked + DISK_EVERY);
        if self.running && (changed || disk_due) {
            self.ask_for_disk(socket, now, at);
        }
    }

    /// `listed`, each with when it last started, which the list does not say: as the engine said
    /// when last it listed the container, or else as it says when asked of the container alone,
    /// once, the first time it lists it. What it would not say then it is not asked again, as it
    /// would answer alike; and a container that restarts between two looks keeps the start it had.
    fn with_starts(&self, socket: &Path, listed: Vec<Container>) -> Vec<Container> {
        let before = self.listed.as_deref().unwrap_or_default();
        let known = |id: &str| before.iter().find(|container| container.id == id);
        let started = |id: &str| {
            let body = http::get(socket, &format!("/containers/{id}/json")).ok()?;
            engine_api::started(&body).ok().flatten()
        };
        listed
            .into_iter()
            .map(|container| Container {
                started: match known(&container.id) {
                    Some(known) => known.started,
                    None => started(&container.id),
                },
                ..container
            })
            .collect()
    }

    fn ask_for_disk(&mut self, socket: &Path, now: Instant, at: SystemTime) {
        self.disk_asked = Some(now);
        let disk = http::get(socket, "/system/df")
            .and_then(|body| engine_api::disk_use(&body, at).map_err(|_| GetError::Malformed));
        match disk {
            Ok(disk) => {
                self.disk = Some(disk);
                self.disk_problem = None;
            }
            // What was last said of the disk stands.
            Err(err) => {
                let socket = socket.to_path_buf();
                let problem = SourceProblem::new(Source::DockerDiskUse, socket, err.kind());
                self.disk_problem = Some(problem);
            }
        }
    }

    /// The engine would not list its containers: they are unknown, and the next list is asked for
    /// later and later while it keeps failing. A server that stopped meanwhile is Docker gone.
    fn failed_to_list(&mut self, socket: &Path, err: &GetError, now: Instant) {
        if let GetError::Connect(err) = err {
            if is_nothing_there(err) {
                return self.forget();
            }
        }
        self.listed = None;
        let problem = SourceProblem::new(Source::Docker, socket.to_path_buf(), err.kind());
        self.list_problem = Some(problem);
        self.failures += 1;
        let backoff = LOOK_EVERY.saturating_mul(1 << self.failures.min(16));
        self.next_look = now + backoff.min(LONGEST_RETRY);
    }

    /// Docker has gone: whatever serves its socket next starts afresh.
    fn forget(&mut self) {
        self.server = None;
        self.quiet = None;
        self.listed = None;
        self.running = false;
        self.disk = None;
        self.disk_asked = None;
        self.list_problem = None;
        self.disk_problem = None;
        self.failures = 0;
    }

    /// What Docker is as far as Sutra knows: away until the engine has said something worth a
    /// card, a container running or its disk use.
    fn seen(&self) -> Seen {
        let problem = self
            .list_problem
            .clone()
            .or_else(|| self.disk_problem.clone());
        let running = self.listed.as_ref().filter(|listed| !listed.is_empty());
        let reading = match &self.server {
            Some((socket, _)) if running.is_some() || self.disk.is_some() => Reading::Said {
                socket: socket.clone(),
                containers: running.cloned().unwrap_or_default(),
                disk: self.disk,
            },
            _ => Reading::Away,
        };
        Seen { reading, problem }
    }
}

/// Whether connecting failed for want of a socket, or of anything serving it.
fn is_nothing_there(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::Ipv4Addr;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::map::docker::http::tests::{FakeEngine, answer};
    use crate::map::test_support::TempDir;
    use crate::map::utc;

    /// A server whose ports a test sets, `None` once it has ended, connected to for real, as
    /// finding the server of a socket nothing serves fails as on this Mac.
    #[derive(Clone)]
    struct FakeHost(Arc<Mutex<Option<BTreeSet<SocketAddr>>>>);

    impl FakeHost {
        fn listening_on(ports: &[u16]) -> FakeHost {
            FakeHost(Arc::new(Mutex::new(Some(on(ports)))))
        }

        fn set(&self, ports: Option<&[u16]>) {
            *self.0.lock().unwrap() = ports.map(on);
        }
    }

    impl Host for FakeHost {
        type Server = ();

        fn server(&self, socket: &Path) -> io::Result<()> {
            UnixStream::connect(socket).map(drop)
        }

        fn listening(&self, (): ()) -> Option<BTreeSet<SocketAddr>> {
            self.0.lock().unwrap().clone()
        }
    }

    fn on(ports: &[u16]) -> BTreeSet<SocketAddr> {
        let at = |&port: &u16| SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        ports.iter().map(at).collect()
    }

    /// An engine that lists what `listed` holds, `[]` to begin with, says that `e99d` started at
    /// `PG_STARTED`, and whose disk use is two gigabytes of images, and nothing else.
    fn engine(test: &str) -> (FakeEngine, Arc<Mutex<String>>) {
        let listed = Arc::new(Mutex::new("[]".to_string()));
        let shown = Arc::clone(&listed);
        let engine = FakeEngine::serving(test, move |path| match path {
            "/containers/json" => answer("200 OK", &shown.lock().unwrap()),
            "/containers/e99d/json" => answer("200 OK", &inspected(PG_STARTED)),
            "/system/df" => answer("200 OK", DISK),
            _ => answer("404 Not Found", "{}"),
        });
        (engine, listed)
    }

    /// When `e99d` started, as the engine dates it.
    const PG_STARTED: &str = "2026-11-09T22:48:31.95367792Z";

    /// What the engine tells of a container started at `started_at`, asked of it alone.
    fn inspected(started_at: &str) -> String {
        format!(r#"{{"Id": "e99d", "State": {{"Running": true, "StartedAt": "{started_at}"}}}}"#)
    }

    const DISK: &str = r#"{
        "ImageUsage": {"TotalCount": 1, "ActiveCount": 1, "TotalSize": 2000000000,
                       "Reclaimable": 0},
        "ContainerUsage": {"TotalCount": 1, "ActiveCount": 1, "TotalSize": 0, "Reclaimable": 0},
        "VolumeUsage": {"TotalCount": 0, "ActiveCount": 0, "TotalSize": 0, "Reclaimable": 0},
        "BuildCacheUsage": {"TotalCount": 0, "TotalSize": 0, "Reclaimable": 0}
    }"#;
    const ONE_RUNNING: &str = r#"[{"Id": "e99d", "Names": ["/pg"], "Image": "postgres:17",
        "Ports": [{"IP": "127.0.0.1", "PrivatePort": 5432, "PublicPort": 36684, "Type": "tcp"}]}]"#;
    const TWO_RUNNING: &str = r#"[
        {"Id": "e99d", "Names": ["/pg"], "Image": "postgres:17",
         "Ports": [{"IP": "127.0.0.1", "PrivatePort": 5432, "PublicPort": 36684, "Type": "tcp"}]},
        {"Id": "c4c4", "Names": ["/cache"], "Image": "redis:7"}
    ]"#;

    fn requests(engine: &FakeEngine) -> Vec<String> {
        engine.requests.lock().unwrap().clone()
    }

    fn containers_of(seen: &Seen) -> Vec<String> {
        match &seen.reading {
            Reading::Said { containers, .. } => containers.iter().map(|c| c.name.clone()).collect(),
            Reading::Away => panic!("Docker said nothing: {seen:?}"),
        }
    }

    const LIST: &str = "GET /containers/json HTTP/1.1";
    const INSPECT_PG: &str = "GET /containers/e99d/json HTTP/1.1";
    const DISK_USE: &str = "GET /system/df HTTP/1.1";

    #[test]
    fn while_containers_run_the_engine_lists_them_at_each_look_and_gives_its_disk_use_at_times() {
        let (engine, listed) = engine("docker-reader-running");
        *listed.lock().unwrap() = ONE_RUNNING.to_string();
        let mut watch = Watch::new(
            vec![engine.socket.clone()],
            FakeHost::listening_on(&[36684]),
        );
        let start = Instant::now();

        let seen = watch.look(start, SystemTime::now());

        assert_eq!(containers_of(&seen), ["pg"]);
        let Reading::Said { disk, .. } = &seen.reading else {
            unreachable!()
        };
        assert_eq!(disk.map(|disk| disk.bytes()), Some(2_000_000_000));
        assert_eq!(
            requests(&engine),
            [LIST, INSPECT_PG, DISK_USE],
            "nothing sent to find the server"
        );
        watch.look(start + LOOK_EVERY, SystemTime::now());
        assert_eq!(
            requests(&engine),
            [LIST, INSPECT_PG, DISK_USE, LIST],
            "the disk use is not due again, nor pg's start"
        );
        watch.look(start + DISK_EVERY, SystemTime::now());
        assert_eq!(
            requests(&engine),
            [LIST, INSPECT_PG, DISK_USE, LIST, LIST, DISK_USE]
        );
    }

    #[test]
    fn the_engine_is_asked_when_a_container_started_once_the_first_time_it_lists_it() {
        let engine = FakeEngine::serving("docker-reader-started", |path| match path {
            "/containers/json" => answer("200 OK", TWO_RUNNING),
            "/containers/e99d/json" => answer("200 OK", &inspected(PG_STARTED)),
            "/system/df" => answer("200 OK", DISK),
            _ => answer("404 Not Found", r#"{"message": "No such container"}"#),
        });
        let mut watch = Watch::new(
            vec![engine.socket.clone()],
            FakeHost::listening_on(&[36684]),
        );
        let start = Instant::now();

        watch.look(start, SystemTime::now());
        let seen = watch.look(start + LOOK_EVERY, SystemTime::now());

        let Reading::Said { containers, .. } = &seen.reading else {
            panic!("Docker said nothing: {seen:?}");
        };
        let started: Vec<(&str, Option<SystemTime>)> = containers
            .iter()
            .map(|container| (container.name.as_str(), container.started))
            .collect();
        assert_eq!(
            started,
            [("cache", None), ("pg", utc::parse(PG_STARTED))],
            "the engine would not say when cache started"
        );
        let inspect_cache = "GET /containers/c4c4/json HTTP/1.1";
        assert_eq!(
            requests(&engine),
            [LIST, inspect_cache, INSPECT_PG, DISK_USE, LIST],
            "neither asked again, as the engine would answer alike"
        );
    }

    #[test]
    fn once_the_engine_lists_none_running_it_is_asked_nothing_until_the_server_listens_anew() {
        let (engine, listed) = engine("docker-reader-quiet");
        let host = FakeHost::listening_on(&[6443]);
        let mut watch = Watch::new(vec![engine.socket.clone()], host.clone());
        let start = Instant::now();

        let seen = watch.look(start, SystemTime::now());
        assert_eq!(
            seen.reading,
            Reading::Away,
            "none runs, and nothing else is known"
        );
        for look in 1..=3 {
            watch.look(start + LOOK_EVERY * look, SystemTime::now());
        }
        assert_eq!(requests(&engine), [LIST], "asked once, then left to sleep");

        *listed.lock().unwrap() = ONE_RUNNING.to_string();
        host.set(Some(&[6443, 36684]));
        let seen = watch.look(start + LOOK_EVERY * 4, SystemTime::now());

        assert_eq!(containers_of(&seen), ["pg"], "a container published a port");
        assert_eq!(requests(&engine), [LIST, LIST, INSPECT_PG, DISK_USE]);
    }

    #[test]
    fn a_docker_found_listening_on_no_port_is_asked_nothing_and_is_not_on_the_map() {
        let (engine, listed) = engine("docker-reader-asleep");
        *listed.lock().unwrap() = ONE_RUNNING.to_string();
        let mut watch = Watch::new(vec![engine.socket.clone()], FakeHost::listening_on(&[]));

        let seen = watch.look(Instant::now(), SystemTime::now());

        assert_eq!(
            seen,
            Seen {
                reading: Reading::Away,
                problem: None
            }
        );
        assert_eq!(requests(&engine), [""; 0], "its engine may sleep");
    }

    #[test]
    fn docker_not_running_is_no_problem_and_a_socket_that_refuses_sutra_is_one() {
        let folder = TempDir::new("docker-reader-refused");
        let missing = folder.path("missing.sock");
        let left = folder.path("left.sock");
        drop(UnixListener::bind(&left).unwrap());
        let host = FakeHost::listening_on(&[36684]);

        let mut away = Watch::new(vec![missing, left], host.clone());
        let seen = away.look(Instant::now(), SystemTime::now());
        assert_eq!(
            seen,
            Seen {
                reading: Reading::Away,
                problem: None
            }
        );

        let sealed = folder.path("sealed.sock");
        let _served = UnixListener::bind(&sealed).unwrap();
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
        let mut refused = Watch::new(vec![sealed.clone()], host);
        let seen = refused.look(Instant::now(), SystemTime::now());
        assert_eq!(seen.reading, Reading::Away);
        assert_eq!(
            seen.problem,
            Some(SourceProblem::new(
                Source::Docker,
                sealed,
                io::ErrorKind::PermissionDenied
            ))
        );
    }

    #[test]
    fn an_engine_that_fails_to_answer_is_a_problem_asked_again_later_and_later_until_it_answers() {
        let answers = Arc::new(Mutex::new(answer("500 Internal Server Error", "{}")));
        let current = Arc::clone(&answers);
        let engine = FakeEngine::serving("docker-reader-failing", move |path| match path {
            "/containers/json" => current.lock().unwrap().clone(),
            _ => answer("200 OK", DISK),
        });
        let mut watch = Watch::new(vec![engine.socket.clone()], FakeHost::listening_on(&[3306]));
        let start = Instant::now();

        let seen = watch.look(start, SystemTime::now());
        assert_eq!(seen.reading, Reading::Away, "it never said what runs");
        let problem =
            SourceProblem::new(Source::Docker, engine.socket.clone(), io::ErrorKind::Other);
        assert_eq!(seen.problem, Some(problem.clone()));
        assert_eq!(watch.next_look, start + LOOK_EVERY * 2);
        watch.look(watch.next_look, SystemTime::now());
        assert_eq!(
            watch.next_look,
            start + LOOK_EVERY * 6,
            "twice as long again"
        );

        *answers.lock().unwrap() = answer("200 OK", ONE_RUNNING);
        let seen = watch.look(watch.next_look, SystemTime::now());
        assert_eq!(containers_of(&seen), ["pg"]);
        assert_eq!(seen.problem, None);
    }

    #[test]
    fn a_disk_use_the_engine_will_not_give_is_a_problem_of_its_own_beside_the_containers_it_lists()
    {
        let engine = FakeEngine::serving("docker-disk-fail", |path| match path {
            "/containers/json" => answer("200 OK", ONE_RUNNING),
            _ => answer("200 OK", r#"{"LayersSize": 1}"#),
        });
        let mut watch = Watch::new(
            vec![engine.socket.clone()],
            FakeHost::listening_on(&[36684]),
        );

        let seen = watch.look(Instant::now(), SystemTime::now());

        assert_eq!(containers_of(&seen), ["pg"]);
        let problem = seen.problem.expect("the disk use could not be read");
        assert_eq!(
            (problem.source, problem.kind),
            (Source::DockerDiskUse, io::ErrorKind::InvalidData)
        );
    }

    #[test]
    fn a_server_that_ends_takes_docker_off_the_map_and_its_problem_with_it() {
        let (engine, listed) = engine("docker-reader-ended");
        *listed.lock().unwrap() = ONE_RUNNING.to_string();
        let host = FakeHost::listening_on(&[36684]);
        let mut watch = Watch::new(vec![engine.socket.clone()], host.clone());
        let start = Instant::now();
        assert_eq!(containers_of(&watch.look(start, SystemTime::now())), ["pg"]);

        host.set(None);
        let seen = watch.look(start + LOOK_EVERY, SystemTime::now());

        assert_eq!(
            seen,
            Seen {
                reading: Reading::Away,
                problem: None
            }
        );
    }

    #[test]
    fn the_reader_tells_what_changes_once_and_ends_when_stopped() {
        let (engine, listed) = engine("docker-reader-thread");
        *listed.lock().unwrap() = ONE_RUNNING.to_string();
        let watch = Watch::new(
            vec![engine.socket.clone()],
            FakeHost::listening_on(&[36684]),
        );
        let (told, heard) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();

        let reader = thread::spawn(move || run(watch, &|seen| told.send(seen).is_ok(), &stopped));

        let first = heard.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(containers_of(&first), ["pg"]);
        drop(stop);
        reader.join().unwrap();
        assert!(heard.try_recv().is_err(), "the same again is not told");
    }
}
