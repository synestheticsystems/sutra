//! Where Sutra may go: which folders the walk skips, which paths any source may touch at all, and
//! whether a project folder can be read.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::map::{FolderAccess, PrivateFolder, ScanOptions};

/// Build output: thousands of directories each, and never a project of its own.
const BUILD_OUTPUT_NAMES: [&str; 2] = ["target", "node_modules"];

/// Home folders that hold no code: system state, the trash and media libraries.
const SYSTEM_FOLDERS: [&str; 6] = [
    "Library",
    ".Trash",
    "Pictures",
    "Music",
    "Movies",
    "Applications",
];

/// Tool caches, toolchains and VM images under the home folder. On Daniel's M4 Max (2026-10-31)
/// they held about 120k of the 146k directories a home walk visits, and their only repositories
/// are dependency clones.
const TOOL_CACHES: [&str; 32] = [
    ".cargo",
    ".rustup",
    ".npm",
    ".bun",
    ".nvm",
    ".pnpm",
    ".yarn",
    ".deno",
    "go/pkg",
    ".gem",
    ".m2",
    ".gradle",
    ".cocoapods",
    ".swiftpm",
    ".android",
    ".expo",
    ".homebrew",
    ".pyenv",
    ".conda",
    ".cache",
    ".local/share",
    ".docker",
    ".lmstudio",
    ".ollama",
    "Parallels",
    "VirtualBox VMs",
    ".vscode",
    ".cursor",
    "google-cloud-sdk",
    ".ipython",
    ".jupyter",
    ".matplotlib",
];

/// What Sutra may enter and read, from the roots, the home folder and the scan options. Discovery
/// walks inside it, and every source that resolves a path someone else recorded (an agent's working
/// directory, an environment's `DIR`) asks it first.
pub struct Boundary {
    /// Canonical roots, in the order given.
    roots: Vec<PathBuf>,
    /// For each parent folder, the names of its children the walk skips.
    skipped_children: HashMap<PathBuf, HashSet<OsString>>,
    /// The private folders Settings keeps Sutra out of, each with its path.
    kept_out: Vec<(PrivateFolder, PathBuf)>,
    /// Canonical.
    home: PathBuf,
}

impl Boundary {
    pub fn new(roots: &[PathBuf], home: &Path, opts: &ScanOptions) -> Self {
        // A root that cannot be resolved is kept as given; the walk then counts it as unreadable.
        let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let home = canonical(home);
        let kept_out: Vec<(PrivateFolder, PathBuf)> = PrivateFolder::ALL
            .into_iter()
            .filter(|folder| !opts.private_folders.contains(folder))
            .map(|folder| (folder, home.join(folder.name())))
            .collect();
        let mut skipped_children: HashMap<PathBuf, HashSet<OsString>> = HashMap::new();
        let skipped = SYSTEM_FOLDERS
            .iter()
            .chain(&TOOL_CACHES)
            .map(|rel| home.join(rel))
            .chain(kept_out.iter().map(|(_, path)| path.clone()));
        for path in skipped {
            if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
                skipped_children
                    .entry(parent.to_path_buf())
                    .or_default()
                    .insert(name.to_os_string());
            }
        }
        Boundary {
            roots: roots.iter().map(|root| canonical(root)).collect(),
            skipped_children,
            kept_out,
            home,
        }
    }

    pub(super) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(super) fn is_home(&self, dir: &Path) -> bool {
        dir == self.home
    }

    /// The children of `dir` the walk must not enter, looked up once per directory so the common
    /// case (a folder with no skipped children) costs one hash lookup, not one per child.
    pub(super) fn skipped_children_of(&self, dir: &Path) -> Option<&HashSet<OsString>> {
        self.skipped_children.get(dir)
    }

    /// Build output is skipped wherever it appears.
    pub(super) fn is_build_output(name: &OsStr) -> bool {
        BUILD_OUTPUT_NAMES.iter().any(|skipped| name == *skipped)
    }

    /// Whether Sutra may touch `path` at all, even to `stat` or resolve it (see `kept_out_by`).
    pub fn may_read(&self, path: &Path) -> bool {
        self.kept_out_by(path).is_none()
    }

    /// The private folder Settings keeps Sutra out of that holds `path`, if one does. Touching such
    /// a path, even to `stat` or resolve it, raises the privacy prompt. A root that lies inside the
    /// folder lets in what it holds: the user chose that root.
    pub fn kept_out_by(&self, path: &Path) -> Option<PrivateFolder> {
        self.kept_out
            .iter()
            .find(|(_, folder)| {
                path.starts_with(folder)
                    && !self
                        .roots
                        .iter()
                        .any(|root| root.starts_with(folder) && path.starts_with(root))
            })
            .map(|(private, _)| *private)
    }

    /// The folders strictly inside `dir` that the walk skips. Sizing a folder skips them too, so a
    /// home folder kept in git is never sized through its `Library` or its private folders.
    pub fn skipped_within(&self, dir: &Path) -> Vec<PathBuf> {
        self.skipped_children
            .iter()
            .filter(|(parent, _)| parent.starts_with(dir))
            .flat_map(|(parent, names)| names.iter().map(|name| parent.join(name)))
            .collect()
    }

    /// The private folders Settings keeps Sutra out of that lie inside a root, so the walk skips
    /// them.
    pub fn skipped_private_folders(&self) -> Vec<PathBuf> {
        self.kept_out
            .iter()
            .map(|(_, folder)| folder)
            .filter(|folder| self.roots.iter().any(|root| folder.starts_with(root)))
            .cloned()
            .collect()
    }
}

