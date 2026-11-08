//! Discovery: find every git worktree under the roots, and the build artifacts inside each, and
//! the scratch folders of every session in Claude Code's scratch root (`scratch`); and where each
//! came from, by the remote its checkout was cloned from (`git`) or the package it builds
//! (`package`).
//!
//! The walk stops at each repository root, then each repository's worktree registry supplies its
//! linked worktrees wherever they live. On Daniel's M4 Max (2026-10-31) that found every worktree
//! in the home folder in about 0.2 s, against 13 s for a full descent.

mod artifacts;
mod boundary;
mod cargo_target;
mod git;
mod package;
mod registry;
mod scratch;
mod walk;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;

pub use artifacts::{Searched, find_artifacts, find_artifacts_throughout};
pub use boundary::{Boundary, MacOsAnswer, folder_access};
pub use cargo_target::{CACHEDIR_TAG_FILE, is_cargo_tag, is_cargo_target};
pub use git::{
    Lineage, git_activity, lineage, lineage_of_folder, lineage_of_scratch_folder, repo_info,
};
pub use package::{own_package, packages_built};
pub use scratch::{ScratchFolder, SessionFolder, scratch_folders, scratch_root, session_folders};

use super::{FolderAccess, RepoInfo, ScanStats, Source, SourceProblem};

/// Everything discovery found under the roots.
#[derive(Debug, Default)]
pub struct Discovery {
    /// Sorted by path, each worktree once.
    pub worktrees: Vec<Worktree>,
    /// Folders publishing dev-runner state of their own, repositories or not. Sorted.
    pub state_folders: Vec<PathBuf>,
    /// What discovery could not read: folders the walk could not list (skipped with everything
    /// below them), then worktrees whose git state could not be read (a `.git` file whose admin
    /// directory was deleted, or a repository that lies outside what Sutra may read). Each is one
    /// `Scan` problem per kind of error, naming the first path.
    pub problems: Vec<SourceProblem>,
    pub stats: ScanStats,
}

/// A path discovery could not read, and why.
type Failure = (PathBuf, io::ErrorKind);

#[derive(Debug, Clone)]
pub struct Worktree {
    /// Canonical path.
    pub path: PathBuf,
    pub repo: RepoInfo,
    /// Whether Sutra reads the folder. A registered worktree may lie in a private folder Sutra
    /// stays out of, outside the sandbox grant, or on a volume that is not mounted (a locked
    /// worktree); the walk reads every worktree it finds.
    pub access: FolderAccess,
}

/// Threads discovery runs on. Its cost is metadata I/O, which contends in the kernel past a few
/// threads. On Daniel's 16-core M4 Max (2026-11-04) a home walk cost 0.9 CPU-seconds and 0.7 s
/// on 4 threads, 1.3 to 2 and 0.4 s on 8, and 9 and 0.8 s on 16, and it repeats after lost events
/// for as long as a heavy build runs.
const DISCOVERY_THREADS: usize = 4;

/// Walk the boundary's roots, stopping at every repository root, then add each repository's linked
/// worktrees from its registry, wherever they live, reading nothing the boundary keeps out.
pub fn discover(boundary: &Boundary) -> Discovery {
    let started = Instant::now();
    on_discovery_threads(|| discover_on_current_pool(boundary, started))
}

/// Run `work` and what it runs in parallel on discovery's threads.
pub(crate) fn on_discovery_threads<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(DISCOVERY_THREADS)
        .thread_name(|index| format!("sutra-discover-{index}"))
        .build();
    match pool {
        Ok(pool) => pool.install(work),
        // Without threads of its own, discovery shares the global pool: slower, and as correct.
        Err(_) => work(),
    }
}

