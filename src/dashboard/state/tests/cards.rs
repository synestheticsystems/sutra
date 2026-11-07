//! What the user does to a card: a clean named by its folder, a clean that asks first, and the
//! environment terminated only while the map still shows it alive.

use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command};

use super::*;
use crate::dashboard::cleaning::tests::prepared_at;
use crate::map::clean::Sharing;
use crate::map::test_support::{cargo_target, environment_at, measured, snapshot_of};

/// Choose Clean on the build folder at `folder`, as a card names it.
fn clean(state: &mut State, folder: &Path) {
    let folder = format::path_text(folder);
    state.perform(Action::Clean { folder }, now());
}

#[test]
fn a_clean_chosen_on_a_card_names_its_folder_and_one_gone_from_the_map_does_nothing() {
    let fixture = TempDir::new("state-clean-by-name");
    fixture.package("app");
    let target = fixture.dir("app/target");
    let (mut state, _inbox) = dashboard(&fixture);
    let app = active_at(
        fixture.path("app"),
        vec![cargo_target(target.clone(), measured(5_000))],
    );
    state.show(Arc::new(snapshot_of(vec![app])), now());

    clean(&mut state, &fixture.path("gone/target"));
    let contents = state.contents(now());
    assert_eq!(
        card_at(&contents, &fixture.path("app")).tray,
        None,
        "nothing at work"
    );

    clean(&mut state, &target);
    let contents = state.contents(now());
    let tray = card_at(&contents, &fixture.path("app")).tray.clone();
    assert_eq!(
        tray.and_then(|tray| tray.disk?.progress),
        Some(0.0),
        "the card shows it at once"
    );
}

#[test]
fn clean_all_on_a_sessions_line_starts_each_build_folder_of_its_scratch_that_a_card_would_offer() {
    let fixture = TempDir::new("state-clean-scratch");
    let (mut state, _inbox) = dashboard(&fixture);
    // A scratch folder of the session `session_id` holding a build folder of `bytes`.
    let scratch = |session_id: &str, folder: &str, bytes: u64| {
        let session_folder = fixture.path(&format!("claude-501/-code-app/{session_id}"));
        let path = session_folder.join("scratchpad").join(folder);
        let target = cargo_target(path.join("target"), measured(bytes));
        Project {
            scratch: Some(crate::map::ScratchSpace {
                session_folder,
                session_id: session_id.to_string(),
                started_in: "app".to_string(),
                session: None,
                session_name: None,
            }),
            ..active_at(path, vec![target])
        }
    };
    let map = snapshot_of(vec![
        scratch("e26ca359", "r567", 5_000_000),
        scratch("e26ca359", "tiny", 10),
        scratch("0b1d4f00", "other", 5_000_000),
    ]);
    state.show(Arc::new(map.clone()), now());

    let session_folder = format::path_text(&fixture.path("claude-501/-code-app/e26ca359"));
    state.perform(Action::CleanScratch { session_folder }, now());

    let cleaning: Vec<bool> = map
        .projects
        .iter()
        .map(|project| state.cleaning.state_of(&project.artifacts[0]).is_some())
        .collect();
    assert_eq!(
        cleaning,
        [true, false, false],
        "neither one too small to be worth it, nor another session's"
    );
}

#[test]
fn a_clean_chosen_on_a_folder_whose_name_is_not_text_starts_on_it_and_not_on_one_shown_alike() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    // Neither folder is made, as APFS refuses a name that is not text: the clean only begins.
    let fixture = TempDir::new("state-clean-not-text");
    let odd = fixture.root().join(OsStr::from_bytes(b"\xffapp"));
    let shown_alike = fixture.path("\u{fffd}app");
    let (mut state, _inbox) = dashboard(&fixture);
    let with_target = |project: &Path| {
        let target = cargo_target(project.join("target"), measured(5_000));
        active_at(project.to_path_buf(), vec![target])
    };
    let map = snapshot_of(vec![with_target(&shown_alike), with_target(&odd)]);
    state.show(Arc::new(map), now());

    clean(&mut state, &odd.join("target"));
    let contents = state.contents(now());
    let progress = |project: &Path| {
        let tray = card_at(&contents, project).tray.as_ref();
        tray.and_then(|tray| tray.disk.as_ref()?.progress)
    };
    assert_eq!(progress(&odd), Some(0.0), "the folder the card named");
    assert_eq!(
        progress(&shown_alike),
        None,
        "nor the one whose name shows alike"
    );
}

