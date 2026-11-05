//! Sutra deletes what `cargo clean` deletes in folders cargo really built: every path
//! `cargo clean --dry-run -v` lists and nothing else, but for the lock files and tag it leaves for
//! a build that may be waiting. After a real `cargo clean` of one copy and Sutra's clean of another
//! the two copies differ by those alone, `Cargo.lock` included.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use sutra::map::clean::{self, Plan, Progress};

use super::fixture::{Entry, Fixture, copy_tree, tree};

/// A package with a build script, which puts output in the host's profile folder as well when
/// built for an explicit target.
fn package_with_build_script(fixture: &Fixture, rel: &str) {
    fixture.package(rel, "");
    fixture.write(
        &format!("{rel}/build.rs"),
        "fn main() { println!(\"cargo::rerun-if-changed=build.rs\"); }\n",
    );
}

/// The workspace in `project` as cargo built it, by `builds` run there in turn.
fn built(fixture: &Fixture, project: &str, builds: &[&[&str]]) {
    for build in builds {
        fixture.run(&fixture.path(project), build);
    }
}

/// Every path `cargo clean --dry-run -v` lists for the workspace in `project`. The first run
/// leaves cargo's rustc cache in the build-dir if it was missing, so the second is the one read.
fn cargo_listing(fixture: &Fixture, project: &str) -> BTreeSet<PathBuf> {
    let dir = fixture.path(project);
    fixture.run(&dir, &["clean", "--dry-run"]);
    let output = fixture.run(&dir, &["clean", "--dry-run", "-v"]);
    String::from_utf8(output.stdout)
        .expect("paths in UTF-8")
        .lines()
        .filter(|line| line.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

/// Every path Sutra's clean of `clicked` deletes: each planned folder and all below it.
fn sutra_listing(fixture: &Fixture, clicked: &Path) -> BTreeSet<PathBuf> {
    let Plan::CargoClean(plan) = fixture.plan(clicked).expect("a plan") else {
        panic!("{} is cargo clean's folder", clicked.display());
    };
    let mut listing = BTreeSet::new();
    for folder in &plan.folders {
        listing.insert(folder.path.clone());
        listing.extend(
            tree(&folder.path)
                .into_keys()
                .map(|rel| folder.path.join(rel)),
        );
    }
    listing
}

/// Copy the built `project` twice, clean one copy with cargo and the other with Sutra, and
/// require the two to end up the same but for what Sutra leaves.
fn assert_same_but_for_the_skeleton(fixture: &Fixture, project: &str) {
    let by_cargo = fixture.path("by-cargo");
    let by_sutra = fixture.path("by-sutra");
    copy_tree(&fixture.path(project), &by_cargo.join(project));
    copy_tree(&fixture.path(project), &by_sutra.join(project));

    fixture.run(&by_cargo.join(project), &["clean"]);
    let plan = fixture
        .plan(&by_sutra.join(project).join("target"))
        .expect("a plan");
    let cleaned = clean::lock(&plan)
        .expect("no build runs")
        .delete(&Progress::default())
        .expect("the clean finishes");

    assert!(
        cleaned.folders.iter().all(|freed| freed.bytes > 0),
        "{cleaned:?}"
    );
    assert!(by_sutra.join(project).join("Cargo.lock").exists());
    assert!(by_sutra.join(project).join("target/CACHEDIR.TAG").exists());
    assert_eq!(tree(&by_cargo), without_skeleton(tree(&by_sutra)));
}

/// `cleaned`, a folder's tree after Sutra's clean, without the lock files and tags in it or the
/// folders that exist only to hold them.
fn without_skeleton(mut cleaned: BTreeMap<PathBuf, Entry>) -> BTreeMap<PathBuf, Entry> {
    const SKELETON: [&str; 4] = [
        "CACHEDIR.TAG",
        ".cargo-lock",
        ".cargo-build-lock",
        ".cargo-artifact-lock",
    ];
    let skeleton_files: Vec<PathBuf> = cleaned
        .keys()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| SKELETON.iter().any(|kept| name == *kept))
        })
        .cloned()
        .collect();
    let holders: BTreeSet<PathBuf> = skeleton_files
        .iter()
        .flat_map(|file| file.ancestors().skip(1))
        .map(Path::to_path_buf)
        .collect();
    let others: BTreeSet<PathBuf> = cleaned
        .keys()
        .filter(|path| !skeleton_files.contains(path) && !holders.contains(*path))
        .cloned()
        .collect();
    let holding_something_else: BTreeSet<PathBuf> = others
        .iter()
        .flat_map(|path| path.ancestors().skip(1))
        .map(Path::to_path_buf)
        .collect();
    for file in &skeleton_files {
        cleaned.remove(file);
    }
    for holder in holders.difference(&holding_something_else) {
        cleaned.remove(holder);
    }
    cleaned
}

