//! The scratch folders of Claude Code's sessions: each folder at the top of a session's scratchpad,
//! or beside the scratchpad in the session's folder (see `scan::scratch`). The engine tracks each
//! as a project from the moment discovery finds it: its build folders are found, its size measured
//! and its file changes followed, and an agent working in it stands on it. But it has a card only
//! while an agent stands on it, or once it holds a build folder outside every worktree on the map,
//! or `LARGE_FOLDER_BYTES` without one. Most sessions throw away a few files, and a card for each
//! of them would bury the projects.
//!
//! A session's folders are listed at every rediscovery, and again when a folder comes, goes or
//! moves among them, two listings each time. A scratch folder is searched through for its build
//! folders when it is new to the map, or another folder takes its place, and again when a folder
//! appears, vanishes or moves in it, or its events went missing. Searching every scratch folder
//! through at every rediscovery would cost each rediscovery several times the walk of the home
//! folder: on Daniel's Mac (2026-11-06) the scratch root held 194 sessions' folders and 87,000
//! folders to list in them.
//!
//! A folder in a scratch folder that the search cannot list is named among the problems, and the
//! build folders the map knew in it stay, as they may be there still. A change at it, in it or at a
//! folder holding it has the scratch folder searched again: giving the folder back its permissions
//! is one, and it comes as no folder appearing.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;

use super::tracked::Tracked;
use super::{OriginReach, State};
use crate::map::agents::WorkingDirectory;
use crate::map::scan::{self, ScratchFolder, SessionFolder};
use crate::map::{
    ActivitySource, AgentSession, DirUsage, FolderAccess, Harness, LARGE_FOLDER_BYTES, Project,
    ScratchSpace, Source, SourceProblem, agents,
};

impl State {
    /// Every scratch folder of every session in the scratch root now, as a project, and the folders
    /// that could not be listed. Each is tracked as `track_scratch_folders` says, and one that
    /// `worktrees` holds as a worktree stays that. One in `previous` that a listing that failed
    /// would have named stays, with the build folders and sizes last found, until a listing that
    /// names the folders beside it leaves it out, or its own path leads nowhere. Whether Sutra can
    /// read it is read again, so nothing is read inside a folder it no longer can.
    pub(super) fn rediscover_scratch(
        &self,
        previous: &mut BTreeMap<PathBuf, Tracked>,
        worktrees: &BTreeMap<PathBuf, Tracked>,
    ) -> (Vec<(PathBuf, Tracked)>, Vec<SourceProblem>) {
        let Some(root) = &self.scratch_root else {
            return (Vec::new(), Vec::new());
        };
        let (sessions, mut failures) = scan::session_folders(root);
        let found: Vec<(SessionFolder, Vec<ScratchFolder>)> = sessions
            .into_iter()
            .map(|session| {
                let (folders, unlisted) = scan::scratch_folders(&session.path);
                failures.extend(unlisted);
                (session, folders)
            })
            .collect();
        let is_worktree = |folder: &Path| worktrees.contains_key(folder);
        // The map traces every project's provenance once the rediscovery is done, whatever changed.
        let TrackedFolders {
            mut kept, searched, ..
        } = self.track_scratch_folders(found, previous, is_worktree);
        kept.extend(searched);
        let unlisted: Vec<PathBuf> = previous
            .iter()
            .filter(|(folder, before)| {
                !is_worktree(folder) && could_not_be_listed(folder, before, &failures)
            })
            .map(|(folder, _)| folder.clone())
            .collect();
        for folder in unlisted {
            let Some(mut before) = previous.remove(&folder) else {
                continue;
            };
            match scan::folder_access(&folder, &self.boundary) {
                FolderAccess::Missing => {}
                access => {
                    before.project.access = access;
                    kept.push((folder, before));
                }
            }
        }
        self.name_sessions(&mut kept, &mut failures);
        (kept, SourceProblem::per_kind(Source::Scan, failures))
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
    /// went or moved among them: search through for its build folders each new one, and each that
    /// took the place of the folder the map found there, and drop each gone. Returns whether any
    /// came, went or was searched anew, or `None` when the session's folders could not all be
    /// listed, which leaves those on the map as they were, for a rediscovery to say why; so does a
    /// folder no session's folder could be, whose name is not text.
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
        let TrackedFolders {
            kept,
            searched,
            origins_changed,
        } = self.track_scratch_folders(vec![(session_folder, folders)], &mut previous, is_worktree);
        // What the listing did not name is left in `previous`, and goes.
        let changed = !searched.is_empty() || !previous.is_empty() || origins_changed;
        self.projects.extend(kept.into_iter().chain(searched));
        if changed {
            self.leave_build_folders_to_nested_projects();
            let sessions: Vec<AgentSession> = self.sessions_on_the_map().cloned().collect();
            self.find_scratch_sessions(&sessions);
            self.trace_provenance();
            self.reindex();
            self.report_unread();
        }
        Some(changed)
    }

