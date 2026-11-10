//! Environments: the two conventions merged into one, which registration speaks for a folder, and
//! what a folder's own state does when its registration goes.

use std::collections::BTreeSet;

use super::*;
use crate::map::docker::{Container, PublishedPort, Reading, Seen};

#[test]
fn scan_once_merges_both_environment_conventions_and_lists_a_plain_folder() {
    let fixture = TempDir::new("engine-scan-once");
    let app = crate_repo(&fixture, "code/app");
    git(
        &app,
        &["worktree", "add", "-q", "-b", "feature", "../app-wt"],
    );
    let worktree = fixture.path("code/app-wt");
    let pid = std::process::id();
    fixture.write("code/app-wt/.dev-runner/server.status", b"ready");
    fixture.write(
        ".dev-runner/ab12",
        format!(
            "DIR={}\nPID={pid}\nSTARTED=1700000000\nSERVER_PORT=3000\n",
            worktree.display()
        )
        .as_bytes(),
    );
    fixture.write(".dev-runner/ab12.vite.status", b"building: deps");
    let blog = fixture.path("sites/blog");
    fixture.write("sites/blog/index.html", b"");
    fixture.write(
        ".dev-runner/cd34",
        format!("DIR={}\nPID={pid}\nSTARTED=1700000000\n", blog.display()).as_bytes(),
    );

    let snapshot = scan_once(&roots(&fixture), &ScanOptions::default(), true);

    let paths: Vec<&Path> = snapshot.projects.iter().map(|p| p.path.as_path()).collect();
    // Both worktrees carry git activity from just now; the plain folder's only activity is its
    // environment's 2023 start, so it sorts last.
    assert_eq!(paths.len(), 3, "{paths:?}");
    assert!(paths[..2].contains(&app.as_path()) && paths[..2].contains(&worktree.as_path()));
    assert_eq!(paths[2], blog);

    let app_row = project(&snapshot, &app).unwrap();
    assert_eq!(app_row.activity_source, Some(ActivitySource::Git));
    assert_eq!(app_row.artifacts.len(), 1);
    assert_eq!(app_row.artifacts[0].kind, ArtifactKind::CargoTarget);
    assert!(target_bytes(&snapshot, &app).is_some_and(|bytes| bytes >= 8192));

    let worktree_row = project(&snapshot, &worktree).unwrap();
    let repo = worktree_row.repo.as_ref().unwrap();
    assert_eq!(repo.branch.as_deref(), Some("feature"));
    assert_eq!(repo.worktree_name.as_deref(), Some("app-wt"));
    let env = worktree_row.environment.as_ref().unwrap();
    assert!(env.sources.global && env.sources.project);
    assert!(env.alive);
    assert_eq!(env.port_for("server"), Some(3000));
    let units: Vec<(&str, &State)> = env
        .units
        .iter()
        .map(|u| (u.name.as_str(), &u.state))
        .collect();
    assert_eq!(
        units,
        [("server", &State::Ready), ("vite", &State::Building)]
    );

    let blog_row = project(&snapshot, &blog).unwrap();
    assert!(blog_row.repo.is_none() && blog_row.access == FolderAccess::Readable);
    assert!(blog_row.environment.is_some());
    assert_eq!(blog_row.activity_source, Some(ActivitySource::Environment));
    assert_eq!(snapshot.roots, [fixture.root().to_path_buf()]);
}

fn environment_of(state: &MapState, folder: &Path) -> Environment {
    let snapshot = state.snapshot();
    let row = project(&snapshot, folder).expect("the folder is on the map");
    row.environment
        .clone()
        .expect("the folder has an environment")
}

fn unit_names(environment: &Environment) -> Vec<&str> {
    environment.units.iter().map(|u| u.name.as_str()).collect()
}

#[test]
fn when_the_winning_process_dies_a_registration_still_alive_speaks_for_the_folder() {
    let fixture = TempDir::new("engine-liveness-winner");
    let own_pid = std::process::id();
    let app = fixture.dir("app");
    register(&fixture, "ab12", &app, own_pid, "server");
    // The project's own state outranks the registration while its process lives.
    let winner = LiveProcess::start();
    let winner_pid = winner.pid();
    fixture.write(
        "app/.dev-runner/meta",
        format!("PID={winner_pid}\nSTARTED=200\nVITE_PORT=5173\n"),
    );
    fixture.write("app/.dev-runner/vite.status", "ready");
    let mut state = scanned(&fixture);
    assert_eq!(environment_of(&state, &app).pid, winner_pid);
    assert!(!state.refresh_liveness(), "nothing died yet");

    drop(winner);

    assert!(state.refresh_liveness());
    let environment = environment_of(&state, &app);
    assert_eq!(environment.pid, own_pid);
    assert!(environment.alive && !environment.has_stopped());
    assert_eq!(environment.started, 100);
    assert_eq!(environment.port_for("server"), Some(3000));
    assert_eq!(environment.port_for("vite"), None);
    assert_eq!(unit_names(&environment), ["server"]);
    assert!(!state.refresh_liveness(), "it is noticed once");
}

#[test]
fn a_process_dying_is_noticed_with_no_state_file_read_and_is_no_activity() {
    let fixture = TempDir::new("engine-liveness-no-read");
    let app = fixture.dir("app");
    let process = LiveProcess::start();
    register(&fixture, "ab12", &app, process.pid(), "server");
    let mut state = scanned(&fixture);
    let activity = project(&state.snapshot(), &app).unwrap().last_activity;
    // A read of the state files would show this.
    fixture.write(".dev-runner/ab12.server.status", "failed");

    drop(process);

    assert!(state.refresh_liveness());
    let environment = environment_of(&state, &app);
    assert!(environment.has_stopped());
    assert_eq!(environment.units[0].state, State::Ready);
    let after = project(&state.snapshot(), &app).unwrap().last_activity;
    assert_eq!(after, activity);
}

