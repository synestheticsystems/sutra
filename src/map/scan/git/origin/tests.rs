//! Lineages read from repositories git itself made: clones of a host, of a local checkout, of a
//! linked worktree and of a bare repository, Cargo's git cache among them, and chains that end at a
//! folder gone, kept out of or sealed, or that loop; and a scratch folder's, read from what it holds.

use super::*;
use crate::map::scan::repo_info;
use crate::map::test_support::{TempDir, git};

/// Anywhere at all.
fn anywhere(_: &Path) -> bool {
    true
}

/// The lineage of the worktree at `worktree`, read wherever it leads.
fn lineage_at(worktree: &Path) -> (Lineage, Vec<Failure>) {
    lineage(&repo_info(worktree).expect("a worktree"), &anywhere)
}

/// A repository at `rel` cloned from the host `url`, as its `origin` remote says.
fn cloned_from_host(fixture: &TempDir, rel: &str, url: &str) -> PathBuf {
    let repo = fixture.repo(rel);
    git(&repo, &["remote", "add", "origin", url]);
    repo
}

#[test]
fn a_clone_of_a_host_names_its_remote_and_was_made_from_nothing_on_this_mac() {
    let fixture = TempDir::new("origin-host");
    let app = cloned_from_host(&fixture, "app", "git@github.com:Synesthetic/Sutra.git");
    assert_eq!(
        lineage_at(&app),
        (
            Lineage {
                made_from: vec![],
                remote: Some("github.com/Synesthetic/Sutra".to_string()),
            },
            vec![]
        )
    );
}

#[test]
fn a_linked_worktree_was_made_from_its_main_worktree_and_shares_its_remote() {
    let fixture = TempDir::new("origin-linked");
    let main = cloned_from_host(&fixture, "app", "https://github.com/me/app.git");
    git(&main, &["worktree", "add", "-q", "-b", "fix", "../app-fix"]);
    let (lineage, failures) = lineage_at(&fixture.path("app-fix"));
    assert_eq!(lineage.made_from, [main]);
    assert_eq!(lineage.remote.as_deref(), Some("github.com/me/app"));
    assert_eq!(failures, []);
}

#[test]
fn a_clone_of_a_local_clone_is_followed_through_it_to_the_host() {
    let fixture = TempDir::new("origin-local-clones");
    let upstream = cloned_from_host(&fixture, "code/alice", "ssh://git@github.com:22/me/alice");
    let first = fixture.path("scratch/alice-1");
    git(
        fixture.root(),
        &["clone", "-q", "code/alice", first.to_str().unwrap()],
    );
    // Cloned by a path relative to where git ran, which git records whole.
    git(
        &fixture.dir("scratch"),
        &["clone", "-q", "alice-1", "alice-2"],
    );

    let (lineage, failures) = lineage_at(&fixture.path("scratch/alice-2"));

    assert_eq!(lineage.made_from, [first, upstream]);
    assert_eq!(lineage.remote.as_deref(), Some("github.com/me/alice"));
    assert_eq!(failures, []);
}

#[test]
fn a_clone_of_a_linked_worktree_was_made_from_it_and_from_its_main_worktree() {
    let fixture = TempDir::new("origin-clone-of-linked");
    let main = cloned_from_host(&fixture, "code/alice", "https://github.com/me/alice");
    let capture = fixture.path("code/alice-wt/capture");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "capture",
            capture.to_str().unwrap(),
        ],
    );
    git(
        fixture.root(),
        &["clone", "-q", capture.to_str().unwrap(), "scratch/review"],
    );

    let (lineage, _) = lineage_at(&fixture.path("scratch/review"));

    assert_eq!(lineage.made_from, [capture, main]);
    assert_eq!(lineage.remote.as_deref(), Some("github.com/me/alice"));
}

#[test]
fn a_clone_of_a_bare_repository_is_followed_through_it_to_its_remote() {
    let fixture = TempDir::new("origin-bare");
    fixture.repo("seed");
    let db = fixture.path("home/repos/ankurah.git");
    git(
        fixture.root(),
        &["clone", "-q", "--bare", "seed", db.to_str().unwrap()],
    );
    git(
        &db,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/ankurah/ankurah",
        ],
    );
    let file_url = format!("file://{}", db.display());
    git(fixture.root(), &["clone", "-q", &file_url, "scratch/ak"]);

    let (lineage, failures) = lineage_at(&fixture.path("scratch/ak"));

    assert_eq!(lineage.made_from, [db]);
    assert_eq!(
        lineage.remote.as_deref(),
        Some("github.com/ankurah/ankurah")
    );
    assert_eq!(failures, []);
}

