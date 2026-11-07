//! The engine's maps and what the window shows of them: the idle offer while the map is live, the
//! title, a replaced engine's maps dropped, and a folder granted in place of none.

use super::*;
use crate::map::test_support::{agent_session, cargo_target, measured, snapshot_of};
use crate::map::{AgentStatus, Source, SourceProblem};

#[test]
fn nothing_is_offered_for_cleaning_once_the_engine_has_stopped_or_a_source_goes_unread() {
    let map = snapshot_of(vec![Project {
        artifacts: vec![cargo_target("/code/app/target", measured(1 << 30))],
        ..project_at("/code/app")
    }]);
    let (mut state, _inbox) = dashboard_at(Path::new("/Users/dev"), Path::new("/code"), None);
    let now = now();
    assert!(!state.can_tell_what_is_idle(), "no map yet");

    state.snapshot = Some(Arc::new(map.clone()));
    assert!(state.idle_offer(now).is_some());
    assert!(state.can_tell_what_is_idle());

    state.engine_stopped = true;
    assert!(state.idle_offer(now).is_none());
    assert!(!state.can_tell_what_is_idle());
    assert!(state.live_map().is_none());

    state.engine_stopped = false;
    state.snapshot = Some(Arc::new(Snapshot {
        problems: vec![SourceProblem::new(
            Source::CodexDatabase,
            PathBuf::from("/Users/dev/.codex/state_5.sqlite"),
            std::io::ErrorKind::PermissionDenied,
        )],
        ..map
    }));
    assert!(state.idle_offer(now).is_none());
    assert!(!state.can_tell_what_is_idle());
}

#[test]
fn the_contents_count_the_agents_waiting_for_the_title_and_the_dock() {
    let (mut state, _inbox) = dashboard_at(Path::new("/Users/dev"), Path::new("/code"), None);
    assert_eq!(state.contents(now()).list, List::Scanning);

    let waiting = Project {
        agents: vec![agent_session("s1", AgentStatus::Waiting)],
        ..project_at("/code/app")
    };
    state.show(Arc::new(snapshot_of(vec![waiting])), now());

    let contents = state.contents(now());
    assert_eq!(contents.title, "Sutra \u{00b7} 1 waiting");
    assert_eq!(contents.dock_badge.as_deref(), Some("1"));
    assert_eq!(contents.stopped, None);
    assert!(card_at(&contents, Path::new("/code/app")).waiting);
}

#[test]
fn once_the_running_engine_stops_the_window_keeps_its_last_map_and_claims_no_news() {
    let fixture = TempDir::new("state-engine-stopped");
    let (mut state, inbox) = dashboard(&fixture);
    state.start_engine();
    let running = match inbox.recv_timeout(Duration::from_secs(30)) {
        Ok(Event::Snapshot(engine, first)) => {
            // The first map only primes the notifier: waits found in it are not news.
            state.on_snapshot(engine, first, now());
            engine
        }
        _ => panic!("the engine's first map"),
    };
    // The engine's last map, as if it had found an agent waiting a moment ago.
    let waiting = Project {
        agents: vec![agent_session("s1", AgentStatus::Waiting)],
        ..project_at("/code/app")
    };
    state.on_snapshot(running, Arc::new(snapshot_of(vec![waiting])), now());
    assert!(state.has_pending_waits());

    state.on_engine_stopped(running);

    let contents = state.contents(now());
    assert_eq!(contents.title, "Sutra \u{00b7} stopped");
    assert_eq!(contents.dock_badge.as_deref(), Some("!"));
    assert_eq!(
        contents.stopped.as_deref(),
        Some("Updates stopped, so this is how things last stood. Quit and reopen Sutra.")
    );
    assert!(
        card_at(&contents, Path::new("/code/app")).waiting,
        "the last map stays"
    );
    assert!(!state.has_pending_waits(), "no wait is announced after it");
    state.stop();
}

#[test]
fn a_map_from_an_engine_since_replaced_and_word_of_its_end_are_dropped() {
    let fixture = TempDir::new("state-engine-replaced");
    let (mut state, inbox) = dashboard(&fixture);
    state.start_engine();
    let (old, old_map) = match inbox.recv_timeout(Duration::from_secs(30)) {
        Ok(Event::Snapshot(engine, map)) => (engine, map),
        _ => panic!("the first engine's first map"),
    };

    let folder = "Desktop".to_string();
    state.perform(
        Action::WatchPrivateFolder {
            folder,
            watch: true,
        },
        now(),
    );
    state.on_snapshot(old, old_map, now());
    assert!(
        state.snapshot.is_none(),
        "the old engine's map is not shown"
    );

    let (mut old_ended, mut new_map) = (false, false);
    while !(old_ended && new_map) {
        match inbox.recv_timeout(Duration::from_secs(30)) {
            Ok(Event::EngineStopped(engine)) => {
                assert_eq!(engine, old, "only the old engine has stopped");
                state.on_engine_stopped(engine);
                assert!(!state.engine_stopped, "the running engine goes on");
                old_ended = true;
            }
            Ok(Event::Snapshot(engine, map)) => {
                assert_ne!(engine, old);
                state.on_snapshot(engine, map, now());
                new_map = state.snapshot.is_some();
            }
            _ => panic!("the old engine's end and the new engine's map"),
        }
    }
    state.stop();
}

#[test]
fn a_folder_granted_is_watched_in_place_of_none_and_its_first_map_awaited() {
    let fixture = TempDir::new("state-folder-granted");
    let (events, _inbox) = mpsc::channel();
    let roots = Roots {
        home: fixture.root().to_path_buf(),
        dirs: Vec::new(),
        scratch: None,
        sandboxed: false,
    };
    let mut notifier = Notifier::new(|_, _| {});
    notifier.toggle_global_mute();
    let mut state = State::new(
        roots,
        None,
        SavedSettings::default(),
        None,
        notifier,
        events,
    );
    assert!(matches!(state.contents(now()).list, List::NoFolder { .. }));

    let folder = format::path_text(fixture.root());
    state.perform(Action::FolderGranted { folder }, now());

    assert_eq!(state.contents(now()).list, List::Scanning);
    assert_eq!(state.roots, [fixture.root()]);
    assert!(state.watching.is_some());
    state.stop();
}
