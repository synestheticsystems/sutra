use std::io;
use std::sync::mpsc;

use notify::event::{DataChange, Flag, MetadataKind, RenameMode};

use super::*;
use crate::map::test_support::{CACHEDIR_TAG, TempDir, at, commit, git, set_mtime};
use crate::map::{ActivitySource, AgentStatus, SourceProblem, agents};

/// A repository at `rel` in `fixture`, with a `target/` Cargo made.
fn crate_with_target(fixture: &TempDir, rel: &str) {
    fixture.write(&format!("{rel}/Cargo.toml"), "[package]\nname = \"app\"\n");
    fixture.write(&format!("{rel}/target/debug/app"), [1; 4096]);
    git(&fixture.path(rel), &["init", "-q", "-b", "main"]);
}

/// An engine on the map of `root`, which is root and home, unstarted and watching no root.
fn engine_over<'a>(
    root: &Path,
    snapshots: &'a SyncSender<Arc<Snapshot>>,
) -> (Engine<'a>, Receiver<Message>) {
    let roots = Roots {
        home: root.to_path_buf(),
        dirs: vec![root.to_path_buf()],
        scratch: None,
        sandboxed: false,
    };
    engine_on_roots(&roots, snapshots)
}

/// An engine on the map of `fixture`'s folder `home`, with its folder `claude-501` for Claude
/// Code's scratch root, unstarted and watching no root.
fn engine_with_scratch<'a>(
    fixture: &TempDir,
    snapshots: &'a SyncSender<Arc<Snapshot>>,
) -> (Engine<'a>, Receiver<Message>) {
    let roots = Roots {
        home: fixture.dir("home"),
        dirs: vec![fixture.path("home")],
        scratch: Some(fixture.path("claude-501")),
        sandboxed: false,
    };
    engine_on_roots(&roots, snapshots)
}

fn engine_on_roots<'a>(
    roots: &Roots,
    snapshots: &'a SyncSender<Arc<Snapshot>>,
) -> (Engine<'a>, Receiver<Message>) {
    let mut state = State::new(roots, &ScanOptions::default());
    state.rediscover(None);
    let (messages, inbox) = mpsc::channel();
    let watches = Watches::over_roots(&[], messages.clone());
    (Engine::new(state, watches, messages, snapshots), inbox)
}

/// An engine on the map of `fixture`.
fn engine_on<'a>(
    fixture: &TempDir,
    snapshots: &'a SyncSender<Arc<Snapshot>>,
) -> (Engine<'a>, Receiver<Message>) {
    engine_over(fixture.root(), snapshots)
}

/// Add a linked worktree of the repository `main` at `folder`, on a branch of that name.
fn add_worktree(main: &Path, folder: &Path) {
    let branch = folder.file_name().unwrap().to_str().unwrap();
    let folder = folder.to_str().unwrap();
    git(main, &["worktree", "add", "-q", "-b", branch, folder]);
}

/// A repository in the folder `home` of `fixture` with a linked worktree at `scratch/wt`, and
/// an engine on the map of `home`. Returns where the worktree is.
fn engine_with_a_worktree_outside_the_root<'a>(
    fixture: &TempDir,
    snapshots: &'a SyncSender<Arc<Snapshot>>,
) -> (Engine<'a>, Receiver<Message>, PathBuf) {
    let worktree = fixture.path("scratch/wt");
    add_worktree(&fixture.repo("home/code/app"), &worktree);
    let (engine, inbox) = engine_over(&fixture.path("home"), snapshots);
    (engine, inbox, worktree)
}

fn watched_outside<'a>(engine: &'a Engine) -> Vec<&'a PathBuf> {
    engine.watches.watched_outside()
}

/// Report `walks` finished measurements to `engine`.
fn take_measurements(engine: &mut Engine, inbox: &Receiver<Message>, walks: usize) {
    for _ in 0..walks {
        let Ok(Message::Measured(measured)) = inbox.recv_timeout(Duration::from_secs(60)) else {
            panic!("a walk should have reported within a minute");
        };
        engine.on_measured(measured);
    }
}

