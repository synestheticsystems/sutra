//! The engine over real folders: a repository with a linked worktree, dev-runner state in both
//! conventions and a Cargo `target/`, scanned once and then kept current as it changes. Each
//! fixture's root is also its home folder, so no test reads the real one.
//!
//! A test announces each change itself, as the watcher would. While a machine writes many files,
//! FSEvents can deliver nothing for longer than any fair wait (during a docs build on 2026-11-04,
//! no event at all in 15 s), so only `watch`'s own tests depend on it.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use notify::event::CreateKind;
use notify::{Event, EventKind};
use rayon::prelude::*;

use super::live::Message;
use super::sizing::Job;
use super::state::State as MapState;
use super::{Handle, start};
use crate::map::grant::Roots;
use crate::map::test_support::{LiveProcess, TempDir, at, git, measured, usage};
use crate::map::{
    ActivitySource, ArtifactKind, FolderAccess, Measurement, Project, ScanOptions, Snapshot,
    Source, SourceProblem, disk,
};
use crate::model::{Environment, State};

mod discovery;
mod environments;
mod private_folders;
mod sizing;

/// The map the engine would publish first, built in the calling thread without watching, so a test
/// can inspect one complete picture. With `measure_sizes` every build folder and every project's
/// own files are sized first, walked as the sizing pool walks them.
fn scan_once(roots: &Roots, opts: &ScanOptions, measure_sizes: bool) -> Snapshot {
    let mut state = MapState::new(roots, opts);
    state.rediscover(None);
    if measure_sizes {
        let measured: Vec<_> = state
            .jobs_newest_first()
            .into_par_iter()
            .map(|job| {
                let usage = match &job {
                    Job::Artifact(dir) => disk::measure(dir),
                    Job::OwnFiles(project) => {
                        let left_out = state.own_files_left_out(project);
                        disk::measure_pruned(project, &|dir| left_out.contains(dir))
                    }
                };
                (job, usage)
            })
            .collect();
        for (job, usage) in measured {
            state.apply_usage(&job, usage);
        }
    }
    state.snapshot()
}

/// How long an announced change may take to show: the slowest path is an artifact, re-measured
/// 3 s after its last event.
const WITHIN: Duration = Duration::from_secs(15);

/// A Rust repository at `rel` on branch `main` with one commit and a built `target/`.
fn crate_repo(fixture: &TempDir, rel: &str) -> PathBuf {
    let dir = fixture.path(rel);
    fixture.write(&format!("{rel}/Cargo.toml"), b"[package]\nname = \"app\"\n");
    fixture.write(&format!("{rel}/target/debug/app"), [1; 8192]);
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "Cargo.toml"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// The fixture as both the home folder and the only root.
fn roots(fixture: &TempDir) -> Roots {
    Roots {
        home: fixture.root().to_path_buf(),
        dirs: vec![fixture.root().to_path_buf()],
        scratch: None,
        sandboxed: false,
    }
}

fn project<'a>(snapshot: &'a Snapshot, path: &Path) -> Option<&'a Project> {
    snapshot
        .projects
        .iter()
        .find(|project| project.path == path)
}

fn target_bytes(snapshot: &Snapshot, project_path: &Path) -> Option<u64> {
    project(snapshot, project_path)?
        .artifacts
        .iter()
        .find(|artifact| artifact.path == project_path.join("target"))?
        .measurement
        .usage()
        .map(|usage| usage.bytes)
}

/// Tell `engine` that `path` changed, as the watcher would.
fn announce(engine: &Handle, path: &Path) {
    let event = Event::new(EventKind::Create(CreateKind::File)).add_path(path.to_path_buf());
    engine
        .messages
        .send(Message::Fs(Ok(event)))
        .expect("the engine is running");
}

/// The first snapshot within `WITHIN` for which `done` holds.
fn wait_for(
    snapshots: &Receiver<Arc<Snapshot>>,
    what: &str,
    done: impl Fn(&Snapshot) -> bool,
) -> Arc<Snapshot> {
    let deadline = Instant::now() + WITHIN;
    loop {
        match snapshots.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(snapshot) if done(&snapshot) => return snapshot,
            Ok(_) => {}
            Err(err) => panic!("no snapshot showed {what} within {WITHIN:?}: {err}"),
        }
    }
}

/// The map over `fixture`, scanned once and kept for its refreshes to be called.
fn scanned(fixture: &TempDir) -> MapState {
    let mut state = MapState::new(&roots(fixture), &ScanOptions::default());
    state.rediscover(None);
    state
}

/// Register `pid` in the fixture's `~/.dev-runner/` for `folder`, with one unit that is ready.
fn register(fixture: &TempDir, id: &str, folder: &Path, pid: u32, unit: &str) {
    fixture.write(
        &format!(".dev-runner/{id}"),
        format!(
            "DIR={}\nPID={pid}\nSTARTED=100\n{}_PORT=3000\n",
            folder.display(),
            unit.to_uppercase()
        ),
    );
    fixture.write(&format!(".dev-runner/{id}.{unit}.status"), "ready");
}

#[test]
fn start_with_no_roots_publishes_one_empty_snapshot_and_closes() {
    let fixture = TempDir::new("engine-no-roots");
    let roots = Roots {
        dirs: Vec::new(),
        ..roots(&fixture)
    };
    let (_engine, snapshots) = start(roots, ScanOptions::default());
    let snapshot = snapshots.recv_timeout(WITHIN).expect("one snapshot");
    assert!(snapshot.projects.is_empty() && snapshot.roots.is_empty());
    assert!(snapshots.recv_timeout(WITHIN).is_err());
}

#[test]
fn a_stopped_engine_closes_its_snapshot_channel() {
    let fixture = TempDir::new("engine-stop");
    let app = crate_repo(&fixture, "app");
    let (engine, snapshots) = start(roots(&fixture), ScanOptions::default());
    wait_for(&snapshots, "the repository", |s| project(s, &app).is_some());

    engine.stop();

    let closed = loop {
        match snapshots.recv_timeout(WITHIN) {
            // Published before the stop.
            Ok(_) => continue,
            Err(err) => break err,
        }
    };
    assert_eq!(closed, std::sync::mpsc::RecvTimeoutError::Disconnected);
}
