//! A rollout is the append-only event log of one Codex thread, one JSON record per line. Only its
//! turn events are read, and only from the end of the file.

use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

/// How much of a rollout's end is read. Rollouts grow to tens of megabytes, and the newest turn
/// event sits near the end unless one turn has written more than this since it started.
const TAIL_BYTES: u64 = 256 * 1024;

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
    /// The newest turn event in the tail, `None` when the tail holds none.
    pub(super) last_turn: Option<Turn>,
    /// When Codex last appended to the rollout, if the file system records it.
    pub(super) written: Option<SystemTime>,
}

/// What each rollout said when last read, so that a load reads only the rollouts written since.
/// The engine loads every recent thread on every tick, and most of them, a burst of finished `codex
/// exec` jobs above all, never write again. It holds only the rollouts of the last load's threads.
#[derive(Debug, Default)]
pub struct Rollouts {
    read: HashMap<PathBuf, Reading>,
}

/// A rollout's state when it was read, and what it said then.
#[derive(Debug)]
struct Reading {
    len: u64,
    written: Option<SystemTime>,
    last_turn: Option<Turn>,
}

impl Rollouts {
    /// What `rollout` says, or `None` when it is missing, unreadable or not a regular file. A
    /// rollout only grows, so one with the length and modification time it had in `earlier`, the
    /// last load's readings, still says what it said then and is not read again.
    pub(super) fn read(&mut self, rollout: &Path, earlier: &Rollouts) -> Option<Rollout> {
        let meta = fs::metadata(rollout).ok()?;
        let (len, written) = (meta.len(), meta.modified().ok());
        let last_turn = match earlier.read.get(rollout) {
            Some(known) if known.len == len && known.written == written => known.last_turn,
            _ => {
                let tail = read_tail(rollout, &meta).ok()?;
                tail.split(|&byte| byte == b'\n').rev().find_map(turn_event)
            }
        };
        let reading = Reading {
            len,
            written,
            last_turn,
        };
        self.read.insert(rollout.to_path_buf(), reading);
        Some(Rollout { last_turn, written })
    }
}

/// The last `TAIL_BYTES` of a file, from the first line that starts inside them.
fn read_tail(path: &Path, meta: &Metadata) -> io::Result<Vec<u8>> {
    // Checked before opening, because opening a FIFO blocks until something writes to it.
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    let mut file = File::open(path)?;
    let start = meta.len().saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    // Codex may append while this reads; the limit keeps the read to the window measured above.
    file.take(TAIL_BYTES).read_to_end(&mut tail)?;
    if start > 0 {
        let first_line_end = tail
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(tail.len(), |newline| newline + 1);
        tail.drain(..first_line_end);
    }
    Ok(tail)
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
    fn events_before_the_tail_window_are_not_read() {
        let home = TempDir::new("agents-rollout-tail");
        let padding = reply_mentioning("nothing");
        let lines_to_fill_window = TAIL_BYTES as usize / padding.len() + 1;
        let mut lines = vec![test_event("task_started")];
        lines.extend(std::iter::repeat_n(padding, lines_to_fill_window));
        assert_eq!(last_turn_of(&home, &lines), None);

        lines.push(test_event("task_complete"));
        assert_eq!(last_turn_of(&home, &lines), Some(Turn::Closed));
    }
}
