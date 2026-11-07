//! Where Sutra refuses a folder that `cargo clean` would delete, and why.
//!
//! Cargo deletes whatever folder its settings name. It checks that the folder looks like its own
//! only when `--target-dir` names it (`src/ops/cargo_clean.rs:73-89`, since cargo 1.97). Sutra
//! checks every folder before deleting it:
//!   - A folder that config or the environment named must hold the `CACHEDIR.TAG` cargo writes
//!     into a build folder it creates, so a setting that points at the wrong folder deletes
//!     nothing. Cargo's own check reads only the tag's signature (`src/ops/cargo_clean.rs:156-185`,
//!     `crates/cargo-util/src/paths.rs:802-815`), which every tool's tag starts with; Sutra asks
//!     also for the words that say cargo wrote it (`scan::holds_cargo_tag`). Cargo's default
//!     `target` beside the root manifest needs no tag.
//!   - No folder may hold the workspace, the project, the home folder or `CARGO_HOME`. A cache
//!     standing alone, a build folder that is a project of its own (`ItsProject::Itself`), has no
//!     project around it to keep: its clean empties it as any clean empties a build folder.
//!   - A symlink is refused: cargo would remove the link alone, which frees nothing.
//!   - In the store edition, a folder outside the granted folders is refused before anything is
//!     tried, rather than failing halfway.
//!
//! A folder deleted alone, which no `cargo clean` of its workspace deletes, must be a folder cargo
//! made: a `target` beside a `Cargo.toml`, or holding cargo's `CACHEDIR.TAG`, as the scan requires
//! of a build folder (`scan::is_cargo_target`).

use std::fs;
use std::path::{Path, PathBuf};

use super::{CargoEnv, Origin, PlanError, PlannedFolder};
use crate::map::scan;

/// Where Sutra may delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach<'a> {
    /// Anywhere the user may: the direct edition.
    Anywhere,
    /// Only inside these folders: the store edition, inside the folders the user granted.
    Within(&'a [PathBuf]),
}

/// The project a build folder to clean is in, which says what its clean must keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItsProject<'a> {
    /// The project at this folder, which holds source of its own, so no clean may take the folder
    /// with it.
    At(&'a Path),
    /// The build folder itself: a Cargo cache standing alone, as a scratch folder that
    /// `CARGO_TARGET_DIR` named is (see `Project::is_its_own_build_folder`). There is no source in
    /// it to keep, and its clean empties it as any clean empties a build folder, its tag and lock
    /// files kept.
    Itself,
}

/// Why Sutra will not delete a folder that `cargo clean` would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Config or the environment named the folder, and cargo never marked it as its own.
    Untagged { origin: Origin },
    /// Deleting the folder would delete `held`, at `path`, with it.
    Holds { held: Held, path: PathBuf },
    /// The folder is a symlink.
    Link,
    /// A file is where the folder should be.
    NotAFolder,
    /// The folder lies outside the folders the store edition was granted.
    OutsideGrant,
    /// A folder to delete alone that cargo did not make.
    NotCargoMade,
}

/// What a clean must never take with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    Workspace,
    Project,
    Home,
    CargoHome,
}

/// The folders no clean may take with it, canonical.
pub(super) struct Protected {
    folders: Vec<(Held, PathBuf)>,
}

impl Protected {
    /// The workspace being cleaned (`None` for a folder no workspace holds), the project the
    /// clicked folder is in when it holds source, the home folder and `CARGO_HOME` as cargo would
    /// have it there. Cargo runs at the workspace's root, else in the project, else beside the
    /// clicked folder, at `beside`, as it runs for a cache standing alone.
    pub(super) fn new(
        workspace: Option<&Path>,
        project: ItsProject,
        beside: &Path,
        env: &CargoEnv,
    ) -> Protected {
        let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let project = match project {
            ItsProject::At(folder) => Some(folder),
            ItsProject::Itself => None,
        };
        let cwd = workspace.or(project).unwrap_or(beside);
        let folders = workspace
            .map(|workspace| (Held::Workspace, canonical(workspace)))
            .into_iter()
            .chain(project.map(|project| (Held::Project, canonical(project))))
            .chain([
                (Held::Home, canonical(&env.home)),
                (Held::CargoHome, canonical(&env.cargo_home_at(cwd))),
            ])
            .collect();
        Protected { folders }
    }
}

