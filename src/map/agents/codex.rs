//! Codex lists every thread in `<home>/.codex/state_5.sqlite` (table `threads`: where it runs and
//! when it last wrote) and appends each thread's events to its rollout file. Whether a turn is
//! running comes from the newest turn event in the rollout, and how long the thread has been
//! silent from when the rollout was last written. The same goes for the threads Codex spawns as
//! sub-agents (see `sub_agents`).

mod rollout;
mod sub_agents;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::mem;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, Row};

pub use self::rollout::Rollouts;
use self::rollout::{Rollout, Turn};
use self::sub_agents::Threads;
use super::session::SubagentWork;
use super::{AgentSession, AgentStatus, Harness, Subagent, resolve_cwd, time_from_unix_ms};
use crate::map::scan::Boundary;
use crate::map::{Source, SourceProblem};

/// Threads that have not written for longer than this are left off the map. It is the only limit
/// on which threads a load asks for. A limit on how many would have a burst of `codex exec` jobs
/// push an interactive thread off the map, and `Rollouts` keeps a burst cheap to load again.
const RECENT_MS: i64 = 60 * 60 * 1000;
/// An open turn whose rollout has gone unwritten for longer than this reads as quiet rather than
/// busy (see `status`). Codex appends to the rollout at every step of a turn, but a long reasoning
/// step or tool run can leave tens of seconds between writes, so a shorter wait would flip a
/// working thread between busy and quiet. The card counts quiet time in whole minutes, so a minute
/// is also when there is first something to count.
const QUIET_AFTER_MS: i64 = 60 * 1000;

/// The threads that wrote recently, newest first, of every source; `sub_agents` sorts them out. A
/// thread without a working directory cannot be placed on the map. The `+` keeps SQLite off the
/// index on `archived`, which holds nearly every thread ever run, so that it visits only the recent
/// rows, by the index on `updated_at_ms`: on 2,700 threads, 0.03 ms where the other way took 3 ms
/// (2026-11-04), and this runs on every tick.
const RECENT_THREADS: &str = "
    SELECT id, rollout_path, cwd, source, updated_at_ms
    FROM threads
    WHERE +archived = 0
      AND updated_at_ms >= ?1
      AND cwd IS NOT NULL
    ORDER BY updated_at_ms DESC";

/// One thread, by id, unless it is off the map.
const THREAD_BY_ID: &str = "
    SELECT id, rollout_path, cwd, source, updated_at_ms
    FROM threads
    WHERE id = ?1
      AND +archived = 0
      AND cwd IS NOT NULL";

/// The user-facing Codex threads (the terminal UI, the Desktop app, `codex exec`) that wrote within
/// the last hour or whose sub-agents did, each counting the work of its sub-agents. No database
/// means Codex never ran here, and yields no sessions; a database that cannot be read or queried
/// (one whose schema Codex changed) is a problem. `rollouts` holds the last load's readings, and
/// this load's afterwards.
pub(super) fn load(
    home: &Path,
    boundary: &Boundary,
    rollouts: &mut Rollouts,
) -> Result<Vec<AgentSession>, SourceProblem> {
    let db = home.join(".codex").join("state_5.sqlite");
    let problem = |kind| SourceProblem::new(Source::CodexDatabase, db.clone(), kind);
    match fs::metadata(&db) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(problem(err.kind())),
    }
    let now = SystemTime::now();
    let now_ms = unix_ms(now);
    let threads = read_threads(&db, now_ms - RECENT_MS).map_err(|err| problem(error_kind(&err)))?;
    let earlier = mem::take(rollouts);
    let mut work: HashMap<String, SubagentWork> = HashMap::new();
    for (sub_agent, parent) in threads.sub_agents {
        let rollout = sub_agent.rollout(rollouts, &earlier);
        let newest_turn = rollout.as_ref().and_then(|rollout| rollout.last_turn);
        let silent_since_ms = sub_agent.silent_since_ms(rollout.as_ref());
        let mid_turn = newest_turn == Some(Turn::Open);
        let written = time_from_codex_ms(silent_since_ms);
        work.entry(parent)
            .or_default()
            .count(mid_turn, written, now, || {
                sub_agent.into_subagent(newest_turn, silent_since_ms, now_ms, boundary)
            });
    }
    Ok(threads
        .user_facing
        .into_iter()
        .map(|thread| {
            let rollout = thread.rollout(rollouts, &earlier);
            let work = work.remove(&thread.id).unwrap_or_default();
            thread
                .into_session(rollout, now_ms, boundary)
                .with_subagent_work(work)
        })
        .collect())
}

