//! The workspace a folder belongs to, found as cargo finds it.

use std::path::{Path, PathBuf};

use super::{CargoRefusal, exists, normalize, read_toml, wrong_kind};
use crate::map::clean::PlanError;

/// A workspace, known by its root manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Workspace {
    pub manifest: PathBuf,
}

impl Workspace {
    /// The folder the root manifest sits in.
    pub(super) fn root(&self) -> &Path {
        self.manifest
            .parent()
            .expect("a manifest path ends in Cargo.toml")
    }
}

/// The workspace `dir` belongs to: the nearest `Cargo.toml` in `dir` or above it
/// (`src/util/important_paths.rs:6-34`), and the root that manifest names or lies under
/// (`src/workspace/workspace.rs:836-854`). Above `cargo_home` no root is looked for.
pub(super) fn find(dir: &Path, cargo_home: &Path) -> Result<Workspace, PlanError> {
    let manifest = nearest_manifest(dir)?.ok_or_else(|| {
        PlanError::Cargo(CargoRefusal::NoManifest {
            searched_from: dir.to_path_buf(),
        })
    })?;
    let manifest = match Role::of(&manifest)?.root(&manifest, &manifest) {
        Some(root) => root,
        None => search_above(&manifest, cargo_home)?.unwrap_or(manifest),
    };
    Ok(Workspace { manifest })
}

/// The nearest `Cargo.toml` in `dir` or above it.
fn nearest_manifest(dir: &Path) -> Result<Option<PathBuf>, PlanError> {
    for folder in dir.ancestors() {
        let manifest = folder.join("Cargo.toml");
        if exists(&manifest)? {
            return Ok(Some(manifest));
        }
    }
    Ok(None)
}

/// What a manifest says about the workspace it is in.
enum Role {
    /// It has a `[workspace]` table: a root, with the paths it claims and leaves out.
    Root {
        members: Vec<String>,
        exclude: Vec<String>,
    },
    /// `package.workspace` names the root's folder, relative to the manifest's.
    Pointer(String),
    /// Neither: its root, if any, lies above it.
    Member,
}

impl Role {
    fn of(manifest: &Path) -> Result<Role, PlanError> {
        let table = read_toml(manifest)?;
        if let Some(workspace) = table.get("workspace") {
            let workspace = workspace
                .as_table()
                .ok_or_else(|| wrong_kind(manifest, "workspace", "a table"))?;
            return Ok(Role::Root {
                members: strings(workspace, "members", manifest)?,
                exclude: strings(workspace, "exclude", manifest)?,
            });
        }
        // `[project]` is the old name of `[package]`, still read before the 2024 edition.
        let package = table.get("package").or_else(|| table.get("project"));
        match package.and_then(|package| package.get("workspace")) {
            Some(pointer) => pointer
                .as_str()
                .map(|pointer| Role::Pointer(pointer.to_string()))
                .ok_or_else(|| wrong_kind(manifest, "package.workspace", "a string")),
            None => Ok(Role::Member),
        }
    }

    /// The root manifest this role names for the package at `look_from`, when the manifest at
    /// `manifest` has this role (`src/workspace/workspace.rs:181-200`).
    fn root(&self, manifest: &Path, look_from: &Path) -> Option<PathBuf> {
        let dir = manifest.parent()?;
        match self {
            Role::Root { members, exclude } => {
                (!excludes(dir, members, exclude, look_from)).then(|| manifest.to_path_buf())
            }
            Role::Pointer(pointer) => Some(normalize(&dir.join(pointer).join("Cargo.toml"))),
            Role::Member => None,
        }
    }
}

/// Whether a workspace rooted at `root` leaves the package at `manifest` out: a path in
/// `exclude` holds it and no path in `members` does. The paths are prefixes, not globs
/// (`src/workspace/workspace.rs:1989-2003`).
fn excludes(root: &Path, members: &[String], exclude: &[String], manifest: &Path) -> bool {
    let holds = |paths: &[String]| {
        paths
            .iter()
            .any(|path| manifest.starts_with(root.join(path)))
    };
    holds(exclude) && !holds(members)
}