/// Whether Sutra reads `folder`: one it may not touch is never probed, and any other is listed.
pub fn folder_access(folder: &Path, boundary: &Boundary) -> FolderAccess {
    match boundary.kept_out_by(folder) {
        Some(private) => FolderAccess::KeptOut(private),
        None => listing_access(folder),
    }
}

/// Whether `folder` can be listed. Listing is the test, rather than `stat`, because inside the
/// sandbox a folder outside the grant can be stat'ed but not listed.
pub(super) fn listing_access(folder: &Path) -> FolderAccess {
    match fs::read_dir(folder) {
        Ok(_) => FolderAccess::Readable,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            FolderAccess::Missing
        }
        Err(_) => FolderAccess::Denied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::test_support::TempDir;

    #[test]
    fn skipped_within_names_the_skipped_folders_inside_a_folder_and_none_elsewhere() {
        let home = PathBuf::from("/Users/me");
        let boundary = Boundary::new(std::slice::from_ref(&home), &home, &ScanOptions::default());
        let in_home = boundary.skipped_within(&home);
        for skipped in ["Library", ".cargo", "go/pkg", "Documents"] {
            assert!(in_home.contains(&home.join(skipped)), "{skipped}");
        }
        assert_eq!(
            boundary.skipped_within(&home.join("go")),
            [home.join("go/pkg")]
        );
        assert!(boundary.skipped_within(&home.join("code/app")).is_empty());
    }

    #[test]
    fn skipped_private_folders_are_those_inside_a_root_that_settings_keep_out() {
        let home = PathBuf::from("/Users/me");
        let rooted_at = |root: &Path, opts| {
            Boundary::new(&[root.to_path_buf()], &home, &opts).skipped_private_folders()
        };
        let all_three: Vec<PathBuf> = ["Desktop", "Documents", "Downloads"]
            .iter()
            .map(|name| home.join(name))
            .collect();
        assert_eq!(rooted_at(&home, ScanOptions::default()), all_three);
        assert!(rooted_at(&home.join("code"), ScanOptions::default()).is_empty());
        let documents = ScanOptions {
            private_folders: [PrivateFolder::Documents].into(),
        };
        assert_eq!(
            rooted_at(&home, documents),
            [home.join("Desktop"), home.join("Downloads")]
        );
    }

    #[test]
    fn each_private_folder_is_kept_out_on_its_own_and_a_root_inside_one_lets_in_what_it_holds() {
        let home = PathBuf::from("/Users/me");
        let documents = ScanOptions {
            private_folders: [PrivateFolder::Documents].into(),
        };
        let boundary = Boundary::new(std::slice::from_ref(&home), &home, &documents);
        assert_eq!(boundary.kept_out_by(&home.join("Documents/app")), None);
        assert_eq!(
            boundary.kept_out_by(&home.join("Desktop/app")),
            Some(PrivateFolder::Desktop)
        );
        assert_eq!(
            boundary.kept_out_by(&home.join("Downloads")),
            Some(PrivateFolder::Downloads)
        );
        assert_eq!(boundary.kept_out_by(&home.join("DesktopTools")), None);

        let chosen = home.join("Desktop/code");
        let inside = Boundary::new(std::slice::from_ref(&chosen), &home, &documents);
        assert_eq!(inside.kept_out_by(&chosen.join("app")), None);
        assert_eq!(
            inside.kept_out_by(&home.join("Desktop/other")),
            Some(PrivateFolder::Desktop)
        );
    }

    #[test]
    fn folder_access_tells_a_missing_folder_from_one_that_cannot_be_listed() {
        let fixture = TempDir::new("boundary-access");
        let boundary = fixture.boundary(&ScanOptions::default());
        let open = fixture.dir("open");
        let sealed = fixture.dir("sealed");
        fixture.restrict(&sealed, 0o000);
        let file = fixture.write("file", "");

        assert_eq!(folder_access(&open, &boundary), FolderAccess::Readable);
        assert_eq!(folder_access(&sealed, &boundary), FolderAccess::Denied);
        assert_eq!(
            folder_access(&fixture.path("gone"), &boundary),
            FolderAccess::Missing
        );
        assert_eq!(folder_access(&file, &boundary), FolderAccess::Missing);
        let documents = fixture.dir("Documents/app");
        assert_eq!(
            folder_access(&documents, &boundary),
            FolderAccess::KeptOut(PrivateFolder::Documents)
        );
    }
}
