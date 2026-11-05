//! Sutra works out the same workspace and the same folders for a clean as cargo, which
//! `cargo metadata` reports through the very calls `cargo clean` makes
//! (`src/ops/cargo_metadata.rs:54-55` at cargo's tag 0.100.0). Where cargo refuses, so does
//! Sutra.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sutra::map::clean::{CargoRefusal, PlanError};

use super::fixture::{CACHEDIR_TAG, Fixture};

/// What `cargo metadata` says about the workspace it is run in.
struct Metadata {
    workspace_root: PathBuf,
    target_directory: PathBuf,
    build_directory: PathBuf,
}

fn metadata(fixture: &Fixture, dir: &Path) -> Result<Metadata, String> {
    let output = fixture
        .cargo(dir)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .expect("start cargo metadata");
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON");
    let path = |key: &str| PathBuf::from(json[key].as_str().expect("a path field"));
    Ok(Metadata {
        workspace_root: path("workspace_root"),
        target_directory: path("target_directory"),
        build_directory: path("build_directory"),
    })
}

/// Cargo run in `cwd`, a workspace root, and Sutra asked about the folder `clicked` beside a
/// manifest agree on the workspace and on the folders a clean deletes. Cargo's folders are made
/// to exist and tagged, as a build leaves them, since Sutra plans only folders that exist.
fn assert_agree(fixture: &Fixture, cwd: &str, clicked: &str) {
    let cargo = metadata(fixture, &fixture.path(cwd)).expect("cargo metadata succeeds");
    for folder in [&cargo.target_directory, &cargo.build_directory] {
        fs::create_dir_all(folder).unwrap();
        fs::write(folder.join("CACHEDIR.TAG"), CACHEDIR_TAG).unwrap();
    }
    let clicked = fixture.path(clicked);
    fs::create_dir_all(&clicked).unwrap();

    let sutra = fixture.cargo_clean_of(&clicked);

    let canonical = |path: &Path| fs::canonicalize(path).unwrap();
    assert_eq!(
        sutra.workspace,
        canonical(&cargo.workspace_root),
        "workspace"
    );
    let target = canonical(&cargo.target_directory);
    let build = canonical(&cargo.build_directory);
    // Cargo deletes a folder inside the other along with it.
    let expected: BTreeSet<PathBuf> = if build.starts_with(&target) {
        [target].into()
    } else if target.starts_with(&build) {
        [build].into()
    } else {
        [target, build].into()
    };
    let planned: BTreeSet<PathBuf> = sutra.folders.into_iter().map(|f| f.path).collect();
    assert_eq!(planned, expected, "folders");
}

/// Cargo and Sutra both refuse the workspace in `cwd`; Sutra's reason, for the test to check.
fn both_refuse(fixture: &Fixture, cwd: &str) -> CargoRefusal {
    let dir = fixture.path(cwd);
    assert!(metadata(fixture, &dir).is_err(), "cargo refuses");
    let clicked = dir.join("target");
    fs::create_dir_all(&clicked).unwrap();
    match fixture.plan(&clicked) {
        Err(PlanError::Cargo(refusal)) => refusal,
        other => panic!("Sutra did not refuse as cargo does: {other:?}"),
    }
}

#[test]
fn a_package_cleans_target_beside_its_manifest() {
    let fixture = Fixture::new("plain");
    fixture.package("app", "");
    assert_agree(&fixture, "app", "app/target");
}

/// A workspace with members found by glob, a member outside it that names it, a package it
/// excludes, and a package with a workspace of its own inside it.
fn workspace(fixture: &Fixture) {
    fixture.write(
        "ws/Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\", \"../outside\"]\nexclude = [\"crates/excluded\"]\n",
    );
    fixture.package("ws/crates/a", "");
    fixture.package("ws/crates/b", "");
    fixture.package("ws/crates/excluded", "");
    fixture.package("outside", "workspace = \"../ws\"\n");
    fixture.package("ws/inner", "\n[workspace]\n");
}

#[test]
fn members_and_a_member_outside_that_names_the_root_share_the_workspace_target() {
    let fixture = Fixture::new("workspace-members");
    workspace(&fixture);
    assert_agree(&fixture, "ws", "ws/target");
    assert_agree(&fixture, "ws/crates/a", "ws/crates/a/target");
    assert_agree(&fixture, "outside", "outside/target");
}

#[test]
fn an_excluded_package_and_one_with_its_own_workspace_clean_their_own_target() {
    let fixture = Fixture::new("workspace-own");
    workspace(&fixture);
    assert_agree(&fixture, "ws/crates/excluded", "ws/crates/excluded/target");
    assert_agree(&fixture, "ws/inner", "ws/inner/target");
}

#[test]
fn a_config_target_dir_is_relative_to_the_folder_holding_its_cargo_folder() {
    let fixture = Fixture::new("config-relative");
    fixture.package("app", "");
    fixture.write("app/.cargo/config.toml", "[build]\ntarget-dir = \"out\"\n");
    assert_agree(&fixture, "app", "app/target");
}

#[test]
fn the_cargo_home_config_counts_and_its_legacy_name_wins() {
    let fixture = Fixture::new("config-cargo-home");
    fixture.package("app", "");
    fixture.write(
        "cargo-home/config.toml",
        "[build]\ntarget-dir = \"home-relative\"\n",
    );
    assert_agree(&fixture, "app", "app/target");
    fixture.write(
        "cargo-home/config",
        "[build]\ntarget-dir = \"legacy-name\"\n",
    );
    assert_agree(&fixture, "app", "app/target");
}

