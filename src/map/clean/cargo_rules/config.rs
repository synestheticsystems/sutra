//! The two settings that move cargo's build folders, `build.target-dir` and `build.build-dir`,
//! read from cargo's config files as cargo reads them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{CargoRefusal, IncludeProblem, exists, normalize, read_toml, wrong_kind};
use crate::map::clean::PlanError;

/// A setting's value and the config file that set it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Setting {
    pub value: String,
    pub file: PathBuf,
}

impl Setting {
    /// What a relative value is relative to: the folder holding the folder the file is in,
    /// which for `<dir>/.cargo/config.toml` is `<dir>` (`src/context/value.rs:103-113`).
    pub(super) fn base(&self) -> &Path {
        let dir = self.file.parent().unwrap_or(Path::new("/"));
        dir.parent().unwrap_or(dir)
    }
}

/// The two settings as the config files cargo reads set them, each `None` when none does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct FolderSettings {
    pub target_dir: Option<Setting>,
    pub build_dir: Option<Setting>,
}

impl FolderSettings {
    /// The settings cargo reads when run in `cwd`. It reads `.cargo/config`, else
    /// `.cargo/config.toml`, in `cwd` and in every folder above it, the nearest first, then the
    /// same in `cargo_home` (`src/context/mod.rs:1646-1721`). The nearest file that sets a
    /// setting wins (`src/context/config_value.rs:213-217`). Cargo skips `cargo_home` when the
    /// search has passed through it already; reading it a second time, last, changes nothing.
    pub(super) fn load(cwd: &Path, cargo_home: &Path) -> Result<FolderSettings, PlanError> {
        let mut raw = RawSettings::default();
        let config_dirs = cwd.ancestors().map(|folder| folder.join(".cargo"));
        for config_dir in config_dirs.chain([cargo_home.to_path_buf()]) {
            if let Some(file) = config_file_in(&config_dir)? {
                raw.keep_nearer(load_file(&file)?);
            }
        }
        let read = |setting: Option<RawSetting>, key| setting.map(|s| s.into_setting(key));
        Ok(FolderSettings {
            target_dir: read(raw.target_dir, "target-dir").transpose()?,
            build_dir: read(raw.build_dir, "build-dir").transpose()?,
        })
    }
}

/// A setting as a config file sets it, of whatever kind. Cargo reads a value's kind only once
/// the files are merged, so a value of the wrong kind is an error only when it wins.
#[derive(Debug, Clone)]
struct RawSetting {
    value: toml::Value,
    file: PathBuf,
}

impl RawSetting {
    /// The setting as the string cargo requires, `key` naming it under `[build]`.
    fn into_setting(self, key: &str) -> Result<Setting, PlanError> {
        match self.value {
            toml::Value::String(value) => Ok(Setting {
                value,
                file: self.file,
            }),
            _ => Err(wrong_kind(&self.file, &format!("build.{key}"), "a string")),
        }
    }
}

/// The two settings, before their kinds are read.
#[derive(Debug, Clone, Default)]
struct RawSettings {
    target_dir: Option<RawSetting>,
    build_dir: Option<RawSetting>,
}

impl RawSettings {
    /// Fill each setting still unset from `farther`.
    fn keep_nearer(&mut self, farther: RawSettings) {
        self.target_dir = self.target_dir.take().or(farther.target_dir);
        self.build_dir = self.build_dir.take().or(farther.build_dir);
    }

    /// Replace each setting that `later` sets.
    fn take_later(&mut self, later: RawSettings) {
        self.target_dir = later.target_dir.or(self.target_dir.take());
        self.build_dir = later.build_dir.or(self.build_dir.take());
    }
}

/// `dir`'s config file: `config` when there is one, else `config.toml`.
fn config_file_in(dir: &Path) -> Result<Option<PathBuf>, PlanError> {
    for name in ["config", "config.toml"] {
        let file = dir.join(name);
        if exists(&file)? {
            return Ok(Some(file));
        }
    }
    Ok(None)
}

/// The settings a config file found by the search makes.
fn load_file(file: &Path) -> Result<RawSettings, PlanError> {
    load_with_includes(file, &mut HashSet::from([file.to_path_buf()]))
}

/// The settings `file` makes: its includes loaded first, each later one over the earlier, and
/// its own settings over them all (`src/context/mod.rs:1386-1457`). `loaded` holds every file this
/// load has read, as cargo refuses to read one twice.
fn load_with_includes(
    file: &Path,
    loaded: &mut HashSet<PathBuf>,
) -> Result<RawSettings, PlanError> {
    let table = read_toml(file)?;
    let mut settings = RawSettings::default();
    for include in includes(&table, file)? {
        let bad = |problem| {
            PlanError::Cargo(CargoRefusal::BadInclude {
                file: file.to_path_buf(),
                include: include.path.clone(),
                problem,
            })
        };
        let path = normalize(&file.parent().unwrap_or(Path::new("/")).join(&include.path));
        if !exists(&path)? {
            if include.optional {
                continue;
            }
            return Err(bad(IncludeProblem::Missing));
        }
        if !loaded.insert(path.clone()) {
            return Err(bad(IncludeProblem::Cycle));
        }
        settings.take_later(load_with_includes(&path, loaded)?);
    }
    settings.take_later(settings_in(&table, file)?);
    Ok(settings)
}

