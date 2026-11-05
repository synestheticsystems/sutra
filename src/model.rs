use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nix::errno::Errno;
use nix::sys::signal;
use nix::unistd::Pid;

/// State of a unit
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    None, // empty/missing status
    Starting,
    Building,
    Running,
    Ready,
    Failed,
    Stopped,
    Other(String), // unrecognized state, stored as-is
}

impl State {
    pub fn parse(s: &str) -> State {
        match s {
            "starting" => State::Starting,
            "building" => State::Building,
            "running" => State::Running,
            "ready" => State::Ready,
            "failed" => State::Failed,
            "stopped" => State::Stopped,
            _ => State::Other(s.to_string()),
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            State::None => "",
            State::Starting => "starting",
            State::Building => "building",
            State::Running => "running",
            State::Ready => "ready",
            State::Failed => "failed",
            State::Stopped => "stopped",
            State::Other(s) => s.as_str(),
        };
        write!(f, "{}", label)
    }
}

/// Status of a single named unit (e.g., "server", "vite")
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitStatus {
    pub name: String,
    pub state: State,
    pub detail: Option<String>,
}

impl UnitStatus {
    /// Parse a status value like "building: Compiling Rust bindings".
    /// The `name` comes from the status filename, not the content.
    /// Uses State::None for empty content.
    ///
    /// The detail separator is `:` (any surrounding whitespace is
    /// trimmed). Both `building: cargo` and `building:cargo` parse the
    /// same way — accepting both is friendlier to dev-script authors
    /// who forget the space and would otherwise see their state fall
    /// through to `State::Other`.
    pub fn parse(name: &str, content: &str) -> UnitStatus {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return UnitStatus {
                name: name.to_string(),
                state: State::None,
                detail: None,
            };
        }

        let (state_str, detail) = match trimmed.split_once(':') {
            Some((s, d)) => {
                let detail = d.trim();
                let detail = if detail.is_empty() {
                    None
                } else {
                    Some(detail.to_string())
                };
                (s.trim_end(), detail)
            }
            None => (trimmed, None),
        };

        let state = State::parse(state_str);
        UnitStatus {
            name: name.to_string(),
            state,
            detail,
        }
    }
}

/// A registered environment instance
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub id: String,
    pub dir: PathBuf,
    /// The supervisor's process id. 0 when no meta file publishes one: a project's own
    /// `.dev-runner/` folder may hold status files only.
    pub pid: u32,
    pub ports: HashMap<String, u16>, // lowercase unit name → port
    pub started: u64,
    pub alive: bool,
    pub units: Vec<UnitStatus>,
    pub sources: EnvSources,
}

/// Which state-folder conventions describe an environment. The worktree map merges both into one
/// environment per project folder and keeps this so it can show where the state came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnvSources {
    /// A hash-named meta file in `~/.dev-runner/`.
    pub global: bool,
    /// The project's own `.dev-runner/` folder.
    pub project: bool,
}

impl Environment {
    /// Load an environment instance from its meta file. What cannot be read goes in `failures`: a
    /// meta loses its environment, and a status file only its unit.
    ///
    /// Meta file: `~/.dev-runner/<hash>` (KEY=VALUE lines)
    /// Status files: `~/.dev-runner/<hash>.<unit_name>.status` (single line: `<state>[: <detail>]`)
    pub(crate) fn load(meta_path: &Path, failures: &mut Vec<ReadFailure>) -> Option<Environment> {
        let content = read_state_file(meta_path, failures)?;
        let id = meta_path.file_name()?.to_str()?.to_string();
        let meta = Meta::parse(&content);
        let pid = meta.pid?;
        let dir = meta.dir?;

        // Scan for status files in both conventions:
        //   new: <hash>.<unit_name>.status
        //   old: .<hash>.<unit_name>.status
        let new_prefix = format!("{id}.");
        let old_prefix = format!(".{id}.");
        let registry = meta_path.parent()?;
        let units = read_units(
            registry,
            |file_name| {
                file_name
                    .strip_prefix(&new_prefix)
                    .or_else(|| file_name.strip_prefix(&old_prefix))?
                    .strip_suffix(".status")
                    .filter(|unit_name| !unit_name.is_empty())
            },
            failures,
        )
        // The environment still shows, without units.
        .unwrap_or_else(|err| {
            note_failure(registry, &err, failures);
            Vec::new()
        });

        Some(Environment {
            id,
            dir,
            pid,
            ports: meta.ports,
            started: meta.started.unwrap_or(0),
            alive: process_exists(pid),
            units,
            sources: EnvSources {
                global: true,
                project: false,
            },
        })
    }

    /// Whether the environment names a process and that process is no longer running, so the
    /// states its units last published may be stale. One that names no process (pid 0) has not
    /// stopped: nothing says it ran.
    pub fn has_stopped(&self) -> bool {
        self.pid != 0 && !self.alive
    }

