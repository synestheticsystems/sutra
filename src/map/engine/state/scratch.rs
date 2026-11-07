//! The scratch folders of Claude Code's sessions. The engine tracks every session's folder as a
//! project from the moment discovery finds it: its build folders are found, its size measured and
//! its file changes followed. But it has a card, and agents stand on it, only once it holds a build
//! folder, or `LARGE_FOLDER_BYTES` without one. Most sessions throw away a few files, and a card
//! for each of them would bury the projects.
//!
//! A scratch folder is searched through for its build folders when discovery first finds it, and
//! again when a folder appears, vanishes or moves in it, or its events went missing. Searching every
//! scratch folder through at every rediscovery would cost each rediscovery several times the walk
//! of the home folder: on Daniel's Mac (2026-11-06) the scratch root held 194 sessions' folders and
//! 87,000 folders to list in them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;

use super::State;
use super::tracked::Tracked;
use crate::map::{
    ActivitySource, AgentSession, DirUsage, Harness, LARGE_FOLDER_BYTES, Project, Source,
    SourceProblem, agents, scan,
};

impl State {
    /// Every session's scratch folder in the scratch root now, as a project, and the folders that
    /// could not be listed. One tracked already is taken from `previous` whole, with the build
    /// folders last found in it, unless Sutra's access to it changed since; a new one is searched
    /// through. A folder that `worktrees` holds as a linked worktree stays that.
    pub(super) fn rediscover_scratch(
        &self,
        previous: &mut BTreeMap<PathBuf, Tracked>,
        worktrees: &BTreeMap<PathBuf, Tracked>,
    ) -> (Vec<(PathBuf, Tracked)>, Vec<SourceProblem>) {
        let Some(root) = &self.scratch_root else {
            return (Vec::new(), Vec::new());
        };
        let (found, failures) = scan::scratch_folders(root);
        let mut kept = Vec::new();
        let mut new = Vec::new();
        for folder in found {
            if worktrees.contains_key(&folder.path) {
                continue;
            }
            let access = scan::folder_access(&folder.path, &self.boundary);
            match previous.remove(&folder.path) {
                Some(before)
                    if before.project.scratch.is_some() && before.project.access == access =>
                {
                    kept.push((folder.path, before));
                }
                _ => new.push(folder),
            }
        }
        // The sessions started in one folder share its name, which is worked out once.
        let mut started_in: HashMap<String, String> = HashMap::new();
        let new: Vec<_> = new
            .into_iter()
            .map(|folder| {
                let name = started_in
                    .entry(folder.project_key.clone())
                    .or_insert_with(|| {
                        agents::working_directory_name(&folder.project_key, &self.boundary)
                    })
                    .clone();
                (folder, name)
            })
            .collect();
        let boundary = &self.boundary;
        // Each search is metadata I/O like discovery's walk.
        let searched: Vec<(PathBuf, Tracked)> = scan::on_discovery_threads(|| {
            new.into_par_iter()
                .map(|(folder, started_in)| {
                    (
                        folder.path.clone(),
                        Tracked::scratch(folder, started_in, boundary),
                    )
                })
                .collect()
        });
        kept.extend(searched);
        (kept, SourceProblem::per_kind(Source::Scan, failures))
    }

    /// Hand each session's scratch folder its session while the session runs, among `sessions`,
    /// and the name that goes with it (see `ScratchSpace::name`). Returns whether a folder with a
    /// card changed.
    pub(super) fn find_scratch_sessions(&mut self, sessions: &[AgentSession]) -> bool {
        let running: HashMap<&str, &AgentSession> = sessions
            .iter()
            .filter(|session| session.harness == Harness::ClaudeCode)
            .map(|session| (session.session_id.as_str(), session))
            .collect();
        let mut changed = false;
        for tracked in self.projects.values_mut() {
            let project = &mut tracked.project;
            let Some(space) = &mut project.scratch else {
                continue;
            };
            let session = running.get(space.session_id.as_str()).copied();
            if space.session.as_ref() != session {
                space.session = session.cloned();
                project.name = space.name();
                changed |= earns_a_card(project);
            }
        }
        changed
    }

