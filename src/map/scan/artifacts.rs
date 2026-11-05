//! Build-artifact directories inside one worktree.

use std::fs;
use std::path::{Path, PathBuf};

use super::boundary::Boundary;
use crate::map::ArtifactKind;

/// How many folder levels below the worktree's top are searched: deep enough for workspace members
/// (`crates/<name>/target`) and JavaScript packages (`packages/<name>/node_modules`), shallow
/// enough to stay out of source trees.
const SEARCH_DEPTH: usize = 2;

/// Build-artifact directories inside one worktree: each `node_modules/`, and each `target/` that
/// Cargo made (its parent holds a `Cargo.toml`, or it holds Cargo's `CACHEDIR.TAG`), at the top and
/// up to two levels down. Sorted by path. The search enters no folder the walk skips, so a home
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
    let skipped = boundary.skipped_children_of(dir);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut target = None;
    let mut node_modules = None;
    let mut has_cargo_manifest = false;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        // A repository or linked worktree nested here reports its own artifacts.
        if name == ".git" && depth > 0 {
            return;
        }
        if name == "Cargo.toml" {
            has_cargo_manifest = true;
        }
        // `DirEntry::file_type` does not follow symlinks, so a symlinked folder is never entered.
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) || name == ".git" {
            continue;
        }
        if name == "target" {
            target = Some(entry.path());
        } else if name == "node_modules" {
            node_modules = Some(entry.path());
        } else if !skipped.is_some_and(|names| names.contains(&name)) {
            subdirs.push(entry.path());
        }
    }
    record(found, target, node_modules, has_cargo_manifest);
    for subdir in subdirs {
        search(&subdir, depth + 1, boundary, found);
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
    record(found, target, node_modules, exists(&dir.join("Cargo.toml")));
}

/// Keep a folder's `node_modules`, and its `target` when Cargo made it.
fn record(
    found: &mut Vec<(PathBuf, ArtifactKind)>,
    target: Option<PathBuf>,
    node_modules: Option<PathBuf>,
    has_cargo_manifest: bool,
) {
    if let Some(target) = target {
        if made_by_cargo(&target, has_cargo_manifest) {
            found.push((target, ArtifactKind::CargoTarget));
        }
    }
    if let Some(node_modules) = node_modules {
        found.push((node_modules, ArtifactKind::NodeModules));
    }
}

/// Whether Cargo made the folder `target`: it sits beside a `Cargo.toml`, or holds the
/// `CACHEDIR.TAG` Cargo writes into every target folder.
pub fn is_cargo_target(target: &Path) -> bool {
    let beside_manifest = target
        .parent()
        .is_some_and(|parent| exists(&parent.join("Cargo.toml")));
    made_by_cargo(target, beside_manifest)
}

/// `is_cargo_target`, for a search that has already listed the folder `target` sits in.
fn made_by_cargo(target: &Path, beside_manifest: bool) -> bool {
    beside_manifest || exists(&target.join("CACHEDIR.TAG"))
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::ScanOptions;
    use crate::map::test_support::TempDir;

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
    fn find_artifacts_takes_a_target_only_when_cargo_made_it() {
        let fixture = TempDir::new("scan-artifacts-cargo-made");
        let worktree = fixture.dir("app");
        fixture.write("app/shared/target/CACHEDIR.TAG", "");
        fixture.dir("app/docs/target");
        fixture.write("app/tools/gen/target/CACHEDIR.TAG", "");
        fixture.dir("app/tools/site/target");

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