    /// Look up the port associated with a unit by name.
    /// Matches against `*_PORT` keys from the meta file (e.g. `SERVER_PORT` → "server").
    pub fn port_for(&self, unit_name: &str) -> Option<u16> {
        self.ports.get(unit_name).copied()
    }

    /// Short display name derived from the project directory.
    pub fn display_name(&self) -> &str {
        self.dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
    }

    /// Elapsed time since started, as a human-readable string.
    pub fn elapsed_string(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let elapsed = now.saturating_sub(self.started);
        if elapsed < 60 {
            format!("{}s", elapsed)
        } else if elapsed < 3600 {
            format!("{}m", elapsed / 60)
        } else if elapsed < 86400 {
            format!("{}h {}m", elapsed / 3600, (elapsed % 3600) / 60)
        } else {
            format!("{}d", elapsed / 86400)
        }
    }
}

/// The values a meta file declares. Each convention decides which of them it requires.
#[derive(Debug, Default)]
pub(crate) struct Meta {
    pub(crate) dir: Option<PathBuf>,
    pub(crate) pid: Option<u32>,
    pub(crate) started: Option<u64>,
    pub(crate) ports: HashMap<String, u16>,
}

impl Meta {
    /// Parse `KEY=VALUE` lines. Unknown keys and lines without `=` are skipped; a value that does
    /// not parse leaves its field unset.
    pub(crate) fn parse(content: &str) -> Meta {
        let mut meta = Meta::default();
        for line in content.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "DIR" => meta.dir = Some(PathBuf::from(value)),
                "PID" => meta.pid = value.parse().ok(),
                "STARTED" => meta.started = value.parse().ok(),
                other => {
                    if let (Some(unit), Ok(port)) = (other.strip_suffix("_PORT"), value.parse()) {
                        meta.ports.insert(unit.to_lowercase(), port);
                    }
                }
            }
        }
        meta
    }
}

/// Whether a process with this id exists. Signal 0 probes without delivering anything. EPERM
/// means the process exists but we may not signal it (another user's process, or any outside
/// process under the App Sandbox); only ESRCH means there is no such process. Pid 0 is no
/// process: `kill(0, ..)` would address our own process group.
pub(crate) fn process_exists(pid: u32) -> bool {
    match i32::try_from(pid) {
        Ok(raw_pid) if raw_pid > 0 => !matches!(
            signal::kill(Pid::from_raw(raw_pid), None),
            Err(Errno::ESRCH)
        ),
        _ => false,
    }
}

/// A file or folder of a state folder that could not be read, and why.
pub(crate) type ReadFailure = (PathBuf, io::ErrorKind);

/// Add a failed read of `path` to `failures`, unless it failed because `path` is gone: writers
/// replace state files by rename and delete them on exit, so one can vanish between being listed
/// and being read.
pub(crate) fn note_failure(path: &Path, err: &io::Error, failures: &mut Vec<ReadFailure>) {
    if err.kind() != io::ErrorKind::NotFound {
        failures.push((path.to_path_buf(), err.kind()));
    }
}

/// What the state file at `path` holds, or `None` when it cannot be read (see `note_failure`).
pub(crate) fn read_state_file(path: &Path, failures: &mut Vec<ReadFailure>) -> Option<String> {
    fs::read_to_string(path)
        .map_err(|err| note_failure(path, &err, failures))
        .ok()
}

/// Read the status files in `dir` that `unit_name_of` maps to a unit name, sorted by unit name. A
/// status file or folder entry that cannot be read goes in `failures`, and the rest still read.
/// Fails when `dir` cannot be listed, a missing `dir` included.
pub(crate) fn read_units(
    dir: &Path,
    unit_name_of: impl Fn(&str) -> Option<&str>,
    failures: &mut Vec<ReadFailure>,
) -> io::Result<Vec<UnitStatus>> {
    let mut units = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                note_failure(dir, &err, failures);
                continue;
            }
        };
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let Some(unit_name) = unit_name_of(&file_name) else {
            continue;
        };
        if let Some(content) = read_state_file(&entry.path(), failures) {
            units.push(UnitStatus::parse(unit_name, &content));
        }
    }
    units.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(units)
}

/// The dev-runner registry inside a home folder.
pub(crate) fn registry_dir(home: &Path) -> PathBuf {
    home.join(".dev-runner")
}

/// Returns the path to the dev-runner registry directory (~/.dev-runner/),
/// or `None` if the home directory cannot be determined. It starts from the
/// real home folder because inside the App Sandbox `$HOME` is the app's container.
pub fn state_dir() -> Option<PathBuf> {
    crate::home::real_home().map(|home| registry_dir(&home))
}