/// The I/O error nearest to a failed read of the database, for the problem it is reported as. The
/// file exists by then, so SQLite's `CannotOpen` means it may not be opened; an error SQLite does
/// not tie to the file (most likely a column Codex renamed) is data Sutra cannot read.
fn error_kind(err: &rusqlite::Error) -> io::ErrorKind {
    match err.sqlite_error_code() {
        Some(ErrorCode::CannotOpen | ErrorCode::PermissionDenied | ErrorCode::ReadOnly) => {
            io::ErrorKind::PermissionDenied
        }
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => io::ErrorKind::ResourceBusy,
        _ => io::ErrorKind::InvalidData,
    }
}

/// One row of `threads`.
#[derive(Debug)]
struct Thread {
    id: String,
    /// `None` reads as an unknown status, like a rollout file that is gone.
    rollout_path: Option<String>,
    cwd: String,
    /// cli, vscode (the Desktop app) or exec for a user-facing thread; JSON for the others.
    source: String,
    updated_at_ms: i64,
}

impl Thread {
    fn from_row(row: &Row) -> rusqlite::Result<Thread> {
        Ok(Thread {
            id: row.get(0)?,
            rollout_path: row.get(1)?,
            cwd: row.get(2)?,
            source: row.get(3)?,
            updated_at_ms: row.get(4)?,
        })
    }

    /// What the thread's rollout says, read as `Rollouts::read` reads it.
    fn rollout(&self, rollouts: &mut Rollouts, earlier: &Rollouts) -> Option<Rollout> {
        let path = self.rollout_path.as_deref()?;
        rollouts.read(Path::new(path), earlier)
    }

    /// When the thread last wrote, as its silence is measured: the rollout's modification time,
    /// which is when Codex last wrote anything for the thread. The row's `updated_at_ms` is written
    /// at a cadence nobody has measured, so it stands in only when the rollout gives no time.
    fn silent_since_ms(&self, rollout: Option<&Rollout>) -> i64 {
        rollout
            .and_then(|rollout| rollout.written)
            .map_or(self.updated_at_ms, unix_ms)
    }

    /// The session this thread is, given what its rollout says.
    fn into_session(
        self,
        rollout: Option<Rollout>,
        now_ms: i64,
        boundary: &Boundary,
    ) -> AgentSession {
        let newest_turn = rollout.as_ref().and_then(|rollout| rollout.last_turn);
        let silent_since_ms = self.silent_since_ms(rollout.as_ref());
        // The row's time counts as activity only when it is the newer one.
        let last_write_ms = silent_since_ms.max(self.updated_at_ms);
        let status = status(newest_turn, now_ms - silent_since_ms);
        AgentSession {
            harness: Harness::Codex,
            session_id: self.id,
            pid: None,
            cwd: resolve_cwd(&self.cwd, boundary),
            status,
            // Codex never writes an approval request to disk, so a thread blocked on the user
            // reads as busy, then as quiet.
            waiting_for: None,
            kind: Some(self.source),
            label: None,
            since: quiet_since(status, silent_since_ms),
            last_activity: time_from_codex_ms(last_write_ms),
            host: None,
            subagents_at_work: Vec::new(),
        }
    }

    /// The sub-agent at work this thread is, given its newest turn event and when it was last
    /// written: busy, or quiet once silent mid-turn as long as the user's own thread would be (see
    /// `status`).
    fn into_subagent(
        self,
        newest_turn: Option<Turn>,
        silent_since_ms: i64,
        now_ms: i64,
        boundary: &Boundary,
    ) -> Subagent {
        let status = status(newest_turn, now_ms - silent_since_ms);
        Subagent {
            name: self.nickname(),
            id: self.id,
            cwd: resolve_cwd(&self.cwd, boundary),
            // Codex records no task in a few words for a sub-agent, as Claude Code does.
            description: None,
            status,
            since: quiet_since(status, silent_since_ms),
            last_write: time_from_codex_ms(silent_since_ms),
        }
    }
}