/// The first manifest above `manifest` that names a root for it: the folders above the
/// package's own, stopping inside a `target/package` folder (where `cargo package` unpacks
/// crates) and at `cargo_home`; an unparsable manifest on the way fails the search
/// (`src/workspace/workspace.rs:2239-2310`).
fn search_above(manifest: &Path, cargo_home: &Path) -> Result<Option<PathBuf>, PlanError> {
    let mut below: Option<&Path> = None;
    for folder in manifest.ancestors().skip(2) {
        if folder.ends_with("target/package") || below == Some(cargo_home) {
            break;
        }
        below = Some(folder);
        let candidate = folder.join("Cargo.toml");
        if !exists(&candidate)? {
            continue;
        }
        if let Some(root) = Role::of(&candidate)?.root(&candidate, manifest) {
            return Ok(Some(root));
        }
    }
    Ok(None)
}

/// The array of strings at `key` in `table`, empty when absent.
fn strings(table: &toml::Table, key: &str, manifest: &Path) -> Result<Vec<String>, PlanError> {
    let Some(value) = table.get(key) else {
        return Ok(Vec::new());
    };
    let wrong = || wrong_kind(manifest, &format!("workspace.{key}"), "an array of strings");
    value
        .as_array()
        .ok_or_else(wrong)?
        .iter()
        .map(|item| item.as_str().map(str::to_string).ok_or_else(wrong))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::map::test_support::TempDir;

    const PACKAGE: &str = "[package]\nname = \"p\"\nversion = \"0.1.0\"\n";

    fn root_of(fixture: &TempDir, dir: &str) -> Result<PathBuf, PlanError> {
        find(&fixture.path(dir), &fixture.path("home/.cargo")).map(|ws| ws.root().to_path_buf())
    }

    #[test]
    fn a_manifest_with_a_workspace_table_is_its_own_root() {
        let fixture = TempDir::new("ws-own-root");
        fixture.write("ws/Cargo.toml", "[workspace]\nmembers = [\"a\"]\n");
        fixture.dir("ws/src/deep");
        assert_eq!(root_of(&fixture, "ws"), Ok(fixture.path("ws")));
        assert_eq!(
            root_of(&fixture, "ws/src/deep"),
            Ok(fixture.path("ws")),
            "the nearest manifest is found from below"
        );
    }

    #[test]
    fn a_member_finds_the_workspace_above_it() {
        let fixture = TempDir::new("ws-member");
        fixture.write("ws/Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        fixture.write("ws/crates/a/Cargo.toml", PACKAGE);
        assert_eq!(root_of(&fixture, "ws/crates/a"), Ok(fixture.path("ws")));
    }

    #[test]
    fn a_package_the_workspace_excludes_is_its_own_root_unless_a_member_path_holds_it() {
        let fixture = TempDir::new("ws-excluded");
        fixture.write(
            "ws/Cargo.toml",
            "[workspace]\nmembers = [\"crates/kept\"]\nexclude = [\"crates\"]\n",
        );
        fixture.write("ws/crates/gone/Cargo.toml", PACKAGE);
        fixture.write("ws/crates/kept/Cargo.toml", PACKAGE);
        assert_eq!(
            root_of(&fixture, "ws/crates/gone"),
            Ok(fixture.path("ws/crates/gone"))
        );
        assert_eq!(root_of(&fixture, "ws/crates/kept"), Ok(fixture.path("ws")));
    }

    #[test]
    fn a_package_workspace_pointer_names_the_root_without_a_search() {
        let fixture = TempDir::new("ws-pointer");
        fixture.write("ws/Cargo.toml", "[workspace]\nmembers = [\"../outside\"]\n");
        fixture.write(
            "outside/Cargo.toml",
            "[package]\nname = \"o\"\nworkspace = \"../ws\"\n",
        );
        assert_eq!(root_of(&fixture, "outside"), Ok(fixture.path("ws")));
    }

    #[test]
    fn an_ancestor_package_with_a_pointer_redirects_the_package_below_it() {
        let fixture = TempDir::new("ws-ancestor-pointer");
        fixture.write("ws/Cargo.toml", "[workspace]\n");
        fixture.write(
            "outer/Cargo.toml",
            "[package]\nname = \"o\"\nworkspace = \"../ws\"\n",
        );
        fixture.write("outer/inner/Cargo.toml", PACKAGE);
        assert_eq!(root_of(&fixture, "outer/inner"), Ok(fixture.path("ws")));
    }

    #[test]
    fn a_package_with_no_workspace_above_is_its_own_root() {
        let fixture = TempDir::new("ws-standalone");
        fixture.write("app/Cargo.toml", PACKAGE);
        fixture.write("app/sub/Cargo.toml", PACKAGE);
        assert_eq!(root_of(&fixture, "app/sub"), Ok(fixture.path("app/sub")));
    }

    #[test]
    fn the_search_stops_in_target_package_and_at_cargo_home() {
        let fixture = TempDir::new("ws-stops");
        fixture.write("ws/Cargo.toml", "[workspace]\n");
        fixture.write("ws/target/package/p-0.1.0/Cargo.toml", PACKAGE);
        fixture.write("home/Cargo.toml", "[workspace]\n");
        fixture.write("home/.cargo/registry/src/p-0.1.0/Cargo.toml", PACKAGE);
        assert_eq!(
            root_of(&fixture, "ws/target/package/p-0.1.0"),
            Ok(fixture.path("ws/target/package/p-0.1.0"))
        );
        assert_eq!(
            root_of(&fixture, "home/.cargo/registry/src/p-0.1.0"),
            Ok(fixture.path("home/.cargo/registry/src/p-0.1.0"))
        );
    }

    #[test]
    fn an_unparsable_manifest_on_the_way_up_fails_the_search() {
        let fixture = TempDir::new("ws-unparsable");
        let broken = fixture.write("ws/Cargo.toml", "this is = = not toml\n");
        fixture.write("ws/child/Cargo.toml", PACKAGE);
        let Err(PlanError::Cargo(CargoRefusal::Invalid { file, .. })) =
            root_of(&fixture, "ws/child")
        else {
            panic!("expected the broken ancestor to fail the search");
        };
        assert_eq!(file, broken);
    }

    #[test]
    fn a_folder_that_cannot_be_inspected_refuses_instead_of_finding_a_manifest_above() {
        let fixture = TempDir::new("ws-sealed");
        fixture.write("ws/Cargo.toml", "[workspace]\n");
        let sealed = fixture.dir("ws/sealed");
        fixture.restrict(&sealed, 0o000);

        assert_eq!(
            root_of(&fixture, "ws/sealed"),
            Err(PlanError::Unreadable {
                path: sealed.join("Cargo.toml"),
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn a_manifest_above_that_cannot_be_inspected_refuses_the_search() {
        let fixture = TempDir::new("ws-sealed-above");
        fixture.write("ws/app/Cargo.toml", PACKAGE);
        fixture.write("sealed/Cargo.toml", "[workspace]\n");
        let linked = fixture.path("ws/Cargo.toml");
        symlink(fixture.path("sealed/Cargo.toml"), &linked).unwrap();
        fixture.restrict(&fixture.path("sealed"), 0o000);

        assert_eq!(
            root_of(&fixture, "ws/app"),
            Err(PlanError::Unreadable {
                path: linked,
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn a_dangling_link_named_like_a_manifest_is_no_manifest() {
        let fixture = TempDir::new("ws-dangling");
        fixture.write("ws/Cargo.toml", "[workspace]\n");
        fixture.dir("ws/app");
        symlink(fixture.path("gone"), fixture.path("ws/app/Cargo.toml")).unwrap();
        assert_eq!(root_of(&fixture, "ws/app"), Ok(fixture.path("ws")));
    }

    #[test]
    fn no_manifest_anywhere_above_is_said_to_be() {
        let fixture = TempDir::new("ws-none");
        let dir = fixture.dir("loose/target");
        assert_eq!(
            root_of(&fixture, "loose/target"),
            Err(PlanError::Cargo(CargoRefusal::NoManifest {
                searched_from: dir
            }))
        );
    }
}
