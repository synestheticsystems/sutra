//! Dev-runner environments from both conventions: the shared `~/.dev-runner/` registry and a
//! project's own `<project>/.dev-runner/`, merged into one environment per project folder.

mod merge;

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use merge::FolderEnvironments;

use super::{FolderAccess, Source, SourceProblem, scan};
use crate::model::{self, EnvSources, Environment, Meta, ReadFailure};

/// Name of the per-project state folder.
pub const PROJECT_STATE_DIR: &str = ".dev-runner";

/// The per-project meta file. Its name is fixed because the folder it sits in already says which
/// project it describes, so no hash is needed.
const PROJECT_STATE_META: &str = "meta";

/// What one load of every environment found.
#[derive(Debug)]
pub struct Loaded {
    /// The environments of each folder any of them describes, sorted by folder.
    pub folders: Vec<FolderEnvironments>,
    /// A registry that could not be listed, then what could not be read of single environments.
    pub problems: Vec<SourceProblem>,
}

/// Load the registry in `home` and the own state folder of each of `readable_projects`, and of each
/// folder the registry names that may be read. A plain folder joins the map through such a
/// registration, so what it publishes itself counts from the first load that finds it.
pub fn load<'a>(
    home: &Path,
    readable_projects: impl IntoIterator<Item = &'a Path>,
    boundary: &scan::Boundary,
) -> Loaded {
    let mut failures = Vec::new();
    let (global, unlisted) = match load_global(home, &mut failures) {
        Ok(global) => (global, None),
        Err(problem) => (Vec::new(), Some(problem)),
    };
    let mut folders: BTreeSet<PathBuf> = readable_projects
        .into_iter()
        .map(Path::to_path_buf)
        .collect();
    for env in &global {
        let folder = merge::project_folder(&env.dir, boundary);
        if !folders.contains(&folder)
            && scan::folder_access(&folder, boundary) == FolderAccess::Readable
        {
            folders.insert(folder);
        }
    }
    let local = folders
        .iter()
        .filter_map(|folder| load_project_state(folder, &mut failures))
        .collect();
    let mut problems: Vec<SourceProblem> = unlisted.into_iter().collect();
    problems.extend(SourceProblem::per_kind(Source::EnvironmentState, failures));
    Loaded {
        folders: merge::by_folder(global, local, boundary),
        problems,
    }
}

/// Environments registered in `<home>/.dev-runner/`. No registry means no dev-runner script ran
/// here, and yields no environments; a registry that cannot be listed is a problem. What cannot be
/// read of one environment goes in `failures`.
fn load_global(
    home: &Path,
    failures: &mut Vec<ReadFailure>,
) -> Result<Vec<Environment>, SourceProblem> {
    let registry = model::registry_dir(home);
    match model::load_registry(&registry, failures) {
        Ok(envs) => Ok(envs),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(SourceProblem::new(Source::Registry, registry, err.kind())),
    }
}

/// The environment a project publishes in its own `.dev-runner/`, if any. What cannot be read of it
/// goes in `failures`, and what can still shows: a `meta` that cannot be read leaves the status
/// files, each of which stands alone.
fn load_project_state(project: &Path, failures: &mut Vec<ReadFailure>) -> Option<Environment> {
    let state_dir = project.join(PROJECT_STATE_DIR);
    let units = match model::read_units(&state_dir, project_state_unit_name, failures) {
        Ok(units) => units,
        // Most projects have no state folder, and a plain file of that name is not one either.
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return None;
        }
        Err(err) => {
            failures.push((state_dir, err.kind()));
            return None;
        }
    };
    // `DIR` is ignored: the project folder is the directory. A meta without a PID claims no
    // process, so it counts as absent and the status files still show.
    let process = model::read_state_file(&state_dir.join(PROJECT_STATE_META), failures)
        .map(|content| Meta::parse(&content))
        .and_then(|meta| Some((meta.pid?, meta.started.unwrap_or(0), meta.ports)));
    if process.is_none() && units.is_empty() {
        return None;
    }
    let dir = fs::canonicalize(project)
        .map_err(|err| model::note_failure(project, &err, failures))
        .ok()?;
    // Pid 0 is how `Environment` says that no meta file publishes a process.
    let (pid, started, ports) = process.unwrap_or((0, 0, HashMap::new()));
    Some(Environment {
        id: folder_id(&dir),
        dir,
        pid,
        ports,
        started,
        alive: model::process_exists(pid),
        units,
        sources: EnvSources {
            global: false,
            project: true,
        },
    })
}

