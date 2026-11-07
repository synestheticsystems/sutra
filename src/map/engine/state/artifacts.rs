//! The map's artifact directories: finding them again when a project's folders change, and keeping
//! each on one project.

use std::io;
use std::path::{Path, PathBuf};

use super::{State, owner};
use crate::map::scan::{self, Boundary};
use crate::map::{ArtifactDir, Measurement, Project};

impl State {
    /// Look for artifact directories in one project again, after a folder appeared or vanished in
    /// it. Sizes of the ones still there are kept, and so is each one in a folder the search could
    /// not list, which may be there still. Returns whether the map changed: the set of build
    /// folders, or the folders the search could not list, which are reported then.
    pub(in crate::map::engine) fn refind_artifacts(&mut self, project: &Path) -> bool {
        let Some(tracked) = self.projects.get(project) else {
            return false;
        };
        if !self.is_readable(project) {
            return false;
        }
        let (mut found, unlisted) = match tracked.project.scratch {
            // Searched through on discovery's threads, not this one alone: it can hold thousands
            // of folders.
            Some(_) => {
                scan::on_discovery_threads(|| artifact_dirs(&tracked.project, &self.boundary))
            }
            None => artifact_dirs(&tracked.project, &self.boundary),
        };
        let unseen = tracked.project.artifacts.iter().filter(|known| {
            let in_unlisted = unlisted
                .iter()
                .any(|(folder, _)| known.path.starts_with(folder));
            in_unlisted && !found.iter().any(|dir| dir.path == known.path)
        });
        let unseen: Vec<ArtifactDir> = unseen.cloned().collect();
        found.extend(unseen);
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found.retain(|dir| owner(&self.projects, &dir.path, |_| true) == Some(project));
        let Some(tracked) = self.projects.get_mut(project) else {
            return false;
        };
        let found = keep_measurements(found, &tracked.project.artifacts);
        let changed = found.len() != tracked.project.artifacts.len()
            || found
                .iter()
                .zip(&tracked.project.artifacts)
                .any(|(new, old)| new.path != old.path);
        tracked.project.artifacts = found;
        let unlisted_changed = tracked.unlisted != unlisted;
        tracked.unlisted = unlisted;
        if changed {
            self.reindex();
        }
        if unlisted_changed {
            self.report_unread();
        }
        changed || unlisted_changed
    }

    /// Leave the build folders inside a project to it alone. The search of a project around it
    /// stops at a nested repository but not at a plain project, so it finds them too, and a
    /// measurement lands on one project: a copy on another would wait for its size for good, and
    /// keep that project's folder size and the end of sizing waiting with it.
    pub(super) fn leave_build_folders_to_nested_projects(&mut self) {
        for nested in self.project_paths() {
            for enclosing in self.enclosing_projects(&nested) {
                if let Some(tracked) = self.projects.get_mut(&enclosing) {
                    let artifacts = &mut tracked.project.artifacts;
                    artifacts.retain(|dir| !dir.path.starts_with(&nested));
                }
            }
        }
    }
}

/// The build folders in `project`'s folder: a session's scratch folder itself, or anywhere in it,
/// where agents build under any name, and in any other folder as deep as `scan::find_artifacts`
/// looks. Returns them, and the folders the search could not list, and why. Only the search
/// through a scratch folder names those: a worktree's goes past them (see `scan::find_artifacts`),
/// as every rediscovery searches a worktree again.
pub(super) fn artifact_dirs(
    project: &Project,
    boundary: &Boundary,
) -> (Vec<ArtifactDir>, Vec<(PathBuf, io::ErrorKind)>) {
    let (found, unlisted) = match project.scratch {
        Some(_) => {
            let searched = scan::find_artifacts_throughout(&project.path);
            (searched.artifacts, searched.unlisted)
        }
        None => (scan::find_artifacts(&project.path, boundary), Vec::new()),
    };
    let dirs = found.into_iter().map(|(path, kind)| ArtifactDir {
        path,
        kind,
        measurement: Measurement::Pending,
    });
    (dirs.collect(), unlisted)
}

/// `found`, with the measurement of each directory `known` already has.
pub(super) fn keep_measurements(
    found: Vec<ArtifactDir>,
    known: &[ArtifactDir],
) -> Vec<ArtifactDir> {
    found
        .into_iter()
        .map(|mut artifact| {
            if let Some(old) = known.iter().find(|old| old.path == artifact.path) {
                artifact.measurement = old.measurement;
            }
            artifact
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{cargo_target, measured, node_modules};

    fn artifact(path: &str, bytes: Option<u64>) -> ArtifactDir {
        cargo_target(path, bytes.map_or(Measurement::Pending, measured))
    }

    #[test]
    fn keep_measurements_keeps_sizes_of_directories_still_found() {
        let known = [
            artifact("/code/app/target", Some(10)),
            artifact("/code/app/old/target", Some(20)),
        ];
        let found = vec![
            artifact("/code/app/target", None),
            node_modules("/code/app/web/node_modules", Measurement::Pending),
        ];
        let kept = keep_measurements(found, &known);
        let sizes: Vec<_> = kept
            .iter()
            .map(|a| {
                let bytes = a.measurement.usage().map(|usage| usage.bytes);
                (a.path.to_str().unwrap(), bytes)
            })
            .collect();
        assert_eq!(
            sizes,
            [
                ("/code/app/target", Some(10)),
                ("/code/app/web/node_modules", None)
            ]
        );
    }
}
