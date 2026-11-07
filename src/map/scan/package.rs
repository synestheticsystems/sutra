//! The packages a folder builds, by the names their manifests give them: a Cargo package's
//! `[package] name`, and a `package.json`'s `name`. A folder with no git is grouped on the list by
//! the package it builds, beside the checkouts that build a package of that name.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Failure;

/// What one manifest says of the folder that holds it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Manifest {
    /// The name of the package it describes, if it describes one.
    name: Option<String>,
    /// The members of the workspace it roots, as the globs it names them by; `None` for a manifest
    /// that roots no workspace.
    members: Option<Vec<String>>,
    /// The paths the workspace leaves out of what its globs name.
    exclude: Vec<String>,
}

/// The two kinds of manifest, each in its own file in the folder it describes.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Cargo,
    Npm,
}

/// The package `folder` builds, when it builds one and roots no workspace: its Cargo package's name,
/// else its npm package's. A workspace root names no package of its own here: what it builds is its
/// members. A manifest that is no valid manifest names nothing, which is no failure of Sutra's:
/// the folder's own build will say what is wrong with it. `Err` names a manifest that could not be
/// read.
pub fn own_package(folder: &Path) -> Result<Option<String>, Failure> {
    for kind in [Kind::Cargo, Kind::Npm] {
        let Some(manifest) = kind.read(folder)? else {
            continue;
        };
        if manifest.members.is_some() {
            return Ok(None);
        }
        if manifest.name.is_some() {
            return Ok(manifest.name);
        }
    }
    Ok(None)
}

/// Every package the checkout at `folder` builds: those its own manifests name, a workspace root's
/// included, and each member's of the Cargo workspace or the npm workspaces it roots. Returns them,
/// sorted, each once, and the manifests and folders that could not be read.
pub fn packages_built(folder: &Path) -> (Vec<String>, Vec<Failure>) {
    let mut names = Vec::new();
    let mut failures = Vec::new();
    for kind in [Kind::Cargo, Kind::Npm] {
        let manifest = match kind.read(folder) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => continue,
            Err(failure) => {
                failures.push(failure);
                continue;
            }
        };
        names.extend(manifest.name);
        let members = manifest.members.unwrap_or_default();
        let (included, excluded): (Vec<&String>, Vec<&String>) =
            members.iter().partition(|glob| !glob.starts_with('!'));
        let excluded = excluded.into_iter().map(|glob| &glob[1..]);
        let excluded: Vec<PathBuf> = excluded
            .chain(manifest.exclude.iter().map(String::as_str))
            .map(|path| folder.join(path))
            .collect();
        for glob in included {
            let named = named_by(folder, glob, &mut failures);
            let kept = named
                .into_iter()
                .filter(|member| !excluded.iter().any(|out| member.starts_with(out)));
            for member in kept {
                match kind.read(&member) {
                    Ok(Some(manifest)) => names.extend(manifest.name),
                    Ok(None) => {}
                    Err(failure) => failures.push(failure),
                }
            }
        }
    }
    names.sort();
    names.dedup();
    (names, failures)
}

impl Kind {
    fn file(self) -> &'static str {
        match self {
            Kind::Cargo => "Cargo.toml",
            Kind::Npm => "package.json",
        }
    }

    /// What the manifest of this kind in `folder` says; `None` when there is none, or it is no
    /// valid manifest.
    fn read(self, folder: &Path) -> Result<Option<Manifest>, Failure> {
        let file = folder.join(self.file());
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(None);
            }
            Err(err) => return Err((file, err.kind())),
        };
        Ok(match self {
            Kind::Cargo => cargo_manifest(&text),
            Kind::Npm => npm_manifest(&text),
        })
    }
}

/// What a `Cargo.toml` says: its `[package] name` (`[project]` before the 2024 edition), and the
/// members and exclusions of its `[workspace]`.
fn cargo_manifest(text: &str) -> Option<Manifest> {
    let table: toml::Table = text.parse().ok()?;
    let package = table.get("package").or_else(|| table.get("project"));
    let name = package
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str);
    let workspace = table.get("workspace");
    let strings = |key: &str| {
        let listed = workspace.and_then(|workspace| workspace.get(key));
        let listed = listed.and_then(toml::Value::as_array).into_iter().flatten();
        let strings = listed.filter_map(toml::Value::as_str).map(str::to_string);
        strings.collect::<Vec<String>>()
    };
    Some(Manifest {
        name: name.map(str::to_string),
        members: workspace.map(|_| strings("members")),
        exclude: strings("exclude"),
    })
}

