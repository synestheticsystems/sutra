//! Claude Code keeps the files of each session under `<home>/.claude/projects`, in a folder named
//! after the working directory the session started in: the session's own transcript,
//! `<session id>.jsonl`, and beside it a folder `<session id>/` of its other files, its subagents'
//! transcripts among them. It names the session's scratch folder after that directory the same way
//! (see `scan::scratch`).

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::map::scan::Boundary;

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

/// The name of the working directory Claude Code derived the folder name `derived` from (see
/// `folder_name`), as far as the disk still shows it where Sutra may look. Several paths derive one
/// name, as every character but a letter or a digit becomes `-`, so each step down from `/` takes
/// the longest folder whose name derives the next part. Where none does, because the directory has
/// gone since or lies where Sutra may not look, the rest of `derived` is the best name left.
pub(crate) fn working_directory_name(derived: &str, boundary: &Boundary) -> String {
    let mut folder = PathBuf::from("/");
    let mut rest = derived;
    while let Some(below) = rest.strip_prefix('-') {
        if !boundary.may_read(&folder) {
            break;
        }
        let Some((name, after)) = longest_subfolder_deriving(&folder, below) else {
            break;
        };
        folder.push(name);
        rest = after;
    }
    let name = match rest {
        "" => folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        rest => Some(rest.trim_start_matches('-').to_string()),
    };
    // Only the root folder has no name of its own.
    name.filter(|name| !name.is_empty())
        .unwrap_or_else(|| "/".to_string())
}

/// The folder in `folder` with the longest name that derives the start of `rest` up to a `-` or its
/// end, and what of `rest` follows it. The listing tells a folder from anything else, so nothing
/// in `folder` is touched, not even a private folder the boundary keeps Sutra out of. A link is
/// passed over: Claude Code records the working directory with every link resolved.
fn longest_subfolder_deriving<'r>(folder: &Path, rest: &'r str) -> Option<(String, &'r str)> {
    let entries = fs::read_dir(folder).ok()?;
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let after = rest.strip_prefix(folder_name(&name).as_str())?;
            let whole_part = after.is_empty() || after.starts_with('-');
            // `DirEntry::file_type` comes from the listing and does not follow links.
            let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
            (whole_part && is_folder).then_some((name, after))
        })
        .min_by_key(|(_, after)| after.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::ScanOptions;
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

    /// The name of the working directory whose derived folder name is that of `cwd`.
    fn named_back(fixture: &TempDir, cwd: &Path) -> String {
        let boundary = fixture.boundary(&ScanOptions::default());
        working_directory_name(&folder_name(cwd.to_str().unwrap()), &boundary)
    }

    #[test]
    fn a_working_directory_is_named_back_from_its_folder_name_by_the_folders_still_there() {
        let fixture = TempDir::new("session-files-named-back");
        fixture.dir("vt/greyorange/greyorange");
        let foundry = fixture.dir("vt/greyorange/greyorange-foundry");
        let worktree = fixture.dir("vt/sim/.claude/worktrees/sad-pare-30adf7");
        let support = fixture.dir("Library/Application Support/Claude");

        assert_eq!(
            named_back(&fixture, &foundry),
            "greyorange-foundry",
            "the longest name of two that derive the part"
        );
        assert_eq!(named_back(&fixture, &worktree), "sad-pare-30adf7");
        assert_eq!(named_back(&fixture, &support), "Claude");

        fs::remove_dir_all(worktree.parent().unwrap()).unwrap();
        assert_eq!(
            named_back(&fixture, &worktree),
            "worktrees-sad-pare-30adf7",
            "past the folders still there, the rest of the name"
        );
    }

    #[test]
    fn a_working_directory_in_a_private_folder_is_named_without_looking_inside() {
        let fixture = TempDir::new("session-files-named-back-private");
        let app = fixture.dir("Documents/app");

        assert_eq!(
            named_back(&fixture, &app.join("gone")),
            "app-gone",
            "Documents, which Settings keep Sutra out of, is never listed"
        );
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
