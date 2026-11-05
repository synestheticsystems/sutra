//! Sizes: activity and measurements after a change, and which project a build folder's size lands on.

use super::*;

#[test]
fn a_file_written_in_a_worktree_shows_as_activity() {
    let fixture = TempDir::new("engine-file-change");
    let app = crate_repo(&fixture, "app");
    let (engine, snapshots) = start(roots(&fixture), ScanOptions::default());
    wait_for(&snapshots, "the repository", |s| project(s, &app).is_some());

    let before = SystemTime::now();
    let written = fixture.write("app/src/main.rs", b"fn main() {}");
    announce(&engine, &written);
    wait_for(&snapshots, "a file change in app", |s| {
        project(s, &app).is_some_and(|row| {
            row.activity_source == Some(ActivitySource::FileChange)
                && row.last_activity.is_some_and(|at| at >= before)
        })
    });
}

#[test]
fn a_build_folder_is_measured_again_after_a_build_writes_to_it() {
    let fixture = TempDir::new("engine-artifact");
    let app = crate_repo(&fixture, "app");
    let (engine, snapshots) = start(roots(&fixture), ScanOptions::default());
    let measured = wait_for(&snapshots, "the first size of target/", |s| {
        target_bytes(s, &app).is_some()
    });
    let first = target_bytes(&measured, &app).unwrap();

    let written = fixture.write("app/target/debug/deps/libbig.rlib", [2; 256 * 1024]);
    announce(&engine, &written);
    wait_for(&snapshots, "a new size of target/", |s| {
        target_bytes(s, &app).is_some_and(|bytes| bytes >= first + 256 * 1024)
    });
}

#[test]
fn scan_once_sizes_a_whole_folder_without_the_worktree_nested_in_it() {
    let fixture = TempDir::new("engine-folder-size");
    let app = crate_repo(&fixture, "code/app");
    git(
        &app,
        &["worktree", "add", "-q", "-b", "wt", ".claude/worktrees/wt"],
    );
    let nested = app.join(".claude/worktrees/wt");
    fixture.write("code/app/.claude/worktrees/wt/big.bin", [3; 512 * 1024]);

    let snapshot = scan_once(&roots(&fixture), &ScanOptions::default(), true);

    let app_size = project(&snapshot, &app).unwrap().folder_size().unwrap();
    let target = target_bytes(&snapshot, &app).unwrap();
    assert!(
        app_size.bytes > target && app_size.bytes < 512 * 1024,
        "{app_size:?}: its target and its own files, without the nested worktree"
    );
    let nested_size = project(&snapshot, &nested).unwrap().folder_size().unwrap();
    assert!(nested_size.bytes >= 512 * 1024, "{nested_size:?}");
    assert!(snapshot.sizing().is_finished());
}

/// A repository `code/app` holding the folder `code/app/web`, whose `node_modules` the repository's
/// search finds. Returns the two folders.
fn repository_holding_a_package(fixture: &TempDir) -> (PathBuf, PathBuf) {
    let app = crate_repo(fixture, "code/app");
    let web = fixture.dir("code/app/web");
    fixture.write("code/app/web/node_modules/dep/index.js", [3; 8192]);
    (app, web)
}

/// The paths of the build folders of the project at `folder` on the map.
fn build_folders(state: &MapState, folder: &Path) -> Vec<PathBuf> {
    let snapshot = state.snapshot();
    let row = project(&snapshot, folder).expect("the folder is on the map");
    row.artifacts.iter().map(|dir| dir.path.clone()).collect()
}

#[test]
fn sizing_finishes_with_a_build_folder_held_by_a_registered_folder_nested_in_a_repository() {
    let fixture = TempDir::new("engine-nested-plain-sizing");
    let (app, web) = repository_holding_a_package(&fixture);
    register(&fixture, "ab12", &web, std::process::id(), "server");

    let (_engine, snapshots) = start(roots(&fixture), ScanOptions::default());

    let sized = wait_for(&snapshots, "every folder sized", |s| {
        project(s, &web).is_some() && s.sizing().is_finished()
    });
    let app_row = project(&sized, &app).unwrap();
    let held: Vec<&Path> = app_row.artifacts.iter().map(|d| d.path.as_path()).collect();
    assert_eq!(held, [app.join("target")]);
    assert!(app_row.folder_size().is_some());
    let web_row = project(&sized, &web).unwrap();
    assert_eq!(web_row.artifacts[0].path, web.join("node_modules"));
    assert!(web_row.folder_size().is_some_and(|size| size.bytes >= 8192));
}

#[test]
fn a_search_of_the_repository_again_leaves_the_build_folder_to_the_folder_nested_in_it() {
    let fixture = TempDir::new("engine-nested-plain-refind");
    let (app, web) = repository_holding_a_package(&fixture);
    register(&fixture, "ab12", &web, std::process::id(), "server");
    let mut state = scanned(&fixture);
    assert_eq!(build_folders(&state, &app), [app.join("target")]);

    assert!(!state.refind_artifacts(&app), "nothing changed");
    assert_eq!(build_folders(&state, &app), [app.join("target")]);

    // Found by both searches, whichever runs first.
    let target = web.join("target");
    fixture.write("code/app/web/Cargo.toml", "");
    fs::create_dir(&target).unwrap();
    assert!(!state.refind_artifacts(&app));
    assert!(state.refind_artifacts(&web));
    assert_eq!(build_folders(&state, &app), [app.join("target")]);
    assert_eq!(
        build_folders(&state, &web),
        [web.join("node_modules"), target]
    );
}

#[test]
fn a_folder_registered_while_running_takes_over_the_build_folder_from_the_repository_around_it() {
    let fixture = TempDir::new("engine-nested-plain-joins");
    let (app, web) = repository_holding_a_package(&fixture);
    let node_modules = web.join("node_modules");
    let mut state = scanned(&fixture);
    assert!(build_folders(&state, &app).contains(&node_modules));

    register(&fixture, "ab12", &web, std::process::id(), "server");
    assert!(state.refresh_environments(None));

    assert!(!build_folders(&state, &app).contains(&node_modules));
    assert_eq!(
        build_folders(&state, &web),
        std::slice::from_ref(&node_modules)
    );
    assert!(state.apply_usage(&Job::Artifact(node_modules), Ok(usage(20))));
    assert_eq!(
        project(&state.snapshot(), &web)
            .unwrap()
            .measured_build_bytes(),
        20,
        "its size lands on the folder that holds it now"
    );
}
