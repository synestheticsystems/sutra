//! Where a checkout came from, as the files git keeps for it record: what the list groups its card
//! by, and hangs it from. A linked worktree was made from its repository's main worktree, or from
//! the bare repository it was added to; a repository, from what its `origin` remote names. When
//! that is a folder on this Mac, a clone of a local checkout, the folder is followed in turn, a
//! worktree or a bare repository, until a remote names a host. A bare repository with no remote,
//! as Cargo's git cache keeps one, came from what it last fetched. Only files are read: no git
//! runs, and nothing is written.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{git_dir_of, linked_common_dir, resolve};
use crate::map::RepoInfo;
use crate::map::scan::Failure;

/// How many repositories a chain of origins follows. On Daniel's Mac (2026-11-07) no checkout lay
/// more than two steps from a remote, so this only stops a chain whose remotes run in a loop.
const LONGEST_CHAIN: usize = 8;

/// Where a checkout came from, as its git files record it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lineage {
    /// The checkouts and repositories it was made from, nearest first: for a linked worktree, its
    /// repository's main worktree or bare repository; then the folder each repository's `origin`
    /// names. Each is canonical, but one that is gone or Sutra may not read keeps the path git
    /// recorded.
    pub made_from: Vec<PathBuf>,
    /// The remote the chain ends at, as `normalized_url` writes it; `None` for a chain that ends at
    /// a repository with no remote, or at a folder that is gone or kept out of.
    pub remote: Option<String>,
}

/// The lineage of the worktree `repo`, following each folder an `origin` remote names as far as
/// `may_follow` lets Sutra read. Returns it, and what could not be read on the way: a folder that
/// is gone ends the chain as an ordinary answer, as a clone outlives what it was cloned from.
pub fn lineage(
    repo: &RepoInfo,
    may_follow: &(dyn Fn(&Path) -> bool + Sync),
) -> (Lineage, Vec<Failure>) {
    let made_from = match repo.worktree_name {
        Some(_) => vec![repo.folder().to_path_buf()],
        None => Vec::new(),
    };
    follow(repo.common_dir.clone(), made_from, may_follow)
}

/// The lineage of the folder `folder` when it is a checkout, which no discovery read as a
/// worktree, as a session's scratch folder is: `None` for a folder with no `.git`, or one whose
/// `.git` could not be read, which the failures then name.
pub fn lineage_of_folder(
    folder: &Path,
    may_follow: &(dyn Fn(&Path) -> bool + Sync),
) -> (Option<Lineage>, Vec<Failure>) {
    match Repository::checked_out_at(folder) {
        Ok(Some(repository)) => {
            let (lineage, failures) = follow(
                repository.common_dir,
                repository.main.into_iter().collect(),
                may_follow,
            );
            (Some(lineage), failures)
        }
        Ok(None) => (None, Vec::new()),
        Err(err) => (None, vec![(folder.join(".git"), err.kind())]),
    }
}

/// The lineage of a repository whose shared git dir is `common_dir`, made from `made_from` so far:
/// the folder its `origin` names, and on to what that one was made from.
fn follow(
    mut common_dir: PathBuf,
    mut made_from: Vec<PathBuf>,
    may_follow: &(dyn Fn(&Path) -> bool + Sync),
) -> (Lineage, Vec<Failure>) {
    let mut failures = Vec::new();
    let mut followed = HashSet::from([common_dir.clone()]);
    let remote = loop {
        let config = common_dir.join("config");
        let url = match fs::read_to_string(&config) {
            Ok(text) => origin_url(&text),
            // A repository without a config is one without a remote.
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => {
                failures.push((config, err.kind()));
                None
            }
        };
        let is_bare = repository_folder(&common_dir) == common_dir;
        let url = match url {
            None if is_bare => fetched_from(&common_dir).unwrap_or_else(|err| {
                failures.push((common_dir.join("FETCH_HEAD"), err.kind()));
                None
            }),
            url => url,
        };
        let Some(url) = url else {
            break None;
        };
        let Some(path) = local_path(&url, repository_folder(&common_dir)) else {
            break Some(normalized_url(&url));
        };
        if !may_follow(&path) {
            made_from.push(path);
            break None;
        }
        match Repository::named_by_remote(&path) {
            Ok(repository) => {
                if followed.len() == LONGEST_CHAIN
                    || !followed.insert(repository.common_dir.clone())
                {
                    break None;
                }
                made_from.push(repository.folder);
                made_from.extend(repository.main);
                common_dir = repository.common_dir;
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    failures.push((path.clone(), err.kind()));
                }
                made_from.push(path);
                break None;
            }
        }
    };
    (Lineage { made_from, remote }, failures)
}

/// A repository a chain of origins reaches.
struct Repository {
    /// The checkout or bare repository: a worktree's folder, or a bare repository's own.
    folder: PathBuf,
    /// When `folder` is a linked worktree, its repository's main worktree, or the bare repository
    /// it was added to, which it was made from in turn.
    main: Option<PathBuf>,
    /// The git dir the repository's worktrees share, whose `config` names its own origin.
    common_dir: PathBuf,
}

