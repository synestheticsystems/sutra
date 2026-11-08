//! The map the engine keeps between snapshots: one entry per project, keyed by canonical path,
//! holding what each source contributed, so a refresh of one source leaves the others alone.

mod artifacts;
mod containers;
mod listening;
mod measurements;
mod placement;
mod scratch;
mod tracked;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::mem;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use rayon::prelude::*;

use super::route::Index;
use crate::map::devrunner::FolderEnvironments;
use crate::map::docker::{self, Container};
use crate::map::grant::Roots;
use crate::map::scan::{Boundary, MacOsAnswer};
use crate::map::{
    ActivitySource, AgentSession, FolderAccess, KeptOutBy, Measurement, PrivateFolder, Project,
    RepoInfo, ScanOptions, ScanStats, ScratchSpace, Snapshot, Source, SourceProblem,
    SubagentWithParent, agents, devrunner, processes, scan,
};
use placement::{Place, Placed};
use scratch::earns_a_card;
use tracked::Tracked;

pub(super) struct State {
    /// Canonical, like every path below: FSEvents reports canonical paths and every source is
    /// joined on them.
    home: PathBuf,
    /// The folders discovery walks.
    roots: Vec<PathBuf>,
    /// Claude Code's scratch root, whose sessions' scratch folders the map holds as projects of
    /// their own (see `scratch`), when this edition reads it.
    scratch_root: Option<PathBuf>,
    /// The sockets Docker may be served on, none in the store edition (see `Roots::docker`).
    docker_sockets: Vec<PathBuf>,
    sandboxed: bool,
    /// What every source may touch, discovery's walk included, built from the roots, the home
    /// folder and the scan options, and kept up with what macOS answers about the private folders.
    boundary: Boundary,
    /// Every project the engine tracks, the scratch folders that have no card among them (see
    /// `scratch::earns_a_card`), which the snapshot leaves out.
    projects: BTreeMap<PathBuf, Tracked>,
    unplaced_agents: Vec<AgentSession>,
    unplaced_subagents: Vec<SubagentWithParent>,
    /// What Docker is, as its reader last found it.
    docker: docker::Reading,
    /// The containers Docker said were running that are for no project on the map, by name.
    unplaced_containers: Vec<Container>,
    /// What each source reported when it was last read, ordered by source.
    problems: Vec<SourceProblem>,
    /// What the last rediscovery could not read, which the problems of `Source::Scan` hold with the
    /// folders the last search of each scratch folder could not list (see `report_unread`).
    unread_by_rediscovery: Vec<SourceProblem>,
    /// The folders discovery found publishing state of their own.
    state_folders: Vec<PathBuf>,
    /// What the agents' growing files said at the last read of the agents.
    agent_files: agents::LastRead,
    /// The commands of the processes the last read of them found listening.
    listener_commands: processes::LastRead,
    scan: ScanStats,
    index: Index,
}

impl State {
    /// An empty map over `roots`, staying out of each private folder the walk would enter until
    /// macOS answers whether Sutra may read it (`macos_answered`); `rediscover` fills it.
    pub(super) fn new(roots: &Roots, opts: &ScanOptions) -> State {
        let home = canonical(&roots.home);
        let dirs: Vec<PathBuf> = roots.dirs.iter().map(|root| canonical(root)).collect();
        let mut boundary = Boundary::new(&dirs, &home, opts);
        boundary.keep_out_until_macos_answers();
        let scratch_root = roots.scratch.as_deref().map(canonical);
        State {
            index: Index::new(&home, scratch_root.as_deref(), []),
            boundary,
            home,
            roots: dirs,
            scratch_root,
            docker_sockets: roots.docker.clone(),
            sandboxed: roots.sandboxed,
            projects: BTreeMap::new(),
            unplaced_agents: Vec::new(),
            unplaced_subagents: Vec::new(),
            docker: docker::Reading::Away,
            unplaced_containers: Vec::new(),
            problems: Vec::new(),
            unread_by_rediscovery: Vec::new(),
            state_folders: Vec::new(),
            agent_files: agents::LastRead::default(),
            listener_commands: processes::LastRead::default(),
            scan: ScanStats::default(),
        }
    }

    pub(super) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Running inside the App Sandbox, as the store edition does.
    pub(super) fn is_sandboxed(&self) -> bool {
        self.sandboxed
    }

    /// The sockets Docker may be served on, in the order tried; none in the store edition.
    pub(super) fn docker_sockets(&self) -> &[PathBuf] {
        &self.docker_sockets
    }

    pub(super) fn scratch_root(&self) -> Option<&Path> {
        self.scratch_root.as_deref()
    }

