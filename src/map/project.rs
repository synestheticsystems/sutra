//! One project on the map: the folder, the repository it belongs to, the agents and dev
//! environment working in it, and how much disk it holds, its build folders apart.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::docker::Container;
use super::processes::ListeningProcess;
use super::{AgentSession, Measurement, PrivateFolder, Provenance, SubagentWithParent};
use crate::model::Environment;

/// How much disk a folder must hold to matter for its size alone: a project this large stays in
/// focus however quiet it is, and a scratch folder this large has a card with no build folder in
/// it. A first guess.
pub const LARGE_FOLDER_BYTES: u64 = 1 << 30;

/// One row of the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Canonical path (symlinks resolved), the key every source is joined on.
    pub path: PathBuf,
    /// The folder's own name. A scratch folder's session goes by its own (see `ScratchSpace::name`).
    pub name: String,
    /// `None` for a plain project folder that only a dev-runner environment names, and for a
    /// scratch folder.
    pub repo: Option<RepoInfo>,
    /// `Some` for a scratch folder: a folder in the space where a Claude Code session keeps what it
    /// throws away.
    pub scratch: Option<ScratchSpace>,
    /// Where it came from: the group its card stands in, and what it was made from.
    pub provenance: Provenance,
    pub last_activity: Option<SystemTime>,
    pub activity_source: Option<ActivitySource>,
    pub agents: Vec<AgentSession>,
    /// The subagents at work here, whichever session they work for, by session: those of its own
    /// sessions, which count them too, and those of sessions that work elsewhere.
    pub subagents: Vec<SubagentWithParent>,
    /// The merged dev-runner environment, if any.
    pub environment: Option<Environment>,
    /// The user's processes listening for TCP connections that run in the folder, and in no project
    /// nested in it, in the order macOS lists them.
    pub listening: Vec<ListeningProcess>,
    /// The Docker containers running for the folder, by name: those compose made there, else those
    /// bind-mounting a folder of it, and in no project nested in it.
    pub containers: Vec<Container>,
    pub artifacts: Vec<ArtifactDir>,
    /// Everything in the folder but its build folders and the projects nested inside it, which
    /// are measured on their own. Never measured while the folder cannot be read.
    pub own_files: Measurement,
    pub access: FolderAccess,
}

impl Project {
    /// Whether something is at work in the folder: an agent's session, or a subagent whichever
    /// session it works for, a dev-runner environment, running or not, a process listening for
    /// connections, or a Docker container running for it.
    pub fn is_at_work(&self) -> bool {
        !self.agents.is_empty()
            || !self.subagents.is_empty()
            || self.environment.is_some()
            || !self.listening.is_empty()
            || !self.containers.is_empty()
    }

    /// The bytes the project's build folders hold, counting the measured ones only.
    pub fn measured_build_bytes(&self) -> u64 {
        self.artifacts
            .iter()
            .filter_map(|dir| dir.measurement.usage())
            .map(|usage| usage.bytes)
            .sum()
    }

    /// The size of the whole folder: its own files and its build folders, once each of them has
    /// been measured. A build folder whose measurement failed adds nothing and makes the size a
    /// lower bound.
    pub fn folder_size(&self) -> Option<FolderSize> {
        let own_files = self.own_files.usage()?;
        let mut size = FolderSize {
            bytes: own_files.bytes,
            partial: own_files.unreadable_dirs > 0,
        };
        for dir in &self.artifacts {
            match &dir.measurement {
                Measurement::Measured(usage) => {
                    size.bytes += usage.bytes;
                    size.partial |= usage.unreadable_dirs > 0;
                }
                Measurement::Failed(_) => size.partial = true,
                Measurement::Pending | Measurement::Measuring => return None,
            }
        }
        Some(size)
    }

    /// Whether the folder is a Cargo build folder and nothing else, as a scratch folder that
    /// `CARGO_TARGET_DIR` named is: cargo tagged the folder itself, so its one build folder is the
    /// folder whole (see `scan::find_artifacts_throughout`), and it holds no source of its own.
    pub fn is_its_own_build_folder(&self) -> bool {
        self.artifacts.iter().any(|dir| dir.path == self.path)
    }

    /// Whether `last_activity` tells all there is to know of the work in the folder, so that none
    /// means none. A scratch folder is dated by the newest file in it once its own files are
    /// walked (see `State::date_scratch_folder`): until that walk lands, or when it failed, its
    /// work is unknown, and may be minutes old.
    pub fn activity_is_known(&self) -> bool {
        self.scratch.is_none() || self.own_files.usage().is_some()
    }

    /// The engine's order, and every sort's last tiebreak: newest activity first, unknown
    /// activity last, then by name and path.
    pub fn newest_first(a: &Project, b: &Project) -> Ordering {
        b.last_activity
            .cmp(&a.last_activity)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    }

    /// Every measurement the folder's size is made of, none while the folder cannot be read.
    pub fn measurements(&self) -> impl Iterator<Item = &Measurement> {
        let readable = self.access == FolderAccess::Readable;
        std::iter::once(&self.own_files)
            .chain(self.artifacts.iter().map(|dir| &dir.measurement))
            .filter(move |_| readable)
    }
}