/// Check a folder a `cargo clean` plan deletes.
pub(super) fn check_planned(
    folder: &PlannedFolder,
    protected: &Protected,
    reach: &Reach,
) -> Result<(), PlanError> {
    check_any(&folder.path, protected, reach)?;
    if folder.origin != Origin::Default && !scan::holds_cargo_tag(&folder.path) {
        let origin = folder.origin.clone();
        return Err(refused(&folder.path, Refusal::Untagged { origin }));
    }
    Ok(())
}

/// Check a folder to delete alone.
pub(super) fn check_delete_folder(
    folder: &Path,
    protected: &Protected,
    reach: &Reach,
) -> Result<(), PlanError> {
    check_any(folder, protected, reach)?;
    if !scan::is_cargo_target(folder) {
        return Err(refused(folder, Refusal::NotCargoMade));
    }
    Ok(())
}

/// The checks every folder Sutra deletes must pass. `folder` is canonical but for a last
/// component that is a symlink.
fn check_any(folder: &Path, protected: &Protected, reach: &Reach) -> Result<(), PlanError> {
    let meta = fs::symlink_metadata(folder).map_err(|err| PlanError::Unreadable {
        path: folder.to_path_buf(),
        kind: err.kind(),
    })?;
    if meta.is_symlink() {
        return Err(refused(folder, Refusal::Link));
    }
    if !meta.is_dir() {
        return Err(refused(folder, Refusal::NotAFolder));
    }
    if let Some((held, path)) = protected
        .folders
        .iter()
        .find(|(_, path)| path.starts_with(folder))
    {
        let reason = Refusal::Holds {
            held: *held,
            path: path.clone(),
        };
        return Err(refused(folder, reason));
    }
    if let Reach::Within(granted) = reach {
        let inside = granted.iter().any(|root| {
            folder.starts_with(fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        });
        if !inside {
            return Err(refused(folder, Refusal::OutsideGrant));
        }
    }
    Ok(())
}

fn refused(folder: &Path, reason: Refusal) -> PlanError {
    PlanError::Refused {
        folder: folder.to_path_buf(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::{CACHEDIR_TAG, PYTEST_TAG, TempDir};

    fn from_config(fixture: &TempDir, path: PathBuf) -> PlannedFolder {
        PlannedFolder {
            path,
            origin: Origin::Config {
                key: "build.target-dir",
                file: fixture.path("app/.cargo/config.toml"),
            },
        }
    }

    /// What the clean of a build folder in the fixture's project at `project` keeps, `workspace`
    /// the workspace it cleans.
    fn protected_in(fixture: &TempDir, workspace: Option<&str>, project: &str) -> Protected {
        let workspace = workspace.map(|workspace| fixture.path(workspace));
        Protected::new(
            workspace.as_deref(),
            ItsProject::At(&fixture.path(project)),
            fixture.root(),
            &fixture.cargo_env(),
        )
    }

    fn reason(result: Result<(), PlanError>) -> Refusal {
        match result {
            Err(PlanError::Refused { reason, .. }) => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_configured_folder_needs_cargos_tag_and_the_default_does_not() {
        let fixture = TempDir::new("safety-tag");
        fixture.dir("home/.cargo");
        let protected = protected_in(&fixture, Some("app"), "app");
        let tagged = fixture
            .write("out/CACHEDIR.TAG", CACHEDIR_TAG)
            .parent()
            .unwrap()
            .to_path_buf();
        let untagged = fixture.dir("bare");
        let lookalike = fixture.write("fake/CACHEDIR.TAG", "Signature: something else");
        let lookalike = lookalike.parent().unwrap().to_path_buf();
        let pytests = fixture.write("pytest/CACHEDIR.TAG", PYTEST_TAG);
        let pytests = pytests.parent().unwrap().to_path_buf();

        assert_eq!(
            check_planned(&from_config(&fixture, tagged), &protected, &Reach::Anywhere),
            Ok(())
        );
        for folder in [untagged.clone(), lookalike, pytests] {
            assert!(matches!(
                reason(check_planned(
                    &from_config(&fixture, folder),
                    &protected,
                    &Reach::Anywhere
                )),
                Refusal::Untagged { .. }
            ));
        }
        let default = PlannedFolder {
            path: untagged,
            origin: Origin::Default,
        };
        assert_eq!(
            check_planned(&default, &protected, &Reach::Anywhere),
            Ok(())
        );
    }

    #[test]
    fn a_folder_holding_what_a_clean_must_keep_is_refused() {
        let fixture = TempDir::new("safety-holds");
        fixture.dir("home/.cargo");
        fixture.dir("ws");
        fixture.dir("wt");
        let protected = protected_in(&fixture, Some("ws"), "wt");
        for (folder, held) in [
            ("ws", Held::Workspace),
            ("wt", Held::Project),
            ("home", Held::Home),
            ("home/.cargo", Held::CargoHome),
            ("", Held::Workspace),
        ] {
            let folder = PlannedFolder {
                path: fixture.path(folder),
                origin: Origin::Default,
            };
            assert!(
                matches!(
                    reason(check_planned(&folder, &protected, &Reach::Anywhere)),
                    Refusal::Holds { held: refused, .. } if refused == held
                ),
                "{folder:?}"
            );
        }
    }

    #[test]
    fn links_files_and_folders_outside_the_grant_are_refused() {
        let fixture = TempDir::new("safety-kinds");
        fixture.dir("home/.cargo");
        let protected = protected_in(&fixture, Some("app"), "app");
        let default = |path: PathBuf| PlannedFolder {
            path,
            origin: Origin::Default,
        };
        let real = fixture.dir("granted/target");
        let link = fixture.path("granted/link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let file = fixture.write("granted/file", "");
        let outside = fixture.dir("elsewhere/target");
        let granted = [fixture.path("granted")];
        let reach = Reach::Within(&granted);

        assert_eq!(
            reason(check_planned(&default(link), &protected, &reach)),
            Refusal::Link
        );
        assert_eq!(
            reason(check_planned(&default(file), &protected, &reach)),
            Refusal::NotAFolder
        );
        assert_eq!(
            reason(check_planned(&default(outside.clone()), &protected, &reach)),
            Refusal::OutsideGrant
        );
        assert_eq!(check_planned(&default(real), &protected, &reach), Ok(()));
        assert_eq!(
            check_planned(&default(outside), &protected, &Reach::Anywhere),
            Ok(())
        );
    }

    #[test]
    fn a_folder_deleted_alone_must_be_one_cargo_made() {
        let fixture = TempDir::new("safety-delete-folder");
        fixture.dir("home/.cargo");
        let protected = protected_in(&fixture, None, "app");
        fixture.write("app/crates/a/Cargo.toml", "");
        let beside_manifest = fixture.dir("app/crates/a/target");
        let tagged = fixture.write("app/tools/target/CACHEDIR.TAG", CACHEDIR_TAG);
        let tagged = tagged.parent().unwrap().to_path_buf();
        let site = fixture.dir("app/site/target");

        assert_eq!(
            check_delete_folder(&beside_manifest, &protected, &Reach::Anywhere),
            Ok(())
        );
        assert_eq!(
            check_delete_folder(&tagged, &protected, &Reach::Anywhere),
            Ok(())
        );
        assert_eq!(
            reason(check_delete_folder(&site, &protected, &Reach::Anywhere)),
            Refusal::NotCargoMade
        );
    }

    #[test]
    fn a_cache_standing_alone_is_emptied_whole_where_a_project_at_its_folder_is_kept() {
        let fixture = TempDir::new("safety-standing-alone");
        fixture.dir("home/.cargo");
        let cache = fixture.write("scratchpad/target-567-fix/CACHEDIR.TAG", CACHEDIR_TAG);
        let cache = cache.parent().unwrap().to_path_buf();
        let tagged_home = fixture.write("home/CACHEDIR.TAG", CACHEDIR_TAG);
        let tagged_home = tagged_home.parent().unwrap().to_path_buf();
        let beside = fixture.path("scratchpad");
        let env = fixture.cargo_env();
        let alone = Protected::new(None, ItsProject::Itself, &beside, &env);
        let in_project = Protected::new(None, ItsProject::At(&cache), &beside, &env);

        assert_eq!(
            check_delete_folder(&cache, &alone, &Reach::Anywhere),
            Ok(())
        );
        assert_eq!(
            reason(check_delete_folder(&cache, &in_project, &Reach::Anywhere)),
            Refusal::Holds {
                held: Held::Project,
                path: cache.clone()
            },
            "a project holding source keeps its folder"
        );
        assert_eq!(
            reason(check_delete_folder(&tagged_home, &alone, &Reach::Anywhere)),
            Refusal::Holds {
                held: Held::Home,
                path: tagged_home.clone()
            },
            "and the home folder is kept whatever the project"
        );
    }
}
