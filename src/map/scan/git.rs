//! Raw reads of the files git keeps for one worktree: no git binary and no git library. Reading the
//! files directly resolves HEAD in tens of microseconds, and it can never write into a git dir (a
//! write would show up in Sutra's own file watcher). Where a checkout came from is read the same
//! way (`origin`).

mod origin;

pub use origin::{Lineage, lineage, lineage_of_folder, lineage_of_scratch_folder};

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::map::RepoInfo;

/// Hex digits of the commit id shown for HEAD.
const SHORT_ID_LEN: usize = 8;

/// Branch and head of one worktree, read from its git dir. Also used to refresh a known worktree.
pub fn repo_info(worktree: &Path) -> Option<RepoInfo> {
    repo_info_within(worktree, &|_| true).ok()
}

/// The newest of the git dir's `index` and `logs/HEAD` modification times. A fresh repository has
/// neither file, so `None` is an ordinary answer, not an error.
pub fn git_activity(repo: &RepoInfo) -> Option<SystemTime> {
    ["index", "logs/HEAD"]
        .iter()
        .filter_map(|file| {
            fs::metadata(repo.git_dir.join(file))
                .and_then(|m| m.modified())
                .ok()
        })
        .max()
}

/// `repo_info` for discovery, which must not touch a git dir outside `may_read` (a repository in a
/// private folder Sutra stays out of): such a git dir fails as `PermissionDenied`.
pub(super) fn repo_info_within(
    worktree: &Path,
    may_read: &dyn Fn(&Path) -> bool,
) -> io::Result<RepoInfo> {
    repo_info_of_git_dir(&git_dir_of(worktree)?, may_read)
}

/// `RepoInfo` from a worktree's git dir: the main `.git` directory, or a linked worktree's admin
/// directory, which holds its own `HEAD` and a `commondir` file pointing at the shared repository.
pub(super) fn repo_info_of_git_dir(
    git_dir: &Path,
    may_read: &dyn Fn(&Path) -> bool,
) -> io::Result<RepoInfo> {
    ensure_may_read(git_dir, may_read)?;
    let linked_common_dir = linked_common_dir(git_dir)?;
    let worktree_name = match &linked_common_dir {
        Some(_) => Some(file_name(git_dir)?),
        None => None,
    };
    let common_dir = linked_common_dir.unwrap_or_else(|| git_dir.to_path_buf());
    ensure_may_read(&common_dir, may_read)?;
    let git_dir = fs::canonicalize(git_dir)?;
    let common_dir = fs::canonicalize(common_dir)?;
    let (branch, head) = read_head(&git_dir, &common_dir)?;
    Ok(RepoInfo {
        common_dir,
        git_dir,
        worktree_name,
        branch,
        head,
    })
}

/// The shared git dir that `git_dir`, a linked worktree's admin directory, names in its `commondir`
/// file; `None` for any other git dir, which has no such file.
fn linked_common_dir(git_dir: &Path) -> io::Result<Option<PathBuf>> {
    match fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => Ok(Some(resolve(git_dir, &text))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Where the worktree's `.git` entry leads: itself when it is a directory, or the `gitdir:` path a
/// linked worktree's `.git` file names.
fn git_dir_of(worktree: &Path) -> io::Result<PathBuf> {
    let dot_git = worktree.join(".git");
    if fs::metadata(&dot_git)?.is_dir() {
        return Ok(dot_git);
    }
    let text = fs::read_to_string(&dot_git)?;
    let target = text
        .trim()
        .strip_prefix("gitdir:")
        .ok_or_else(|| invalid_data(&dot_git, "no `gitdir:` line"))?;
    Ok(resolve(worktree, target))
}

/// The `.git` file a registry entry (`<common>/worktrees/<name>/`) names, in the linked worktree's
/// folder.
pub(super) fn registered_dot_git(admin_dir: &Path) -> io::Result<PathBuf> {
    let gitdir = admin_dir.join("gitdir");
    Ok(resolve(admin_dir, &fs::read_to_string(&gitdir)?))
}

/// The branch HEAD names (`None` when detached) and the short id it resolves to (`None` when the
/// branch is unborn).
fn read_head(git_dir: &Path, common_dir: &Path) -> io::Result<(Option<String>, Option<String>)> {
    let head_file = git_dir.join("HEAD");
    let text = fs::read_to_string(&head_file)?;
    let text = text.trim();
    let Some(refname) = text.strip_prefix("ref:").map(str::trim) else {
        return Ok((None, Some(short_id(&head_file, text)?)));
    };
    let branch = refname.strip_prefix("refs/heads/").map(str::to_owned);
    Ok((branch, resolve_ref(common_dir, refname)?))
}

/// A branch's short commit id: from its loose ref file, else from `packed-refs`, else `None` (the
/// branch has no commit yet). Branch refs live in the common dir, shared by every worktree.
fn resolve_ref(common_dir: &Path, refname: &str) -> io::Result<Option<String>> {
    let loose = common_dir.join(refname);
    match fs::read_to_string(&loose) {
        Ok(text) => return short_id(&loose, text.trim()).map(Some),
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
        Err(_) => {}
    }
    let packed = common_dir.join("packed-refs");
    let text = match fs::read_to_string(&packed) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    // Lines are `<id> <refname>`; `#` starts the header and `^` a peeled tag id.
    for line in text.lines() {
        if let Some((id, name)) = line.split_once(' ') {
            if name == refname && !id.starts_with(['#', '^']) {
                return short_id(&packed, id).map(Some);
            }
        }
    }
    Ok(None)
}

/// The first hex digits of a full object id (40 hex digits for SHA-1, 64 for SHA-256).
fn short_id(file: &Path, id: &str) -> io::Result<String> {
    if !matches!(id.len(), 40 | 64) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid_data(file, "not an object id"));
    }
    Ok(id[..SHORT_ID_LEN].to_owned())
}

