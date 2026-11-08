//! Each rule of a project's provenance, over origins as the map holds them, and the whole of it
//! over real folders: repositories git made, a session's scratch, and a rediscovery that reads them.

use super::*;
use crate::map::grant::Roots;
use crate::map::test_support::{CACHEDIR_TAG, TempDir, git};
use crate::map::{FolderAccess, ScanOptions, ScratchSpace};

/// A checkout at `path` made from `made_from`, nearest first, whose chain ends at `remote`, and
/// which builds `builds`.
fn checkout(path: &str, made_from: &[&str], remote: Option<&str>, builds: &[&str]) -> Tracked {
    let mut tracked = Tracked::bare(PathBuf::from(path), FolderAccess::Readable);
    tracked.origin = Origin::Checkout {
        lineage: Lineage {
            made_from: made_from.iter().map(PathBuf::from).collect(),
            remote: remote.map(str::to_string),
        },
        builds: builds.iter().map(|name| name.to_string()).collect(),
    };
    tracked
}

/// A folder at `path` with no git, which builds `package`: one only an environment names.
fn folder(path: &str, package: Option<&str>) -> Tracked {
    let mut tracked = Tracked::bare(PathBuf::from(path), FolderAccess::Readable);
    tracked.origin = Origin::Folder {
        package: package.map(str::to_string),
    };
    tracked
}

/// `folder`, a scratch folder of a session that started in `started_in`, when the disk still shows
/// that folder.
fn in_scratch(started_in: Option<&str>, mut folder: Tracked) -> Tracked {
    folder.project.scratch = Some(ScratchSpace {
        session_folder: PathBuf::from("/private/tmp/claude-501/-code-app/e26ca359"),
        session_id: "e26ca359".to_string(),
        started_in: "app".to_string(),
        started_in_folder: started_in.map(PathBuf::from),
        session: None,
        session_name: None,
    });
    folder
}

/// The provenance `trace` gives each of `projects`.
fn traced(projects: Vec<Tracked>) -> BTreeMap<PathBuf, Provenance> {
    let projects = projects
        .into_iter()
        .map(|tracked| (tracked.project.path.clone(), tracked))
        .collect();
    trace(&projects)
}

fn upstream(remote: &str) -> GroupKey {
    GroupKey::Upstream(remote.to_string())
}