fn renamed(path: PathBuf) -> notify::Result<Event> {
    Ok(Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Any))).add_path(path))
}

/// The build folders the map shows on `project`.
fn build_folders(engine: &Engine, project: &Path) -> Vec<PathBuf> {
    let snapshot = engine.state.snapshot();
    let on_map = snapshot.projects.iter().find(|row| row.path == project);
    let artifacts = &on_map.expect("on the map").artifacts;
    artifacts.iter().map(|dir| dir.path.clone()).collect()
}

/// What `project`'s build folders' sizes add up to on the map.
fn build_bytes(engine: &Engine, project: PathBuf) -> u64 {
    engine
        .state
        .snapshot()
        .projects
        .iter()
        .find(|on_map| on_map.path == project)
        .expect("on the map")
        .measured_build_bytes()
}

/// The private folders `state` still waits for macOS's answer about.
fn awaiting(state: &State) -> Vec<PrivateFolder> {
    let awaiting = state.awaiting_macos().into_iter();
    awaiting.map(|(folder, _)| folder).collect()
}

#[test]
fn the_first_walk_waits_for_macos_until_every_folder_is_answered_its_wait_is_up_or_a_stop() {
    let fixture = TempDir::new("live-wait-for-macos");
    let roots = Roots {
        home: fixture.root().to_path_buf(),
        dirs: vec![fixture.root().to_path_buf()],
        scratch: None,
        sandboxed: false,
    };
    let all = ScanOptions {
        private_folders: PrivateFolder::ALL.into(),
    };
    let mut state = State::new(&roots, &all);
    let (messages, inbox) = mpsc::channel();
    let answered = |folder, answer| Message::MacOsAnswered(folder, answer);

    messages
        .send(answered(PrivateFolder::Documents, MacOsAnswer::LetIn))
        .unwrap();
    let started = Instant::now();
    assert!(wait_for_macos(&mut state, 3, &inbox).is_continue());
    assert!(
        started.elapsed() >= WAIT_FOR_MACOS,
        "two are still unanswered"
    );
    assert_eq!(
        awaiting(&state),
        [PrivateFolder::Desktop, PrivateFolder::Downloads]
    );

    for message in [
        answered(PrivateFolder::Desktop, MacOsAnswer::Refused),
        answered(PrivateFolder::Downloads, MacOsAnswer::LetIn),
        Message::Stop,
    ] {
        messages.send(message).unwrap();
    }
    assert!(wait_for_macos(&mut state, 2, &inbox).is_continue());
    assert_eq!(awaiting(&state), []);
    assert!(
        matches!(inbox.try_recv(), Ok(Message::Stop)),
        "the wait ends with the last answer"
    );

    let mut asking = State::new(&roots, &all);
    messages.send(Message::Stop).unwrap();
    assert!(wait_for_macos(&mut asking, 3, &inbox).is_break());
}

#[test]
fn a_change_below_the_top_of_a_tree_shows_once_every_tree_is_walked_after_lost_events() {
    let fixture = TempDir::new("live-walk-after-loss");
    crate_with_target(&fixture, "a");
    fixture.dir("a/target/debug/deps");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, inbox) = engine_on(&fixture, &snapshots);
    engine.queue_unmeasured(Instant::now());
    engine.dispatch();
    take_measurements(&mut engine, &inbox, 2);
    let before = build_bytes(&engine, fixture.path("a"));

    // Under a folder the tree's top does not look into, and with its event lost.
    fixture.write("a/target/debug/deps/libnew.rlib", [2; 64 * 1024]);
    engine.on_fs_event(Ok(Event::new(EventKind::Other)
        .set_flag(Flag::Rescan)
        .add_path(fixture.root().to_path_buf())));
    let lost_at = Instant::now() + Duration::from_secs(3);
    engine.run_due(lost_at);
    assert_eq!(
        build_bytes(&engine, fixture.path("a")),
        before,
        "the top looks the same, so nothing is walked yet"
    );

    engine.run_due(lost_at + WALK_AFTER_LOST_EVENTS - Duration::from_secs(10));
    assert!(
        inbox.recv_timeout(Duration::from_millis(200)).is_err(),
        "nothing is walked before the time"
    );

    engine.run_due(lost_at + WALK_AFTER_LOST_EVENTS);
    take_measurements(&mut engine, &inbox, 2);
    assert!(build_bytes(&engine, fixture.path("a")) >= before + 64 * 1024);
    assert!(
        engine.lost_events.is_none(),
        "made good until events go missing again"
    );
}

