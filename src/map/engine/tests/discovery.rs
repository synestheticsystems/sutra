//! Discovery while running: what it reports, what it finds, and what it reads again.

use super::*;
use crate::map::test_support::CACHEDIR_TAG;

#[test]
fn scan_once_reports_the_sources_it_cannot_read_instead_of_showing_them_empty() {
    let fixture = TempDir::new("engine-problems");
    crate_repo(&fixture, "code/app");
    let sessions = fixture.path(".claude/sessions");
    let sealed = fixture.path("code/sealed");
    for dir in [&sessions, &sealed] {
        fs::create_dir_all(dir).unwrap();
        fixture.restrict(dir, 0o000);
    }
    // Rooted at `code`, as a sandbox grant of `~/code` is, so the walk does not also meet the
    // sealed session registry.
    let roots = Roots {
        dirs: vec![fixture.path("code")],
        ..roots(&fixture)
    };

    let snapshot = scan_once(&roots, &ScanOptions::default(), false);

    let denied = io::ErrorKind::PermissionDenied;
    assert_eq!(
        snapshot.problems,
        [
            SourceProblem::new(Source::Scan, sealed, denied),
            SourceProblem::new(Source::ClaudeSessions, sessions, denied),
        ]
    );
}

#[test]
fn a_worktree_added_while_running_is_discovered() {
    let fixture = TempDir::new("engine-new-worktree");
    let app = crate_repo(&fixture, "app");
    let (engine, snapshots) = start(roots(&fixture), ScanOptions::default());
    wait_for(&snapshots, "the repository", |s| project(s, &app).is_some());

    git(&app, &["worktree", "add", "-q", "-b", "wt", "../app-wt"]);
    announce(&engine, &app.join(".git/worktrees/app-wt"));
    let worktree = fixture.path("app-wt");
    wait_for(&snapshots, "the new worktree", |s| {
        project(s, &worktree)
            .and_then(|row| row.repo.as_ref())
            .is_some_and(|repo| repo.branch.as_deref() == Some("wt"))
    });
}

#[test]
fn rediscovery_reads_a_plain_folder_again_and_keeps_only_what_still_applies() {
    let fixture = TempDir::new("engine-plain-rediscovered");
    let blog = fixture.dir("sites/blog");
    register(&fixture, "ab12", &blog, std::process::id(), "server");
    let mut state = scanned(&fixture);
    let later = || Some(SystemTime::now());
    assert!(
        project(&state.snapshot(), &blog)
            .unwrap()
            .artifacts
            .is_empty()
    );
    let touched = at(2_000_000_000);
    assert!(state.touch(&blog, touched));

    // Made while events went missing, with nothing else to announce it.
    let node_modules = fixture.dir("sites/blog/node_modules");
    state.rediscover(later());
    let found = |state: &MapState| {
        let snapshot = state.snapshot();
        let row = project(&snapshot, &blog).expect("still registered").clone();
        (row.access, row.artifacts, row.own_files, row.last_activity)
    };
    let (access, artifacts, _, activity) = found(&state);
    assert_eq!(access, FolderAccess::Readable);
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].path, node_modules);
    assert_eq!(activity, Some(touched), "activity observed earlier stays");

    let own_files = Job::OwnFiles(blog.clone());
    assert!(state.apply_usage(&own_files, Ok(usage(10))));
    assert!(state.apply_usage(&Job::Artifact(node_modules.clone()), Ok(usage(20))));
    state.rediscover(later());
    let (_, artifacts, own, _) = found(&state);
    assert_eq!(own, measured(10), "sizes that still apply stay");
    assert_eq!(artifacts[0].measurement, measured(20));

    fixture.restrict(&blog, 0o000);
    state.rediscover(later());
    let (access, artifacts, own, _) = found(&state);
    assert_eq!(access, FolderAccess::Denied);
    assert!(artifacts.is_empty(), "nothing is read in a sealed folder");
    assert_eq!(own, Measurement::Pending, "a sealed folder shows no size");

    fs::set_permissions(&blog, fs::Permissions::from_mode(0o755)).unwrap();
    state.rediscover(later());
    let (access, artifacts, own, _) = found(&state);
    assert_eq!(access, FolderAccess::Readable, "readable again");
    assert_eq!(artifacts[0].measurement, Measurement::Pending);
    assert_eq!(own, Measurement::Pending);

    fs::remove_dir_all(&blog).unwrap();
    state.rediscover(later());
    let (access, artifacts, ..) = found(&state);
    assert_eq!(access, FolderAccess::Missing);
    assert!(artifacts.is_empty());
}