fn paths(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

#[test]
fn two_clones_of_one_remote_share_its_group_and_hang_from_nothing_on_the_map() {
    let traced = traced(vec![
        checkout("/work/app", &[], Some("github.com/me/app"), &[]),
        checkout("/code/app", &[], Some("github.com/me/app"), &[]),
    ]);
    for path in ["/work/app", "/code/app"] {
        assert_eq!(
            traced[Path::new(path)],
            Provenance {
                group: upstream("github.com/me/app"),
                made_from: vec![],
            },
            "{path}"
        );
    }
}

#[test]
fn a_repository_with_no_remote_groups_its_copies_by_the_checkout_at_the_root_of_their_chain() {
    let traced = traced(vec![
        checkout("/code/notes", &[], None, &[]),
        checkout("/code/notes-wt/draft", &["/code/notes"], None, &[]),
        checkout(
            "/tmp/review",
            &["/code/notes-wt/draft", "/code/notes"],
            None,
            &[],
        ),
    ]);
    let notes = GroupKey::LocalRepository(PathBuf::from("/code/notes"));
    assert_eq!(traced[Path::new("/code/notes")].group, notes);
    assert_eq!(traced[Path::new("/code/notes-wt/draft")].group, notes);
    assert_eq!(
        traced[Path::new("/tmp/review")],
        Provenance {
            group: notes,
            made_from: paths(&["/code/notes-wt/draft", "/code/notes"]),
        }
    );
}

#[test]
fn what_a_project_was_made_from_is_cut_to_the_projects_the_map_tracks() {
    let cache = "/Users/me/.cargo/git/db/ankurah-1a2b";
    let traced = traced(vec![
        checkout("/tmp/ak", &[cache], Some("github.com/ankurah/ankurah"), &[]),
        checkout("/tmp/old-copy", &["/tmp/gone"], None, &[]),
    ]);
    assert!(traced[Path::new("/tmp/ak")].made_from.is_empty());
    assert_eq!(
        traced[Path::new("/tmp/old-copy")],
        Provenance {
            group: GroupKey::LocalRepository(PathBuf::from("/tmp/gone")),
            made_from: vec![],
        },
        "keyed by the folder it came from, gone or not"
    );
}

#[test]
fn a_folder_with_no_git_joins_the_one_group_that_builds_its_package_and_else_has_its_own() {
    let traced = traced(vec![
        checkout(
            "/code/ankurah",
            &[],
            Some("github.com/ak/ankurah"),
            &["ankurah-core"],
        ),
        checkout("/code/fork", &[], Some("github.com/me/fork"), &["shared"]),
        checkout("/code/other", &[], Some("github.com/me/other"), &["shared"]),
        folder("/tmp/core-copy", Some("ankurah-core")),
        folder("/tmp/shared-copy", Some("shared")),
        folder("/tmp/probe", Some("probe")),
        folder("/tmp/notes", None),
    ]);
    let group = |path: &str| traced[Path::new(path)].group.clone();
    assert_eq!(group("/tmp/core-copy"), upstream("github.com/ak/ankurah"));
    assert_eq!(
        group("/tmp/shared-copy"),
        GroupKey::Package("shared".to_string()),
        "two groups build it"
    );
    assert_eq!(group("/tmp/probe"), GroupKey::Package("probe".to_string()));
    assert_eq!(group("/tmp/notes"), GroupKey::Other);
}

#[test]
fn a_scratch_folder_with_no_git_or_package_joins_its_sessions_checkout_and_hangs_from_it() {
    let traced = traced(vec![
        checkout("/code/alice", &[], Some("github.com/me/alice"), &[]),
        checkout(
            "/code/alice-wt/capture",
            &["/code/alice"],
            Some("github.com/me/alice"),
            &[],
        ),
        in_scratch(
            Some("/code/alice-wt/capture/src"),
            folder("/tmp/s/tasks", None),
        ),
        in_scratch(Some("/Users/me"), folder("/tmp/s/notes", None)),
        in_scratch(None, folder("/tmp/s/lost", None)),
    ]);
    assert_eq!(
        traced[Path::new("/tmp/s/tasks")],
        Provenance {
            group: upstream("github.com/me/alice"),
            made_from: paths(&["/code/alice-wt/capture", "/code/alice"]),
        },
        "from the checkout holding the folder its session started in"
    );
    assert_eq!(
        traced[Path::new("/tmp/s/notes")].group,
        GroupKey::Other,
        "a folder no one project holds"
    );
    assert_eq!(traced[Path::new("/tmp/s/lost")].group, GroupKey::Other);
}

#[test]
fn a_scratch_folder_joined_by_its_package_hangs_from_its_sessions_checkout_in_that_group_only() {
    let traced = traced(vec![
        checkout(
            "/code/ak",
            &[],
            Some("github.com/ak/ankurah"),
            &["ankurah-core"],
        ),
        checkout("/code/sutra", &[], Some("github.com/ss/sutra"), &["sutra"]),
        in_scratch(
            Some("/code/ak"),
            folder("/tmp/s/core", Some("ankurah-core")),
        ),
        in_scratch(
            Some("/code/sutra"),
            folder("/tmp/s/core-2", Some("ankurah-core")),
        ),
    ]);
    assert_eq!(
        traced[Path::new("/tmp/s/core")].made_from,
        paths(&["/code/ak"])
    );
    assert_eq!(
        traced[Path::new("/tmp/s/core-2")],
        Provenance {
            group: upstream("github.com/ak/ankurah"),
            made_from: vec![],
        },
        "its session worked in another group"
    );
}

#[test]
fn a_scratch_folder_whose_session_started_in_another_takes_that_ones_group_once_it_has_one() {
    let traced = traced(vec![
        checkout("/code/app", &[], Some("github.com/me/app"), &[]),
        in_scratch(Some("/code/app"), folder("/tmp/s/first", None)),
        in_scratch(Some("/tmp/s/first/deeper"), folder("/tmp/t/second", None)),
    ]);
    assert_eq!(
        traced[Path::new("/tmp/t/second")],
        Provenance {
            group: upstream("github.com/me/app"),
            made_from: paths(&["/tmp/s/first", "/code/app"]),
        }
    );
}

/// A map whose roots are the fixture's `home`, which is also its home folder, with Claude Code's
/// scratch root in its `claude-501`.
fn state_over(fixture: &TempDir) -> State {
    let roots = Roots {
        home: fixture.dir("home"),
        dirs: vec![fixture.path("home")],
        scratch: Some(fixture.path("claude-501")),
        docker: Vec::new(),
        sandboxed: false,
    };
    State::new(&roots, &ScanOptions::default())
}

/// Claude Code's name for `folder`: each character but a letter or a digit becomes `-`.
fn key_of(folder: &Path) -> String {
    let path = folder.to_str().unwrap();
    path.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
}

#[test]
fn a_rediscovery_groups_a_repository_its_clone_in_scratch_and_its_sessions_scratch_together() {
    let fixture = TempDir::new("engine-provenance-rediscovered");
    let alice = fixture.repo("home/code/alice");
    git(
        &alice,
        &["remote", "add", "origin", "git@github.com:me/alice.git"],
    );
    let key = key_of(&alice);
    let scratchpad = format!("claude-501/{key}/e26ca359/scratchpad");
    let clone = fixture.path(&format!("{scratchpad}/alice-review"));
    git(
        fixture.root(),
        &[
            "clone",
            "-q",
            alice.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    fixture.write(
        &format!("{scratchpad}/alice-review/target/CACHEDIR.TAG"),
        CACHEDIR_TAG,
    );
    fixture.write(&format!("{scratchpad}/out-1/CACHEDIR.TAG"), CACHEDIR_TAG);
    // With no build folder, too small for a card: it earns none, but its group is known all the
    // same.
    let other = fixture.repo(&format!("{scratchpad}/other"));
    git(
        &other,
        &["remote", "add", "origin", "https://github.com/me/other"],
    );
    let mut state = state_over(&fixture);

    state.rediscover(None);

    let snapshot = state.snapshot();
    let provenance = |path: &Path| {
        let project = snapshot
            .projects
            .iter()
            .find(|project| project.path == path);
        project.map(|project| project.provenance.clone())
    };
    let alice_group = upstream("github.com/me/alice");
    assert_eq!(
        provenance(&alice),
        Some(Provenance {
            group: alice_group.clone(),
            made_from: vec![],
        })
    );
    assert_eq!(
        provenance(&clone),
        Some(Provenance {
            group: alice_group.clone(),
            made_from: vec![alice.clone()],
        }),
        "a clone of a checkout on the map hangs from it"
    );
    assert_eq!(
        provenance(&fixture.path(&format!("{scratchpad}/out-1"))),
        Some(Provenance {
            group: alice_group.clone(),
            made_from: vec![alice.clone()],
        }),
        "a build folder its session made while working in alice"
    );
    assert_eq!(provenance(&other), None, "no card");
    assert_eq!(
        *snapshot.ancestry,
        Ancestry {
            groups: BTreeSet::from([alice_group, upstream("github.com/me/other")]),
            made_from: BTreeSet::from([alice]),
        },
        "the group of the clone with no card among them"
    );
}