    /// Date the work in `project`, when it is a session's scratch folder, by the newest file the
    /// walk of its own files found. A scratch folder has no git and no environment to date it, so
    /// before any file event its work would be unknown, and the folder taken for quiet for good.
    /// The file's time counts as a change to it would (see `live::worked`): by its own time, and
    /// never later than now. The walk leaves out its build folders, as a build's writes are no
    /// project's activity.
    pub(super) fn date_scratch_folder(&mut self, project: &Path, usage: &DirUsage) {
        let Some(tracked) = self.projects.get_mut(project) else {
            return;
        };
        let Some(written) = usage
            .newest_write
            .filter(|_| tracked.project.scratch.is_some())
        else {
            return;
        };
        let at = written.min(SystemTime::now());
        if tracked.observed.is_none_or(|(seen, _)| seen < at) {
            tracked.observed = Some((at, ActivitySource::FileChange));
            tracked.recompute_activity();
        }
    }

    /// Whether `project` is a session's scratch folder the map tracks.
    pub(in crate::map::engine) fn is_scratch_folder(&self, project: &Path) -> bool {
        self.projects
            .get(project)
            .is_some_and(|tracked| tracked.project.scratch.is_some())
    }

    /// The session folders the map tracks that hold or lie inside one of `folders`, so whatever a
    /// lost event announced in them is searched for again.
    pub(in crate::map::engine) fn scratch_folders_meeting(
        &self,
        folders: &[PathBuf],
    ) -> Vec<PathBuf> {
        self.projects
            .values()
            .filter(|tracked| tracked.project.scratch.is_some())
            .map(|tracked| &tracked.project.path)
            .filter(|path| {
                let meets = |folder: &PathBuf| path.starts_with(folder) || folder.starts_with(path);
                folders.iter().any(meets)
            })
            .cloned()
            .collect()
    }
}