    /// The roots a file watcher can watch now: the folders discovery walks, and the scratch root
    /// while it is there. Claude Code makes it with the first session after the Mac starts, which
    /// is often after Sutra starts, so the engine looks again on its tick.
    pub(super) fn watched_roots(&self) -> Vec<PathBuf> {
        let scratch_root = self.scratch_root.as_ref().filter(|root| root.is_dir());
        self.roots.iter().chain(scratch_root).cloned().collect()
    }

    pub(super) fn index(&self) -> &Index {
        &self.index
    }

    /// The folders outside the roots and the scratch root that the map needs file events from: each
    /// readable project's folder, and each repository's shared git dir, which holds the git dirs of
    /// its worktrees. One inside another of them is left out, because a watch covers what lies below
    /// it. One that does not read is left out too: inside the store edition's sandbox every folder
    /// outside the grant is one, and watching one in a private folder would raise the privacy
    /// prompt. One that has gone stays, so that failing to watch it has discovery read again.
    pub(super) fn folders_outside_the_roots(&self) -> BTreeSet<PathBuf> {
        let outside = |folder: &Path| {
            let mut roots = self.roots.iter().chain(&self.scratch_root);
            !roots.any(|root| folder.starts_with(root))
        };
        let forbidden = |folder: &Path| {
            matches!(
                scan::folder_access(folder, &self.boundary),
                FolderAccess::Denied | FolderAccess::KeptOut(..)
            )
        };
        let folders: BTreeSet<&Path> = self
            .projects
            .values()
            .flat_map(|tracked| {
                let project = &tracked.project;
                let readable = project.access == FolderAccess::Readable;
                let folder = readable.then_some(project.path.as_path());
                let shared_git_dir = project
                    .repo
                    .as_ref()
                    .map(|repo| repo.common_dir.as_path())
                    .filter(|dir| outside(dir) && !forbidden(dir));
                folder.into_iter().chain(shared_git_dir)
            })
            .filter(|folder| outside(folder))
            .collect();
        let mut outermost = BTreeSet::new();
        let mut last: Option<&Path> = None;
        for folder in folders {
            // Paths order by component, so the folders inside one follow it without a gap.
            if !last.is_some_and(|last| folder.starts_with(last)) {
                outermost.insert(folder.to_path_buf());
                last = Some(folder);
            }
        }
        outermost
    }

    /// Whether Sutra may touch `path` at all, even to `stat` it (see `Boundary::may_read`).
    pub(super) fn may_read(&self, path: &Path) -> bool {
        self.boundary.may_read(path)
    }

    /// The private folders waiting for macOS to answer whether Sutra may read them, each with its
    /// path, for the engine to ask about.
    pub(super) fn awaiting_macos(&self) -> Vec<(PrivateFolder, PathBuf)> {
        self.boundary.awaiting_macos()
    }

    /// macOS answered whether Sutra may read the private folder `folder`. One it let in is entered
    /// by the next rediscovery. In one it refused nothing more can be read, so only what keeps its
    /// projects out has changed, and they say so at once.
    pub(super) fn macos_answered(&mut self, folder: PrivateFolder, answer: MacOsAnswer) {
        self.boundary.macos_answered(folder, answer);
        if answer == MacOsAnswer::Refused {
            let waiting = FolderAccess::KeptOut(folder, KeptOutBy::Prompt);
            for tracked in self.projects.values_mut() {
                if tracked.project.access == waiting {
                    tracked.project.access = FolderAccess::KeptOut(folder, KeptOutBy::MacOs);
                }
            }
        }
    }

    /// Rebuild the map from discovery and every source. What discovery cannot see carries over:
    /// artifact sizes, activity observed while watching, and the environment last seen, so the
    /// reload that follows can tell whether it changed. `observed_at` is `None` for the first
    /// scan, which has nothing to compare with.
    pub(super) fn rediscover(&mut self, observed_at: Option<SystemTime>) {
        // Without a root (the user declined the sandbox grant) Sutra may read nothing.
        if self.roots.is_empty() {
            return;
        }
        let started = Instant::now();
        let discovery = scan::discover(&self.boundary);
        let mut previous = mem::take(&mut self.projects);
        let boundary = &self.boundary;
        // Each worktree's search for build folders is metadata I/O like the walk before it.
        let mut projects: BTreeMap<PathBuf, Tracked> = scan::on_discovery_threads(|| {
            discovery
                .worktrees
                .into_par_iter()
                .map(|worktree| {
                    let tracked = Tracked::worktree(worktree, boundary);
                    (tracked.project.path.clone(), tracked)
                })
                .collect()
        });
        for (path, tracked) in &mut projects {
            if let Some(before) = previous.remove(path) {
                tracked.carry_over(before);
            }
        }
        let (scratch_folders, scratch_problems) = self.rediscover_scratch(&mut previous, &projects);
        // Plain folders come from environments; the reload below decides which stay. Each is read
        // again like a worktree, because no event may have told of a change to it.
        let plain_folders = previous
            .into_iter()
            .filter(|(_, before)| before.project.repo.is_none() && before.project.scratch.is_none())
            .map(|(path, before)| {
                let mut tracked = Tracked::plain(path.clone(), boundary);
                tracked.carry_over(before);
                (path, tracked)
            });
        projects.extend(plain_folders);
        projects.extend(scratch_folders);
        self.projects = projects;
        self.unread_by_rediscovery = [discovery.problems, scratch_problems].concat();
        self.report_unread();
        self.state_folders = discovery.state_folders;
        self.place_environments(observed_at);
        self.leave_build_folders_to_nested_projects();
        self.refresh_agents();
        for tracked in self.projects.values_mut() {
            tracked.recompute_activity();
        }
        self.reindex();
        self.scan = ScanStats {
            dirs_visited: discovery.stats.dirs_visited,
            elapsed: started.elapsed(),
        };
    }

