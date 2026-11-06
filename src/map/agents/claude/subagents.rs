//! Claude Code writes a transcript for each subagent a session starts, at
//! `<home>/.claude/projects/<folder>/<session id>/subagents/agent-<agent id>.jsonl`, and those of a
//! workflow's subagents one folder down, in `subagents/workflows/<workflow id>/`. What a
//! transcript says is read from its end (see `transcript`), and only from the transcripts of
//! sessions still running. A subagent at work is named by the meta file beside its transcript
//! (see `meta`).

mod meta;
mod transcript;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::io;
use std::mem;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use self::meta::{Meta, meta_path, read_meta};
use self::transcript::{Tail, read_tail};
use crate::map::AgentStatus;
use crate::map::agents::resolve_cwd;
use crate::map::agents::session::{Subagent, SubagentWork};
use crate::map::scan::Boundary;
use crate::model::{ReadFailure, note_failure};

/// The longest folder name Claude Code derives from a working directory. It cuts a longer one to
/// this length and appends `-` and a hash of the whole path.
const MAX_FOLDER_NAME: usize = 200;

/// What each subagent transcript said when last read, so that a load reads again only the
/// transcripts written since, and what the meta files of the subagents at work said. It holds only
/// what the last load asked about, of sessions still running, so it does not grow with the
/// thousands of transcripts a machine keeps.
#[derive(Debug, Default)]
pub struct Transcripts {
    read: HashMap<PathBuf, Reading>,
    /// By the meta file's path. Claude Code writes a subagent's name and task there as it starts
    /// the subagent, so one read while the subagent works is not read again.
    metas: HashMap<PathBuf, Meta>,
}

/// A transcript's state when it was read, and what it said then.
#[derive(Debug, Clone)]
struct Reading {
    len: u64,
    modified: Option<SystemTime>,
    tail: Tail,
}

/// One load's reading of the subagent transcripts of the live sessions.
pub(super) struct Reader<'a> {
    /// Claude Code's `~/.claude/projects`.
    projects: PathBuf,
    /// Where a subagent's working folder may be resolved.
    boundary: &'a Boundary,
    now: SystemTime,
    /// The last load's readings.
    earlier: Transcripts,
    /// This load's, for the next.
    transcripts: &'a mut Transcripts,
    /// The meta files this load could not read.
    unread_metas: Vec<ReadFailure>,
}

impl<'a> Reader<'a> {
    /// A reading of the transcripts under `home`, going by what `transcripts` says the last load
    /// read, and leaving there what this one reads. Working folders are resolved only where
    /// `boundary` allows.
    pub(super) fn new(
        home: &Path,
        boundary: &'a Boundary,
        transcripts: &'a mut Transcripts,
    ) -> Self {
        Reader {
            projects: home.join(".claude").join("projects"),
            boundary,
            now: SystemTime::now(),
            earlier: mem::take(transcripts),
            transcripts,
            unread_metas: Vec::new(),
        }
    }

    /// The work of the subagents of the session `session_id`, which Claude Code started in `cwd`
    /// (as it recorded it) in a process that started at `started`. A folder or transcript that
    /// cannot be read goes in `failures`, and the rest still count.
    pub(super) fn session_work(
        &mut self,
        cwd: &str,
        session_id: &str,
        started: Option<SystemTime>,
        failures: &mut Vec<ReadFailure>,
    ) -> SubagentWork {
        let mut work = SubagentWork::default();
        let folder = match session_folder(&self.projects, cwd, session_id) {
            Ok(Some(folder)) => folder,
            Ok(None) => return work,
            Err(err) => {
                note_failure(&self.projects, &err, failures);
                return work;
            }
        };
        // A session's subagents run in its process and end with it, so one last written before
        // this process started is not at work. A transcript not modified since then is not read.
        let before_start =
            |at: Option<SystemTime>| started.zip(at).is_some_and(|(started, at)| at < started);
        for (path, listed) in transcripts_in(&folder.join("subagents"), failures) {
            if before_start(listed.modified().ok()) {
                continue;
            }
            match self.read(&path, &listed) {
                Ok(tail) if before_start(tail.written) => {}
                Ok(tail) => {
                    let (mid_turn, written, now) = (tail.mid_turn, tail.written, self.now);
                    work.count(mid_turn, written, now, || self.subagent(&path, tail, cwd));
                }
                Err(err) => note_failure(&path, &err, failures),
            }
        }
        work
    }

