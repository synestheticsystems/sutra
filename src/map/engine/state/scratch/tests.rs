//! The scratch folders of sessions over real folders: which earn a card, how discovery and a
//! session's listing find them and search them through, the session each holds and goes by, and
//! how its work is dated.

use super::*;
use crate::map::engine::sizing::Job;
use crate::map::grant::Roots;
use crate::map::test_support::{
    CACHEDIR_TAG, TempDir, agent_session, at, cargo_target, environment_at, git, listening_in,
    measured, project_at, subagent_at_work, usage,
};
use crate::map::{AgentStatus, Measurement, ScanOptions, SubagentWithParent};

mod cards;
mod dating;
mod search;
mod sessions;

/// A map whose scratch root is the fixture's `claude-501`, and whose home is its `home`.
fn state_over(fixture: &TempDir) -> State {
    let roots = Roots {
        home: fixture.dir("home"),
        dirs: vec![fixture.path("home")],
        scratch: Some(fixture.path("claude-501")),
        docker: Vec::new(),
        sandboxed: false,
    };
    State::new(&roots, &ScanOptions::default())
}

fn paths(state: &State) -> Vec<PathBuf> {
    let snapshot = state.snapshot();
    let mut paths: Vec<PathBuf> = snapshot.projects.into_iter().map(|p| p.path).collect();
    paths.sort();
    paths
}

/// The build folders the map holds in the project at `path`.
fn build_folders(state: &State, path: &Path) -> Vec<PathBuf> {
    let artifacts = state.projects[path].project.artifacts.iter();
    artifacts.map(|dir| dir.path.clone()).collect()
}

/// Claude Code's name for `folder`: each character but a letter or a digit becomes `-`.
fn key_of(folder: &Path) -> String {
    let path = folder.to_str().unwrap();
    path.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
}

/// The names the sessions of `state`'s scratch cards go by, in the snapshot's order.
fn session_names(state: &State) -> Vec<String> {
    let snapshot = state.snapshot();
    let spaces = snapshot
        .projects
        .iter()
        .filter_map(|card| card.scratch.as_ref());
    spaces.map(ScratchSpace::name).collect()
}

/// Registers the live session `e26ca359` of this process under `fixture`'s home, named
/// `name`. Returns the file that registers it.
fn register_session(fixture: &TempDir, name: &str) -> PathBuf {
    // This process stands for the session's, so it runs for as long as the test.
    let me = std::process::id();
    fixture.write(
        &format!("home/.claude/sessions/{me}.json"),
        format!(
            r#"{{"pid": {me}, "sessionId": "e26ca359", "cwd": "/Users/me/vt/foundry",
                "status": "busy", "name": "{name}"}}"#
        ),
    )
}
