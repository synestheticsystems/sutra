//! Which part of the map a file event concerns, decided from its path alone. FSEvents reports
//! canonical paths, and every path here is canonical, so routing is prefix matching.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::map::{Project, agents, devrunner};
use crate::model;

/// What to refresh for one event path.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Route {
    Ignore,
    /// A Claude Code session registry file, or the transcript of a session's subagent.
    ClaudeSessions,
    /// Codex's thread database or one of its rollout files.
    Codex,
    /// A dev-runner state folder: the global registry or a project's own.
    Environments,
    /// The git state of these worktrees.
    Git(Vec<PathBuf>),
    /// A worktree may have been added or removed.
    Rediscover,
    /// The contents of this artifact directory.
    Artifact(PathBuf),
    /// A file in this project changed.
    FileChange(PathBuf),
    /// In a root but in no project: nothing to refresh, unless a folder was moved here.
    OutsideProjects,
}

/// The paths events are matched against, rebuilt whenever the set of projects or artifacts
/// changes.
#[derive(Debug, Default)]
pub(super) struct Index {
    home: HomePaths,
    projects: HashSet<PathBuf>,
    /// Each artifact directory and the project that holds it.
    artifacts: HashMap<PathBuf, PathBuf>,
    /// Every git dir of every known worktree, linked worktrees' admin dirs and shared dirs alike.
    git_dirs: HashMap<PathBuf, GitDir>,
}

/// The folders in the home folder that belong to a source rather than to a project.
#[derive(Debug, Default)]
struct HomePaths {
    /// System state and the trash: the busiest folders in a home stream, with nothing for the map.
    ignored: [PathBuf; 2],
    claude_sessions: PathBuf,
    /// Claude Code's transcripts, of which only the subagents' route to a source.
    claude_projects: PathBuf,
    codex: PathBuf,
    dev_runner: PathBuf,
}

#[derive(Debug)]
struct GitDir {
    /// A repository's shared dir (`common_dir`), rather than one linked worktree's admin dir.
    shared: bool,
    /// The worktrees whose state lives here: all of the repository's for a shared dir.
    worktrees: Vec<PathBuf>,
}

impl Index {
    /// `home` is canonical, like every project and artifact path.
    pub(super) fn new<'a>(home: &Path, projects: impl IntoIterator<Item = &'a Project>) -> Index {
        let mut index = Index {
            home: HomePaths {
                ignored: [home.join("Library"), home.join(".Trash")],
                claude_sessions: home.join(".claude").join("sessions"),
                claude_projects: home.join(".claude").join("projects"),
                codex: home.join(".codex"),
                dev_runner: model::registry_dir(home),
            },
            ..Index::default()
        };
        for project in projects {
            index.projects.insert(project.path.clone());
            for artifact in &project.artifacts {
                index
                    .artifacts
                    .insert(artifact.path.clone(), project.path.clone());
            }
            let Some(repo) = &project.repo else {
                continue;
            };
            index
                .git_dirs
                .entry(repo.common_dir.clone())
                .or_insert_with(|| GitDir {
                    shared: true,
                    worktrees: Vec::new(),
                })
                .worktrees
                .push(project.path.clone());
            if repo.git_dir != repo.common_dir {
                index.git_dirs.insert(
                    repo.git_dir.clone(),
                    GitDir {
                        shared: false,
                        worktrees: vec![project.path.clone()],
                    },
                );
            }
        }
        index
    }

    /// The project that holds `artifact`, if it is on the map.
    pub(super) fn artifact_owner(&self, artifact: &Path) -> Option<&Path> {
        self.artifacts.get(artifact).map(PathBuf::as_path)
    }

    pub(super) fn route(&self, path: &Path) -> Route {
        if let Some(route) = self.home.route(path) {
            return route;
        }
        let lock_file = path.extension() == Some(OsStr::new("lock"));
        // A lock file in a state folder publishes nothing, so it is no reason to read the state
        // folders again, nor to look for a project publishing state.
        if lock_file && in_state_folder(path) {
            return Route::Ignore;
        }
        // The deepest known folder holding the path decides: a linked worktree inside its main
        // worktree, or a `target/` inside a project, is matched before the folder around it.
        for folder in path.ancestors() {
            if let Some(git_dir) = self.git_dirs.get(folder) {
                // Git takes a lock file for every ref and index update; the rename that ends the
                // update is the event worth acting on.
                if lock_file {
                    return Route::Ignore;
                }
                return git_dir.route(below(path, folder));
            }
            if self.artifacts.contains_key(folder) {
                return Route::Artifact(folder.to_path_buf());
            }
            if self.projects.contains(folder) {
                return project_route(path, folder);
            }
        }
        // `git init`, `git clone` and `git worktree add` inside a root all create a `.git` entry,
        // and a plain folder becomes a project by publishing state of its own.
        if path.file_name() == Some(OsStr::new(".git")) || in_state_folder(path) {
            Route::Rediscover
        } else {
            Route::OutsideProjects
        }
    }
}