    /// Track the scratch folders `found`, listed by the session folder holding them: one tracked
    /// already is taken from `previous` whole, with the build folders last found in it, while it is
    /// still the folder searched and Sutra's access to it is unchanged, and where it came from read
    /// again, as it may have become a clone since, or come to hold one; one new to the map, or
    /// another folder in the place of one on it, is searched through, on discovery's threads, in
    /// the space its session's folders on the map share, with what the map knows of the session,
    /// or in a new one. A folder `is_worktree` names stays the worktree it is, as git's claim on it
    /// comes first.
    fn track_scratch_folders(
        &self,
        found: Vec<(SessionFolder, Vec<ScratchFolder>)>,
        previous: &mut BTreeMap<PathBuf, Tracked>,
        is_worktree: impl Fn(&Path) -> bool,
    ) -> TrackedFolders {
        let mut kept: Vec<(PathBuf, Tracked)> = Vec::new();
        let mut new = Vec::new();
        // The sessions started in one folder share where they started, which is worked out once.
        let mut started_in: HashMap<String, WorkingDirectory> = HashMap::new();
        for (session, folders) in found {
            let first_kept = kept.len();
            let mut new_here = Vec::new();
            for listed in folders
                .into_iter()
                .filter(|folder| !is_worktree(&folder.path))
            {
                let access = scan::folder_access(&listed.path, &self.boundary);
                match previous.remove(&listed.path) {
                    // Only a scratch folder has an inode on the map.
                    Some(before)
                        if before.inode == Some(listed.inode)
                            && before.project.access == access =>
                    {
                        kept.push((listed.path, before));
                    }
                    _ => new_here.push(listed),
                }
            }
            if new_here.is_empty() {
                continue;
            }
            let mut kept_here = kept[first_kept..].iter();
            let known = kept_here.find_map(|(_, tracked)| tracked.project.scratch.clone());
            let space = known.unwrap_or_else(|| self.space_of(&session, &mut started_in));
            new.extend(new_here.into_iter().map(|listed| (listed, space.clone())));
        }
        let boundary = &self.boundary;
        let reach = OriginReach::of(boundary, self.sandboxed, &self.roots);
        let may_follow = |folder: &Path| reach.may_follow(folder);
        // Each search is metadata I/O like discovery's walk, and so is each reading of where a
        // folder came from.
        let (searched, origins_changed) = scan::on_discovery_threads(|| {
            let origins_changed = kept
                .par_iter_mut()
                .map(|(_, tracked)| {
                    let before = tracked.origin.clone();
                    tracked.read_origin_again(&may_follow);
                    tracked.origin != before
                })
                .reduce(|| false, |one, other| one || other);
            let searched = new
                .into_par_iter()
                .map(|(listed, space)| {
                    let path = listed.path.clone();
                    (path, Tracked::scratch(listed, space, boundary, &may_follow))
                })
                .collect();
            (searched, origins_changed)
        });
        TrackedFolders {
            kept,
            searched,
            origins_changed,
        }
    }

    /// The scratch space of the session whose folder is `session`, before the session is found
    /// running (see `find_scratch_sessions`). `started_in` holds where the sessions started, by
    /// Claude Code's name for the folder, as worked out so far.
    fn space_of(
        &self,
        session: &SessionFolder,
        started_in: &mut HashMap<String, WorkingDirectory>,
    ) -> ScratchSpace {
        let started_in = started_in
            .entry(session.project_key.clone())
            .or_insert_with(|| agents::working_directory(&session.project_key, &self.boundary));
        ScratchSpace {
            session_folder: session.path.clone(),
            session_id: session.session_id.clone(),
            started_in: started_in.name.clone(),
            started_in_folder: started_in.folder.clone(),
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

    /// Whether `path`, in the scratch folder `project`, holds or lies in a folder the last search of
    /// `project` could not list, so a change there, as one to its permissions, may let a search in.
    pub(in crate::map::engine) fn search_could_not_list(
        &self,
        project: &Path,
        path: &Path,
    ) -> bool {
        let Some(tracked) = self.projects.get(project) else {
            return false;
        };
        let unlisted = tracked.unlisted.iter();
        unlisted
            .map(|(folder, _)| folder)
            .any(|folder| folder.starts_with(path) || path.starts_with(folder))
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

/// Whether `tracked`, at `folder`, is a scratch folder that one of the listings that failed,
/// `failures`, would have named: that of the folder holding it, or of one holding its session's
/// folder, which then went unlisted. The listing of the session's folder itself names only the
/// folders beside its scratchpad, which is listed on its own.
fn could_not_be_listed(
    folder: &Path,
    tracked: &Tracked,
    failures: &[(PathBuf, io::ErrorKind)],
) -> bool {
    let Some(space) = &tracked.project.scratch else {
        return false;
    };
    // The session's folder is `<scratch root>/<project key>/<session id>`.
    let holds_its_session = |failed: &Path| {
        let key = space.session_folder.parent();
        key.is_some_and(|key| key.starts_with(failed))
    };
    failures
        .iter()
        .any(|(failed, _)| folder.parent() == Some(failed.as_path()) || holds_its_session(failed))
}

/// The scratch folders one listing named, as the map tracks them (see
/// `State::track_scratch_folders`).
struct TrackedFolders {
    /// Those the map held, taken whole.
    kept: Vec<(PathBuf, Tracked)>,
    /// Those searched through anew.
    searched: Vec<(PathBuf, Tracked)>,
    /// Where one of those kept came from has changed since the map read it last.
    origins_changed: bool,
}

/// Whether `project` has a card on the map. A scratch folder earns one while something works in it
/// (see `Project::is_at_work`): an agent standing on it, a session or a subagent at work there (see
/// `Place`), or a dev-runner environment describing it, a process listening in it or a container
/// running for it, whose servers would show nowhere else. It earns one too by holding a build
/// folder outside every worktree on the map, which keeps its own, or `LARGE_FOLDER_BYTES` without
/// one; every other project has one.
/// The session whose scratch it is earns it none by running: the session stands where it works.
pub(super) fn earns_a_card(project: &Project) -> bool {
    let large = || {
        project
            .own_files
            .usage()
            .is_some_and(|usage| usage.bytes >= LARGE_FOLDER_BYTES)
    };
    project.scratch.is_none() || project.is_at_work() || !project.artifacts.is_empty() || large()
}

#[cfg(test)]
mod tests;