#[test]
fn includes_load_in_order_under_the_including_files_own_values() {
    let fixture = Fixture::new("config-include");
    fixture.package("app", "");
    fixture.write(
        "app/.cargo/config.toml",
        "include = [{ path = \"missing.toml\", optional = true }, \"first.toml\", \"../shared/second.toml\"]\n",
    );
    fixture.write("app/.cargo/first.toml", "[build]\ntarget-dir = \"first\"\n");
    fixture.write(
        "app/shared/second.toml",
        "[build]\ntarget-dir = \"second\"\n",
    );
    assert_agree(&fixture, "app", "app/target");
    fixture.write(
        "app/.cargo/config.toml",
        "include = [\"first.toml\"]\n[build]\ntarget-dir = \"own\"\n",
    );
    assert_agree(&fixture, "app", "app/target");
}

#[test]
fn absolute_tilde_and_dotdot_target_dirs_are_taken_as_cargo_takes_them() {
    let fixture = Fixture::new("config-paths");
    fixture.package("app", "");
    let absolute = fixture.path("absolute-out");
    for value in [absolute.to_str().unwrap(), "~/tilde-out", "../sibling-out"] {
        fixture.write(
            "app/.cargo/config.toml",
            format!("[build]\ntarget-dir = \"{value}\"\n"),
        );
        assert_agree(&fixture, "app", "app/target");
    }
}

#[test]
fn cargo_target_dir_beats_cargo_build_target_dir_which_beats_config() {
    let mut fixture = Fixture::new("env-target");
    fixture.package("app", "");
    fixture.write(
        "app/.cargo/config.toml",
        "[build]\ntarget-dir = \"from-config\"\n",
    );
    fixture.set_var("CARGO_BUILD_TARGET_DIR", "from-build-var");
    assert_agree(&fixture, "app", "app/target");
    fixture.set_var("CARGO_TARGET_DIR", "from-target-var");
    assert_agree(&fixture, "app", "app/target");
}

#[test]
fn a_build_dir_set_on_its_own_is_cleaned_too() {
    let fixture = Fixture::new("build-dir-root");
    fixture.package("app", "");
    fixture.write(
        "app/.cargo/config.toml",
        "[build]\nbuild-dir = \"{workspace-root}/bcache\"\n",
    );
    assert_agree(&fixture, "app", "app/target");
}

#[test]
fn a_build_dir_in_cargo_home_by_workspace_hash_is_the_one_cargo_uses() {
    let fixture = Fixture::new("build-dir-hash");
    fixture.package("app", "");
    fixture.write(
        "app/.cargo/config.toml",
        "[build]\nbuild-dir = \"{cargo-cache-home}/build/{workspace-path-hash}\"\n",
    );
    assert_agree(&fixture, "app", "app/target");
}

#[test]
fn a_build_dir_from_the_environment_inside_or_equal_to_the_target_dir() {
    let mut fixture = Fixture::new("build-dir-env");
    fixture.package("app", "");
    fixture.set_var("CARGO_BUILD_BUILD_DIR", "env-build");
    assert_agree(&fixture, "app", "app/target");

    let mut inside = Fixture::new("build-dir-inside");
    inside.package("app", "");
    inside.set_var("CARGO_BUILD_BUILD_DIR", "target/intermediate");
    assert_agree(&inside, "app", "app/target");

    let mut follows = Fixture::new("build-dir-follows");
    follows.package("app", "");
    follows.set_var("CARGO_TARGET_DIR", "elsewhere");
    assert_agree(&follows, "app", "app/target");
}

#[test]
fn a_plain_file_named_cargo_holds_no_config_for_either() {
    let fixture = Fixture::new("plain-cargo-file");
    fixture.package("ws/app", "");
    fixture.write("ws/app/.cargo", "not a folder");
    fixture.write("ws/.cargo/config.toml", "[build]\ntarget-dir = \"far\"\n");
    assert_agree(&fixture, "ws/app", "ws/app/target");
}

#[test]
fn settings_cargo_rejects_are_refused_by_both() {
    let mut fixture = Fixture::new("refusals");
    fixture.package("empty", "");
    fixture.write("empty/.cargo/config.toml", "[build]\ntarget-dir = \"\"\n");
    assert!(matches!(
        both_refuse(&fixture, "empty"),
        CargoRefusal::Empty { .. }
    ));

    fixture.package("missing-include", "");
    fixture.write(
        "missing-include/.cargo/config.toml",
        "include = [\"gone.toml\"]\n",
    );
    assert!(matches!(
        both_refuse(&fixture, "missing-include"),
        CargoRefusal::BadInclude { .. }
    ));

    fixture.package("broken", "");
    fixture.write("broken/.cargo/config.toml", "[build\n");
    assert!(matches!(
        both_refuse(&fixture, "broken"),
        CargoRefusal::Invalid { .. }
    ));

    fixture.package("template", "");
    fixture.write(
        "template/.cargo/config.toml",
        "[build]\nbuild-dir = \"{workspace-rot}/x\"\n",
    );
    assert!(matches!(
        both_refuse(&fixture, "template"),
        CargoRefusal::BadTemplate { .. }
    ));

    fixture.package("empty-var", "");
    fixture.set_var("CARGO_TARGET_DIR", "");
    assert!(matches!(
        both_refuse(&fixture, "empty-var"),
        CargoRefusal::Empty { .. }
    ));
}
