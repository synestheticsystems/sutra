//! Claude Code keeps one registry file per running process, `<home>/.claude/sessions/<pid>.json`,
//! and rewrites it whenever the session's status changes. The format is undocumented; the fields
//! read here were checked against Claude Code 2.1.310, and `entrypoint` and `hostSessionId`
//! against 2.1.317. The registry says nothing of a session's subagents, whose transcripts
//! `subagents` reads, nor of the model the session works with, which its own transcript names
//! (see `session_model`). Both lie where Claude Code keeps the session's files (see
//! `session_files`).

mod app_link;
mod session_files;
mod session_model;
mod subagents;
mod transcript;

use std::fs::{self, File};
use std::io::{self, Read};
use std::mem;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use self::session_files::{session_folder, session_transcript};
pub use self::session_model::SessionModels;
#[cfg(test)]
pub(crate) use self::subagents::write_test_subagent_at_work;
pub use self::subagents::{Transcripts, is_subagent_transcript};
use super::session::SubagentWork;
use super::{
    AgentSession, AgentStatus, Harness, Loaded, SessionHost, resolve_cwd, time_from_unix_ms,
};
use crate::map::scan::Boundary;
use crate::map::{Source, SourceProblem};
use crate::model::{note_failure, process_exists};

/// The most of one registry file read. Claude writes a few hundred bytes; a larger file is not one
/// of its entries, and is reported rather than read whole.
const MAX_ENTRY_BYTES: u64 = 64 * 1024;

/// How long after its last write a registry file that does not parse may still be mid-rewrite.
/// Claude rewrites the file in place, so a reader can catch it half-written, and the next load,
/// at most two seconds later, reads it whole. A file still invalid this long after its last write
/// is not being rewritten.
const REWRITE_SETTLES_WITHIN: Duration = Duration::from_secs(10);

/// The registry fields the map uses. A file missing a required field does not parse.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryEntry {
    pid: u32,
    session_id: String,
    cwd: String,
    /// busy, waiting, idle or shell.
    status: String,
    /// Why a waiting session waits, for example "permission_prompt".
    waiting_for: Option<String>,
    /// interactive, bg, daemon or daemon-worker.
    kind: Option<String>,
    name: Option<String>,
    /// Milliseconds since the Unix epoch, like `updated_at` and `started_at`.
    status_updated_at: Option<u64>,
    updated_at: Option<u64>,
    /// When the process started.
    started_at: Option<u64>,
    /// What started the session: "claude-desktop" for the Claude app, "cli" for a terminal, or
    /// another entry point Sutra does not open. This and `host_session_id` are read loosely, as
    /// any JSON value, because one that is not a string must cost only the session's host, not
    /// the whole session.
    #[serde(default)]
    entrypoint: serde_json::Value,
    /// The Claude app's own id for a session it runs.
    #[serde(default)]
    host_session_id: serde_json::Value,
}

