//! The worktree map: everything Sutra knows about each project folder on this machine, in one shape.
//!
//! A project is a git worktree (main or linked), any folder a dev-runner environment names, or one
//! of a Claude Code session's scratch folders that holds a build folder or a lot of disk. One
//! `Project` row merges four sources:
//!   - git: branch, head and the time of the last git activity, read from the worktree's git dir
//!     without ever writing to it (a write would show up in our own file watcher);
//!   - dev-runner environments, from `~/.dev-runner/` and from the project's own `.dev-runner/`;
//!   - coding-agent sessions (Claude Code, Codex) whose working directory lies inside the project,
//!     and the subagents at work inside it, whichever session they work for;
//!   - build artifacts (`target/`, `node_modules/`) and their deduplicated disk use, and the disk
//!     use of the whole folder.
//!
//! The same code serves both editions. The direct build scans the real home folder and Claude
//! Code's scratch root; the Mac App Store build scans the folder the user granted through an open
//! panel (see `grant`). Inside the sandbox `$HOME` points at the app's container, so every path
//! here starts from `crate::home::real_home`.

pub mod agents;
pub mod clean;
pub mod devrunner;
pub mod disk;
pub mod engine;
pub mod grant;
mod project;
pub mod scan;
#[cfg(test)]
pub(crate) mod test_support;

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub use agents::{
    AgentSession, AgentStatus, Harness, ParentSession, SessionHost, Subagent, SubagentWithParent,
};
pub use disk::{DirUsage, Measurement};
pub use project::{
    ActivitySource, ArtifactDir, ArtifactKind, FolderAccess, FolderSize, KeptOutBy,
    LARGE_FOLDER_BYTES, Project, RepoInfo, ScratchSpace,
};

/// A home folder macOS guards with a privacy prompt the first time an app reads anything inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PrivateFolder {
    Desktop,
    Documents,
    Downloads,
}

impl PrivateFolder {
    pub const ALL: [PrivateFolder; 3] = [
        PrivateFolder::Desktop,
        PrivateFolder::Documents,
        PrivateFolder::Downloads,
    ];

    /// Its name in the home folder.
    pub fn name(self) -> &'static str {
        match self {
            PrivateFolder::Desktop => "Desktop",
            PrivateFolder::Documents => "Documents",
            PrivateFolder::Downloads => "Downloads",
        }
    }

    /// The private folder called `name` in the home folder, if one is.
    pub fn named(name: &str) -> Option<PrivateFolder> {
        PrivateFolder::ALL
            .into_iter()
            .find(|folder| folder.name() == name)
    }
}

/// What the scanner may enter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// The private folders the scanner may enter, as the user chose them in Settings. macOS asks
    /// the user about each the first time an app reads anything inside it, and the scanner stays
    /// out of each until macOS has answered (see `Boundary::keep_out_until_macos_answers`).
    pub private_folders: BTreeSet<PrivateFolder>,
}

/// One complete picture of the machine, published whole so a consumer never sees a half-updated map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The folders scanned and watched.
    pub roots: Vec<PathBuf>,
    /// The private folders inside the roots that Settings keeps Sutra out of, which the walk skips
    /// (see `ScanOptions`).
    pub skipped_private_folders: Vec<PathBuf>,
    /// The private folders Settings lets Sutra into and macOS keeps it out of, because the user
    /// said no to macOS's prompt. Only System Settings can change that.
    pub refused_private_folders: Vec<PrivateFolder>,
    /// Running inside the App Sandbox, where signals to other processes fail, so the UI offers no
    /// terminate button.
    pub sandboxed: bool,
    /// Newest activity first.
    pub projects: Vec<Project>,
    /// Agent sessions whose working directory lies in no known project.
    pub unplaced_agents: Vec<AgentSession>,
    /// Subagents at work outside every known project, which holds none of the folders they may be
    /// said to work in (see `Subagent::folders`), whichever session they work for, by session.
    pub unplaced_subagents: Vec<SubagentWithParent>,
    /// The sources that could not be read, ordered by source, so the map can say that what it
    /// shows from them is incomplete rather than empty.
    pub problems: Vec<SourceProblem>,
    pub scan: ScanStats,
    pub generated_at: SystemTime,
}

impl Snapshot {
    /// The build folder at `path`, when a project on the map holds it.
    pub fn artifact(&self, path: &Path) -> Option<&ArtifactDir> {
        self.projects
            .iter()
            .flat_map(|project| &project.artifacts)
            .find(|dir| dir.path == path)
    }

    /// How many agents are waiting on the user, in projects and outside them.
    pub fn waiting_agents(&self) -> usize {
        self.projects
            .iter()
            .flat_map(|project| &project.agents)
            .chain(&self.unplaced_agents)
            .filter(|agent| agent.status == AgentStatus::Waiting)
            .count()
    }

    /// How far the sizing of every project folder has come.
    pub fn sizing(&self) -> SizingProgress {
        let measurements = self.projects.iter().flat_map(Project::measurements);
        let (sized, total) = measurements.fold((0, 0), |(sized, total), measurement| {
            (sized + usize::from(measurement.is_settled()), total + 1)
        });
        SizingProgress { sized, total }
    }
}

/// How many of the folders Sutra sizes have a size: each readable project's own files and each of
/// its build folders. A folder whose measurement failed counts as sized, because it waits for a
/// change rather than a retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizingProgress {
    pub sized: usize,
    pub total: usize,
}

impl SizingProgress {
    pub fn is_finished(&self) -> bool {
        self.sized == self.total
    }
}