fn discover_on_current_pool(boundary: &Boundary, started: Instant) -> Discovery {
    let walked = walk::walk(boundary);

    let may_read = |path: &Path| boundary.may_read(path);
    let resolved: Vec<_> = walked
        .repo_roots
        .into_par_iter()
        .map(|path| (git::repo_info_within(&path, &may_read), path))
        .collect();
    let mut worktrees = BTreeMap::new();
    // A map, because a worktree can be both walked and registered.
    let mut unresolved = BTreeMap::new();
    for (repo, path) in resolved {
        match repo {
            Ok(repo) => {
                let worktree = Worktree {
                    path: path.clone(),
                    repo,
                    access: FolderAccess::Readable,
                };
                worktrees.insert(path, worktree);
            }
            Err(err) => {
                unresolved.insert(path, err.kind());
            }
        }
    }

    // Every repository the walk reached, through its main worktree or a linked one.
    let common_dirs: BTreeSet<PathBuf> = worktrees
        .values()
        .map(|worktree| worktree.repo.common_dir.clone())
        .collect();
    let registries: Vec<_> = common_dirs
        .par_iter()
        .map(|common_dir| registry::read(common_dir, boundary))
        .collect();
    let mut unreadable_dirs = walked.unreadable_dirs;
    for registry in registries {
        unreadable_dirs.extend(registry.unreadable_dirs);
        unresolved.extend(registry.unresolved);
        for worktree in registry.worktrees {
            // The walk already listed the registered worktrees that lie inside the roots.
            worktrees.entry(worktree.path.clone()).or_insert(worktree);
        }
    }

    // A worktree the walk could not resolve but its registry could is no problem.
    unresolved.retain(|path, _| !worktrees.contains_key(path));
    let mut problems = SourceProblem::per_kind(Source::Scan, unreadable_dirs);
    problems.extend(SourceProblem::per_kind(
        Source::Scan,
        unresolved.into_iter().collect(),
    ));
    let mut state_folders = walked.state_folders;
    state_folders.sort();
    Discovery {
        worktrees: worktrees.into_values().collect(),
        state_folders,
        problems,
        stats: ScanStats {
            dirs_visited: walked.dirs_visited,
            elapsed: started.elapsed(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{CACHEDIR_TAG, TempDir, git};
    use crate::map::{KeptOutBy, PrivateFolder, ScanOptions};

    /// The boundary of a scan with the fixture's `home` folder as both the root and the home folder.
    fn home_boundary(fixture: &TempDir, opts: &ScanOptions) -> Boundary {
        let home = fixture.path("home");
        Boundary::new(std::slice::from_ref(&home), &home, opts)
    }

    /// Discover with the fixture's `home` folder as both the root and the home folder.
    fn discover_home(fixture: &TempDir, opts: &ScanOptions) -> Discovery {
        discover(&home_boundary(fixture, opts))
    }

    fn paths(discovery: &Discovery) -> Vec<PathBuf> {
        discovery.worktrees.iter().map(|w| w.path.clone()).collect()
    }

    fn worktree<'a>(discovery: &'a Discovery, path: &Path) -> &'a Worktree {
        discovery
            .worktrees
            .iter()
            .find(|w| w.path == path)
            .unwrap_or_else(|| panic!("{} not discovered", path.display()))
    }

    #[test]
    fn discover_lists_a_repo_and_its_linked_worktrees_inside_and_outside_the_root_once_each() {
        let fixture = TempDir::new("scan-discover-linked");
        let main = fixture.repo("home/code/app");
        git(
            &main,
            &["worktree", "add", "-q", "-b", "inside", "../app-inside"],
        );
        let outside = fixture.path("scratch/app-outside");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "outside",
                outside.to_str().unwrap(),
            ],
        );

        let found = discover_home(&fixture, &ScanOptions::default());
        let inside = fixture.path("home/code/app-inside");
        assert_eq!(
            paths(&found),
            [main.clone(), inside.clone(), outside.clone()]
        );
        assert!(
            found
                .worktrees
                .iter()
                .all(|w| w.access == FolderAccess::Readable)
        );
        assert!(
            found
                .worktrees
                .iter()
                .all(|w| w.repo.common_dir == main.join(".git"))
        );
        assert_eq!(worktree(&found, &main).repo.worktree_name, None);
        let inside = &worktree(&found, &inside).repo;
        assert_eq!(inside.worktree_name.as_deref(), Some("app-inside"));
        assert_eq!(inside.branch.as_deref(), Some("inside"));
        let outside = &worktree(&found, &outside).repo;
        assert_eq!(outside.branch.as_deref(), Some("outside"));
        assert_eq!(found.problems, []);
    }

    #[test]
    fn discover_finds_a_plain_folder_by_its_own_state_folder_and_not_the_home_registry() {
        let fixture = TempDir::new("scan-state-folders");
        fixture.dir("home/.dev-runner");
        fixture.dir("home/code/site/.dev-runner");
        fixture.write("home/code/not-a-state-folder/.dev-runner", "a plain file");
        fixture.write("home/code/plain/README.md", "");

        let found = discover_home(&fixture, &ScanOptions::default());

        assert_eq!(found.state_folders, [fixture.path("home/code/site")]);
        assert!(found.worktrees.is_empty());
    }

    #[test]
    fn discover_leaves_out_a_registration_whose_folder_was_deleted() {
        let fixture = TempDir::new("scan-discover-stale");
        let main = fixture.repo("home/app");
        let gone = fixture.path("scratch/gone");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "gone",
                gone.to_str().unwrap(),
            ],
        );
        std::fs::remove_dir_all(&gone).unwrap();

        let found = discover_home(&fixture, &ScanOptions::default());
        assert_eq!(paths(&found), [main]);
    }

    #[test]
    fn discover_lists_a_locked_registration_whose_folder_is_gone_as_missing() {
        let fixture = TempDir::new("scan-discover-locked");
        let main = fixture.repo("home/app");
        let unmounted = fixture.path("volume/app");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "away",
                unmounted.to_str().unwrap(),
            ],
        );
        git(&main, &["worktree", "lock", unmounted.to_str().unwrap()]);
        std::fs::remove_dir_all(fixture.path("volume")).unwrap();

        let found = discover_home(&fixture, &ScanOptions::default());
        let away = worktree(&found, &unmounted);
        assert_eq!(away.access, FolderAccess::Missing);
        assert_eq!(away.repo.branch.as_deref(), Some("away"));
    }

    #[test]
    fn discover_lists_a_registered_worktree_it_cannot_read_with_the_branch_from_its_admin_dir() {
        let fixture = TempDir::new("scan-discover-sealed");
        let main = fixture.repo("home/app");
        let sealed = fixture.path("scratch/sealed");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "sealed",
                sealed.to_str().unwrap(),
            ],
        );
        fixture.restrict(&sealed, 0o000);

        let found = discover_home(&fixture, &ScanOptions::default());
        let sealed = worktree(&found, &sealed);
        assert_eq!(sealed.access, FolderAccess::Denied);
        assert_eq!(sealed.repo.branch.as_deref(), Some("sealed"));
    }

    #[test]
    fn discover_stops_at_a_repository_root() {
        let fixture = TempDir::new("scan-discover-stop");
        let outer = fixture.repo("home/outer");
        fixture.repo("home/outer/vendor/inner");

        let found = discover_home(&fixture, &ScanOptions::default());
        assert_eq!(paths(&found), [outer]);
        // `home` and `home/outer`; nothing below the repository root is listed.
        assert_eq!(found.stats.dirs_visited, 2);
    }

    #[test]
    fn discover_lists_a_root_that_is_a_repository_and_still_walks_below_it() {
        let fixture = TempDir::new("scan-discover-root-repo");
        let home = fixture.repo("home");
        let app = fixture.repo("home/code/app");
        fixture.repo("home/code/app/vendor/inner");

        let found = discover_home(&fixture, &ScanOptions::default());
        assert_eq!(paths(&found), [home.clone(), app]);
        assert_eq!(worktree(&found, &home).repo.common_dir, home.join(".git"));
    }

    #[test]
    fn discover_skips_build_output_tool_caches_and_system_folders() {
        let fixture = TempDir::new("scan-discover-skips");
        for skipped in [
            "home/plain/target/debug/dep",
            "home/plain/node_modules/dep",
            "home/Library/Application Support/app",
            "home/.cargo/git/checkouts/dep",
            "home/.local/share/tool",
            "home/Documents/notes",
        ] {
            fixture.repo(skipped);
        }
        // Go keeps module downloads in `~/go/pkg`, but GOPATH projects live in `~/go/src`.
        fixture.repo("home/go/pkg/mod/dep");
        // Cargo builds where `CARGO_TARGET_DIR` says, under any name, and tags the folder.
        fixture.write("home/plain/out-567/CACHEDIR.TAG", CACHEDIR_TAG);
        fixture.repo("home/plain/out-567/debug/build/dep");
        let kept = [
            fixture.repo("home/.local/state/tool"),
            fixture.repo("home/code/app"),
            fixture.repo("home/go/src/app"),
        ];

        let found = discover_home(&fixture, &ScanOptions::default());
        assert_eq!(paths(&found), kept);
    }

    #[test]
    fn discover_enters_only_the_private_folders_settings_let_in() {
        let fixture = TempDir::new("scan-discover-private");
        let notes = fixture.repo("home/Documents/notes");
        fixture.repo("home/Desktop/draft");
        let documents = ScanOptions {
            private_folders: [PrivateFolder::Documents].into(),
        };

        assert!(
            discover_home(&fixture, &ScanOptions::default())
                .worktrees
                .is_empty()
        );
        assert_eq!(paths(&discover_home(&fixture, &documents)), [notes]);
    }

    #[test]
    fn discover_does_not_read_a_registered_worktree_in_a_private_folder() {
        let fixture = TempDir::new("scan-discover-private-linked");
        let main = fixture.repo("home/code/app");
        let private = fixture.path("home/Desktop/app-wt");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt",
                private.to_str().unwrap(),
            ],
        );
        let desktop = ScanOptions {
            private_folders: [PrivateFolder::Desktop].into(),
        };

        let found = discover_home(&fixture, &ScanOptions::default());
        assert_eq!(
            worktree(&found, &private).access,
            FolderAccess::KeptOut(PrivateFolder::Desktop, KeptOutBy::Settings)
        );
        assert_eq!(
            worktree(&discover_home(&fixture, &desktop), &private).access,
            FolderAccess::Readable
        );
    }

    #[test]
    fn discover_stays_out_of_a_private_folder_until_macos_lets_sutra_in() {
        let fixture = TempDir::new("scan-discover-asking");
        let notes = fixture.repo("home/Documents/notes");
        let main = fixture.repo("home/code/app");
        let task = fixture.path("home/Documents/Codex/task");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "task",
                task.to_str().unwrap(),
            ],
        );
        let documents = ScanOptions {
            private_folders: [PrivateFolder::Documents].into(),
        };
        let mut boundary = home_boundary(&fixture, &documents);
        boundary.keep_out_until_macos_answers();

        let found = discover(&boundary);
        assert_eq!(paths(&found), [task.clone(), main.clone()]);
        let waiting = worktree(&found, &task);
        assert_eq!(
            waiting.access,
            FolderAccess::KeptOut(PrivateFolder::Documents, KeptOutBy::Prompt)
        );
        assert_eq!(
            waiting.repo.branch.as_deref(),
            Some("task"),
            "its registration outside the folder still reads"
        );

        boundary.macos_answered(PrivateFolder::Documents, MacOsAnswer::LetIn);
        let found = discover(&boundary);
        assert_eq!(paths(&found), [task.clone(), notes, main]);
        assert_eq!(worktree(&found, &task).access, FolderAccess::Readable);
    }

    #[test]
    fn discover_reports_unreadable_folders_as_one_problem_and_walks_past_them() {
        let fixture = TempDir::new("scan-discover-unreadable");
        let repo = fixture.repo("home/a/app");
        fixture.dir("home/sealed/inner");
        fixture.restrict(&fixture.path("home/sealed"), 0o000);
        fixture.dir("home/z-sealed");
        fixture.restrict(&fixture.path("home/z-sealed"), 0o000);

        let found = discover_home(&fixture, &ScanOptions::default());
        assert_eq!(paths(&found), [repo]);
        assert_eq!(
            found.problems,
            [SourceProblem {
                count: 2,
                ..SourceProblem::new(
                    Source::Scan,
                    fixture.path("home/sealed"),
                    io::ErrorKind::PermissionDenied
                )
            }]
        );
        // `home`, `home/a` and `home/a/app`.
        assert_eq!(found.stats.dirs_visited, 3);
    }

    #[test]
    fn discover_reports_a_walked_worktree_whose_admin_dir_is_gone_as_a_problem() {
        let fixture = TempDir::new("scan-discover-unresolved");
        let main = fixture.repo("home/app");
        git(
            &main,
            &["worktree", "add", "-q", "-b", "broken", "../app-broken"],
        );
        std::fs::remove_dir_all(main.join(".git/worktrees/app-broken")).unwrap();

        let found = discover_home(&fixture, &ScanOptions::default());
        assert_eq!(paths(&found), [main]);
        assert_eq!(
            found.problems,
            [SourceProblem::new(
                Source::Scan,
                fixture.path("home/app-broken"),
                io::ErrorKind::NotFound
            )]
        );
    }

    #[test]
    fn discover_resolves_a_symlinked_root_but_follows_no_symlink_below_it() {
        let fixture = TempDir::new("scan-discover-symlinks");
        let app = fixture.repo("home/app");
        let elsewhere = fixture.repo("elsewhere/lib");
        std::os::unix::fs::symlink(&elsewhere, fixture.path("home/lib-link")).unwrap();
        std::os::unix::fs::symlink(fixture.path("home"), fixture.path("home-link")).unwrap();

        let link = fixture.path("home-link");
        let boundary = Boundary::new(std::slice::from_ref(&link), &link, &ScanOptions::default());
        let found = discover(&boundary);
        assert_eq!(paths(&found), [app]);
    }
}
