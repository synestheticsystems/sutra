//! A temporary folder for one test, the cargo that runs the tests started in it, and what both
//! cargo and Sutra are told about it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use sutra::map::clean::{self, CargoEnv, CheckedPlan, CleanPlan, ItsProject, Plan, Reach};

/// Cargo's tag, as it writes it into a build folder it creates.
pub const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
    # This file is a cache directory tag created by cargo.\n\
    # For information about cache directory tags see https://bford.info/cachedir/\n";

/// A fresh folder under the system temp dir for one test, removed when dropped. Its
/// `cargo-home` is the `CARGO_HOME` of every cargo it starts.
pub struct Fixture {
    root: PathBuf,
    /// Environment variables given to cargo and to Sutra alike.
    vars: Vec<(&'static str, String)>,
}

impl Fixture {
    /// `test` names the folder and must be unique among the tests, which run in parallel.
    pub fn new(test: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("sutra-cargo-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("cargo-home")).expect("create fixture");
        Fixture {
            root: fs::canonicalize(&root).expect("canonicalize fixture"),
            vars: Vec::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn write(&self, rel: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let file = self.path(rel);
        fs::create_dir_all(file.parent().expect("a parent")).expect("create parent");
        fs::write(&file, contents).expect("write fixture file");
        file
    }

    /// A binary package in `rel`, its manifest ending in `extra`.
    pub fn package(&self, rel: &str, extra: &str) -> PathBuf {
        let name = rel
            .rsplit('/')
            .next()
            .expect("a name")
            .replace(['-', '.'], "_");
        self.write(
            &format!("{rel}/Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{extra}"
            ),
        );
        self.write(&format!("{rel}/src/main.rs"), "fn main() {}\n");
        self.path(rel)
    }

    /// Set `var` for every cargo this fixture starts from now on, and for Sutra.
    pub fn set_var(&mut self, var: &'static str, value: &str) {
        self.vars.push((var, value.to_string()));
    }

    /// The cargo running these tests, in `dir`, with nothing in its environment but what cargo
    /// needs to find its toolchain, this fixture's `CARGO_HOME` and the variables set.
    pub fn cargo(&self, dir: &Path) -> Command {
        let mut cargo = Command::new(std::env::var_os("CARGO").expect("cargo test sets CARGO"));
        cargo.current_dir(dir).env_clear();
        for var in ["PATH", "HOME", "RUSTUP_TOOLCHAIN"] {
            if let Some(value) = std::env::var_os(var) {
                cargo.env(var, value);
            }
        }
        cargo.env("CARGO_HOME", self.path("cargo-home"));
        for (var, value) in &self.vars {
            cargo.env(var, value);
        }
        cargo
    }

    /// Run cargo in `dir` with `args`, which must succeed.
    pub fn run(&self, dir: &Path, args: &[&str]) -> Output {
        let output = self.cargo(dir).args(args).output().expect("start cargo");
        assert!(
            output.status.success(),
            "cargo {args:?} in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    /// What Sutra is told of the environment: this fixture's `CARGO_HOME` and the variables set.
    pub fn env(&self) -> CargoEnv {
        let var = |name: &str| {
            self.vars
                .iter()
                .find(|(var, _)| *var == name)
                .map(|(_, value)| OsString::from(value))
        };
        CargoEnv {
            home: PathBuf::from(std::env::var_os("HOME").expect("HOME is set")),
            cargo_home: Some(self.path("cargo-home").into()),
            target_dir: var("CARGO_TARGET_DIR"),
            build_target_dir: var("CARGO_BUILD_TARGET_DIR"),
            build_build_dir: var("CARGO_BUILD_BUILD_DIR"),
        }
    }

    /// Sutra's plan for cleaning `clicked`, anywhere Sutra may delete, its folders checked.
    pub fn plan(&self, clicked: &Path) -> Result<CheckedPlan, clean::PlanError> {
        clean::plan(
            clicked,
            ItsProject::At(&self.root),
            &self.env(),
            &Reach::Anywhere,
        )
    }

    /// What `cargo clean` deletes for the workspace Sutra planned `clicked` in, whether the
    /// clicked folder is part of it or not.
    pub fn cargo_clean_of(&self, clicked: &Path) -> CleanPlan {
        match self.plan(clicked).expect("a plan").plan {
            Plan::CargoClean(clean) => clean,
            Plan::DeleteFolder {
                cargo_clean: Some(clean),
                ..
            } => clean,
            other => panic!("no workspace for {}: {other:?}", clicked.display()),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// What is in a folder, for comparing two: each path below it, relative, with a file's bytes or
/// a link's target.
pub fn tree(root: &Path) -> BTreeMap<PathBuf, Entry> {
    let mut entries = BTreeMap::new();
    collect(root, root, &mut entries);
    entries
}

#[derive(Debug, PartialEq, Eq)]
pub enum Entry {
    Folder,
    File(Vec<u8>),
    Link(PathBuf),
}

fn collect(root: &Path, dir: &Path, entries: &mut BTreeMap<PathBuf, Entry>) {
    for entry in fs::read_dir(dir).expect("list fixture folder") {
        let path = entry.expect("fixture entry").path();
        let rel = path
            .strip_prefix(root)
            .expect("below the root")
            .to_path_buf();
        let kind = fs::symlink_metadata(&path)
            .expect("stat fixture entry")
            .file_type();
        if kind.is_symlink() {
            entries.insert(rel, Entry::Link(fs::read_link(&path).expect("read link")));
        } else if kind.is_dir() {
            entries.insert(rel, Entry::Folder);
            collect(root, &path, entries);
        } else {
            entries.insert(
                rel,
                Entry::File(fs::read(&path).expect("read fixture file")),
            );
        }
    }
}

/// Copy the folder `from` to `to`, links as links.
pub fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create copy");
    for entry in fs::read_dir(from).expect("list folder to copy") {
        let path = entry.expect("entry to copy").path();
        let target = to.join(path.file_name().expect("a name"));
        let kind = fs::symlink_metadata(&path)
            .expect("stat entry to copy")
            .file_type();
        if kind.is_symlink() {
            let link = fs::read_link(&path).expect("read link to copy");
            std::os::unix::fs::symlink(link, &target).expect("copy link");
        } else if kind.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).expect("copy file");
        }
    }
}

/// Wait until `done` holds, checking every few milliseconds, for at most a minute.
pub fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "{what} did not happen within a minute"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
