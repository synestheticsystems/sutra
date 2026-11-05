//! What a bare `cargo clean` deletes, worked out from the files cargo reads, as cargo 1.99.1
//! decides it. The line references are to cargo's source at tag 0.100.0
//! (<https://github.com/rust-lang/cargo/tree/0.100.0>), the commit that cargo was built from.
//!
//! The workspace a folder belongs to is `workspace`; the config files that can move its build
//! folders are `config`; the environment variables Sutra can see are `env`; how environment,
//! config and defaults combine into the folders the clean deletes is `folders`.
//!
//! What cargo checks and Sutra does not: cargo loads and validates every member manifest and
//! every profile, and runs `rustc -vV`, before it deletes. A workspace that fails any of those
//! makes cargo refuse where Sutra cleans; the folders are the same either way, as none of those
//! decides where they are. The other way round, a config file or manifest Sutra cannot look at
//! makes Sutra refuse where cargo may not (`exists`), as it could decide where they are.

mod config;
mod env;
mod folders;
mod workspace;

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use super::{CleanPlan, PlanError, PlannedFolder};

pub use env::CargoEnv;
pub use folders::Origin;

/// Why `cargo clean` itself would fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CargoRefusal {
    /// No `Cargo.toml` in `searched_from` or any folder above it.
    NoManifest { searched_from: PathBuf },
    /// A manifest or config file that is not TOML, or that holds a value of the wrong kind where
    /// cargo reads one. `problem` is the parser's account, or names the value.
    Invalid { file: PathBuf, problem: String },
    /// A config file's `include` that cargo rejects or cannot load.
    BadInclude {
        file: PathBuf,
        include: String,
        problem: IncludeProblem,
    },
    /// A build folder's setting is the empty string.
    Empty { origin: Origin },
    /// A `build.build-dir` template with a variable cargo does not know, or a stray brace.
    BadTemplate { origin: Origin, template: String },
    /// An environment variable, or a path a template puts in, that is not valid UTF-8.
    NotUnicode { origin: Origin },
}

/// What is wrong with a config `include` (`src/context/mod.rs:1386-1549`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncludeProblem {
    /// The file does not exist, and the include is not marked `optional`.
    Missing,
    /// The path does not end in `.toml`.
    NotToml,
    /// The path holds glob characters or braces, which cargo rejects.
    Pattern,
    /// The file was already loaded through this chain of includes.
    Cycle,
}

/// What a bare `cargo clean` deletes when run at the root of the workspace that `dir` belongs to
/// (`dir` is canonical).
pub(super) fn clean_at_workspace_of(dir: &Path, env: &CargoEnv) -> Result<CleanPlan, PlanError> {
    let workspace = workspace::find(dir, &env.cargo_home_at(dir))?;
    // The clean runs at the workspace root, so that is the folder cargo starts every search
    // from and resolves relative variables against.
    let cwd = workspace.root();
    let cargo_home = env.cargo_home_at(cwd);
    let settings = config::FolderSettings::load(cwd, &cargo_home)?;
    let resolved = folders::resolve(&workspace, &settings, env, cwd, &cargo_home)?;
    let mut planned: Vec<PlannedFolder> = Vec::new();
    for folder in resolved {
        let Some(path) = located(&folder.path)? else {
            continue;
        };
        if planned.iter().any(|other| path.starts_with(&other.path)) {
            continue;
        }
        planned.retain(|other| !other.path.starts_with(&path));
        planned.push(PlannedFolder {
            path,
            origin: folder.origin,
        });
    }
    Ok(CleanPlan {
        workspace: cwd.to_path_buf(),
        folders: planned,
    })
}

/// Where `path` is, canonical, but with its last component kept when that is a symlink, so that
/// the link itself is what the safety checks judge; `None` when nothing is there.
fn located(path: &Path) -> Result<Option<PathBuf>, PlanError> {
    let unreadable = |err: io::Error| PlanError::Unreadable {
        path: path.to_path_buf(),
        kind: err.kind(),
    };
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(unreadable(err)),
    };
    let canonical = match (meta.is_symlink(), path.parent(), path.file_name()) {
        (true, Some(parent), Some(name)) => fs::canonicalize(parent).map(|dir| dir.join(name)),
        _ => fs::canonicalize(path),
    };
    canonical.map(Some).map_err(unreadable)
}

