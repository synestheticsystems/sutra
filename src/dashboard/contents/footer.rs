//! What shows under the list when there is something to show: how far the first sizing has come,
//! the sources whose failure loses only some folders or some changes, and where to let Sutra into
//! the private folders macOS keeps it out of.

use std::io;
use std::path::Path;

use super::banner::problem_line;
use crate::dashboard::{Footer, format};
use crate::map::{PrivateFolder, Snapshot, Source, SourceProblem};

/// The footer under `map`, `None` while there is nothing to say.
pub(in crate::dashboard) fn footer(map: &Snapshot, home: &Path) -> Option<Footer> {
    let problems: Vec<String> = map
        .problems
        .iter()
        .filter(|problem| worth_showing(problem))
        .map(|problem| problem_line(problem, home))
        .collect();
    let refused = refused(&map.refused_private_folders);
    let sizing = map.sizing();
    // Unfinished, some folder is still unsized, so the total is above nought.
    let sizing = (!sizing.is_finished()).then(|| sizing.sized as f32 / sizing.total as f32);
    let needs_saying = sizing.is_some() || !problems.is_empty() || refused.is_some();
    needs_saying.then_some(Footer {
        sizing,
        problems,
        refused,
    })
}

/// Where to let Sutra into the private `folders` macOS keeps it out of: `macOS said no to
/// Documents. Change its mind in System Settings › Privacy & Security › Files and Folders.` `None`
/// while macOS keeps Sutra out of none.
fn refused(folders: &[PrivateFolder]) -> Option<String> {
    let names: Vec<&str> = folders.iter().map(|folder| folder.name()).collect();
    (!names.is_empty()).then(|| {
        format!(
            "macOS said no to {}. Change its mind in System Settings \u{203a} Privacy & Security \
             \u{203a} Files and Folders.",
            format::all(&names)
        )
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
    fn the_footer_shows_while_sizing_runs_and_not_once_everything_is_sized() {
        let home = Path::new("/Users/dev");
        let sizing = snapshot_of(vec![crate::map::Project {
            artifacts: vec![cargo_target("/code/app/target", Measurement::Measuring)],
            own_files: measured(10),
            ..project_at("/code/app")
        }]);
        let shown = footer(&sizing, home).unwrap();
        assert_eq!(shown.sizing, Some(0.5));

        let sized = snapshot_of(Vec::new());
        assert_eq!(footer(&sized, home), None);
    }

    #[test]
    fn the_footer_says_where_to_let_sutra_into_the_private_folders_macos_keeps_it_out_of() {
        let home = Path::new("/Users/dev");
        let refusing = |folders: &[PrivateFolder]| Snapshot {
            refused_private_folders: folders.to_vec(),
            ..snapshot_of(Vec::new())
        };
        let documents = footer(&refusing(&[PrivateFolder::Documents]), home).unwrap();
        assert_eq!(
            documents.refused.as_deref(),
            Some(
                "macOS said no to Documents. Change its mind in System Settings \u{203a} \
                 Privacy & Security \u{203a} Files and Folders."
            )
        );
        let two = [PrivateFolder::Desktop, PrivateFolder::Documents];
        let both = footer(&refusing(&two), home).unwrap().refused.unwrap();
        assert!(both.starts_with("macOS said no to Desktop and Documents. "));
        assert_eq!(footer(&refusing(&[]), home), None);
    }
}
