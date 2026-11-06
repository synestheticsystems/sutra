//! Fixtures shared by the crate's tests: a temporary folder that removes itself, git repositories
//! made with the real `git` CLI, so the files under test are exactly the ones git writes, and the
//! map's values with nothing on them, for a test to fill in only what its scenario turns on.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::clean::CargoEnv;
use super::scan::Boundary;
use super::{
    AgentSession, AgentStatus, ArtifactDir, ArtifactKind, DirUsage, FolderAccess, Harness,
    Measurement, Project, ScanOptions, ScanStats, Snapshot, Subagent,
};
use crate::model::{EnvSources, Environment};

/// Cargo's signature, as it writes it into a build folder it creates.
pub(crate) const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n";

/// A pid no process holds: above every pid limit macOS and Linux allow.
pub(crate) const DEAD_PID: u32 = i32::MAX as u32;

/// A running process for a fixture to name, killed and reaped when dropped, pass or fail, so its
/// pid reads as dead from then on.
pub(crate) struct LiveProcess(Child);

impl LiveProcess {
    pub(crate) fn start() -> LiveProcess {
        let sleeper = Command::new("sleep").arg("60").spawn();
        LiveProcess(sleeper.expect("start a process"))
    }

    pub(crate) fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for LiveProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A fresh folder under the system temp dir for one test, removed when dropped, pass or fail.
pub(crate) struct TempDir {
    /// Canonical, so expected paths compare equal to the canonical paths the map reports.
    root: PathBuf,
    /// Paths given restricted permissions, restored before removal.
    restricted: RefCell<Vec<PathBuf>>,
}

impl TempDir {
    /// `test` names the folder and must be unique across the crate's tests, which run in parallel.
    pub(crate) fn new(test: &str) -> TempDir {
        let root = std::env::temp_dir().join(format!("sutra-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create temp dir");
        TempDir {
            root: fs::canonicalize(&root).expect("canonicalize temp dir"),
            restricted: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub(crate) fn dir(&self, rel: &str) -> PathBuf {
        let dir = self.path(rel);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    pub(crate) fn write(&self, rel: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let file = self.path(rel);
        fs::create_dir_all(file.parent().expect("file has a parent")).expect("create parent");
        fs::write(&file, content).expect("write fixture file");
        file
    }

    /// A repository on branch `main` with one commit, so it has an index, a reflog and a ref.
    pub(crate) fn repo(&self, rel: &str) -> PathBuf {
        let dir = self.dir(rel);
        git(&dir, &["init", "-q", "-b", "main"]);
        commit(&dir, "README");
        dir
    }

    /// A Cargo package in the folder `rel`, with a manifest and nothing else.
    pub(crate) fn package(&self, rel: &str) {
        self.write(
            &format!("{rel}/Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
    }

    /// The environment of a clean with this folder's `home` for the home folder, so `CARGO_HOME`
    /// is inside the folder too, and none of cargo's variables set.
    pub(crate) fn cargo_env(&self) -> CargoEnv {
        cargo_env(self.path("home"))
    }

    /// Give `path` the permission bits `mode` until this folder is dropped: `0o000` makes it
    /// unreadable, as a folder outside the sandbox grant is; `0o555` makes a folder read-only, as
    /// a grant can be.
    pub(crate) fn restrict(&self, path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("restrict");
        self.restricted.borrow_mut().push(path.to_path_buf());
    }

    /// The boundary of a scan rooted at this folder as the home folder.
    pub(crate) fn boundary(&self, opts: &ScanOptions) -> Boundary {
        Boundary::new(std::slice::from_ref(&self.root), &self.root, opts)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        for path in self.restricted.borrow().iter() {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Run git in `dir` with no user or system config, so a global hook or signing setting cannot
/// change what the fixture looks like. Returns trimmed stdout.
pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_command(dir, args).output().expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf-8 git output")
        .trim()
        .to_owned()
}

/// git in `dir`, acting on the repository around `dir` and no other. A command git itself runs,
/// such as `git rebase --exec`, inherits `GIT_DIR`, which points git at the repository the tests
/// run in, whatever folder a fixture's git runs in. Run that way, a fixture's `git init` once marked
/// Sutra's own repository bare, and its commits landed on the branch being rebased (2026-11-06).
fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.current_dir(dir);
    for variable in repository_variables() {
        command.env_remove(variable);
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=Sutra Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args);
    command
}

/// The environment variables that point git at a repository, as this git lists them, so that a
/// git that knows more of them than Sutra does has all of them cleared.
fn repository_variables() -> &'static [String] {
    static VARIABLES: OnceLock<Vec<String>> = OnceLock::new();
    VARIABLES.get_or_init(|| {
        let output = Command::new("git")
            .args(["rev-parse", "--local-env-vars"])
            .output()
            .expect("run git");
        assert!(output.status.success(), "git lists its variables");
        String::from_utf8(output.stdout)
            .expect("utf-8 git output")
            .lines()
            .map(str::to_owned)
            .collect()
    })
}

/// Add a file named `name` and commit it.
pub(crate) fn commit(dir: &Path, name: &str) {
    fs::write(dir.join(name), name).expect("write committed file");
    git(dir, &["add", name]);
    git(dir, &["commit", "-q", "-m", name]);
}

pub(crate) fn set_mtime(file: &Path, time: SystemTime) {
    fs::File::options()
        .write(true)
        .open(file)
        .and_then(|file| file.set_modified(time))
        .expect("set mtime");
}

/// The environment of a clean with `home` for the home folder and none of cargo's variables set.
pub(crate) fn cargo_env(home: impl Into<PathBuf>) -> CargoEnv {
    CargoEnv {
        home: home.into(),
        cargo_home: None,
        target_dir: None,
        build_target_dir: None,
        build_build_dir: None,
    }
}

/// The moment `secs` seconds after the epoch.
pub(crate) fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

/// A tree holding `bytes`, every folder of it read, measured at the epoch.
pub(crate) fn usage(bytes: u64) -> DirUsage {
    DirUsage {
        bytes,
        files: 1,
        unreadable_dirs: 0,
        measured_at: UNIX_EPOCH,
        elapsed: Duration::ZERO,
    }
}

/// A finished measurement of `usage(bytes)`.
pub(crate) fn measured(bytes: u64) -> Measurement {
    Measurement::Measured(usage(bytes))
}

/// A Cargo build folder at `path`.
pub(crate) fn cargo_target(path: impl Into<PathBuf>, measurement: Measurement) -> ArtifactDir {
    ArtifactDir {
        path: path.into(),
        kind: ArtifactKind::CargoTarget,
        measurement,
    }
}

/// A `node_modules` folder at `path`.
pub(crate) fn node_modules(path: impl Into<PathBuf>, measurement: Measurement) -> ArtifactDir {
    ArtifactDir {
        path: path.into(),
        kind: ArtifactKind::NodeModules,
        measurement,
    }
}

/// A readable project at `path`, named after its folder, with no repository, activity, agent,
/// environment or build folder, and nothing measured.
pub(crate) fn project_at(path: impl Into<PathBuf>) -> Project {
    let path = path.into();
    Project {
        name: path
            .file_name()
            .expect("a project folder has a name")
            .to_string_lossy()
            .into_owned(),
        path,
        repo: None,
        last_activity: None,
        activity_source: None,
        agents: Vec::new(),
        subagents: Vec::new(),
        environment: None,
        artifacts: Vec::new(),
        own_files: Measurement::Pending,
        access: FolderAccess::Readable,
    }
}

/// A Claude Code session `id` in `status`, working in `/code/app`, its harness recording nothing
/// more and no subagent of it at work.
pub(crate) fn agent_session(id: &str, status: AgentStatus) -> AgentSession {
    AgentSession {
        harness: Harness::ClaudeCode,
        session_id: id.to_string(),
        pid: None,
        cwd: PathBuf::from("/code/app"),
        status,
        waiting_for: None,
        kind: None,
        label: None,
        since: None,
        last_activity: None,
        host: None,
        subagents_at_work: Vec::new(),
    }
}

/// A busy subagent `id` working in `/code/app`, its harness recording nothing more of it.
pub(crate) fn subagent_at_work(id: &str) -> Subagent {
    Subagent {
        id: id.to_string(),
        cwd: PathBuf::from("/code/app"),
        name: None,
        description: None,
        status: AgentStatus::Busy,
        since: None,
        last_write: None,
    }
}

/// The environment of the folder `dir`, with no process, no units and no state folder behind it.
pub(crate) fn environment_at(dir: impl Into<PathBuf>) -> Environment {
    Environment {
        id: "env".to_string(),
        dir: dir.into(),
        pid: 0,
        ports: HashMap::new(),
        started: 0,
        alive: false,
        units: Vec::new(),
        sources: EnvSources::default(),
    }
}

/// A map of `projects`, in the order given, under the one root `/code`, generated at the epoch,
/// with nothing else on it.
pub(crate) fn snapshot_of(projects: Vec<Project>) -> Snapshot {
    Snapshot {
        roots: vec![PathBuf::from("/code")],
        skipped_private_folders: Vec::new(),
        sandboxed: false,
        projects,
        unplaced_agents: Vec::new(),
        unplaced_subagents: Vec::new(),
        problems: Vec::new(),
        scan: ScanStats::default(),
        generated_at: UNIX_EPOCH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixtures_git_clears_every_variable_that_would_point_it_at_another_repository() {
        let command = git_command(Path::new("/"), &["status"]);
        let cleared: Vec<String> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            assert!(cleared.iter().any(|name| name == variable), "{variable}");
        }
    }
}
