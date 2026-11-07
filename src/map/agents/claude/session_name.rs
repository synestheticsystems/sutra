//! Claude Code records the name a session goes by in the session's own transcript, beside the
//! folder of its other files (see `session_files`), after each prompt. While the session runs, its
//! registry file names it too; once it has ended, that file is gone, and the transcript is where
//! its name lasts, for the scratch it left behind to go by.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::session_files::session_transcript;
use super::transcript::{TAIL_BYTES, newest_name, read_span};

/// The name the session `session_id` last went by, as the end of its transcript records it, when
/// the session started in the folder Claude Code calls `project_key` (see `session_files`).
/// `None` when the session never had a name, or has no transcript, or one that is anything but a
/// regular file, which is not Claude's. A transcript that cannot be read is noted in `failures`.
///
/// Only the end is read, `TAIL_BYTES`, as a transcript runs to many megabytes, and Claude Code
/// names the session again after every prompt: in each of the 70 named transcripts of the sessions
/// in Daniel's scratch root, the newest name lay within the last 34 KB (2026-11-07).
pub(crate) fn recorded_session_name(
    home: &Path,
    project_key: &str,
    session_id: &str,
    failures: &mut Vec<(PathBuf, io::ErrorKind)>,
) -> Option<String> {
    let projects = home.join(".claude").join("projects");
    let transcript = session_transcript(&projects.join(project_key).join(session_id));
    read_name(&transcript).unwrap_or_else(|err| {
        failures.push((transcript, err.kind()));
        None
    })
}

fn read_name(transcript: &Path) -> io::Result<Option<String>> {
    // Checked before opening, because opening a FIFO blocks until something writes to it.
    let len = match fs::metadata(transcript) {
        Ok(listed) if listed.is_file() => listed.len(),
        Ok(_) => return Ok(None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let bytes = read_span(transcript, len.saturating_sub(TAIL_BYTES), len)?;
    Ok(newest_name(&bytes))
}

#[cfg(test)]
mod tests {
    use super::super::transcript::{
        test_context_past_the_end_read, test_named, test_prompt, write_test_transcript,
    };
    use super::*;
    use crate::map::test_support::{TempDir, at};

    const KEY: &str = "-Users-me-vt-foundry";

    /// What `recorded_session_name` finds for the session `e26ca359` under `home`, and the
    /// transcripts it could not read.
    fn named(home: &TempDir) -> (Option<String>, Vec<(PathBuf, io::ErrorKind)>) {
        let mut failures = Vec::new();
        let name = recorded_session_name(home.root(), KEY, "e26ca359", &mut failures);
        (name, failures)
    }

    fn transcript(home: &TempDir) -> PathBuf {
        home.path(&format!(".claude/projects/{KEY}/e26ca359.jsonl"))
    }

    #[test]
    fn a_session_goes_by_the_name_the_end_of_its_transcript_records() {
        let home = TempDir::new("session-name-recorded");
        let mut lines = test_named("e26ca359", "Foundry").to_vec();
        lines.push(test_context_past_the_end_read());
        lines.extend(test_named("e26ca359", "Foundry Agent Studio architecture"));
        lines.push(test_prompt(at(1_000), "/Users/me/vt/foundry"));
        write_test_transcript(&transcript(&home), &lines);

        assert_eq!(
            named(&home),
            (
                Some("Foundry Agent Studio architecture".to_string()),
                vec![]
            )
        );
    }

    #[test]
    fn a_session_with_no_name_or_no_transcript_has_none_and_nothing_failed() {
        let home = TempDir::new("session-name-none");
        assert_eq!(named(&home), (None, vec![]), "no transcript");

        write_test_transcript(&transcript(&home), &[test_prompt(at(1_000), "/code/app")]);
        assert_eq!(named(&home), (None, vec![]), "never named");

        let dir = transcript(&home);
        fs::remove_file(&dir).unwrap();
        fs::create_dir(&dir).unwrap();
        assert_eq!(named(&home), (None, vec![]), "not a file Claude wrote");
    }

    #[test]
    fn a_transcript_that_cannot_be_read_is_noted_and_names_nothing() {
        let home = TempDir::new("session-name-unreadable");
        let path = transcript(&home);
        write_test_transcript(&path, &test_named("e26ca359", "Foundry"));
        home.restrict(&path, 0o000);

        assert_eq!(
            named(&home),
            (None, vec![(path, io::ErrorKind::PermissionDenied)])
        );
    }
}