impl HomePaths {
    fn route(&self, path: &Path) -> Option<Route> {
        if self.ignored.iter().any(|folder| path.starts_with(folder)) {
            return Some(Route::Ignore);
        }
        if path.starts_with(&self.claude_sessions) {
            return Some(Route::ClaudeSessions);
        }
        // Of Claude Code's transcripts only its subagents' tell the map anything: a session's own
        // adds nothing to its registry file, so it routes like any other path.
        let below_projects = path.strip_prefix(&self.claude_projects);
        if below_projects.is_ok_and(agents::is_subagent_transcript) {
            return Some(Route::ClaudeSessions);
        }
        if path.starts_with(&self.dev_runner) {
            return Some(Route::Environments);
        }
        // Other folders under `~/.codex` can hold projects (Codex's own worktrees), so only the
        // thread database and the rollouts route here.
        let in_codex = path.starts_with(self.codex.join("sessions"))
            || (path.parent() == Some(self.codex.as_path())
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("state_5.sqlite")));
        in_codex.then_some(Route::Codex)
    }
}

impl GitDir {
    /// `rest` is the event path below this git dir.
    fn route(&self, rest: Option<&OsStr>) -> Route {
        match rest {
            // The git dir itself was created or removed.
            None => Route::Rediscover,
            Some(first) if self.shared && first == "objects" => Route::Ignore,
            // A linked worktree's admin dir that no known worktree owns: one is being added or
            // removed.
            Some(first) if self.shared && first == "worktrees" => Route::Rediscover,
            Some(_) => Route::Git(self.worktrees.clone()),
        }
    }
}

fn project_route(path: &Path, project: &Path) -> Route {
    match below(path, project) {
        // The project folder itself was created, renamed or removed.
        None => Route::Rediscover,
        Some(first) if first == devrunner::PROJECT_STATE_DIR => Route::Environments,
        // A linked worktree's `.git` file, which only `git worktree` rewrites.
        Some(first) if first == ".git" => Route::Ignore,
        Some(_) => Route::FileChange(project.to_path_buf()),
    }
}

fn in_state_folder(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str() == devrunner::PROJECT_STATE_DIR)
}

