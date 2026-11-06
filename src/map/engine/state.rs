//! The map the engine keeps between snapshots: one entry per project, keyed by canonical path,
//! holding what each source contributed, so a refresh of one source leaves the others alone.

mod artifacts;
mod measurements;
mod placement;
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
use crate::map::grant::Roots;
use crate::map::scan::Boundary;
use crate::map::{
    ActivitySource, AgentSession, FolderAccess, Measurement, Project, RepoInfo, ScanOptions,
    ScanStats, Snapshot, Source, SourceProblem, SubagentWithParent, agents, devrunner, scan,
};
use artifacts::artifact_dirs;
use placement::{Place, Placed};
use tracked::Tracked;

pub(super) struct State {
    /// Canonical, like every path below: FSEvents reports canonical paths and every source is
    /// joined on them.
    home: PathBuf,
    roots: Vec<PathBuf>,
    sandboxed: bool,
    opts: ScanOptions,
    /// What every source may touch, built from the roots, the home folder and `opts`.
    boundary: Boundary,
    projects: BTreeMap<PathBuf, Tracked>,
    unplaced_agents: Vec<AgentSession>,
    unplaced_subagents: Vec<SubagentWithParent>,
    /// What each source reported when it was last read, ordered by source.
    problems: Vec<SourceProblem>,
    /// The folders discovery found publishing state of their own.
    state_folders: Vec<PathBuf>,
    /// What the agents' growing files said at the last read of the agents.
    agent_files: agents::LastRead,
    scan: ScanStats,
    index: Index,
}

impl State {
    /// An empty map over `roots`; `rediscover` fills it.
    pub(super) fn new(roots: &Roots, opts: &ScanOptions) -> State {
        let home = canonical(&roots.home);
        let dirs: Vec<PathBuf> = roots.dirs.iter().map(|root| canonical(root)).collect();
        State {
            index: Index::new(&home, []),
            boundary: Boundary::new(&dirs, &home, opts),
            home,
            roots: dirs,
            sandboxed: roots.sandboxed,
            opts: opts.clone(),
            projects: BTreeMap::new(),
            unplaced_agents: Vec::new(),
            unplaced_subagents: Vec::new(),
            problems: Vec::new(),
            state_folders: Vec::new(),
            agent_files: agents::LastRead::default(),
            scan: ScanStats::default(),
        }
    }

    pub(super) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(super) fn index(&self) -> &Index {
        &self.index
    }

