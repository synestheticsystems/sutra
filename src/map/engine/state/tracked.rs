//! One project on the map, with the facts its last activity is the newest of, so a refresh of
//! one source can recompute it without re-reading the others.

use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::artifacts::{artifact_dirs, keep_measurements};
use super::new_project;
use super::provenance::Origin;
use crate::map::devrunner::FolderEnvironments;
use crate::map::scan::{self, Boundary, ScratchFolder, Worktree};
use crate::map::{ActivitySource, FolderAccess, Project, ScratchSpace};
use crate::model::Environment;

pub(super) struct Tracked {
    pub(super) project: Project,
    /// From `scan::git_activity`.
    pub(super) git_activity: Option<SystemTime>,
    /// The newest change the engine saw happen while watching: a file written in the project, or
    /// its environment publishing a new status. A scratch folder also takes the newest file a walk
    /// of it found (see `State::date_scratch_folder`).
    pub(super) observed: Option<(SystemTime, ActivitySource)>,
    /// The newest activity of any agent seen here, a session or a subagent at work for a session
    /// elsewhere. It outlives the agent: one that only read leaves nothing else behind when it
    /// ends, and without this the project would fall back to its last commit and count as idle at
    /// once.
    pub(super) agent_activity: Option<SystemTime>,
    /// Every environment describing this folder; `project.environment` is what they merge into.
    pub(super) environments: Option<FolderEnvironments>,
    /// For a scratch folder, the inode of the folder the map found at its path, so that one moved
    /// into its place later is told from it, and searched through as the new folder it is (see
    /// `scratch`).
    pub(super) inode: Option<u64>,
    /// For a scratch folder, the folders in it that its last search for build folders could not
    /// list, and why, where a build folder may have gone unfound (see `scratch`).
    pub(super) unlisted: Vec<(PathBuf, io::ErrorKind)>,
    /// What its own files say of where it came from, read with it (see `provenance`).
    pub(super) origin: Origin,
    /// The files that reading `origin` could not read, and why.
    pub(super) origin_unread: Vec<(PathBuf, io::ErrorKind)>,
}

/// Whether Sutra may follow a checkout's origin to a folder (see `State::origin_reach`).
pub(super) type MayFollow<'a> = &'a (dyn Fn(&Path) -> bool + Sync);

impl Tracked {
    pub(super) fn worktree(
        worktree: Worktree,
        boundary: &Boundary,
        may_follow: MayFollow,
    ) -> Tracked {
        let git_activity = scan::git_activity(&worktree.repo);
        let (path, access) = (worktree.path, worktree.access);
        let (origin, origin_unread) =
            Origin::of_worktree(&path, &worktree.repo, access, may_follow);
        let provenance = origin.own_provenance(&path);
        let project = new_project(path, Some(worktree.repo), None, access, provenance);
        Tracked {
            git_activity,
            ..Tracked::searched(project, None, (origin, origin_unread), boundary)
        }
    }

    /// A folder only environments name. Like a registered worktree, one that is gone, outside the
    /// grant or in a private folder Sutra stays out of stays on the map, unreadable.
    pub(super) fn plain(folder: PathBuf, boundary: &Boundary, may_follow: MayFollow) -> Tracked {
        let access = scan::folder_access(&folder, boundary);
        let (origin, origin_unread) = Origin::of_folder(&folder, access, may_follow);
        let provenance = origin.own_provenance(&folder);
        let project = new_project(folder, None, None, access, provenance);
        Tracked::searched(project, None, (origin, origin_unread), boundary)
    }

    /// The scratch folder `listed` in `space`, in the scratch root `scratch_root`, its build folders
    /// found anywhere in it, itself included.
    pub(super) fn scratch(
        listed: ScratchFolder,
        space: ScratchSpace,
        boundary: &Boundary,
        scratch_root: &Path,
        may_follow: MayFollow,
    ) -> Tracked {
        let access = scan::folder_access(&listed.path, boundary);
        let (origin, origin_unread) =
            Origin::of_scratch_folder(&listed.path, access, scratch_root, may_follow);
        let provenance = origin.own_provenance(&listed.path);
        let project = new_project(listed.path, None, Some(space), access, provenance);
        Tracked::searched(
            project,
            Some(listed.inode),
            (origin, origin_unread),
            boundary,
        )
    }

