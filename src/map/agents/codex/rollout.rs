//! A rollout is the append-only event log of one Codex thread, one JSON record per line. Only its
//! turn events are read: the newest is searched for back from the end of the file, as far back as
//! it lies, and after that only what Codex appends is read.

use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

/// How much of a rollout the search for its newest turn event reads at a time, going back. That
/// event can lie far back: one turn in eight writes more than 256 KiB after the event that opens it,
/// and the longest wrote 62.5 MB (6,315 turns in 859 rollouts on Daniel's Mac, 2026-11-05).
const CHUNK_BYTES: u64 = 256 * 1024;

/// Rollout event types that open or close a turn.
const TURN_EVENTS: [(&str, Turn); 3] = [
    ("task_started", Turn::Open),
    ("task_complete", Turn::Closed),
    ("turn_aborted", Turn::Closed),
];

/// Whether the newest turn event opened a turn or closed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Turn {
    Open,
    Closed,
}

/// What a rollout says about its thread.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Rollout {
    /// The newest turn event, `None` when the rollout holds none.
    pub(super) last_turn: Option<Turn>,
    /// When Codex last appended to the rollout, if the file system records it.
    pub(super) written: Option<SystemTime>,
}

/// What each rollout said when last read, and how far it was read, so that a load reads only what
/// Codex has appended since. The engine loads every recent thread on every tick, and most of them,
/// a burst of finished `codex exec` jobs above all, never write again. It holds only the rollouts
/// of the last load's threads.
#[derive(Debug, Default)]
pub struct Rollouts {
    read: HashMap<PathBuf, Reading>,
}

/// A rollout's state when it was read, and what it said then.
#[derive(Debug, Clone, Copy)]
struct Reading {
    len: u64,
    written: Option<SystemTime>,
    /// The newest turn event in the lines before `read_to`.
    last_turn: Option<Turn>,
    /// Where the last whole line ends, and the next read begins. A line Codex was still writing
    /// begins here.
    read_to: u64,
}

impl Rollouts {
    /// What `rollout` says, or `None` when it is missing, unreadable or not a regular file. A
    /// rollout only grows, so what the last load read of it, in `earlier`, still holds: one with
    /// the length and modification time it had then is not read again, and of one that has grown,
    /// only what Codex appended is read. One that shrank is no longer the file that was read, and
    /// is read from the start. So only a first read searches far back: through a 19.9 MB rollout
    /// whose open turn began at its start it took 26 ms, and reading on through 32 KiB that Codex
    /// appended then took 0.04 ms (2026-11-05).
    pub(super) fn read(&mut self, rollout: &Path, earlier: &Rollouts) -> Option<Rollout> {
        let meta = fs::metadata(rollout).ok()?;
        let (len, written) = (meta.len(), meta.modified().ok());
        let reading = match earlier.read.get(rollout) {
            Some(&known) if known.len == len && known.written == written => known,
            Some(known) if known.len <= len => known.read_on(rollout, &meta).ok()?,
            _ => Reading::UNREAD.read_on(rollout, &meta).ok()?,
        };
        self.read.insert(rollout.to_path_buf(), reading);
        Some(Rollout {
            last_turn: reading.last_turn,
            written,
        })
    }
}

impl Reading {
    /// A rollout before its first read: no turn event is known, and the search for one may go back
    /// to the start of the file.
    const UNREAD: Reading = Reading {
        len: 0,
        written: None,
        last_turn: None,
        read_to: 0,
    };

    /// This reading carried on through what Codex has appended to `rollout` since, as `meta`
    /// measures the file. A turn event there replaces the one known; without one, the turn stands
    /// as it was, however much the turn writes.
    fn read_on(&self, rollout: &Path, meta: &Metadata) -> io::Result<Reading> {
        // Checked before opening, because opening a FIFO blocks until something writes to it.
        if !meta.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a regular file", rollout.display()),
            ));
        }
        let file = File::open(rollout)?;
        // Codex may append while this reads; reading only up to the length measured keeps the
        // reading true to `meta`, and the next read takes the rest.
        let (newest, read_to) = newest_turn_event(&file, self.read_to, meta.len())?;
        Ok(Reading {
            len: meta.len(),
            written: meta.modified().ok(),
            last_turn: newest.or(self.last_turn),
            read_to,
        })
    }
}

