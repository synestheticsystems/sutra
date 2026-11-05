//! The sizes the sizing pool reports, recorded on the map, and what each walk covers: a build
//! folder whole, and a project's own files without the folders measured on their own.

use std::collections::{BTreeSet, HashSet};
use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};

use super::State;
use crate::map::engine::sizing::Job;
use crate::map::{DirUsage, FolderAccess, Measurement, Project};

impl State {
    /// Every measurement the map holds, those of the most recently active projects first: each
    /// readable project's build folders, then its own files.
    pub(in crate::map::engine) fn jobs_newest_first(&self) -> Vec<Job> {
        self.jobs_where(|_| true)
    }

    /// The measurements with no size yet and none on its way. One whose last measurement failed
    /// waits for a change instead of being retried here.
    pub(in crate::map::engine) fn unmeasured(&self) -> Vec<Job> {
        self.jobs_where(|measurement| matches!(measurement, Measurement::Pending))
    }

    fn jobs_where(&self, wanted: impl Fn(&Measurement) -> bool) -> Vec<Job> {
        let mut projects: Vec<&Project> = self
            .projects
            .values()
            .map(|tracked| &tracked.project)
            .filter(|project| project.access == FolderAccess::Readable)
            .collect();
        projects.sort_by(|a, b| Project::newest_first(a, b));
        let mut jobs = Vec::new();
        for project in projects {
            let artifacts = project
                .artifacts
                .iter()
                .filter(|dir| wanted(&dir.measurement))
                .map(|dir| Job::Artifact(dir.path.clone()));
            jobs.extend(artifacts);
            if wanted(&project.own_files) {
                jobs.push(Job::OwnFiles(project.path.clone()));
            }
        }
        jobs
    }

    /// Whether `folder` is a build folder on the map.
    pub(in crate::map::engine) fn is_build_folder(&self, folder: &Path) -> bool {
        self.index.artifact_owner(folder).is_some()
    }

    /// Show that a measurement for `job` is queued or running. A measured folder keeps showing
    /// its last size until the new one lands.
    pub(in crate::map::engine) fn mark_measuring(&mut self, job: &Job) {
        if let Some(measurement) = self.measurement_mut(job) {
            if !matches!(measurement, Measurement::Measured(_)) {
                *measurement = Measurement::Measuring;
            }
        }
    }