    /// `project`, new to the map, its build folders found where its access lets Sutra look, with
    /// where it came from as its own files tell, `origin`. An unreadable folder is not entered: it
    /// may be a private folder whose first read raises the privacy prompt.
    fn searched(
        mut project: Project,
        inode: Option<u64>,
        (origin, origin_unread): (Origin, Vec<(PathBuf, io::ErrorKind)>),
        boundary: &Boundary,
    ) -> Tracked {
        let (artifacts, unlisted) = match project.access {
            FolderAccess::Readable => artifact_dirs(&project, boundary),
            _ => (Vec::new(), Vec::new()),
        };
        project.artifacts = artifacts;
        Tracked {
            project,
            git_activity: None,
            observed: None,
            agent_activity: None,
            environments: None,
            inode,
            unlisted,
            origin,
            origin_unread,
        }
    }

    /// Read where the scratch folder, in the scratch root `scratch_root`, came from again, as one
    /// kept on the map may have become a clone since, or come to hold one, or ceased to.
    pub(super) fn read_origin_again(&mut self, scratch_root: &Path, may_follow: MayFollow) {
        let project = &self.project;
        let (origin, unread) =
            Origin::of_scratch_folder(&project.path, project.access, scratch_root, may_follow);
        self.origin = origin;
        self.origin_unread = unread;
    }

    /// Keep what discovery cannot see from the project as it was before the rediscovery: the
    /// activity observed while watching, the environment, the processes listening, until the next
    /// read of them, and the sizes measured. Its own files keep their size only while the folder
    /// stays readable; a folder that turned unreadable shows none.
    pub(super) fn carry_over(&mut self, before: Tracked) {
        self.observed = before.observed;
        self.agent_activity = before.agent_activity;
        self.project.environment = before.project.environment;
        self.project.listening = before.project.listening;
        self.project.artifacts = keep_measurements(
            mem::take(&mut self.project.artifacts),
            &before.project.artifacts,
        );
        if self.project.access == FolderAccess::Readable
            && before.project.access == FolderAccess::Readable
        {
            self.project.own_files = before.project.own_files;
        }
    }

    /// Replace the environments describing this folder with those just read. Returns whether what
    /// they merge into changed (see `set_environment`).
    pub(super) fn replace_environments(
        &mut self,
        environments: Option<FolderEnvironments>,
        observed_at: Option<SystemTime>,
    ) -> bool {
        let merged = environments.as_ref().map(FolderEnvironments::merged);
        self.environments = environments;
        self.set_environment(merged, observed_at)
    }

    /// Probe the processes of the environments describing this folder, reading no file, and merge
    /// them again when one died or came back: another may speak for the folder now. Returns whether
    /// what they merge into changed. A process dying is no activity: it published nothing.
    pub(super) fn probe_environments(&mut self) -> bool {
        let Some(environments) = &mut self.environments else {
            return false;
        };
        if !environments.probe_processes() {
            return false;
        }
        let merged = environments.merged();
        self.set_environment(Some(merged), None)
    }

    /// Returns whether the environment changed. A change in what it publishes counts as activity
    /// at `observed_at`; a change in liveness alone does not, because liveness is probed with a
    /// signal rather than read from the state folder, so a process that died published nothing.
    fn set_environment(
        &mut self,
        env: Option<Environment>,
        observed_at: Option<SystemTime>,
    ) -> bool {
        let old = self.project.environment.as_ref();
        if old == env.as_ref() {
            return false;
        }
        let liveness_only = old
            .zip(env.as_ref())
            .is_some_and(|(old, new)| only_liveness_differs(old, new));
        if !liveness_only {
            if let Some(at) = observed_at {
                self.observed = Some((at, ActivitySource::Environment));
            }
        }
        self.project.environment = env;
        self.recompute_activity();
        true
    }

    pub(super) fn recompute_activity(&mut self) {
        let sessions = self.project.agents.iter().map(|agent| agent.last_activity);
        let subagents = self.project.subagents.iter();
        let agent = sessions
            .chain(subagents.map(|working| working.subagent.last_write))
            .flatten()
            .max();
        self.agent_activity = self.agent_activity.max(agent);
        let newest = [
            self.git_activity.map(|at| (at, ActivitySource::Git)),
            self.agent_activity.map(|at| (at, ActivitySource::Agent)),
            self.project
                .environment
                .as_ref()
                .and_then(environment_started)
                .map(|at| (at, ActivitySource::Environment)),
            self.observed,
        ]
        .into_iter()
        .flatten()
        .max_by_key(|(at, _)| *at);
        self.project.last_activity = newest.map(|(at, _)| at);
        self.project.activity_source = newest.map(|(_, source)| source);
    }
}

/// Whether two readings of one environment differ in liveness and nothing else.
fn only_liveness_differs(old: &Environment, new: &Environment) -> bool {
    old.alive != new.alive
        && *old
            == (Environment {
                alive: old.alive,
                ..new.clone()
            })
}

