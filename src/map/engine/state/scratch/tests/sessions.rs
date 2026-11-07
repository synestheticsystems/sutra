//! The session each scratch folder holds while it runs, and the name the folder's space goes by:
//! the session's own, the one its transcript recorded, or where it started and its id.

use super::*;

#[test]
fn an_unnamed_sessions_scratch_goes_by_where_the_session_started_and_its_id() {
    let fixture = TempDir::new("engine-scratch-names");
    let key = key_of(&fixture.dir("home/vt/greyorange-foundry"));
    fixture.write(
        &format!("claude-501/{key}/e26ca359-da00/scratchpad/target-567/CACHEDIR.TAG"),
        CACHEDIR_TAG,
    );
    let mut state = state_over(&fixture);

    state.rediscover(None);

    let snapshot = state.snapshot();
    let [card] = snapshot.projects.as_slice() else {
        panic!("one card: {:?}", snapshot.projects);
    };
    assert_eq!(card.name, "target-567");
    assert_eq!(session_names(&state), ["greyorange-foundry e26ca359"]);
    let space = card.scratch.as_ref().expect("a scratch folder");
    assert_eq!(
        space.place_of(&card.path),
        Path::new("scratchpad/target-567")
    );
    assert_eq!(space.session_id, "e26ca359-da00");
}

#[test]
fn a_session_that_ended_before_the_map_saw_it_goes_by_the_name_its_transcript_recorded() {
    let fixture = TempDir::new("engine-scratch-transcript-name");
    let key = key_of(&fixture.dir("home/vt/foundry"));
    let scratchpad = format!("claude-501/{key}/e26ca359/scratchpad");
    fixture.write(&format!("{scratchpad}/r567/CACHEDIR.TAG"), CACHEDIR_TAG);
    let transcript = format!("home/.claude/projects/{key}/e26ca359.jsonl");
    let named = |name: &str| {
        format!(
            "{{\"type\":\"custom-title\",\"customTitle\":\"{name}\",\"sessionId\":\"e26ca359\"}}\n"
        )
    };
    fixture.write(&transcript, named("Foundry Agent Studio architecture"));
    let mut state = state_over(&fixture);

    state.rediscover(None);
    assert_eq!(session_names(&state), ["Foundry Agent Studio architecture"]);

    fixture.write(&transcript, named("Renamed after it ended"));
    fixture.write(&format!("{scratchpad}/r593/CACHEDIR.TAG"), CACHEDIR_TAG);
    state.rediscover(None);
    assert_eq!(
        session_names(&state),
        ["Foundry Agent Studio architecture"; 2],
        "a new folder takes the name its session's folders already go by"
    );
}

#[test]
fn a_session_never_named_goes_by_where_it_started_and_says_what_it_could_not_read() {
    let fixture = TempDir::new("engine-scratch-unnamed");
    let key = key_of(&fixture.dir("home/vt/foundry"));
    fixture.write(
        &format!("claude-501/{key}/e26ca359/scratchpad/r567/CACHEDIR.TAG"),
        CACHEDIR_TAG,
    );
    let transcript = fixture.write(&format!("home/.claude/projects/{key}/e26ca359.jsonl"), "");
    fixture.restrict(&transcript, 0o000);
    let mut state = state_over(&fixture);

    state.rediscover(None);

    assert_eq!(session_names(&state), ["foundry e26ca359"]);
    let snapshot = state.snapshot();
    let unread = snapshot
        .problems
        .iter()
        .find(|problem| problem.path == transcript);
    assert_eq!(
        unread.map(|problem| (problem.source, problem.kind)),
        Some((Source::Scan, io::ErrorKind::PermissionDenied))
    );
}

#[test]
fn each_scratch_folder_holds_its_session_while_it_runs_and_goes_by_the_sessions_name() {
    let fixture = TempDir::new("engine-scratch-session");
    let key = key_of(&fixture.dir("home/vt/foundry"));
    for folder in ["r567", "r593"] {
        fixture.write(
            &format!("claude-501/{key}/e26ca359/scratchpad/{folder}/CACHEDIR.TAG"),
            CACHEDIR_TAG,
        );
    }
    let registry = register_session(&fixture, "Foundry Agent Studio architecture");
    let mut state = state_over(&fixture);
    state.rediscover(None);
    let snapshot = state.snapshot();
    for card in &snapshot.projects {
        let session = card
            .scratch
            .as_ref()
            .and_then(|space| space.session.as_ref());
        assert_eq!(
            session.map(|session| session.session_id.as_str()),
            Some("e26ca359")
        );
        assert_eq!(card.agents, [], "it counts where it works");
    }
    assert_eq!(
        session_names(&state),
        ["Foundry Agent Studio architecture"; 2]
    );

    std::fs::remove_file(registry).unwrap();
    assert!(state.refresh_agents(), "the session ended");

    let snapshot = state.snapshot();
    assert!(snapshot.projects.iter().all(|card| {
        card.scratch
            .as_ref()
            .is_some_and(|space| space.session.is_none())
    }));
    assert_eq!(
        session_names(&state),
        ["Foundry Agent Studio architecture"; 2],
        "the name it went by outlasts it"
    );
    assert!(!state.refresh_agents(), "nothing more changed");
}
