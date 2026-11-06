//! Where Sutra may go: which folders the walk skips, which paths any source may touch at all, and
//! whether a project folder can be read.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::map::{FolderAccess, KeptOutBy, PrivateFolder, ScanOptions};

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

/// What Sutra may enter and read, from the roots, the home folder, the scan options and what macOS
/// has answered about the private folders. Discovery walks inside it, and every source that
/// resolves a path someone else recorded (an agent's working directory, an environment's `DIR`)
/// asks it first.
pub struct Boundary {
    /// Canonical roots, in the order given.
    roots: Vec<PathBuf>,
    /// For each parent folder, the names of its children the walk skips.
    skipped_children: HashMap<PathBuf, HashSet<OsString>>,
    /// The private folders Sutra stays out of.
    kept_out: Vec<KeptOutFolder>,
    /// Canonical.
    home: PathBuf,
}

/// A private folder Sutra stays out of.
struct KeptOutFolder {
    folder: PrivateFolder,
    by: KeptOutBy,
    path: PathBuf,
}

/// What macOS answered the first time Sutra read inside a private folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacOsAnswer {
    /// Sutra may read the folder, or there is no folder to guard.
    LetIn,
    /// The user said no to macOS's prompt, now or on an earlier launch.
    Refused,
}

impl Boundary {
    /// Everything under `roots` but the system folders, the tool caches and the private folders
    /// `opts` keeps out. It enters the private folders `opts` lets in as though macOS had let
    /// Sutra into them already; `keep_out_until_macos_answers` keeps them out until it has.
    pub fn new(roots: &[PathBuf], home: &Path, opts: &ScanOptions) -> Self {
        // A root that cannot be resolved is kept as given; the walk then counts it as unreadable.
        let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut boundary = Boundary {
            roots: roots.iter().map(|root| canonical(root)).collect(),
            skipped_children: HashMap::new(),
            kept_out: Vec::new(),
            home: canonical(home),
        };
        for rel in SYSTEM_FOLDERS.iter().chain(&TOOL_CACHES) {
            let skipped = boundary.home.join(rel);
            boundary.skip(&skipped);
        }
        for folder in PrivateFolder::ALL {
            if !opts.private_folders.contains(&folder) {
                boundary.keep_out(folder, KeptOutBy::Settings);
            }
        }
        boundary
    }

    /// Keep out each private folder this lets in that lies inside a root, until macOS answers
    /// whether Sutra may read it (`macos_answered`). The walk would enter it first thing, and the
    /// first read inside it raises macOS's privacy prompt and waits for the user's answer. A root
    /// that is itself a private folder lets in what it holds whatever this says (see
    /// `kept_out_by`). A private folder outside every root is never walked, and only the store
    /// edition has one, where the sandbox keeps Sutra out of what lies outside the grant.
    pub fn keep_out_until_macos_answers(&mut self) {
        let lets_in =
            |folder: &PrivateFolder| !self.kept_out.iter().any(|kept| kept.folder == *folder);
        let in_a_root = |folder: &PrivateFolder| {
            let path = self.home.join(folder.name());
            self.roots
                .iter()
                .any(|root| path.starts_with(root) && path != *root)
        };
        let asked: Vec<PrivateFolder> = PrivateFolder::ALL
            .into_iter()
            .filter(|folder| lets_in(folder) && in_a_root(folder))
            .collect();
        for folder in asked {
            self.keep_out(folder, KeptOutBy::Prompt);
        }
    }

    /// The private folders waiting for macOS to answer whether Sutra may read them, each with its
    /// path.
    pub fn awaiting_macos(&self) -> Vec<(PrivateFolder, PathBuf)> {
        self.kept_out
            .iter()
            .filter(|kept| kept.by == KeptOutBy::Prompt)
            .map(|kept| (kept.folder, kept.path.clone()))
            .collect()
    }