/// The id of a merged environment: its canonical folder, so a unit keeps the same
/// `(environment id, unit name)` key whichever convention it came from.
fn folder_id(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
}

/// `<unit>.status` names a unit. Unit names hold no `.`, which also keeps the hash-prefixed files
/// of `~/.dev-runner/` from reading as units when the project is the home folder itself.
fn project_state_unit_name(file_name: &str) -> Option<&str> {
    file_name
        .strip_suffix(".status")
        .filter(|unit| !unit.is_empty() && !unit.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::ScanOptions;
    use crate::map::test_support::{DEAD_PID, TempDir};
    use crate::model::State;

    pub(super) fn units_of(env: &Environment) -> Vec<(&str, State)> {
        env.units
            .iter()
            .map(|unit| (unit.name.as_str(), unit.state.clone()))
            .collect()
    }

    /// `load_global` of a registry all of which reads.
    fn registered(home: &Path) -> Result<Vec<Environment>, SourceProblem> {
        let mut failures = Vec::new();
        let loaded = load_global(home, &mut failures);
        assert!(failures.is_empty(), "{failures:?}");
        loaded
    }

    /// `load_project_state` of a project all of whose state reads.
    fn published(project: &Path) -> Option<Environment> {
        let mut failures = Vec::new();
        let loaded = load_project_state(project, &mut failures);
        assert!(failures.is_empty(), "{failures:?}");
        loaded
    }

    #[test]
    fn load_global_reads_hash_named_metas_with_both_status_conventions() {
        let home = TempDir::new("devrunner-load-global");
        let own_pid = std::process::id();
        home.write(
            ".dev-runner/abc123",
            format!("DIR=/code/app\nPID={own_pid}\nSERVER_PORT=3000\n"),
        );
        home.write(".dev-runner/abc123.server.status", "ready");
        home.write(".dev-runner/.abc123.vite.status", "building: vite");
        home.write(".dev-runner/abc123.server.status.tmp", "failed");
        home.write(".dev-runner/def456", "DIR=/code/no-pid\n");

        let envs = registered(home.root()).unwrap();

        assert_eq!(envs.len(), 1, "the meta without a PID is skipped");
        let env = &envs[0];
        assert_eq!(env.id, "abc123");
        assert_eq!(env.dir, PathBuf::from("/code/app"));
        assert!(env.alive);
        assert_eq!(env.port_for("server"), Some(3000));
        assert_eq!(
            units_of(env),
            [("server", State::Ready), ("vite", State::Building)]
        );
        assert_eq!(
            env.sources,
            EnvSources {
                global: true,
                project: false
            }
        );
    }

    #[test]
    fn load_global_without_a_registry_yields_no_environments() {
        let home = TempDir::new("devrunner-load-global-none");
        assert!(registered(home.root()).unwrap().is_empty());
    }

    #[test]
    fn load_global_reports_a_registry_it_cannot_list() {
        let home = TempDir::new("devrunner-load-global-sealed");
        let registry = home.dir(".dev-runner");
        home.restrict(&registry, 0o000);

        let loaded = registered(home.root());

        assert_eq!(
            loaded.unwrap_err(),
            SourceProblem::new(Source::Registry, registry, io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn load_project_state_reads_meta_and_status_files_and_ignores_dir() {
        let project = TempDir::new("devrunner-project-state-full");
        let own_pid = std::process::id();
        project.write(
            ".dev-runner/meta",
            format!("DIR=/somewhere/else\nPID={own_pid}\nSTARTED=1700000000\nSERVER_PORT=3000\n"),
        );
        project.write(".dev-runner/server.status", "ready");
        project.write(".dev-runner/vite.status", "building: vite");

        let env = published(project.root()).unwrap();

        let canonical = project.root().to_path_buf();
        assert_eq!(env.id, canonical.to_string_lossy());
        assert_eq!(env.dir, canonical);
        assert_eq!(env.pid, own_pid);
        assert!(env.alive);
        assert_eq!(env.started, 1_700_000_000);
        assert_eq!(env.port_for("server"), Some(3000));
        assert_eq!(
            units_of(&env),
            [("server", State::Ready), ("vite", State::Building)]
        );
        assert_eq!(
            env.sources,
            EnvSources {
                global: false,
                project: true
            }
        );
    }

    #[test]
    fn load_project_state_without_meta_yields_a_status_only_environment() {
        let project = TempDir::new("devrunner-project-state-no-meta");
        project.write(".dev-runner/server.status", "failed: exit code 1");

        let env = published(project.root()).unwrap();

        assert_eq!(env.pid, 0);
        assert!(!env.alive);
        assert_eq!(env.started, 0);
        assert!(env.ports.is_empty());
        assert_eq!(units_of(&env), [("server", State::Failed)]);
    }

    #[test]
    fn load_project_state_treats_a_meta_without_a_valid_pid_as_absent() {
        let project = TempDir::new("devrunner-project-state-bad-pid");
        project.write(
            ".dev-runner/meta",
            "PID=not-a-number\nSTARTED=1700000000\nSERVER_PORT=3000\n",
        );
        project.write(".dev-runner/server.status", "running");

        let env = published(project.root()).unwrap();

        assert_eq!(env.pid, 0);
        assert!(!env.alive);
        assert_eq!(env.started, 0);
        assert!(env.ports.is_empty());
        assert_eq!(units_of(&env), [("server", State::Running)]);
    }

    #[test]
    fn load_project_state_reports_a_dead_pid_as_not_alive() {
        let project = TempDir::new("devrunner-project-state-dead");
        project.write(".dev-runner/meta", format!("PID={DEAD_PID}\n"));

        let env = published(project.root()).unwrap();

        assert_eq!(env.pid, DEAD_PID);
        assert!(!env.alive);
        assert!(env.units.is_empty());
    }

    #[test]
    fn load_project_state_ignores_files_that_are_not_unit_status_files() {
        let project = TempDir::new("devrunner-project-state-junk");
        // A hash-prefixed status file, as in `~/.dev-runner/` when the project is the home folder.
        project.write(".dev-runner/abc123.server.status", "ready");
        project.write(".dev-runner/server.status.tmp", "ready");
        project.write(".dev-runner/.status", "ready");
        project.write(".dev-runner/notes.txt", "hello");

        assert!(published(project.root()).is_none());
    }

    #[test]
    fn load_project_state_without_a_state_folder_or_with_an_empty_one_is_none() {
        let project = TempDir::new("devrunner-project-state-none");
        assert!(published(project.root()).is_none());
        project.dir(".dev-runner");
        assert!(published(project.root()).is_none());
    }

    #[test]
    fn load_project_state_takes_a_plain_file_for_no_state_folder() {
        let project = TempDir::new("devrunner-project-state-plain-file");
        project.write(".dev-runner", "not a folder");
        assert!(published(project.root()).is_none());
    }

    #[test]
    fn load_project_state_reports_a_state_folder_it_cannot_list() {
        let project = TempDir::new("devrunner-project-state-sealed");
        let state = project.dir(".dev-runner");
        project.restrict(&state, 0o000);
        let mut failures = Vec::new();

        assert!(load_project_state(project.root(), &mut failures).is_none());

        assert_eq!(failures, [(state, io::ErrorKind::PermissionDenied)]);
    }

    #[test]
    fn load_project_state_keeps_the_status_files_of_a_meta_it_cannot_read_and_reports_the_meta() {
        let project = TempDir::new("devrunner-project-state-sealed-meta");
        let meta = project.write(".dev-runner/meta", "PID=1\n");
        project.restrict(&meta, 0o000);
        project.write(".dev-runner/server.status", "ready");
        let mut failures = Vec::new();

        let env = load_project_state(project.root(), &mut failures).unwrap();

        assert_eq!(env.pid, 0);
        assert_eq!(units_of(&env), [("server", State::Ready)]);
        assert_eq!(failures, [(meta, io::ErrorKind::PermissionDenied)]);
    }

    #[test]
    fn load_project_state_keeps_the_meta_and_other_units_of_a_status_file_it_cannot_read() {
        let project = TempDir::new("devrunner-project-state-sealed-status");
        let own_pid = std::process::id();
        project.write(".dev-runner/meta", format!("PID={own_pid}\n"));
        project.write(".dev-runner/server.status", "ready");
        let sealed = project.write(".dev-runner/vite.status", "ready");
        project.restrict(&sealed, 0o000);
        let mut failures = Vec::new();

        let env = load_project_state(project.root(), &mut failures).unwrap();

        assert_eq!(env.pid, own_pid);
        assert_eq!(units_of(&env), [("server", State::Ready)]);
        assert_eq!(failures, [(sealed, io::ErrorKind::PermissionDenied)]);
    }

    /// What `load` makes of a home folder with no project on the map yet.
    fn load_home(home: &TempDir) -> Loaded {
        load(home.root(), [], &home.boundary(&ScanOptions::default()))
    }

    #[test]
    fn load_tells_a_registry_it_cannot_list_from_one_environment_it_cannot_read_in_full() {
        let home = TempDir::new("devrunner-load-whole-or-part");
        let registry = home.dir(".dev-runner");
        home.write(".dev-runner/ab12", "DIR=/code/app\nPID=1\n");
        let sealed = home.write(".dev-runner/ab12.server.status", "ready");
        home.restrict(&sealed, 0o000);

        let part = load_home(&home);

        assert_eq!(part.folders.len(), 1, "the environment still shows");
        assert_eq!(
            part.problems,
            [SourceProblem::new(
                Source::EnvironmentState,
                sealed,
                io::ErrorKind::PermissionDenied
            )]
        );

        home.restrict(&registry, 0o000);

        assert_eq!(
            load_home(&home).problems,
            [SourceProblem::new(
                Source::Registry,
                registry,
                io::ErrorKind::PermissionDenied
            )]
        );
    }

    #[test]
    fn load_reads_the_own_state_of_a_plain_folder_the_registry_names() {
        let home = TempDir::new("devrunner-load-plain-folder");
        let own_pid = std::process::id();
        let blog = home.dir("sites/blog");
        home.write(
            ".dev-runner/ab12",
            format!("DIR={}\nPID={own_pid}\n", blog.display()),
        );
        home.write("sites/blog/.dev-runner/vite.status", "ready");

        let loaded = load_home(&home);

        assert_eq!(loaded.folders.len(), 1);
        assert_eq!(loaded.folders[0].dir(), blog);
        let env = loaded.folders[0].merged();
        assert!(env.sources.global && env.sources.project);
        assert_eq!(env.pid, own_pid);
        assert_eq!(units_of(&env), [("vite", State::Ready)]);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
    }

    #[test]
    fn load_does_not_enter_a_folder_the_registry_names_in_a_private_folder() {
        let home = TempDir::new("devrunner-load-private-folder");
        let private = home.dir("Documents/app");
        home.write(
            ".dev-runner/ab12",
            format!("DIR={}\nPID=1\n", private.display()),
        );
        home.write("Documents/app/.dev-runner/vite.status", "ready");

        let loaded = load_home(&home);

        assert_eq!(loaded.folders.len(), 1);
        let env = loaded.folders[0].merged();
        assert!(env.sources.global && !env.sources.project);
        assert!(env.units.is_empty());
    }
}