#[test]
fn a_tree_walked_since_the_latest_loss_is_not_walked_again_for_it() {
    let fixture = TempDir::new("live-walked-since-loss");
    crate_with_target(&fixture, "a");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, inbox) = engine_on(&fixture, &snapshots);
    engine.queue_unmeasured(Instant::now());
    engine.dispatch();
    take_measurements(&mut engine, &inbox, 2);
    let long_ago = Instant::now() - Duration::from_secs(60);
    engine.lost_events = Some(LostEvents {
        first: long_ago,
        latest: long_ago,
    });

    assert!(engine.walk_trees_after_lost_events(long_ago + WALK_AFTER_LOST_EVENTS));
    engine.dispatch();

    assert!(inbox.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn a_walk_that_began_before_the_latest_loss_does_not_count_though_it_ended_after() {
    let fixture = TempDir::new("live-walk-across-loss");
    crate_with_target(&fixture, "a");
    fixture.dir("a/target/debug/deps");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, inbox) = engine_on(&fixture, &snapshots);
    engine.queue_unmeasured(Instant::now());
    engine.dispatch();
    // Both walks have passed every folder, and neither has reported yet.
    let walks: Vec<Message> = (0..2)
        .map(|_| inbox.recv_timeout(Duration::from_secs(60)).unwrap())
        .collect();

    fixture.write("a/target/debug/deps/libnew.rlib", [2; 64 * 1024]);
    let lost_at = Instant::now();
    engine.lost_events = Some(LostEvents {
        first: lost_at,
        latest: lost_at,
    });
    for walk in walks {
        let Message::Measured(measured) = walk else {
            panic!("nothing but walks reports here");
        };
        engine.on_measured(measured);
    }
    let before = build_bytes(&engine, fixture.path("a"));

    assert!(engine.walk_trees_after_lost_events(lost_at + WALK_AFTER_LOST_EVENTS));
    engine.dispatch();
    take_measurements(&mut engine, &inbox, 2);

    assert!(build_bytes(&engine, fixture.path("a")) >= before + 64 * 1024);
}

#[test]
fn a_folder_just_cleaned_is_measured_at_once_and_one_off_the_map_is_not() {
    let fixture = TempDir::new("live-after-clean");
    crate_with_target(&fixture, "a");
    fixture.write("a/target/debug/big", [1; 64 * 1024]);
    fixture.write("elsewhere/target/debug/big", [1; 64 * 1024]);
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, inbox) = engine_on(&fixture, &snapshots);
    engine.queue_unmeasured(Instant::now());
    engine.dispatch();
    take_measurements(&mut engine, &inbox, 2);
    let before = build_bytes(&engine, fixture.path("a"));

    fs::remove_file(fixture.path("a/target/debug/big")).unwrap();
    engine.on_cleaned(vec![fixture.path("a/target")], Instant::now());
    take_measurements(&mut engine, &inbox, 1);
    assert!(build_bytes(&engine, fixture.path("a")) + 64 * 1024 <= before);

    engine.on_cleaned(vec![fixture.path("elsewhere/target")], Instant::now());
    assert!(inbox.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn a_folder_renamed_into_a_project_has_its_build_folders_looked_for() {
    let fixture = TempDir::new("live-rename-in-project");
    crate_with_target(&fixture, "a");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);
    let now = Instant::now();
    let later = now + Duration::from_secs(3_600);

    fixture.dir("a/web/node_modules");
    engine.on_fs_event(renamed(fixture.path("a/web")));
    assert_eq!(engine.timers.layout.take_due(later), [fixture.path("a")]);

    fixture.write("a/saved.txt", "a file saved by a rename");
    engine.on_fs_event(renamed(fixture.path("a/saved.txt")));
    assert!(
        engine.timers.layout.take_due(later).is_empty(),
        "saving a file is no reason to look"
    );

    engine.on_fs_event(renamed(fixture.path("a/moved-away")));
    assert_eq!(
        engine.timers.layout.take_due(later),
        [fixture.path("a")],
        "what was renamed away may have been a folder"
    );
}

