//! Which folders Sutra may read, and how it got them.
//!
//! The direct edition reads the real home folder and Claude Code's scratch root with no questions
//! asked. The Mac App Store edition runs in the App Sandbox, where it can read only what the user
//! picked in the window's open panel; that choice is kept as a security-scoped bookmark so the
//! question is asked once.

use std::path::PathBuf;

use super::{docker, scan};

#[cfg(target_os = "macos")]
pub use macos::GrantError;

/// Why a folder grant could not be restored or saved. Only the sandboxed macOS edition has
/// grants, so elsewhere there is no such error.
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub enum GrantError {}

#[cfg(not(target_os = "macos"))]
impl std::fmt::Display for GrantError {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

/// The folders to scan and watch.
pub struct Roots {
    /// The real home directory (see `crate::home::real_home`).
    pub home: PathBuf,
    /// The folders walked for worktrees and project folders.
    pub dirs: Vec<PathBuf>,
    /// Claude Code's scratch root, read for its sessions' scratch folders (see `scan::scratch`).
    /// The store edition has none: the sandbox keeps it out of `/private/tmp`, and reading it
    /// there would take a grant of its own, kept as a second bookmark beside the one of `dirs`.
    pub scratch: Option<PathBuf>,
    /// The sockets Docker's engine may be served on, in the order tried (see `docker`). The store
    /// edition has none: the sandbox keeps it out of them.
    pub docker: Vec<PathBuf>,
    /// Running inside the App Sandbox.
    pub sandboxed: bool,
}

/// Decide the roots for this process: the real home folder, Claude Code's scratch root and Docker's
/// sockets outside the sandbox; inside it, the folder granted on an earlier launch. Inside the
/// sandbox with no folder granted yet, `dirs` is empty and the window asks, so the user reads why
/// before any panel opens; the folder chosen is kept with `save_grant`. A saved grant that could
/// not be used is returned beside the roots, for the window to explain.
pub fn resolve_roots() -> (Roots, Option<GrantError>) {
    let home = crate::home::real_home()
        .expect("the passwd database or $HOME names the user's home folder");
    #[cfg(target_os = "macos")]
    if macos::is_sandboxed() {
        let (dirs, lost) = match macos::restore_grant() {
            Ok(granted) => (granted.into_iter().collect(), None),
            Err(err) => (Vec::new(), Some(err)),
        };
        let roots = Roots {
            home,
            dirs,
            scratch: None,
            docker: Vec::new(),
            sandboxed: true,
        };
        return (roots, lost);
    }
    let docker_host = std::env::var("DOCKER_HOST").ok();
    let roots = Roots {
        dirs: vec![home.clone()],
        docker: docker::socket_candidates(&home, docker_host.as_deref()),
        home,
        scratch: Some(scan::scratch_root()),
        sandboxed: false,
    };
    (roots, None)
}

/// Save the grant of `folder`, which the user just chose in the system open panel, for the next
/// launch. The panel itself grants access until the process exits, so this run needs nothing more:
/// an error means only that the next launch asks again. Off macOS there is no sandbox, and no grant
/// to keep.
pub fn save_grant(folder: &str) -> Result<(), GrantError> {
    #[cfg(target_os = "macos")]
    return macos::save_grant(folder);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = folder;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::fmt;
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    use crate::home::{app_data_file, write_atomically};
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::Bool;
    use objc2_foundation::{
        NSData, NSError, NSString, NSURL, NSURLBookmarkCreationOptions,
        NSURLBookmarkResolutionOptions,
    };

    /// Why the saved grant could not be used or a new one could not be saved.
    #[derive(Debug)]
    pub enum GrantError {
        ReadBookmark(io::Error),
        ResolveBookmark(Retained<NSError>),
        /// The bookmark resolved but the sandbox refused to extend access to its folder.
        AccessRefused,
        CreateBookmark(Retained<NSError>),
        WriteBookmark(io::Error),
    }

    impl fmt::Display for GrantError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::ReadBookmark(err) => write!(f, "reading the saved bookmark failed: {err}"),
                Self::ResolveBookmark(err) => {
                    write!(f, "the saved bookmark did not resolve: {err}")
                }
                Self::AccessRefused => write!(f, "the sandbox refused access to the saved folder"),
                Self::CreateBookmark(err) => write!(f, "creating the bookmark failed: {err}"),
                Self::WriteBookmark(err) => write!(f, "writing the bookmark failed: {err}"),
            }
        }
    }

    /// The App Sandbox puts this variable in the environment of every process it confines.
    pub(super) fn is_sandboxed() -> bool {
        std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
    }

    /// Runs on whichever thread asks: `NSURL` and its bookmarks need no main thread, and the pool
    /// releases what Foundation hands back on a thread that has no pool of its own.
    pub(super) fn save_grant(folder: &str) -> Result<(), GrantError> {
        autoreleasepool(|_| {
            // The open panel itself granted access to the folder until the process exits, so this
            // URL needs no `startAccessingSecurityScopedResource`; only a URL resolved from a
            // bookmark on a later launch does.
            let url =
                unsafe { NSURL::fileURLWithPath_isDirectory(&NSString::from_str(folder), true) };
            save_bookmark(&url)
        })
    }

    /// Where the grant is kept between launches.
    fn bookmark_path() -> PathBuf {
        app_data_file("root.bookmark").expect("macOS always has an Application Support folder")
    }

    /// The folder granted on an earlier launch, with access to it started for this process.
    /// `None` when no launch has saved a grant yet: the first launch, or every panel so far was
    /// cancelled.
    pub(super) fn restore_grant() -> Result<Option<PathBuf>, GrantError> {
        let bytes = match fs::read(bookmark_path()) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(GrantError::ReadBookmark(err)),
        };
        let mut stale = Bool::NO;
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &NSData::with_bytes(&bytes),
                NSURLBookmarkResolutionOptions::NSURLBookmarkResolutionWithSecurityScope,
                None,
                &mut stale,
            )
        }
        .map_err(GrantError::ResolveBookmark)?;
        if !unsafe { url.startAccessingSecurityScopedResource() } {
            return Err(GrantError::AccessRefused);
        }
        // A stale bookmark still resolves (the folder was moved or renamed); Apple's guidance is to
        // replace the saved copy with a fresh bookmark of the resolved URL rather than ask again.
        if stale.as_bool() {
            if let Err(err) = save_bookmark(&url) {
                eprintln!("sutra: the moved folder's bookmark was not refreshed, because {err}");
            }
        }
        let dir = path_of(&url);
        // The scanner, the file watcher and every measurement read inside the grant until the
        // process exits, the span the open panel's own grant has too, so access is never stopped.
        // It is started once a launch, so forgetting the URL that must stop it costs nothing.
        std::mem::forget(url);
        Ok(Some(dir))
    }

    /// Keep `url`'s grant as a security-scoped bookmark for the next launch.
    fn save_bookmark(url: &NSURL) -> Result<(), GrantError> {
        // Without `SecurityScopeAllowOnlyReadAccess` the scope is read-write, which deleting build
        // folders inside the grant needs.
        let bookmark = unsafe {
            url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::NSURLBookmarkCreationWithSecurityScope,
                None,
                None,
            )
        }
        .map_err(GrantError::CreateBookmark)?;
        write_atomically(&bookmark_path(), bookmark.bytes()).map_err(GrantError::WriteBookmark)
    }

    fn path_of(url: &NSURL) -> PathBuf {
        let path = unsafe { url.path() }.expect("a file URL always has a path");
        PathBuf::from(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_roots_outside_the_sandbox_watches_the_real_home_folder_claudes_scratch_and_docker() {
        // `cargo test` runs unsandboxed, so this exercises the direct edition's path.
        let (roots, lost) = resolve_roots();
        let home = crate::home::real_home().unwrap();
        assert!(!roots.sandboxed);
        assert_eq!(roots.home, home);
        assert_eq!(roots.dirs, std::slice::from_ref(&home));
        assert_eq!(roots.scratch, Some(scan::scratch_root()));
        let usual = [
            home.join(".docker/run/docker.sock"),
            "/var/run/docker.sock".into(),
        ];
        assert!(
            roots.docker.ends_with(&usual),
            "after the one $DOCKER_HOST may name: {:?}",
            roots.docker
        );
        assert!(lost.is_none());
    }
}
