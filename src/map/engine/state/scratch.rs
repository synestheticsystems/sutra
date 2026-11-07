//! The scratch folders of Claude Code's sessions: each folder at the top of a session's scratchpad,
//! or beside the scratchpad in the session's folder (see `scan::scratch`). The engine tracks each
//! as a project from the moment discovery finds it: its build folders are found, its size measured
//! and its file changes followed. But it has a card, and agents stand on it, only once it holds a
//! build folder outside every worktree on the map, or `LARGE_FOLDER_BYTES` without one. Most
//! sessions throw away a few files, and a card for each of them would bury the projects.
//!
//! A session's folders are listed at every rediscovery, and again when a folder comes, goes or
//! moves among them, two listings each time. A scratch folder is searched through for its build
//! folders when it is new to the map, and again when a folder appears, vanishes or moves in it, or
//! its events went missing. Searching every scratch folder through at every rediscovery would cost
//! each rediscovery several times the walk of the home folder: on Daniel's Mac (2026-11-06) the
//! scratch root held 194 sessions' folders and 87,000 folders to list in them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;

use super::State;
use super::tracked::Tracked;
use crate::map::scan::{self, SessionFolder};
use crate::map::{
    ActivitySource, AgentSession, DirUsage, Harness, LARGE_FOLDER_BYTES, Project, ScratchSpace,
    Source, SourceProblem, agents,
};

impl State {
    /// Every scratch folder of every session in the scratch root now, as a project, and the folders
    /// that could not be listed. Each is tracked as `track_scratch_folders` says, and one that
    /// `worktrees` holds as a worktree stays that.
    pub(super) fn rediscover_scratch(
        &self,
        previous: &mut BTreeMap<PathBuf, Tracked>,
        worktrees: &BTreeMap<PathBuf, Tracked>,
    ) -> (Vec<(PathBuf, Tracked)>, Vec<SourceProblem>) {
        let Some(root) = &self.scratch_root else {
            return (Vec::new(), Vec::new());
        };
        let (sessions, mut failures) = scan::session_folders(root);
        let found: Vec<(SessionFolder, Vec<PathBuf>)> = sessions
            .into_iter()
            .map(|session| {
                let (folders, unlisted) = scan::scratch_folders(&session.path);
                failures.extend(unlisted);
                (session, folders)
            })
            .collect();
        let is_worktree = |folder: &Path| worktrees.contains_key(folder);
        let mut tracked = self.track_scratch_folders(found, previous, is_worktree);
        self.name_sessions(&mut tracked, &mut failures);
        (tracked, SourceProblem::per_kind(Source::Scan, failures))
    }

    /// Name each session among the scratch folders `tracked` that has no name yet and is not seen
    /// running, on every one of its folders, by the name its transcript last recorded (see
    /// `agents::recorded_session_name`). A running session names itself (see
    /// `find_scratch_sessions`), and a session that ended keeps the name it went by. A session
    /// never named is looked up again at each rediscovery, which reads only the end of a
    /// transcript, if it has one. The transcripts that could not be read go to `failures`.
    fn name_sessions(
        &self,
        tracked: &mut [(PathBuf, Tracked)],
        failures: &mut Vec<(PathBuf, io::ErrorKind)>,
    ) {
        let mut names: HashMap<PathBuf, Option<String>> = HashMap::new();
        let spaces = tracked
            .iter_mut()
            .filter_map(|(_, t)| t.project.scratch.as_mut());
        for space in spaces.filter(|space| space.session_name.is_none() && space.session.is_none())
        {
            let recorded = || {
                // The session's folder is `<scratch root>/<project key>/<session id>`.
                let key = space.session_folder.parent()?.file_name()?.to_str()?;
                agents::recorded_session_name(&self.home, key, &space.session_id, failures)
            };
            let name = names.entry(space.session_folder.clone());
            space.session_name.clone_from(name.or_insert_with(recorded));
        }
    }