#[test]
fn a_built_package_loses_what_cargo_clean_removes_and_no_more() {
    let fixture = Fixture::new("delete-plain");
    package_with_build_script(&fixture, "app");
    built(&fixture, "app", &[&["build"], &["build", "--release"]]);

    let cargo = cargo_listing(&fixture, "app");
    assert_eq!(sutra_listing(&fixture, &fixture.path("app/target")), cargo);
    assert_same_but_for_the_skeleton(&fixture, "app");
}

#[test]
fn a_built_workspace_loses_its_one_target() {
    let fixture = Fixture::new("delete-workspace");
    fixture.write("ws/Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    fixture.package("ws/crates/a", "");
    package_with_build_script(&fixture, "ws/crates/b");
    built(&fixture, "ws", &[&["build"]]);

    let cargo = cargo_listing(&fixture, "ws");
    assert_eq!(sutra_listing(&fixture, &fixture.path("ws/target")), cargo);
    assert_same_but_for_the_skeleton(&fixture, "ws");
}

#[test]
fn a_cross_build_loses_its_target_and_host_profile_folders() {
    let fixture = Fixture::new("delete-cross");
    package_with_build_script(&fixture, "app");
    let host = host_triple(&fixture);
    built(
        &fixture,
        "app",
        &[&["build", "--target", &host], &["build"]],
    );
    assert!(
        fixture
            .path("app/target")
            .join(&host)
            .join("debug")
            .exists()
    );

    let cargo = cargo_listing(&fixture, "app");
    assert_eq!(sutra_listing(&fixture, &fixture.path("app/target")), cargo);
    assert_same_but_for_the_skeleton(&fixture, "app");
}

#[test]
fn a_build_dir_of_its_own_goes_with_the_target_dir() {
    let fixture = Fixture::new("delete-build-dir");
    fixture.package("app", "");
    fixture.write(
        "app/.cargo/config.toml",
        "[build]\nbuild-dir = \"{workspace-root}/bcache\"\n",
    );
    built(&fixture, "app", &[&["build"]]);
    assert!(fixture.path("app/bcache").exists());

    let cargo = cargo_listing(&fixture, "app");
    assert_eq!(sutra_listing(&fixture, &fixture.path("app/target")), cargo);
    assert_same_but_for_the_skeleton(&fixture, "app");
}

#[test]
fn a_build_dir_in_cargo_home_by_workspace_hash_is_listed_as_cargo_lists_it() {
    // A copy would sit at another path and so hash to another folder; the listing alone is
    // compared.
    let fixture = Fixture::new("delete-build-dir-hash");
    fixture.package("app", "");
    fixture.write(
        "app/.cargo/config.toml",
        "[build]\nbuild-dir = \"{cargo-cache-home}/build/{workspace-path-hash}\"\n",
    );
    built(&fixture, "app", &[&["build"]]);
    assert!(fs::read_dir(fixture.path("cargo-home/build")).is_ok());

    let cargo = cargo_listing(&fixture, "app");
    assert_eq!(sutra_listing(&fixture, &fixture.path("app/target")), cargo);
}

/// A `target` left from before a package joined a workspace: no `cargo clean` of the workspace
/// deletes it, and clicking it clears it and nothing else.
#[test]
fn a_folder_cargo_clean_leaves_is_cleared_alone_when_clicked() {
    let fixture = Fixture::new("delete-folder");
    let member = fixture.package("ws/crates/a", "");
    fixture.run(&member, &["build"]);
    fixture.write(
        "ws/Cargo.toml",
        "[workspace]\nmembers = [\"crates/a\"]\nresolver = \"2\"\n",
    );
    let workspace = fixture.path("ws");
    fixture.run(&workspace, &["build"]);
    let left_over = member.join("target");
    let workspace_builds = tree(&workspace.join("target"));

    let plan = fixture.plan(&left_over).expect("a plan");

    assert!(matches!(plan, Plan::DeleteFolder { .. }), "{plan:?}");
    assert_eq!(plan.folders(), [left_over.as_path()]);
    clean::lock(&plan)
        .expect("no build is running")
        .delete(&Progress::default())
        .expect("the delete");
    assert!(!left_over.join("debug/deps").exists());
    assert!(member.join("src/main.rs").exists());
    assert_eq!(tree(&workspace.join("target")), workspace_builds);
}

/// The triple of the machine running the tests, from `cargo -vV`.
fn host_triple(fixture: &Fixture) -> String {
    let output = fixture.run(fixture.root(), &["-vV"]);
    String::from_utf8(output.stdout)
        .expect("UTF-8 version")
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("cargo -vV names the host")
        .to_string()
}