#[test]
fn a_clean_reaching_other_projects_asks_once_and_the_answer_decides_it() {
    let fixture = TempDir::new("state-clean-asks");
    fixture.write("app/target/debug/big", [1; 64 * 1024]);
    let target = fixture.path("app/target");
    let project = fixture.path("app");
    let (mut state, inbox) = dashboard(&fixture);
    let app = active_at(
        project.clone(),
        vec![cargo_target(target.clone(), measured(64 * 1024))],
    );
    state.show(Arc::new(snapshot_of(vec![app])), now());
    let reaching = || {
        let mut prepared = prepared_at(&target);
        prepared.sharing = Sharing {
            projects: vec!["fwd".to_string()],
            ..Sharing::default()
        };
        prepared
    };
    let ask = |state: &mut State| {
        clean(state, &target);
        let prepared = Report::Prepared {
            clicked: vec![target.clone()],
            prepared: Some(vec![Ok(reaching())]),
        };
        state.on_report(prepared, now());
        state.contents(now()).dialog.expect("a question")
    };

    let question = ask(&mut state);
    assert_eq!(question.title, "Clean target in app?");
    assert_eq!(question.action.as_deref(), Some("Clean"));
    assert!(
        question.destructive,
        "its yes deletes other projects' builds"
    );
    let contents = state.contents(now());
    let progress = card_at(&contents, &project)
        .tray
        .as_ref()
        .and_then(|tray| tray.disk.as_ref()?.progress);
    assert_eq!(
        progress,
        Some(0.0),
        "the card shows the clean while the question is open"
    );

    answer(&mut state, &question, false);
    let contents = state.contents(now());
    assert_eq!(contents.dialog, None);
    assert_eq!(
        card_at(&contents, &project).tray,
        None,
        "a no shows the folder as it was"
    );
    assert!(target.join("debug/big").exists());

    let question = ask(&mut state);
    answer(&mut state, &question, true);
    assert!(state.is_deleting(), "a yes starts it");
    drive_until(&mut state, &inbox, "the clean to end", |state| {
        !state.is_deleting()
    });
    assert!(!target.join("debug/big").exists());
    assert_eq!(state.contents(now()).dialog, None, "it went as asked");
}

/// A `sleep` the test signals or kills, and reaps.
fn sleeper() -> Child {
    Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("start a process")
}

/// Whether `child` ended of a SIGTERM within a few seconds.
fn terminated(child: &mut Child) -> bool {
    for _ in 0..50 {
        if let Some(status) = child.try_wait().expect("look at the child") {
            return status.signal() == Some(Signal::SIGTERM as i32);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn terminate_signals_only_a_live_environment_the_map_still_shows() {
    let (mut state, _inbox) = dashboard_at(Path::new("/Users/dev"), Path::new("/code"), None);
    let mut shown = sleeper();
    let mut freed = sleeper();
    let environment = |pid| crate::model::Environment {
        pid,
        alive: true,
        ..environment_at("/code/app")
    };
    state.show(
        Arc::new(snapshot_of(vec![Project {
            environment: Some(environment(shown.id())),
            ..project_at("/code/app")
        }])),
        now(),
    );

    state.perform(Action::TerminateEnvironment { pid: freed.id() }, now());
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        freed.try_wait().unwrap().is_none(),
        "a pid off the map gets no signal"
    );
    freed.kill().unwrap();
    freed.wait().unwrap();

    state.perform(Action::TerminateEnvironment { pid: shown.id() }, now());
    assert!(terminated(&mut shown));
}