    /// The folders outside the roots that the map needs file events from: each readable project's
    /// folder, and each repository's shared git dir, which holds the git dirs of its worktrees. One
    /// inside another of them is left out, because a watch covers what lies below it. One that does
    /// not read is left out too: inside the store edition's sandbox every folder outside the grant
    /// is one, and watching one in a private folder would raise the privacy prompt. One that has
    /// gone stays, so that failing to watch it has discovery read again.
    pub(super) fn folders_outside_the_roots(&self) -> BTreeSet<PathBuf> {
        let outside = |folder: &Path| !self.roots.iter().any(|root| folder.starts_with(root));
        let forbidden = |folder: &Path| {
            matches!(
                scan::folder_access(folder, &self.boundary),
                FolderAccess::Denied | FolderAccess::KeptOut(_)
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
        let discovery = scan::discover(&self.roots, &self.home, &self.opts);
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
        // Plain folders come from environments; the reload below decides which stay. Each is read
        // again like a worktree, because no event may have told of a change to it.
        let plain_folders = previous
            .into_iter()
            .filter(|(_, before)| before.project.repo.is_none())
            .map(|(path, before)| {
                let mut tracked = Tracked::plain(path.clone(), boundary);
                tracked.carry_over(before);
                (path, tracked)
            });
        projects.extend(plain_folders);
        self.projects = projects;
        self.report_problems(&[Source::Scan], discovery.problems);
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
        self.projects
            .retain(|path, tracked| tracked.project.repo.is_some() || by_dir.contains_key(path));
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

    /// The sessions on the map of these processes, as last read.
    fn sessions_of(&self, processes: &[u32]) -> Vec<AgentSession> {
        let on_the_map = self
            .projects
            .values()
            .flat_map(|tracked| &tracked.project.agents)
            .chain(&self.unplaced_agents);
        on_the_map
            .filter(|session| session.pid.is_some_and(|pid| processes.contains(&pid)))
            .cloned()
            .collect()
    }

    /// Reload every agent session and place each where it works (see `Place`), and each of its
    /// subagents at work where the subagent works, even beside the session (see
    /// `SubagentWithParent`). Returns whether anything changed.
    pub(super) fn refresh_agents(&mut self) -> bool {
        let loaded = agents::load(&self.home, &self.boundary, &mut self.agent_files);
        let mut changed = self.report_problems(
            &[
                Source::ClaudeSessions,
                Source::ClaudeSessionFile,
                Source::ClaudeSubagentMeta,
                Source::CodexDatabase,
            ],
            loaded.problems,
        );
        let mut placed: HashMap<PathBuf, Placed> = HashMap::new();
        let mut unplaced = Placed::default();
        let mut sessions = loaded.sessions;
        sessions.extend(self.sessions_of(&loaded.being_rewritten));
        for session in sessions {
            for subagent in &session.subagents_at_work {
                let there = match Place::of(&self.projects, &subagent.cwd) {
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

    /// Record a file change in `project` at `at`. Returns whether the project is on the map.
    pub(super) fn touch(&mut self, project: &Path, at: SystemTime) -> bool {
        let Some(tracked) = self.projects.get_mut(project) else {
            return false;
        };
        tracked.observed = Some((at, ActivitySource::FileChange));
        tracked.recompute_activity();
        true
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
            .map(|tracked| tracked.project.clone())
            .collect();
        projects.sort_by(Project::newest_first);
        Snapshot {
            roots: self.roots.clone(),
            skipped_private_folders: self.boundary.skipped_private_folders(),
            sandboxed: self.sandboxed,
            projects,
            unplaced_agents: self.unplaced_agents.clone(),
            unplaced_subagents: self.unplaced_subagents.clone(),
            problems: self.problems.clone(),
            scan: self.scan.clone(),
            generated_at: SystemTime::now(),
        }
    }

    fn reindex(&mut self) {
        self.index = Index::new(&self.home, self.projects.values().map(|t| &t.project));
    }
}

fn new_project(
    path: PathBuf,
    repo: Option<RepoInfo>,
    access: FolderAccess,
    boundary: &Boundary,
) -> Project {
    // An unreadable folder is not entered: it may be a private folder whose first read raises the
    // privacy prompt.
    let artifacts = if access == FolderAccess::Readable {
        artifact_dirs(&path, boundary)
    } else {
        Vec::new()
    };
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Project {
        path,
        name,
        repo,
        last_activity: None,
        activity_source: None,
        agents: Vec::new(),
        subagents: Vec::new(),
        environment: None,
        artifacts,
        own_files: Measurement::Pending,
        access,
    }
}

/// The project an agent working in `path` works on: the deepest project holding `path`, or, when
/// none does, the one project `path` holds. Codex Desktop records a task's folder as the thread's
/// working directory, one level above the worktree the task checks out in it
/// (`<task>/work/<repo>`). A folder holding several projects is no one project's.
fn owner<'a>(projects: &'a BTreeMap<PathBuf, Tracked>, path: &'a Path) -> Option<&'a Path> {
    if let Some(holder) = path
        .ancestors()
        .find(|folder| projects.contains_key(*folder))
    {
        return Some(holder);
    }
    // Paths order by component, so the projects inside `path` follow it in a run.
    let mut held = projects
        .range::<Path, _>((Bound::Excluded(path), Bound::Unbounded))
        .map(|(project, _)| project.as_path())
        .take_while(|project| project.starts_with(path));
    match (held.next(), held.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
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
    use crate::map::test_support::TempDir;

    fn tracked(path: &str) -> Tracked {
        let boundary = Boundary::new(&[], Path::new("/"), &ScanOptions::default());
        Tracked {
            project: new_project(PathBuf::from(path), None, FolderAccess::Missing, &boundary),
            git_activity: None,
            observed: None,
            agent_activity: None,
            environments: None,
        }
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
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());
        for path in projects {
            state.projects.insert(PathBuf::from(path), tracked(path));
        }
        state
    }

    /// The live, idle session `0b1d` of this process working in `cwd`, under `home`, with a
    /// subagent at work in each of `subagents`, named after the folder's last part.
    fn session_with_subagents(home: &TempDir, cwd: &str, subagents: &[&str]) {
        let me = std::process::id();
        let now = SystemTime::now();
        let started = now - std::time::Duration::from_secs(600);
        let ms = |at: SystemTime| at.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let registry = serde_json::json!({
            "pid": me, "sessionId": "0b1d", "cwd": cwd, "status": "idle",
            "startedAt": ms(started), "updatedAt": ms(started),
        });
        home.write(&format!(".claude/sessions/{me}.json"), registry.to_string());
        for works_in in subagents {
            let agent = Path::new(works_in).file_name().unwrap().to_str().unwrap();
            agents::write_test_subagent_at_work(home.root(), cwd, "0b1d", agent, works_in, now);
        }
    }

    /// The subagents the map shows at `path`, or elsewhere for `None`, by the folder each works in.
    fn subagents_at(state: &State, path: Option<&str>) -> Vec<PathBuf> {
        let subagents = match path {
            Some(path) => &state.projects[Path::new(path)].project.subagents,
            None => &state.unplaced_subagents,
        };
        subagents
            .iter()
            .map(|one| one.subagent.cwd.clone())
            .collect()
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
    fn report_problems_replaces_only_the_sources_named_and_says_whether_they_changed() {
        let roots = Roots {
            home: PathBuf::from("/nonexistent/home"),
            dirs: Vec::new(),
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
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());

        assert!(state.refresh_agents(), "a new session");
        assert!(!state.refresh_agents(), "the same session read again");
        home.write(&registry, session("waiting"));
        assert!(state.refresh_agents(), "a status change");
    }
}