#[test]
fn a_container_on_a_port_only_a_registration_names_stands_on_its_folder_once_the_winner_dies() {
    let fixture = TempDir::new("engine-liveness-container");
    let app = fixture.dir("app");
    // The registration names 3000 for its server; the folder's own state, 5173 for vite.
    register(&fixture, "ab12", &app, std::process::id(), "server");
    let winner = LiveProcess::start();
    let winner_pid = winner.pid();
    fixture.write(
        "app/.dev-runner/meta",
        format!("PID={winner_pid}\nSTARTED=200\nVITE_PORT=5173\n"),
    );
    let mut state = scanned(&fixture);
    let database = Container {
        id: "c1".to_string(),
        name: "server-db".to_string(),
        image: "postgres:17".to_string(),
        compose_folder: None,
        bound: Vec::new(),
        ports: vec![PublishedPort {
            port: 3000,
            to: 5432,
            addresses: BTreeSet::from(["127.0.0.1".parse().unwrap()]),
            udp: false,
        }],
        started: None,
    };
    state.saw_docker(Seen {
        reading: Reading::Said {
            socket: PathBuf::from("/run/docker.sock"),
            containers: vec![database],
            disk: None,
        },
        problem: None,
    });
    let on_app = |state: &MapState| project(&state.snapshot(), &app).unwrap().containers.len();
    assert_eq!(
        on_app(&state),
        0,
        "the folder's own state speaks, naming 5173"
    );

    drop(winner);

    assert!(state.refresh_liveness());
    assert_eq!(
        on_app(&state),
        1,
        "the registration speaks now, naming 3000"
    );
}

#[test]
fn a_plain_folder_keeps_its_own_state_through_its_registration_going_and_leaves_with_it() {
    let fixture = TempDir::new("engine-plain-folder");
    let blog = fixture.dir("sites/blog");
    register(&fixture, "ab12", &blog, std::process::id(), "server");
    fixture.write("sites/blog/.dev-runner/vite.status", "ready");

    let mut state = scanned(&fixture);

    let environment = environment_of(&state, &blog);
    assert!(environment.sources.global && environment.sources.project);
    assert_eq!(unit_names(&environment), ["server", "vite"]);
    assert!(!state.refresh_environments(None), "read again, unchanged");

    fs::remove_dir_all(fixture.path(".dev-runner")).unwrap();
    assert!(state.refresh_environments(None));
    let environment = environment_of(&state, &blog);
    assert!(!environment.sources.global && environment.sources.project);
    assert_eq!(unit_names(&environment), ["vite"]);

    fs::remove_dir_all(fixture.path("sites/blog/.dev-runner")).unwrap();
    assert!(state.refresh_environments(None));
    assert!(project(&state.snapshot(), &blog).is_none());
}

#[test]
fn a_plain_folder_publishing_only_its_own_state_is_found_by_a_fresh_start() {
    let fixture = TempDir::new("engine-plain-own-state");
    let site = fixture.dir("sites/site");
    fixture.write("sites/site/.dev-runner/vite.status", "ready");
    fixture.dir("sites/quiet/.dev-runner");

    let state = scanned(&fixture);

    let environment = environment_of(&state, &site);
    assert!(!environment.sources.global && environment.sources.project);
    assert_eq!(unit_names(&environment), ["vite"]);
    assert!(
        project(&state.snapshot(), &fixture.path("sites/quiet")).is_none(),
        "a state folder publishing nothing makes no project"
    );
}

#[test]
fn state_a_project_cannot_read_is_reported_beside_what_it_can_until_it_reads_again() {
    let fixture = TempDir::new("engine-environment-problem");
    let app = fixture.dir("app");
    register(&fixture, "ab12", &app, std::process::id(), "server");
    let sealed = fixture.write("app/.dev-runner/vite.status", "ready");
    fixture.restrict(&sealed, 0o000);

    let mut state = scanned(&fixture);

    assert_eq!(
        state.snapshot().problems,
        [SourceProblem::new(
            Source::EnvironmentState,
            sealed.clone(),
            io::ErrorKind::PermissionDenied
        )]
    );
    assert_eq!(unit_names(&environment_of(&state, &app)), ["server"]);
    assert!(!state.refresh_environments(None), "the same problem again");

    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(state.refresh_environments(None));
    assert_eq!(state.snapshot().problems, []);
    assert_eq!(
        unit_names(&environment_of(&state, &app)),
        ["server", "vite"]
    );
}

#[test]
fn a_new_environment_status_shows_as_activity() {
    let fixture = TempDir::new("engine-environment");
    let app = crate_repo(&fixture, "app");
    let (engine, snapshots) = start(roots(&fixture), ScanOptions::default());
    wait_for(&snapshots, "the repository", |s| project(s, &app).is_some());

    let written = fixture.write("app/.dev-runner/server.status", b"ready");
    announce(&engine, &written);
    wait_for(&snapshots, "the new unit as activity", |s| {
        project(s, &app).is_some_and(|row| {
            row.activity_source == Some(ActivitySource::Environment)
                && row
                    .environment
                    .as_ref()
                    .is_some_and(|env| env.units.iter().any(|u| u.state == State::Ready))
        })
    });
}
