//! When the work in a scratch folder was done, which no git dates: the newest file a walk of its
//! own files finds.

use super::*;

#[test]
fn a_scratch_folders_work_is_unknown_to_cleaning_until_the_walk_of_its_own_files_dates_it() {
    let fixture = TempDir::new("engine-scratch-work-unknown");
    let scratchpad = "claude-501/-Users-me/e26ca359/scratchpad";
    for name in ["r567", "r593"] {
        fixture.write(
            &format!("{scratchpad}/{name}/target/CACHEDIR.TAG"),
            CACHEDIR_TAG,
        );
    }
    let folder = |name: &str| fixture.path(&format!("{scratchpad}/{name}"));
    let mut state = state_over(&fixture);
    state.rediscover(None);
    let now = SystemTime::now();
    let idle = |state: &State| {
        let snapshot = state.snapshot();
        let hour = std::time::Duration::from_secs(60 * 60);
        let found = crate::map::clean::idle_folders(&snapshot, now, hour);
        found
            .iter()
            .map(|idle| idle.project.path.clone())
            .collect::<Vec<_>>()
    };
    // Each build folder is sized before the scratch folder's own files.
    for name in ["r567", "r593"] {
        let target = Job::Artifact(folder(name).join("target"));
        state.apply_usage(&target, Ok(usage(1 << 30)));
    }
    assert!(idle(&state).is_empty(), "the work in each is unknown");

    let written = |secs_ago: u64| DirUsage {
        newest_write: Some(now - std::time::Duration::from_secs(secs_ago)),
        ..usage(4096)
    };
    state.apply_usage(&Job::OwnFiles(folder("r567")), Ok(written(5 * 60)));
    state.apply_usage(&Job::OwnFiles(folder("r593")), Ok(written(2 * 86_400)));

    assert_eq!(
        idle(&state),
        [folder("r593")],
        "r567's own files were written minutes ago"
    );
}

#[test]
fn a_scratch_folder_is_dated_by_the_newest_file_its_walks_find_and_a_worktree_is_not() {
    let fixture = TempDir::new("engine-scratch-dated");
    fixture.write(
        "claude-501/-Users-me/e26ca359/scratchpad/out/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    fixture.write("home/code/app/Cargo.toml", "[package]\n");
    git(&fixture.path("home/code/app"), &["init", "-q"]);
    let mut state = state_over(&fixture);
    state.rediscover(None);
    let scratch = fixture.path("claude-501/-Users-me/e26ca359/scratchpad/out");
    let app = fixture.path("home/code/app");
    let walked = |secs| DirUsage {
        newest_write: Some(at(secs)),
        ..usage(4096)
    };
    let activity = |state: &State, path: &Path| state.projects[path].project.last_activity;

    state.apply_usage(&Job::OwnFiles(scratch.clone()), Ok(walked(2_000)));
    state.apply_usage(&Job::OwnFiles(app.clone()), Ok(walked(2_000)));
    assert_eq!(activity(&state, &scratch), Some(at(2_000)));
    assert_eq!(activity(&state, &app), None, "git dates a worktree");

    state.apply_usage(&Job::OwnFiles(scratch.clone()), Ok(walked(1_000)));
    assert_eq!(
        activity(&state, &scratch),
        Some(at(2_000)),
        "a file deleted since leaves the work done where it was"
    );
}
