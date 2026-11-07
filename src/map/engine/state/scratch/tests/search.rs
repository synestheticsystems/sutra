//! Finding a session's scratch folders and searching each through for its build folders: a
//! worktree checked out there, a folder that turns readable, one whose search cannot list a folder
//! in it, a session's folders listed again, one moved into another's place, and one a failed
//! listing would have named.

use super::*;

#[test]
fn a_worktree_in_a_scratch_folder_keeps_its_build_folder_and_the_folder_has_none_of_its_own() {
    let fixture = TempDir::new("engine-scratch-worktrees");
    let scratchpad = "claude-501/-Users-me-vt-foundry/e26ca359/scratchpad";
    let main = fixture.repo("home/vt/sim");
    let nested = fixture.path(&format!("{scratchpad}/d208/wt-567"));
    let alone = fixture.path(&format!("{scratchpad}/wt"));
    for worktree in [&nested, &alone] {
        let (folder, branch) = (worktree.to_str().unwrap(), worktree.file_name().unwrap());
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch.to_str().unwrap(),
                folder,
            ],
        );
    }
    fixture.write(
        &format!("{scratchpad}/d208/wt-567/target/CACHEDIR.TAG"),
        CACHEDIR_TAG,
    );
    let mut state = state_over(&fixture);

    state.rediscover(None);

    let d208 = nested.parent().unwrap();
    assert_eq!(
        paths(&state),
        [nested.clone(), alone.clone(), fixture.path("home/vt/sim")],
        "the worktrees alone, as d208 holds nothing outside them"
    );
    assert_eq!(build_folders(&state, &nested), [nested.join("target")]);
    assert!(state.is_scratch_folder(d208));
    assert!(!state.is_scratch_folder(&alone), "a worktree stays one");
}

#[test]
fn a_scratch_folder_that_turns_readable_is_searched_again_at_the_next_rediscovery() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = TempDir::new("engine-scratch-access");
    fixture.write(
        "claude-501/-Users-me/e26ca359/scratchpad/out/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    let folder = fixture.path("claude-501/-Users-me/e26ca359/scratchpad/out");
    fixture.restrict(&folder, 0o000);
    let mut state = state_over(&fixture);
    state.rediscover(None);
    assert!(paths(&state).is_empty(), "nothing in it to see");

    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    state.rediscover(None);

    assert_eq!(paths(&state), [folder]);
}

#[test]
fn a_folder_a_scratch_folders_search_cannot_list_is_a_problem_and_keeps_its_build_folders() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = TempDir::new("engine-scratch-search-unlisted");
    let r567 = fixture.dir("claude-501/-Users-me/e26ca359/scratchpad/r567");
    fixture.write(
        "claude-501/-Users-me/e26ca359/scratchpad/r567/sub/cache/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    let (sub, sealed) = (r567.join("sub"), r567.join("sealed"));
    fixture.dir("claude-501/-Users-me/e26ca359/scratchpad/r567/sealed");
    fixture.restrict(&sealed, 0o000);
    let mut state = state_over(&fixture);
    let unread = |state: &State| {
        let problems = state.snapshot().problems.into_iter();
        let problems = problems.map(|problem| (problem.path, problem.kind, problem.count));
        problems.collect::<Vec<_>>()
    };
    let denied = io::ErrorKind::PermissionDenied;

    state.rediscover(None);
    assert_eq!(unread(&state), [(sealed.clone(), denied, 1)]);
    assert_eq!(build_folders(&state, &r567), [sub.join("cache")]);

    fixture.restrict(&sub, 0o000);
    assert!(state.refind_artifacts(&r567));
    assert_eq!(
        build_folders(&state, &r567),
        [sub.join("cache")],
        "it may be there still"
    );
    assert_eq!(unread(&state), [(sealed.clone(), denied, 2)]);

    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(state.refind_artifacts(&r567));
    assert_eq!(unread(&state), [(sub.clone(), denied, 1)]);
    assert!(!state.refind_artifacts(&r567), "nothing more changed");
}