    /// macOS answered whether Sutra may read `folder`, which waited for its answer: let it in, or
    /// keep it out for the rest of the run. An answer about a folder not waiting changes nothing.
    pub fn macos_answered(&mut self, folder: PrivateFolder, answer: MacOsAnswer) {
        let waiting = self
            .kept_out
            .iter()
            .position(|kept| kept.folder == folder && kept.by == KeptOutBy::Prompt);
        let Some(waiting) = waiting else {
            return;
        };
        match answer {
            MacOsAnswer::LetIn => {
                let let_in = self.kept_out.remove(waiting);
                self.enter(&let_in.path);
            }
            MacOsAnswer::Refused => self.kept_out[waiting].by = KeptOutBy::MacOs,
        }
    }

    /// Stay out of `folder`, in the home folder, kept out `by` Settings or macOS.
    fn keep_out(&mut self, folder: PrivateFolder, by: KeptOutBy) {
        let path = self.home.join(folder.name());
        self.skip(&path);
        self.kept_out.push(KeptOutFolder { folder, by, path });
    }

    /// Have the walk skip `path`.
    fn skip(&mut self, path: &Path) {
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            self.skipped_children
                .entry(parent.to_path_buf())
                .or_default()
                .insert(name.to_os_string());
        }
    }

    /// Have the walk enter `path`, which it skipped.
    fn enter(&mut self, path: &Path) {
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            if let Some(names) = self.skipped_children.get_mut(parent) {
                names.remove(name);
            }
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

    /// The private folder Sutra stays out of that holds `path`, if one does, and what keeps Sutra
    /// out. Touching such a path, even to `stat` or resolve it, raises the privacy prompt, or waits
    /// for the user to answer it. A root that lies inside the folder lets in what it holds: the
    /// user chose that root.
    pub fn kept_out_by(&self, path: &Path) -> Option<(PrivateFolder, KeptOutBy)> {
        self.kept_out
            .iter()
            .find(|kept| {
                path.starts_with(&kept.path)
                    && !self
                        .roots
                        .iter()
                        .any(|root| root.starts_with(&kept.path) && path.starts_with(root))
            })
            .map(|kept| (kept.folder, kept.by))
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
            .filter(|kept| kept.by == KeptOutBy::Settings)
            .map(|kept| &kept.path)
            .filter(|folder| self.roots.iter().any(|root| folder.starts_with(root)))
            .cloned()
            .collect()
    }

    /// The private folders macOS keeps Sutra out of, because the user said no to its prompt.
    pub fn refused_private_folders(&self) -> Vec<PrivateFolder> {
        self.kept_out
            .iter()
            .filter(|kept| kept.by == KeptOutBy::MacOs)
            .map(|kept| kept.folder)
            .collect()
    }
}