/// When a thread's status began: a quiet thread's dates from its last write, at
/// `silent_since_ms`, and the others record no start.
fn quiet_since(status: AgentStatus, silent_since_ms: i64) -> Option<SystemTime> {
    (status == AgentStatus::Quiet)
        .then(|| time_from_codex_ms(silent_since_ms))
        .flatten()
}

/// A time as Codex stores it, in milliseconds since the Unix epoch, or `None` before the epoch or
/// past the clock's range.
fn time_from_codex_ms(ms: i64) -> Option<SystemTime> {
    u64::try_from(ms).ok().and_then(time_from_unix_ms)
}

/// Read the threads written since `since_ms`, and those their sub-agents descend from, through a
/// read-only connection. Such a connection still needs the write-ahead log's shared-memory file.
/// While Codex runs that file exists; once Codex exits it is gone, and a read-only folder (a
/// sandboxed build's grant can be one) does not let SQLite create it. Then the read is retried
/// with `immutable=1`, which skips the log. That loses nothing after a clean exit, because closing
/// the database copies the log into the main file.
fn read_threads(db: &Path, since_ms: i64) -> rusqlite::Result<Threads> {
    match query_threads(db, "mode=ro", since_ms) {
        Err(err) if cannot_use_log(&err) => query_threads(db, "mode=ro&immutable=1", since_ms),
        result => result,
    }
}

/// The errors SQLite reports when it cannot open or create the log or its shared-memory file.
fn cannot_use_log(err: &rusqlite::Error) -> bool {
    matches!(
        err.sqlite_error_code(),
        Some(ErrorCode::CannotOpen | ErrorCode::ReadOnly)
    )
}