    /// Reload the environments of both conventions and place each on its project folder. A
    /// folder an environment names that is no worktree is a project of its own. Returns whether
    /// anything changed.
    pub(super) fn refresh_environments(&mut self, observed_at: Option<SystemTime>) -> bool {
        let (changed, reshaped) = self.place_environments(observed_at);
        if reshaped {
            self.leave_build_folders_to_nested_projects();
            self.reindex();
        }
        changed
    }

    /// Returns whether anything changed, and whether plain folders joined or left the map.
    fn place_environments(&mut self, observed_at: Option<SystemTime>) -> (bool, bool) {
        let readable = self
            .projects
            .values()
            .filter(|tracked| tracked.project.access == FolderAccess::Readable)
            .map(|tracked| tracked.project.path.as_path())
            .chain(self.state_folders.iter().map(PathBuf::as_path));
        let loaded = devrunner::load(&self.home, readable, &self.boundary);
        let mut changed = self.report_problems(
            &[Source::Registry, Source::EnvironmentState],
            loaded.problems,
        );
        let mut by_dir: HashMap<PathBuf, FolderEnvironments> = loaded
            .folders
            .into_iter()
            .map(|folder| (folder.dir().to_path_buf(), folder))
            .collect();
        let before = self.projects.len();
        self.projects.retain(|path, tracked| {
            let project = &tracked.project;
            project.repo.is_some() || project.scratch.is_some() || by_dir.contains_key(path)
        });
        let mut reshaped = self.projects.len() != before;
        changed |= reshaped;
        for (path, tracked) in &mut self.projects {
            changed |= tracked.replace_environments(by_dir.remove(path), observed_at);
        }
        for (dir, environments) in by_dir {
            let mut tracked = Tracked::plain(dir.clone(), &self.boundary);
            tracked.replace_environments(Some(environments), observed_at);
            self.projects.insert(dir, tracked);
            reshaped = true;
            changed = true;
        }
        (changed, reshaped)
    }

    /// Every session on the map, in a project or elsewhere, as last read.
    fn sessions_on_the_map(&self) -> impl Iterator<Item = &AgentSession> {
        self.projects
            .values()
            .flat_map(|tracked| &tracked.project.agents)
            .chain(&self.unplaced_agents)
    }

    /// The sessions on the map of these processes, as last read.
    fn sessions_of(&self, processes: &[u32]) -> Vec<AgentSession> {
        self.sessions_on_the_map()
            .filter(|session| session.pid.is_some_and(|pid| processes.contains(&pid)))
            .cloned()
            .collect()
    }

    /// Reload every agent session and place each where it works (see `Place`), and each of its
    /// subagents at work where the subagent works, even beside the session (see
    /// `Place::of_subagent`); and hand each scratch folder its session while the session runs (see
    /// `find_scratch_sessions`). Returns whether anything changed.
    pub(super) fn refresh_agents(&mut self) -> bool {
        let loaded = agents::load(&self.home, &self.boundary, &mut self.agent_files);
        let mut changed = self.report_problems(
            &[
                Source::ClaudeSessions,
                Source::ClaudeSessionFile,
                Source::ClaudeSubagentMeta,
                Source::ClaudeSessionTranscript,
                Source::CodexDatabase,
            ],
            loaded.problems,
        );
        let mut placed: HashMap<PathBuf, Placed> = HashMap::new();
        let mut unplaced = Placed::default();
        let mut sessions = loaded.sessions;
        sessions.extend(self.sessions_of(&loaded.being_rewritten));
        changed |= self.find_scratch_sessions(&sessions);
        for session in sessions {
            for subagent in &session.subagents_at_work {
                let there = match Place::of_subagent(&self.projects, subagent) {
                    Place::Project(project) => placed.entry(project.to_path_buf()).or_default(),
                    Place::Elsewhere => &mut unplaced,
                };
                let working = SubagentWithParent::new(subagent.clone(), &session);
                there.subagents.push(working);
            }
            match Place::of(&self.projects, &session.cwd) {
                Place::Project(project) => placed.entry(project.to_path_buf()).or_default(),
                Place::Elsewhere => &mut unplaced,
            }
            .sessions
            .push(session);
        }
        unplaced.sort();
        changed |= self.unplaced_agents != unplaced.sessions
            || self.unplaced_subagents != unplaced.subagents;
        self.unplaced_agents = unplaced.sessions;
        self.unplaced_subagents = unplaced.subagents;
        for (path, tracked) in &mut self.projects {
            let mut here = placed.remove(path).unwrap_or_default();
            here.sort();
            let project = &mut tracked.project;
            if project.agents != here.sessions || project.subagents != here.subagents {
                project.agents = here.sessions;
                project.subagents = here.subagents;
                tracked.recompute_activity();
                changed = true;
            }
        }
        changed
    }

