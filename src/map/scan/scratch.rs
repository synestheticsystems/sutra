//! Claude Code's scratch root, `/private/tmp/claude-<uid>`. It holds a folder for each folder that
//! sessions start in, named as Claude Code names it (`-Users-me-code-app`), and in that a folder
//! for each session started there, named by the session's id. A session clones, builds and keeps
//! whatever it throws away in its own folder, mostly in the `scratchpad` there, outside every
//! project, often with `CARGO_TARGET_DIR` pointing there. Each folder at the top of the scratchpad,
//! or beside it in the session's folder, is one of the session's scratch folders, one piece of
//! throwaway work, as `r567fixture` or `target-567-fix` is. The root holds other things too (a file
//! of Claude Code's own state per session, folders of its tools), which are no session's.

use std::fs;
use std::io;
use std::os::unix::fs::DirEntryExt;
use std::path::{Path, PathBuf};

use nix::unistd::getuid;

use super::Failure;

/// The folder in a session's folder that holds most of what the session throws away.
const SCRATCHPAD: &str = "scratchpad";

/// The scratch root of the user this process runs as. Claude Code makes it in `/tmp`, which on
/// macOS links to `/private/tmp`, and file events name the folder the link leads to.
pub fn scratch_root() -> PathBuf {
    PathBuf::from(format!("/private/tmp/claude-{}", getuid()))
}

/// One session's folder in the scratch root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFolder {
    /// `<scratch root>/<project key>/<session id>`.
    pub path: PathBuf,
    /// Claude Code's name for the folder the session started in.
    pub project_key: String,
    pub session_id: String,
}

impl SessionFolder {
    /// The session folder at `path`, which a project key's folder holds, as `session_folders`
    /// finds them.
    pub fn at(path: &Path) -> Option<SessionFolder> {
        let name = |path: &Path| path.file_name()?.to_str().map(str::to_string);
        Some(SessionFolder {
            path: path.to_path_buf(),
            project_key: name(path.parent()?)?,
            session_id: name(path)?,
        })
    }
}

/// One of a session's scratch folders, as listing the session's folder found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchFolder {
    pub path: PathBuf,
    /// Which folder stood at `path`: its inode, which no other folder on the volume has while it
    /// exists. A folder moved into another's place comes from the same volume, so its inode alone
    /// tells it from the folder that stood there before, and so does that of a folder made anew
    /// there, as APFS never hands out an inode twice.
    pub inode: u64,
}