#[test]
fn a_repository_moved_into_a_root_joins_the_map_with_no_git_event() {
    let fixture = TempDir::new("live-rename-repo");
    crate_with_target(&fixture, "a");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);

    crate_with_target(&fixture, "moved/here");
    engine.on_fs_event(renamed(fixture.path("moved")));
    engine.run_due(Instant::now() + Duration::from_secs(3));

    assert!(
        engine
            .state
            .project_paths()
            .contains(&fixture.path("moved/here"))
    );
}

#[test]
fn a_folder_moved_out_of_the_roots_takes_the_projects_in_it_off_the_map() {
    let fixture = TempDir::new("live-moved-away");
    let elsewhere = TempDir::new("live-moved-away-dest");
    crate_with_target(&fixture, "group/app");
    crate_with_target(&fixture, "stays");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);

    fs::rename(fixture.path("group"), elsewhere.path("group")).unwrap();
    engine.on_fs_event(renamed(fixture.path("group")));
    engine.run_due(Instant::now() + Duration::from_secs(3));

    let on_map = engine.state.project_paths();
    assert!(!on_map.contains(&fixture.path("group/app")));
    assert!(on_map.contains(&fixture.path("stays")));
}

#[test]
fn a_folder_moved_out_of_a_project_takes_the_worktree_in_it_off_the_map() {
    let fixture = TempDir::new("live-moved-out-of-project");
    let elsewhere = TempDir::new("live-moved-out-of-project-dest");
    crate_with_target(&fixture, "a");
    let main = fixture.path("a");
    commit(&main, "README");
    let worktree = fixture.path("a/.claude/worktrees/wt");
    add_worktree(&main, &worktree);
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);
    assert!(engine.state.project_paths().contains(&worktree));

    fs::rename(fixture.path("a/.claude"), elsewhere.path("moved")).unwrap();
    engine.on_fs_event(renamed(fixture.path("a/.claude")));
    engine.run_due(Instant::now() + Duration::from_secs(3));

    assert!(!engine.state.project_paths().contains(&worktree));
}

#[test]
fn a_project_moved_out_of_the_roots_leaves_the_map() {
    let fixture = TempDir::new("live-project-moved-away");
    let elsewhere = TempDir::new("live-project-moved-away-dest");
    crate_with_target(&fixture, "a");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);

    fs::rename(fixture.path("a"), elsewhere.path("a")).unwrap();
    engine.on_fs_event(renamed(fixture.path("a")));
    engine.run_due(Instant::now() + Duration::from_secs(3));

    assert!(!engine.state.project_paths().contains(&fixture.path("a")));
}

#[test]
fn a_rename_at_a_path_holding_no_project_starts_no_rediscovery() {
    let fixture = TempDir::new("live-rename-away");
    crate_with_target(&fixture, "a");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);

    engine.on_fs_event(renamed(fixture.path("renamed-away")));

    assert_eq!(engine.timers.rediscover.next_due(), None);
    assert_eq!(engine.timers.moved_in.next_due(), None);
}

