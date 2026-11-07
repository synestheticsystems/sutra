//! Placing the user's processes that listen for connections: each on the deepest project holding
//! the folder it runs in, where its port shows on the project's card. A process that runs in no
//! project's folder is left off the map: most are apps and their helpers, which run in `/` or in
//! their own containers, and would bury the projects.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::State;
use super::tracked::Tracked;
use crate::map::processes::{self, ListeningProcess};
use crate::map::{Source, SourceProblem};

impl State {
    /// Read the processes listening in the projects' folders, and place each on the deepest project
    /// holding the folder it runs in. Returns whether anything changed. When macOS will not list
    /// the processes, none shows, and the map says why.
    pub(in crate::map::engine) fn refresh_processes(&mut self) -> bool {
        let projects = &self.projects;
        let read = processes::listening(
            |folder| holder(projects, folder).is_some(),
            &mut self.listener_commands,
        );
        let (found, problems) = match read {
            Ok(found) => (found, Vec::new()),
            Err(err) => {
                let unlisted = SourceProblem::new(Source::Processes, PathBuf::new(), err.kind());
                (Vec::new(), vec![unlisted])
            }
        };
        let mut changed = self.report_problems(&[Source::Processes], problems);
        let mut placed: HashMap<PathBuf, Vec<ListeningProcess>> = HashMap::new();
        for process in found {
            if let Some(project) = holder(&self.projects, &process.folder) {
                placed
                    .entry(project.to_path_buf())
                    .or_default()
                    .push(process);
            }
        }
        for (path, tracked) in &mut self.projects {
            let here = placed.remove(path).unwrap_or_default();
            if tracked.project.listening != here {
                tracked.project.listening = here;
                changed = true;
            }
        }
        changed
    }
}

/// The deepest project on the map holding `folder`, itself included.
fn holder<'a>(projects: &'a BTreeMap<PathBuf, Tracked>, folder: &Path) -> Option<&'a Path> {
    let mut held = folder
        .ancestors()
        .filter_map(|ancestor| projects.get_key_value(ancestor));
    held.next().map(|(path, _)| path.as_path())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::net::{Ipv4Addr, TcpListener};

    use super::*;
    use crate::map::ScanOptions;
    use crate::map::grant::Roots;
    use crate::map::scan::Boundary;

    /// The map's projects at `paths`, none of them on disk.
    fn projects(paths: &[&Path]) -> BTreeMap<PathBuf, Tracked> {
        let boundary = Boundary::new(&[], Path::new("/"), &ScanOptions::default());
        let tracked = |path: &&Path| Tracked::plain(path.to_path_buf(), &boundary);
        let paths = paths.iter();
        paths
            .map(|path| (path.to_path_buf(), tracked(path)))
            .collect()
    }

    #[test]
    fn a_folder_belongs_to_the_deepest_project_holding_it_and_to_none_when_none_holds_it() {
        let projects = projects(&[Path::new("/code/app"), Path::new("/code/app/.claude/wt")]);
        let holder = |folder: &str| holder(&projects, Path::new(folder));
        assert_eq!(holder("/code/app/web"), Some(Path::new("/code/app")));
        assert_eq!(holder("/code/app"), Some(Path::new("/code/app")));
        assert_eq!(
            holder("/code/app/.claude/wt/server"),
            Some(Path::new("/code/app/.claude/wt"))
        );
        assert_eq!(
            holder("/code"),
            None,
            "a folder holding a project is no project's"
        );
        assert_eq!(holder("/"), None);
    }

    /// A map of no root, under a home that is no folder, holding `projects` alone.
    fn state_holding(projects: &[&Path]) -> State {
        let roots = Roots {
            home: PathBuf::from("/nonexistent/home"),
            dirs: Vec::new(),
            scratch: None,
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());
        state.projects = self::projects(projects);
        state
    }

    #[test]
    fn a_process_listening_in_a_projects_folder_shows_on_its_card_until_it_stops() {
        // Cargo runs the tests in the folder of the package, which stands for a project here.
        let package = std::env::current_dir().unwrap().canonicalize().unwrap();
        let mut state = state_holding(&[package.parent().unwrap(), &package]);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let listening_here = |state: &State| {
            let project = &state.projects[&package].project;
            let me = project
                .listening
                .iter()
                .find(|p| p.pid == std::process::id());
            me.is_some_and(|me| me.ports().contains(&port))
        };

        assert!(state.refresh_processes());

        assert!(listening_here(&state), "on the deepest project holding it");
        let around = &state.projects[package.parent().unwrap()].project;
        assert!(around.listening.iter().all(|p| p.pid != std::process::id()));
        drop(listener);
        state.refresh_processes();
        assert!(!listening_here(&state), "gone once it stops listening");
    }
}