    /// Probe every process the environments name: one that crashed deletes no state file, so no
    /// event announces it. Returns whether any folder's environment changed.
    pub(super) fn refresh_liveness(&mut self) -> bool {
        let mut changed = false;
        for tracked in self.projects.values_mut() {
            changed |= tracked.probe_environments();
        }
        changed
    }

    /// Re-read one worktree's branch, head and git activity. Returns whether it is on the map.
    pub(super) fn refresh_git(&mut self, project: &Path) -> bool {
        let Some(tracked) = self.projects.get_mut(project) else {
            return false;
        };
        let Some(repo) = &mut tracked.project.repo else {
            return false;
        };
        // A worktree outside the grant cannot be resolved from its folder, but its admin dir in
        // the repository still reads. One being removed no longer resolves; the removal of its
        // admin dir triggers the rediscovery that takes it off the map.
        if tracked.project.access == FolderAccess::Readable {
            if let Some(fresh) = scan::repo_info(project) {
                *repo = fresh;
            }
        }
        tracked.git_activity = scan::git_activity(repo);
        tracked.recompute_activity();
        true
    }

    /// Record that a file in `project` was worked on at `at`. Returns whether that is news: the
    /// project has a card on the map, and nothing seen in it was as late.
    pub(super) fn touch(&mut self, project: &Path, at: SystemTime) -> bool {
        let Some(tracked) = self.projects.get_mut(project) else {
            return false;
        };
        if tracked.project.last_activity.is_some_and(|last| last >= at) {
            return false;
        }
        tracked.observed = Some((at, ActivitySource::FileChange));
        tracked.recompute_activity();
        earns_a_card(&tracked.project)
    }

    /// Report what discovery could not read: what the last rediscovery could not, and the folders
    /// in each scratch folder that its last search could not list. Returns whether that changed.
    fn report_unread(&mut self) -> bool {
        let unlisted = self.projects.values().flat_map(|tracked| &tracked.unlisted);
        let searches = SourceProblem::per_kind(Source::Scan, unlisted.cloned().collect());
        let unread = [self.unread_by_rediscovery.clone(), searches].concat();
        self.report_problems(&[Source::Scan], unread)
    }

    /// Replace the problems `sources` reported last time with `fresh`, what they reported now.
    /// Returns whether anything changed.
    pub(super) fn report_problems(
        &mut self,
        sources: &[Source],
        fresh: Vec<SourceProblem>,
    ) -> bool {
        let (old, mut kept): (Vec<_>, Vec<_>) = mem::take(&mut self.problems)
            .into_iter()
            .partition(|problem| sources.contains(&problem.source));
        let changed = old != fresh;
        kept.extend(fresh);
        // Stable, so the problems of one source keep the order it reported them in.
        kept.sort_by_key(|problem| problem.source);
        self.problems = kept;
        changed
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        let mut projects: Vec<Project> = self
            .projects
            .values()
            .map(|tracked| &tracked.project)
            .filter(|project| earns_a_card(project))
            .cloned()
            .collect();
        projects.sort_by(Project::newest_first);
        Snapshot {
            roots: self
                .roots
                .iter()
                .chain(&self.scratch_root)
                .cloned()
                .collect(),
            skipped_private_folders: self.boundary.skipped_private_folders(),
            refused_private_folders: self.boundary.refused_private_folders(),
            sandboxed: self.sandboxed,
            projects,
            unplaced_agents: self.unplaced_agents.clone(),
            unplaced_subagents: self.unplaced_subagents.clone(),
            docker: self.docker_on_the_map(),
            problems: self.problems.clone(),
            scan: self.scan.clone(),
            generated_at: SystemTime::now(),
        }
    }

    fn reindex(&mut self) {
        let projects = self.projects.values().map(|t| &t.project);
        self.index = Index::new(&self.home, self.scratch_root.as_deref(), projects);
    }
}