/// Every session's folder in `root`, sorted by path, and the folders that could not be listed.
/// With no root, no session has kept scratch since the Mac started, which is no problem. The folder
/// of a project key is the one whose name starts with `-`, as every absolute path's name does.
pub fn session_folders(root: &Path) -> (Vec<SessionFolder>, Vec<Failure>) {
    let mut found = Vec::new();
    let mut failures = Vec::new();
    let keys = match subfolders(root) {
        Ok(keys) => keys,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (found, failures),
        Err(err) => return (found, vec![(root.to_path_buf(), err.kind())]),
    };
    for (project_key, _) in keys.into_iter().filter(|(name, _)| name.starts_with('-')) {
        let folder = root.join(&project_key);
        match subfolders(&folder) {
            Ok(sessions) => {
                found.extend(sessions.into_iter().map(|(session_id, _)| SessionFolder {
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

/// The scratch folders of the session whose folder is `session`: each folder at the top of its
/// scratchpad, and each other folder beside the scratchpad, sorted by path, and the folders that
/// could not be listed. A session folder or a scratchpad removed meanwhile holds none, which is
/// no problem.
pub fn scratch_folders(session: &Path) -> (Vec<ScratchFolder>, Vec<Failure>) {
    let mut found = Vec::new();
    let mut failures = Vec::new();
    let mut list = |folder: &Path, leaving_out: Option<&str>| match subfolders(folder) {
        Ok(listed) => {
            let kept = listed
                .into_iter()
                .filter(|(name, _)| Some(name.as_str()) != leaving_out);
            found.extend(kept.map(|(name, inode)| ScratchFolder {
                path: folder.join(name),
                inode,
            }));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => failures.push((folder.to_path_buf(), err.kind())),
    };
    list(session, Some(SCRATCHPAD));
    list(&session.join(SCRATCHPAD), None);
    found.sort_by(|a, b| a.path.cmp(&b.path));
    (found, failures)
}

/// The folders in `dir`, each by its name and its inode, leaving out links and names that are not
/// UTF-8, which Claude Code never makes here.
fn subfolders(dir: &Path) -> io::Result<Vec<(String, u64)>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // `DirEntry::file_type` does not follow symlinks.
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Ok(name) = entry.file_name().into_string() {
            found.push((name, entry.ino()));
        }
    }
    Ok(found)
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
        let fixture = TempDir::new("scratch-session-folders");
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

        let (found, failures) = session_folders(&root);

        let folder = |key: &str, session: &str| SessionFolder {
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
        assert_eq!(
            SessionFolder::at(&root.join("-Users-me/e26c-cccc")),
            Some(folder("-Users-me", "e26c-cccc")),
            "a session folder read back from its path"
        );
    }

    #[test]
    fn no_scratch_root_is_no_problem_and_one_that_cannot_be_listed_is() {
        let fixture = TempDir::new("scratch-session-folders-missing");
        assert_eq!(
            session_folders(&fixture.path("claude-501")),
            (vec![], vec![])
        );

        let root = fixture.dir("sealed/claude-501");
        let key = fixture.dir("sealed/claude-501/-Users-me");
        fixture.restrict(&key, 0o000);
        assert_eq!(
            session_folders(&root),
            (vec![], vec![(key, io::ErrorKind::PermissionDenied)])
        );
        fixture.restrict(&root, 0o000);
        assert_eq!(
            session_folders(&root),
            (
                vec![],
                vec![(root.clone(), io::ErrorKind::PermissionDenied)]
            )
        );
    }

    /// The paths of the scratch folders `found`.
    fn paths(found: &[ScratchFolder]) -> Vec<PathBuf> {
        found.iter().map(|folder| folder.path.clone()).collect()
    }

    #[test]
    fn a_sessions_scratch_folders_are_those_atop_its_scratchpad_and_beside_it() {
        let fixture = TempDir::new("scratch-folders-of-a-session");
        let session = fixture.dir("e26ca359");
        fixture.dir("e26ca359/scratchpad/r567fixture/target");
        fixture.dir("e26ca359/scratchpad/target-567-fix");
        fixture.write("e26ca359/scratchpad/notes.md", "");
        fixture.dir("e26ca359/tasks");
        std::os::unix::fs::symlink(fixture.dir("elsewhere"), session.join("scratchpad/linked"))
            .unwrap();

        let (found, failures) = scratch_folders(&session);

        assert_eq!(
            paths(&found),
            [
                session.join("scratchpad/r567fixture"),
                session.join("scratchpad/target-567-fix"),
                session.join("tasks"),
            ],
            "neither the scratchpad itself, its files, its links nor what lies deeper"
        );
        assert_eq!(failures, []);
    }

    #[test]
    fn a_scratch_folder_is_listed_with_the_inode_of_the_folder_at_its_path() {
        use std::os::unix::fs::MetadataExt;
        let fixture = TempDir::new("scratch-folders-inode");
        let session = fixture.dir("e26ca359");
        let r567 = fixture.dir("e26ca359/scratchpad/r567");
        let first = fs::symlink_metadata(&r567).unwrap().ino();

        let (found, _) = scratch_folders(&session);
        assert_eq!(
            found,
            [ScratchFolder {
                path: r567.clone(),
                inode: first
            }]
        );

        fs::rename(&r567, fixture.path("e26ca359/moved-away")).unwrap();
        fs::rename(fixture.dir("elsewhere"), &r567).unwrap();
        let (found, _) = scratch_folders(&session);
        let listed = found.iter().find(|folder| folder.path == r567);
        assert_ne!(
            listed.map(|folder| folder.inode),
            Some(first),
            "a folder moved into its place"
        );
    }

    #[test]
    fn a_session_with_no_scratchpad_has_none_atop_it_and_one_that_cannot_be_listed_says_so() {
        let fixture = TempDir::new("scratch-folders-unlisted");
        let session = fixture.dir("e26ca359/tasks");
        let session = session.parent().unwrap().to_path_buf();
        let (found, failures) = scratch_folders(&session);
        assert_eq!(
            (paths(&found), failures),
            (vec![session.join("tasks")], vec![])
        );
        assert_eq!(
            scratch_folders(&fixture.path("gone")),
            (vec![], vec![]),
            "a session folder removed meanwhile"
        );

        let scratchpad = fixture.dir("e26ca359/scratchpad");
        fixture.restrict(&scratchpad, 0o000);
        let (found, failures) = scratch_folders(&session);
        assert_eq!(
            (paths(&found), failures),
            (
                vec![session.join("tasks")],
                vec![(scratchpad, io::ErrorKind::PermissionDenied)]
            )
        );
    }
}