    /// Record a finished measurement. Returns whether what it measured is still on the map.
    pub(in crate::map::engine) fn apply_usage(
        &mut self,
        job: &Job,
        usage: io::Result<DirUsage>,
    ) -> bool {
        match (job, usage) {
            // `cargo clean` or `rm -rf node_modules`: the directory is gone, so it leaves the map.
            (Job::Artifact(artifact), Err(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                let Some(owner) = self.index.artifact_owner(artifact).map(Path::to_path_buf) else {
                    return false;
                };
                if let Some(tracked) = self.projects.get_mut(&owner) {
                    tracked
                        .project
                        .artifacts
                        .retain(|dir| dir.path != *artifact);
                }
                self.reindex();
                true
            }
            (job, usage) => {
                let Some(measurement) = self.measurement_mut(job) else {
                    return false;
                };
                *measurement = match usage {
                    Ok(usage) => Measurement::Measured(usage),
                    Err(err) => Measurement::Failed(err.kind()),
                };
                true
            }
        }
    }

    fn measurement_mut(&mut self, job: &Job) -> Option<&mut Measurement> {
        match job {
            Job::Artifact(artifact) => {
                let owner = self.index.artifact_owner(artifact)?;
                let dir = self
                    .projects
                    .get_mut(owner)?
                    .project
                    .artifacts
                    .iter_mut()
                    .find(|dir| dir.path == *artifact)?;
                Some(&mut dir.measurement)
            }
            Job::OwnFiles(project) => Some(&mut self.projects.get_mut(project)?.project.own_files),
        }
    }

    /// What the walk of `project`'s own files leaves out: its build folders and the projects
    /// nested inside it, which are measured on their own, and the folders the scan skips.
    pub(in crate::map::engine) fn own_files_left_out(&self, project: &Path) -> HashSet<PathBuf> {
        let mut left_out: HashSet<PathBuf> =
            self.boundary.skipped_within(project).into_iter().collect();
        if let Some(tracked) = self.projects.get(project) {
            left_out.extend(tracked.project.artifacts.iter().map(|dir| dir.path.clone()));
        }
        left_out.extend(self.projects_inside(project).cloned());
        left_out
    }

    /// The projects on the map inside `folder`.
    fn projects_inside<'a>(&'a self, folder: &'a Path) -> impl Iterator<Item = &'a PathBuf> {
        // Paths order by component, so the projects inside `folder` follow it without a gap.
        self.projects
            .range::<Path, _>((Bound::Excluded(folder), Bound::Unbounded))
            .map(|(path, _)| path)
            .take_while(move |path| path.starts_with(folder))
    }

    /// Whether any project on the map lies inside `folder`.
    pub(in crate::map::engine) fn holds_projects(&self, folder: &Path) -> bool {
        self.projects_inside(folder).next().is_some()
    }

    /// Whether `project` is on the map and its folder reads, so walking it raises no prompt.
    pub(in crate::map::engine) fn is_readable(&self, project: &Path) -> bool {
        self.projects
            .get(project)
            .is_some_and(|tracked| tracked.project.access == FolderAccess::Readable)
    }

    /// The projects on the map that hold `path`, nearest first.
    pub(in crate::map::engine) fn enclosing_projects(&self, path: &Path) -> Vec<PathBuf> {
        path.ancestors()
            .skip(1)
            .filter(|folder| self.projects.contains_key(*folder))
            .map(Path::to_path_buf)
            .collect()
    }

    pub(in crate::map::engine) fn project_paths(&self) -> BTreeSet<PathBuf> {
        self.projects.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::new_project;
    use super::super::tracked::Tracked;
    use super::*;
    use crate::map::ScanOptions;
    use crate::map::grant::Roots;
    use crate::map::test_support::{at, cargo_target, usage};

    /// A map holding the projects at `paths`, each readable, with the build folders `artifacts`
    /// placed on the project that holds them, and none of them measured.
    fn state_with(paths: &[&str], artifacts: &[&str]) -> State {
        let roots = Roots {
            home: PathBuf::from("/nonexistent/home"),
            dirs: Vec::new(),
            sandboxed: false,
        };
        let mut state = State::new(&roots, &ScanOptions::default());
        for (n, path) in paths.iter().enumerate() {
            let path = PathBuf::from(path);
            let mut project =
                new_project(path.clone(), None, FolderAccess::Missing, &state.boundary);
            project.access = FolderAccess::Readable;
            // Later paths are more recently active.
            project.last_activity = Some(at(n as u64));
            project.artifacts = artifacts
                .iter()
                .map(PathBuf::from)
                .filter(|artifact| artifact.parent() == Some(&path))
                .map(|artifact| cargo_target(artifact, Measurement::Pending))
                .collect();
            let tracked = Tracked {
                project,
                git_activity: None,
                observed: None,
                agent_activity: None,
                environments: None,
            };
            state.projects.insert(path, tracked);
        }
        state.reindex();
        state
    }

    fn target_of(state: &State) -> Measurement {
        state.projects[Path::new("/code/app")].project.artifacts[0].measurement
    }

    #[test]
    fn a_failed_measurement_is_recorded_and_a_new_one_keeps_the_last_size_until_it_lands() {
        let target = Job::Artifact(PathBuf::from("/code/app/target"));
        let mut state = state_with(&["/code/app"], &["/code/app/target"]);
        assert!(state.apply_usage(&target, Ok(usage(10))));

        state.mark_measuring(&target);
        assert_eq!(target_of(&state).usage().map(|u| u.bytes), Some(10));

        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(state.apply_usage(&target, Err(denied)));
        assert!(matches!(
            target_of(&state),
            Measurement::Failed(io::ErrorKind::PermissionDenied)
        ));
        assert_eq!(
            state.unmeasured(),
            [Job::OwnFiles(PathBuf::from("/code/app"))],
            "a failed directory waits for a change"
        );

        state.mark_measuring(&target);
        assert!(matches!(target_of(&state), Measurement::Measuring));
    }

    #[test]
    fn a_build_folder_that_is_gone_leaves_the_map() {
        let target = Job::Artifact(PathBuf::from("/code/app/target"));
        let mut state = state_with(&["/code/app"], &["/code/app/target"]);
        let gone = io::Error::from(io::ErrorKind::NotFound);
        assert!(state.apply_usage(&target, Err(gone)));
        assert!(
            state.projects[Path::new("/code/app")]
                .project
                .artifacts
                .is_empty()
        );
    }

    #[test]
    fn a_projects_own_files_are_recorded_on_it() {
        let own_files = Job::OwnFiles(PathBuf::from("/code/app"));
        let mut state = state_with(&["/code/app"], &[]);
        state.mark_measuring(&own_files);
        let project = &state.projects[Path::new("/code/app")].project;
        assert_eq!(project.own_files, Measurement::Measuring);
        assert!(state.apply_usage(&own_files, Ok(usage(42))));
        let project = &state.projects[Path::new("/code/app")].project;
        assert_eq!(project.own_files.usage().map(|u| u.bytes), Some(42));
        assert!(!state.apply_usage(&Job::OwnFiles(PathBuf::from("/code/gone")), Ok(usage(1))));
    }

    #[test]
    fn jobs_list_build_folders_then_own_files_newest_project_first_and_skip_unreadable_ones() {
        let mut state = state_with(
            &["/code/old", "/code/sealed", "/code/new"],
            &["/code/new/target", "/code/old/target"],
        );
        state
            .projects
            .get_mut(Path::new("/code/sealed"))
            .unwrap()
            .project
            .access = FolderAccess::Denied;
        assert_eq!(
            state.jobs_newest_first(),
            [
                Job::Artifact(PathBuf::from("/code/new/target")),
                Job::OwnFiles(PathBuf::from("/code/new")),
                Job::Artifact(PathBuf::from("/code/old/target")),
                Job::OwnFiles(PathBuf::from("/code/old")),
            ]
        );
    }

    #[test]
    fn own_files_leave_out_the_build_folders_and_the_projects_nested_inside() {
        let state = state_with(
            &[
                "/code/app",
                "/code/app/.claude/worktrees/wt",
                "/code/app/vendor/lib",
                "/code/app-copy",
                "/code/app.old",
            ],
            &["/code/app/target"],
        );
        let left_out = state.own_files_left_out(Path::new("/code/app"));
        let expected: HashSet<PathBuf> = [
            "/code/app/target",
            "/code/app/.claude/worktrees/wt",
            "/code/app/vendor/lib",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(left_out, expected);
    }

    #[test]
    fn a_folder_holds_the_projects_inside_it_and_neither_itself_nor_a_sibling_sharing_its_name() {
        let state = state_with(&["/code/group/app", "/code/group-b"], &[]);
        assert!(state.holds_projects(Path::new("/code/group")));
        assert!(state.holds_projects(Path::new("/code")));
        assert!(!state.holds_projects(Path::new("/code/group/app")));
        assert!(!state.holds_projects(Path::new("/code/gro")));
    }

    #[test]
    fn the_projects_enclosing_a_path_are_named_nearest_first() {
        let state = state_with(&["/code/app", "/code/app/.claude/worktrees/wt"], &[]);
        assert_eq!(
            state.enclosing_projects(Path::new("/code/app/.claude/worktrees/wt/inner")),
            [
                PathBuf::from("/code/app/.claude/worktrees/wt"),
                PathBuf::from("/code/app")
            ]
        );
        assert!(state.enclosing_projects(Path::new("/code/app")).is_empty());
    }
}
