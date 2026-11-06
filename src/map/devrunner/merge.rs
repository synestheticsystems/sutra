//! Merging the environments both conventions publish into one per project folder.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use super::folder_id;
use crate::map::scan::Boundary;
use crate::model::{self, EnvSources, Environment};

/// Every environment that describes one project folder, as their state files last said, so that a
/// process dying can change which of them speaks for the folder without those files being read
/// again.
#[derive(Debug)]
pub struct FolderEnvironments {
    dir: PathBuf,
    describing: Vec<Environment>,
}

impl FolderEnvironments {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The one environment the folder shows (see `merge_folder`).
    pub fn merged(&self) -> Environment {
        merge_folder(self.dir.clone(), self.describing.clone())
    }

    /// Probe every process these environments name. Returns whether any of them died or came back
    /// since the last probe, which can change what they merge into.
    pub fn probe_processes(&mut self) -> bool {
        let mut changed = false;
        for env in &mut self.describing {
            let alive = model::process_exists(env.pid);
            changed |= env.alive != alive;
            env.alive = alive;
        }
        changed
    }
}

/// The environments of each canonical project folder, sorted by folder.
pub(super) fn by_folder(
    global: Vec<Environment>,
    local: Vec<Environment>,
    boundary: &Boundary,
) -> Vec<FolderEnvironments> {
    let mut by_dir: BTreeMap<PathBuf, Vec<Environment>> = BTreeMap::new();
    for env in global.into_iter().chain(local) {
        by_dir
            .entry(project_folder(&env.dir, boundary))
            .or_default()
            .push(env);
    }
    by_dir
        .into_iter()
        .map(|(dir, describing)| FolderEnvironments { dir, describing })
        .collect()
}

/// The folder an environment that published `dir` describes: `dir` resolved, where `boundary`
/// allows. The path the environment published is the best key when resolving it would raise the
/// privacy prompt, or when its folder is gone (a dead environment's often is).
pub(super) fn project_folder(dir: &Path, boundary: &Boundary) -> PathBuf {
    Some(dir)
        .filter(|dir| boundary.may_read(dir))
        .and_then(|dir| fs::canonicalize(dir).ok())
        .unwrap_or_else(|| dir.to_path_buf())
}

/// Fold every environment that describes one folder into one. The fold runs from the lowest
/// precedence up, so each value comes from the highest-precedence environment that has it.
/// An environment whose process is dead ranks below one that is live or claims no process (a
/// project state folder holding status files only, pid 0): a meta left behind by a killed script is
/// stale, however specific its folder. Then a project's own state beats the global registry,
/// because the project's own folder is the more specific claim, and a later start beats an earlier
/// one, because two global metas for one folder are a stale registration and a current one. While
/// any environment is live, the units only dead ones describe are left out: they are what the stale
/// runs last wrote.
fn merge_folder(dir: PathBuf, mut group: Vec<Environment>) -> Environment {
    let is_dead = |env: &Environment| env.pid != 0 && !env.alive;
    group.sort_by_key(|env| (!is_dead(env), env.sources.project, env.started));
    let any_live = group.iter().any(|env| env.alive);
    let mut merged = Environment {
        id: folder_id(&dir),
        dir,
        pid: 0,
        ports: HashMap::new(),
        started: 0,
        alive: false,
        units: Vec::new(),
        sources: EnvSources::default(),
    };
    let mut units = BTreeMap::new();
    for env in group {
        merged.sources.global |= env.sources.global;
        merged.sources.project |= env.sources.project;
        if !(any_live && is_dead(&env)) {
            units.extend(env.units.into_iter().map(|unit| (unit.name.clone(), unit)));
        }
        // A project state folder holding status files only (pid 0) publishes no process, so the
        // process fields stay with an environment that does.
        if env.pid != 0 {
            merged.pid = env.pid;
            merged.started = env.started;
            merged.ports = env.ports;
            merged.alive = env.alive;
        }
    }
    merged.units = units.into_values().collect();
    merged
}