    /// The meta files this load could not read, which cost their subagents only their names.
    pub(super) fn unread_metas(self) -> Vec<ReadFailure> {
        self.unread_metas
    }

    /// What the transcript at `path` says, as `listed` measures it. One with the length and
    /// modification time it had at the last load still says what it said then, because Claude
    /// Code only appends to it, and is not read again. Of one that has grown, the end is read
    /// again, and a folder the last load read there holds until an entry names another.
    fn read(&mut self, path: &Path, listed: &Metadata) -> io::Result<Tail> {
        let (len, modified) = (listed.len(), listed.modified().ok());
        let tail = match self.earlier.read.get(path) {
            Some(known) if known.len == len && known.modified == modified => known.tail.clone(),
            Some(known) if known.len <= len => read_tail(path, len)?.after(&known.tail),
            // Not read before, or shrunk, so no longer the file that was.
            _ => read_tail(path, len)?,
        };
        let reading = Reading {
            len,
            modified,
            tail: tail.clone(),
        };
        self.transcripts.read.insert(path.to_path_buf(), reading);
        Ok(tail)
    }

    /// The subagent at work whose transcript at `transcript` ends as `tail` says, of a session
    /// Claude Code started in `session_cwd`. Until its transcript names a folder, it is taken to
    /// work where Claude Code starts a subagent without a worktree of its own: in its session's.
    fn subagent(&mut self, transcript: &Path, tail: Tail, session_cwd: &str) -> Subagent {
        let meta = self.meta(transcript);
        let cwd = tail.cwd.as_deref().unwrap_or(session_cwd);
        Subagent {
            id: agent_id(transcript),
            cwd: resolve_cwd(cwd, self.boundary),
            name: meta.name,
            description: meta.description,
            // Its transcript says only that its turn goes on.
            status: AgentStatus::Busy,
            since: None,
            last_write: tail.written,
        }
    }

    /// What the meta file beside `transcript` says, read only when the last load did not read it.
    /// One not there yet, or that could not be read, names nothing, and the next load reads it
    /// again.
    fn meta(&mut self, transcript: &Path) -> Meta {
        let path = meta_path(transcript);
        let meta = match self.earlier.metas.remove(&path) {
            Some(meta) => meta,
            None => match read_meta(&path) {
                Ok(Some(meta)) => meta,
                Ok(None) => return Meta::default(),
                Err(err) => {
                    note_failure(&path, &err, &mut self.unread_metas);
                    return Meta::default();
                }
            },
        };
        self.transcripts.metas.insert(path, meta.clone());
        meta
    }
}

/// Claude Code's id for the subagent whose transcript is `transcript`, named `agent-<id>.jsonl`.
fn agent_id(transcript: &Path) -> String {
    let stem = transcript.file_stem().unwrap_or_default().to_string_lossy();
    stem.strip_prefix("agent-").unwrap_or(&stem).to_string()
}

/// Whether `path`, relative to Claude Code's `~/.claude/projects`, is where a subagent's transcript
/// lies: `<folder>/<session id>/subagents/`, or a workflow's folder inside, then `<name>.jsonl`.
pub fn is_subagent_transcript(path: &Path) -> bool {
    path.components().nth(2) == Some(Component::Normal(OsStr::new("subagents")))
        && path.extension() == Some(OsStr::new("jsonl"))
}

/// Claude Code's folder for the files of the session `session_id` started in `cwd`, under
/// `projects`. `None` when the folder of a long name is not there.
fn session_folder(projects: &Path, cwd: &str, session_id: &str) -> io::Result<Option<PathBuf>> {
    let name = folder_name(cwd);
    if name.len() <= MAX_FOLDER_NAME {
        return Ok(Some(projects.join(name).join(session_id)));
    }
    // Claude Code 2.1.310 looks such a folder up by the part of its name it kept rather than by
    // the hash, and so does this.
    let kept = format!("{}-", &name[..MAX_FOLDER_NAME]);
    let dir = match fs::read_dir(projects) {
        Ok(dir) => dir,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    for entry in dir {
        let folder = entry?.path();
        let named_so = folder
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(&kept));
        if named_so && folder.join(session_id).is_dir() {
            return Ok(Some(folder.join(session_id)));
        }
    }
    Ok(None)
}

/// The folder name Claude Code derives from a working directory: each UTF-16 unit that is not an
/// ASCII letter or digit becomes `-`, as its `replace(/[^a-zA-Z0-9]/g, "-")` does.
fn folder_name(cwd: &str) -> String {
    cwd.encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => char::from(byte),
            _ => '-',
        })
        .collect()
}