/// One entry of a config file's `include` list.
struct Include {
    path: String,
    optional: bool,
}

/// The `include` list of the config file `file`, checked as cargo checks it
/// (`src/context/mod.rs:1460-1549`).
fn includes(table: &toml::Table, file: &Path) -> Result<Vec<Include>, PlanError> {
    let Some(value) = table.get("include") else {
        return Ok(Vec::new());
    };
    let wrong = || wrong_kind(file, "include", "a list of strings or tables");
    let mut includes = Vec::new();
    for entry in value.as_array().ok_or_else(wrong)? {
        let include = match entry {
            toml::Value::String(path) => Include {
                path: path.clone(),
                optional: false,
            },
            toml::Value::Table(entry) => Include {
                path: entry
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| wrong_kind(file, "include.path", "a string"))?
                    .to_string(),
                optional: match entry.get("optional") {
                    None => false,
                    Some(optional) => optional
                        .as_bool()
                        .ok_or_else(|| wrong_kind(file, "include.optional", "a boolean"))?,
                },
            },
            _ => return Err(wrong()),
        };
        let problem = if Path::new(&include.path).extension() != Some("toml".as_ref()) {
            Some(IncludeProblem::NotToml)
        } else if include.path.contains(['*', '?', '[', ']', '{', '}']) {
            Some(IncludeProblem::Pattern)
        } else {
            None
        };
        if let Some(problem) = problem {
            return Err(PlanError::Cargo(CargoRefusal::BadInclude {
                file: file.to_path_buf(),
                include: include.path,
                problem,
            }));
        }
        includes.push(include);
    }
    Ok(includes)
}

