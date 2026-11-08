//! Docker, as its engine tells of it over its Unix socket: the containers running, the ports they
//! publish and the folders they are for, and the disk Docker takes, its images, its containers,
//! its volumes and its build cache apart. The map places each container on the project whose
//! folder it is for, and keeps the rest, with the disk use, for a card of Docker's own.
//!
//! Sutra only reads: every request is a GET (see `http`), and nothing here starts, stops or
//! cleans anything. Nor does it ever keep Docker awake: Docker Desktop's Resource Saver stops the
//! virtual machine the engine runs in once no container has run for a while, and a request to the
//! engine may start it again, so the engine is asked only while containers run (see `reader`). The
//! direct edition alone reads Docker: the sandbox keeps the store edition out of its socket.

// What reads Docker needs libproc to tell when its engine may be asked (see `reader`).
#[cfg(target_os = "macos")]
mod engine_api;
#[cfg(target_os = "macos")]
mod http;
#[cfg(target_os = "macos")]
mod reader;

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::SourceProblem;

#[cfg(target_os = "macos")]
pub(crate) use reader::Reader;

/// A container the engine said was running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    /// Its name, without the `/` the engine writes before it: `pg596`.
    pub name: String,
    /// The image it runs, as the engine names it: `postgres:17`.
    pub image: String,
    /// The folder `docker compose` ran in when compose made it, as its
    /// `com.docker.compose.project.working_dir` label names it: the project it is for.
    pub compose_folder: Option<PathBuf>,
    /// The folders of this Mac it bind-mounts, by path, each once: the engine lists a container's
    /// mounts in no order of its own, so none tells more than another.
    pub bound: Vec<PathBuf>,
    /// The ports it publishes on this Mac, by port.
    pub ports: Vec<PublishedPort>,
    /// When it was made.
    pub created: SystemTime,
}

/// A port of this Mac that Docker forwards to a container's port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedPort {
    /// The port on this Mac.
    pub port: u16,
    /// The port inside the container it forwards to.
    pub to: u16,
    /// Where on this Mac it listens: `127.0.0.1`, or `0.0.0.0` and `::` for every address.
    pub addresses: BTreeSet<IpAddr>,
    /// UDP rather than TCP, which no browser opens.
    pub udp: bool,
}

impl Container {
    /// The folders it is for: its compose folder, then those it bind-mounts.
    pub fn folders(&self) -> impl Iterator<Item = &Path> {
        let compose = self.compose_folder.iter();
        compose.chain(&self.bound).map(PathBuf::as_path)
    }

    /// The first of its folders that lies in `project`.
    pub fn folder_in(&self, project: &Path) -> Option<&Path> {
        self.folders().find(|folder| folder.starts_with(project))
    }
}

/// What Docker takes on disk, as the engine last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockerDisk {
    pub images: Store,
    pub containers: Store,
    pub volumes: Store,
    pub build_cache: Store,
    /// When the engine said so.
    pub read_at: SystemTime,
}

impl DockerDisk {
    /// What the four take between them.
    pub fn bytes(&self) -> u64 {
        [self.images, self.containers, self.volumes, self.build_cache]
            .iter()
            .map(|store| store.bytes)
            .sum()
    }
}

/// One kind of thing Docker keeps on disk: its images, its containers, its volumes or its build
/// cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Store {
    pub bytes: u64,
    /// How many there are.
    pub count: u64,
    /// How many are in use: images and volumes a container uses, containers running, build cache
    /// a build holds.
    pub in_use: u64,
    /// What the engine says a prune would give back, which Sutra never does: volumes hold data.
    pub reclaimable: u64,
}

/// Docker on the map: the containers running in no project's folder, and its disk use, as the
/// engine last said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Docker {
    /// The socket Docker is read through, which tells this Docker from another.
    pub socket: PathBuf,
    /// The containers running in no project's folder, by name.
    pub containers: Vec<Container>,
    /// `None` until the engine has said, as it is asked only while containers run.
    pub disk: Option<DockerDisk>,
}

/// What the reader found of Docker, as the engine thread receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) reading: Reading,
    /// Why the socket, or the engine behind it, could not be read: a socket that is there and
    /// refuses Sutra, or an engine that failed to answer, which leaves its containers unknown.
    /// Docker not running, or not installed, is no problem.
    pub(crate) problem: Option<SourceProblem>,
}

/// What Docker is, as far as Sutra knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reading {
    /// Docker is not here: nothing serves its socket, or the engine has said nothing worth a card
    /// yet, as no container was known to run when Sutra found it, or none ran when it said.
    Away,
    /// What the engine said at `socket`: the containers running, by name, none while a request
    /// fails, and the disk use, as last said.
    Said {
        socket: PathBuf,
        containers: Vec<Container>,
        disk: Option<DockerDisk>,
    },
}

/// The socket Docker's engine is served on, of those this Mac has: the one `$DOCKER_HOST` names,
/// when it names a Unix socket, then Docker Desktop's in the home folder, then the system's, which
/// Docker Desktop and other engines link to theirs.
pub(crate) fn socket_candidates(home: &Path, docker_host: Option<&str>) -> Vec<PathBuf> {
    let named = docker_host.and_then(|host| host.strip_prefix("unix://"));
    let named = named
        .filter(|path| path.starts_with('/'))
        .map(PathBuf::from);
    let usual = [
        home.join(".docker/run/docker.sock"),
        "/var/run/docker.sock".into(),
    ];
    named.into_iter().chain(usual).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_host_naming_a_unix_socket_comes_first_and_one_naming_anything_else_is_passed_over() {
        let home = Path::new("/Users/dev");
        let usual = [
            PathBuf::from("/Users/dev/.docker/run/docker.sock"),
            PathBuf::from("/var/run/docker.sock"),
        ];
        let named = socket_candidates(home, Some("unix:///Users/dev/.colima/default/docker.sock"));
        assert_eq!(
            named[0],
            Path::new("/Users/dev/.colima/default/docker.sock")
        );
        assert_eq!(named[1..], usual);
        for passed_over in [
            None,
            Some("tcp://10.0.0.2:2376"),
            Some("unix://relative.sock"),
        ] {
            assert_eq!(
                socket_candidates(home, passed_over),
                usual,
                "{passed_over:?}"
            );
        }
    }

    #[test]
    fn a_containers_folder_on_a_project_is_its_compose_folder_there_else_the_first_it_binds() {
        let container = Container {
            id: "c1".to_string(),
            name: "web".to_string(),
            image: "nginx".to_string(),
            compose_folder: Some(PathBuf::from("/Users/dev/code/site")),
            bound: vec![
                PathBuf::from("/Users/dev/code/app"),
                PathBuf::from("/Users/dev/code/app/public"),
                PathBuf::from("/Users/dev/code/site/public"),
            ],
            ports: Vec::new(),
            created: SystemTime::UNIX_EPOCH,
        };
        assert_eq!(
            container.folder_in(Path::new("/Users/dev/code/app")),
            Some(Path::new("/Users/dev/code/app"))
        );
        assert_eq!(
            container.folder_in(Path::new("/Users/dev/code/site")),
            Some(Path::new("/Users/dev/code/site"))
        );
        assert_eq!(container.folder_in(Path::new("/Users/dev/notes")), None);
    }
}