#[cfg(test)]
mod tests {
    use super::super::tests::units_of;
    use super::*;
    use crate::map::test_support::{DEAD_PID, LiveProcess, TempDir, environment_at};
    use crate::map::{PrivateFolder, ScanOptions};
    use crate::model::{State, UnitStatus};

    /// Each folder's merged environment, with no folder off limits.
    fn merge_in(
        fixture: &TempDir,
        global: Vec<Environment>,
        local: Vec<Environment>,
    ) -> Vec<Environment> {
        merged(global, local, &fixture.boundary(&ScanOptions::default()))
    }

    fn merged(
        global: Vec<Environment>,
        local: Vec<Environment>,
        boundary: &Boundary,
    ) -> Vec<Environment> {
        by_folder(global, local, boundary)
            .iter()
            .map(FolderEnvironments::merged)
            .collect()
    }

    fn env(
        dir: PathBuf,
        project_state: bool,
        pid: u32,
        started: u64,
        units: &[(&str, &str)],
    ) -> Environment {
        Environment {
            id: if project_state {
                folder_id(&dir)
            } else {
                "abc123".into()
            },
            pid,
            started,
            alive: crate::model::process_exists(pid),
            units: units
                .iter()
                .map(|(name, status)| UnitStatus::parse(name, status))
                .collect(),
            sources: EnvSources {
                global: !project_state,
                project: project_state,
            },
            ..environment_at(dir)
        }
    }

    #[test]
    fn merge_of_a_global_environment_alone_renames_it_to_its_canonical_folder() {
        let fixture = TempDir::new("devrunner-merge-global-only");
        fixture.dir("app");
        std::os::unix::fs::symlink(fixture.path("app"), fixture.path("link")).unwrap();
        let global = env(
            fixture.path("link"),
            false,
            DEAD_PID,
            5,
            &[("server", "ready")],
        );

        let merged = merge_in(&fixture, vec![global], Vec::new());

        assert_eq!(merged.len(), 1);
        let canonical = fixture.path("app");
        assert_eq!(merged[0].id, canonical.to_string_lossy());
        assert_eq!(merged[0].dir, canonical);
        assert_eq!(merged[0].pid, DEAD_PID);
        assert_eq!(merged[0].started, 5);
        assert_eq!(units_of(&merged[0]), [("server", State::Ready)]);
        assert_eq!(
            merged[0].sources,
            EnvSources {
                global: true,
                project: false
            }
        );
    }

    #[test]
    fn merge_of_a_project_state_environment_alone_keeps_its_values() {
        let fixture = TempDir::new("devrunner-merge-local-only");
        let dir = fs::canonicalize(fixture.dir("app")).unwrap();
        let own_pid = std::process::id();
        let local = env(dir.clone(), true, own_pid, 7, &[("vite", "ready")]);

        let merged = merge_in(&fixture, Vec::new(), vec![local]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, dir.to_string_lossy());
        assert_eq!(merged[0].pid, own_pid);
        assert!(merged[0].alive);
        assert_eq!(units_of(&merged[0]), [("vite", State::Ready)]);
        assert_eq!(
            merged[0].sources,
            EnvSources {
                global: false,
                project: true
            }
        );
    }

    #[test]
    fn merge_unites_units_by_name_and_takes_process_fields_from_the_project_state() {
        let fixture = TempDir::new("devrunner-merge-both");
        fixture.dir("app");
        std::os::unix::fs::symlink(fixture.path("app"), fixture.path("link")).unwrap();
        let own_pid = std::process::id();
        let mut global = env(
            fixture.path("link"),
            false,
            1,
            100,
            &[("server", "running"), ("wasm", "building")],
        );
        global.ports.insert("server".into(), 3000);
        let mut local = env(
            fixture.path("app"),
            true,
            own_pid,
            200,
            &[("server", "ready"), ("vite", "ready")],
        );
        local.ports.insert("vite".into(), 5173);

        let merged = merge_in(&fixture, vec![global], vec![local]);

        assert_eq!(merged.len(), 1, "both describe the same canonical folder");
        let env = &merged[0];
        assert_eq!(env.id, fixture.path("app").to_string_lossy());
        assert_eq!(
            units_of(env),
            [
                ("server", State::Ready),
                ("vite", State::Ready),
                ("wasm", State::Building)
            ]
        );
        assert_eq!(env.pid, own_pid);
        assert_eq!(env.started, 200);
        assert_eq!(env.ports, HashMap::from([("vite".to_string(), 5173)]));
        assert_eq!(
            env.sources,
            EnvSources {
                global: true,
                project: true
            }
        );
    }