/// A path read from a git pointer file. Git writes absolute paths unless
/// `worktree.useRelativePaths` is set, in which case the path is relative to the file's folder.
fn resolve(base: &Path, text: &str) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in base.join(text.trim()).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

fn ensure_may_read(path: &Path, may_read: &dyn Fn(&Path) -> bool) -> io::Result<()> {
    if may_read(path) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} lies outside what discovery may read", path.display()),
        ))
    }
}

fn file_name(path: &Path) -> io::Result<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| invalid_data(path, "no folder name"))
}

fn invalid_data(path: &Path, what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{}: {what}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;
    use crate::map::test_support::{TempDir, commit, git, set_mtime};

    fn short_head(dir: &Path) -> Option<String> {
        Some(git(dir, &["rev-parse", "--short=8", "HEAD"]))
    }

    #[test]
    fn repo_info_of_a_main_worktree_reads_the_branch_from_its_loose_ref() {
        let fixture = TempDir::new("scan-git-loose-ref");
        let repo = fixture.repo("app");
        assert!(repo.join(".git/refs/heads/main").is_file());

        let info = repo_info(&repo).unwrap();
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.head, short_head(&repo));
        assert_eq!(info.git_dir, repo.join(".git"));
        assert_eq!(info.common_dir, repo.join(".git"));
        assert_eq!(info.worktree_name, None);
    }

    #[test]
    fn repo_info_falls_back_to_packed_refs_when_the_loose_ref_is_gone() {
        let fixture = TempDir::new("scan-git-packed-ref");
        let repo = fixture.repo("app");
        git(&repo, &["tag", "-a", "-m", "release", "v1"]);
        git(&repo, &["pack-refs", "--all"]);
        assert!(!repo.join(".git/refs/heads/main").exists());

        let info = repo_info(&repo).unwrap();
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.head, short_head(&repo));
    }

    #[test]
    fn repo_info_of_a_detached_head_has_a_head_and_no_branch() {
        let fixture = TempDir::new("scan-git-detached");
        let repo = fixture.repo("app");
        git(&repo, &["switch", "-q", "--detach", "HEAD"]);

        let info = repo_info(&repo).unwrap();
        assert_eq!(info.branch, None);
        assert_eq!(info.head, short_head(&repo));
    }

    #[test]
    fn repo_info_of_an_unborn_branch_has_a_branch_and_no_head() {
        let fixture = TempDir::new("scan-git-unborn");
        let repo = fixture.dir("app");
        git(&repo, &["init", "-q", "-b", "fresh"]);

        let info = repo_info(&repo).unwrap();
        assert_eq!(info.branch.as_deref(), Some("fresh"));
        assert_eq!(info.head, None);
    }

    #[test]
    fn repo_info_of_a_linked_worktree_reads_its_admin_dir_and_the_shared_refs() {
        let fixture = TempDir::new("scan-git-linked");
        let main = fixture.repo("app");
        git(
            &main,
            &["worktree", "add", "-q", "-b", "feature", "../app-feature"],
        );
        let linked = fixture.path("app-feature");
        commit(&linked, "feature.txt");

        let info = repo_info(&linked).unwrap();
        assert_eq!(info.worktree_name.as_deref(), Some("app-feature"));
        assert_eq!(info.git_dir, main.join(".git/worktrees/app-feature"));
        assert_eq!(info.common_dir, main.join(".git"));
        assert_eq!(info.branch.as_deref(), Some("feature"));
        assert_eq!(info.head, short_head(&linked));
        assert_ne!(info.head, short_head(&main));
    }

    #[test]
    fn repo_info_within_refuses_a_git_dir_outside_what_may_be_read() {
        let fixture = TempDir::new("scan-git-may-read");
        let main = fixture.repo("private/app");
        git(
            &main,
            &["worktree", "add", "-q", "-b", "feature", "../../code/app"],
        );
        let private = fixture.path("private");

        let refused = repo_info_within(&fixture.path("code/app"), &|path| {
            !path.starts_with(&private)
        });
        assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn git_activity_is_the_newer_of_the_index_and_the_reflog() {
        let fixture = TempDir::new("scan-git-activity");
        let repo = fixture.repo("app");
        let info = repo_info(&repo).unwrap();
        let older = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let newer = older + Duration::from_secs(3600);

        set_mtime(&repo.join(".git/index"), older);
        set_mtime(&repo.join(".git/logs/HEAD"), newer);
        assert_eq!(git_activity(&info), Some(newer));

        set_mtime(&repo.join(".git/index"), newer + Duration::from_secs(1));
        assert_eq!(git_activity(&info), Some(newer + Duration::from_secs(1)));
    }

    #[test]
    fn git_activity_of_a_linked_worktree_reads_its_own_index_not_the_main_one() {
        let fixture = TempDir::new("scan-git-activity-linked");
        let main = fixture.repo("app");
        git(
            &main,
            &["worktree", "add", "-q", "-b", "feature", "../app-feature"],
        );
        let admin = main.join(".git/worktrees/app-feature");
        let older = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let newer = older + Duration::from_secs(3600);
        for file in [".git/index", ".git/logs/HEAD"] {
            set_mtime(&main.join(file), older);
        }
        set_mtime(&admin.join("index"), newer);
        set_mtime(&admin.join("logs/HEAD"), older);

        let linked = repo_info(&fixture.path("app-feature")).unwrap();
        assert_eq!(git_activity(&linked), Some(newer));
        assert_eq!(git_activity(&repo_info(&main).unwrap()), Some(older));
    }
}