/// Whether Sutra reads a project's folder, and why not. A folder it does not read still has a
/// row, because git or a dev-runner environment names it, and what they record about it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderAccess {
    Readable,
    /// In a private folder Sutra stays out of, and what keeps it out. Reading anything inside one
    /// would raise macOS's privacy prompt, or wait for the user to answer it.
    KeptOut(PrivateFolder, KeptOutBy),
    /// The folder no longer exists, or lies on a volume that is not mounted (a locked worktree).
    Missing,
    /// The folder exists but cannot be listed. Inside the App Sandbox every folder outside the
    /// grant is like this: it can be seen to exist, not opened.
    Denied,
}

/// What keeps Sutra out of a private folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptOutBy {
    /// The folder's switch in Settings is off.
    Settings,
    /// macOS's privacy prompt, which the user has not answered yet. Until macOS says whether Sutra
    /// may read the folder, any read inside it waits for the answer, so Sutra stays out.
    Prompt,
    /// macOS, because the user said no to its prompt, now or on an earlier launch.
    MacOs,
}

/// A project folder's size, its build folders included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderSize {
    pub bytes: u64,
    /// Part of the folder could not be read, so `bytes` is a lower bound.
    pub partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySource {
    Git,
    Agent,
    Environment,
    FileChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// The main repository's `.git` directory, shared by all of its worktrees.
    pub common_dir: PathBuf,
    /// This worktree's own git dir: `common_dir` for the main worktree,
    /// `common_dir/worktrees/<name>` for a linked one.
    pub git_dir: PathBuf,
    /// `None` for the main worktree.
    pub worktree_name: Option<String>,
    /// `None` when HEAD is detached or unborn.
    pub branch: Option<String>,
    /// Short commit id of HEAD, when it resolves.
    pub head: Option<String>,
}

impl RepoInfo {
    /// The folder the repository goes by: for the usual repository, whose common dir is its main
    /// worktree's `.git`, the main worktree's folder; for a bare one, the common dir itself.
    pub fn folder(&self) -> &Path {
        match self.common_dir.file_name() {
            Some(dir) if dir == ".git" => self.common_dir.parent().unwrap_or(&self.common_dir),
            _ => &self.common_dir,
        }
    }
}

/// The scratch space of a Claude Code session, its folder `<scratch root>/<project key>/<session
/// id>`, where the session clones, builds and keeps whatever it throws away, outside every project
/// (see `scan::scratch`). Each scratch folder in it is a project of its own, and the space is what
/// they share: the session that made them, whose end ends their use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchSpace {
    /// The session's folder in the scratch root, which holds every scratch folder of the session.
    pub session_folder: PathBuf,
    pub session_id: String,
    /// The last part of the folder the session started in, as Claude Code's name for that folder
    /// still shows it on the disk, as `greyorange-foundry` reads.
    pub started_in: String,
    /// The folder the session started in, while the disk still shows all of it: the checkout a
    /// scratch folder that nothing else places is grouped with (see `Provenance`).
    pub started_in_folder: Option<PathBuf>,
    /// The session while it runs. It counts on the card of the folder it works in alone, and while
    /// it runs, it may build in any folder of its scratch.
    pub session: Option<AgentSession>,
    /// The name the session last went by: while it runs, its own, as its robot gives it; once it
    /// has ended, the one it went by while the map watched it run, or else the one its transcript
    /// last recorded. `None` for a session never named.
    pub session_name: Option<String>,
}

impl ScratchSpace {
    /// What the space goes by: the name its session last went by, so that the space keeps it once
    /// the session ends; else the folder the session started in, and the first eight characters of
    /// its id, as `greyorange-foundry e26ca359` reads.
    pub fn name(&self) -> String {
        if let Some(name) = &self.session_name {
            return name.clone();
        }
        let short_id: String = self.session_id.chars().take(8).collect();
        format!("{} {short_id}", self.started_in)
    }

