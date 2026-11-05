//! What shows under the list when there is something to show: how far the first sizing has come,
//! the warning that a stopped scanner needs a relaunch, and the sources whose failure loses only
//! some folders or some changes.

use std::io;
use std::path::Path;

use super::banner::problem_line;
use crate::dashboard::Footer;
use crate::map::{Snapshot, Source, SourceProblem};

/// The footer under `map`, `None` while there is nothing to say. `engine_stopped` when the
/// engine's channel closed, so the map no longer updates.
pub(in crate::dashboard) fn footer(
    map: &Snapshot,
    engine_stopped: bool,
    home: &Path,
) -> Option<Footer> {
    let problems: Vec<String> = map
        .problems
        .iter()
        .filter(|problem| worth_showing(problem))
        .map(|problem| problem_line(problem, home))
        .collect();
    let stopped = engine_stopped.then(|| "Scanner stopped. Quit and relaunch Sutra.".to_string());
    let sizing = map.sizing();
    // Unfinished, some folder is still unsized, so the total is above nought.
    let sizing = (!sizing.is_finished()).then(|| sizing.sized as f32 / sizing.total as f32);
    let needs_saying = sizing.is_some() || !problems.is_empty() || stopped.is_some();
    needs_saying.then_some(Footer {
        sizing,
        problems,
        stopped,
    })
}

/// Whether a problem belongs under the list. A folder whose `.git` file names an admin folder that
/// is gone is a worktree left behind by its repository: nothing failed to read, and there is
/// nothing to do about it here.
fn worth_showing(problem: &SourceProblem) -> bool {
    !problem.source.is_sole_source()
        && !(problem.source == Source::Scan && problem.kind == io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::Measurement;
    use crate::map::test_support::{cargo_target, measured, project_at, snapshot_of};

    #[test]
    fn a_worktree_left_behind_by_its_repository_is_not_worth_a_warning() {
        let problem = |source, kind| SourceProblem::new(source, PathBuf::from("/code/app"), kind);
        assert!(!worth_showing(&problem(
            Source::Scan,
            io::ErrorKind::NotFound
        )));
        assert!(worth_showing(&problem(
            Source::Scan,
            io::ErrorKind::PermissionDenied
        )));
        assert!(worth_showing(&problem(
            Source::Watcher,
            io::ErrorKind::NotFound
        )));
    }

    #[test]
    fn the_footer_shows_while_sizing_runs_or_the_scanner_stopped_and_not_otherwise() {
        let home = Path::new("/Users/dev");
        let sizing = snapshot_of(vec![crate::map::Project {
            artifacts: vec![cargo_target("/code/app/target", Measurement::Measuring)],
            own_files: measured(10),
            ..project_at("/code/app")
        }]);
        let shown = footer(&sizing, false, home).unwrap();
        assert_eq!(shown.sizing, Some(0.5));
        assert_eq!(shown.stopped, None);

        let sized = snapshot_of(Vec::new());
        assert_eq!(footer(&sized, false, home), None);
        let stopped = footer(&sized, true, home).unwrap();
        assert_eq!(
            stopped.stopped.as_deref(),
            Some("Scanner stopped. Quit and relaunch Sutra.")
        );
        assert_eq!(stopped.sizing, None);
    }
}
