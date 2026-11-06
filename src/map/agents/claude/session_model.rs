//! Claude Code records on each reply in a session's own transcript, `<session id>.jsonl` beside the
//! folder of the session's other files (see `session_files`), the model that wrote it. The
//! registry names no model, so the transcript is read for that alone: back from its end to its
//! newest reply on a first read, and after that only what Claude Code appends.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::transcript::{TAIL_BYTES, newest_model, read_span};

/// The model each live session's transcript named when last read, and how far it was read, so
/// that a load reads only what Claude Code appended since: a busy session appends at every step of
/// its turn. It holds only the transcripts of the last load's sessions.
#[derive(Debug, Default)]
pub struct SessionModels {
    read: HashMap<PathBuf, Reading>,
}

/// A transcript's state when it was read, and what it said then.
#[derive(Debug, Clone)]
struct Reading {
    len: u64,
    modified: Option<SystemTime>,
    /// Where the last whole line read ends, and the next read begins. A line Claude Code was still
    /// writing begins here.
    read_to: u64,
    /// The model the newest reply read names.
    model: Option<String>,
}

impl SessionModels {
    /// The model the session whose transcript is `transcript` works with: the one its newest reply
    /// names (see `newest_model`). Claude Code only appends, so what the last load read, in
    /// `earlier`, still holds: a transcript unchanged since is not read again, and of one that has
    /// grown only what was appended is read. A model read therefore holds until a newer reply names
    /// another. A transcript that is not there, or that is anything but a regular file, which is
    /// not Claude's, names no model.
    pub(super) fn read(
        &mut self,
        transcript: &Path,
        earlier: &SessionModels,
    ) -> io::Result<Option<String>> {
        // Checked before opening, because opening a FIFO blocks until something writes to it.
        let listed = match fs::metadata(transcript) {
            Ok(listed) if listed.is_file() => listed,
            Ok(_) => return Ok(None),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let (len, modified) = (listed.len(), listed.modified().ok());
        let reading = match earlier.read.get(transcript) {
            Some(known) if known.len == len && known.modified == modified => known.clone(),
            Some(known) if known.len <= len => known.read_on(transcript, len, modified)?,
            // Not read before, or shrunk, so no longer the file that was.
            _ => Reading::UNREAD.read_on(transcript, len, modified)?,
        };
        let model = reading.model.clone();
        self.read.insert(transcript.to_path_buf(), reading);
        Ok(model)
    }
}

impl Reading {
    /// A transcript before its first read: no model is known, and the read may go back to its
    /// start.
    const UNREAD: Reading = Reading {
        len: 0,
        modified: None,
        read_to: 0,
        model: None,
    };

