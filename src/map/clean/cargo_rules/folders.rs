//! How cargo combines its environment, its config files and its defaults into the folders a bare
//! `cargo clean` deletes: the target-dir (`src/context/mod.rs:708-736`), and the build-dir
//! (`src/context/mod.rs:741-825`), which is the target-dir unless set on its own
//! (`src/workspace/workspace.rs:491-525`).

use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::config::{FolderSettings, Setting};
use super::workspace::Workspace;
use super::{CargoEnv, CargoRefusal};
use crate::map::clean::PlanError;

/// Where a build folder's location came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Cargo's default: `target` in the workspace root.
    Default,
    /// One of Sutra's own environment variables, named here.
    Env(&'static str),
    /// A config file's `key`.
    Config { key: &'static str, file: PathBuf },
}

/// A build folder as cargo resolves it, before anything about the disk is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Resolved {
    pub path: PathBuf,
    pub origin: Origin,
}

/// The target-dir, then the build-dir when it is set on its own. `cwd` is where the clean runs,
/// which environment variables are relative to.
pub(super) fn resolve(
    workspace: &Workspace,
    settings: &FolderSettings,
    env: &CargoEnv,
    cwd: &Path,
    cargo_home: &Path,
) -> Result<Vec<Resolved>, PlanError> {
    let target = if let Some(dir) = &env.target_dir {
        from_env(dir, "CARGO_TARGET_DIR", cwd)?
    } else if let Some(dir) = &env.build_target_dir {
        from_env(dir, "CARGO_BUILD_TARGET_DIR", cwd)?
    } else if let Some(setting) = &settings.target_dir {
        from_config(setting, "build.target-dir")?
    } else {
        Resolved {
            path: workspace.root().join("target"),
            origin: Origin::Default,
        }
    };
    let build = if let Some(dir) = &env.build_build_dir {
        let origin = Origin::Env("CARGO_BUILD_BUILD_DIR");
        let template = dir.to_str().ok_or_else(|| not_unicode(origin.clone()))?;
        Some(templated(template, cwd, origin, workspace, cargo_home)?)
    } else if let Some(setting) = &settings.build_dir {
        let origin = Origin::Config {
            key: "build.build-dir",
            file: setting.file.clone(),
        };
        Some(templated(
            &setting.value,
            setting.base(),
            origin,
            workspace,
            cargo_home,
        )?)
    } else {
        None
    };
    Ok(std::iter::once(target).chain(build).collect())
}

fn from_env(dir: &OsStr, var: &'static str, cwd: &Path) -> Result<Resolved, PlanError> {
    let origin = Origin::Env(var);
    if dir.is_empty() {
        return Err(PlanError::Cargo(CargoRefusal::Empty { origin }));
    }
    Ok(Resolved {
        path: cwd.join(dir),
        origin,
    })
}

fn from_config(setting: &Setting, key: &'static str) -> Result<Resolved, PlanError> {
    let origin = Origin::Config {
        key,
        file: setting.file.clone(),
    };
    if setting.value.is_empty() {
        return Err(PlanError::Cargo(CargoRefusal::Empty { origin }));
    }
    Ok(Resolved {
        path: setting.base().join(&setting.value),
        origin,
    })
}

/// A `build.build-dir` value with its variables filled in, relative to `base`
/// (`src/context/path.rs:41-76`).
fn templated(
    template: &str,
    base: &Path,
    origin: Origin,
    workspace: &Workspace,
    cargo_home: &Path,
) -> Result<Resolved, PlanError> {
    let text = |path: &Path| {
        path.to_str()
            .map(str::to_string)
            .ok_or_else(|| not_unicode(origin.clone()))
    };
    let filled = template
        .replace("{workspace-root}", &text(workspace.root())?)
        .replace("{cargo-cache-home}", &text(cargo_home)?)
        .replace(
            "{workspace-path-hash}",
            &workspace_path_hash(&workspace.manifest),
        );
    if filled.contains(['{', '}']) {
        return Err(PlanError::Cargo(CargoRefusal::BadTemplate {
            origin,
            template: template.to_string(),
        }));
    }
    if template.is_empty() {
        return Err(PlanError::Cargo(CargoRefusal::Empty { origin }));
    }
    Ok(Resolved {
        path: base.join(filled),
        origin,
    })
}

/// `{workspace-path-hash}`: the canonical root manifest path hashed as cargo hashes it, in hex,
/// its first two characters a folder of their own (`src/context/mod.rs:775-780`,
/// `src/util/hex.rs:6-31`).
fn workspace_path_hash(manifest: &Path) -> String {
    let real = fs::canonicalize(manifest).unwrap_or_else(|_| manifest.to_path_buf());
    let mut hasher = rustc_stable_hash::StableSipHasher128::new();
    real.hash(&mut hasher);
    let hex: String = Hasher::finish(&hasher)
        .to_le_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}/{}", &hex[..2], &hex[2..])
}