/// The newest turn event among the whole lines of `file` from `from`, where a line begins, to
/// `to`, and where the last of those lines ends. What follows it is a line Codex is still writing,
/// for the next read to take from its start. The search goes back a chunk at a time and stops at
/// the first turn event it meets.
fn newest_turn_event(file: &File, from: u64, to: u64) -> io::Result<(Option<Turn>, u64)> {
    let mut chunk = Vec::new();
    let mut long_line = Vec::new();
    let mut read_to = from;
    // Where the line the search has come to ends: the offset of its newline, once there is one.
    let mut line_end = None;
    let mut end = to;
    while end > from {
        let start = end.saturating_sub(CHUNK_BYTES).max(from);
        chunk.resize((end - start) as usize, 0);
        file.read_exact_at(&mut chunk, start)?;
        let newlines = chunk.iter().enumerate().rev();
        let newlines = newlines.filter(|&(_, &byte)| byte == b'\n');
        // Each line begins after a newline, except the first, which begins at `from`.
        let line_starts = newlines.map(|(at, _)| start + at as u64 + 1);
        for line_start in line_starts.chain((start == from).then_some(from)) {
            match line_end {
                None => read_to = line_start,
                Some(line_end) => {
                    let line = if line_end <= end {
                        &chunk[(line_start - start) as usize..(line_end - start) as usize]
                    } else {
                        // A line that runs on past this chunk is read whole.
                        long_line.resize((line_end - line_start) as usize, 0);
                        file.read_exact_at(&mut long_line, line_start)?;
                        long_line.as_slice()
                    };
                    if let Some(turn) = turn_event(line) {
                        return Ok((Some(turn), read_to));
                    }
                }
            }
            // The newline before this line ends the one before it; at the start of the file,
            // there is none.
            line_end = line_start.checked_sub(1);
        }
        end = start;
    }
    Ok((None, read_to))
}

/// One rollout line, reduced to the two fields that identify an event.
#[derive(Deserialize)]
struct RolloutRecord {
    #[serde(rename = "type")]
    kind: String,
    payload: RolloutPayload,
}

#[derive(Deserialize)]
struct RolloutPayload {
    #[serde(rename = "type")]
    kind: String,
}

/// The turn event a rollout line records, if it records one.
fn turn_event(line: &[u8]) -> Option<Turn> {
    // Most lines are large model and tool records; only lines naming a turn event are parsed.
    let line = std::str::from_utf8(line).ok()?;
    if !TURN_EVENTS.iter().any(|(name, _)| line.contains(name)) {
        return None;
    }
    let record: RolloutRecord = serde_json::from_str(line).ok()?;
    if record.kind != "event_msg" {
        return None;
    }
    TURN_EVENTS
        .iter()
        .find(|(name, _)| *name == record.payload.kind)
        .map(|&(_, turn)| turn)
}

/// A rollout line recording the event `kind`, shaped as Codex writes it.
#[cfg(test)]
pub(super) fn test_event(kind: &str) -> String {
    serde_json::json!({
        "timestamp": "2026-11-03T10:00:00.000Z",
        "type": "event_msg",
        "payload": {"type": kind},
    })
    .to_string()
}

/// A rollout line `len` bytes long that records a tool's output, which tells nothing of the turn.
#[cfg(test)]
fn test_output(len: usize) -> String {
    let line = |output: &str| {
        serde_json::json!({
            "timestamp": "2026-11-03T10:00:00.000Z",
            "type": "response_item",
            "payload": {"type": "function_call_output", "output": output},
        })
        .to_string()
    };
    line(&"x".repeat(len - line("").len()))
}

/// What a long turn writes after the event that opens it: more tool output than one chunk holds.
#[cfg(test)]
pub(super) fn test_outputs_past_a_chunk() -> Vec<String> {
    let line_len = 16 * 1024;
    vec![test_output(line_len); CHUNK_BYTES as usize / line_len + 1]
}