/// A source Sutra could not read. Without it, an unreadable source would read as an empty one:
/// inside the sandbox, a grant that does not cover `~/.claude` would look like no agent running.
/// A source that is simply absent (a harness never installed) is no problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProblem {
    pub source: Source,
    /// The file or folder that could not be read; when `count` is above one, the first of them.
    pub path: PathBuf,
    pub kind: io::ErrorKind,
    /// How many paths of this source failed with this `kind`.
    pub count: usize,
}

impl SourceProblem {
    /// One path that failed.
    pub(crate) fn new(source: Source, path: PathBuf, kind: io::ErrorKind) -> SourceProblem {
        SourceProblem {
            source,
            path,
            kind,
            count: 1,
        }
    }

    /// One problem per kind of error among `failures`, each naming the first of its paths, so a
    /// home folder with a hundred sealed folders reads as one line, not a hundred. A path that
    /// failed twice the same way counts once. The order is always the same for the same failures,
    /// so a source read again with nothing changed reports no change.
    pub(crate) fn per_kind(
        source: Source,
        mut failures: Vec<(PathBuf, io::ErrorKind)>,
    ) -> Vec<SourceProblem> {
        failures.sort();
        failures.dedup();
        let mut problems: Vec<SourceProblem> = Vec::new();
        for (path, kind) in failures {
            match problems.iter_mut().find(|problem| problem.kind == kind) {
                Some(problem) => problem.count += 1,
                None => problems.push(SourceProblem::new(source, path, kind)),
            }
        }
        problems
    }
}

/// Where a problem arose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// Discovery: a folder the walk could not list, or a worktree whose git state it could not
    /// read.
    Scan,
    /// The dev-runner registry, `~/.dev-runner/`, which could not be listed at all.
    Registry,
    /// Part of one environment's state: a meta or status file in the registry, or a project's own
    /// `.dev-runner/`. The other environments still show.
    EnvironmentState,
    /// Claude Code's session registry, `~/.claude/sessions/`, which could not be listed at all.
    ClaudeSessions,
    /// One session's own files: its file in that registry, or its subagents' transcripts under
    /// `~/.claude/projects/` and their folders. The other sessions still show.
    ClaudeSessionFile,
    /// The meta file beside a subagent's transcript, which names the subagent and its task. The
    /// subagent still shows, its transcript telling all that it is doing, but unnamed.
    ClaudeSubagentMeta,
    /// A session's own transcript under `~/.claude/projects/`, which names the model the session
    /// works with. The session still shows, its registry file telling all that it is doing, but
    /// without its model.
    ClaudeSessionTranscript,
    /// Codex's thread database, `~/.codex/state_5.sqlite`.
    CodexDatabase,
    /// A file watcher could not start, or could not watch a root or a folder outside the roots that
    /// the map needs file events from. The map then misses file changes there and refreshes only
    /// what its 2 s tick re-reads.
    Watcher,
}

impl Source {
    /// Whether the source tells what is at work in a project: failing to read it may hide an agent,
    /// a running environment or a project's latest activity. Discovery failing only loses folders,
    /// a subagent's meta file only its name, and a session's transcript only its model.
    pub fn tells_what_is_at_work(self) -> bool {
        match self {
            Source::Registry
            | Source::EnvironmentState
            | Source::ClaudeSessions
            | Source::ClaudeSessionFile
            | Source::CodexDatabase
            | Source::Watcher => true,
            Source::Scan | Source::ClaudeSubagentMeta | Source::ClaudeSessionTranscript => false,
        }
    }

    /// Whether the source alone tells of a kind of thing: Claude Code's sessions, Codex's
    /// threads, the environments registered in `~/.dev-runner/`. One that cannot be read hides
    /// that kind from the map whole, where discovery or the watcher failing, or one environment's
    /// or one session's file, loses only some of it.
    pub fn is_sole_source(self) -> bool {
        match self {
            Source::Registry | Source::ClaudeSessions | Source::CodexDatabase => true,
            Source::Scan
            | Source::EnvironmentState
            | Source::ClaudeSessionFile
            | Source::ClaudeSubagentMeta
            | Source::ClaudeSessionTranscript
            | Source::Watcher => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanStats {
    pub dirs_visited: usize,
    pub elapsed: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_of_one_kind_read_as_one_problem_naming_the_first_path_each_path_once() {
        let denied = io::ErrorKind::PermissionDenied;
        let binary = io::ErrorKind::InvalidData;
        let failures = vec![
            (PathBuf::from("/b"), denied),
            (PathBuf::from("/c"), binary),
            (PathBuf::from("/a"), denied),
            (PathBuf::from("/b"), denied),
        ];

        assert_eq!(
            SourceProblem::per_kind(Source::Scan, failures),
            [
                SourceProblem {
                    count: 2,
                    ..SourceProblem::new(Source::Scan, PathBuf::from("/a"), denied)
                },
                SourceProblem::new(Source::Scan, PathBuf::from("/c"), binary),
            ]
        );
    }

    #[test]
    fn one_environments_or_one_sessions_file_hides_only_that_one() {
        assert!(!Source::EnvironmentState.is_sole_source());
        assert!(!Source::ClaudeSessionFile.is_sole_source());
    }

    #[test]
    fn a_subagents_meta_file_hides_neither_its_subagent_nor_what_is_at_work() {
        assert!(!Source::ClaudeSubagentMeta.is_sole_source());
        assert!(!Source::ClaudeSubagentMeta.tells_what_is_at_work());
    }

    #[test]
    fn a_sessions_transcript_hides_neither_its_session_nor_what_is_at_work() {
        assert!(!Source::ClaudeSessionTranscript.is_sole_source());
        assert!(!Source::ClaudeSessionTranscript.tells_what_is_at_work());
    }
}