/// The sessions of every Claude Code process still running, each with the model it works with and
/// the work of its subagents. No registry folder means Claude Code never ran here, and yields no
/// sessions; a registry folder that cannot be listed is a problem. So is a file in it, or a
/// subagent's transcript or folder, that cannot be read, which costs only its own session; a
/// subagent's meta file, which costs only the subagent's name; and a session's own transcript,
/// which costs only its model. `models` and `transcripts` hold the last load's readings of the
/// sessions' transcripts and of their subagents' files, and this load's afterwards.
pub(super) fn load(
    home: &Path,
    boundary: &Boundary,
    models: &mut SessionModels,
    transcripts: &mut Transcripts,
) -> Loaded {
    let registry = home.join(".claude").join("sessions");
    let dir = match fs::read_dir(&registry) {
        Ok(dir) => dir,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Loaded::default(),
        Err(err) => {
            return Loaded {
                problems: vec![SourceProblem::new(
                    Source::ClaudeSessions,
                    registry,
                    err.kind(),
                )],
                ..Loaded::default()
            };
        }
    };
    let mut sessions = Vec::new();
    let mut being_rewritten = Vec::new();
    let mut failures = Vec::new();
    let projects = home.join(".claude").join("projects");
    let earlier_models = mem::take(models);
    let mut unread_transcripts = Vec::new();
    let mut subagents = subagents::Reader::new(boundary, transcripts);
    for file in dir {
        let file = match file {
            Ok(file) => file,
            Err(err) => {
                note_failure(&registry, &err, &mut failures);
                continue;
            }
        };
        let path = file.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        // Anything but a regular file is not Claude's, and reading a FIFO would block the load.
        // `DirEntry::file_type` does not follow symlinks.
        match file.file_type() {
            Ok(kind) if kind.is_file() => {}
            Ok(_) => continue,
            Err(err) => {
                note_failure(&path, &err, &mut failures);
                continue;
            }
        }
        match read_entry(&path) {
            // A process that crashed leaves its file behind until the next Claude Code launch.
            Ok(Reading::Entry(entry)) if !process_exists(entry.pid) => {}
            Ok(Reading::Entry(entry)) => {
                let started = entry.started_at.and_then(time_from_unix_ms);
                let folder = session_folder(&projects, &entry.cwd, &entry.session_id);
                let (work, model) = match folder {
                    Ok(Some(folder)) => {
                        let work =
                            subagents.session_work(&folder, &entry.cwd, started, &mut failures);
                        let transcript = session_transcript(&folder);
                        let model = models.read(&transcript, &earlier_models);
                        let model = model.unwrap_or_else(|err| {
                            note_failure(&transcript, &err, &mut unread_transcripts);
                            None
                        });
                        (work, model)
                    }
                    Ok(None) => (SubagentWork::default(), None),
                    Err(err) => {
                        note_failure(&projects, &err, &mut failures);
                        (SubagentWork::default(), None)
                    }
                };
                let session = (*entry).into_session(boundary, model);
                sessions.push(session.with_subagent_work(work));
            }
            Ok(Reading::Gone) => {}
            Ok(Reading::Rewriting) => {
                being_rewritten.extend(process_named_by(&path).filter(|pid| process_exists(*pid)))
            }
            Err(err) => failures.push((path, err.kind())),
        }
    }
    let mut problems = SourceProblem::per_kind(Source::ClaudeSessionFile, failures);
    let unread_metas = subagents.unread_metas();
    problems.extend(SourceProblem::per_kind(
        Source::ClaudeSubagentMeta,
        unread_metas,
    ));
    problems.extend(SourceProblem::per_kind(
        Source::ClaudeSessionTranscript,
        unread_transcripts,
    ));
    Loaded {
        sessions,
        problems,
        being_rewritten,
    }
}

/// What reading a registry file came to.
enum Reading {
    Entry(Box<RegistryEntry>),
    /// It is gone: its process exited.
    Gone,
    /// It does not parse but may just be mid-rewrite (see `REWRITE_SETTLES_WITHIN`).
    Rewriting,
}

/// The process a registry file belongs to, which names it `<pid>.json`.
fn process_named_by(path: &Path) -> Option<u32> {
    path.file_stem()?.to_str()?.parse().ok()
}

/// Read a registry file. One larger than `MAX_ENTRY_BYTES` is `FileTooLarge`, and one that stays
/// unparsable is `InvalidData`.
fn read_entry(path: &Path) -> io::Result<Reading> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Reading::Gone),
        Err(err) => return Err(err),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    match serde_json::from_slice(&bytes) {
        Ok(entry) => Ok(Reading::Entry(Box::new(entry))),
        Err(err) => {
            // `elapsed` fails for a write dated in the future, which says nothing of its age.
            let age = file.metadata()?.modified()?.elapsed();
            if age.is_ok_and(|age| age >= REWRITE_SETTLES_WITHIN) {
                Err(io::Error::new(io::ErrorKind::InvalidData, err))
            } else {
                Ok(Reading::Rewriting)
            }
        }
    }
}

