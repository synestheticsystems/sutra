//! A program running from inside a folder a clean deletes, as a long simulation, a benchmark or a
//! test binary started from a terminal does: the clean would take its files from under it, and
//! nothing else tells of it, as it holds no build lock once its build ended, and it may write
//! nothing and listen on no port for hours. So a clean is refused while one runs, as it is while a
//! build is, both where it is worked out and when its turn comes (see `run::lock`). macOS is asked
//! at those two moments alone; between them a program may start, and the next clean worked out
//! tells of it. Only the direct edition can tell: the App Sandbox shows an app no other process,
//! so the store edition never asks.

use std::fs;
use std::path::PathBuf;

use super::open_folder::OpenFolder;
use super::run::{CleanError, io_error};
use crate::map::processes;

/// Refuse while one of the user's processes runs a file inside one of `folders`, each held open as
/// the folder its checks passed. Each is compared by its resolved path, as macOS gives a program's,
/// so a folder reached through a link still counts.
pub(super) fn refuse_while_one_runs(folders: &[(PathBuf, OpenFolder)]) -> Result<(), CleanError> {
    let resolved = (folders.iter())
        .map(|(path, _)| fs::canonicalize(path).map_err(|err| io_error(path, &err)))
        .collect::<Result<Vec<PathBuf>, CleanError>>()?;
    match processes::running_inside(&resolved) {
        Ok(None) => Ok(()),
        Ok(Some(program)) => Err(CleanError::ProgramRunning {
            executable: program.executable,
        }),
        Err(err) => Err(CleanError::ProgramsUnlisted { kind: err.kind() }),
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) mod tests {
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::process::{Child, Command};

    use super::*;
    use crate::map::test_support::TempDir;

    /// A small program running from `executable`, a copy of `sleep`, until dropped.
    pub(crate) struct Running(Child);

    impl Running {
        pub(crate) fn from(executable: &Path) -> Running {
            fs::create_dir_all(executable.parent().unwrap()).unwrap();
            fs::copy("/bin/sleep", executable).unwrap();
            Running(Command::new(executable).arg("60").spawn().unwrap())
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn held(path: &Path) -> (PathBuf, OpenFolder) {
        (path.to_path_buf(), OpenFolder::at(path).unwrap())
    }

    #[test]
    fn a_program_running_from_a_folder_reached_through_a_link_is_found_and_one_elsewhere_is_not() {
        let fixture = TempDir::new("clean-programs-running");
        let target = fixture.dir("app/target");
        let other = fixture.dir("site/target");
        symlink(fixture.path("app"), fixture.path("linked")).unwrap();
        let through_link = fixture.path("linked/target");
        let executable = target.join("release/sim");
        let running = Running::from(&executable);

        assert_eq!(
            refuse_while_one_runs(&[held(&other), held(&through_link)]),
            Err(CleanError::ProgramRunning { executable })
        );
        assert_eq!(refuse_while_one_runs(&[held(&other)]), Ok(()));
        drop(running);
        assert_eq!(
            refuse_while_one_runs(&[held(&through_link)]),
            Ok(()),
            "it quit"
        );
    }
}
