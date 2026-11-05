//! The environment variables that move cargo's build folders, as Sutra itself sees them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::normalize;

/// The environment cargo would read for a clean, as far as Sutra can know it: its own process
/// environment, which holds what `launchctl setenv` set and never what a shell's startup files
/// export, because the Finder and the Dock start apps outside any shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoEnv {
    /// The real home folder, whose `.cargo` is `CARGO_HOME` when that is not set. Inside the
    /// sandbox `$HOME` names the app's container instead, so this comes from
    /// `crate::home::real_home`.
    pub home: PathBuf,
    /// `CARGO_HOME`.
    pub cargo_home: Option<OsString>,
    /// `CARGO_TARGET_DIR`.
    pub target_dir: Option<OsString>,
    /// `CARGO_BUILD_TARGET_DIR`.
    pub build_target_dir: Option<OsString>,
    /// `CARGO_BUILD_BUILD_DIR`.
    pub build_build_dir: Option<OsString>,
}

impl CargoEnv {
    /// The variables as this process has them, with `home` the real home folder.
    pub fn of_this_process(home: &Path) -> CargoEnv {
        CargoEnv {
            home: home.to_path_buf(),
            cargo_home: std::env::var_os("CARGO_HOME"),
            target_dir: std::env::var_os("CARGO_TARGET_DIR"),
            build_target_dir: std::env::var_os("CARGO_BUILD_TARGET_DIR"),
            build_build_dir: std::env::var_os("CARGO_BUILD_BUILD_DIR"),
        }
    }

    /// `CARGO_HOME` as cargo works it out when run in `cwd`: the variable when it is set and not
    /// empty, a relative one taken from `cwd`, else `.cargo` in the home folder; then normalized
    /// (`crates/home/src/env.rs:67-81`, `src/context/mod.rs:2182-2189`).
    pub(in crate::map::clean) fn cargo_home_at(&self, cwd: &Path) -> PathBuf {
        let home = match self.cargo_home.as_deref().filter(|home| !home.is_empty()) {
            Some(home) => cwd.join(home),
            None => self.home.join(".cargo"),
        };
        normalize(&home)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::cargo_env;

    fn with_cargo_home(cargo_home: Option<&str>) -> CargoEnv {
        CargoEnv {
            cargo_home: cargo_home.map(OsString::from),
            ..cargo_env("/Users/dev")
        }
    }

    #[test]
    fn cargo_home_is_the_variable_else_dot_cargo_in_the_home_folder() {
        let cwd = Path::new("/code/app");
        assert_eq!(
            with_cargo_home(None).cargo_home_at(cwd),
            Path::new("/Users/dev/.cargo")
        );
        assert_eq!(
            with_cargo_home(Some("")).cargo_home_at(cwd),
            Path::new("/Users/dev/.cargo"),
            "an empty variable counts as unset"
        );
        assert_eq!(
            with_cargo_home(Some("/opt/cargo/./home")).cargo_home_at(cwd),
            Path::new("/opt/cargo/home")
        );
        assert_eq!(
            with_cargo_home(Some("../cache")).cargo_home_at(cwd),
            Path::new("/code/cache"),
            "a relative one starts where cargo runs"
        );
    }
}