#[test]
fn the_folders_outside_the_roots_to_watch_are_the_readable_projects_and_shared_git_dirs_there() {
    let fixture = TempDir::new("engine-outside-folders");
    let home = fixture.path("home");
    let add_worktree = |main: &Path, folder: &Path| {
        let branch = folder.file_name().unwrap().to_str().unwrap();
        let folder = folder.to_str().unwrap();
        git(main, &["worktree", "add", "-q", "-b", branch, folder]);
    };
    let main = fixture.repo("home/code/app");
    add_worktree(&main, &fixture.path("home/code/in-the-root"));
    let open = fixture.path("scratch/open");
    add_worktree(&main, &open);
    let sealed = fixture.path("scratch/sealed");
    add_worktree(&main, &sealed);
    fixture.restrict(&sealed, 0o000);
    // A repository outside the root, reached through its worktree inside the root.
    let away = fixture.repo("away/lib");
    add_worktree(&away, &fixture.path("home/code/lib-wt"));
    // Folders only environments name: outside the root, nested in a project outside it, gone, and
    // inside the root.
    let blog = fixture.dir("sites/blog");
    let nested = fixture.dir("scratch/open/nested");
    let gone = fixture.path("sites/gone");
    let inside = fixture.dir("home/sites/inside");
    for (id, folder) in [("1", &blog), ("2", &nested), ("3", &gone), ("4", &inside)] {
        fixture.write(
            &format!("home/.dev-runner/{id}"),
            format!("DIR={}\nPID={}\n", folder.display(), std::process::id()),
        );
    }
    let roots = Roots {
        home: home.clone(),
        dirs: vec![home],
        scratch: None,
        sandboxed: false,
    };
    let mut state = MapState::new(&roots, &ScanOptions::default());

    state.rediscover(None);

    let on_map = state.project_paths();
    for listed in [&open, &sealed, &blog, &nested, &gone, &inside] {
        assert!(on_map.contains(listed), "{}", listed.display());
    }
    let folders: Vec<PathBuf> = state.folders_outside_the_roots().into_iter().collect();
    assert_eq!(folders, [away.join(".git"), open.clone(), blog.clone()]);

    // Only a project's own access says whether its folder reads, so its shared git dir is tried.
    fixture.restrict(&away.join(".git"), 0o000);
    let folders: Vec<PathBuf> = state.folders_outside_the_roots().into_iter().collect();
    assert_eq!(
        folders,
        [open, blog],
        "a git dir that does not read goes unwatched"
    );
}

#[test]
fn the_folders_in_the_scratch_root_need_no_watch_of_their_own() {
    let fixture = TempDir::new("engine-scratch-not-outside");
    let home = fixture.path("home");
    let main = fixture.repo("home/code/app");
    let session = fixture.dir("claude-501/-Users-me-code-app/e26ca359");
    fixture.write(
        "claude-501/-Users-me-code-app/e26ca359/scratchpad/out/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    // A linked worktree of a repository in the home folder, checked out in the session's scratch.
    let worktree = session.join("scratchpad/wt-567");
    let branch_out = ["worktree", "add", "-q", "-b", "wt-567"];
    git(
        &main,
        &[&branch_out[..], &[worktree.to_str().unwrap()]].concat(),
    );
    let roots = Roots {
        home: home.clone(),
        dirs: vec![home],
        scratch: Some(fixture.path("claude-501")),
        sandboxed: false,
    };
    let mut state = MapState::new(&roots, &ScanOptions::default());

    state.rediscover(None);

    let on_map = state.project_paths();
    assert!(on_map.contains(&session.join("scratchpad/out")) && on_map.contains(&worktree));
    assert!(
        state.folders_outside_the_roots().is_empty(),
        "the stream over the scratch root hears them"
    );
}
