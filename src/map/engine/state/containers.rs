//! Placing Docker's containers on the cards of the projects they are for, where their ports show,
//! and the others, with Docker's disk use, on Docker's own card. A container compose made is for
//! the project holding the folder compose ran in. Any other is for each project holding a folder it
//! bind-mounts, as it runs what that project holds: the engine lists a container's mounts in no
//! order of its own (one of Daniel's listed four in two orders on 2026-11-07), so none of those
//! projects is more its own than another.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::tracked::Tracked;
use super::{State, holder};
use crate::map::docker::{Container, Docker, Reading, Seen};
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

    /// Place each container Docker said was running on the projects it is for, as the projects on
    /// the map now are, and keep the others for Docker's card. Returns whether any container moved.
    pub(in crate::map::engine) fn place_containers(&mut self) -> bool {
        let running = match &self.docker {
            Reading::Said { containers, .. } => containers.as_slice(),
            Reading::Away => &[],
        };
        let mut placed: HashMap<PathBuf, Vec<Container>> = HashMap::new();
        let mut unplaced = Vec::new();
        for container in running {
            let projects = projects_for(&self.projects, container);
            if projects.is_empty() {
                unplaced.push(container.clone());
            }
            for project in projects {
                let here = placed.entry(project.to_path_buf()).or_default();
                here.push(container.clone());
            }
        }
        let mut changed = self.unplaced_containers != unplaced;
        self.unplaced_containers = unplaced;
        for (path, tracked) in &mut self.projects {
            let here = placed.remove(path).unwrap_or_default();
            if tracked.project.containers != here {
                tracked.project.containers = here;
                changed = true;
            }
        }
        changed
    }

    /// Docker as the map shows it: what its engine last said, with the containers for no project.
    pub(super) fn docker_on_the_map(&self) -> Option<Docker> {
        match &self.docker {
            Reading::Said { socket, disk, .. } => Some(Docker {
                socket: socket.clone(),
                containers: self.unplaced_containers.clone(),
                disk: *disk,
            }),
            Reading::Away => None,
        }
    }
}

/// The projects on the map `container` is for, each once: the one holding its compose folder, when
/// one does, or else each holding a folder it bind-mounts, by path.
fn projects_for<'a>(
    projects: &'a BTreeMap<PathBuf, Tracked>,
    container: &Container,
) -> Vec<&'a Path> {
    let compose = container.compose_folder.as_deref();
    if let Some(project) = compose.and_then(|folder| holder(projects, folder)) {
        return vec![project];
    }
    let mut found: Vec<&Path> = Vec::new();
    for folder in &container.bound {
        let project = holder(projects, folder);
        if let Some(project) = project.filter(|project| !found.contains(project)) {
            found.push(project);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;
    use std::time::SystemTime;

    use super::*;
    use crate::map::ScanOptions;
    use crate::map::engine::state::tracked::Tracked;
    use crate::map::grant::Roots;
    use crate::map::scan::Boundary;

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

    /// The running container `name`, made by compose in `compose` if given, bind-mounting `bound`.
    fn container(name: &str, compose: Option<&str>, bound: &[&str]) -> Container {
        Container {
            id: name.to_string(),
            name: name.to_string(),
            image: "postgres:17".to_string(),
            compose_folder: compose.map(PathBuf::from),
            bound: bound.iter().map(PathBuf::from).collect(),
            ports: Vec::new(),
            created: SystemTime::UNIX_EPOCH,
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

    #[test]
    fn a_container_compose_made_shows_on_the_project_compose_ran_in_and_no_other() {
        let mut state = state_holding(&["/code/app", "/code/app/api", "/code/site"]);
        let web = container("app-web-1", Some("/code/app"), &["/code/app", "/code/site"]);

        assert!(state.saw_docker(said(vec![web])));

        let on = |project: &str| names(&state.projects[Path::new(project)].project.containers);
        assert_eq!(on("/code/app"), ["app-web-1"]);
        assert_eq!(
            on("/code/site"),
            [""; 0],
            "it binds a folder there, but compose ran in app"
        );
    }

    #[test]
    fn a_container_shows_on_each_project_holding_a_folder_it_binds_and_else_on_dockers_card() {
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
        let docker = state.snapshot().docker.unwrap();
        assert_eq!(names(&docker.containers), ["pg", "cache"]);
        assert_eq!(docker.socket, Path::new("/run/docker.sock"));
    }

    #[test]
    fn a_project_that_joins_the_map_takes_its_containers_from_dockers_card() {
        let mut state = state_holding(&["/code/app"]);
        state.saw_docker(said(vec![container("web", None, &["/code/site"])]));
        assert_eq!(names(&state.snapshot().docker.unwrap().containers), ["web"]);

        let boundary = Boundary::new(&[], Path::new("/"), &ScanOptions::default());
        let site = Tracked::plain(PathBuf::from("/code/site"), &boundary, &|_| true);
        state.projects.insert(PathBuf::from("/code/site"), site);

        assert!(state.place_containers());
        assert_eq!(names(&state.snapshot().docker.unwrap().containers), [""; 0]);
        let on_site = &state.projects[Path::new("/code/site")].project.containers;
        assert_eq!(names(on_site), ["web"]);
        assert!(!state.place_containers(), "placed already");
    }

    #[test]
    fn docker_gone_takes_its_card_its_containers_and_its_problem_off_the_map() {
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