/// A project at `path`: a worktree of `repo`, a session's scratch folder (`scratch`), or a plain
/// folder when neither, with nothing found or measured in it yet (see `Tracked::searched`).
fn new_project(
    path: PathBuf,
    repo: Option<RepoInfo>,
    scratch: Option<ScratchSpace>,
    access: FolderAccess,
) -> Project {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Project {
        path,
        name,
        repo,
        scratch,
        last_activity: None,
        activity_source: None,
        agents: Vec::new(),
        subagents: Vec::new(),
        environment: None,
        listening: Vec::new(),
        containers: Vec::new(),
        artifacts: Vec::new(),
        own_files: Measurement::Pending,
        access,
    }
}

/// The project an agent working in `path` works on: the deepest project holding `path`, or, when
/// none does, the one project `path` holds. Codex Desktop records a task's folder as the thread's
/// working directory, one level above the worktree the task checks out in it (`<task>/work/<repo>`).
/// A folder holding several projects is no one project's.
fn owner<'a>(projects: &'a BTreeMap<PathBuf, Tracked>, path: &'a Path) -> Option<&'a Path> {
    if let Some(holder) = holder(projects, path) {
        return Some(holder);
    }
    // Paths order by component, so the projects inside `path` follow it in a run.
    let mut held = projects
        .range::<Path, _>((Bound::Excluded(path), Bound::Unbounded))
        .take_while(|(project, _)| project.starts_with(path))
        .map(|(project, _)| project.as_path());
    match (held.next(), held.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// The deepest project on the map holding `folder`, itself included: where a process running in
/// it, or a container for it, shows.
fn holder<'a>(projects: &'a BTreeMap<PathBuf, Tracked>, folder: &Path) -> Option<&'a Path> {
    let mut held = folder
        .ancestors()
        .filter_map(|ancestor| projects.get_key_value(ancestor));
    held.next().map(|(path, _)| path.as_path())
}