fn not_unicode(origin: Origin) -> PlanError {
    PlanError::Cargo(CargoRefusal::NotUnicode { origin })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::map::test_support::{TempDir, cargo_env};

    fn env() -> CargoEnv {
        cargo_env("/Users/dev")
    }

    fn setting(value: &str, file: &str) -> Option<Setting> {
        Some(Setting {
            value: value.to_string(),
            file: PathBuf::from(file),
        })
    }

    /// The folders of the workspace `/code/app`, cleaned there.
    fn folders(settings: FolderSettings, env: &CargoEnv) -> Result<Vec<Resolved>, PlanError> {
        let workspace = Workspace {
            manifest: PathBuf::from("/code/app/Cargo.toml"),
        };
        resolve(
            &workspace,
            &settings,
            env,
            Path::new("/code/app"),
            Path::new("/Users/dev/.cargo"),
        )
    }

    fn resolved(path: &str, origin: Origin) -> Resolved {
        Resolved {
            path: PathBuf::from(path),
            origin,
        }
    }

    #[test]
    fn cargo_target_dir_beats_cargo_build_target_dir_which_beats_config() {
        let settings = FolderSettings {
            target_dir: setting("cfg", "/code/app/.cargo/config.toml"),
            build_dir: None,
        };
        let config = Origin::Config {
            key: "build.target-dir",
            file: PathBuf::from("/code/app/.cargo/config.toml"),
        };
        assert_eq!(
            folders(settings.clone(), &env()),
            Ok(vec![resolved("/code/app/cfg", config)])
        );
        let with_build_var = CargoEnv {
            build_target_dir: Some(OsString::from("from-build-var")),
            ..env()
        };
        assert_eq!(
            folders(settings.clone(), &with_build_var),
            Ok(vec![resolved(
                "/code/app/from-build-var",
                Origin::Env("CARGO_BUILD_TARGET_DIR")
            )])
        );
        let with_both = CargoEnv {
            target_dir: Some(OsString::from("/abs/out")),
            ..with_build_var
        };
        assert_eq!(
            folders(settings, &with_both),
            Ok(vec![resolved("/abs/out", Origin::Env("CARGO_TARGET_DIR"))])
        );
    }

    #[test]
    fn a_build_dir_from_the_environment_names_its_variable_as_where_it_came_from() {
        let from_var = CargoEnv {
            build_build_dir: Some(OsString::from("cache")),
            ..env()
        };
        assert_eq!(
            folders(FolderSettings::default(), &from_var).unwrap()[1],
            resolved("/code/app/cache", Origin::Env("CARGO_BUILD_BUILD_DIR"))
        );
    }

    #[test]
    fn the_workspace_path_hash_is_of_the_real_path_so_a_link_to_the_workspace_hashes_alike() {
        let fixture = TempDir::new("folders-path-hash");
        let manifest = fixture.write("real/Cargo.toml", "[package]\n");
        std::os::unix::fs::symlink(fixture.path("real"), fixture.path("link")).unwrap();
        let hash = workspace_path_hash(&manifest);
        assert_eq!(workspace_path_hash(&fixture.path("link/Cargo.toml")), hash);
        assert_eq!(
            hash.find('/'),
            Some(2),
            "its first two characters are a folder"
        );
        assert_ne!(
            hash,
            workspace_path_hash(Path::new("/elsewhere/Cargo.toml"))
        );
    }

    #[test]
    fn empty_settings_and_unknown_template_variables_are_cargos_errors() {
        let empty_var = CargoEnv {
            target_dir: Some(OsString::new()),
            ..env()
        };
        assert_eq!(
            folders(FolderSettings::default(), &empty_var),
            Err(PlanError::Cargo(CargoRefusal::Empty {
                origin: Origin::Env("CARGO_TARGET_DIR")
            }))
        );
        let file = "/code/app/.cargo/config.toml";
        let empty_config = FolderSettings {
            target_dir: setting("", file),
            build_dir: None,
        };
        assert!(matches!(
            folders(empty_config, &env()),
            Err(PlanError::Cargo(CargoRefusal::Empty { .. }))
        ));
        for template in ["{workspace-rot}/x", "x/{", "x}/y"] {
            let unknown = FolderSettings {
                target_dir: None,
                build_dir: setting(template, file),
            };
            assert_eq!(
                folders(unknown, &env()),
                Err(PlanError::Cargo(CargoRefusal::BadTemplate {
                    origin: Origin::Config {
                        key: "build.build-dir",
                        file: PathBuf::from(file),
                    },
                    template: template.to_string(),
                })),
                "{template}"
            );
        }
    }
}