    #[test]
    fn merge_keeps_the_global_process_when_the_project_state_has_no_meta() {
        let fixture = TempDir::new("devrunner-merge-status-only");
        let dir = fs::canonicalize(fixture.dir("app")).unwrap();
        let own_pid = std::process::id();
        let mut global = env(dir.clone(), false, own_pid, 100, &[("server", "running")]);
        global.ports.insert("server".into(), 3000);
        let local = env(dir, true, 0, 0, &[("server", "failed")]);

        let merged = merge_in(&fixture, vec![global], vec![local]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].pid, own_pid);
        assert!(merged[0].alive);
        assert_eq!(merged[0].started, 100);
        assert_eq!(merged[0].port_for("server"), Some(3000));
        assert_eq!(units_of(&merged[0]), [("server", State::Failed)]);
    }

    #[test]
    fn merge_prefers_a_live_global_registration_over_a_stale_one_and_drops_the_stale_units() {
        let fixture = TempDir::new("devrunner-merge-stale-global");
        fixture.dir("app");
        std::os::unix::fs::symlink(fixture.path("app"), fixture.path("link")).unwrap();
        let own_pid = std::process::id();
        let live = env(
            fixture.path("app"),
            false,
            own_pid,
            100,
            &[("server", "ready")],
        );
        let stale = env(
            fixture.path("link"),
            false,
            DEAD_PID,
            300,
            &[("worker", "failed")],
        );

        let merged = merge_in(&fixture, vec![live, stale], Vec::new());

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].pid, own_pid);
        assert!(merged[0].alive);
        assert_eq!(units_of(&merged[0]), [("server", State::Ready)]);
    }

    #[test]
    fn merge_prefers_a_live_global_process_over_a_dead_project_meta() {
        let fixture = TempDir::new("devrunner-merge-dead-local");
        let dir = fs::canonicalize(fixture.dir("app")).unwrap();
        let own_pid = std::process::id();
        let mut global = env(dir.clone(), false, own_pid, 100, &[("server", "ready")]);
        global.ports.insert("server".into(), 3000);
        let local = env(
            dir,
            true,
            DEAD_PID,
            500,
            &[("server", "failed"), ("vite", "building")],
        );

        let merged = merge_in(&fixture, vec![global], vec![local]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].pid, own_pid);
        assert!(merged[0].alive);
        assert_eq!(merged[0].started, 100);
        assert_eq!(merged[0].port_for("server"), Some(3000));
        assert_eq!(units_of(&merged[0]), [("server", State::Ready)]);
    }

    #[test]
    fn merge_unites_the_units_of_dead_environments_when_none_is_live() {
        let fixture = TempDir::new("devrunner-merge-all-dead");
        let dir = fs::canonicalize(fixture.dir("app")).unwrap();
        let global = env(dir.clone(), false, DEAD_PID, 100, &[("server", "ready")]);
        let local = env(dir, true, DEAD_PID, 50, &[("vite", "failed")]);

        let merged = merge_in(&fixture, vec![global], vec![local]);

        assert_eq!(merged.len(), 1);
        assert!(!merged[0].alive);
        assert_eq!(
            merged[0].started, 50,
            "the project's own meta is the more specific claim"
        );
        assert_eq!(
            units_of(&merged[0]),
            [("server", State::Ready), ("vite", State::Failed)]
        );
    }

    #[test]
    fn merge_keeps_the_published_folder_inside_a_private_folder_unless_settings_let_it_in() {
        let fixture = TempDir::new("devrunner-merge-private");
        let app = fixture.root().to_path_buf().join("Desktop").join("app");
        fs::create_dir_all(&app).unwrap();
        let published = fixture.root().to_path_buf().join("Desktop").join("link");
        std::os::unix::fs::symlink(&app, &published).unwrap();
        let global = || env(published.clone(), false, DEAD_PID, 0, &[]);
        let desktop = ScanOptions {
            private_folders: [PrivateFolder::Desktop].into(),
        };

        let kept = merged(
            vec![global()],
            Vec::new(),
            &fixture.boundary(&ScanOptions::default()),
        );
        let resolved = merged(vec![global()], Vec::new(), &fixture.boundary(&desktop));

        assert_eq!(kept[0].dir, published);
        assert_eq!(resolved[0].dir, app);
    }

    #[test]
    fn a_probe_that_finds_every_process_dead_leaves_the_environment_stopped_with_all_its_units() {
        let fixture = TempDir::new("devrunner-probe-all-dead");
        let dir = fs::canonicalize(fixture.dir("app")).unwrap();
        let last = LiveProcess::start();
        let global = env(dir.clone(), false, DEAD_PID, 100, &[("server", "ready")]);
        let local = env(dir, true, last.pid(), 200, &[("vite", "ready")]);
        let mut folders = by_folder(
            vec![global],
            vec![local],
            &fixture.boundary(&ScanOptions::default()),
        );
        let folder = &mut folders[0];
        assert_eq!(units_of(&folder.merged()), [("vite", State::Ready)]);

        drop(last);

        assert!(folder.probe_processes());
        let merged = folder.merged();
        assert!(merged.has_stopped());
        assert_eq!(
            units_of(&merged),
            [("server", State::Ready), ("vite", State::Ready)]
        );
    }

    #[test]
    fn a_probe_of_a_status_only_folder_has_nothing_to_find() {
        let fixture = TempDir::new("devrunner-probe-status-only");
        let dir = fs::canonicalize(fixture.dir("app")).unwrap();
        let mut folders = by_folder(
            Vec::new(),
            vec![env(dir, true, 0, 0, &[("server", "failed")])],
            &fixture.boundary(&ScanOptions::default()),
        );

        assert!(!folders[0].probe_processes());
        assert!(!folders[0].merged().has_stopped());
    }

    #[test]
    fn a_live_process_that_dies_loses_its_units_while_the_winner_lives_on() {
        let fixture = TempDir::new("devrunner-probe-runner-up");
        let dir = fs::canonicalize(fixture.dir("app")).unwrap();
        let own_pid = std::process::id();
        let runner_up = LiveProcess::start();
        let global = env(
            dir.clone(),
            false,
            runner_up.pid(),
            100,
            &[("worker", "ready")],
        );
        let local = env(dir, true, own_pid, 200, &[("server", "ready")]);
        let mut folders = by_folder(
            vec![global],
            vec![local],
            &fixture.boundary(&ScanOptions::default()),
        );
        let folder = &mut folders[0];
        assert_eq!(
            units_of(&folder.merged()),
            [("server", State::Ready), ("worker", State::Ready)]
        );

        drop(runner_up);

        assert!(folder.probe_processes());
        let merged = folder.merged();
        assert_eq!(merged.pid, own_pid);
        assert_eq!(units_of(&merged), [("server", State::Ready)]);
    }

    #[test]
    fn merge_keeps_different_folders_apart_sorted_by_canonical_folder() {
        let fixture = TempDir::new("devrunner-merge-sorted");
        let a = fs::canonicalize(fixture.dir("a")).unwrap();
        let b = fs::canonicalize(fixture.dir("b")).unwrap();
        let global = env(b.clone(), false, DEAD_PID, 0, &[]);
        let local = env(a.clone(), true, DEAD_PID, 0, &[]);

        let merged = merge_in(&fixture, vec![global], vec![local]);

        let dirs: Vec<&Path> = merged.iter().map(|env| env.dir.as_path()).collect();
        assert_eq!(dirs, [a.as_path(), b.as_path()]);
    }
}