/// What a `package.json` says: its `name`, and its `workspaces`, a list of globs or, as Yarn writes
/// them, an object listing them under `packages`. A glob that starts with `!` leaves out what it
/// names.
fn npm_manifest(text: &str) -> Option<Manifest> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let name = json.get("name").and_then(serde_json::Value::as_str);
    let workspaces = json.get("workspaces").map(|workspaces| {
        let globs = workspaces.get("packages").unwrap_or(workspaces);
        let globs = globs.as_array().into_iter().flatten();
        let globs = globs
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string);
        globs.collect()
    });
    Some(Manifest {
        name: name.map(str::to_string),
        members: workspaces,
        exclude: Vec::new(),
    })
}

/// The folders in `root` that `glob`, a workspace member's path, names: each part with a `*` or a
/// `?` matched against the names of the folders where it stands, as Cargo and npm match them, and
/// each other part taken as written. A glob with `**`, which reaches into every depth, names none.
/// The folders that could not be listed go to `failures`.
fn named_by(root: &Path, glob: &str, failures: &mut Vec<Failure>) -> Vec<PathBuf> {
    let parts: Vec<&str> = glob.split('/').filter(|part| !part.is_empty()).collect();
    if parts.contains(&"**") {
        return Vec::new();
    }
    let mut folders = vec![root.to_path_buf()];
    for part in parts {
        if part == "." {
            continue;
        }
        if !part.contains(['*', '?']) {
            folders = folders
                .into_iter()
                .map(|folder| folder.join(part))
                .collect();
            continue;
        }
        let mut matched = Vec::new();
        for folder in folders {
            let entries = match fs::read_dir(&folder) {
                Ok(entries) => entries,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    failures.push((folder, err.kind()));
                    continue;
                }
            };
            for entry in entries.flatten() {
                let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
                let name = entry.file_name();
                let named = name.to_str().is_some_and(|name| wildcard_match(part, name));
                if is_folder && named {
                    matched.push(entry.path());
                }
            }
        }
        matched.sort();
        folders = matched;
    }
    folders
}

