//! Cargo's own builds and Sutra's clean keep out of each other's way: a running build or check
//! stops a clean, wherever in the folder it builds, a build that starts during a clean waits for
//! it and then builds on locks a later build respects, and a build that starts in a new profile
//! folder before the delete stops it. The builds hold still in a build script that waits for a
//! signal file.

use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use sutra::map::clean::{self, CleanError, Plan, Progress};

use super::fixture::{Fixture, wait_until};

/// A build script that writes `SUTRA_TEST_STARTED` and then waits for `SUTRA_TEST_RELEASE` to
/// exist, when they are set. It runs again whenever either changes. It waits five minutes at most,
/// far longer than any test holds it, because a test run killed outright drops no `Held`, and its
/// build would otherwise poll forever.
const HOLDING_BUILD_SCRIPT: &str = r#"
fn main() {
    println!("cargo::rerun-if-env-changed=SUTRA_TEST_STARTED");
    println!("cargo::rerun-if-env-changed=SUTRA_TEST_RELEASE");
    if let Some(started) = std::env::var_os("SUTRA_TEST_STARTED") {
        std::fs::write(started, "").unwrap();
    }
    if let Some(release) = std::env::var_os("SUTRA_TEST_RELEASE") {
        let holding = std::time::Instant::now();
        while !std::path::Path::new(&release).exists() && holding.elapsed().as_secs() < 300 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
"#;

/// A package with the holding build script, built once, and the plan to clean its `target`.
fn built_package(fixture: &Fixture) -> Plan {
    let app = fixture.package("app", "");
    fixture.write("app/build.rs", HOLDING_BUILD_SCRIPT);
    fixture.run(&app, &["build"]);
    fixture.plan(&app.join("target")).expect("a plan")
}

/// How long a dropped `Held` gives its build to finish once released, before killing it.
const FINISH_WITHIN: Duration = Duration::from_secs(5);

/// A cargo `command` started in the package, held in its build script until released, its
/// output in `<name>.log`. A test that panics while holding one still ends the build: see its
/// `Drop`.
struct Held {
    child: Child,
    started: PathBuf,
    release: PathBuf,
    log: PathBuf,
}

impl Held {
    fn start(fixture: &Fixture, name: &str, command: &[&str]) -> Held {
        let started = fixture.path(&format!("{name}.started"));
        let release = fixture.path(&format!("{name}.release"));
        let log = fixture.path(&format!("{name}.log"));
        let child = fixture
            .cargo(&fixture.path("app"))
            .args(command)
            .env("SUTRA_TEST_STARTED", &started)
            .env("SUTRA_TEST_RELEASE", &release)
            .stderr(File::create(&log).unwrap())
            // A process group of its own, so that dropping this reaches all that cargo started.
            .process_group(0)
            .spawn()
            .expect("start cargo");
        Held {
            child,
            started,
            release,
            log,
        }
    }

    fn wait_until_building(&self) {
        wait_until("the build script starting", || self.started.exists());
    }

    fn wait_until_blocked(&self) {
        wait_until("cargo waiting for a lock", || {
            fs::read_to_string(&self.log)
                .is_ok_and(|log| log.contains("Blocking waiting for file lock"))
        });
    }

    fn release_and_finish(mut self) {
        fs::write(&self.release, "").unwrap();
        // Bounded, so a build that hangs fails its test instead of hanging it; `Drop` then ends it.
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "the build did not finish within a minute of its release"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let log = fs::read_to_string(&self.log).unwrap_or_default();
        assert!(status.success(), "the build failed: {log}");
    }
}

/// A test that panics before `release_and_finish` would leave cargo's build script polling for its
/// release file. So this releases the build too, waits a bounded time, and kills what has not
/// finished by then, with everything it started. Cargo may be blocked on a lock the test holds, and
/// then only the kill ends it.
impl Drop for Held {
    fn drop(&mut self) {
        let _ = fs::write(&self.release, "");
        let deadline = Instant::now() + FINISH_WITHIN;
        while let Ok(None) = self.child.try_wait() {
            if Instant::now() >= deadline {
                // Cargo is not reaped yet, so its pid, which names the group, is still its own.
                let group = Pid::from_raw(self.child.id() as i32);
                let _ = killpg(group, Signal::SIGKILL);
                let _ = self.child.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Whether the lock named by a refusal lies in `folder`.
fn lock_in(err: CleanError, folder: &Path) -> bool {
    matches!(err, CleanError::BuildRunning { lock } if lock.starts_with(folder))
}

#[test]
fn a_running_build_or_check_stops_the_clean_until_it_finishes() {
    let fixture = Fixture::new("lock-running");
    let plan = built_package(&fixture);
    let target = fixture.path("app/target");
    for (name, command) in [("build", &["build"][..]), ("check", &["check"][..])] {
        let held = Held::start(&fixture, name, command);
        held.wait_until_building();

        let refused = clean::lock(&plan).expect_err("a build is running");
        assert!(lock_in(refused, &target), "{name}");

        held.release_and_finish();
        assert!(clean::lock(&plan).is_ok(), "{name} finished");
    }
}

#[test]
fn a_build_deep_in_the_folder_or_inside_a_profile_folder_stops_the_clean() {
    let fixture = Fixture::new("lock-deep");
    let plan = built_package(&fixture);
    for (name, target_dir) in [("deep", "target/a/b/c"), ("inside", "target/debug/own")] {
        let held = Held::start(&fixture, name, &["build", "--target-dir", target_dir]);
        held.wait_until_building();

        let refused = clean::lock(&plan).expect_err("a build is running");
        assert!(
            lock_in(refused, &fixture.path("app").join(target_dir).join("debug")),
            "{name}"
        );

        held.release_and_finish();
        assert!(clean::lock(&plan).is_ok(), "{name} finished");
    }
}

#[test]
fn a_build_that_starts_during_the_clean_waits_then_builds_in_a_tagged_folder_on_live_locks() {
    let fixture = Fixture::new("lock-waiting");
    let plan = built_package(&fixture);
    let target = fixture.path("app/target");
    let locked = clean::lock(&plan).unwrap();
    let waiting = Held::start(&fixture, "waiting", &["build"]);
    waiting.wait_until_blocked();

    locked.delete(&Progress::default()).unwrap();

    waiting.wait_until_building();
    assert!(target.join("CACHEDIR.TAG").exists(), "the tag stays");
    #[cfg(target_os = "macos")]
    {
        assert!(
            excluded_from_backups(&target),
            "and so does the Time Machine exclusion"
        );
    }
    // A check started now waits for the build that took the folder: its locks are real files.
    let later = Held::start(&fixture, "later", &["check"]);
    later.wait_until_blocked();
    waiting.release_and_finish();
    later.release_and_finish();
}

#[test]
fn a_build_in_a_profile_folder_that_appears_after_the_locks_stops_the_delete() {
    let fixture = Fixture::new("lock-new-profile");
    let plan = built_package(&fixture);
    let target = fixture.path("app/target");
    let locked = clean::lock(&plan).unwrap();
    let release = Held::start(&fixture, "release", &["build", "--release"]);
    release.wait_until_building();

    let refused = locked
        .delete(&Progress::default())
        .expect_err("a build is running");

    assert!(lock_in(refused, &target.join("release")));
    assert!(target.join("debug/app").exists(), "nothing was deleted");
    release.release_and_finish();
}

/// Whether macOS keeps `folder` out of Time Machine, as cargo marks a build folder it creates.
#[cfg(target_os = "macos")]
fn excluded_from_backups(folder: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    use nix::libc;

    let path = CString::new(folder.as_os_str().as_bytes()).unwrap();
    let name = CString::new("com.apple.metadata:com_apple_backup_excludeItem").unwrap();
    // SAFETY: both strings are NUL-terminated, and a null buffer of size 0 asks only for the
    // attribute's size.
    let size =
        unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
    size > 0
}