/// The two settings as `file`'s own table sets them.
fn settings_in(table: &toml::Table, file: &Path) -> Result<RawSettings, PlanError> {
    let Some(build) = table.get("build") else {
        return Ok(RawSettings::default());
    };
    let build = build
        .as_table()
        .ok_or_else(|| wrong_kind(file, "build", "a table"))?;
    let setting = |key: &str| {
        build.get(key).map(|value| RawSetting {
            value: value.clone(),
            file: file.to_path_buf(),
        })
    };
    Ok(RawSettings {
        target_dir: setting("target-dir"),
        build_dir: setting("build-dir"),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::map::test_support::TempDir;

    fn target_dir(fixture: &TempDir, cwd: &str) -> Result<Option<(String, PathBuf)>, PlanError> {
        FolderSettings::load(&fixture.path(cwd), &fixture.path("home/.cargo"))
            .map(|settings| settings.target_dir.map(|s| (s.value, s.file)))
    }

    fn set(value: &str, file: PathBuf) -> Result<Option<(String, PathBuf)>, PlanError> {
        Ok(Some((value.to_string(), file)))
    }

    #[test]
    fn the_nearest_file_wins_and_the_cargo_home_file_comes_last() {
        let fixture = TempDir::new("config-nearest");
        let near = fixture.write(
            "ws/crates/a/.cargo/config.toml",
            "[build]\ntarget-dir = \"near\"\n",
        );
        let far = fixture.write(
            "ws/.cargo/config.toml",
            "[build]\ntarget-dir = \"far\"\nbuild-dir = \"b\"\n",
        );
        let home = fixture.write(
            "home/.cargo/config.toml",
            "[build]\ntarget-dir = \"home\"\n",
        );
        fixture.dir("other");

        assert_eq!(target_dir(&fixture, "ws/crates/a"), set("near", near));
        assert_eq!(target_dir(&fixture, "ws"), set("far", far.clone()));
        assert_eq!(target_dir(&fixture, "other"), set("home", home));
        let settings =
            FolderSettings::load(&fixture.path("ws/crates/a"), &fixture.path("home/.cargo"))
                .unwrap();
        assert_eq!(
            settings.build_dir,
            Some(Setting {
                value: "b".to_string(),
                file: far
            }),
            "each setting is looked up on its own"
        );
    }

    #[test]
    fn a_config_folder_that_cannot_be_inspected_refuses_instead_of_falling_to_a_farther_file() {
        let fixture = TempDir::new("config-sealed");
        fixture.write("ws/.cargo/config.toml", "[build]\ntarget-dir = \"far\"\n");
        fixture.dir("ws/app");
        let sealed = fixture.dir("ws/app/.cargo");
        fixture.restrict(&sealed, 0o000);

        assert_eq!(
            target_dir(&fixture, "ws/app"),
            Err(PlanError::Unreadable {
                path: sealed.join("config"),
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn a_legacy_config_that_cannot_be_inspected_refuses_instead_of_falling_to_config_toml() {
        let fixture = TempDir::new("config-sealed-legacy");
        let config = fixture.path("app/.cargo/config");
        fixture.write("app/.cargo/config.toml", "[build]\ntarget-dir = \"toml\"\n");
        fixture.write("sealed/config", "[build]\ntarget-dir = \"legacy\"\n");
        symlink(fixture.path("sealed/config"), &config).unwrap();
        fixture.restrict(&fixture.path("sealed"), 0o000);

        assert_eq!(
            target_dir(&fixture, "app"),
            Err(PlanError::Unreadable {
                path: config,
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn a_cargo_home_that_cannot_be_inspected_refuses_instead_of_falling_to_the_default() {
        let fixture = TempDir::new("config-sealed-home");
        fixture.dir("app");
        let home = fixture.dir("home/.cargo");
        fixture.restrict(&home, 0o000);

        assert_eq!(
            target_dir(&fixture, "app"),
            Err(PlanError::Unreadable {
                path: home.join("config"),
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn an_optional_include_that_cannot_be_inspected_refuses_instead_of_being_skipped() {
        let fixture = TempDir::new("config-sealed-include");
        fixture.write(
            "app/.cargo/config.toml",
            "include = [{ path = \"../sealed/extra.toml\", optional = true }]\n",
        );
        let sealed = fixture.dir("app/sealed");
        fixture.restrict(&sealed, 0o000);

        assert_eq!(
            target_dir(&fixture, "app"),
            Err(PlanError::Unreadable {
                path: sealed.join("extra.toml"),
                kind: io::ErrorKind::PermissionDenied,
            })
        );
    }

    #[test]
    fn includes_load_in_order_under_the_including_files_own_values() {
        let fixture = TempDir::new("config-includes");
        let config = fixture.path("app/.cargo/config.toml");
        fixture.write(
            "app/.cargo/config.toml",
            "include = [\"first.toml\", { path = \"../shared/second.toml\" }, { path = \"gone.toml\", optional = true }]\n",
        );
        fixture.write(
            "app/.cargo/first.toml",
            "[build]\ntarget-dir = \"first\"\nbuild-dir = \"first\"\n",
        );
        let second = fixture.write(
            "app/shared/second.toml",
            "[build]\ntarget-dir = \"second\"\n",
        );
        assert_eq!(target_dir(&fixture, "app"), set("second", second));

        fs::write(
            &config,
            "include = [\"first.toml\"]\n[build]\ntarget-dir = \"own\"\n",
        )
        .unwrap();
        assert_eq!(target_dir(&fixture, "app"), set("own", config));
    }

    #[test]
    fn includes_cargo_rejects_fail_the_load() {
        let fixture = TempDir::new("config-bad-includes");
        let config = fixture.path("app/.cargo/config.toml");
        fixture.write("app/.cargo/loop.toml", "include = [\"loop.toml\"]\n");
        for (include, problem) in [
            ("missing.toml", IncludeProblem::Missing),
            ("extra.json", IncludeProblem::NotToml),
            ("*.toml", IncludeProblem::Pattern),
            ("{env}.toml", IncludeProblem::Pattern),
        ] {
            fixture.write(
                "app/.cargo/config.toml",
                format!("include = [\"{include}\"]\n"),
            );
            assert_eq!(
                target_dir(&fixture, "app"),
                Err(PlanError::Cargo(CargoRefusal::BadInclude {
                    file: config.clone(),
                    include: include.to_string(),
                    problem,
                })),
                "{include}"
            );
        }
        fixture.write("app/.cargo/config.toml", "include = [\"loop.toml\"]\n");
        assert_eq!(
            target_dir(&fixture, "app"),
            Err(PlanError::Cargo(CargoRefusal::BadInclude {
                file: fixture.path("app/.cargo/loop.toml"),
                include: "loop.toml".to_string(),
                problem: IncludeProblem::Cycle,
            }))
        );
    }

    #[test]
    fn a_value_of_the_wrong_kind_fails_only_when_it_wins() {
        let fixture = TempDir::new("config-wrong-kind");
        fixture.write("ws/.cargo/config.toml", "[build]\ntarget-dir = 5\n");
        let near = fixture.write(
            "ws/a/.cargo/config.toml",
            "[build]\ntarget-dir = \"near\"\n",
        );
        assert_eq!(target_dir(&fixture, "ws/a"), set("near", near));
        assert_eq!(
            target_dir(&fixture, "ws"),
            Err(PlanError::Cargo(CargoRefusal::Invalid {
                file: fixture.path("ws/.cargo/config.toml"),
                problem: "`build.target-dir` must be a string".to_string(),
            }))
        );
    }

    #[test]
    fn a_file_that_is_not_toml_fails_the_load_naming_it_and_the_line() {
        let fixture = TempDir::new("config-not-toml");
        let config = fixture.write("app/.cargo/config.toml", "[build]\n[build\n");
        let Err(PlanError::Cargo(CargoRefusal::Invalid { file, problem })) =
            target_dir(&fixture, "app")
        else {
            panic!("expected the file to be refused");
        };
        assert_eq!(file, config);
        assert!(problem.ends_with("on line 2"), "{problem}");
    }
}