#[test]
fn a_bare_repository_with_no_remote_was_made_from_what_it_fetched_as_cargos_git_cache_is() {
    let fixture = TempDir::new("origin-cargo");
    fixture.repo("seed");
    let db = fixture.path("home/.cargo/git/db/world-proto-31e1");
    git(
        fixture.root(),
        &["clone", "-q", "--bare", "seed", db.to_str().unwrap()],
    );
    git(&db, &["remote", "remove", "origin"]);
    // As Cargo writes it, each fetched commit followed by the URL it came from.
    fixture.write(
        "home/.cargo/git/db/world-proto-31e1/FETCH_HEAD",
        "cd4923516827d79fbac742179a4193627dafa43b\t\t'cd4923516827d79fbac742179a4193627dafa43b' \
         of https://github.com/greyorange/world-proto.git\n",
    );
    let file_url = format!("file://{}", db.display());
    git(fixture.root(), &["clone", "-q", &file_url, "scratch/proto"]);

    let (lineage, failures) = lineage_at(&fixture.path("scratch/proto"));

    assert_eq!(lineage.made_from, std::slice::from_ref(&db));
    assert_eq!(
        lineage.remote.as_deref(),
        Some("github.com/greyorange/world-proto")
    );
    assert_eq!(failures, []);
    fs::remove_file(db.join("FETCH_HEAD")).unwrap();
    let (unfetched, _) = lineage_at(&fixture.path("scratch/proto"));
    assert_eq!(
        unfetched.remote, None,
        "a bare repository that fetched nothing"
    );
}

#[test]
fn a_repository_with_no_remote_ends_its_chain_with_none() {
    let fixture = TempDir::new("origin-no-remote");
    let notes = fixture.repo("notes");
    git(
        &notes,
        &["worktree", "add", "-q", "-b", "draft", "../notes-draft"],
    );
    assert_eq!(lineage_at(&notes), (Lineage::default(), vec![]));
    let (draft, _) = lineage_at(&fixture.path("notes-draft"));
    assert_eq!(draft.made_from, [notes]);
    assert_eq!(draft.remote, None);
}

#[test]
fn a_chain_that_reaches_a_folder_gone_ends_there_and_that_is_no_failure() {
    let fixture = TempDir::new("origin-gone");
    fixture.repo("old");
    git(fixture.root(), &["clone", "-q", "old", "copy"]);
    let old = fixture.path("old");
    std::fs::remove_dir_all(&old).unwrap();
    assert_eq!(
        lineage_at(&fixture.path("copy")),
        (
            Lineage {
                made_from: vec![old],
                remote: None,
            },
            vec![]
        )
    );
}

#[test]
fn a_chain_stops_where_sutra_may_not_follow_without_reading_there() {
    let fixture = TempDir::new("origin-kept-out");
    let private = cloned_from_host(&fixture, "Desktop/app", "https://github.com/me/app");
    git(fixture.root(), &["clone", "-q", "Desktop/app", "code/app"]);
    // Sealed, so a read of its config would fail and say so.
    fixture.restrict(&private.join(".git"), 0o000);
    let desktop = fixture.path("Desktop");
    let outside_desktop = |path: &Path| !path.starts_with(&desktop);

    let repo = repo_info(&fixture.path("code/app")).unwrap();
    let (lineage, failures) = lineage(&repo, &outside_desktop);

    assert_eq!(lineage.made_from, [private]);
    assert_eq!(lineage.remote, None);
    assert_eq!(failures, [], "nothing was read there");
}