    /// List the scratch folders of the session whose folder is `session` again, after a folder came,
    /// went or moved among them: track each new one, searched through for its build folders, and
    /// drop each gone. Returns whether any came or went, or `None` when the session's folders could
    /// not all be listed, which leaves those on the map as they were, for a rediscovery to say why;
    /// so does a folder no session's folder could be, whose name is not text.
    pub(in crate::map::engine) fn refind_scratch_folders(
        &mut self,
        session: &Path,
    ) -> Option<bool> {
        let session_folder = SessionFolder::at(session)?;
        let (folders, failures) = scan::scratch_folders(session);
        if !failures.is_empty() {
            return None;
        }
        let before: Vec<PathBuf> = self
            .projects
            .iter()
            .filter(|(_, tracked)| {
                let space = tracked.project.scratch.as_ref();
                space.is_some_and(|space| space.session_folder == session)
            })
            .map(|(path, _)| path.clone())
            .collect();
        let mut previous: BTreeMap<PathBuf, Tracked> = before
            .iter()
            .filter_map(|path| Some((path.clone(), self.projects.remove(path)?)))
            .collect();
        let is_worktree = |folder: &Path| {
            let tracked = self.projects.get(folder);
            tracked.is_some_and(|tracked| tracked.project.repo.is_some())
        };
        let tracked =
            self.track_scratch_folders(vec![(session_folder, folders)], &mut previous, is_worktree);
        let after: BTreeSet<&PathBuf> = tracked.iter().map(|(path, _)| path).collect();
        let changed = after != before.iter().collect();
        self.projects.extend(tracked);
        if changed {
            self.leave_build_folders_to_nested_projects();
            let sessions: Vec<AgentSession> = self.sessions_on_the_map().cloned().collect();
            self.find_scratch_sessions(&sessions);
            self.reindex();
        }
        Some(changed)
    }

    /// Track the scratch folders `found`, listed by the session folder holding them: one tracked
    /// already is taken from `previous` whole, with the build folders last found in it, unless
    /// Sutra's access to it changed since; a new one is searched through, on discovery's threads,
    /// in the space its session's folders on the map share, with what the map knows of the
    /// session, or in a new one. A folder `is_worktree` names stays the worktree it is, as git's
    /// claim on it comes first.
    fn track_scratch_folders(
        &self,
        found: Vec<(SessionFolder, Vec<PathBuf>)>,
        previous: &mut BTreeMap<PathBuf, Tracked>,
        is_worktree: impl Fn(&Path) -> bool,
    ) -> Vec<(PathBuf, Tracked)> {
        let mut kept: Vec<(PathBuf, Tracked)> = Vec::new();
        let mut new = Vec::new();
        // The sessions started in one folder share its name, which is worked out once.
        let mut started_in: HashMap<String, String> = HashMap::new();
        for (session, folders) in found {
            let first_kept = kept.len();
            let mut new_here = Vec::new();
            for folder in folders.into_iter().filter(|folder| !is_worktree(folder)) {
                let access = scan::folder_access(&folder, &self.boundary);
                match previous.remove(&folder) {
                    Some(before)
                        if before.project.scratch.is_some() && before.project.access == access =>
                    {
                        kept.push((folder, before));
                    }
                    _ => new_here.push(folder),
                }
            }
            if new_here.is_empty() {
                continue;
            }
            let mut kept_here = kept[first_kept..].iter();
            let known = kept_here.find_map(|(_, tracked)| tracked.project.scratch.clone());
            let space = known.unwrap_or_else(|| self.space_of(&session, &mut started_in));
            new.extend(new_here.into_iter().map(|folder| (folder, space.clone())));
        }
        let boundary = &self.boundary;
        // Each search is metadata I/O like discovery's walk.
        let searched: Vec<(PathBuf, Tracked)> = scan::on_discovery_threads(|| {
            new.into_par_iter()
                .map(|(folder, space)| (folder.clone(), Tracked::scratch(folder, space, boundary)))
                .collect()
        });
        kept.extend(searched);
        kept
    }

    /// The scratch space of the session whose folder is `session`, before the session is found
    /// running (see `find_scratch_sessions`). `names` holds the name of each folder sessions started
    /// in, by Claude Code's name for it, as worked out so far.
    fn space_of(
        &self,
        session: &SessionFolder,
        names: &mut HashMap<String, String>,
    ) -> ScratchSpace {
        let started_in = names.entry(session.project_key.clone()).or_insert_with(|| {
            agents::working_directory_name(&session.project_key, &self.boundary)
        });
        ScratchSpace {
            session_folder: session.path.clone(),
            session_id: session.session_id.clone(),
            started_in: started_in.clone(),
            // Found with the session's other agents (see `State::refresh_agents`).
            session: None,
            // Found with the session, or in its transcript (see `name_sessions`).
            session_name: None,
        }
    }

    /// Hand each scratch folder its session while the session runs, among `sessions`, and the name
    /// the session goes by, which the folder keeps once the session ends (see
    /// `ScratchSpace::name`). Returns whether a folder with a card changed.
    pub(super) fn find_scratch_sessions(&mut self, sessions: &[AgentSession]) -> bool {
        let running: HashMap<&str, &AgentSession> = sessions
            .iter()
            .filter(|session| session.harness == Harness::ClaudeCode)
            .map(|session| (session.session_id.as_str(), session))
            .collect();
        let mut changed = false;
        for tracked in self.projects.values_mut() {
            let has_a_card = earns_a_card(&tracked.project);
            let Some(space) = &mut tracked.project.scratch else {
                continue;
            };
            let session = running.get(space.session_id.as_str()).copied();
            if space.session.as_ref() != session {
                space.session = session.cloned();
                if let Some(label) = session.and_then(|session| session.label.as_ref()) {
                    space.session_name = Some(label.clone());
                }
                changed |= has_a_card;
            }
        }
        changed
    }

