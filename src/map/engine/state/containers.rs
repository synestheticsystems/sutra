//! Placing Docker's containers on the cards of the projects that account for them, where their
//! ports show, and keeping every container, each with those projects, for Docker's group.
//! Three claims account for a container, the most specific first, and the first any project on the
//! map makes settles it:
//!   - compose made it, in a folder a project holds: that project, as compose says where it ran;
//!   - a dev-runner environment names, for one of its units, a port the container publishes on
//!     this Mac: each project such an environment describes, whether its process still runs or not,
//!     as the database an environment started may outlive it;
//!   - it bind-mounts a folder a project holds: each such project, as it runs what that project
//!     holds. A mount is the least specific claim, as a folder many share is mounted by many; and
//!     the engine lists a container's mounts in no order of its own (one of Daniel's listed four in
//!     two orders on 2026-11-07), so none of those projects is more its own than another.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::tracked::Tracked;
use super::{State, holder};
use crate::map::docker::{Container, Docker, Reading, RunningContainer, Seen};
use crate::map::{Source, SourceProblem};

impl State {
    /// Take what the reader found of Docker, and place its containers. Returns whether anything
    /// changed.
    pub(in crate::map::engine) fn saw_docker(&mut self, seen: Seen) -> bool {
        let problems: Vec<SourceProblem> = seen.problem.into_iter().collect();
        let reported = self.report_problems(&[Source::Docker, Source::DockerDiskUse], problems);
        let read_again = self.docker != seen.reading;
        self.docker = seen.reading;
        // Not short-circuited: the containers are placed whatever else changed.
        reported | read_again | self.place_containers()
    }

    /// Place each container Docker said was running on the projects that account for it, as the
    /// projects on the map and their environments now are, and keep every one with those projects
    /// for Docker's group. Returns whether any container moved.
    pub(in crate::map::engine) fn place_containers(&mut self) -> bool {
        let listed = match &self.docker {
            Reading::Said { containers, .. } => containers.as_slice(),
            Reading::Away => &[],
        };
        let named = named_ports(&self.projects);
        let running: Vec<RunningContainer> = listed
            .iter()
            .map(|container| RunningContainer {
                projects: projects_for(&self.projects, &named, container),
                container: container.clone(),
            })
            .collect();
        let mut placed: HashMap<&Path, Vec<Container>> = HashMap::new();
        for running in &running {
            for project in &running.projects {
                let here = placed.entry(project).or_default();
                here.push(running.container.clone());
            }
        }
        let mut changed = false;
        for (path, tracked) in &mut self.projects {
            let here = placed.remove(path.as_path()).unwrap_or_default();
            if tracked.project.containers != here {
                tracked.project.containers = here;
                changed = true;
            }
        }
        if self.running_containers != running {
            self.running_containers = running;
            changed = true;
        }
        changed
    }

    /// Docker as the map shows it: what its engine last said, with every container running and
    /// the projects that account for each.
    pub(super) fn docker_on_the_map(&self) -> Option<Docker> {
        match &self.docker {
            Reading::Said { socket, disk, .. } => Some(Docker {
                socket: socket.clone(),
                running: self.running_containers.clone(),
                disk: *disk,
            }),
            Reading::Away => None,
        }
    }
}

/// Each port a dev-runner environment on the map names for one of its units, with the projects
/// those environments describe, by path.
fn named_ports(projects: &BTreeMap<PathBuf, Tracked>) -> HashMap<u16, Vec<&Path>> {
    let mut named: HashMap<u16, Vec<&Path>> = HashMap::new();
    for (path, tracked) in projects {
        let Some(environment) = &tracked.project.environment else {
            continue;
        };
        for &port in environment.ports.values() {
            let naming = named.entry(port).or_default();
            // Two of its units may name one port; the projects come in turn, by path.
            if naming.last() != Some(&path.as_path()) {
                naming.push(path);
            }
        }
    }
    named
}