#[test]
fn a_file_renamed_outside_every_project_starts_no_rediscovery() {
    let fixture = TempDir::new("live-rename-file");
    crate_with_target(&fixture, "a");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);

    fixture.write("notes.txt", "a file");
    engine.on_fs_event(renamed(fixture.path("notes.txt")));

    assert_eq!(engine.timers.moved_in.next_due(), None);
}

#[test]
fn a_project_outside_the_roots_is_watched_read_again_once_and_left_when_it_leaves_the_map() {
    let fixture = TempDir::new("live-watch-outside");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox, worktree) =
        engine_with_a_worktree_outside_the_root(&fixture, &snapshots);

    assert_eq!(watched_outside(&engine), [&worktree]);
    assert!(
        engine.timers.rediscover.next_due().is_some(),
        "what changed before the watch began is read again"
    );

    engine.run_due(Instant::now() + Duration::from_secs(3));
    assert_eq!(
        engine.timers.rediscover.next_due(),
        None,
        "the same folders again begin no watch, so this ends"
    );
    assert_eq!(watched_outside(&engine), [&worktree]);

    let main = fixture.path("home/code/app");
    git(&main, &["worktree", "remove", worktree.to_str().unwrap()]);
    engine.on_fs_event(renamed(main.join(".git/worktrees/wt")));
    engine.run_due(Instant::now() + Duration::from_secs(3));
    assert!(watched_outside(&engine).is_empty());
}

#[test]
fn a_new_project_outside_the_roots_is_watched_when_rediscovery_finds_it() {
    let fixture = TempDir::new("live-watch-found");
    let main = fixture.repo("home/code/app");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_over(&fixture.path("home"), &snapshots);
    assert!(watched_outside(&engine).is_empty());

    let worktree = fixture.path("scratch/wt");
    add_worktree(&main, &worktree);
    engine.on_fs_event(renamed(main.join(".git/worktrees/wt")));
    engine.run_due(Instant::now() + Duration::from_secs(3));

    assert_eq!(watched_outside(&engine), [&worktree]);
}

#[test]
fn a_folder_gone_before_its_watch_began_is_reported_and_read_again_until_it_leaves_the_map() {
    let fixture = TempDir::new("live-watch-gone");
    let main = fixture.repo("home/code/app");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_over(&fixture.path("home"), &snapshots);
    let worktree = fixture.path("scratch/wt");
    add_worktree(&main, &worktree);
    engine.state.rediscover(None);

    fs::remove_dir_all(&worktree).unwrap();
    let now = Instant::now();
    engine.keep_watches_current(now);
    assert_eq!(
        engine.state.snapshot().problems,
        [SourceProblem::new(
            Source::Watcher,
            worktree.clone(),
            io::ErrorKind::NotFound
        )]
    );

    engine.run_due(now + Duration::from_secs(3));
    assert!(!engine.state.project_paths().contains(&worktree));
    assert_eq!(engine.state.snapshot().problems, []);
}

#[test]
fn a_folder_around_a_watched_project_moving_takes_the_project_off_the_map_at_the_next_tick() {
    let fixture = TempDir::new("live-watch-around");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox, worktree) =
        engine_with_a_worktree_outside_the_root(&fixture, &snapshots);
    engine.timers.rediscover.clear();

    fs::rename(fixture.path("scratch"), fixture.path("moved")).unwrap();
    let tick = Instant::now() + Duration::from_secs(3);
    engine.run_due(tick);
    assert!(watched_outside(&engine).is_empty());
    engine.run_due(tick + Duration::from_secs(3));

    assert!(!engine.state.project_paths().contains(&worktree));
}