    /// Date the work in `project`, when it is a scratch folder, by the newest file the walk of its
    /// own files found. A scratch folder has no git and no environment to date it, so before any
    /// file event its work would be unknown, and the folder taken for quiet for good. The file's
    /// time counts as a change to it would (see `live::worked`): by its own time, and never later
    /// than now. The walk leaves out its build folders, as a build's writes are no project's
    /// activity.
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

    /// Whether `project` is a scratch folder the map tracks.
    pub(in crate::map::engine) fn is_scratch_folder(&self, project: &Path) -> bool {
        self.projects
            .get(project)
            .is_some_and(|tracked| tracked.project.scratch.is_some())
    }

    /// The scratch folders the map tracks that hold or lie inside one of `folders`, so whatever a
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

/// Whether `project` has a card on the map. A scratch folder earns one by holding a build folder
/// outside every worktree on the map, which keeps its own, or `LARGE_FOLDER_BYTES` without one;
/// every other project has one.
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
    use crate::map::{Measurement, ScanOptions};

    /// A scratch folder holding the build folders `artifacts` and own files of `own_files`.
    fn scratch(artifacts: &[&str], own_files: Measurement) -> Project {
        let session = PathBuf::from("/private/tmp/claude-501/-Users-me/e26ca359");
        let path = session.join("scratchpad/r567");
        Project {
            scratch: Some(ScratchSpace {
                session_folder: session,
                session_id: "e26ca359".to_string(),
                started_in: "me".to_string(),
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

    /// The build folders the map holds in the project at `path`.
    fn build_folders(state: &State, path: &Path) -> Vec<PathBuf> {
        let artifacts = state.projects[path].project.artifacts.iter();
        artifacts.map(|dir| dir.path.clone()).collect()
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
    fn a_scratch_folders_work_is_unknown_to_cleaning_until_the_walk_of_its_own_files_dates_it() {
        let fixture = TempDir::new("engine-scratch-work-unknown");
        let scratchpad = "claude-501/-Users-me/e26ca359/scratchpad";
        for name in ["r567", "r593"] {
            fixture.write(
                &format!("{scratchpad}/{name}/target/CACHEDIR.TAG"),
                CACHEDIR_TAG,
            );
        }
        let folder = |name: &str| fixture.path(&format!("{scratchpad}/{name}"));
        let mut state = state_over(&fixture);
        state.rediscover(None);
        let now = SystemTime::now();
        let idle = |state: &State| {
            let snapshot = state.snapshot();
            let hour = std::time::Duration::from_secs(60 * 60);
            let found = crate::map::clean::idle_folders(&snapshot, now, hour);
            found
                .iter()
                .map(|idle| idle.project.path.clone())
                .collect::<Vec<_>>()
        };
        // Each build folder is sized before the scratch folder's own files.
        for name in ["r567", "r593"] {
            let target = Job::Artifact(folder(name).join("target"));
            state.apply_usage(&target, Ok(usage(1 << 30)));
        }
        assert!(idle(&state).is_empty(), "the work in each is unknown");

        let written = |secs_ago: u64| DirUsage {
            newest_write: Some(now - std::time::Duration::from_secs(secs_ago)),
            ..usage(4096)
        };
        state.apply_usage(&Job::OwnFiles(folder("r567")), Ok(written(5 * 60)));
        state.apply_usage(&Job::OwnFiles(folder("r593")), Ok(written(2 * 86_400)));

        assert_eq!(
            idle(&state),
            [folder("r593")],
            "r567's own files were written minutes ago"
        );
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
        let scratch = fixture.path("claude-501/-Users-me/e26ca359/scratchpad/out");
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
    fn a_session_whose_folders_cannot_all_be_listed_keeps_those_on_the_map() {
        let fixture = TempDir::new("engine-scratch-unlisted");
        let scratchpad = "claude-501/-Users-me/e26ca359/scratchpad";
        fixture.write(&format!("{scratchpad}/r567/CACHEDIR.TAG"), CACHEDIR_TAG);
        let session = fixture.path("claude-501/-Users-me/e26ca359");
        let mut state = state_over(&fixture);
        state.rediscover(None);

        fixture.restrict(&fixture.path(scratchpad), 0o000);

        assert_eq!(state.refind_scratch_folders(&session), None);
        assert_eq!(paths(&state), [session.join("scratchpad/r567")]);
    }
}