fn canonical(path: &Path) -> PathBuf {
    // A root that cannot be resolved (outside the grant) is kept as given; discovery then
    // counts it as unreadable.
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::time::UNIX_EPOCH;

    use super::*;
    use crate::map::AgentStatus;
    use crate::map::agents::TestCall;
    use crate::map::test_support::TempDir;

    fn tracked(path: &str) -> Tracked {
        Tracked {
            project: new_project(PathBuf::from(path), None, None, FolderAccess::Missing),
            git_activity: None,
            observed: None,
            agent_activity: None,
            environments: None,
            inode: None,
            unlisted: Vec::new(),
        }
    }

    #[test]
    fn a_folder_is_held_by_the_deepest_project_holding_it_and_by_none_when_none_holds_it() {
        let projects: BTreeMap<PathBuf, Tracked> = ["/code/app", "/code/app/.claude/wt"]
            .into_iter()
            .map(|path| (PathBuf::from(path), tracked(path)))
            .collect();
        let holder = |folder: &str| holder(&projects, Path::new(folder));
        assert_eq!(holder("/code/app/web"), Some(Path::new("/code/app")));
        assert_eq!(holder("/code/app"), Some(Path::new("/code/app")));
        assert_eq!(
            holder("/code/app/.claude/wt/server"),
            Some(Path::new("/code/app/.claude/wt"))
        );
        assert_eq!(
            holder("/code"),
            None,
            "a folder holding a project is no project's"
        );
        assert_eq!(holder("/"), None);
    }

    #[test]
    fn owner_is_the_deepest_project_holding_the_path() {
        let projects: BTreeMap<PathBuf, Tracked> = ["/code/app", "/code/app/.claude/worktrees/wt"]
            .into_iter()
            .map(|path| (PathBuf::from(path), tracked(path)))
            .collect();
        assert_eq!(
            owner(&projects, Path::new("/code/app/.claude/worktrees/wt/src")),
            Some(Path::new("/code/app/.claude/worktrees/wt"))
        );
        assert_eq!(
            owner(&projects, Path::new("/code/app/src")),
            Some(Path::new("/code/app"))
        );
        assert_eq!(owner(&projects, Path::new("/code/other")), None);
    }

    #[test]
    fn a_folder_in_no_project_is_the_one_project_s_it_holds_and_none_s_when_it_holds_more() {
        let projects: BTreeMap<PathBuf, Tracked> = [
            "/tasks/fix/work/app",
            "/tasks/pair/work/api",
            "/tasks/pair/work/web",
            "/tasks/pair-2/work/app",
        ]
        .into_iter()
        .map(|path| (PathBuf::from(path), tracked(path)))
        .collect();
        assert_eq!(
            owner(&projects, Path::new("/tasks/fix")),
            Some(Path::new("/tasks/fix/work/app")),
            "the task folder Codex Desktop records holds one worktree"
        );
        assert_eq!(owner(&projects, Path::new("/tasks/pair")), None);
        assert_eq!(owner(&projects, Path::new("/tasks")), None);
        assert_eq!(
            owner(&projects, Path::new("/tasks/pair-2")),
            Some(Path::new("/tasks/pair-2/work/app")),
            "a folder whose name begins like another's holds only its own"
        );
    }

    /// A state reading the fixture `home`, with `projects` on its map.
    fn state_with(home: &TempDir, projects: &[&str]) -> State {
        let roots = Roots {
            home: home.root().to_path_buf(),
            dirs: Vec::new(),
            scratch: None,
            docker: Vec::new(),
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());
        for path in projects {
            state.projects.insert(PathBuf::from(path), tracked(path));
        }
        state
    }

    /// Registers the live, idle session `0b1d` of this process working in `cwd`, under `home`,
    /// which started ten minutes ago. Returns the moment its subagents write.
    fn live_session(home: &TempDir, cwd: &str) -> SystemTime {
        let me = std::process::id();
        let now = SystemTime::now();
        let started = now - std::time::Duration::from_secs(600);
        let ms = |at: SystemTime| at.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let registry = serde_json::json!({
            "pid": me, "sessionId": "0b1d", "cwd": cwd, "status": "idle",
            "startedAt": ms(started), "updatedAt": ms(started),
        });
        home.write(&format!(".claude/sessions/{me}.json"), registry.to_string());
        now
    }

    /// The live session `0b1d` working in `cwd`, under `home`, with a subagent at work in each of
    /// `subagents`, run there and acting nowhere else, named after the folder's last part.
    fn session_with_subagents(home: &TempDir, cwd: &str, subagents: &[&str]) {
        let now = live_session(home, cwd);
        for works_in in subagents {
            let agent = Path::new(works_in).file_name().unwrap().to_str().unwrap();
            agents::write_test_subagent_at_work(home.root(), cwd, "0b1d", agent, works_in, now);
        }
    }

    /// The live session `0b1d` working in `cwd`, under `home`, with a subagent at work for each of
    /// `subagents`, by id with the tool calls it made, run in the session's folder.
    fn session_with_acting_subagents(home: &TempDir, cwd: &str, subagents: &[(&str, &[TestCall])]) {
        let now = live_session(home, cwd);
        for (agent, calls) in subagents {
            agents::write_test_subagent_acting(home.root(), cwd, "0b1d", agent, calls, now);
        }
    }

    /// The subagents the map shows at `path`, or elsewhere for `None`.
    fn shown_at<'a>(state: &'a State, path: Option<&str>) -> &'a [SubagentWithParent] {
        match path {
            Some(path) => &state.projects[Path::new(path)].project.subagents,
            None => &state.unplaced_subagents,
        }
    }

    /// The subagents the map shows at `path`, or elsewhere for `None`, by the folder Claude Code
    /// runs each in.
    fn subagents_at(state: &State, path: Option<&str>) -> Vec<PathBuf> {
        let subagents = shown_at(state, path).iter();
        subagents.map(|one| one.subagent.cwd.clone()).collect()
    }

    /// The subagents the map shows at `path`, or elsewhere for `None`, by id.
    fn ids_at<'a>(state: &'a State, path: Option<&str>) -> Vec<&'a str> {
        let subagents = shown_at(state, path).iter();
        subagents.map(|one| one.subagent.id.as_str()).collect()
    }

    #[test]
    fn a_subagent_shows_on_the_card_of_the_newest_project_it_entered_else_edited_in_else_runs_in() {
        let home = TempDir::new("state-subagents-where-they-act");
        let (seed, polish, notes) = ("/code/app-wt/seed", "/code/app-wt/polish", "/code/notes");
        use TestCall::{Command, Edit};
        session_with_acting_subagents(
            &home,
            "/code/app",
            &[
                (
                    "seed",
                    &[
                        Command("cd /code/app-wt/seed && cargo test"),
                        Edit("/code/notes/seed.md"),
                    ],
                ),
                (
                    "side-trip",
                    &[
                        Command("cd /code/app-wt/seed && cargo build"),
                        Command("cd /nonexistent/scratch && ls"),
                    ],
                ),
                (
                    "unexpanded",
                    &[
                        Command("cd /code/app-wt/polish && cargo test"),
                        Command("cd $WT/fix-2 && git log"),
                        Edit("/code/notes/polish.md"),
                    ],
                ),
                (
                    "relative",
                    &[
                        Command("cd crates/core && cargo test"),
                        Edit("/code/notes/core.md"),
                    ],
                ),
                (
                    "reviewer",
                    &[
                        Command("cd /nonexistent/clone && cargo test"),
                        Edit("/nonexistent/clone/review.md"),
                    ],
                ),
                ("triage", &[]),
            ],
        );
        let mut state = state_with(&home, &["/code/app", seed, polish, notes]);

        assert!(state.refresh_agents());

        assert_eq!(
            ids_at(&state, Some(seed)),
            ["seed", "side-trip"],
            "where each went, past the notes it wrote and the folder in no project it went to"
        );
        assert_eq!(
            ids_at(&state, Some(polish)),
            ["unexpanded"],
            "past a cd into a variable, the cd before"
        );
        assert_eq!(
            ids_at(&state, Some(notes)),
            ["relative"],
            "a relative cd names no folder, so the file it edited tells"
        );
        assert_eq!(
            ids_at(&state, Some("/code/app")),
            ["reviewer", "triage"],
            "acting only outside every project, or nowhere yet, it shows where it runs"
        );
        assert_eq!(ids_at(&state, None), [""; 0]);
        assert!(!state.refresh_agents(), "the same subagents read again");
    }

    #[test]
    fn each_subagent_at_work_shows_where_it_works_beside_its_session_too_and_counts_on_it() {
        let home = TempDir::new("state-subagents-where-they-work");
        let worktree = "/code/app/.claude/worktrees/agent-a1";
        let beside = "/code/app/src";
        let outside = "/nonexistent/scratch";
        session_with_subagents(&home, "/code/app", &[worktree, beside, outside]);
        let mut state = state_with(&home, &["/code/app", worktree]);

        assert!(state.refresh_agents());

        let app = &state.projects[Path::new("/code/app")].project;
        let [session] = app.agents.as_slice() else {
            panic!("the session works in its project: {:?}", app.agents);
        };
        assert_eq!(session.status, AgentStatus::Busy);
        assert_eq!(
            session.subagents_at_work.len(),
            3,
            "it still counts them all"
        );
        assert_eq!(
            subagents_at(&state, Some("/code/app")),
            [PathBuf::from(beside)],
            "the subagent beside its session shows there too"
        );
        assert_eq!(
            subagents_at(&state, Some(worktree)),
            [PathBuf::from(worktree)]
        );
        assert_eq!(subagents_at(&state, None), [PathBuf::from(outside)]);
        let in_worktree = &state.projects[Path::new(worktree)].project.subagents[0];
        assert_eq!(in_worktree.parent.session_id, "0b1d");
        assert!(!state.refresh_agents(), "the same subagents read again");
    }

    #[test]
    fn a_subagent_of_a_session_elsewhere_shows_where_it_works_inside_its_sessions_folder_or_not() {
        let home = TempDir::new("state-subagents-elsewhere");
        let session = "/nonexistent/scratch";
        let inside = "/nonexistent/scratch/deeper";
        let other = "/nonexistent/other";
        let project = "/code/app/web";
        session_with_subagents(&home, session, &[inside, other, project]);
        let mut state = state_with(&home, &["/code/app"]);

        state.refresh_agents();

        assert_eq!(state.unplaced_agents.len(), 1);
        assert_eq!(
            subagents_at(&state, None),
            [PathBuf::from(inside), PathBuf::from(other)]
        );
        assert_eq!(
            subagents_at(&state, Some("/code/app")),
            [PathBuf::from(project)]
        );
    }

    #[test]
    fn an_agent_at_work_in_a_scratch_folder_stands_on_it_which_has_a_card_while_one_does() {
        let home = TempDir::new("state-agents-scratch");
        let session = "/private/tmp/claude-501/-code-app/e26ca359";
        let r567 = "/private/tmp/claude-501/-code-app/e26ca359/scratchpad/r567";
        let probe = "/private/tmp/claude-501/-code-app/e26ca359/scratchpad/probe";
        let review = "/private/tmp/claude-501/-code-app/e26ca359/scratchpad/review";
        let in_r567 = "/private/tmp/claude-501/-code-app/e26ca359/scratchpad/r567/src";
        session_with_subagents(&home, "/code/app", &[in_r567, probe]);
        let mut state = state_with(&home, &["/code/app"]);
        for (path, build_folders) in [(r567, &["target"][..]), (probe, &[]), (review, &[])] {
            let mut tracked = tracked(path);
            let project = &mut tracked.project;
            project.scratch = Some(ScratchSpace {
                session_folder: PathBuf::from(session),
                session_id: "e26ca359".to_string(),
                started_in: "app".to_string(),
                session: None,
                session_name: None,
            });
            project.artifacts = build_folders
                .iter()
                .map(|dir| {
                    crate::map::test_support::cargo_target(
                        project.path.join(dir),
                        Measurement::Pending,
                    )
                })
                .collect();
            state.projects.insert(PathBuf::from(path), tracked);
        }
        let carded = |state: &State| {
            let snapshot = state.snapshot();
            let scratch = snapshot
                .projects
                .into_iter()
                .filter(|p| p.scratch.is_some());
            let mut carded: Vec<PathBuf> = scratch.map(|project| project.path).collect();
            carded.sort();
            carded
        };
        assert_eq!(carded(&state), [PathBuf::from(r567)]);

        assert!(state.refresh_agents());
        assert_eq!(subagents_at(&state, Some(r567)), [PathBuf::from(in_r567)]);
        assert_eq!(
            subagents_at(&state, Some(probe)),
            [PathBuf::from(probe)],
            "one with no build folder too"
        );
        assert!(subagents_at(&state, None).is_empty());
        assert_eq!(carded(&state), [probe, r567].map(PathBuf::from));

        live_session(&home, review);
        assert!(state.refresh_agents());
        let on_review = &state.projects[Path::new(review)].project.agents;
        assert_eq!(on_review.len(), 1, "a session working there stands there");
        assert!(carded(&state).contains(&PathBuf::from(review)));

        std::fs::remove_file(home.path(&format!(".claude/sessions/{}.json", std::process::id())))
            .unwrap();
        assert!(state.refresh_agents());
        assert_eq!(
            carded(&state),
            [PathBuf::from(r567)],
            "the others go with their agents"
        );
    }

    #[test]
    fn report_problems_replaces_only_the_sources_named_and_says_whether_they_changed() {
        let roots = Roots {
            home: PathBuf::from("/nonexistent/home"),
            dirs: Vec::new(),
            scratch: None,
            docker: Vec::new(),
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());
        let problem = |source| {
            SourceProblem::new(source, PathBuf::from("/x"), io::ErrorKind::PermissionDenied)
        };

        assert!(state.report_problems(&[Source::Watcher], vec![problem(Source::Watcher)]));
        assert!(state.report_problems(&[Source::Scan], vec![problem(Source::Scan)]));
        assert!(!state.report_problems(&[Source::Watcher], vec![problem(Source::Watcher)]));
        assert_eq!(
            state.snapshot().problems,
            [problem(Source::Scan), problem(Source::Watcher)]
        );
        assert!(state.report_problems(&[Source::Scan], Vec::new()));
        assert_eq!(state.snapshot().problems, [problem(Source::Watcher)]);
    }

    #[test]
    fn a_session_file_that_cannot_be_read_is_reported_until_it_reads_again() {
        let fixture = TempDir::new("state-session-problem");
        let me = std::process::id();
        let sealed = fixture.write(
            &format!(".claude/sessions/{me}.json"),
            format!(r#"{{"pid": {me}, "sessionId": "s", "cwd": "/", "status": "busy"}}"#),
        );
        fixture.restrict(&sealed, 0o000);
        let roots = Roots {
            home: fixture.root().to_path_buf(),
            dirs: Vec::new(),
            scratch: None,
            docker: Vec::new(),
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());

        assert!(state.refresh_agents());
        assert_eq!(
            state.snapshot().problems,
            [SourceProblem::new(
                Source::ClaudeSessionFile,
                sealed.clone(),
                io::ErrorKind::PermissionDenied
            )]
        );
        assert!(!state.refresh_agents(), "the same problem again");

        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(state.refresh_agents());
        assert_eq!(state.snapshot().problems, []);
        assert_eq!(state.snapshot().unplaced_agents.len(), 1);
    }

    #[test]
    fn a_session_whose_file_is_mid_rewrite_stays_as_it_was_and_goes_once_its_file_does() {
        let home = TempDir::new("state-agents-rewriting");
        let me = std::process::id();
        let registry = format!(".claude/sessions/{me}.json");
        home.write(
            &registry,
            format!(r#"{{"pid": {me}, "sessionId": "s", "cwd": "/", "status": "busy"}}"#),
        );
        let roots = Roots {
            home: home.root().to_path_buf(),
            dirs: Vec::new(),
            scratch: None,
            docker: Vec::new(),
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());
        assert!(state.refresh_agents());
        assert_eq!(state.snapshot().unplaced_agents.len(), 1);

        home.write(&registry, "{\"pid\": ");
        assert!(!state.refresh_agents(), "nothing has changed yet");
        assert_eq!(state.snapshot().unplaced_agents.len(), 1);

        std::fs::remove_file(home.path(&registry)).unwrap();
        assert!(state.refresh_agents());
        assert_eq!(state.snapshot().unplaced_agents, []);
    }

    #[test]
    fn refresh_agents_reports_a_change_only_when_a_session_changed() {
        let home = TempDir::new("state-agents");
        let me = std::process::id();
        let session = |status: &str| {
            format!(r#"{{"pid": {me}, "sessionId": "s", "cwd": "/", "status": "{status}"}}"#)
        };
        let registry = format!(".claude/sessions/{me}.json");
        home.write(&registry, session("busy"));
        let roots = Roots {
            home: home.root().to_path_buf(),
            dirs: Vec::new(),
            scratch: None,
            docker: Vec::new(),
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());

        assert!(state.refresh_agents(), "a new session");
        assert!(!state.refresh_agents(), "the same session read again");
        home.write(&registry, session("waiting"));
        assert!(state.refresh_agents(), "a status change");
    }
}
