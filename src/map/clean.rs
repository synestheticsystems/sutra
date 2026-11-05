//! Cleaning a Cargo build folder from its card: deleting what a bare `cargo clean` deletes, as
//! cargo decides it, but without running cargo and never while a build uses the folder. Of each
//! folder the lock files and the tag stay, for the build that may be waiting on them (`run`).
//!
//! Sutra cannot run cargo for this. The Mac App Store edition may run only code from its own
//! bundle, and `cargo clean` needs the user's toolchain anyway: it runs `rustc -vV` before it
//! deletes anything. So Sutra reads the files cargo reads, works out what cargo would delete
//! (`cargo_rules`, checked against cargo 1.99.1 and its source at tag 0.100.0, whose lines it
//! cites), and deletes that under cargo's own build locks (`run`).
//!
//! A clean is worked out for the folder the user clicked (`plan`):
//!   - The folder's workspace is the one cargo finds from the folder beside it, and the clean is
//!     the one `cargo clean` does at that workspace's root: the target-dir, and the build-dir too
//!     when `build.build-dir` puts it elsewhere.
//!   - Each folder keeps where its location came from (cargo's default, a config file, or one of
//!     Sutra's own environment variables), so a question before the delete can say. A variable
//!     set only in a shell never reaches an app opened from the Finder or the Dock, so a shell's
//!     `CARGO_TARGET_DIR` plays no part here.
//!   - A clicked folder that this clean would not delete gets a plain delete of that folder alone
//!     (`Plan::DeleteFolder`), as no bare `cargo clean` of its workspace touches it. It must be a
//!     folder cargo made, it goes under the same locks, and a question says what it is: left over
//!     from an earlier build, or where another workspace builds.
//!
//! Where Sutra refuses a folder cargo would delete (`safety`), each refusal naming the folder:
//!   - a folder that config or the environment named, when cargo never marked it as its own
//!     with a `CACHEDIR.TAG` (cargo checks the tag only for `--target-dir`, so a typo such as
//!     `target-dir = "."` would make it delete source code);
//!   - a folder that holds the workspace, the project, the home folder or `CARGO_HOME`;
//!   - a folder that is a symlink, where cargo would remove the link alone and free nothing;
//!   - in the store edition, a folder outside the folders the user granted.
//!
//! A slip in a setting should cost a refusal, not a source tree, whatever cargo would do with it.
//! And removing a link frees no space and undoes a redirect someone set up on purpose.

mod cargo_rules;
mod idle;
mod locks;
mod prepare;
mod removal;
mod run;
mod safety;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use cargo_rules::{CargoEnv, CargoRefusal, IncludeProblem, Origin};
pub use idle::{
    IdleClean, IdleCleans, IdleFolder, LeftOut, WORTH_CLEANING_BYTES, Why,
    can_tell_what_is_at_work, idle_folders, is_blocked_by_a_sharer, is_still_idle, prepare_idle,
};
pub use prepare::{PrepareError, Prepared, Sharing, prepare};
pub use run::{CleanError, Cleaned, Freed, LockedPlan, Progress, lock};
pub use safety::{Held, Reach, Refusal};

/// What deleting the clicked folder means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// The clicked folder is one a bare `cargo clean` deletes, and this is all that it deletes.
    CargoClean(CleanPlan),
    /// A plain delete of the clicked folder alone. A bare `cargo clean` of its workspace deletes
    /// `cargo_clean` and not this folder, or no workspace holds the folder at all (`None`). It is
    /// then left over from an earlier build or setup, or it is where another workspace builds,
    /// which `Sharing` tells.
    DeleteFolder {
        /// Canonical.
        folder: PathBuf,
        cargo_clean: Option<CleanPlan>,
    },
}

impl Plan {
    /// The folders that deleting this plan deletes, and nothing else.
    pub fn folders(&self) -> Vec<&Path> {
        match self {
            Plan::CargoClean(clean) => clean.folders.iter().map(|f| f.path.as_path()).collect(),
            Plan::DeleteFolder { folder, .. } => vec![folder.as_path()],
        }
    }
}

