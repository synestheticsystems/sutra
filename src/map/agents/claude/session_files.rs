//! Claude Code keeps the files of each session under `<home>/.claude/projects`, in a folder named
//! after the working directory the session started in: the session's own transcript,
//! `<session id>.jsonl`, and beside it a folder `<session id>/` of its other files, its subagents'
//! transcripts among them.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The longest folder name Claude Code derives from a working directory. It cuts a longer one to
/// this length and appends `-` and a hash of the whole path.
const MAX_FOLDER_NAME: usize = 200;

/// Claude Code's folder for the files of the session `session_id` started in `cwd`, under
/// `projects`, beside which lies the session's transcript (see `session_transcript`). `None` when
/// the folder of a long name is not there.
pub(super) fn session_folder(
    projects: &Path,
    cwd: &str,
    session_id: &str,
) -> io::Result<Option<PathBuf>> {
    let name = folder_name(cwd);
    if name.len() <= MAX_FOLDER_NAME {
        return Ok(Some(projects.join(name).join(session_id)));
    }
    // Claude Code 2.1.310 looks such a folder up by the part of its name it kept rather than by
    // the hash, and so does this. A session that has started no subagent may have only its
    // transcript there.
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
        let session = folder.join(session_id);
        if named_so && (session.is_dir() || session_transcript(&session).is_file()) {
            return Ok(Some(session));
        }
    }
    Ok(None)
}

/// The transcript of the session whose files Claude Code keeps in `folder`: `<session id>.jsonl`,
/// beside that folder.
pub(super) fn session_transcript(folder: &Path) -> PathBuf {
    let mut transcript = folder.as_os_str().to_os_string();
    transcript.push(".jsonl");
    PathBuf::from(transcript)
}

/// The folder name Claude Code derives from a working directory: each UTF-16 unit that is not an
/// ASCII letter or digit becomes `-`, as its `replace(/[^a-zA-Z0-9]/g, "-")` does.
pub(super) fn folder_name(cwd: &str) -> String {
    cwd.encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => char::from(byte),
            _ => '-',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

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
        let home = TempDir::new("session-files-long-folder");
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
        // A session that has started no subagent, so that only its transcript is there.
        let transcript_only = projects.join(format!("{kept}-a7c3de")).join("quiet");
        fs::create_dir_all(transcript_only.parent().unwrap()).unwrap();
        fs::write(session_transcript(&transcript_only), "").unwrap();

        assert_eq!(
            session_folder(&projects, &cwd, "session").unwrap(),
            Some(session)
        );
        let quiet = session_folder(&projects, &cwd, "quiet").unwrap();
        assert_eq!(quiet, Some(transcript_only));
        assert_eq!(session_folder(&projects, &cwd, "gone").unwrap(), None);
        let short = session_folder(&projects, "/code/app", "session").unwrap();
        assert_eq!(short, Some(projects.join("-code-app").join("session")));
    }

    #[test]
    fn a_sessions_transcript_lies_beside_the_folder_of_its_other_files() {
        assert_eq!(
            session_transcript(Path::new("/p/-code-app/0b1d")),
            Path::new("/p/-code-app/0b1d.jsonl")
        );
    }
}