    /// Where `folder`, one of the space's scratch folders, lies in the session's folder, as
    /// `scratchpad/r567fixture` reads.
    pub fn place_of<'a>(&self, folder: &'a Path) -> &'a Path {
        folder.strip_prefix(&self.session_folder).unwrap_or(folder)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactDir {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub measurement: Measurement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactKind {
    /// A folder Cargo builds in: a `target` beside a `Cargo.toml`, or any folder holding Cargo's
    /// `CACHEDIR.TAG`, whatever `CARGO_TARGET_DIR` named it (see `scan::is_cargo_target`). What
    /// cleaning it deletes is for `clean` to work out.
    CargoTarget,
    NodeModules,
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::map::DirUsage;
    use crate::map::test_support::{at, cargo_target, measured, project_at, usage};

    fn partly_measured(bytes: u64, unreadable_dirs: u64) -> Measurement {
        Measurement::Measured(DirUsage {
            unreadable_dirs,
            ..usage(bytes)
        })
    }

    /// A project whose own files and only build folder are both as given.
    fn sized(own_files: Measurement, target: Measurement) -> Project {
        Project {
            artifacts: vec![cargo_target("/code/app/target", target)],
            own_files,
            ..project_at("/code/app")
        }
    }

    #[test]
    fn the_folder_size_adds_the_build_folders_to_the_folders_own_files() {
        assert_eq!(
            sized(measured(300), measured(700)).folder_size(),
            Some(FolderSize {
                bytes: 1_000,
                partial: false
            })
        );
    }

    #[test]
    fn the_folder_size_waits_for_its_own_files_and_every_build_folder() {
        assert_eq!(
            sized(Measurement::Measuring, measured(700)).folder_size(),
            None
        );
        assert_eq!(
            sized(measured(300), Measurement::Pending).folder_size(),
            None
        );
        let failed = Measurement::Failed(io::ErrorKind::PermissionDenied);
        assert_eq!(sized(failed, measured(700)).folder_size(), None);
    }

    #[test]
    fn the_folder_size_is_a_lower_bound_when_any_part_could_not_be_read() {
        let partial = |own_files, target| sized(own_files, target).folder_size().unwrap();
        assert!(partial(partly_measured(300, 1), measured(700)).partial);
        assert!(partial(measured(300), partly_measured(700, 2)).partial);
        let failed_target = partial(
            measured(300),
            Measurement::Failed(io::ErrorKind::PermissionDenied),
        );
        assert_eq!(failed_target.bytes, 300);
        assert!(failed_target.partial);
    }

    #[test]
    fn newest_first_puts_unknown_activity_last_then_orders_by_name() {
        let mut projects: Vec<Project> =
            [("b", None), ("a", None), ("old", Some(1)), ("new", Some(2))]
                .into_iter()
                .map(|(name, activity)| Project {
                    last_activity: activity.map(at),
                    ..project_at(PathBuf::from("/code").join(name))
                })
                .collect();
        projects.sort_by(Project::newest_first);
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["new", "old", "a", "b"]);
    }

    #[test]
    fn a_repository_goes_by_its_main_worktrees_folder_and_a_bare_one_by_its_own() {
        let repo = |common_dir: &str| RepoInfo {
            common_dir: PathBuf::from(common_dir),
            git_dir: PathBuf::from(common_dir),
            worktree_name: None,
            branch: None,
            head: None,
        };
        assert_eq!(repo("/code/app/.git").folder(), Path::new("/code/app"));
        assert_eq!(repo("/code/app.git").folder(), Path::new("/code/app.git"));
    }

    #[test]
    fn a_scratch_space_goes_by_its_sessions_last_name_else_where_it_started_and_its_id() {
        let session = "/private/tmp/claude-501/-Users-me-vt-foundry/e26ca359-da00";
        let mut space = ScratchSpace {
            session_folder: PathBuf::from(session),
            session_id: "e26ca359-da00".to_string(),
            started_in: "foundry".to_string(),
            started_in_folder: None,
            session: None,
            session_name: None,
        };
        assert_eq!(space.name(), "foundry e26ca359");
        let folder = PathBuf::from(session).join("scratchpad/r567fixture");
        assert_eq!(space.place_of(&folder), Path::new("scratchpad/r567fixture"));

        space.session_name = Some("Foundry Agent Studio architecture".to_string());
        assert_eq!(space.name(), "Foundry Agent Studio architecture");
    }

    #[test]
    fn a_scratch_folders_activity_is_known_once_its_own_files_are_walked_and_any_others_always() {
        let session = "/private/tmp/claude-501/-Users-me/e26ca359";
        let scratch = |own_files| Project {
            scratch: Some(ScratchSpace {
                session_folder: PathBuf::from(session),
                session_id: "e26ca359".to_string(),
                started_in: "me".to_string(),
                started_in_folder: None,
                session: None,
                session_name: None,
            }),
            own_files,
            ..project_at(format!("{session}/scratchpad/r567"))
        };
        assert!(project_at("/code/app").activity_is_known(), "git dates it");
        assert!(scratch(measured(0)).activity_is_known());
        for unknown in [
            Measurement::Pending,
            Measurement::Measuring,
            Measurement::Failed(io::ErrorKind::PermissionDenied),
        ] {
            assert!(!scratch(unknown).activity_is_known(), "{unknown:?}");
        }
    }

    #[test]
    fn a_folder_cargo_tagged_whole_is_its_own_build_folder_and_one_holding_a_target_is_not() {
        let cache = "/private/tmp/claude-501/-Users-me/e26ca359/scratchpad/target-567-fix";
        let alone = Project {
            artifacts: vec![cargo_target(cache, measured(700))],
            ..project_at(cache)
        };
        assert!(alone.is_its_own_build_folder());
        assert!(!sized(measured(300), measured(700)).is_its_own_build_folder());
    }

    #[test]
    fn a_folder_sutra_cannot_read_has_no_measurements_to_wait_for() {
        let readable = sized(Measurement::Measuring, measured(700));
        assert_eq!(readable.measurements().count(), 2);
        let denied = Project {
            access: FolderAccess::Denied,
            ..readable
        };
        assert_eq!(denied.measurements().count(), 0);
    }
}
