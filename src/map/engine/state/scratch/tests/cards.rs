//! Which scratch folders earn a card: those holding a build folder or a lot of disk, and those an
//! agent, a dev environment or a server works in.

use super::*;

/// A scratch folder holding the build folders `artifacts` and own files of `own_files`.
fn scratch(artifacts: &[&str], own_files: Measurement) -> Project {
    let session = PathBuf::from("/private/tmp/claude-501/-Users-me/e26ca359");
    let path = session.join("scratchpad/r567");
    Project {
        scratch: Some(ScratchSpace {
            session_folder: session,
            session_id: "e26ca359".to_string(),
            started_in: "me".to_string(),
            started_in_folder: None,
            session: None,
            session_name: None,
        }),
        artifacts: artifacts
            .iter()
            .map(|dir| cargo_target(path.join(dir), Measurement::Pending))
            .collect(),
        own_files,
        ..project_at(&path)
    }
}

#[test]
fn a_scratch_folder_earns_a_card_with_a_build_folder_or_a_large_size_and_others_always_have_one() {
    assert!(earns_a_card(&project_at("/code/app")));
    assert!(earns_a_card(&scratch(&["target"], Measurement::Pending)));
    assert!(earns_a_card(&scratch(&[], measured(LARGE_FOLDER_BYTES))));
    for small in [
        Measurement::Pending,
        Measurement::Measuring,
        measured(LARGE_FOLDER_BYTES - 1),
    ] {
        assert!(!earns_a_card(&scratch(&[], small)), "{small:?}");
    }
}

#[test]
fn a_scratch_folder_earns_a_card_while_a_dev_environment_or_a_server_runs_in_it() {
    let small = scratch(&[], measured(4096));
    let described = Project {
        environment: Some(environment_at(&small.path)),
        ..small.clone()
    };
    let listening = Project {
        listening: vec![listening_in(&small.path, "node", &[5177])],
        ..small.clone()
    };
    assert!(!earns_a_card(&small));
    assert!(earns_a_card(&described), "its servers show on its card");
    assert!(earns_a_card(&listening), "its port shows on its card");
}

#[test]
fn a_scratch_folder_earns_a_card_while_an_agent_stands_on_it_and_not_by_its_session_running() {
    let small = || scratch(&[], measured(4096));
    let session = agent_session("0b1d", AgentStatus::Busy);
    let at_work = Project {
        agents: vec![session.clone()],
        ..small()
    };
    let subagent = SubagentWithParent::new(subagent_at_work("a1"), &session);
    let subagent_at_work = Project {
        subagents: vec![subagent],
        ..small()
    };
    assert!(earns_a_card(&at_work));
    assert!(earns_a_card(&subagent_at_work));

    let mut running = small();
    let space = running.scratch.as_mut().expect("a scratch folder");
    space.session = Some(agent_session("e26ca359", AgentStatus::Busy));
    assert!(!earns_a_card(&running), "its session stands where it works");
}

#[test]
fn each_scratch_folder_has_a_card_once_it_holds_a_build_folder_or_measures_large() {
    let fixture = TempDir::new("engine-scratch-cards");
    let session = fixture.dir("claude-501/-Users-me-vt-foundry/e26ca359/scratchpad");
    let session = session.parent().unwrap().to_path_buf();
    let folder = |rel: &str| session.join(rel);
    fixture.write(
        "claude-501/-Users-me-vt-foundry/e26ca359/scratchpad/r567fixture/target/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    fixture.write(
        "claude-501/-Users-me-vt-foundry/e26ca359/scratchpad/target-567-fix/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    fixture.dir("claude-501/-Users-me-vt-foundry/e26ca359/scratchpad/notes");
    fixture.dir("claude-501/-Users-me-vt-foundry/e26ca359/scratchpad/fixtures");
    fixture.write(
        "claude-501/-Users-me-vt-foundry/e26ca359/tasks/out/CACHEDIR.TAG",
        CACHEDIR_TAG,
    );
    let mut state = state_over(&fixture);

    state.rediscover(None);

    assert_eq!(
        paths(&state),
        [
            folder("scratchpad/r567fixture"),
            folder("scratchpad/target-567-fix"),
            folder("tasks"),
        ],
        "the others wait for a size"
    );
    assert_eq!(
        build_folders(&state, &folder("scratchpad/target-567-fix")),
        [folder("scratchpad/target-567-fix")],
        "a folder cargo tagged is its own one build folder"
    );
    let large_job = Job::OwnFiles(folder("scratchpad/fixtures"));
    let small_job = Job::OwnFiles(folder("scratchpad/notes"));
    assert!(state.apply_usage(&large_job, Ok(usage(LARGE_FOLDER_BYTES))));
    assert!(
        !state.apply_usage(&small_job, Ok(usage(4096))),
        "a folder with no card shows nothing new"
    );
    assert!(paths(&state).contains(&folder("scratchpad/fixtures")));
    assert!(!paths(&state).contains(&folder("scratchpad/notes")));
}