/// Returns true if a filename looks like a hex-only meta file (no dots, not hidden).
fn is_meta_file(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains('.')
        && name.chars().all(|c| c.is_ascii_hexdigit())
}

/// Load all environment instances from the registry. The terminal UI has nowhere to say that the
/// registry or part of it could not be read, so what cannot be read just does not show there.
pub fn load_all() -> Vec<Environment> {
    state_dir()
        .and_then(|dir| load_registry(&dir, &mut Vec::new()).ok())
        .unwrap_or_default()
}

/// Load every environment registered in `dir`, sorted by project directory. What cannot be read of
/// one environment goes in `failures`, and the others still load. Fails when `dir` cannot be
/// listed, a missing registry included.
pub(crate) fn load_registry(
    dir: &Path,
    failures: &mut Vec<ReadFailure>,
) -> io::Result<Vec<Environment>> {
    let entries = fs::read_dir(dir)?;

    let mut envs = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                note_failure(dir, &err, failures);
                continue;
            }
        };
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        if is_meta_file(&fname_str) {
            if let Some(env) = Environment::load(&entry.path(), failures) {
                envs.push(env);
            }
        }
    }

    envs.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(envs)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::map::test_support::{DEAD_PID, TempDir, environment_at};

    /// Every `<unit>.status` in `dir`, with what could not be read.
    fn units_in(dir: &Path) -> (Vec<(String, State)>, Vec<ReadFailure>) {
        let mut failures = Vec::new();
        let units = read_units(
            dir,
            |file_name| file_name.strip_suffix(".status"),
            &mut failures,
        )
        .unwrap();
        let units = units.into_iter().map(|u| (u.name, u.state)).collect();
        (units, failures)
    }

    #[test]
    fn a_status_file_that_cannot_be_read_is_a_failure_and_its_siblings_still_read() {
        let fixture = TempDir::new("model-units-sealed");
        fixture.write("state/a.status", "ready");
        let sealed = fixture.write("state/b.status", "ready");
        fixture.restrict(&sealed, 0o000);
        fixture.write("state/c.status", "failed");

        let (units, failures) = units_in(&fixture.path("state"));

        assert_eq!(
            units,
            [
                ("a".to_string(), State::Ready),
                ("c".to_string(), State::Failed)
            ]
        );
        assert_eq!(failures, [(sealed, io::ErrorKind::PermissionDenied)]);
    }

    #[test]
    fn a_status_file_that_is_not_text_is_a_failure() {
        let fixture = TempDir::new("model-units-binary");
        let binary = fixture.write("state/a.status", [0xff, 0xfe]);

        let (units, failures) = units_in(&fixture.path("state"));

        assert!(units.is_empty());
        assert_eq!(failures, [(binary, io::ErrorKind::InvalidData)]);
    }

    #[test]
    fn a_status_file_gone_by_the_time_it_is_read_is_no_failure() {
        let fixture = TempDir::new("model-units-vanished");
        let state = fixture.dir("state");
        // Listed, and not there when opened, as a file deleted between the two is.
        symlink(fixture.path("gone"), state.join("a.status")).unwrap();
        fixture.write("state/b.status", "ready");

        let (units, failures) = units_in(&state);

        assert_eq!(units, [("b".to_string(), State::Ready)]);
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn a_meta_that_cannot_be_read_is_a_failure_and_the_other_environments_still_load() {
        let fixture = TempDir::new("model-registry-sealed");
        let registry = fixture.dir("registry");
        let sealed = fixture.write("registry/ab12", "DIR=/code/sealed\nPID=1\n");
        fixture.restrict(&sealed, 0o000);
        fixture.write("registry/cd34", "DIR=/code/open\nPID=1\n");
        fixture.write("registry/cd34.server.status", "ready");
        let mut failures = Vec::new();

        let envs = load_registry(&registry, &mut failures).unwrap();

        let dirs: Vec<&Path> = envs.iter().map(|env| env.dir.as_path()).collect();
        assert_eq!(dirs, [Path::new("/code/open")]);
        assert_eq!(envs[0].units.len(), 1);
        assert_eq!(failures, [(sealed, io::ErrorKind::PermissionDenied)]);
    }

    #[test]
    fn a_status_file_that_cannot_be_read_costs_an_environment_only_that_unit() {
        let fixture = TempDir::new("model-registry-status");
        let registry = fixture.dir("registry");
        fixture.write("registry/ab12", "DIR=/code/app\nPID=1\n");
        fixture.write("registry/ab12.server.status", "ready");
        let sealed = fixture.write("registry/ab12.vite.status", "ready");
        fixture.restrict(&sealed, 0o000);
        let mut failures = Vec::new();

        let envs = load_registry(&registry, &mut failures).unwrap();

        assert_eq!(envs.len(), 1);
        let units: Vec<&str> = envs[0].units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(units, ["server"]);
        assert_eq!(failures, [(sealed, io::ErrorKind::PermissionDenied)]);
    }

    #[test]
    fn an_environment_has_stopped_only_when_the_process_it_names_is_gone() {
        let env = |pid, alive| Environment {
            pid,
            alive,
            ..environment_at("/code/app")
        };
        assert!(env(4242, false).has_stopped());
        assert!(!env(4242, true).has_stopped());
        assert!(
            !env(0, false).has_stopped(),
            "no process named, none stopped"
        );
    }

    #[test]
    fn parse_state_canonical() {
        assert_eq!(State::parse("starting"), State::Starting);
        assert_eq!(State::parse("building"), State::Building);
        assert_eq!(State::parse("running"), State::Running);
        assert_eq!(State::parse("ready"), State::Ready);
        assert_eq!(State::parse("failed"), State::Failed);
        assert_eq!(State::parse("stopped"), State::Stopped);
    }

    #[test]
    fn parse_state_unknown_falls_to_other() {
        assert_eq!(State::parse("done"), State::Other("done".into()));
        // Case-sensitive: capitalized variants are not canonical.
        assert_eq!(State::parse("Ready"), State::Other("Ready".into()));
    }

    #[test]
    fn unit_parse_state_only() {
        let u = UnitStatus::parse("server", "ready");
        assert_eq!(u.state, State::Ready);
        assert_eq!(u.detail, None);
    }

    #[test]
    fn unit_parse_state_with_detail_canonical_separator() {
        let u = UnitStatus::parse("server", "building: cargo");
        assert_eq!(u.state, State::Building);
        assert_eq!(u.detail.as_deref(), Some("cargo"));
    }

    #[test]
    fn unit_parse_state_with_detail_no_space_after_colon() {
        // Lenient: "building:cargo" should parse same as "building: cargo".
        let u = UnitStatus::parse("server", "building:cargo");
        assert_eq!(u.state, State::Building);
        assert_eq!(u.detail.as_deref(), Some("cargo"));
    }

    #[test]
    fn unit_parse_state_trailing_colon_no_detail() {
        // "failed:" with no detail should give state=Failed, detail=None.
        let u = UnitStatus::parse("server", "failed:");
        assert_eq!(u.state, State::Failed);
        assert_eq!(u.detail, None);
    }

    #[test]
    fn unit_parse_extra_whitespace() {
        let u = UnitStatus::parse("server", "  building :  cargo  ");
        assert_eq!(u.state, State::Building);
        assert_eq!(u.detail.as_deref(), Some("cargo"));
    }

    #[test]
    fn unit_parse_empty_is_none() {
        let u = UnitStatus::parse("server", "");
        assert_eq!(u.state, State::None);
        assert_eq!(u.detail, None);
    }

    #[test]
    fn unit_parse_whitespace_only_is_none() {
        let u = UnitStatus::parse("server", "   \n  ");
        assert_eq!(u.state, State::None);
    }

    #[test]
    fn meta_filename_validation() {
        assert!(is_meta_file("a"));
        assert!(is_meta_file("df79fed95eebc05d"));
        assert!(is_meta_file("0123456789abcdef"));
        assert!(!is_meta_file(""));
        assert!(!is_meta_file(".hidden"));
        assert!(!is_meta_file("has.dot"));
        assert!(!is_meta_file("nothex_g"));
        // Note: STATE_SPEC.md says "lowercase hex" but the code uses
        // `is_ascii_hexdigit` which accepts both cases. Documented as
        // permissive-but-conventional rather than enforced.
        assert!(is_meta_file("ABCDEF"));
    }

    #[test]
    fn meta_parse_reads_known_keys_and_lowercases_port_units() {
        let meta = Meta::parse(
            "DIR=/code/app\nPID=42\nSTARTED=1700000000\nSERVER_PORT=3000\nBAD_PORT=x\nOTHER=1\nno equals sign",
        );
        assert_eq!(meta.dir, Some(PathBuf::from("/code/app")));
        assert_eq!(meta.pid, Some(42));
        assert_eq!(meta.started, Some(1_700_000_000));
        assert_eq!(meta.ports, HashMap::from([("server".to_string(), 3000)]));
    }

    #[test]
    fn process_exists_for_our_own_pid() {
        assert!(process_exists(std::process::id()));
    }

    #[test]
    fn process_exists_is_false_for_a_pid_no_process_holds() {
        assert!(!process_exists(DEAD_PID));
    }

    #[test]
    fn process_exists_for_a_process_we_may_not_signal() {
        // launchd/init belongs to root, so kill reports EPERM unless the tests run as root.
        assert!(process_exists(1));
    }

    #[test]
    fn process_exists_is_false_for_pid_zero() {
        assert!(!process_exists(0));
    }
}
