//! Coding-agent sessions, read from each harness's own files. Nothing here runs or drives an agent.

mod claude;
mod codex;
mod session;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) use claude::is_subagent_transcript;
#[cfg(test)]
pub(crate) use claude::write_test_subagent_at_work;
pub use session::{AgentSession, AgentStatus, Harness, SessionHost};

use super::SourceProblem;
use super::scan::Boundary;

/// What the harness files that grow as an agent works said when last read: Codex's rollouts and
/// the transcripts of Claude Code's subagents. It is kept from one load to the next, so that a load
/// reads again only the files written since.
#[derive(Debug, Default)]
pub struct LastRead {
    rollouts: codex::Rollouts,
    transcripts: claude::Transcripts,
}

/// What one load found.
#[derive(Debug, Default)]
pub struct Loaded {
    pub sessions: Vec<AgentSession>,
    /// The harness files that exist but could not be read. A harness's sessions are all missing
    /// when its registry or database failed, while the other's still show; only one session is
    /// when one file failed.
    pub problems: Vec<SourceProblem>,
    /// The processes of the Claude Code sessions whose file was mid-rewrite, which said nothing:
    /// each is as it last was.
    pub being_rewritten: Vec<u32>,
}

/// Every live Claude Code session and every Codex thread that wrote within the last hour, as the
/// harnesses record them under the real home directory, each counting the work of its subagents.
/// Working directories are resolved only where `boundary` allows.
pub fn load(home: &Path, boundary: &Boundary, last_read: &mut LastRead) -> Loaded {
    let mut loaded = claude::load(home, boundary, &mut last_read.transcripts);
    match codex::load(home, boundary, &mut last_read.rollouts) {
        Ok(sessions) => loaded.sessions.extend(sessions),
        Err(problem) => loaded.problems.push(problem),
    }
    loaded
}

/// The canonical form of a recorded working directory, or the path as recorded when it may not or
/// cannot be resolved. Resolving a folder inside a private folder Settings keeps Sutra out of
/// would raise the privacy prompt; a session can outlive its folder (agents delete worktrees); and
/// inside the sandbox a folder outside the grant cannot be resolved. The recorded path is then
/// still the best join key.
fn resolve_cwd(recorded: &str, boundary: &Boundary) -> PathBuf {
    let recorded = PathBuf::from(recorded);
    if !boundary.may_read(&recorded) {
        return recorded;
    }
    fs::canonicalize(&recorded).unwrap_or(recorded)
}

/// A harness timestamp in milliseconds since the Unix epoch, or `None` past the clock's range.
fn time_from_unix_ms(ms: u64) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;
    use crate::map::{PrivateFolder, ScanOptions};

    #[test]
    fn missing_working_directory_keeps_its_recorded_path() {
        let home = TempDir::new("agents-cwd-missing");
        let gone = "/nonexistent/sutra/worktree";
        assert_eq!(
            resolve_cwd(gone, &home.boundary(&ScanOptions::default())),
            PathBuf::from(gone)
        );
    }

    #[test]
    fn existing_working_directory_is_canonicalized() {
        let home = TempDir::new("agents-cwd-canonical");
        let via_dot = home.root().join(".");
        assert_eq!(
            resolve_cwd(
                via_dot.to_str().unwrap(),
                &home.boundary(&ScanOptions::default())
            ),
            fs::canonicalize(home.root()).unwrap()
        );
    }

    #[test]
    fn working_directory_in_a_private_folder_is_resolved_only_when_settings_let_it_in() {
        let home = TempDir::new("agents-cwd-private");
        let app = home.root().join("Desktop").join("app");
        fs::create_dir_all(&app).unwrap();
        let recorded = home.root().join("Desktop").join(".").join("app");
        let recorded = recorded.to_str().unwrap();
        let desktop = ScanOptions {
            private_folders: [PrivateFolder::Desktop].into(),
        };

        assert_eq!(
            resolve_cwd(recorded, &home.boundary(&ScanOptions::default())),
            PathBuf::from(recorded)
        );
        assert_eq!(resolve_cwd(recorded, &home.boundary(&desktop)), app);
    }
}