#[test]
fn a_project_gone_from_the_map_hands_its_build_folders_back_to_the_project_around_it() {
    let fixture = TempDir::new("live-hand-back");
    crate_with_target(&fixture, "a");
    let web = fixture.dir("a/web");
    let node_modules = fixture.dir("a/web/node_modules");
    let registration = fixture.write(
        ".dev-runner/ab12",
        format!("DIR={}\nPID={}\n", web.display(), std::process::id()),
    );
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);
    let target = fixture.path("a/target");
    let a = fixture.path("a");
    assert_eq!(build_folders(&engine, &a), std::slice::from_ref(&target));
    assert_eq!(
        build_folders(&engine, &web),
        std::slice::from_ref(&node_modules)
    );

    fs::remove_file(&registration).unwrap();
    let removed = Event::new(EventKind::Remove(RemoveKind::File)).add_path(registration);
    engine.on_fs_event(Ok(removed));
    let now = Instant::now() + Duration::from_secs(1);
    engine.run_due(now);
    assert!(!engine.state.project_paths().contains(&web));
    engine.run_due(now + Duration::from_secs(3));

    assert_eq!(build_folders(&engine, &a), [target, node_modules]);
}

#[test]
fn a_lockfile_changed_alone_counts_as_activity_and_has_the_project_walked() {
    let fixture = TempDir::new("live-lockfile");
    crate_with_target(&fixture, "a");
    let lockfile = fixture.write("a/Cargo.lock", "# changed by cargo update");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);

    let change = EventKind::Modify(ModifyKind::Data(DataChange::Content));
    engine.on_fs_event(Ok(Event::new(change).add_path(lockfile)));

    let snapshot = engine.state.snapshot();
    assert_eq!(
        snapshot.projects[0].activity_source,
        Some(ActivitySource::FileChange)
    );
    let later = Instant::now() + Duration::from_secs(3_600);
    assert_eq!(engine.timers.own_files.take_due(later), [fixture.path("a")]);
}

#[test]
fn a_file_icloud_drive_brings_back_leaves_the_activity_where_it_was_and_a_write_moves_it() {
    let fixture = TempDir::new("live-download");
    crate_with_target(&fixture, "a");
    let a = fixture.path("a");
    commit(&a, "README.md");
    let committed = at(1_700_000_000);
    for file in [".git/index", ".git/logs/HEAD"] {
        set_mtime(&a.join(file), committed);
    }
    let readme = a.join("README.md");
    set_mtime(&readme, at(1_690_000_000));
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);
    let activity = |engine: &Engine| {
        let snapshot = engine.state.snapshot();
        (
            snapshot.projects[0].last_activity,
            snapshot.projects[0].activity_source,
        )
    };
    let as_committed = (Some(committed), Some(ActivitySource::Git));
    assert_eq!(activity(&engine), as_committed);

    // What FSEvents reports when an app opens the file and iCloud Drive brings it back.
    let download = [
        ModifyKind::Data(DataChange::Content),
        ModifyKind::Metadata(MetadataKind::Extended),
        ModifyKind::Metadata(MetadataKind::Ownership),
        ModifyKind::Metadata(MetadataKind::Any),
    ];
    for kind in download {
        engine.on_fs_event(Ok(
            Event::new(EventKind::Modify(kind)).add_path(readme.clone())
        ));
    }
    assert_eq!(activity(&engine), as_committed);

    fixture.write("a/README.md", "edited");
    let written = EventKind::Modify(ModifyKind::Data(DataChange::Content));
    engine.on_fs_event(Ok(Event::new(written).add_path(readme)));
    let edited = activity(&engine);
    assert_eq!(edited.1, Some(ActivitySource::FileChange));

    // Another old file brought back after the edit leaves the edit as the latest work.
    let notes = fixture.write("a/NOTES.md", "evicted long ago");
    set_mtime(&notes, at(1_690_000_000));
    for kind in download {
        engine.on_fs_event(Ok(
            Event::new(EventKind::Modify(kind)).add_path(notes.clone())
        ));
    }
    assert_eq!(activity(&engine), edited);
}

