//! Where the user's home folder is, for everything that reads the machine's state from it: the
//! dev-runner registry, the worktree map and the folder grant. And where Sutra keeps its own files
//! between launches.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The user's real home directory. Inside the App Sandbox `$HOME` (and `dirs::home_dir()`) point at
/// the container, so this asks the passwd database instead.
pub fn real_home() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use nix::unistd::{User, getuid};
        if let Ok(Some(user)) = User::from_uid(getuid()) {
            return Some(user.dir);
        }
    }
    dirs::home_dir()
}

/// Where Sutra keeps the file `name` between launches, when the system says where such files go.
/// Inside the sandbox this lies in the app's container, which the app may write without any grant.
pub fn app_data_file(name: &str) -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Sutra").join(name))
}

/// Replace `path` so a reader sees the old bytes or the new ones, never a torn file.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("partial");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn write_atomically_creates_missing_parents_and_replaces_the_old_file() {
        let fixture = TempDir::new("home-write-atomically");
        let path = fixture.path("Support/Sutra/kept");

        write_atomically(&path, b"first").unwrap();
        write_atomically(&path, b"second").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert!(!path.with_extension("partial").exists());
    }
}