#[test]
fn a_worktree_of_a_repository_where_sutra_may_not_follow_stops_its_chain_without_reading_there() {
    let fixture = TempDir::new("origin-kept-out-worktree");
    let private = cloned_from_host(&fixture, "Desktop/app", "https://github.com/me/app");
    let add_worktree = |folder: &Path| {
        let branch = folder.file_name().unwrap().to_str().unwrap();
        let folder = folder.to_str().unwrap();
        git(&private, &["worktree", "add", "-q", "-b", branch, folder]);
    };
    let top = fixture.path("scratch/top");
    add_worktree(&top);
    add_worktree(&fixture.path("scratch/task/held"));
    git(
        fixture.root(),
        &["clone", "-q", top.to_str().unwrap(), "scratch/copy"],
    );
    // Sealed, so a read in it would fail and say so.
    fixture.restrict(&private.join(".git"), 0o000);
    let desktop = fixture.path("Desktop");
    let outside_desktop = |path: &Path| !path.starts_with(&desktop);
    let ends_at = |made_from: Vec<PathBuf>| Lineage {
        made_from,
        remote: None,
    };

    assert_eq!(
        lineage_of_folder(&top, &outside_desktop),
        (Some(ends_at(vec![private.clone()])), vec![]),
        "a worktree of it"
    );
    assert_eq!(
        lineage_of_scratch_folder(&top, &outside_desktop),
        (None, vec![]),
        "a scratch folder that is one, its chain ending at no remote"
    );
    assert_eq!(
        lineage_of_scratch_folder(&fixture.path("scratch/task"), &outside_desktop),
        (None, vec![]),
        "a scratch folder holding one"
    );
    let copy = repo_info(&fixture.path("scratch/copy")).expect("a worktree");
    assert_eq!(
        lineage(&copy, &outside_desktop),
        (ends_at(vec![top, private]), vec![]),
        "a clone of one"
    );
}

#[test]
fn a_chain_through_a_repository_that_cannot_be_read_names_it_among_the_failures() {
    let fixture = TempDir::new("origin-sealed");
    let sealed = cloned_from_host(&fixture, "sealed/app", "https://github.com/me/app");
    git(fixture.root(), &["clone", "-q", "sealed/app", "copy"]);
    fixture.restrict(&sealed.join(".git/config"), 0o000);

    let (lineage, failures) = lineage_at(&fixture.path("copy"));

    assert_eq!(lineage.made_from, std::slice::from_ref(&sealed));
    assert_eq!(lineage.remote, None);
    assert_eq!(
        failures,
        [(sealed.join(".git/config"), io::ErrorKind::PermissionDenied)]
    );
}

#[test]
fn a_chain_whose_remotes_loop_ends_where_it_comes_round_again() {
    let fixture = TempDir::new("origin-loop");
    let a = fixture.repo("a");
    git(fixture.root(), &["clone", "-q", "a", "b"]);
    let b = fixture.path("b");
    git(&a, &["remote", "add", "origin", b.to_str().unwrap()]);

    let (lineage, failures) = lineage_at(&b);

    assert_eq!(lineage.made_from, [a]);
    assert_eq!(lineage.remote, None);
    assert_eq!(failures, []);
}

#[test]
fn a_folder_that_is_a_checkout_has_a_lineage_and_one_that_is_not_has_none() {
    let fixture = TempDir::new("origin-folder");
    let clone = cloned_from_host(&fixture, "scratch/clone", "https://github.com/me/app");
    let plain = fixture.dir("scratch/notes");
    let (lineage, failures) = lineage_of_folder(&clone, &anywhere);
    assert_eq!(
        lineage.and_then(|lineage| lineage.remote).as_deref(),
        Some("github.com/me/app")
    );
    assert_eq!(failures, []);
    assert_eq!(lineage_of_folder(&plain, &anywhere), (None, vec![]));
}

#[test]
fn a_scratch_folder_with_no_git_goes_by_the_one_folder_in_it_holding_one_and_no_link() {
    let fixture = TempDir::new("origin-scratch-folder");
    let review = fixture.dir("scratch/review");
    cloned_from_host(&fixture, "scratch/review/app", "https://github.com/me/app");
    fixture.dir("scratch/review/notes");
    fixture.write("scratch/review/README.md", "");
    let elsewhere = cloned_from_host(&fixture, "elsewhere", "https://github.com/me/elsewhere");
    std::os::unix::fs::symlink(&elsewhere, review.join("linked")).unwrap();
    let remote = |folder: &Path| {
        let (lineage, failures) = lineage_of_scratch_folder(folder, &anywhere);
        assert_eq!(failures, []);
        lineage.and_then(|lineage| lineage.remote)
    };
    assert_eq!(remote(&review).as_deref(), Some("github.com/me/app"));

    let own = cloned_from_host(&fixture, "scratch/own", "https://github.com/me/own");
    cloned_from_host(
        &fixture,
        "scratch/own/vendored",
        "https://github.com/me/vendored",
    );
    assert_eq!(
        remote(&own).as_deref(),
        Some("github.com/me/own"),
        "its own git comes first"
    );

    let sealed = fixture.dir("scratch/review/sealed");
    fixture.restrict(&sealed, 0o000);
    assert_eq!(
        lineage_of_scratch_folder(&review, &anywhere),
        (None, vec![]),
        "a folder it cannot look into may hold a checkout too, and its search names that folder"
    );
}