#[test]
fn lost_events_mark_changed_only_the_trees_whose_top_changed() {
    let fixture = TempDir::new("live-lost-events");
    crate_with_target(&fixture, "a");
    crate_with_target(&fixture, "b");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, inbox) = engine_on(&fixture, &snapshots);
    engine.queue_unmeasured(Instant::now());
    engine.dispatch();
    // Each project's own files and its target.
    take_measurements(&mut engine, &inbox, 4);

    fixture.write("a/target/debug/libnew.rlib", [2; 4096]);
    fixture.write("b/README.md", "new");
    let notice = || {
        Event::new(EventKind::Other)
            .set_flag(Flag::Rescan)
            .add_path(fixture.root().to_path_buf())
    };
    engine.on_fs_event(Ok(notice()));
    // Past the two seconds a notice waits for the rest of its burst.
    let now = Instant::now() + Duration::from_secs(3);
    engine.run_due(now);
    let later = now + Duration::from_secs(3_600);
    assert_eq!(
        engine.timers.artifacts.take_due(later),
        [fixture.path("a/target")]
    );
    assert_eq!(engine.timers.own_files.take_due(later), [fixture.path("b")]);

    engine.remeasure_changed_under(&[fixture.path("elsewhere")], now);
    assert!(engine.timers.artifacts.take_due(later).is_empty());
    assert!(engine.timers.own_files.take_due(later).is_empty());

    engine.on_fs_event(Ok(notice()));
    assert_eq!(
        engine.timers.rescan.next_due(),
        Some(now + Duration::from_secs(60)),
        "the next recovery waits a minute after the last"
    );
}

#[test]
fn the_scratch_root_is_watched_once_claude_code_makes_it_and_read_again_then() {
    let fixture = TempDir::new("live-scratch-root-appears");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_with_scratch(&fixture, &snapshots);
    let scratch_root = fixture.path("claude-501");
    let now = Instant::now();
    let watched = |engine: &Engine| engine.watches.roots().contains(&scratch_root);

    engine.run_due(now + Duration::from_secs(3));
    assert!(!watched(&engine));
    assert_eq!(engine.timers.rediscover.next_due(), None, "no root yet");

    fixture.dir("claude-501/-code-app/e26ca359/scratchpad");
    engine.run_due(now + Duration::from_secs(6));
    assert!(watched(&engine));
    assert!(engine.timers.rediscover.next_due().is_some());

    fs::remove_dir_all(&scratch_root).unwrap();
    engine.run_due(now + Duration::from_secs(9));
    assert!(
        !watched(&engine),
        "a stream over a folder that went hears nothing"
    );
}

/// Whether the map shows a card for the project at `path`.
fn carded(engine: &Engine, path: &Path) -> bool {
    let snapshot = engine.state.snapshot();
    snapshot.projects.iter().any(|project| project.path == path)
}

#[test]
fn a_build_folder_made_in_a_scratch_folder_while_its_events_went_missing_is_found() {
    let fixture = TempDir::new("live-scratch-lost-events");
    let folder = fixture.dir("claude-501/-code-app/e26ca359/scratchpad/probe");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_with_scratch(&fixture, &snapshots);
    assert!(engine.state.project_paths().contains(&folder));
    assert!(
        !carded(&engine, &folder),
        "no card while it holds no build folder"
    );

    fixture.write(
        "claude-501/-code-app/e26ca359/scratchpad/probe/out-567/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    engine.on_fs_event(Ok(Event::new(EventKind::Other)
        .set_flag(Flag::Rescan)
        .add_path(fixture.path("claude-501"))));
    let now = Instant::now() + Duration::from_secs(3);
    engine.run_due(now);
    assert!(
        !carded(&engine, &folder),
        "searched again once the folder is quiet"
    );
    engine.run_due(now + Duration::from_secs(11));

    assert_eq!(build_folders(&engine, &folder), [folder.join("out-567")]);
}