/// When the environment's process started. Devrunner exposes no status-file times, so this is
/// the only time an environment offers before the engine watches it change; it says nothing about
/// a long-running environment's recent work.
fn environment_started(env: &Environment) -> Option<SystemTime> {
    (env.started > 0).then(|| UNIX_EPOCH + Duration::from_secs(env.started))
}

#[cfg(test)]
impl Tracked {
    /// The folder at `path`, with `access`, as the map tracks a folder it has read nothing of: no
    /// git or package to say where it came from, no activity, environment or build folder.
    pub(super) fn bare(path: PathBuf, access: FolderAccess) -> Tracked {
        let origin = Origin::Folder { package: None };
        let provenance = origin.own_provenance(&path);
        Tracked {
            project: new_project(path, None, None, access, provenance),
            git_activity: None,
            observed: None,
            agent_activity: None,
            environments: None,
            inode: None,
            unlisted: Vec::new(),
            origin,
            origin_unread: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{agent_session, at, environment_at, subagent_at_work};
    use crate::map::{AgentSession, AgentStatus, Subagent, SubagentWithParent};

    fn tracked() -> Tracked {
        Tracked::bare(PathBuf::from("/code/app"), FolderAccess::Missing)
    }

    fn agent(last_activity: u64) -> AgentSession {
        AgentSession {
            last_activity: Some(at(last_activity)),
            ..agent_session(&last_activity.to_string(), AgentStatus::Unknown)
        }
    }

    fn environment(started: u64) -> Environment {
        Environment {
            started,
            ..environment_at("/code/app")
        }
    }

    #[test]
    fn activity_is_the_newest_source_and_names_it() {
        let mut project = tracked();
        project.git_activity = Some(at(100));
        project.project.agents = vec![agent(300), agent(200)];
        project.project.environment = Some(environment(250));
        project.recompute_activity();
        assert_eq!(project.project.last_activity, Some(at(300)));
        assert_eq!(project.project.activity_source, Some(ActivitySource::Agent));

        project.observed = Some((at(400), ActivitySource::FileChange));
        project.recompute_activity();
        assert_eq!(project.project.last_activity, Some(at(400)));
        assert_eq!(
            project.project.activity_source,
            Some(ActivitySource::FileChange)
        );
    }

    #[test]
    fn a_subagent_at_work_here_for_a_session_elsewhere_is_this_projects_agent_activity() {
        let mut project = tracked();
        project.git_activity = Some(at(100));
        project.project.agents = vec![agent(200)];
        let subagent = Subagent {
            last_write: Some(at(300)),
            ..subagent_at_work("a1")
        };
        let session = agent_session("elsewhere", AgentStatus::Idle);
        project.project.subagents = vec![SubagentWithParent::new(subagent, &session)];

        project.recompute_activity();

        assert_eq!(project.project.last_activity, Some(at(300)));
        assert_eq!(project.project.activity_source, Some(ActivitySource::Agent));
    }

    #[test]
    fn an_agents_activity_outlives_its_session_and_a_rediscovery() {
        let mut project = tracked();
        project.git_activity = Some(at(100));
        project.project.agents = vec![agent(500)];
        project.recompute_activity();
        assert_eq!(project.project.last_activity, Some(at(500)));

        project.project.agents = Vec::new();
        project.recompute_activity();
        assert_eq!(
            project.project.last_activity,
            Some(at(500)),
            "its session ended"
        );
        assert_eq!(project.project.activity_source, Some(ActivitySource::Agent));

        let mut found_again = tracked();
        found_again.carry_over(project);
        found_again.recompute_activity();
        assert_eq!(found_again.project.last_activity, Some(at(500)));
    }

    #[test]
    fn an_environment_with_no_start_time_contributes_no_activity() {
        let mut project = tracked();
        project.project.environment = Some(environment(0));
        project.recompute_activity();
        assert_eq!(project.project.last_activity, None);
        assert_eq!(project.project.activity_source, None);
    }

    #[test]
    fn a_published_environment_change_is_activity_but_a_liveness_change_is_not() {
        let mut project = tracked();
        project.project.environment = Some(environment(1));
        let mut revived = environment(1);
        revived.alive = true;
        assert!(project.set_environment(Some(revived), Some(at(500))));
        assert_eq!(project.observed, None);
        assert!(project.set_environment(Some(environment(2)), Some(at(600))));
        assert_eq!(
            project.observed,
            Some((at(600), ActivitySource::Environment))
        );
    }

    #[test]
    fn an_environment_read_again_unchanged_is_no_change() {
        let mut project = tracked();
        assert!(!project.set_environment(None, Some(at(400))));
        project.project.environment = Some(environment(1));
        assert!(!project.set_environment(Some(environment(1)), Some(at(500))));
        assert_eq!(project.observed, None);
    }
}