#[test]
fn in_scratch_only_a_chain_that_ends_at_a_remote_counts_a_folders_own_or_its_one_checkouts() {
    let fixture = TempDir::new("origin-scratch-remote");
    let notes = fixture.repo("home/notes");
    let clone_of_notes = |into: &str| {
        let (from, into) = (notes.to_str().unwrap(), fixture.path(into));
        git(
            fixture.root(),
            &["clone", "-q", from, into.to_str().unwrap()],
        );
        into
    };
    let made_there = fixture.repo("scratch/git-check");
    let copy = clone_of_notes("scratch/notes-copy");
    clone_of_notes("scratch/task/notes");
    let work = fixture.repo("scratch/work");
    cloned_from_host(&fixture, "scratch/work/app", "https://github.com/me/app");
    let lineage = |folder: &Path| lineage_of_scratch_folder(folder, &anywhere);

    assert_eq!(lineage(&made_there), (None, vec![]), "made there");
    assert_eq!(
        lineage(&copy),
        (None, vec![]),
        "cloned from a repository with no remote"
    );
    assert_eq!(
        lineage(&fixture.path("scratch/task")),
        (None, vec![]),
        "holding such a clone"
    );
    let (held, failures) = lineage(&work);
    assert_eq!(
        held.and_then(|held| held.remote).as_deref(),
        Some("github.com/me/app"),
        "a repository with no remote, holding one clone of a host"
    );
    assert_eq!(failures, []);
}

#[test]
fn a_url_is_keyed_without_its_scheme_user_port_or_git_and_with_its_host_in_lowercase() {
    for (url, key) in [
        ("https://github.com/owner/repo.git", "github.com/owner/repo"),
        (
            "https://user:token@GitHub.com/owner/repo/",
            "github.com/owner/repo",
        ),
        ("git@github.com:owner/repo.git", "github.com/owner/repo"),
        (
            "ssh://git@github.com:22/owner/repo.git",
            "github.com/owner/repo",
        ),
        (
            "git://gitlab.example.org/group/sub/repo",
            "gitlab.example.org/group/sub/repo",
        ),
        ("  github.com:owner/Repo  ", "github.com/owner/Repo"),
    ] {
        assert_eq!(normalized_url(url), key, "{url}");
    }
}

#[test]
fn a_remote_names_a_folder_by_its_path_or_a_file_url_and_a_host_otherwise() {
    let base = Path::new("/code/app");
    for (url, folder) in [
        ("/Users/me/code/alice", Some("/Users/me/code/alice")),
        (
            "file:///Users/me/.cargo/git/db/x",
            Some("/Users/me/.cargo/git/db/x"),
        ),
        ("../alice", Some("/code/alice")),
        ("./vendor/alice:fork", Some("/code/app/vendor/alice:fork")),
        ("git@github.com:owner/repo", None),
        ("https://github.com/owner/repo", None),
        ("host:repo", None),
    ] {
        assert_eq!(
            local_path(url, base).as_deref(),
            folder.map(Path::new),
            "{url}"
        );
    }
}

#[test]
fn the_origin_url_is_the_first_url_of_the_origin_remotes_section_as_git_reads_it() {
    let config = r#"
[core]
	url = not-a-remote
[remote "upstream"]
	url = https://github.com/upstream/repo
[Remote "origin"] fetch = +refs/heads/*:refs/remotes/origin/*
	URL = "https://github.com/me/repo" ; where it was cloned from
	url = https://github.com/me/second
"#;
    assert_eq!(
        origin_url(config).as_deref(),
        Some("https://github.com/me/repo")
    );
    let older = "[remote.origin]\n\turl = /Users/me/code/app # a local clone\n";
    assert_eq!(origin_url(older).as_deref(), Some("/Users/me/code/app"));
    let quoted = "[remote \"origin\"]\n\turl = \"/Users/me/My \\\"Code\\\"/app\"\n";
    assert_eq!(
        origin_url(quoted).as_deref(),
        Some("/Users/me/My \"Code\"/app")
    );
    assert_eq!(origin_url("[remote \"upstream\"]\n\turl = x\n"), None);
}