/// Whether anything is at `path`, followed through links as cargo follows them. A search moves on
/// to its next candidate only when this says no: when nothing is there, or when part of the path
/// is a plain file (a file named `.cargo` holds no `.cargo/config`, and cargo passes over it too).
/// Any other failure could hide a file cargo reads, and passing over that file would settle on a
/// farther one or on the default folder and so delete something else, so it refuses the clean.
/// Cargo takes every failure here for "absent" (`src/context/mod.rs:1646-1691`,
/// `src/util/important_paths.rs:6-34`), but what fails for Sutra, above all inside the sandbox,
/// need not fail for the cargo the user runs. Inside the sandbox a file outside the grant can still
/// be seen to exist, and reading it is what fails.
pub(super) fn exists(path: &Path) -> Result<bool, PlanError> {
    match path.try_exists() {
        Ok(exists) => Ok(exists),
        Err(err) if err.kind() == io::ErrorKind::NotADirectory => Ok(false),
        Err(err) => Err(PlanError::Unreadable {
            path: path.to_path_buf(),
            kind: err.kind(),
        }),
    }
}

/// `path` with `.` dropped and each `..` taking off the component before it, without looking at
/// the disk, as cargo's `paths::normalize_path` does (`crates/cargo-util/src/paths.rs:86-118`).
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if normal.ends_with(Component::ParentDir) => {
                normal.push(Component::ParentDir);
            }
            Component::ParentDir => {
                if !normal.pop() && !normal.has_root() {
                    normal.push(Component::ParentDir);
                }
            }
            other => normal.push(other),
        }
    }
    normal
}

/// A manifest or config file read as TOML. A file that cannot be read is `Unreadable`: inside
/// the sandbox, any file outside the granted folder.
fn read_toml(file: &Path) -> Result<toml::Table, PlanError> {
    let text = fs::read_to_string(file).map_err(|err| PlanError::Unreadable {
        path: file.to_path_buf(),
        kind: err.kind(),
    })?;
    text.parse().map_err(|err: toml::de::Error| {
        let problem = match err.span() {
            Some(span) => {
                let before = text.get(..span.start).unwrap_or(&text);
                let line = before.matches('\n').count() + 1;
                format!("{} on line {line}", err.message())
            }
            None => err.message().to_string(),
        };
        PlanError::Cargo(CargoRefusal::Invalid {
            file: file.to_path_buf(),
            problem,
        })
    })
}

/// That `file` holds a value of the wrong kind at `key`.
fn wrong_kind(file: &Path, key: &str, expected: &str) -> PlanError {
    PlanError::Cargo(CargoRefusal::Invalid {
        file: file.to_path_buf(),
        problem: format!("`{key}` must be {expected}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn normalize_resolves_dots_lexically() {
        for (path, normal) in [
            ("/a/./b/../c", "/a/c"),
            ("/a/b/../../..", "/"),
            ("a/../../b", "../b"),
            ("../../a", "../../a"),
        ] {
            assert_eq!(normalize(Path::new(path)), Path::new(normal), "{path}");
        }
    }

    #[test]
    fn a_folder_inside_another_planned_folder_goes_with_it() {
        let fixture = TempDir::new("rules-nested-folders");
        fixture.write("app/Cargo.toml", "[package]\nname = \"app\"\n");
        fixture.write(
            "app/.cargo/config.toml",
            "[build]\nbuild-dir = \"target/intermediate\"\n",
        );
        fixture.dir("app/target/intermediate");
        let env = fixture.cargo_env();

        let clean = clean_at_workspace_of(&fixture.path("app"), &env).unwrap();

        assert_eq!(
            clean.folders,
            vec![PlannedFolder {
                path: fixture.path("app/target"),
                origin: Origin::Default,
            }]
        );
    }

    #[test]
    fn a_planned_folder_that_does_not_exist_is_left_out() {
        let fixture = TempDir::new("rules-missing-folder");
        fixture.write("app/Cargo.toml", "[package]\nname = \"app\"\n");
        let env = fixture.cargo_env();

        let clean = clean_at_workspace_of(&fixture.path("app"), &env).unwrap();

        assert_eq!(clean.workspace, fixture.path("app"));
        assert!(clean.folders.is_empty());
    }
}
