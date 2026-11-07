//! Claude Code's scratch root, `/private/tmp/claude-<uid>`. It holds a folder for each folder that
//! sessions start in, named as Claude Code names it (`-Users-me-code-app`), and in that a folder
//! for each session started there, named by the session's id. A session clones, builds and keeps
//! whatever it throws away in its own folder, outside every project, often with `CARGO_TARGET_DIR`
//! pointing there. The root holds other things too (a file of Claude Code's own state per
//! session, folders of its tools), which are no session's.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nix::unistd::getuid;

use super::Failure;

/// The scratch root of the user this process runs as. Claude Code makes it in `/tmp`, which on
/// macOS links to `/private/tmp`, and file events name the folder the link leads to.
pub fn scratch_root() -> PathBuf {
    PathBuf::from(format!("/private/tmp/claude-{}", getuid()))
}

/// One session's scratch folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchFolder {
    /// `<scratch root>/<project key>/<session id>`.
    pub path: PathBuf,
    /// Claude Code's name for the folder the session started in.
    pub project_key: String,
    pub session_id: String,
}

/// Every session's scratch folder in `root`, sorted by path, and the folders that could not be
/// listed. With no root, no session has kept scratch since the Mac started, which is no problem.
/// The folder of a project key is the one whose name starts with `-`, as every absolute path's
/// name does.
pub fn scratch_folders(root: &Path) -> (Vec<ScratchFolder>, Vec<Failure>) {
    let mut found = Vec::new();
    let mut failures = Vec::new();
    let keys = match subfolders(root) {
        Ok(keys) => keys,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (found, failures),
        Err(err) => return (found, vec![(root.to_path_buf(), err.kind())]),
    };
    for project_key in keys.into_iter().filter(|name| name.starts_with('-')) {
        let folder = root.join(&project_key);
        match subfolders(&folder) {
            Ok(sessions) => {
                found.extend(sessions.into_iter().map(|session_id| ScratchFolder {
                    path: folder.join(&session_id),
                    project_key: project_key.clone(),
                    session_id,
                }));
            }
            // Removed since the root was listed.
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => failures.push((folder, err.kind())),
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    (found, failures)
}

/// The names of the folders in `dir`, leaving out links and names that are not UTF-8, which
/// Claude Code never makes here.
fn subfolders(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // `DirEntry::file_type` does not follow symlinks.
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Ok(name) = entry.file_name().into_string() {
            names.push(name);
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn the_scratch_root_is_this_users_in_the_folder_tmp_links_to() {
        let root = scratch_root();
        assert_eq!(root.parent(), Some(Path::new("/private/tmp")));
        assert_eq!(
            root.file_name().and_then(|name| name.to_str()),
            Some(format!("claude-{}", getuid()).as_str())
        );
    }

    #[test]
    fn every_sessions_folder_under_a_project_key_is_found_and_nothing_else_in_the_root() {
        let fixture = TempDir::new("scratch-folders");
        let root = fixture.dir("claude-501");
        fixture.dir("claude-501/-Users-me-code-app/0b1d-aaaa/scratchpad");
        fixture.dir("claude-501/-Users-me-code-app/7c3e-bbbb");
        fixture.dir("claude-501/-Users-me/e26c-cccc/tasks");
        fixture.write("claude-501/cache-break-state-0b1d-aaaa.json", "{}");
        fixture.dir("claude-501/bundled-skills/review");
        fixture.write("claude-501/-Users-me/stray-file", "");
        std::os::unix::fs::symlink(
            fixture.dir("elsewhere"),
            root.join("-Users-me-code-app/linked"),
        )
        .unwrap();

        let (found, failures) = scratch_folders(&root);

        let folder = |key: &str, session: &str| ScratchFolder {
            path: root.join(key).join(session),
            project_key: key.to_string(),
            session_id: session.to_string(),
        };
        assert_eq!(
            found,
            [
                folder("-Users-me", "e26c-cccc"),
                folder("-Users-me-code-app", "0b1d-aaaa"),
                folder("-Users-me-code-app", "7c3e-bbbb"),
            ]
        );
        assert_eq!(failures, []);
    }

    #[test]
    fn no_scratch_root_is_no_problem_and_one_that_cannot_be_listed_is() {
        let fixture = TempDir::new("scratch-folders-missing");
        assert_eq!(
            scratch_folders(&fixture.path("claude-501")),
            (vec![], vec![])
        );

        let root = fixture.dir("sealed/claude-501");
        let key = fixture.dir("sealed/claude-501/-Users-me");
        fixture.restrict(&key, 0o000);
        assert_eq!(
            scratch_folders(&root),
            (vec![], vec![(key, io::ErrorKind::PermissionDenied)])
        );
        fixture.restrict(&root, 0o000);
        assert_eq!(
            scratch_folders(&root),
            (
                vec![],
                vec![(root.clone(), io::ErrorKind::PermissionDenied)]
            )
        );
    }
}