impl RegistryEntry {
    /// The session this entry registers, working with `model`, which its transcript names.
    fn into_session(self, boundary: &Boundary, model: Option<String>) -> AgentSession {
        let status = match self.status.as_str() {
            "busy" => AgentStatus::Busy,
            "waiting" => AgentStatus::Waiting,
            "idle" => AgentStatus::Idle,
            // "shell", whose meaning is unverified, and any status a later version adds.
            _ => AgentStatus::Unknown,
        };
        // The reason belongs to the current wait; a session that is not waiting waits for nothing.
        let waiting_for = self.waiting_for.filter(|_| status == AgentStatus::Waiting);
        AgentSession {
            harness: Harness::ClaudeCode,
            session_id: self.session_id,
            pid: Some(self.pid),
            cwd: resolve_cwd(&self.cwd, boundary),
            status,
            waiting_for,
            kind: self.kind,
            label: self.name,
            model,
            since: self.status_updated_at.and_then(time_from_unix_ms),
            last_activity: self
                .updated_at
                .max(self.status_updated_at)
                .and_then(time_from_unix_ms),
            host: host(self.entrypoint.as_str(), self.host_session_id.as_str()),
            subagents_at_work: Vec::new(),
        }
    }
}

/// Where the session runs, from its `entrypoint` and, in the Claude app, its `hostSessionId`.
fn host(entrypoint: Option<&str>, host_session_id: Option<&str>) -> Option<SessionHost> {
    match entrypoint? {
        "claude-desktop" => Some(SessionHost::ClaudeApp {
            app_session_id: host_session_id.map(str::to_string),
        }),
        "cli" => Some(SessionHost::Terminal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::map::ScanOptions;
    use crate::map::test_support::{DEAD_PID, TempDir, set_mtime};

    fn registry_json(pid: u32, session_id: &str, cwd: &Path, status: &str) -> String {
        serde_json::json!({
            "pid": pid,
            "sessionId": session_id,
            "cwd": cwd,
            "kind": "interactive",
            "status": status,
            "waitingFor": "permission_prompt",
            "startedAt": 1_000,
            "statusUpdatedAt": 2_000,
            "updatedAt": 3_000,
            "name": "fix the parser",
            "entrypoint": "cli",
            "hostSessionId": null,
        })
        .to_string()
    }

    fn write_registry(home: &Path, file: &str, contents: &str) {
        let dir = home.join(".claude").join("sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(file), contents).unwrap();
    }

    fn load_home(home: &TempDir) -> Loaded {
        load(
            home.root(),
            &home.boundary(&ScanOptions::default()),
            &mut SessionModels::default(),
            &mut Transcripts::default(),
        )
    }

    fn ids(loaded: &Loaded) -> Vec<&str> {
        let sessions = loaded.sessions.iter();
        sessions
            .map(|session| session.session_id.as_str())
            .collect()
    }

    /// The session one registry file describes, read with no folder off limits.
    fn session(json: &str) -> AgentSession {
        let entry: RegistryEntry = serde_json::from_str(json).unwrap();
        let boundary = Boundary::new(&[], Path::new("/"), &ScanOptions::default());
        entry.into_session(&boundary, None)
    }

    #[test]
    fn live_process_file_becomes_a_session() {
        let home = TempDir::new("agents-claude-live");
        let me = std::process::id();
        let json = registry_json(me, "live", home.root(), "waiting");
        write_registry(home.root(), &format!("{me}.json"), &json);

        let sessions = load_home(&home).sessions;

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.harness, Harness::ClaudeCode);
        assert_eq!(session.session_id, "live");
        assert_eq!(session.pid, Some(me));
        assert_eq!(session.cwd, fs::canonicalize(home.root()).unwrap());
        assert_eq!(session.status, AgentStatus::Waiting);
        assert_eq!(session.waiting_for.as_deref(), Some("permission_prompt"));
        assert_eq!(session.kind.as_deref(), Some("interactive"));
        assert_eq!(session.label.as_deref(), Some("fix the parser"));
        assert_eq!(
            session.since,
            Some(UNIX_EPOCH + Duration::from_millis(2_000))
        );
        assert_eq!(
            session.last_activity,
            Some(UNIX_EPOCH + Duration::from_millis(3_000))
        );
        assert_eq!(session.host, Some(SessionHost::Terminal));
    }

    /// A busy session started by `entrypoint` with `host_session_id`, each a JSON value.
    fn hosted(entrypoint: &str, host_session_id: &str) -> AgentSession {
        session(&format!(
            r#"{{"pid": 1, "sessionId": "s", "cwd": "/", "status": "busy",
                "entrypoint": {entrypoint}, "hostSessionId": {host_session_id}}}"#
        ))
    }

    #[test]
    fn a_session_in_the_claude_app_carries_the_apps_id_for_it() {
        let app_session_id = "local_2b5d1f5a-66c8-4826-b823-26ed94dd90a3";
        assert_eq!(
            hosted(r#""claude-desktop""#, &format!(r#""{app_session_id}""#)).host,
            Some(SessionHost::ClaudeApp {
                app_session_id: Some(app_session_id.to_string())
            })
        );
    }

    #[test]
    fn a_session_in_the_claude_app_without_a_usable_id_keeps_its_host_and_no_id() {
        let no_id = SessionHost::ClaudeApp {
            app_session_id: None,
        };
        for missing_or_malformed in ["null", "42", r#"{"id": "local_x"}"#] {
            let session = hosted(r#""claude-desktop""#, missing_or_malformed);
            assert_eq!(session.host, Some(no_id.clone()), "{missing_or_malformed}");
            assert_eq!(session.session_id, "s", "the session itself still loads");
        }
        let absent = session(
            r#"{"pid": 1, "sessionId": "s", "cwd": "/", "status": "busy",
                "entrypoint": "claude-desktop"}"#,
        );
        assert_eq!(absent.host, Some(no_id));
    }

    #[test]
    fn a_terminal_session_has_a_terminal_host_whatever_id_it_carries() {
        assert_eq!(
            hosted(r#""cli""#, r#""local_abc""#).host,
            Some(SessionHost::Terminal)
        );
    }

    #[test]
    fn an_unknown_or_missing_entry_point_names_no_host() {
        assert_eq!(hosted(r#""sdk-ts""#, "null").host, None);
        assert_eq!(hosted("7", "null").host, None);
        let absent = session(r#"{"pid": 1, "sessionId": "s", "cwd": "/", "status": "busy"}"#);
        assert_eq!(absent.host, None);
    }

    #[test]
    fn dead_foreign_and_just_written_malformed_files_are_skipped_without_a_problem() {
        let home = TempDir::new("agents-claude-skip");
        let me = std::process::id();
        assert!(!process_exists(DEAD_PID));
        let dead = registry_json(DEAD_PID, "dead", home.root(), "busy");
        write_registry(home.root(), &format!("{DEAD_PID}.json"), &dead);
        let live = registry_json(me, "live", home.root(), "busy");
        write_registry(home.root(), "half-written.json", &live[..live.len() / 2]);
        write_registry(
            home.root(),
            "no-status.json",
            &format!(r#"{{"pid": {me}, "sessionId": "s", "cwd": "/"}}"#),
        );
        write_registry(home.root(), ".0123abcd.key", &live);
        write_registry(home.root(), &format!("{me}.json"), &live);

        let loaded = load_home(&home);

        assert_eq!(ids(&loaded), ["live"]);
        assert_eq!(loaded.problems, []);
        assert_eq!(loaded.being_rewritten, [0u32; 0]);
    }

    #[test]
    fn a_live_processs_file_mid_rewrite_says_so_and_a_dead_ones_does_not() {
        let home = TempDir::new("agents-claude-rewriting");
        let me = std::process::id();
        let half = "{\"pid\": ";
        write_registry(home.root(), &format!("{me}.json"), half);
        write_registry(home.root(), &format!("{DEAD_PID}.json"), half);

        let loaded = load_home(&home);

        assert_eq!(ids(&loaded), [""; 0]);
        assert_eq!(loaded.problems, []);
        assert_eq!(loaded.being_rewritten, [me]);
    }

    /// `file` in the fixture's registry, last written long enough ago to have settled.
    fn settled(home: &TempDir, file: &str, contents: &str) -> PathBuf {
        let path = home.write(&format!(".claude/sessions/{file}"), contents);
        set_mtime(&path, SystemTime::now() - 2 * REWRITE_SETTLES_WITHIN);
        path
    }

    #[test]
    fn a_file_still_malformed_once_a_rewrite_would_have_settled_is_a_problem() {
        let home = TempDir::new("agents-claude-malformed");
        let me = std::process::id();
        let live = registry_json(me, "live", home.root(), "busy");
        write_registry(home.root(), &format!("{me}.json"), &live);
        let cut_short = settled(&home, "half-written.json", &live[..live.len() / 2]);
        let no_status = settled(
            &home,
            "no-status.json",
            &format!(r#"{{"pid": {me}, "sessionId": "s", "cwd": "/"}}"#),
        );

        let loaded = load_home(&home);

        assert_eq!(ids(&loaded), ["live"]);
        let first = cut_short.min(no_status);
        assert_eq!(
            loaded.problems,
            [SourceProblem {
                count: 2,
                ..SourceProblem::new(Source::ClaudeSessionFile, first, io::ErrorKind::InvalidData)
            }]
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_is_a_problem_beside_its_siblings() {
        let home = TempDir::new("agents-claude-sealed-file");
        let me = std::process::id();
        let live = registry_json(me, "live", home.root(), "busy");
        write_registry(home.root(), &format!("{me}.json"), &live);
        let sealed = home.write(".claude/sessions/sealed.json", &live);
        home.restrict(&sealed, 0o000);

        let loaded = load_home(&home);

        assert_eq!(ids(&loaded), ["live"]);
        assert_eq!(
            loaded.problems,
            [SourceProblem::new(
                Source::ClaudeSessionFile,
                sealed,
                io::ErrorKind::PermissionDenied
            )]
        );
    }

    #[test]
    fn a_file_gone_by_the_time_it_is_read_is_no_problem() {
        let home = TempDir::new("agents-claude-vanished");
        let gone = home.path(".claude/sessions/4242.json");
        assert!(matches!(read_entry(&gone), Ok(Reading::Gone)));
    }

    #[test]
    fn a_registry_file_that_is_not_regular_is_passed_over_and_one_too_large_is_a_problem() {
        let home = TempDir::new("agents-claude-bounded");
        let me = std::process::id();
        let live = registry_json(me, "live", home.root(), "busy");
        write_registry(home.root(), &format!("{me}.json"), &live);
        // Valid JSON once read whole, so only the size cap keeps it out.
        let padded = registry_json(me, "oversized", home.root(), "busy")
            + &" ".repeat(MAX_ENTRY_BYTES as usize);
        write_registry(home.root(), "oversized.json", &padded);
        let oversized = home.path(".claude/sessions/oversized.json");
        let fifo = home
            .root()
            .join(".claude")
            .join("sessions")
            .join("fifo.json");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());

        // On a thread, so a read that blocks on the FIFO fails the test instead of hanging it.
        let (sender, receiver) = std::sync::mpsc::channel();
        let path = home.root().to_path_buf();
        std::thread::spawn(move || {
            let boundary = Boundary::new(&[], &path, &ScanOptions::default());
            let (mut models, mut transcripts) = Default::default();
            let _ = sender.send(load(&path, &boundary, &mut models, &mut transcripts));
        });
        let loaded = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the load returns despite a FIFO in the registry");

        assert_eq!(ids(&loaded), ["live"]);
        assert_eq!(
            loaded.problems,
            [SourceProblem::new(
                Source::ClaudeSessionFile,
                oversized,
                io::ErrorKind::FileTooLarge
            )]
        );
    }

    /// A live session `0b1d` of this process in `cwd`, whose process started two minutes before
    /// `now` and which has been idle for the last minute.
    fn idle_session(home: &TempDir, cwd: &Path, now: SystemTime) {
        let ms = |ago: u64| {
            let at = now - Duration::from_secs(ago);
            at.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
        };
        let me = std::process::id();
        let json = serde_json::json!({
            "pid": me,
            "sessionId": "0b1d",
            "cwd": cwd,
            "status": "idle",
            "startedAt": ms(120),
            "statusUpdatedAt": ms(60),
            "updatedAt": ms(60),
        });
        write_registry(home.root(), &format!("{me}.json"), &json.to_string());
    }

    #[test]
    fn a_session_idle_while_its_subagent_works_is_busy_and_counts_it() {
        let home = TempDir::new("agents-claude-subagent");
        let now = SystemTime::now();
        let cwd = home.dir("app");
        idle_session(&home, &cwd, now);
        let cwd = cwd.to_str().unwrap();
        let last_write = now - Duration::from_secs(30);
        write_test_subagent_at_work(home.root(), cwd, "0b1d", "working", cwd, last_write);
        // Cut off mid-turn when the session's earlier process ended.
        let cut_off = now - Duration::from_secs(300);
        write_test_subagent_at_work(home.root(), cwd, "0b1d", "cut-off", cwd, cut_off);

        let sessions = load_home(&home).sessions;

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.status, AgentStatus::Busy);
        let subagents = session.subagents_at_work.iter();
        let ids: Vec<&str> = subagents.map(|subagent| subagent.id.as_str()).collect();
        assert_eq!(ids, ["working"]);
        assert_eq!(session.since, None);
        // Claude Code dates an entry to the millisecond.
        let behind = last_write.duration_since(session.last_activity.unwrap());
        assert!(
            behind.unwrap() < Duration::from_millis(1),
            "the subagent's write"
        );
    }

    #[test]
    fn a_subagents_folder_that_cannot_be_read_costs_its_session_its_subagents_alone() {
        let home = TempDir::new("agents-claude-subagents-sealed");
        let now = SystemTime::now();
        let cwd = home.dir("app");
        idle_session(&home, &cwd, now);
        let cwd = cwd.to_str().unwrap();
        let working = write_test_subagent_at_work(home.root(), cwd, "0b1d", "working", cwd, now);
        let folder = working.parent().unwrap().to_path_buf();
        home.restrict(&folder, 0o000);

        let loaded = load_home(&home);

        assert_eq!(ids(&loaded), ["0b1d"]);
        assert_eq!(loaded.sessions[0].status, AgentStatus::Idle);
        assert_eq!(
            loaded.problems,
            [SourceProblem::new(
                Source::ClaudeSessionFile,
                folder,
                io::ErrorKind::PermissionDenied
            )]
        );
    }

    #[test]
    fn a_subagents_meta_file_that_cannot_be_read_is_a_problem_of_its_own_and_costs_only_names() {
        let home = TempDir::new("agents-claude-subagent-meta");
        let now = SystemTime::now();
        let cwd = home.dir("app");
        idle_session(&home, &cwd, now);
        let cwd = cwd.to_str().unwrap();
        let working = write_test_subagent_at_work(home.root(), cwd, "0b1d", "working", cwd, now);
        let meta = working.with_extension("meta.json");
        fs::write(&meta, r#"{"name": "dashboard-fixes"}"#).unwrap();
        home.restrict(&meta, 0o000);

        let loaded = load_home(&home);

        let session = &loaded.sessions[0];
        assert_eq!(
            session.status,
            AgentStatus::Busy,
            "its subagent still counts"
        );
        assert_eq!(session.subagents_at_work[0].name, None);
        assert_eq!(
            loaded.problems,
            [SourceProblem::new(
                Source::ClaudeSubagentMeta,
                meta,
                io::ErrorKind::PermissionDenied
            )]
        );
    }

    /// Writes the transcript of the session `0b1d` that `idle_session` registers in `cwd`, holding
    /// `lines`. Returns its path.
    fn write_session_transcript(home: &TempDir, cwd: &Path, lines: &[String]) -> PathBuf {
        let projects = home.path(".claude/projects");
        let folder = projects.join(session_files::folder_name(cwd.to_str().unwrap()));
        let path = session_transcript(&folder.join("0b1d"));
        transcript::write_test_transcript(&path, lines);
        path
    }

    #[test]
    fn a_live_session_works_with_the_model_its_transcript_names_as_read_on_from_the_last_load() {
        let home = TempDir::new("agents-claude-model");
        let now = SystemTime::now();
        let cwd = home.dir("app");
        idle_session(&home, &cwd, now);
        let reply = transcript::test_reply(now, Some("end_turn"), "text");
        let path = write_session_transcript(&home, &cwd, std::slice::from_ref(&reply));
        let boundary = home.boundary(&ScanOptions::default());
        let (mut models, mut transcripts) = Default::default();
        let mut model = || {
            let loaded = load(home.root(), &boundary, &mut models, &mut transcripts);
            loaded.sessions[0].model.clone()
        };
        let opus = Some(transcript::TEST_MODEL.to_string());
        assert_eq!(model(), opus);

        // Another model's reply of the same length takes the first one's place, where only a read
        // from the start would find it, and the session writes on.
        let other = transcript::test_written_by(reply, serde_json::json!("claude-test-5-5"));
        let prompt = transcript::test_prompt(now, cwd.to_str().unwrap());
        transcript::write_test_transcript(&path, &[other, prompt]);

        assert_eq!(
            model(),
            opus,
            "this load reads on from where the last stopped"
        );
        let afresh = load_home(&home).sessions[0].model.clone();
        assert_eq!(afresh.as_deref(), Some("claude-test-5-5"));
    }

    #[test]
    fn a_sessions_transcript_that_cannot_be_read_is_a_problem_of_its_own_and_costs_only_its_model()
    {
        let home = TempDir::new("agents-claude-transcript-sealed");
        let now = SystemTime::now();
        let cwd = home.dir("app");
        idle_session(&home, &cwd, now);
        let reply = transcript::test_reply(now, Some("end_turn"), "text");
        let path = write_session_transcript(&home, &cwd, &[reply]);
        home.restrict(&path, 0o000);

        let loaded = load_home(&home);

        assert_eq!(ids(&loaded), ["0b1d"]);
        let session = &loaded.sessions[0];
        assert_eq!((session.status, &session.model), (AgentStatus::Idle, &None));
        assert_eq!(
            loaded.problems,
            [SourceProblem::new(
                Source::ClaudeSessionTranscript,
                path,
                io::ErrorKind::PermissionDenied
            )]
        );
    }

    #[test]
    fn missing_registry_folder_yields_no_sessions() {
        let home = TempDir::new("agents-claude-none");
        let loaded = load_home(&home);
        assert!(loaded.sessions.is_empty());
        assert_eq!(loaded.problems, []);
    }

    #[test]
    fn registry_folder_that_cannot_be_listed_is_a_problem_not_an_empty_list() {
        let home = TempDir::new("agents-claude-sealed");
        let registry = home.root().join(".claude").join("sessions");
        fs::create_dir_all(&registry).unwrap();
        home.restrict(&registry, 0o000);

        assert_eq!(
            load_home(&home).problems,
            [SourceProblem::new(
                Source::ClaudeSessions,
                registry,
                io::ErrorKind::PermissionDenied
            )]
        );
    }

    #[test]
    fn statuses_map_to_the_shared_vocabulary() {
        let cases = [
            ("busy", AgentStatus::Busy),
            ("waiting", AgentStatus::Waiting),
            ("idle", AgentStatus::Idle),
            ("shell", AgentStatus::Unknown),
            ("rebooting", AgentStatus::Unknown),
        ];
        for (raw, expected) in cases {
            let json = registry_json(1, "s", Path::new("/"), raw);
            assert_eq!(session(&json).status, expected, "{raw}");
        }
    }

    #[test]
    fn reason_for_waiting_is_dropped_once_the_session_stops_waiting() {
        let json = registry_json(1, "s", Path::new("/"), "idle");
        assert_eq!(session(&json).waiting_for, None);
    }

    #[test]
    fn optional_fields_may_be_absent_or_null() {
        let session = session(
            r#"{"pid": 1, "sessionId": "s", "cwd": "/gone/worktree", "status": "busy",
                "waitingFor": null, "updatedAt": 5000}"#,
        );
        assert_eq!(session.cwd, PathBuf::from("/gone/worktree"));
        assert_eq!(session.kind, None);
        assert_eq!(session.label, None);
        assert_eq!(session.since, None);
        assert_eq!(
            session.last_activity,
            Some(UNIX_EPOCH + Duration::from_millis(5_000))
        );
    }

    #[test]
    fn last_activity_is_the_later_of_the_two_update_times() {
        let session = session(
            r#"{"pid": 1, "sessionId": "s", "cwd": "/", "status": "busy",
                "statusUpdatedAt": 9000, "updatedAt": 4000}"#,
        );
        assert_eq!(
            session.last_activity,
            Some(UNIX_EPOCH + Duration::from_millis(9_000))
        );
    }
}