/// Writes a rollout file holding `lines`.
#[cfg(test)]
pub(super) fn write_test_rollout(path: &Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{TempDir, set_mtime};

    /// A model reply that names `kind` in its text, as replies about Codex itself do.
    fn reply_mentioning(kind: &str) -> String {
        serde_json::json!({
            "type": "response_item",
            "payload": {"type": "message", "text": format!("waiting for {kind}")},
        })
        .to_string()
    }

    /// A read with no earlier load to go by.
    fn read(rollout: &Path) -> Option<Rollout> {
        Rollouts::default().read(rollout, &Rollouts::default())
    }

    fn last_turn_of(home: &TempDir, lines: &[String]) -> Option<Turn> {
        let rollout = home.root().join("rollout.jsonl");
        write_test_rollout(&rollout, lines);
        read(&rollout).expect("a readable rollout").last_turn
    }

    #[test]
    fn newest_turn_event_wins() {
        let home = TempDir::new("agents-rollout-newest");
        let cases = [
            (vec!["task_started"], Turn::Open),
            (
                vec!["task_started", "task_complete", "token_count"],
                Turn::Closed,
            ),
            (vec!["task_started", "turn_aborted"], Turn::Closed),
            (vec!["task_complete", "task_started"], Turn::Open),
        ];
        for (kinds, expected) in cases {
            let lines: Vec<String> = kinds.iter().map(|kind| test_event(kind)).collect();
            assert_eq!(last_turn_of(&home, &lines), Some(expected), "{kinds:?}");
        }
    }

    #[test]
    fn event_name_inside_another_record_is_not_a_turn_event() {
        let home = TempDir::new("agents-rollout-mention");
        let lines = [reply_mentioning("task_complete")];
        assert_eq!(last_turn_of(&home, &lines), None);
    }

    #[test]
    fn missing_rollout_reads_as_none() {
        let home = TempDir::new("agents-rollout-missing");
        assert_eq!(read(&home.root().join("gone.jsonl")), None);
    }

    #[test]
    fn rollout_records_when_it_was_last_written() {
        let home = TempDir::new("agents-rollout-written");
        let rollout = home.root().join("rollout.jsonl");
        write_test_rollout(&rollout, &[test_event("task_started")]);
        let written = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        set_mtime(&rollout, written);

        assert_eq!(read(&rollout).unwrap().written, Some(written));
    }

    #[test]
    fn rollout_that_is_not_a_regular_file_is_not_opened() {
        let home = TempDir::new("agents-rollout-fifo");
        let fifo = home.root().join("rollout.jsonl");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());

        // On a thread, so an open that blocks on the FIFO fails the test instead of hanging it.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read(&fifo));
        });
        let read = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the read returns despite the FIFO");
        assert_eq!(read, None);
    }

    /// What a load finds, as the next load will read it.
    fn load_of(rollout: &Path, earlier: &Rollouts) -> (Option<Rollout>, Rollouts) {
        let mut this_load = Rollouts::default();
        (this_load.read(rollout, earlier), this_load)
    }

    #[test]
    fn a_rollout_unchanged_since_the_last_load_is_not_read_again() {
        let home = TempDir::new("agents-rollout-unchanged");
        let rollout = home.root().join("rollout.jsonl");
        write_test_rollout(&rollout, &[test_event("task_complete")]);
        let (first, earlier) = load_of(&rollout, &Rollouts::default());
        assert_eq!(first.unwrap().last_turn, Some(Turn::Closed));

        // Opening the file now fails, and its state can still be looked at.
        home.restrict(&rollout, 0o000);

        let (second, _) = load_of(&rollout, &earlier);
        assert_eq!(second.unwrap().last_turn, Some(Turn::Closed));
        assert_eq!(
            read(&rollout),
            None,
            "without the earlier reading it would have to be opened"
        );
    }

    #[test]
    fn a_rollout_written_since_the_last_load_is_read_again() {
        let home = TempDir::new("agents-rollout-written-since");
        let rollout = home.root().join("rollout.jsonl");
        write_test_rollout(&rollout, &[test_event("task_complete")]);
        let (_, earlier) = load_of(&rollout, &Rollouts::default());

        write_test_rollout(
            &rollout,
            &[test_event("task_complete"), test_event("task_started")],
        );

        let (second, _) = load_of(&rollout, &earlier);
        assert_eq!(second.unwrap().last_turn, Some(Turn::Open));
    }

    #[test]
    fn a_load_remembers_only_the_rollouts_it_asked_about() {
        let home = TempDir::new("agents-rollout-forgotten");
        let left_off = home.root().join("left-off.jsonl");
        write_test_rollout(&left_off, &[test_event("task_complete")]);
        let rollout = home.root().join("rollout.jsonl");
        write_test_rollout(&rollout, &[test_event("task_complete")]);
        let mut earlier = Rollouts::default();
        for path in [&left_off, &rollout] {
            earlier.read(path, &Rollouts::default());
        }
        assert_eq!(earlier.read.len(), 2);

        let (_, this_load) = load_of(&rollout, &earlier);

        assert_eq!(this_load.read.keys().collect::<Vec<_>>(), [&rollout]);
    }

    #[test]
    fn a_turn_open_since_many_chunks_back_reads_as_open() {
        let home = TempDir::new("agents-rollout-far-back");
        let mut lines = vec![test_event("task_complete"), test_event("task_started")];
        for _ in 0..3 {
            lines.extend(test_outputs_past_a_chunk());
        }

        assert_eq!(last_turn_of(&home, &lines), Some(Turn::Open));
    }

    #[test]
    fn a_turn_event_is_read_whole_wherever_a_chunk_begins_in_or_around_its_line() {
        let home = TempDir::new("agents-rollout-cut");
        let started = test_event("task_started");
        let opened_at = test_event("task_complete").len() + 1;
        let after_started = opened_at + started.len() + 1;
        // The search's first chunk begins this far into the started line: at its start, inside
        // it, on its newline, or on the line after it.
        for into_line in [0, 1, started.len() / 2, started.len(), started.len() + 1] {
            let chunk_starts = opened_at + into_line;
            let output_len = CHUNK_BYTES as usize - (after_started - chunk_starts) - 1;
            let lines = [
                test_event("task_complete"),
                started.clone(),
                test_output(output_len),
            ];

            assert_eq!(
                last_turn_of(&home, &lines),
                Some(Turn::Open),
                "a chunk beginning {into_line} bytes into it"
            );
        }
    }

    #[test]
    fn lines_longer_than_a_chunk_are_read_whole() {
        let home = TempDir::new("agents-rollout-long-lines");
        let message = "x".repeat(2 * CHUNK_BYTES as usize);
        let long_event = serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "task_complete", "last_agent_message": message},
        });
        let lines = [
            test_event("task_started"),
            long_event.to_string(),
            test_output(3 * CHUNK_BYTES as usize),
        ];

        assert_eq!(last_turn_of(&home, &lines), Some(Turn::Closed));
    }

    #[test]
    fn what_codex_appends_without_a_turn_event_leaves_the_turn_as_it_was() {
        let home = TempDir::new("agents-rollout-appended");
        let rollout = home.root().join("rollout.jsonl");
        let mut lines = vec![test_event("task_started")];
        write_test_rollout(&rollout, &lines);
        let (_, earlier) = load_of(&rollout, &Rollouts::default());

        lines.extend(test_outputs_past_a_chunk());
        write_test_rollout(&rollout, &lines);
        let (second, earlier) = load_of(&rollout, &earlier);
        assert_eq!(second.unwrap().last_turn, Some(Turn::Open));

        lines.push(test_event("task_complete"));
        write_test_rollout(&rollout, &lines);
        let (third, _) = load_of(&rollout, &earlier);
        assert_eq!(third.unwrap().last_turn, Some(Turn::Closed));
    }

    #[test]
    fn a_rollout_that_has_grown_is_read_on_from_where_the_last_load_stopped() {
        let home = TempDir::new("agents-rollout-read-on");
        let rollout = home.root().join("rollout.jsonl");
        let mut lines = vec![test_event("task_started"), test_output(1024)];
        write_test_rollout(&rollout, &lines);
        let (_, earlier) = load_of(&rollout, &Rollouts::default());

        // A closing event of the same length takes the opening one's place, where only a read
        // from the start would find it, and the turn writes on.
        lines[0] = test_event("turn_aborted");
        assert_eq!(lines[0].len(), test_event("task_started").len());
        lines.push(test_output(1024));
        write_test_rollout(&rollout, &lines);

        let (second, _) = load_of(&rollout, &earlier);
        assert_eq!(second.unwrap().last_turn, Some(Turn::Open));
        assert_eq!(
            read(&rollout).unwrap().last_turn,
            Some(Turn::Closed),
            "read from the start, it would find the closing event"
        );
    }

    #[test]
    fn a_line_still_being_written_is_read_once_it_is_whole() {
        let home = TempDir::new("agents-rollout-partial");
        let rollout = home.root().join("rollout.jsonl");
        let started = test_event("task_started");
        let complete = test_event("task_complete");
        let (begun, rest) = complete.split_at(complete.len() / 2);
        fs::write(&rollout, format!("{started}\n{begun}")).unwrap();
        let (first, earlier) = load_of(&rollout, &Rollouts::default());
        assert_eq!(first.unwrap().last_turn, Some(Turn::Open));

        fs::write(&rollout, format!("{started}\n{begun}{rest}\n")).unwrap();

        let (second, _) = load_of(&rollout, &earlier);
        assert_eq!(second.unwrap().last_turn, Some(Turn::Closed));
    }

    #[test]
    fn a_rollout_that_shrank_is_read_from_the_start() {
        let home = TempDir::new("agents-rollout-shrank");
        let rollout = home.root().join("rollout.jsonl");
        write_test_rollout(&rollout, &[test_event("task_started"), test_output(1024)]);
        let (_, earlier) = load_of(&rollout, &Rollouts::default());

        write_test_rollout(&rollout, &[test_event("task_complete")]);

        let (second, _) = load_of(&rollout, &earlier);
        assert_eq!(second.unwrap().last_turn, Some(Turn::Closed));
    }
}