/// Whether `name` matches `pattern`, in which `*` stands for any run of characters and `?` for any
/// one.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let (pattern, name): (Vec<char>, Vec<char>) =
        (pattern.chars().collect(), name.chars().collect());
    // The pattern's place and the name's after the last `*`, to go back to when a match fails.
    let (mut p, mut n, mut star) = (0, 0, None);
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((star_p, star_n)) => {
                    p = star_p + 1;
                    n = star_n + 1;
                    star = Some((star_p, star_n + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    fn cargo(name: &str) -> String {
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n")
    }

    #[test]
    fn a_folder_builds_its_cargo_package_else_its_npm_package() {
        let fixture = TempDir::new("package-own");
        fixture.write("both/Cargo.toml", cargo("ankurah-core"));
        fixture.write("both/package.json", r#"{"name": "ankurah-web"}"#);
        fixture.write("npm/package.json", r#"{"name": "@ankurah/react"}"#);
        fixture.dir("neither");
        let own = |folder: &str| own_package(&fixture.path(folder)).unwrap();
        assert_eq!(own("both").as_deref(), Some("ankurah-core"));
        assert_eq!(own("npm").as_deref(), Some("@ankurah/react"));
        assert_eq!(own("neither"), None);
        assert_eq!(
            own("gone"),
            None,
            "a folder removed meanwhile builds nothing"
        );
    }

    #[test]
    fn a_workspace_root_names_no_package_of_its_own_and_an_invalid_manifest_names_none() {
        let fixture = TempDir::new("package-workspace-root");
        let rooted = format!("{}[workspace]\nmembers = [\"ffi\"]\n", cargo("sutra"));
        fixture.write("sutra/Cargo.toml", rooted);
        fixture.write(
            "web/package.json",
            r#"{"name": "web", "workspaces": ["packages/*"]}"#,
        );
        fixture.write("broken/Cargo.toml", "this is = = not toml\n");
        fixture.write("broken/package.json", r#"{"name": "broken-web"}"#);
        let own = |folder: &str| own_package(&fixture.path(folder)).unwrap();
        assert_eq!(own("sutra"), None);
        assert_eq!(own("web"), None);
        assert_eq!(
            own("broken").as_deref(),
            Some("broken-web"),
            "the valid manifest beside it speaks"
        );
    }

    #[test]
    fn a_manifest_that_cannot_be_read_is_a_failure() {
        let fixture = TempDir::new("package-sealed");
        let manifest = fixture.write("app/Cargo.toml", cargo("app"));
        fixture.restrict(&manifest, 0o000);
        assert_eq!(
            own_package(&fixture.path("app")),
            Err((manifest.clone(), io::ErrorKind::PermissionDenied))
        );
        let (names, failures) = packages_built(&fixture.path("app"));
        assert_eq!(names, [""; 0]);
        assert_eq!(failures, [(manifest, io::ErrorKind::PermissionDenied)]);
    }

    #[test]
    fn a_checkout_builds_its_own_package_and_its_cargo_workspaces_members_less_those_left_out() {
        let fixture = TempDir::new("package-cargo-members");
        let root = format!(
            "{}[workspace]\nmembers = [\"crates/*\", \"cli\", \"./tools/ankurah-*\"]\n\
             exclude = [\"crates/scratch\"]\n",
            cargo("ankurah")
        );
        fixture.write("ankurah/Cargo.toml", root);
        fixture.write("ankurah/crates/core/Cargo.toml", cargo("ankurah-core"));
        fixture.write("ankurah/crates/proto/Cargo.toml", cargo("ankurah-proto"));
        fixture.write("ankurah/crates/scratch/Cargo.toml", cargo("left-out"));
        fixture.dir("ankurah/crates/docs");
        fixture.write("ankurah/cli/Cargo.toml", cargo("ankurah-cli"));
        fixture.write(
            "ankurah/tools/ankurah-bench/Cargo.toml",
            cargo("ankurah-bench"),
        );
        fixture.write("ankurah/tools/other/Cargo.toml", cargo("not-named"));

        let (names, failures) = packages_built(&fixture.path("ankurah"));

        assert_eq!(
            names,
            [
                "ankurah",
                "ankurah-bench",
                "ankurah-cli",
                "ankurah-core",
                "ankurah-proto"
            ]
        );
        assert_eq!(failures, []);
    }

    #[test]
    fn a_checkout_builds_its_npm_workspaces_members_listed_either_way_less_those_negated() {
        let fixture = TempDir::new("package-npm-members");
        fixture.write(
            "web/package.json",
            r#"{"name": "web", "workspaces": ["packages/*", "!packages/old"]}"#,
        );
        fixture.write("web/packages/ui/package.json", r#"{"name": "@web/ui"}"#);
        fixture.write("web/packages/old/package.json", r#"{"name": "@web/old"}"#);
        fixture.write(
            "yarn/package.json",
            r#"{"workspaces": {"packages": ["apps/**", "libs/*"]}}"#,
        );
        fixture.write("yarn/libs/kit/package.json", r#"{"name": "kit"}"#);
        fixture.write("yarn/apps/site/package.json", r#"{"name": "site"}"#);

        assert_eq!(packages_built(&fixture.path("web")).0, ["@web/ui", "web"]);
        assert_eq!(
            packages_built(&fixture.path("yarn")).0,
            ["kit"],
            "a glob into every depth names none"
        );
    }

    #[test]
    fn a_wildcard_stands_for_any_run_and_a_question_mark_for_one_character() {
        for (pattern, name, matched) in [
            ("*", "core", true),
            ("ankurah-*", "ankurah-core", true),
            ("ankurah-*", "core", false),
            ("*-core", "ankurah-core", true),
            ("a*b*c", "axxbyyc", true),
            ("a*b*c", "axxbyy", false),
            ("v?", "v1", true),
            ("v?", "v10", false),
        ] {
            assert_eq!(wildcard_match(pattern, name), matched, "{pattern} {name}");
        }
    }
}