impl Repository {
    /// The repository `folder` checks out, by its `.git`: `None` when it has none.
    fn checked_out_at(folder: &Path) -> io::Result<Option<Repository>> {
        let git_dir = match git_dir_of(folder) {
            Ok(git_dir) => git_dir,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let linked = linked_common_dir(&git_dir)?;
        let main = linked
            .as_deref()
            .map(repository_folder)
            .map(Path::to_path_buf);
        let common_dir = fs::canonicalize(linked.unwrap_or(git_dir))?;
        Ok(Some(Repository {
            folder: fs::canonicalize(folder)?,
            main: main.map(|main| fs::canonicalize(&main).unwrap_or(main)),
            common_dir,
        }))
    }

    /// The repository at `path`, which a remote names: a checkout, or a git dir itself, as a bare
    /// repository is, or a worktree's `.git`.
    fn named_by_remote(path: &Path) -> io::Result<Repository> {
        let path = fs::canonicalize(path)?;
        if let Some(checkout) = Repository::checked_out_at(&path)? {
            return Ok(checkout);
        }
        // Every git dir holds a `HEAD`; a folder without one is no repository.
        fs::metadata(path.join("HEAD"))?;
        Ok(Repository {
            folder: repository_folder(&path).to_path_buf(),
            main: None,
            common_dir: path,
        })
    }
}

/// The folder the repository whose shared git dir is `common_dir` goes by, as `RepoInfo::folder`
/// says: the main worktree's, which holds the `.git`, or a bare repository's own.
fn repository_folder(common_dir: &Path) -> &Path {
    match common_dir.file_name() {
        Some(name) if name == ".git" => common_dir.parent().unwrap_or(common_dir),
        _ => common_dir,
    }
}

/// The URL the repository whose git dir is `git_dir` last fetched from, as its `FETCH_HEAD`
/// records it, each line ending `of` and the URL: `None` while it has fetched nothing. Cargo's git
/// cache keeps a bare repository for each dependency it fetches from git, with no remote but this.
fn fetched_from(git_dir: &Path) -> io::Result<Option<String>> {
    let text = match fs::read_to_string(git_dir.join("FETCH_HEAD")) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let first = text.lines().next().unwrap_or_default();
    let url = first.rsplit_once(" of ").map(|(_, url)| url.trim());
    Ok(url.filter(|url| !url.is_empty()).map(str::to_string))
}

/// The folder `url`, a remote's URL, names on this Mac: an absolute path, a `file://` URL, or a
/// path relative to `base`, the folder of the repository whose remote it is, as git reads one.
/// `None` for the URL of a host, whether it has a scheme (`https://`, `ssh://`) or is written
/// `host:path`, which a colon before any slash tells.
fn local_path(url: &str, base: &Path) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        return Some(PathBuf::from(path));
    }
    if url.starts_with('/') {
        return Some(PathBuf::from(url));
    }
    let names_a_host = url.contains("://")
        || url
            .find(':')
            .is_some_and(|colon| !url[..colon].contains('/'));
    (!names_a_host).then(|| resolve(base, url))
}

/// `url` as the key of the group its clones share: without its scheme, user or port, `host:path`
/// written `host/path`, without `.git` or a slash at its end, and its host in lowercase. So
/// `git@github.com:owner/repo.git` and `https://github.com/owner/repo` both read
/// `github.com/owner/repo`.
fn normalized_url(url: &str) -> String {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((_scheme, rest)) => rest.split_once('/').unwrap_or((rest, "")),
        None => url.split_once(':').unwrap_or((url, "")),
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _port)| host);
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    format!("{}/{}", host.to_lowercase(), path.trim_end_matches('/'))
}

/// The URL the `origin` remote has in the git config `text`: the first `url` in its section, which
/// git fetches from.
fn origin_url(text: &str) -> Option<String> {
    let mut in_origin = false;
    for line in text.lines() {
        let mut line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            let (header, after) = header.split_once(']').unwrap_or((header, ""));
            in_origin = is_origin_section(header);
            // A variable may follow its section's header on the same line.
            line = after.trim();
        }
        if !in_origin {
            continue;
        }
        if let Some(url) = value_of(line, "url") {
            return Some(url);
        }
    }
    None
}

/// Whether a section's header names the `origin` remote: `remote "origin"`, or the older
/// `remote.origin`. The section's name is matched ignoring case, as git matches it, and the
/// remote's name exactly.
fn is_origin_section(header: &str) -> bool {
    let header = header.trim();
    match header.split_once(char::is_whitespace) {
        Some((section, name)) => {
            section.eq_ignore_ascii_case("remote") && name.trim() == "\"origin\""
        }
        None => header.eq_ignore_ascii_case("remote.origin"),
    }
}

/// The value `line` gives the variable `name`, matched ignoring case as git matches it: what
/// follows the `=`, without its quotes or a comment after it, and with its escapes read.
fn value_of(line: &str, name: &str) -> Option<String> {
    let (variable, value) = line.split_once('=')?;
    if !variable.trim().eq_ignore_ascii_case(name) {
        return None;
    }
    let mut read = String::new();
    let mut quoted = false;
    let mut chars = value.trim().chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' => match chars.next() {
                Some('n') => read.push('\n'),
                Some('t') => read.push('\t'),
                Some(escaped) => read.push(escaped),
                None => {}
            },
            '#' | ';' if !quoted => break,
            c => read.push(c),
        }
    }
    Some(read.trim_end().to_string())
}

#[cfg(test)]
mod tests;
