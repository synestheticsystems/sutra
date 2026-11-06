//! Beside each subagent's transcript, Claude Code keeps `agent-<id>.meta.json`, written as it starts
//! the subagent and replaced by a new file as the subagent finishes: 561 of the 568 marked finished
//! were born after their transcript. The format is undocumented; what is read here was checked
//! against 2,311 files written by Claude Code 2.1.142 to 2.1.317 (2026-11-05). Only the subagent's
//! name and the task it was given are read, loosely, so that a file or a field that is missing
//! costs only those words: what the subagent does, its transcript tells.
//!
//! The file also holds `finished`, which says no more than the transcript's end. Claude Code wrote
//! it from 2.1.303 on, always `true`, and only once the transcript said the turn was over: of the
//! 589 subagents run since, the 568 that finished carry it, and the 10 others whose turn had
//! closed and the 11 mid-turn lack it (2026-11-05). Nothing writes it for a subagent that dies
//! mid-turn either, which only `SUBAGENT_SILENCE_LIMIT` takes for gone.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The most of one meta file read. Claude Code writes a few hundred bytes; a larger file is not
/// one of its own, and is reported rather than read whole.
const MAX_META_BYTES: u64 = 64 * 1024;

/// What a subagent's meta file says of it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Meta {
    /// What the session called the subagent, as `dashboard-fixes` reads.
    pub(super) name: Option<String>,
    /// The task in a few words, as `Fix dashboard review findings` reads.
    pub(super) description: Option<String>,
}

/// The fields read, each as any JSON value, so that one of another type costs only itself.
#[derive(Deserialize)]
struct Fields {
    #[serde(default)]
    name: serde_json::Value,
    #[serde(default)]
    description: serde_json::Value,
}

/// The meta file of the subagent whose transcript is `transcript`: `agent-<id>.meta.json` beside
/// `agent-<id>.jsonl`.
pub(super) fn meta_path(transcript: &Path) -> PathBuf {
    transcript.with_extension("meta.json")
}

/// What the meta file at `path` says, or `None` when there is none: it is not there, or it is
/// anything but a regular file, which is not Claude's. One larger than `MAX_META_BYTES` is
/// `FileTooLarge`, and one that does not parse is `InvalidData`.
pub(super) fn read_meta(path: &Path) -> io::Result<Option<Meta>> {
    // Checked before opening, because opening a FIFO blocks until something writes to it.
    match fs::metadata(path) {
        Ok(found) if found.is_file() => {}
        Ok(_) => return Ok(None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut bytes = Vec::new();
    file.take(MAX_META_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_META_BYTES {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    let fields: Fields = serde_json::from_slice(&bytes)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let text = |value: serde_json::Value| match value {
        serde_json::Value::String(text) => Some(text),
        _ => None,
    };
    Ok(Some(Meta {
        name: text(fields.name),
        description: text(fields.description),
    }))
}

/// Writes the meta file Claude Code keeps beside the transcript at `transcript`, naming the
/// subagent `name` and its task `description`.
#[cfg(test)]
pub(super) fn write_test_meta(transcript: &Path, name: &str, description: &str) {
    let meta = serde_json::json!({
        "agentType": "general-purpose",
        "worktreePath": "/code/app/.claude/worktrees/agent-a1",
        "spawnedWithWorktree": true,
        "worktreeBranch": "worktree-agent-a1",
        "description": description,
        "name": name,
        "spawnDepth": 1,
    });
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(meta_path(transcript), meta.to_string()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn a_subagents_meta_file_lies_beside_its_transcript() {
        let transcript = Path::new("/p/-code-app/0b1d/subagents/agent-a1eadea8.jsonl");
        assert_eq!(
            meta_path(transcript),
            Path::new("/p/-code-app/0b1d/subagents/agent-a1eadea8.meta.json")
        );
    }

    #[test]
    fn a_meta_file_names_the_subagent_and_its_task() {
        let home = TempDir::new("subagent-meta-named");
        let transcript = home.path("agent-a1.jsonl");
        write_test_meta(
            &transcript,
            "dashboard-fixes",
            "Fix dashboard review findings",
        );

        assert_eq!(
            read_meta(&meta_path(&transcript)).unwrap(),
            Some(Meta {
                name: Some("dashboard-fixes".to_string()),
                description: Some("Fix dashboard review findings".to_string()),
            })
        );
    }

    #[test]
    fn a_meta_file_not_there_says_nothing_and_a_field_missing_or_of_another_type_costs_only_itself()
    {
        let home = TempDir::new("subagent-meta-loose");
        assert_eq!(read_meta(&home.path("agent-gone.meta.json")).unwrap(), None);
        let odd = home.write(
            "agent-odd.meta.json",
            r#"{"name": 7, "description": "Map the engine", "finished": true}"#,
        );
        assert_eq!(
            read_meta(&odd).unwrap(),
            Some(Meta {
                name: None,
                description: Some("Map the engine".to_string()),
            })
        );
        let bare = home.write("agent-bare.meta.json", r#"{"agentType": "Explore"}"#);
        assert_eq!(read_meta(&bare).unwrap(), Some(Meta::default()));
    }

    #[test]
    fn a_meta_file_that_is_not_regular_says_nothing_and_one_too_large_malformed_or_sealed_fails() {
        let home = TempDir::new("subagent-meta-bounded");
        let fifo = home.path("agent-fifo.meta.json");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // On a thread, so a read that blocks on the FIFO fails the test instead of hanging it.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read_meta(&fifo).map_err(|err| err.kind()));
        });
        let read = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the read returns despite the FIFO");
        assert_eq!(read, Ok(None));

        // Valid JSON once read whole, so only the size cap keeps it out.
        let padded = format!(
            r#"{{"name": "big"}}{}"#,
            " ".repeat(MAX_META_BYTES as usize)
        );
        let large = home.write("agent-large.meta.json", padded);
        let kind = read_meta(&large).map_err(|err| err.kind());
        assert_eq!(kind, Err(io::ErrorKind::FileTooLarge));

        let cut_short = home.write("agent-cut.meta.json", r#"{"agentType":"Explore","na"#);
        let kind = read_meta(&cut_short).map_err(|err| err.kind());
        assert_eq!(kind, Err(io::ErrorKind::InvalidData));

        let sealed = home.write("agent-sealed.meta.json", r#"{"name": "sealed"}"#);
        home.restrict(&sealed, 0o000);
        let kind = read_meta(&sealed).map_err(|err| err.kind());
        assert_eq!(kind, Err(io::ErrorKind::PermissionDenied));
    }
}