/// The projects on the map that account for `container`, each once, by path: the one holding its
/// compose folder, when one does; or else each whose environment names a port it publishes
/// (`named`); or else each holding a folder it bind-mounts.
fn projects_for(
    projects: &BTreeMap<PathBuf, Tracked>,
    named: &HashMap<u16, Vec<&Path>>,
    container: &Container,
) -> Vec<PathBuf> {
    let compose = container.compose_folder.as_deref();
    if let Some(project) = compose.and_then(|folder| holder(projects, folder)) {
        return vec![project.to_path_buf()];
    }
    let naming = container
        .ports
        .iter()
        .filter_map(|port| named.get(&port.port));
    let mut found: Vec<&Path> = naming.flatten().copied().collect();
    if found.is_empty() {
        let binding = container.bound.iter();
        found = binding
            .filter_map(|folder| holder(projects, folder))
            .collect();
    }
    found.sort_unstable();
    found.dedup();
    found.into_iter().map(Path::to_path_buf).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::io;
    use std::path::Path;

    use super::*;
    use crate::map::ScanOptions;
    use crate::map::docker::PublishedPort;
    use crate::map::engine::state::tracked::Tracked;
    use crate::map::grant::Roots;
    use crate::map::scan::Boundary;
    use crate::map::test_support::{DEAD_PID, environment_at};
    use crate::model::Environment;

    /// A map of no root holding projects at `paths`, none of them on disk.
    fn state_holding(paths: &[&str]) -> State {
        let roots = Roots {
            home: PathBuf::from("/nonexistent/home"),
            dirs: Vec::new(),
            scratch: None,
            docker: Vec::new(),
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());
        let boundary = Boundary::new(&[], Path::new("/"), &ScanOptions::default());
        for path in paths {
            let tracked = Tracked::plain(PathBuf::from(path), &boundary, &|_| true);
            state.projects.insert(PathBuf::from(path), tracked);
        }
        state
    }

    /// Give the project at `folder` on `state` an environment run by `pid`, naming `ports` for its
    /// units.
    fn describe(state: &mut State, folder: &str, pid: u32, ports: &[(&str, u16)]) {
        let named = ports.iter().map(|&(unit, port)| (unit.to_string(), port));
        let environment = Environment {
            pid,
            alive: pid != DEAD_PID,
            ports: named.collect::<HashMap<String, u16>>(),
            ..environment_at(folder)
        };
        let project = &mut state.projects.get_mut(Path::new(folder)).unwrap().project;
        project.environment = Some(environment);
    }

    /// The running container `name`, made by compose in `compose` if given, bind-mounting `bound`.
    fn container(name: &str, compose: Option<&str>, bound: &[&str]) -> Container {
        Container {
            id: name.to_string(),
            name: name.to_string(),
            image: "postgres:17".to_string(),
            compose_folder: compose.map(PathBuf::from),
            bound: bound.iter().map(PathBuf::from).collect(),
            ports: Vec::new(),
            started: None,
        }
    }

    /// `container`, publishing each of `ports` on 127.0.0.1, to Postgres's port inside it.
    fn publishing(container: Container, ports: &[u16]) -> Container {
        let published = ports.iter().map(|&port| PublishedPort {
            port,
            to: 5432,
            addresses: BTreeSet::from(["127.0.0.1".parse().unwrap()]),
            udp: false,
        });
        Container {
            ports: published.collect(),
            ..container
        }
    }

    /// What the reader tells when the engine at `/run/docker.sock` lists `containers`.
    fn said(containers: Vec<Container>) -> Seen {
        Seen {
            reading: Reading::Said {
                socket: PathBuf::from("/run/docker.sock"),
                containers,
                disk: None,
            },
            problem: None,
        }
    }

    fn names(containers: &[Container]) -> Vec<&str> {
        containers.iter().map(|c| c.name.as_str()).collect()
    }

    /// Each container Docker on the map holds, by name, with the projects that account for it.
    fn accounted(state: &State) -> Vec<(String, Vec<PathBuf>)> {
        let docker = state.snapshot().docker.expect("Docker said");
        let running = docker.running.into_iter();
        running
            .map(|running| (running.container.name, running.projects))
            .collect()
    }

    fn by(name: &str, projects: &[&str]) -> (String, Vec<PathBuf>) {
        (
            name.to_string(),
            projects.iter().map(PathBuf::from).collect(),
        )
    }

    #[test]
    fn a_container_compose_made_shows_on_the_project_compose_ran_in_and_no_other() {
        let mut state = state_holding(&["/code/app", "/code/app/api", "/code/site"]);
        describe(&mut state, "/code/site", 4242, &[("db", 55432)]);
        let web = container("app-web-1", Some("/code/app"), &["/code/app", "/code/site"]);

        assert!(state.saw_docker(said(vec![publishing(web, &[55432])])));

        let on = |project: &str| names(&state.projects[Path::new(project)].project.containers);
        assert_eq!(on("/code/app"), ["app-web-1"]);
        assert_eq!(
            on("/code/site"),
            [""; 0],
            "it binds a folder there, and site's environment names its port, but compose ran in app"
        );
    }

    #[test]
    fn a_project_whose_environment_names_a_port_a_container_publishes_accounts_for_it_before_a_mount()
     {
        let mut state = state_holding(&["/code/core", "/code/studio", "/code/studio-ownership"]);
        // The environment that started studio-pg-5069 has stopped; its database still runs.
        describe(
            &mut state,
            "/code/studio-ownership",
            DEAD_PID,
            &[("postgres", 31224)],
        );
        describe(
            &mut state,
            "/code/studio",
            4242,
            &[("postgres", 46823), ("api", 27378)],
        );
        let seen = said(vec![
            publishing(container("studio-pg-5069", None, &[]), &[31224]),
            publishing(
                container("studio-pg-74a9", None, &["/code/core/init"]),
                &[46823],
            ),
            publishing(container("pg596", None, &[]), &[55596]),
        ]);

        assert!(state.saw_docker(seen));

        assert_eq!(
            accounted(&state),
            [
                by("studio-pg-5069", &["/code/studio-ownership"]),
                by("studio-pg-74a9", &["/code/studio"]),
                by("pg596", &[]),
            ],
            "74a9 binds a folder of core, but studio's environment names its port"
        );
        let on = |project: &str| names(&state.projects[Path::new(project)].project.containers);
        assert_eq!(on("/code/studio-ownership"), ["studio-pg-5069"]);
        assert_eq!(on("/code/core"), [""; 0]);
    }

    #[test]
    fn a_container_shows_on_each_project_holding_a_folder_it_binds_and_docker_lists_every_one() {
        let mut state = state_holding(&["/code/app", "/code/app/api", "/code/core", "/code/site"]);
        let seen = said(vec![
            container(
                "main",
                None,
                &["/code/app/api/db", "/code/core", "/shared/backplane"],
            ),
            container(
                "payment",
                Some("/shared"),
                &["/code/core/lib", "/code/core/sdk"],
            ),
            container("pg", None, &[]),
            container("cache", None, &["/shared/backplane"]),
        ]);

        assert!(state.saw_docker(seen));

        let on = |project: &str| names(&state.projects[Path::new(project)].project.containers);
        assert_eq!(
            on("/code/app/api"),
            ["main"],
            "the deepest project holding the folder"
        );
        assert_eq!(on("/code/app"), [""; 0]);
        assert_eq!(
            on("/code/core"),
            ["main", "payment"],
            "once each, compose having run in no project"
        );
        assert_eq!(on("/code/site"), [""; 0]);
        assert_eq!(
            accounted(&state),
            [
                by("main", &["/code/app/api", "/code/core"]),
                by("payment", &["/code/core"]),
                by("pg", &[]),
                by("cache", &[]),
            ]
        );
        let docker = state.snapshot().docker.unwrap();
        assert_eq!(docker.socket, Path::new("/run/docker.sock"));
    }

    #[test]
    fn a_project_that_joins_the_map_or_names_a_port_anew_accounts_for_its_containers_once_placed() {
        let mut state = state_holding(&["/code/app"]);
        state.saw_docker(said(vec![
            container("web", None, &["/code/site"]),
            publishing(container("pg", None, &[]), &[55432]),
        ]));
        assert_eq!(accounted(&state), [by("web", &[]), by("pg", &[])]);

        let boundary = Boundary::new(&[], Path::new("/"), &ScanOptions::default());
        let site = Tracked::plain(PathBuf::from("/code/site"), &boundary, &|_| true);
        state.projects.insert(PathBuf::from("/code/site"), site);
        describe(&mut state, "/code/app", 4242, &[("db", 55432)]);

        assert!(state.place_containers());
        assert_eq!(
            accounted(&state),
            [by("web", &["/code/site"]), by("pg", &["/code/app"])]
        );
        let on_site = &state.projects[Path::new("/code/site")].project.containers;
        assert_eq!(names(on_site), ["web"]);
        assert!(!state.place_containers(), "placed already");
    }

    #[test]
    fn docker_gone_takes_its_containers_and_its_problem_off_the_map() {
        let mut state = state_holding(&["/code/app"]);
        let failing = Seen {
            problem: Some(SourceProblem::new(
                Source::Docker,
                PathBuf::from("/run/docker.sock"),
                io::ErrorKind::TimedOut,
            )),
            ..said(vec![container("web", None, &["/code/app"])])
        };
        assert!(state.saw_docker(failing));
        assert_eq!(state.snapshot().problems.len(), 1);

        let away = Seen {
            reading: Reading::Away,
            problem: None,
        };
        assert!(state.saw_docker(away.clone()));

        let snapshot = state.snapshot();
        assert_eq!(snapshot.docker, None);
        assert_eq!(snapshot.problems, []);
        assert!(snapshot.projects[0].containers.is_empty());
        assert!(!state.saw_docker(away), "nothing new");
    }
}