    /// This reading carried on through what Claude Code has appended to `transcript` since, now
    /// `len` bytes long and last modified at `modified`. A reply there replaces the model known;
    /// without one, the model stands, however much the session writes. The newest reply is looked
    /// for back from the end, `TAIL_BYTES` at first and four times as far each time after, until
    /// one is found or the read reaches where this reading stopped. Nearly always the first window
    /// holds it, but a prompt with large attachments can push a session's newest reply further
    /// back: in 9 of the 325 session transcripts on Daniel's Mac, as far as 1 MB (2026-11-05).
    fn read_on(
        &self,
        transcript: &Path,
        len: u64,
        modified: Option<SystemTime>,
    ) -> io::Result<Reading> {
        let mut window = TAIL_BYTES;
        loop {
            let from = self.read_to.max(len.saturating_sub(window));
            let bytes = read_span(transcript, from, len)?;
            // What follows the last newline is a line Claude Code is still appending, for the next
            // read to take from its start.
            let whole = bytes.iter().rposition(|&byte| byte == b'\n');
            let whole = whole.map_or(0, |newline| newline + 1);
            let model = newest_model(&bytes[..whole]);
            if model.is_some() || from == self.read_to {
                return Ok(Reading {
                    len,
                    modified,
                    read_to: from + whole as u64,
                    model: model.or_else(|| self.model.clone()),
                });
            }
            window = window.saturating_mul(4);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::super::transcript::{
        TEST_FOLDER, TEST_MODEL, test_context_past_the_end_read, test_prompt, test_reply as reply,
        test_request as request, test_written_by, write_test_transcript,
    };
    use super::*;
    use crate::map::test_support::{TempDir, at};

    /// What a load finds, as the next load will read it.
    fn load_of(transcript: &Path, earlier: &SessionModels) -> (Option<String>, SessionModels) {
        let mut this_load = SessionModels::default();
        let model = this_load
            .read(transcript, earlier)
            .expect("a readable transcript");
        (model, this_load)
    }

    fn opus() -> Option<String> {
        Some(TEST_MODEL.to_string())
    }

    #[test]
    fn a_sessions_model_is_the_one_its_newest_reply_names() {
        let home = TempDir::new("session-model-newest");
        let path = home.path("0b1d.jsonl");
        let t = at(1_000);
        let earlier = test_written_by(reply(t, Some("end_turn"), "text"), json!("claude-fable-5"));
        let lines = [earlier, test_prompt(t, TEST_FOLDER), reply(t, None, "text")];
        write_test_transcript(&path, &lines);

        assert_eq!(load_of(&path, &SessionModels::default()).0, opus());
    }

    #[test]
    fn a_transcript_unchanged_since_the_last_load_is_not_read_again() {
        let home = TempDir::new("session-model-unchanged");
        let path = home.path("0b1d.jsonl");
        write_test_transcript(&path, &[reply(at(1_000), Some("end_turn"), "text")]);
        let (first, earlier) = load_of(&path, &SessionModels::default());
        assert_eq!(first, opus());

        // Opening the transcript now fails, and its state can still be looked at.
        home.restrict(&path, 0o000);

        assert_eq!(load_of(&path, &earlier).0, opus());
        let kind = SessionModels::default()
            .read(&path, &SessionModels::default())
            .map_err(|err| err.kind());
        assert_eq!(
            kind,
            Err(io::ErrorKind::PermissionDenied),
            "without the earlier reading it would have to be opened"
        );
    }

    #[test]
    fn a_first_read_looks_back_past_the_end_read_for_the_newest_reply_as_far_as_the_start() {
        let home = TempDir::new("session-model-first-read");
        let path = home.path("0b1d.jsonl");
        let t = at(1_000);
        // A prompt with attachments larger than the end read, which push the reply out of it.
        let prompted = || {
            let attachments = test_context_past_the_end_read();
            [test_prompt(t, TEST_FOLDER), attachments, request(t)]
        };
        let mut lines = vec![reply(t, Some("end_turn"), "text")];
        lines.extend(prompted());
        write_test_transcript(&path, &lines);
        assert_eq!(load_of(&path, &SessionModels::default()).0, opus());

        write_test_transcript(&path, &prompted());
        let unanswered = load_of(&path, &SessionModels::default()).0;
        assert_eq!(unanswered, None, "no reply yet, from the end to the start");
    }

    #[test]
    fn a_transcript_that_has_grown_is_read_on_from_where_the_last_load_stopped() {
        let home = TempDir::new("session-model-read-on");
        let path = home.path("0b1d.jsonl");
        let t = at(1_000);
        let mut lines = vec![reply(t, Some("end_turn"), "text")];
        write_test_transcript(&path, &lines);
        let (_, earlier) = load_of(&path, &SessionModels::default());

        // Another model's reply of the same length takes the first one's place, where only a read
        // from the start would find it, and the session writes on, more than the end read holds.
        lines[0] = test_written_by(lines[0].clone(), json!("claude-test-5-5"));
        assert_eq!(lines[0].len(), reply(t, Some("end_turn"), "text").len());
        let attachments = test_context_past_the_end_read();
        lines.extend([test_prompt(t, TEST_FOLDER), attachments, request(t)]);
        write_test_transcript(&path, &lines);

        let (held, earlier) = load_of(&path, &earlier);
        assert_eq!(held, opus(), "what was appended names no model");
        let fresh = load_of(&path, &SessionModels::default()).0;
        assert_eq!(
            fresh.as_deref(),
            Some("claude-test-5-5"),
            "read afresh, it finds the other"
        );

        let switched = test_written_by(reply(t, None, "text"), json!("claude-fable-5-1"));
        lines.push(switched);
        write_test_transcript(&path, &lines);
        let (switched, _) = load_of(&path, &earlier);
        assert_eq!(
            switched.as_deref(),
            Some("claude-fable-5-1"),
            "a newer reply"
        );
    }

    #[test]
    fn a_reply_claude_code_made_up_leaves_the_model_as_it_was() {
        let home = TempDir::new("session-model-made-up");
        let path = home.path("0b1d.jsonl");
        let t = at(1_000);
        let mut lines = vec![reply(t, Some("end_turn"), "text")];
        write_test_transcript(&path, &lines);
        let (_, earlier) = load_of(&path, &SessionModels::default());

        let failed = reply(t, Some("stop_sequence"), "text");
        lines.extend([request(t), test_written_by(failed, json!("<synthetic>"))]);
        write_test_transcript(&path, &lines);

        assert_eq!(load_of(&path, &earlier).0, opus());
    }

    #[test]
    fn a_reply_still_being_appended_is_read_once_it_is_whole() {
        let home = TempDir::new("session-model-partial");
        let path = home.path("0b1d.jsonl");
        let t = at(1_000);
        let first = reply(t, Some("end_turn"), "text");
        let next = test_written_by(reply(t, None, "text"), json!("claude-fable-5-1"));
        let (begun, rest) = next.split_at(next.len() / 2);
        fs::write(&path, format!("{first}\n{begun}")).unwrap();
        let (model, earlier) = load_of(&path, &SessionModels::default());
        assert_eq!(model, opus());

        fs::write(&path, format!("{first}\n{begun}{rest}\n")).unwrap();

        let (model, _) = load_of(&path, &earlier);
        assert_eq!(model.as_deref(), Some("claude-fable-5-1"));
    }

    #[test]
    fn a_transcript_that_shrank_is_read_afresh() {
        let home = TempDir::new("session-model-shrank");
        let path = home.path("0b1d.jsonl");
        let t = at(1_000);
        let switched = test_written_by(reply(t, None, "text"), json!("claude-fable-5-1"));
        write_test_transcript(&path, &[test_prompt(t, TEST_FOLDER), switched]);
        let (_, earlier) = load_of(&path, &SessionModels::default());

        write_test_transcript(&path, &[reply(t, None, "text")]);

        assert_eq!(load_of(&path, &earlier).0, opus());
    }

    #[test]
    fn a_load_remembers_only_the_transcripts_it_asked_about() {
        let home = TempDir::new("session-model-forgotten");
        let left = home.path("left.jsonl");
        let path = home.path("0b1d.jsonl");
        for transcript in [&left, &path] {
            write_test_transcript(transcript, &[reply(at(1_000), None, "text")]);
        }
        let mut earlier = SessionModels::default();
        for transcript in [&left, &path] {
            earlier.read(transcript, &SessionModels::default()).unwrap();
        }

        let (_, this_load) = load_of(&path, &earlier);

        assert_eq!(this_load.read.keys().collect::<Vec<_>>(), [&path]);
    }

    #[test]
    fn a_transcript_not_there_or_not_a_regular_file_names_no_model() {
        let home = TempDir::new("session-model-absent");
        let gone = home.path("gone.jsonl");
        assert_eq!(load_of(&gone, &SessionModels::default()).0, None);

        let fifo = home.path("fifo.jsonl");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // On a thread, so a read that blocks on the FIFO fails the test instead of hanging it.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let read = SessionModels::default().read(&fifo, &SessionModels::default());
            let _ = sender.send(read.map_err(|err| err.kind()));
        });
        let read = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the read returns despite the FIFO");
        assert_eq!(read, Ok(None));
    }
}