fn query_threads(db: &Path, params: &str, since_ms: i64) -> rusqlite::Result<Threads> {
    let uri = format!("file:{}?{params}", uri_path(db));
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let recent = conn
        .prepare(RECENT_THREADS)?
        .query_map([since_ms], Thread::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let mut by_id = conn.prepare(THREAD_BY_ID)?;
    Threads::sort_out(recent, |id| {
        by_id.query_row([id], Thread::from_row).optional()
    })
}

/// A file path as a SQLite URI path: SQLite reads `?` and `#` as delimiters and `%` as an escape.
fn uri_path(path: &Path) -> String {
    let mut uri = String::new();
    for c in path.to_string_lossy().chars() {
        match c {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            c => uri.push(c),
        }
    }
    uri
}

/// Milliseconds since the Unix epoch, the unit Codex stores. A clock set before 1970 reads as 0.
fn unix_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

/// What a thread is doing, from its newest turn event and how long ago its rollout was written.
fn status(newest_turn: Option<Turn>, silent_ms: i64) -> AgentStatus {
    match newest_turn {
        Some(Turn::Closed) => AgentStatus::Idle,
        Some(Turn::Open) if silent_ms <= QUIET_AFTER_MS => AgentStatus::Busy,
        // An open turn silent this long is working on something long, blocked on an approval
        // prompt, which Codex never writes down, or its process died mid-turn; the files cannot
        // tell which.
        Some(Turn::Open) => AgentStatus::Quiet,
        None => AgentStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::rollout::{test_event, test_outputs_past_a_chunk, write_test_rollout};
    use super::*;
    use crate::map::ScanOptions;
    use crate::map::test_support::{TempDir, set_mtime};

    const SECOND_MS: i64 = 1000;
    const MINUTE_MS: i64 = 60 * SECOND_MS;

    /// A Codex home holding a `threads` table with the columns `load` reads, in the write-ahead
    /// log mode Codex uses.
    fn create_db(home: &Path) -> Connection {
        let dir = home.join(".codex");
        fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("state_5.sqlite")).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE threads (
                 id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, created_at_ms INTEGER,
                 updated_at_ms INTEGER, source TEXT NOT NULL, cwd TEXT NOT NULL, title TEXT,
                 archived INTEGER NOT NULL DEFAULT 0, git_branch TEXT
             );",
        )
        .unwrap();
        conn
    }

    struct Row<'a> {
        id: &'a str,
        rollout: &'a Path,
        cwd: &'a Path,
        source: &'a str,
        age_ms: i64,
        archived: bool,
    }

    /// A user-facing, unarchived thread that last wrote `age_ms` ago.
    fn recent<'a>(id: &'a str, rollout: &'a Path, cwd: &'a Path, age_ms: i64) -> Row<'a> {
        Row {
            id,
            rollout,
            cwd,
            source: "cli",
            age_ms,
            archived: false,
        }
    }

    fn insert(conn: &Connection, row: Row) {
        conn.execute(
            "INSERT INTO threads (id, rollout_path, updated_at_ms, source, cwd, archived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                row.id,
                row.rollout.to_str().unwrap(),
                unix_ms(SystemTime::now()) - row.age_ms,
                row.source,
                row.cwd.to_str().unwrap(),
                row.archived,
            ),
        )
        .unwrap();
    }

    /// A rollout `name` under the fake Codex home whose newest turn event is `kind`, last written
    /// `age_ms` ago.
    fn rollout(home: &Path, name: &str, kind: &str, age_ms: i64) -> PathBuf {
        let path = home
            .join(".codex")
            .join("sessions")
            .join(format!("{name}.jsonl"));
        write_test_rollout(&path, &[test_event(kind)]);
        let age = Duration::from_millis(age_ms as u64);
        set_mtime(&path, SystemTime::now() - age);
        path
    }

    /// How long ago `session` was last active.
    fn idle_for(session: &AgentSession) -> Duration {
        SystemTime::now()
            .duration_since(session.last_activity.unwrap())
            .unwrap()
    }

    fn load_home(home: &TempDir) -> Result<Vec<AgentSession>, SourceProblem> {
        load(
            home.root(),
            &home.boundary(&ScanOptions::default()),
            &mut Rollouts::default(),
        )
    }

    fn ids(sessions: &[AgentSession]) -> Vec<&str> {
        sessions.iter().map(|s| s.session_id.as_str()).collect()
    }

    #[test]
    fn missing_database_yields_no_sessions() {
        let home = TempDir::new("agents-codex-none");
        assert!(load_home(&home).unwrap().is_empty());
    }

    #[test]
    fn recent_user_threads_load_newest_first_with_status_from_the_rollout() {
        let home = TempDir::new("agents-codex-load");
        let h = home.root();
        let cwd = h.join("worktree");
        fs::create_dir_all(&cwd).unwrap();
        let started = rollout(h, "started", "task_started", 10 * SECOND_MS);
        let complete = rollout(h, "complete", "task_complete", 2 * MINUTE_MS);
        let silent = rollout(h, "silent", "task_started", 30 * MINUTE_MS);
        let gone = h.join(".codex").join("sessions").join("gone.jsonl");
        let conn = create_db(h);
        insert(&conn, recent("busy", &started, &cwd, MINUTE_MS));
        insert(&conn, recent("idle", &complete, &cwd, 2 * MINUTE_MS));
        insert(&conn, recent("no-rollout", &gone, &cwd, 3 * MINUTE_MS));
        insert(&conn, recent("silent", &silent, &cwd, 30 * MINUTE_MS));
        insert(&conn, recent("old", &started, &cwd, 2 * RECENT_MS));
        let archived = recent("archived", &started, &cwd, MINUTE_MS);
        insert(
            &conn,
            Row {
                archived: true,
                ..archived
            },
        );
        let sub_agent = recent("sub-agent", &started, &cwd, MINUTE_MS);
        insert(
            &conn,
            Row {
                source: "thread_spawn",
                ..sub_agent
            },
        );

        let sessions = load_home(&home).unwrap();

        assert_eq!(ids(&sessions), ["busy", "idle", "no-rollout", "silent"]);
        let statuses: Vec<AgentStatus> = sessions.iter().map(|s| s.status).collect();
        use AgentStatus::{Busy, Idle, Quiet, Unknown};
        assert_eq!(statuses, [Busy, Idle, Unknown, Quiet]);
        let busy = &sessions[0];
        assert_eq!(busy.harness, Harness::Codex);
        assert_eq!(busy.cwd, fs::canonicalize(&cwd).unwrap());
        assert_eq!(busy.kind.as_deref(), Some("cli"));
        assert_eq!(busy.pid, None);
        assert_eq!(busy.waiting_for, None);
        assert_eq!(busy.label, None);
        assert_eq!(busy.since, None);
        assert_eq!(busy.host, None);
        // The rollout, written 10 s ago, is newer than the row.
        let age = idle_for(busy);
        assert!(
            age >= Duration::from_secs(10) && age < Duration::from_secs(20),
            "{age:?}"
        );
    }

    #[test]
    fn a_thread_silent_mid_turn_for_over_a_minute_is_quiet_since_its_last_write() {
        let home = TempDir::new("agents-codex-quiet");
        let h = home.root();
        let four_minutes = rollout(h, "four", "task_started", 4 * MINUTE_MS);
        let just_under = rollout(h, "under", "task_started", QUIET_AFTER_MS - 5 * SECOND_MS);
        let conn = create_db(h);
        insert(&conn, recent("quiet", &four_minutes, h, 4 * MINUTE_MS));
        insert(&conn, recent("busy", &just_under, h, 5 * MINUTE_MS));

        let sessions = load_home(&home).unwrap();

        assert_eq!(ids(&sessions), ["quiet", "busy"]);
        let quiet = &sessions[0];
        assert_eq!(quiet.status, AgentStatus::Quiet);
        let silent_for = SystemTime::now()
            .duration_since(quiet.since.expect("a quiet thread dates its silence"))
            .unwrap();
        assert!(
            silent_for >= Duration::from_secs(240) && silent_for < Duration::from_secs(250),
            "{silent_for:?}"
        );
        assert_eq!(sessions[1].status, AgentStatus::Busy);
        assert_eq!(sessions[1].since, None);
    }

    #[test]
    fn silence_is_measured_from_the_rollout_and_activity_is_the_newer_write() {
        let home = TempDir::new("agents-codex-silence");
        let h = home.root();
        let fresh = rollout(h, "fresh", "task_started", 10 * SECOND_MS);
        let stale = rollout(h, "stale", "task_started", 30 * MINUTE_MS);
        let conn = create_db(h);
        insert(&conn, recent("rollout-silent", &stale, h, MINUTE_MS));
        insert(&conn, recent("row-behind", &fresh, h, 30 * MINUTE_MS));

        let sessions = load_home(&home).unwrap();

        assert_eq!(ids(&sessions), ["rollout-silent", "row-behind"]);
        let statuses: Vec<AgentStatus> = sessions.iter().map(|s| s.status).collect();
        assert_eq!(statuses, [AgentStatus::Quiet, AgentStatus::Busy]);
        // Activity is the newer write: the row for the first thread, the rollout for the second.
        let ages: Vec<u64> = sessions.iter().map(|s| idle_for(s).as_secs()).collect();
        assert!((60..70).contains(&ages[0]), "{ages:?}");
        assert!((10..20).contains(&ages[1]), "{ages:?}");
    }

    #[test]
    fn a_quiet_interactive_thread_survives_more_than_fifty_newer_finished_exec_jobs() {
        let home = TempDir::new("agents-codex-burst");
        let h = home.root();
        let interactive = rollout(h, "interactive", "task_started", 40 * MINUTE_MS);
        let finished = rollout(h, "finished", "task_complete", MINUTE_MS);
        let conn = create_db(h);
        insert(
            &conn,
            recent("interactive", &interactive, h, 40 * MINUTE_MS),
        );
        let jobs: Vec<String> = (0..60).map(|n| format!("job{n:02}")).collect();
        for (n, id) in jobs.iter().enumerate() {
            let job = recent(id, &finished, h, n as i64 * SECOND_MS);
            insert(
                &conn,
                Row {
                    source: "exec",
                    ..job
                },
            );
        }

        let sessions = load_home(&home).unwrap();

        assert_eq!(sessions.len(), 61, "every thread that wrote this hour");
        let oldest = sessions.last().unwrap();
        assert_eq!(oldest.session_id, "interactive");
        assert_eq!(oldest.status, AgentStatus::Quiet);
        assert_eq!(oldest.kind.as_deref(), Some("cli"));
    }

    #[test]
    fn a_thread_keeps_its_status_across_loads_without_its_rollout_being_read_again() {
        let home = TempDir::new("agents-codex-unchanged");
        let h = home.root();
        let finished = rollout(h, "finished", "task_complete", MINUTE_MS);
        let conn = create_db(h);
        insert(&conn, recent("job", &finished, h, MINUTE_MS));
        let boundary = home.boundary(&ScanOptions::default());
        let mut rollouts = Rollouts::default();
        let first = load(h, &boundary, &mut rollouts).unwrap();
        assert_eq!(first[0].status, AgentStatus::Idle);

        // Opening the rollout now fails, as a read that happened would show.
        home.restrict(&finished, 0o000);

        let second = load(h, &boundary, &mut rollouts).unwrap();
        assert_eq!(second[0].status, AgentStatus::Idle);
    }

    /// The source of a thread Codex spawned as a sub-agent of `parent`.
    fn spawned_by(parent: &str) -> String {
        serde_json::json!({"subagent": {"thread_spawn": {
            "parent_thread_id": parent, "depth": 1, "agent_nickname": "Boyle", "agent_role": "explorer",
        }}})
        .to_string()
    }

    #[test]
    fn a_sub_agent_at_work_makes_its_resting_parent_busy_and_is_no_session_of_its_own() {
        let home = TempDir::new("agents-codex-sub-agent");
        let h = home.root();
        let resting = rollout(h, "parent", "task_complete", 5 * MINUTE_MS);
        let working = rollout(h, "explorer", "task_started", 10 * SECOND_MS);
        let reviewing = rollout(h, "guardian", "task_started", 5 * SECOND_MS);
        let conn = create_db(h);
        insert(&conn, recent("parent", &resting, h, 5 * MINUTE_MS));
        let explorer = spawned_by("parent");
        let sub_agent = recent("explorer", &working, h, 10 * SECOND_MS);
        insert(
            &conn,
            Row {
                source: &explorer,
                ..sub_agent
            },
        );
        let guardian = recent("guardian", &reviewing, h, 5 * SECOND_MS);
        let source = r#"{"subagent":{"other":"guardian"}}"#;
        insert(&conn, Row { source, ..guardian });

        let sessions = load_home(&home).unwrap();

        assert_eq!(ids(&sessions), ["parent"]);
        let parent = &sessions[0];
        assert_eq!(parent.status, AgentStatus::Busy);
        assert_eq!(
            parent.subagents_at_work.len(),
            1,
            "the guardian counts for nothing"
        );
        let age = idle_for(parent);
        assert!(
            age >= Duration::from_secs(10) && age < Duration::from_secs(20),
            "the sub-agent's write: {age:?}"
        );
    }

    #[test]
    fn a_sub_agent_at_work_is_named_by_codex_works_in_its_threads_folder_and_quiets_as_a_thread() {
        let home = TempDir::new("agents-codex-sub-agent-described");
        let h = home.root();
        let web = home.dir("web");
        let resting = rollout(h, "parent", "task_complete", 30 * MINUTE_MS);
        let working = rollout(h, "explorer", "task_started", 10 * SECOND_MS);
        let silent = rollout(h, "reviewer", "task_started", 5 * MINUTE_MS);
        let conn = create_db(h);
        insert(&conn, recent("parent", &resting, h, 30 * MINUTE_MS));
        let source = spawned_by("parent");
        let explorer = recent("explorer", &working, &web, 10 * SECOND_MS);
        let reviewer = recent("reviewer", &silent, h, 5 * MINUTE_MS);
        insert(
            &conn,
            Row {
                source: &source,
                ..explorer
            },
        );
        insert(
            &conn,
            Row {
                source: &source,
                ..reviewer
            },
        );

        let sessions = load_home(&home).unwrap();

        let [explorer, reviewer] = sessions[0].subagents_at_work.as_slice() else {
            panic!(
                "two sub-agents at work: {:?}",
                sessions[0].subagents_at_work
            );
        };
        assert_eq!(explorer.id, "explorer");
        assert_eq!(explorer.cwd, fs::canonicalize(&web).unwrap());
        assert_eq!(explorer.name.as_deref(), Some("Boyle"));
        assert_eq!(explorer.description, None);
        assert_eq!((explorer.status, explorer.since), (AgentStatus::Busy, None));
        assert_eq!(reviewer.status, AgentStatus::Quiet);
        let silent_for = SystemTime::now()
            .duration_since(reviewer.since.expect("a quiet sub-agent dates its silence"))
            .unwrap();
        assert!(
            silent_for >= 5 * Duration::from_secs(60) && silent_for < Duration::from_secs(310),
            "{silent_for:?}"
        );
        assert_eq!(reviewer.last_write, reviewer.since);
    }

    #[test]
    fn a_parent_unwritten_for_hours_shows_while_the_sub_agent_of_its_sub_agent_works() {
        let home = TempDir::new("agents-codex-sub-agent-chain");
        let h = home.root();
        let resting = rollout(h, "parent", "task_complete", 3 * RECENT_MS);
        let waiting = rollout(h, "lead", "task_started", 2 * RECENT_MS);
        let working = rollout(h, "worker", "task_started", 10 * SECOND_MS);
        let conn = create_db(h);
        insert(&conn, recent("parent", &resting, h, 3 * RECENT_MS));
        let lead = spawned_by("parent");
        insert(
            &conn,
            Row {
                source: &lead,
                ..recent("lead", &waiting, h, 2 * RECENT_MS)
            },
        );
        let worker = spawned_by("lead");
        insert(
            &conn,
            Row {
                source: &worker,
                ..recent("worker", &working, h, 10 * SECOND_MS)
            },
        );

        let sessions = load_home(&home).unwrap();

        assert_eq!(ids(&sessions), ["parent"]);
        assert_eq!(sessions[0].status, AgentStatus::Busy);
        assert_eq!(
            sessions[0].subagents_at_work.len(),
            1,
            "the lead wrote too long ago to count"
        );
    }

    #[test]
    fn a_thread_and_a_sub_agent_whose_turns_outgrow_a_chunk_stay_at_work_across_loads() {
        let home = TempDir::new("agents-codex-long-turns");
        let h = home.root();
        let sessions = h.join(".codex").join("sessions");
        let written_just_now = |name: &str, lines: &[String]| {
            let path = sessions.join(format!("{name}.jsonl"));
            write_test_rollout(&path, lines);
            set_mtime(&path, SystemTime::now() - Duration::from_secs(5));
            path
        };
        let mut long_turn = vec![test_event("task_started")];
        long_turn.extend(test_outputs_past_a_chunk());
        let working = written_just_now("working", &long_turn);
        let explorer = written_just_now("explorer", &long_turn);
        let resting = rollout(h, "resting", "task_complete", 5 * MINUTE_MS);
        let conn = create_db(h);
        insert(&conn, recent("working", &working, h, MINUTE_MS));
        insert(&conn, recent("resting", &resting, h, 5 * MINUTE_MS));
        let source = spawned_by("resting");
        insert(
            &conn,
            Row {
                source: &source,
                ..recent("explorer", &explorer, h, 10 * SECOND_MS)
            },
        );
        let boundary = home.boundary(&ScanOptions::default());
        let mut rollouts = Rollouts::default();
        let at_work = |sessions: &[AgentSession]| -> Vec<(AgentStatus, usize)> {
            let sessions = sessions.iter();
            sessions
                .map(|s| (s.status, s.subagents_at_work.len()))
                .collect()
        };

        let first = load(h, &boundary, &mut rollouts).unwrap();
        assert_eq!(ids(&first), ["working", "resting"]);
        let busy = AgentStatus::Busy;
        assert_eq!(
            at_work(&first),
            [(busy, 0), (busy, 1)],
            "when the rollouts are first read"
        );

        long_turn.extend(test_outputs_past_a_chunk());
        written_just_now("working", &long_turn);
        written_just_now("explorer", &long_turn);

        let second = load(h, &boundary, &mut rollouts).unwrap();
        assert_eq!(
            at_work(&second),
            [(busy, 0), (busy, 1)],
            "when only what the turns appended is read"
        );
    }

    #[test]
    fn a_sub_agent_done_or_silent_mid_turn_too_long_lends_its_parent_only_its_writes() {
        let home = TempDir::new("agents-codex-sub-agent-resting");
        let h = home.root();
        let resting = rollout(h, "parent", "task_complete", 30 * MINUTE_MS);
        let done = rollout(h, "done", "task_complete", MINUTE_MS);
        let silent = rollout(h, "silent", "task_started", 20 * MINUTE_MS);
        let conn = create_db(h);
        insert(&conn, recent("parent", &resting, h, 30 * MINUTE_MS));
        let source = spawned_by("parent");
        insert(
            &conn,
            Row {
                source: &source,
                ..recent("done", &done, h, MINUTE_MS)
            },
        );
        insert(
            &conn,
            Row {
                source: &source,
                ..recent("silent", &silent, h, 20 * MINUTE_MS)
            },
        );

        let sessions = load_home(&home).unwrap();

        assert_eq!(ids(&sessions), ["parent"]);
        let parent = &sessions[0];
        assert_eq!(parent.status, AgentStatus::Idle);
        assert_eq!(parent.subagents_at_work, []);
        let age = idle_for(parent);
        assert!(
            age >= Duration::from_secs(60) && age < Duration::from_secs(70),
            "the newest sub-agent write: {age:?}"
        );
    }

    #[test]
    fn the_queries_visit_only_the_recent_rows_or_the_one_asked_for_whatever_indexes_codex_keeps() {
        let home = TempDir::new("agents-codex-plan");
        let conn = create_db(home.root());
        conn.execute_batch(
            "CREATE INDEX idx_threads_archived ON threads(archived);
             CREATE INDEX idx_threads_source ON threads(source);
             CREATE INDEX idx_threads_updated_at_ms ON threads(updated_at_ms DESC, id DESC);",
        )
        .unwrap();
        let plan = |query: &str, parameter: &dyn rusqlite::ToSql| -> Vec<String> {
            let mut plan = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
                .unwrap();
            plan.query_map([parameter], |row| row.get("detail"))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };

        assert_eq!(
            plan(RECENT_THREADS, &0_i64),
            ["SEARCH threads USING INDEX idx_threads_updated_at_ms (updated_at_ms>?)"]
        );
        assert_eq!(
            plan(THREAD_BY_ID, &"a-thread"),
            ["SEARCH threads USING INDEX sqlite_autoindex_threads_1 (id=?)"]
        );
    }

    #[test]
    fn database_in_a_read_only_folder_is_read_without_its_log() {
        let home = TempDir::new("agents-codex-readonly");
        let rollout = rollout(home.root(), "started", "task_started", MINUTE_MS);
        let conn = create_db(home.root());
        insert(&conn, recent("t", &rollout, home.root(), MINUTE_MS));
        // Closing the only connection copies the log into the main file and deletes the log.
        drop(conn);
        let codex = home.root().join(".codex");
        home.restrict(&codex, 0o555);
        let db = codex.join("state_5.sqlite");
        let plain = query_threads(&db, "mode=ro", 0).err();
        assert!(plain.as_ref().is_some_and(cannot_use_log), "{plain:?}");

        let sessions = load_home(&home).unwrap();

        assert_eq!(ids(&sessions), ["t"]);
    }

    #[test]
    fn database_that_cannot_be_opened_is_a_problem_not_an_empty_list() {
        let home = TempDir::new("agents-codex-sealed");
        drop(create_db(home.root()));
        let db = home.root().join(".codex").join("state_5.sqlite");
        home.restrict(&db, 0o000);

        assert_eq!(
            load_home(&home).unwrap_err(),
            SourceProblem::new(
                Source::CodexDatabase,
                db.clone(),
                io::ErrorKind::PermissionDenied
            )
        );
    }

    #[test]
    fn database_whose_schema_changed_is_a_problem_not_an_empty_list() {
        let home = TempDir::new("agents-codex-schema");
        let dir = home.root().join(".codex");
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state_5.sqlite");
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY);")
            .unwrap();

        assert_eq!(
            load_home(&home).unwrap_err(),
            SourceProblem::new(Source::CodexDatabase, db, io::ErrorKind::InvalidData)
        );
    }

    #[test]
    fn uri_path_escapes_sqlite_delimiters() {
        assert_eq!(uri_path(Path::new("/a?b#c%d")), "/a%3fb%23c%25d");
    }
}