/// The transcripts in a session's `subagents` folder, its workflows' included, each with its
/// metadata. A folder that is not there holds none.
fn transcripts_in(subagents: &Path, failures: &mut Vec<ReadFailure>) -> Vec<(PathBuf, Metadata)> {
    let mut found = Vec::new();
    for (path, meta) in entries(subagents, failures) {
        if meta.is_dir() && path.file_name() == Some(OsStr::new("workflows")) {
            for (workflow, meta) in entries(&path, failures) {
                if meta.is_dir() {
                    let inside = entries(&workflow, failures);
                    found.extend(
                        inside
                            .into_iter()
                            .filter(|(path, meta)| is_transcript(path, meta)),
                    );
                }
            }
        } else if is_transcript(&path, &meta) {
            found.push((path, meta));
        }
    }
    found
}

/// Whether a folder entry is a transcript. Anything but a regular file is not Claude's, and reading
/// a FIFO would block the load. The metadata comes from the folder entry, which does not follow a
/// symlink.
fn is_transcript(path: &Path, meta: &Metadata) -> bool {
    meta.is_file() && path.extension() == Some(OsStr::new("jsonl"))
}

/// The entries of `folder` with their metadata.
fn entries(folder: &Path, failures: &mut Vec<ReadFailure>) -> Vec<(PathBuf, Metadata)> {
    let dir = match fs::read_dir(folder) {
        Ok(dir) => dir,
        Err(err) => {
            note_failure(folder, &err, failures);
            return Vec::new();
        }
    };
    let mut entries = Vec::new();
    for entry in dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                note_failure(folder, &err, failures);
                continue;
            }
        };
        let path = entry.path();
        match entry.metadata() {
            Ok(meta) => entries.push((path, meta)),
            Err(err) => note_failure(&path, &err, failures),
        }
    }
    entries
}