/// What a bare `cargo clean` at a workspace's root deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanPlan {
    /// The workspace's root folder, where the clean runs.
    pub workspace: PathBuf,
    /// The target-dir, then the build-dir when that is another folder. Only folders that exist
    /// are listed, as a missing one has nothing to delete, and one inside another is left to
    /// the deletion of the other.
    pub folders: Vec<PlannedFolder>,
}

/// A folder a clean deletes, and where its location came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFolder {
    /// Canonical, its last component kept when that is a symlink.
    pub path: PathBuf,
    pub origin: Origin,
}

/// Why the clicked folder is not cleaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The clicked folder is gone, or no longer a folder.
    Gone { folder: PathBuf },
    /// `cargo clean` would fail here too.
    Cargo(CargoRefusal),
    /// A file `cargo clean` would read that Sutra cannot, or a path where one may be that Sutra
    /// cannot look at: outside the folder the store edition was granted, or unreadable for another
    /// reason.
    Unreadable { path: PathBuf, kind: io::ErrorKind },
    /// Sutra will not delete `folder`, though cargo would.
    Refused { folder: PathBuf, reason: Refusal },
}

/// Work out what cleaning `clicked`, a build folder in the project at `project`, deletes. The
/// environment cargo would see is `env`; `reach` is where Sutra may delete.
pub fn plan(
    clicked: &Path,
    project: &Path,
    env: &CargoEnv,
    reach: &Reach,
) -> Result<Plan, PlanError> {
    let gone = || PlanError::Gone {
        folder: clicked.to_path_buf(),
    };
    // Any other failure says nothing of whether the folder is there.
    let cannot_look = |err: io::Error| match err.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => gone(),
        kind => PlanError::Unreadable {
            path: clicked.to_path_buf(),
            kind,
        },
    };
    if !fs::symlink_metadata(clicked).map_err(cannot_look)?.is_dir() {
        return Err(gone());
    }
    let clicked = fs::canonicalize(clicked).map_err(cannot_look)?;
    let beside = clicked.parent().ok_or_else(gone)?;
    let cargo_clean = match cargo_rules::clean_at_workspace_of(beside, env) {
        Ok(clean) => Some(clean),
        Err(PlanError::Cargo(CargoRefusal::NoManifest { .. })) => None,
        Err(err) => return Err(err),
    };
    let protected = |workspace: Option<&Path>| safety::Protected::new(workspace, project, env);
    match cargo_clean {
        Some(clean) if clean.folders.iter().any(|folder| folder.path == clicked) => {
            let protected = protected(Some(&clean.workspace));
            for folder in &clean.folders {
                safety::check_planned(folder, &protected, reach)?;
            }
            Ok(Plan::CargoClean(clean))
        }
        cargo_clean => {
            let workspace = cargo_clean.as_ref().map(|clean| clean.workspace.as_path());
            safety::check_delete_folder(&clicked, &protected(workspace), reach)?;
            Ok(Plan::DeleteFolder {
                folder: clicked,
                cargo_clean,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{CACHEDIR_TAG, TempDir};

    #[test]
    fn a_workspace_root_target_is_its_cargo_clean() {
        let fixture = TempDir::new("clean-plan-default");
        fixture.package("app");
        let target = fixture.dir("app/target");

        let plan = plan(
            &target,
            &fixture.path("app"),
            &fixture.cargo_env(),
            &Reach::Anywhere,
        )
        .unwrap();

        assert_eq!(
            plan,
            Plan::CargoClean(CleanPlan {
                workspace: fixture.path("app"),
                folders: vec![PlannedFolder {
                    path: target,
                    origin: Origin::Default,
                }],
            })
        );
    }

    #[test]
    fn a_member_target_is_a_leftover_of_the_workspace_that_builds_elsewhere() {
        let fixture = TempDir::new("clean-plan-member");
        fixture.write("ws/Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        fixture.package("ws/crates/a");
        let stale = fixture.dir("ws/crates/a/target");
        let real = fixture.dir("ws/target");

        let plan = plan(
            &stale,
            &fixture.path("ws"),
            &fixture.cargo_env(),
            &Reach::Anywhere,
        )
        .unwrap();

        let Plan::DeleteFolder {
            folder,
            cargo_clean: Some(clean),
        } = plan
        else {
            panic!("expected a leftover, got {plan:?}");
        };
        assert_eq!(folder, stale);
        assert_eq!(clean.workspace, fixture.path("ws"));
        assert_eq!(clean.folders[0].path, real);
    }

    #[test]
    fn a_tagged_folder_with_no_manifest_around_is_a_leftover_of_no_workspace() {
        let fixture = TempDir::new("clean-plan-orphan");
        let orphan = fixture.write("loose/target/CACHEDIR.TAG", CACHEDIR_TAG);
        let orphan = orphan.parent().unwrap().to_path_buf();

        let plan = plan(
            &orphan,
            &fixture.path("loose"),
            &fixture.cargo_env(),
            &Reach::Anywhere,
        )
        .unwrap();

        assert_eq!(
            plan,
            Plan::DeleteFolder {
                folder: orphan,
                cargo_clean: None
            }
        );
    }

    #[test]
    fn a_folder_that_is_gone_is_said_to_be() {
        let fixture = TempDir::new("clean-plan-gone");
        fixture.package("app");
        let gone = fixture.path("app/target");

        assert_eq!(
            plan(
                &gone,
                &fixture.path("app"),
                &fixture.cargo_env(),
                &Reach::Anywhere
            ),
            Err(PlanError::Gone { folder: gone })
        );
    }

    #[test]
    fn a_refused_planned_folder_refuses_the_whole_clean() {
        let fixture = TempDir::new("clean-plan-refused");
        fixture.package("app");
        fixture.write(
            "app/.cargo/config.toml",
            "[build]\nbuild-dir = \"{workspace-root}/untagged\"\n",
        );
        let target = fixture.dir("app/target");
        let untagged = fixture.dir("app/untagged");

        let refused = plan(
            &target,
            &fixture.path("app"),
            &fixture.cargo_env(),
            &Reach::Anywhere,
        );

        assert_eq!(
            refused,
            Err(PlanError::Refused {
                folder: untagged,
                reason: Refusal::Untagged {
                    origin: Origin::Config {
                        key: "build.build-dir",
                        file: fixture.path("app/.cargo/config.toml"),
                    }
                },
            })
        );
    }

    #[test]
    fn a_folder_that_cannot_be_looked_at_is_unreadable_and_one_that_is_not_there_is_gone() {
        let fixture = TempDir::new("clean-plan-sealed-parent");
        fixture.package("app");
        let target = fixture.dir("app/target");
        let env = fixture.cargo_env();
        let gone = fixture.path("app/nothing");
        assert_eq!(
            plan(&gone, &fixture.path("app"), &env, &Reach::Anywhere),
            Err(PlanError::Gone { folder: gone })
        );

        fixture.restrict(&fixture.path("app"), 0o000);

        assert_eq!(
            plan(&target, &fixture.path("app"), &env, &Reach::Anywhere),
            Err(PlanError::Unreadable {
                path: target,
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn a_config_that_cannot_be_inspected_refuses_the_clean() {
        let fixture = TempDir::new("clean-plan-sealed-config");
        fixture.package("app");
        let target = fixture.dir("app/target");
        let sealed = fixture.dir("app/.cargo");
        fixture.restrict(&sealed, 0o000);

        let refused = plan(
            &target,
            &fixture.path("app"),
            &fixture.cargo_env(),
            &Reach::Anywhere,
        );

        assert_eq!(
            refused,
            Err(PlanError::Unreadable {
                path: sealed.join("config"),
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }
}