/// The first component of `path` below `folder`, or `None` when they are the same.
fn below<'a>(path: &'a Path, folder: &Path) -> Option<&'a OsStr> {
    match path.strip_prefix(folder).ok()?.components().next()? {
        Component::Normal(name) => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{cargo_target, project_at};
    use crate::map::{Measurement, RepoInfo};

    const HOME: &str = "/Users/me";

    fn p(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    fn project(path: &str, repo: Option<(&str, &str)>, artifacts: &[&str]) -> Project {
        Project {
            repo: repo.map(|(common_dir, git_dir)| RepoInfo {
                common_dir: p(common_dir),
                git_dir: p(git_dir),
                worktree_name: None,
                branch: None,
                head: None,
            }),
            artifacts: artifacts
                .iter()
                .map(|artifact| cargo_target(*artifact, Measurement::Pending))
                .collect(),
            ..project_at(path)
        }
    }

    /// A main worktree `app` with a `target/`, a linked worktree `app-wt` beside it, a linked
    /// worktree nested inside `app`, and a plain folder `site`.
    fn index() -> Index {
        let common = "/Users/me/code/app/.git";
        let projects = [
            project(
                "/Users/me/code/app",
                Some((common, common)),
                &["/Users/me/code/app/target"],
            ),
            project(
                "/Users/me/code/app-wt",
                Some((common, "/Users/me/code/app/.git/worktrees/app-wt")),
                &[],
            ),
            project(
                "/Users/me/code/app/.claude/worktrees/nested",
                Some((common, "/Users/me/code/app/.git/worktrees/nested")),
                &[],
            ),
            project("/Users/me/code/site", None, &[]),
        ];
        Index::new(Path::new(HOME), &projects)
    }

    fn route(path: &str) -> Route {
        index().route(Path::new(path))
    }

    #[test]
    fn library_and_trash_are_ignored() {
        assert_eq!(route("/Users/me/Library/Caches/x"), Route::Ignore);
        assert_eq!(route("/Users/me/.Trash/app"), Route::Ignore);
    }

    #[test]
    fn agent_and_environment_state_route_to_their_source() {
        assert_eq!(
            route("/Users/me/.claude/sessions/123.json"),
            Route::ClaudeSessions
        );
        assert_eq!(route("/Users/me/.codex/state_5.sqlite-wal"), Route::Codex);
        assert_eq!(
            route("/Users/me/.codex/sessions/2026/11/03/rollout.jsonl"),
            Route::Codex
        );
        assert_eq!(
            route("/Users/me/.dev-runner/ab12.server.status"),
            Route::Environments
        );
        assert_eq!(
            route("/Users/me/code/site/.dev-runner/server.status"),
            Route::Environments
        );
    }

    #[test]
    fn a_subagents_transcript_refreshes_claude_sessions_and_the_files_beside_it_do_not() {
        let session = "/Users/me/.claude/projects/-Users-me-code-app/0b1d";
        for transcript in [
            "subagents/agent-a1.jsonl",
            "subagents/workflows/wf_9c/agent-a2.jsonl",
        ] {
            let path = format!("{session}/{transcript}");
            assert_eq!(route(&path), Route::ClaudeSessions, "{path}");
        }
        for beside in [
            ".jsonl",
            "/subagents/agent-a1.meta.json",
            "/tool-results/x.txt",
        ] {
            let path = format!("{session}{beside}");
            assert_eq!(route(&path), Route::OutsideProjects, "{path}");
        }
    }

    #[test]
    fn other_codex_folders_route_like_any_other_folder() {
        assert_eq!(
            route("/Users/me/.codex/log/codex.log"),
            Route::OutsideProjects
        );
        assert_eq!(
            route("/Users/me/.claude/projects/x.jsonl"),
            Route::OutsideProjects
        );
    }

    #[test]
    fn a_shared_git_dir_change_refreshes_every_worktree_of_the_repo() {
        assert_eq!(
            route("/Users/me/code/app/.git/refs/heads/main"),
            Route::Git(vec![
                p("/Users/me/code/app"),
                p("/Users/me/code/app-wt"),
                p("/Users/me/code/app/.claude/worktrees/nested"),
            ])
        );
    }

    #[test]
    fn a_linked_admin_dir_change_refreshes_only_that_worktree() {
        assert_eq!(
            route("/Users/me/code/app/.git/worktrees/app-wt/logs/HEAD"),
            Route::Git(vec![p("/Users/me/code/app-wt")])
        );
    }

    #[test]
    fn git_objects_and_gits_own_lock_files_are_ignored() {
        assert_eq!(
            route("/Users/me/code/app/.git/objects/ab/cdef"),
            Route::Ignore
        );
        for lock in [
            "/Users/me/code/app/.git/index.lock",
            "/Users/me/code/app/.git/refs/heads/main.lock",
            "/Users/me/code/app/.git/worktrees/app-wt/HEAD.lock",
            "/Users/me/code/app/.git/worktrees/new-wt/index.lock",
        ] {
            assert_eq!(route(lock), Route::Ignore, "{lock}");
        }
    }

    #[test]
    fn a_lock_file_in_a_repository_with_no_git_dir_named_dot_git_is_gits_own_too() {
        let common = "/Users/me/code/app.git";
        let index = Index::new(
            Path::new(HOME),
            &[project(
                "/Users/me/code/wt",
                Some((common, "/Users/me/code/app.git/worktrees/wt")),
                &[],
            )],
        );
        assert_eq!(
            index.route(Path::new("/Users/me/code/app.git/worktrees/wt/index.lock")),
            Route::Ignore
        );
    }

    #[test]
    fn a_dependency_lockfile_is_a_file_change_in_its_project() {
        for lockfile in ["Cargo.lock", "bun.lock", "web/yarn.lock", "flake.lock"] {
            assert_eq!(
                route(&format!("/Users/me/code/app/{lockfile}")),
                Route::FileChange(p("/Users/me/code/app")),
                "{lockfile}"
            );
        }
    }

    #[test]
    fn the_lock_files_of_a_build_route_to_its_build_folder_like_all_it_writes() {
        for lock in [
            "/Users/me/code/app/target/debug/.cargo-lock",
            "/Users/me/code/app/target/debug/incremental/app-1/s-abc.lock",
        ] {
            assert_eq!(
                route(lock),
                Route::Artifact(p("/Users/me/code/app/target")),
                "{lock}"
            );
        }
    }

    #[test]
    fn a_lock_file_in_a_state_folder_is_ignored_in_a_project_and_outside_every_project() {
        assert_eq!(
            route("/Users/me/code/site/.dev-runner/run.lock"),
            Route::Ignore
        );
        assert_eq!(
            route("/Users/me/code/fresh/.dev-runner/run.lock"),
            Route::Ignore
        );
    }

    #[test]
    fn an_unknown_or_vanishing_admin_dir_rediscovers() {
        assert_eq!(
            route("/Users/me/code/app/.git/worktrees/new-wt/HEAD"),
            Route::Rediscover
        );
        assert_eq!(
            route("/Users/me/code/app/.git/worktrees/app-wt"),
            Route::Rediscover
        );
    }

    #[test]
    fn a_change_inside_an_artifact_dir_routes_to_the_artifact() {
        assert_eq!(
            route("/Users/me/code/app/target/debug/deps/libx.rlib"),
            Route::Artifact(p("/Users/me/code/app/target"))
        );
    }

    #[test]
    fn a_file_change_belongs_to_the_deepest_project_holding_it() {
        assert_eq!(
            route("/Users/me/code/app/src/main.rs"),
            Route::FileChange(p("/Users/me/code/app"))
        );
        assert_eq!(
            route("/Users/me/code/app/.claude/worktrees/nested/src/lib.rs"),
            Route::FileChange(p("/Users/me/code/app/.claude/worktrees/nested"))
        );
        assert_eq!(
            route("/Users/me/code/site/index.html"),
            Route::FileChange(p("/Users/me/code/site"))
        );
    }

    #[test]
    fn a_linked_worktrees_git_file_is_ignored() {
        assert_eq!(route("/Users/me/code/app-wt/.git"), Route::Ignore);
    }

    #[test]
    fn a_new_git_entry_outside_every_project_rediscovers() {
        assert_eq!(route("/Users/me/code/fresh/.git"), Route::Rediscover);
        assert_eq!(
            route("/Users/me/code/fresh/README.md"),
            Route::OutsideProjects
        );
        assert_eq!(route("/Users/me/.zsh_history"), Route::OutsideProjects);
    }

    #[test]
    fn state_published_outside_every_project_rediscovers() {
        assert_eq!(route("/Users/me/code/fresh/.dev-runner"), Route::Rediscover);
        assert_eq!(
            route("/Users/me/code/fresh/.dev-runner/server.status"),
            Route::Rediscover
        );
    }

    #[test]
    fn artifact_owner_names_the_project_holding_it() {
        let index = index();
        assert_eq!(
            index.artifact_owner(Path::new("/Users/me/code/app/target")),
            Some(Path::new("/Users/me/code/app"))
        );
        assert_eq!(
            index.artifact_owner(Path::new("/Users/me/code/app/src")),
            None
        );
    }
}