/// Whether Sutra reads `folder`: one it may not touch is never probed, and any other is listed.
pub fn folder_access(folder: &Path, boundary: &Boundary) -> FolderAccess {
    match boundary.kept_out_by(folder) {
        Some((private, by)) => FolderAccess::KeptOut(private, by),
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
            rooted_at(&home, documents.clone()),
            [home.join("Desktop"), home.join("Downloads")]
        );
        let mut asking = Boundary::new(std::slice::from_ref(&home), &home, &documents);
        asking.keep_out_until_macos_answers();
        assert_eq!(
            asking.skipped_private_folders(),
            [home.join("Desktop"), home.join("Downloads")],
            "Documents waits for macOS, which is not Settings' doing"
        );
    }

    #[test]
    fn each_private_folder_is_kept_out_on_its_own_and_a_root_inside_one_lets_in_what_it_holds() {
        let home = PathBuf::from("/Users/me");
        let documents = ScanOptions {
            private_folders: [PrivateFolder::Documents].into(),
        };
        let boundary = Boundary::new(std::slice::from_ref(&home), &home, &documents);
        let switched_off = |folder| Some((folder, KeptOutBy::Settings));
        assert_eq!(boundary.kept_out_by(&home.join("Documents/app")), None);
        assert_eq!(
            boundary.kept_out_by(&home.join("Desktop/app")),
            switched_off(PrivateFolder::Desktop)
        );
        assert_eq!(
            boundary.kept_out_by(&home.join("Downloads")),
            switched_off(PrivateFolder::Downloads)
        );
        assert_eq!(boundary.kept_out_by(&home.join("DesktopTools")), None);

        let chosen = home.join("Desktop/code");
        let inside = Boundary::new(std::slice::from_ref(&chosen), &home, &documents);
        assert_eq!(inside.kept_out_by(&chosen.join("app")), None);
        assert_eq!(
            inside.kept_out_by(&home.join("Desktop/other")),
            switched_off(PrivateFolder::Desktop)
        );
    }

    #[test]
    fn a_private_folder_settings_let_in_stays_out_until_macos_answers_and_then_for_good_on_a_no() {
        let home = PathBuf::from("/Users/me");
        let all = ScanOptions {
            private_folders: PrivateFolder::ALL.into(),
        };
        let mut boundary = Boundary::new(std::slice::from_ref(&home), &home, &all);
        boundary.keep_out_until_macos_answers();
        let waiting: Vec<PrivateFolder> = boundary
            .awaiting_macos()
            .into_iter()
            .map(|(folder, path)| {
                assert_eq!(path, home.join(folder.name()));
                folder
            })
            .collect();
        assert_eq!(waiting, PrivateFolder::ALL);
        let documents = home.join("Documents");
        assert_eq!(
            boundary.kept_out_by(&documents.join("app")),
            Some((PrivateFolder::Documents, KeptOutBy::Prompt))
        );
        assert!(boundary.skipped_within(&home).contains(&documents));

        boundary.macos_answered(PrivateFolder::Documents, MacOsAnswer::LetIn);
        assert_eq!(boundary.kept_out_by(&documents.join("app")), None);
        assert!(
            !boundary.skipped_within(&home).contains(&documents),
            "the walk enters it"
        );

        boundary.macos_answered(PrivateFolder::Desktop, MacOsAnswer::Refused);
        let refused = Some((PrivateFolder::Desktop, KeptOutBy::MacOs));
        assert_eq!(boundary.kept_out_by(&home.join("Desktop/app")), refused);
        assert!(
            boundary
                .skipped_within(&home)
                .contains(&home.join("Desktop"))
        );
        assert_eq!(boundary.refused_private_folders(), [PrivateFolder::Desktop]);
        assert_eq!(
            boundary.awaiting_macos(),
            [(PrivateFolder::Downloads, home.join("Downloads"))]
        );

        boundary.macos_answered(PrivateFolder::Desktop, MacOsAnswer::LetIn);
        assert_eq!(
            boundary.kept_out_by(&home.join("Desktop/app")),
            refused,
            "macOS's answer stands for the rest of the run"
        );
    }

    #[test]
    fn macos_is_asked_only_about_the_private_folders_settings_let_in_inside_a_root() {
        let home = PathBuf::from("/Users/me");
        let documents = ScanOptions {
            private_folders: [PrivateFolder::Documents].into(),
        };
        let asked_with_root = |root: PathBuf| {
            let mut boundary = Boundary::new(&[root], &home, &documents);
            boundary.keep_out_until_macos_answers();
            boundary.awaiting_macos()
        };
        assert_eq!(
            asked_with_root(home.clone()),
            [(PrivateFolder::Documents, home.join("Documents"))],
            "Desktop and Downloads are switched off"
        );
        assert_eq!(asked_with_root(home.join("code")), [], "never walked");
        assert_eq!(
            asked_with_root(home.join("Documents")),
            [],
            "a root lets in what it holds"
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
            FolderAccess::KeptOut(PrivateFolder::Documents, KeptOutBy::Settings)
        );
    }
}