/// Writes the transcript of the subagent `agent` of the session `session_id` started in `cwd`,
/// under `home`: set to work in `works_in`, it sent a request to the model at `at` and awaits the
/// reply. Returns its path.
#[cfg(test)]
pub(crate) fn write_test_subagent_at_work(
    home: &Path,
    cwd: &str,
    session_id: &str,
    agent: &str,
    works_in: &str,
    at: SystemTime,
) -> PathBuf {
    let folder = home.join(".claude").join("projects").join(folder_name(cwd));
    let subagents = folder.join(session_id).join("subagents");
    let path = subagents.join(format!("agent-{agent}.jsonl"));
    let lines = [
        transcript::test_prompt(at, works_in),
        transcript::test_request(at),
    ];
    transcript::write_test_transcript(&path, &lines);
    path
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::meta::write_test_meta;
    use super::transcript::{
        TEST_FOLDER, test_prompt as prompt, test_reply as reply, test_request as request,
        timestamp, write_test_transcript,
    };
    use super::*;
    use crate::map::ScanOptions;
    use crate::map::test_support::{TempDir, set_mtime};

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn a_folder_name_is_the_working_directory_with_a_dash_for_each_other_character() {
        let cases = [
            (
                "/Users/daniel/ak/ankurah/.claude/worktrees/optimistic-ptolemy-8868fb",
                "-Users-daniel-ak-ankurah--claude-worktrees-optimistic-ptolemy-8868fb",
            ),
            (
                "/Users/daniel/Library/Application Support/Claude",
                "-Users-daniel-Library-Application-Support-Claude",
            ),
            ("/Users/me/café", "-Users-me-caf-"),
            // JavaScript counts a character outside the BMP as two units.
            ("/Users/me/🚀", "-Users-me---"),
        ];
        for (cwd, name) in cases {
            assert_eq!(folder_name(cwd), name, "{cwd}");
        }
    }

    #[test]
    fn a_session_whose_folder_name_was_cut_short_is_found_by_the_part_kept() {
        let home = TempDir::new("subagents-long-folder");
        let projects = home.dir("projects");
        let cwd = format!("/private/tmp/{}", "deep/".repeat(50));
        let kept = &folder_name(&cwd)[..MAX_FOLDER_NAME];
        fs::create_dir_all(
            projects
                .join(format!("{kept}-u0ko8l"))
                .join("other-session"),
        )
        .unwrap();
        let session = projects.join(format!("{kept}-x9k2pq")).join("session");
        fs::create_dir_all(&session).unwrap();

        assert_eq!(
            session_folder(&projects, &cwd, "session").unwrap(),
            Some(session)
        );
        assert_eq!(session_folder(&projects, &cwd, "gone").unwrap(), None);
        let short = session_folder(&projects, "/code/app", "session").unwrap();
        assert_eq!(short, Some(projects.join("-code-app").join("session")));
    }

    #[test]
    fn subagent_transcripts_lie_in_a_sessions_subagents_folder_or_a_workflows() {
        for path in [
            "-code-app/0b1d/subagents/agent-a1.jsonl",
            "-code-app/0b1d/subagents/workflows/wf_9c/agent-a2.jsonl",
        ] {
            assert!(is_subagent_transcript(Path::new(path)), "{path}");
        }
        for path in [
            "-code-app/0b1d.jsonl",
            "-code-app/0b1d/subagents/agent-a1.meta.json",
            "-code-app/0b1d/subagents",
            "-code-app/memory/subagents.jsonl",
        ] {
            assert!(!is_subagent_transcript(Path::new(path)), "{path}");
        }
    }

    /// A load as of `now` of the subagents of the session `0b1d` started in `/code/app` at
    /// `started`, under `home`.
    fn session_work(
        home: &TempDir,
        started: SystemTime,
        now: SystemTime,
        transcripts: &mut Transcripts,
    ) -> (SubagentWork, Vec<ReadFailure>) {
        let (work, failures, _) = described_work(home, started, now, transcripts);
        (work, failures)
    }

    /// That load, with the meta files it could not read.
    fn described_work(
        home: &TempDir,
        started: SystemTime,
        now: SystemTime,
        transcripts: &mut Transcripts,
    ) -> (SubagentWork, Vec<ReadFailure>, Vec<ReadFailure>) {
        let boundary = home.boundary(&ScanOptions::default());
        let mut reader = Reader::new(home.root(), &boundary, transcripts);
        reader.now = now;
        let mut failures = Vec::new();
        let work = reader.session_work(TEST_FOLDER, "0b1d", Some(started), &mut failures);
        (work, failures, reader.unread_metas())
    }

    /// Where the transcript of the subagent `agent` of that session lies.
    fn transcript_path(home: &TempDir, agent: &str) -> PathBuf {
        let subagents = home.path(".claude/projects/-code-app/0b1d/subagents");
        subagents.join(format!("agent-{agent}.jsonl"))
    }

    #[test]
    fn a_session_counts_its_subagents_mid_turn_and_their_newest_write() {
        let home = TempDir::new("subagents-count");
        let now = SystemTime::now();
        let started = now - 60 * MINUTE;
        write_test_transcript(
            &transcript_path(&home, "working"),
            &[request(now - 2 * MINUTE)],
        );
        let done = [
            request(now - 3 * MINUTE),
            reply(now - MINUTE, Some("end_turn"), "text"),
        ];
        write_test_transcript(&transcript_path(&home, "done"), &done);
        let silent = [request(now - 20 * MINUTE)];
        write_test_transcript(&transcript_path(&home, "silent"), &silent);
        let workflow = transcript_path(&home, "unused")
            .with_file_name("workflows")
            .join("wf_9c")
            .join("agent-step.jsonl");
        let calling = reply(now - 4 * MINUTE, Some("tool_use"), "tool_use");
        write_test_transcript(&workflow, &[calling]);

        let (work, failures) = session_work(&home, started, now, &mut Transcripts::default());

        let subagents = work.at_work.iter();
        let mut ids: Vec<&str> = subagents.map(|subagent| subagent.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            ["step", "working"],
            "the working one and the workflow's"
        );
        let newest = work.last_write.map(timestamp);
        assert_eq!(newest, Some(timestamp(now - MINUTE)), "the one done");
        assert_eq!(failures, []);
    }

    #[test]
    fn the_subagents_of_a_sessions_earlier_process_count_for_nothing() {
        let home = TempDir::new("subagents-earlier-process");
        let now = SystemTime::now();
        let started = now - 5 * MINUTE;
        // Cut off mid-turn when the session's last process ended, and rewritten since.
        let cut_off = transcript_path(&home, "cut-off");
        write_test_transcript(&cut_off, &[request(started - MINUTE)]);
        // Unchanged since then, so not even read.
        let untouched = transcript_path(&home, "untouched");
        write_test_transcript(&untouched, &[request(started - MINUTE)]);
        set_mtime(&untouched, started - MINUTE);
        home.restrict(&untouched, 0o000);

        let (work, failures) = session_work(&home, started, now, &mut Transcripts::default());

        assert_eq!(work, SubagentWork::default());
        assert_eq!(failures, []);
    }

    #[test]
    fn a_transcript_unchanged_since_the_last_load_is_not_read_again() {
        let home = TempDir::new("subagents-unchanged");
        let now = SystemTime::now();
        let path = transcript_path(&home, "working");
        write_test_transcript(&path, &[request(now)]);
        let mut transcripts = Transcripts::default();
        let (first, _) = session_work(&home, now - MINUTE, now, &mut transcripts);
        assert_eq!(first.at_work.len(), 1);

        // Opening the transcript now fails, and its state can still be looked at.
        home.restrict(&path, 0o000);

        let (second, failures) = session_work(&home, now - MINUTE, now, &mut transcripts);
        assert_eq!((second, failures), (first, vec![]));
        let (_, failures) = session_work(&home, now - MINUTE, now, &mut Transcripts::default());
        assert_eq!(
            failures,
            [(path, io::ErrorKind::PermissionDenied)],
            "without the earlier reading it would have to be opened"
        );
    }

    #[test]
    fn a_transcript_written_since_the_last_load_is_read_again_and_one_gone_is_forgotten() {
        let home = TempDir::new("subagents-written-since");
        let now = SystemTime::now();
        let started = now - 5 * MINUTE;
        let path = transcript_path(&home, "finishing");
        write_test_transcript(&path, &[request(now - MINUTE)]);
        let mut transcripts = Transcripts::default();
        let at_work = |transcripts: &mut Transcripts| {
            session_work(&home, started, now, transcripts)
                .0
                .at_work
                .len()
        };
        assert_eq!(at_work(&mut transcripts), 1);

        let finished = [request(now - MINUTE), reply(now, Some("end_turn"), "text")];
        write_test_transcript(&path, &finished);
        assert_eq!(at_work(&mut transcripts), 0);
        assert_eq!(transcripts.read.keys().collect::<Vec<_>>(), [&path]);

        fs::remove_file(&path).unwrap();
        session_work(&home, started, now, &mut transcripts);
        assert!(
            transcripts.read.is_empty(),
            "a load keeps only what it read"
        );
    }

    #[test]
    fn a_subagents_folder_that_cannot_be_read_is_a_failure_and_one_not_there_is_none() {
        let home = TempDir::new("subagents-sealed");
        let now = SystemTime::now();
        let (work, failures) = session_work(&home, now, now, &mut Transcripts::default());
        assert_eq!((work, failures), (SubagentWork::default(), vec![]));

        let path = transcript_path(&home, "working");
        write_test_transcript(&path, &[request(now)]);
        let folder = path.parent().unwrap().to_path_buf();
        home.restrict(&folder, 0o000);

        let (work, failures) = session_work(&home, now, now, &mut Transcripts::default());
        assert_eq!(work, SubagentWork::default());
        assert_eq!(failures, [(folder, io::ErrorKind::PermissionDenied)]);
    }

    /// Where the subagents of the session `0b1d` work when given a worktree of their own.
    const WORKTREE: &str = "/code/app/.claude/worktrees/agent-a1";

    #[test]
    fn a_subagent_at_work_is_named_by_its_meta_file_and_works_where_its_transcript_says() {
        let home = TempDir::new("subagents-described");
        let now = SystemTime::now();
        let path = transcript_path(&home, "a1");
        write_test_transcript(&path, &[prompt(now - MINUTE, WORKTREE), request(now)]);
        write_test_meta(&path, "dashboard-fixes", "Fix dashboard review findings");

        let (work, failures, unread_metas) =
            described_work(&home, now - 60 * MINUTE, now, &mut Transcripts::default());

        let [subagent] = work.at_work.as_slice() else {
            panic!("one subagent at work: {:?}", work.at_work);
        };
        assert_eq!(subagent.id, "a1");
        assert_eq!(subagent.cwd, Path::new(WORKTREE));
        assert_eq!(subagent.name.as_deref(), Some("dashboard-fixes"));
        let task = subagent.description.as_deref();
        assert_eq!(task, Some("Fix dashboard review findings"));
        assert_eq!(subagent.status, AgentStatus::Busy);
        assert_eq!(subagent.since, None);
        assert_eq!(subagent.last_write.map(timestamp), Some(timestamp(now)));
        assert_eq!((failures, unread_metas), (vec![], vec![]));
    }

    #[test]
    fn a_subagent_with_no_meta_file_or_folder_named_works_unnamed_in_its_sessions_folder() {
        let home = TempDir::new("subagents-undescribed");
        let now = SystemTime::now();
        write_test_transcript(&transcript_path(&home, "a1"), &[request(now)]);

        let (work, _, unread_metas) =
            described_work(&home, now - MINUTE, now, &mut Transcripts::default());

        let [subagent] = work.at_work.as_slice() else {
            panic!("one subagent at work: {:?}", work.at_work);
        };
        assert_eq!(subagent.cwd, Path::new(TEST_FOLDER), "its session's folder");
        assert_eq!((&subagent.name, &subagent.description), (&None, &None));
        assert_eq!(unread_metas, [], "a meta file not there is no failure");
    }

    #[test]
    fn the_folder_a_subagent_works_in_holds_across_loads_while_its_transcripts_end_names_none() {
        let home = TempDir::new("subagents-folder-held");
        let now = SystemTime::now();
        let path = transcript_path(&home, "a1");
        let mut lines = vec![prompt(now - MINUTE, WORKTREE), request(now - MINUTE)];
        write_test_transcript(&path, &lines);
        let folder = |transcripts: &mut Transcripts| {
            let (work, ..) = described_work(&home, now - 60 * MINUTE, now, transcripts);
            work.at_work[0].cwd.clone()
        };
        let mut transcripts = Transcripts::default();
        assert_eq!(folder(&mut transcripts), Path::new(WORKTREE));

        // The prompt now lies further back than the end read.
        lines.extend([transcript::test_context_past_the_end_read(), request(now)]);
        write_test_transcript(&path, &lines);

        assert_eq!(folder(&mut transcripts), Path::new(WORKTREE));
        assert_eq!(
            folder(&mut Transcripts::default()),
            Path::new(TEST_FOLDER),
            "without the earlier load's reading, only its session's folder is known"
        );
    }

    #[test]
    fn a_meta_file_is_read_once_while_its_subagent_works_and_one_unreadable_costs_only_names() {
        let home = TempDir::new("subagents-meta-once");
        let now = SystemTime::now();
        let path = transcript_path(&home, "a1");
        let mut lines = vec![prompt(now - MINUTE, WORKTREE), request(now - MINUTE)];
        write_test_transcript(&path, &lines);
        write_test_meta(&path, "dashboard-fixes", "Fix dashboard review findings");
        let mut transcripts = Transcripts::default();
        let named = |transcripts: &mut Transcripts| {
            let (work, _, unread_metas) =
                described_work(&home, now - 60 * MINUTE, now, transcripts);
            (work.at_work[0].name.clone(), unread_metas)
        };
        let dashboard_fixes = Some("dashboard-fixes".to_string());
        assert_eq!(named(&mut transcripts), (dashboard_fixes.clone(), vec![]));

        // Opening the meta file now fails, as a read that happened would show.
        let meta = meta_path(&path);
        home.restrict(&meta, 0o000);
        lines.push(reply(now, Some("tool_use"), "tool_use"));
        write_test_transcript(&path, &lines);

        assert_eq!(named(&mut transcripts), (dashboard_fixes, vec![]));
        assert_eq!(
            named(&mut Transcripts::default()),
            (None, vec![(meta, io::ErrorKind::PermissionDenied)]),
            "without the earlier load's reading it is read, fails, and the subagent still counts"
        );
    }

    #[test]
    fn a_transcript_that_is_not_a_regular_file_is_passed_over() {
        let home = TempDir::new("subagents-fifo");
        let now = SystemTime::now();
        let fifo = transcript_path(&home, "fifo");
        fs::create_dir_all(fifo.parent().unwrap()).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());

        // On a thread, so a read that blocks on the FIFO fails the test instead of hanging it.
        let (sender, receiver) = std::sync::mpsc::channel();
        let root = home.root().to_path_buf();
        std::thread::spawn(move || {
            let mut transcripts = Transcripts::default();
            let boundary = Boundary::new(&[], &root, &ScanOptions::default());
            let mut reader = Reader::new(&root, &boundary, &mut transcripts);
            let mut failures = Vec::new();
            let work = reader.session_work(TEST_FOLDER, "0b1d", Some(now), &mut failures);
            let _ = sender.send((work, failures));
        });
        let read = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the read returns despite the FIFO");

        assert_eq!(read, (SubagentWork::default(), vec![]));
    }
}