#[test]
fn a_folder_made_in_a_scratch_folder_has_it_searched_through_once_its_folders_settle() {
    let fixture = TempDir::new("live-scratch-layout");
    let folder = fixture.dir("claude-501/-code-app/e26ca359/scratchpad/r567");
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_with_scratch(&fixture, &snapshots);
    let now = Instant::now();

    let made = fixture.write(
        "claude-501/-code-app/e26ca359/scratchpad/r567/target/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    let target = made.parent().unwrap().to_path_buf();
    engine.on_fs_event(Ok(
        Event::new(EventKind::Create(CreateKind::Folder)).add_path(target)
    ));

    assert!(
        engine
            .timers
            .layout
            .take_due(now + Duration::from_secs(5))
            .is_empty()
    );
    assert_eq!(
        engine
            .timers
            .scratch_layout
            .take_due(now + Duration::from_secs(11)),
        [folder]
    );
}

#[test]
fn a_folder_made_at_the_top_of_a_sessions_scratch_joins_the_map_once_its_folders_are_listed() {
    let fixture = TempDir::new("live-scratch-folders");
    let session = fixture.dir("claude-501/-code-app/e26ca359/scratchpad");
    let session = session.parent().unwrap().to_path_buf();
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_with_scratch(&fixture, &snapshots);
    let now = Instant::now();
    let event = |kind, path: &Path| Ok(Event::new(kind).add_path(path.to_path_buf()));

    let notes = fixture.write("claude-501/-code-app/e26ca359/scratchpad/notes.md", "");
    engine.on_fs_event(event(EventKind::Create(CreateKind::File), &notes));
    assert_eq!(
        engine.timers.scratch_folders.next_due(),
        None,
        "a file at the top is no folder coming"
    );

    fixture.write(
        "claude-501/-code-app/e26ca359/scratchpad/target-567/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    let made = session.join("scratchpad/target-567");
    engine.on_fs_event(event(EventKind::Create(CreateKind::Folder), &made));
    engine.run_due(now + Duration::from_secs(3));

    assert!(carded(&engine, &made));
    assert_eq!(build_folders(&engine, &made), std::slice::from_ref(&made));

    fs::remove_dir_all(&made).unwrap();
    engine.on_fs_event(event(EventKind::Remove(RemoveKind::Folder), &made));
    engine.run_due(now + Duration::from_secs(6));
    assert!(
        !engine.state.project_paths().contains(&made),
        "gone with it"
    );
}

#[test]
fn a_subagent_set_to_work_in_an_idle_session_shows_within_a_second() {
    let fixture = TempDir::new("live-subagent");
    let me = std::process::id();
    let now = SystemTime::now();
    let ms = |at: SystemTime| {
        at.duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    };
    let registry = serde_json::json!({
        "pid": me, "sessionId": "0b1d", "cwd": "/elsewhere", "status": "idle",
        "startedAt": ms(now - Duration::from_secs(600)), "updatedAt": ms(now),
    });
    fixture.write(&format!(".claude/sessions/{me}.json"), registry.to_string());
    let (snapshots, _published) = mpsc::sync_channel(1);
    let (mut engine, _inbox) = engine_on(&fixture, &snapshots);
    let status = |engine: &Engine| engine.state.snapshot().unplaced_agents[0].status;
    assert_eq!(status(&engine), AgentStatus::Idle);

    let transcript = agents::write_test_subagent_at_work(
        fixture.root(),
        "/elsewhere",
        "0b1d",
        "a1",
        "/elsewhere",
        now,
    );
    let written = EventKind::Modify(ModifyKind::Data(DataChange::Content));
    engine.on_fs_event(Ok(Event::new(written).add_path(transcript)));
    // Past the burst's quiet, and before the tick that reads every agent anyway.
    engine.run_due(Instant::now() + Duration::from_millis(500));

    assert_eq!(status(&engine), AgentStatus::Busy);
}