#[test]
fn a_sessions_folders_listed_again_take_on_the_new_and_drop_the_gone_and_its_name() {
    let fixture = TempDir::new("engine-scratch-listed-again");
    let key = key_of(&fixture.dir("home/vt/foundry"));
    let scratchpad = format!("claude-501/{key}/e26ca359/scratchpad");
    fixture.write(&format!("{scratchpad}/r567/CACHEDIR.TAG"), CACHEDIR_TAG);
    let session = fixture.path(&format!("claude-501/{key}/e26ca359"));
    register_session(&fixture, "Foundry Agent Studio architecture");
    let mut state = state_over(&fixture);
    state.rediscover(None);
    assert_eq!(state.refind_scratch_folders(&session), Some(false));

    fixture.write(
        &format!("{scratchpad}/r593/target/CACHEDIR.TAG"),
        CACHEDIR_TAG,
    );
    std::fs::remove_dir_all(fixture.path(&format!("{scratchpad}/r567"))).unwrap();
    assert_eq!(state.refind_scratch_folders(&session), Some(true));

    let r593 = session.join("scratchpad/r593");
    assert_eq!(paths(&state), std::slice::from_ref(&r593));
    assert_eq!(build_folders(&state, &r593), [r593.join("target")]);
    assert_eq!(
        session_names(&state),
        ["Foundry Agent Studio architecture"],
        "a new folder knows its running session at once"
    );
    assert!(
        state.index().artifact_owner(&r593.join("target")).is_some(),
        "and file events find its build folder"
    );
}

#[test]
fn a_folder_moved_into_the_place_of_a_scratch_folder_is_searched_anew_at_the_next_rediscovery() {
    let fixture = TempDir::new("engine-scratch-replaced");
    let scratchpad = "claude-501/-Users-me/e26ca359/scratchpad";
    fixture.write(
        &format!("{scratchpad}/r567/target/CACHEDIR.TAG"),
        CACHEDIR_TAG,
    );
    let r567 = fixture.path(&format!("{scratchpad}/r567"));
    let mut state = state_over(&fixture);
    state.rediscover(None);
    state.apply_usage(&Job::Artifact(r567.join("target")), Ok(usage(1 << 30)));
    state.apply_usage(&Job::OwnFiles(r567.clone()), Ok(usage(4096)));

    std::fs::rename(&r567, fixture.path("moved-away")).unwrap();
    fixture.write("elsewhere/r593/out-593/CACHEDIR.TAG", CACHEDIR_TAG);
    std::fs::rename(fixture.path("elsewhere/r593"), &r567).unwrap();
    state.rediscover(None);

    assert_eq!(build_folders(&state, &r567), [r567.join("out-593")]);
    assert_eq!(
        state.projects[&r567].project.own_files,
        Measurement::Pending,
        "nothing measured of the folder that stood there"
    );
}

#[test]
fn a_scratch_folder_a_failed_listing_would_name_stays_on_the_map_until_it_is_known_gone() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = TempDir::new("engine-scratch-unlisted");
    let session = fixture.path("claude-501/-Users-me/e26ca359");
    let (key, scratchpad) = (
        fixture.path("claude-501/-Users-me"),
        session.join("scratchpad"),
    );
    let [r567, tasks, notes] =
        ["scratchpad/r567", "tasks", "notes"].map(|place| session.join(place));
    for folder in [&r567, &tasks, &notes] {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join("CACHEDIR.TAG"), CACHEDIR_TAG).unwrap();
    }
    let mut state = state_over(&fixture);
    state.rediscover(None);
    let all = [notes.clone(), r567.clone(), tasks.clone()];
    let unread = |state: &State| {
        let problems = state.snapshot().problems.into_iter();
        problems.map(|problem| problem.path).collect::<Vec<_>>()
    };
    let access = |state: &State, folder: &Path| state.projects[folder].project.access;

    fixture.restrict(&scratchpad, 0o000);
    assert_eq!(state.refind_scratch_folders(&session), None);
    assert_eq!(paths(&state), all);
    state.rediscover(None);
    assert_eq!(paths(&state), all, "nor does the rediscovery that says why");
    assert_eq!(unread(&state), std::slice::from_ref(&scratchpad));
    assert_eq!(
        access(&state, &r567),
        FolderAccess::Denied,
        "nothing is read in it, as nothing can be"
    );

    fixture.restrict(&key, 0o000);
    state.rediscover(None);
    assert_eq!(
        paths(&state),
        all,
        "nor while the folder of its project key cannot be listed"
    );
    assert_eq!(unread(&state), std::slice::from_ref(&key));

    for folder in [&key, &scratchpad] {
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for gone in [&r567, &notes] {
        std::fs::remove_dir_all(gone).unwrap();
    }
    // Its folders can be reached, and the folders beside its scratchpad not listed.
    fixture.restrict(&session, 0o311);
    state.rediscover(None);
    assert_eq!(
        paths(&state),
        std::slice::from_ref(&tasks),
        "the scratchpad's listing tells that r567 has gone, and the path of notes leads nowhere"
    );
    assert_eq!(unread(&state), [session]);
    assert_eq!(access(&state, &tasks), FolderAccess::Readable);
}