/// Whether `project` has a card on the map. A session's scratch folder earns one by holding a build
/// folder, or `LARGE_FOLDER_BYTES` without one; every other project has one.
pub(super) fn earns_a_card(project: &Project) -> bool {
    let large = || {
        project
            .own_files
            .usage()
            .is_some_and(|usage| usage.bytes >= LARGE_FOLDER_BYTES)
    };
    project.scratch.is_none() || !project.artifacts.is_empty() || large()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::engine::sizing::Job;
    use crate::map::grant::Roots;
    use crate::map::test_support::{
        CACHEDIR_TAG, TempDir, at, cargo_target, git, measured, project_at, usage,
    };
    use crate::map::{Measurement, ScanOptions, ScratchSpace};

    /// A scratch folder holding the build folders `artifacts` and own files of `own_files`.
    fn scratch(artifacts: &[&str], own_files: Measurement) -> Project {
        let path = PathBuf::from("/private/tmp/claude-501/-Users-me/e26ca359");
        Project {
            scratch: Some(ScratchSpace {
                session_id: "e26ca359".to_string(),
                started_in: "me".to_string(),
                session: None,
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
    fn a_scratch_folder_earns_a_card_with_a_build_folder_or_a_large_size_and_others_always_have_one()
     {
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

    /// A map whose scratch root is the fixture's `claude-501`, and whose home is its `home`.
    fn state_over(fixture: &TempDir) -> State {
        let roots = Roots {
            home: fixture.dir("home"),
            dirs: vec![fixture.path("home")],
            scratch: Some(fixture.path("claude-501")),
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

    #[test]
    fn a_session_folder_has_a_card_once_it_holds_a_build_folder_or_measures_large() {
        let fixture = TempDir::new("engine-scratch-cards");
        let foundry = "claude-501/-Users-me-vt-foundry";
        let builds = fixture.dir(&format!("{foundry}/e26ca359/scratchpad"));
        fixture.write(
            &format!("{foundry}/e26ca359/scratchpad/r567/target/CACHEDIR.TAG"),
            CACHEDIR_TAG,
        );
        let quiet = fixture.dir(&format!("{foundry}/0b1d4f00/scratchpad"));
        let large = fixture.dir("claude-501/-Users-me-sk/d5fa0639/scratchpad");
        let mut state = state_over(&fixture);

        state.rediscover(None);

        let card = |path: &Path| path.parent().unwrap().to_path_buf();
        assert_eq!(paths(&state), [card(&builds)], "the others wait for a size");
        let large_job = Job::OwnFiles(card(&large));
        let quiet_job = Job::OwnFiles(card(&quiet));
        assert!(state.apply_usage(&large_job, Ok(usage(LARGE_FOLDER_BYTES))));
        assert!(
            !state.apply_usage(&quiet_job, Ok(usage(4096))),
            "a folder with no card shows nothing new"
        );
        assert_eq!(paths(&state), [card(&large), card(&builds)]);
    }

    /// Claude Code's name for `folder`: each character but a letter or a digit becomes `-`.
    fn key_of(folder: &Path) -> String {
        let path = folder.to_str().unwrap();
        path.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    }

    #[test]
    fn a_session_folder_that_turns_readable_is_searched_again_at_the_next_rediscovery() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = TempDir::new("engine-scratch-access");
        fixture.write(
            "claude-501/-Users-me/e26ca359/scratchpad/out/CACHEDIR.TAG",
            CACHEDIR_TAG,
        );
        let session = fixture.path("claude-501/-Users-me/e26ca359");
        fixture.restrict(&session, 0o000);
        let mut state = state_over(&fixture);
        state.rediscover(None);
        assert!(paths(&state).is_empty(), "nothing in it to see");

        std::fs::set_permissions(&session, std::fs::Permissions::from_mode(0o755)).unwrap();
        state.rediscover(None);

        assert_eq!(paths(&state), [session]);
    }

    #[test]
    fn a_scratch_card_goes_by_the_folder_its_session_started_in_and_its_id() {
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
        assert_eq!(card.name, "greyorange-foundry e26ca359");
        let artifacts: Vec<&Path> = card
            .artifacts
            .iter()
            .map(|dir| dir.path.as_path())
            .collect();
        assert_eq!(artifacts, [card.path.join("scratchpad/target-567")]);
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
        let scratch = fixture.path("claude-501/-Users-me/e26ca359");
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

    #[test]
    fn a_scratch_card_holds_its_session_while_it_runs_and_goes_by_the_sessions_name() {
        let fixture = TempDir::new("engine-scratch-session");
        let key = key_of(&fixture.dir("home/vt/foundry"));
        fixture.write(
            &format!("claude-501/{key}/e26ca359/scratchpad/out/CACHEDIR.TAG"),
            CACHEDIR_TAG,
        );
        // This process stands for the session's, so it runs for as long as the test.
        let me = std::process::id();
        let registry = fixture.write(
            &format!("home/.claude/sessions/{me}.json"),
            format!(
                r#"{{"pid": {me}, "sessionId": "e26ca359", "cwd": "/Users/me/vt/foundry",
                    "status": "busy", "name": "Foundry Agent Studio architecture"}}"#
            ),
        );
        let mut state = state_over(&fixture);
        state.rediscover(None);
        let card = |state: &State| state.snapshot().projects.into_iter().next().unwrap();
        let running = card(&state);
        let session = running.scratch.and_then(|space| space.session);
        assert_eq!(
            session.map(|session| session.session_id).as_deref(),
            Some("e26ca359")
        );
        assert_eq!(running.name, "Foundry Agent Studio architecture");
        assert_eq!(running.agents, [], "it counts where it works");

        std::fs::remove_file(registry).unwrap();
        assert!(state.refresh_agents(), "the session ended");

        let gone = card(&state);
        assert_eq!(gone.scratch.and_then(|space| space.session), None);
        assert_eq!(gone.name, "foundry e26ca359");
        assert!(!state.refresh_agents(), "nothing more changed");
    }
}
