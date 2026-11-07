//! Build-artifact directories inside one worktree.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::boundary::Boundary;
use super::cargo_target::{CACHEDIR_TAG_FILE, is_tagged_build_folder};
use crate::map::ArtifactKind;

/// How many folder levels below the worktree's top are searched: deep enough for workspace members
/// (`crates/<name>/target`) and JavaScript packages (`packages/<name>/node_modules`), shallow
/// enough to stay out of source trees.
const SEARCH_DEPTH: usize = 2;

/// Build-artifact directories inside one worktree: each `node_modules/`, and each Cargo build
/// folder (see `is_cargo_target`), at the top and up to two levels down. Below the top, a folder
/// cargo tagged is one whatever its name, as `CARGO_TARGET_DIR` names it; two levels down only a
/// `target` is looked for. Sorted by path. The search enters no folder the walk skips, so a home
/// folder kept in git never has its private folders or `Library` searched.
pub fn find_artifacts(worktree: &Path, boundary: &Boundary) -> Vec<(PathBuf, ArtifactKind)> {
    let mut found = Vec::new();
    search(worktree, 0, boundary, &mut found);
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// A folder that cannot be listed is skipped: artifacts are a report on what the worktree holds,
/// and an unreadable corner of it has nothing Sutra could size or clean.
fn search(dir: &Path, depth: usize, boundary: &Boundary, found: &mut Vec<(PathBuf, ArtifactKind)>) {
    if depth == SEARCH_DEPTH {
        return probe(dir, found);
    }
    let Some(listed) = Listed::read(dir, boundary.skipped_children_of(dir)) else {
        return;
    };
    if depth > 0 {
        // A repository or linked worktree nested here reports its own artifacts.
        if listed.is_repo {
            return;
        }
        if listed.is_cargo_target(dir) {
            found.push((dir.to_path_buf(), ArtifactKind::CargoTarget));
            return;
        }
    }
    let (build_folders, subfolders) = listed.into_parts();
    found.extend(build_folders);
    for subfolder in subfolders {
        search(&subfolder, depth + 1, boundary, found);
    }
}

/// The deepest level checks for the two artifact names instead of listing the folder: two `lstat`s
/// cost under half of a listing, and the deepest level holds most of the folders searched (on
/// Daniel's M4 Max, 2026-11-03, 22,683 of 32,519).
fn probe(dir: &Path, found: &mut Vec<(PathBuf, ArtifactKind)>) {
    let is_dir = |path: &PathBuf| fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    let target = Some(dir.join("target")).filter(is_dir);
    let node_modules = Some(dir.join("node_modules")).filter(is_dir);
    if (target.is_none() && node_modules.is_none()) || exists(&dir.join(".git")) {
        return;
    }
    let listed = Listed {
        holds_tag: false,
        holds_manifest: exists(&dir.join("Cargo.toml")),
        is_repo: false,
        target,
        node_modules,
        subfolders: Vec::new(),
    };
    found.extend(listed.into_parts().0);
}

/// What one listing of a folder tells a search for build folders.
struct Listed {
    /// The folder holds a `CACHEDIR.TAG`, which may be cargo's.
    holds_tag: bool,
    holds_manifest: bool,
    /// The folder holds a `.git` entry: a repository or a linked worktree.
    is_repo: bool,
    /// Its subfolders named for build output, which a search never enters.
    target: Option<PathBuf>,
    node_modules: Option<PathBuf>,
    /// Its other subfolders, but `.git` and those `skipped` names.
    subfolders: Vec<PathBuf>,
}

impl Listed {
    /// `None` when `dir` cannot be listed. `DirEntry::file_type` does not follow symlinks, so a
    /// symlinked folder is never entered.
    fn read(dir: &Path, skipped: Option<&HashSet<OsString>>) -> Option<Listed> {
        let entries = fs::read_dir(dir).ok()?;
        let mut listed = Listed {
            holds_tag: false,
            holds_manifest: false,
            is_repo: false,
            target: None,
            node_modules: None,
            subfolders: Vec::new(),
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            listed.holds_tag |= name == CACHEDIR_TAG_FILE;
            listed.holds_manifest |= name == "Cargo.toml";
            if name == ".git" {
                listed.is_repo = true;
                continue;
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            if name == "target" {
                listed.target = Some(entry.path());
            } else if name == "node_modules" {
                listed.node_modules = Some(entry.path());
            } else if !skipped.is_some_and(|names| names.contains(&name)) {
                listed.subfolders.push(entry.path());
            }
        }
        Some(listed)
    }

    /// Whether `dir`, the folder listed, is itself a Cargo build folder, by cargo's tag in it.
    fn is_cargo_target(&self, dir: &Path) -> bool {
        self.holds_tag && is_tagged_build_folder(dir)
    }

    /// Its build folders, `node_modules` and a `target` cargo made, and the subfolders a search
    /// goes on into.
    fn into_parts(self) -> (Vec<(PathBuf, ArtifactKind)>, Vec<PathBuf>) {
        let holds_manifest = self.holds_manifest;
        let target = self
            .target
            .filter(|target| holds_manifest || is_tagged_build_folder(target))
            .map(|target| (target, ArtifactKind::CargoTarget));
        let node_modules = self
            .node_modules
            .map(|node_modules| (node_modules, ArtifactKind::NodeModules));
        (
            target.into_iter().chain(node_modules).collect(),
            self.subfolders,
        )
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::ScanOptions;
    use crate::map::test_support::{CACHEDIR_TAG, PYTEST_TAG, TempDir};

    /// `find_artifacts` in a worktree that is its own scan root and home folder.
    fn find_in(worktree: &Path) -> Vec<(PathBuf, ArtifactKind)> {
        let roots = [worktree.to_path_buf()];
        find_artifacts(
            worktree,
            &Boundary::new(&roots, worktree, &ScanOptions::default()),
        )
    }

    #[test]
    fn find_artifacts_reports_cargo_targets_and_node_modules_down_to_two_levels() {
        let fixture = TempDir::new("scan-artifacts-depth");
        let worktree = fixture.dir("app");
        fixture.write("app/Cargo.toml", "");
        fixture.dir("app/target/debug");
        fixture.write("app/crates/core/Cargo.toml", "");
        fixture.dir("app/crates/core/target");
        fixture.dir("app/node_modules/dep/node_modules");
        fixture.dir("app/packages/web/node_modules");
        fixture.write("app/a/b/c/Cargo.toml", "");
        fixture.dir("app/a/b/c/target");

        assert_eq!(
            find_in(&worktree),
            [
                (
                    fixture.path("app/crates/core/target"),
                    ArtifactKind::CargoTarget
                ),
                (fixture.path("app/node_modules"), ArtifactKind::NodeModules),
                (
                    fixture.path("app/packages/web/node_modules"),
                    ArtifactKind::NodeModules
                ),
                (fixture.path("app/target"), ArtifactKind::CargoTarget),
            ]
        );
    }

    #[test]
    fn find_artifacts_takes_a_target_with_no_manifest_beside_it_only_when_cargo_tagged_it() {
        let fixture = TempDir::new("scan-artifacts-cargo-made");
        let worktree = fixture.dir("app");
        fixture.write("app/shared/target/CACHEDIR.TAG", CACHEDIR_TAG);
        fixture.dir("app/docs/target");
        fixture.write("app/tools/gen/target/CACHEDIR.TAG", CACHEDIR_TAG);
        fixture.dir("app/tools/site/target");
        fixture.write("app/tools/empty/target/CACHEDIR.TAG", "");

        assert_eq!(
            find_in(&worktree),
            [
                (fixture.path("app/shared/target"), ArtifactKind::CargoTarget),
                (
                    fixture.path("app/tools/gen/target"),
                    ArtifactKind::CargoTarget
                ),
            ]
        );
    }

    #[test]
    fn find_artifacts_takes_a_folder_cargo_tagged_below_the_top_whatever_its_name() {
        let fixture = TempDir::new("scan-artifacts-any-name");
        let worktree = fixture.dir("app");
        fixture.write("app/target-mas/CACHEDIR.TAG", CACHEDIR_TAG);
        fixture.dir("app/target-mas/release/deps");
        fixture.write("app/.pytest_cache/CACHEDIR.TAG", PYTEST_TAG);
        fixture.write("app/CACHEDIR.TAG", CACHEDIR_TAG);

        assert_eq!(
            find_in(&worktree),
            [(fixture.path("app/target-mas"), ArtifactKind::CargoTarget)],
            "neither another tool's cache nor the worktree itself"
        );
    }

    #[test]
    fn find_artifacts_leaves_a_nested_repository_to_report_its_own() {
        let fixture = TempDir::new("scan-artifacts-nested-repo");
        let worktree = fixture.dir("app");
        fixture.dir("app/.git");
        fixture.dir("app/vendor/.git");
        fixture.dir("app/vendor/node_modules");
        fixture.write("app/.worktrees/feature/.git", "");
        fixture.write("app/.worktrees/feature/Cargo.toml", "");
        fixture.dir("app/.worktrees/feature/target");
        fixture.dir("app/web/node_modules");

        assert_eq!(
            find_in(&worktree),
            [(
                fixture.path("app/web/node_modules"),
                ArtifactKind::NodeModules
            )]
        );
    }

    #[test]
    fn find_artifacts_enters_no_folder_the_walk_skips() {
        let fixture = TempDir::new("scan-artifacts-skipped");
        let home = fixture.dir("home");
        fixture.dir("home/.git");
        fixture.dir("home/Desktop/app/node_modules");
        fixture.dir("home/Library/tool/node_modules");
        fixture.dir("home/web/node_modules");

        assert_eq!(
            find_in(&home),
            [(
                fixture.path("home/web/node_modules"),
                ArtifactKind::NodeModules
            )]
        );
    }
}
