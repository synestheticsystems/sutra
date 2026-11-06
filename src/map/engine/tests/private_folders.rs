//! Private folders while macOS asks the user about them: the first map goes out without one still
//! waiting for an answer and with one answered at once, the folder joins the map on a yes and
//! stays out on a no, and the engine stops without waiting for a prompt still up. The test answers
//! for macOS, and each fixture's private folders lie in its own home folder.

use std::sync::mpsc::{self, Sender};
use std::thread;

use super::*;
use crate::map::engine::probe::Probe;
use crate::map::engine::start_asking;
use crate::map::{KeptOutBy, PrivateFolder};

/// A folder the engine asked macOS about, waiting until the test answers for macOS.
struct Asked {
    folder: PathBuf,
    answer: Sender<io::Result<()>>,
}

/// A probe that hands the test each folder it lists, through the receiver returned, and returns
/// what the test answers. A folder the test never answers is let go when the test drops its
/// `Asked`, once no engine is left to hear of it.
fn macos_played_by_the_test() -> (Probe, Receiver<Asked>) {
    let (asking, asked) = mpsc::channel();
    let probe: Probe = Arc::new(move |folder: &Path| {
        let (answer, answered) = mpsc::channel();
        let question = Asked {
            folder: folder.to_path_buf(),
            answer,
        };
        asking.send(question).expect("the test hears the question");
        answered
            .recv()
            .unwrap_or_else(|_| Err(io::ErrorKind::Interrupted.into()))
    });
    (probe, asked)
}

/// Settings as they start: Documents watched, Desktop and Downloads not.
fn documents_watched() -> ScanOptions {
    ScanOptions {
        private_folders: [PrivateFolder::Documents].into(),
    }
}

/// Whether Sutra reads the project at `path`, and why not, while it is on the map.
fn access_of(snapshot: &Snapshot, path: &Path) -> Option<FolderAccess> {
    project(snapshot, path).map(|row| row.access)
}

#[test]
fn the_first_map_goes_out_without_a_private_folder_macos_is_asking_about_which_joins_on_a_yes() {
    let fixture = TempDir::new("engine-macos-yes");
    let app = fixture.repo("code/app");
    let notes = fixture.repo("Documents/notes");
    let (probe, questions) = macos_played_by_the_test();
    let (engine, snapshots) = start_asking(roots(&fixture), documents_watched(), probe);
    let asked = questions.recv_timeout(WITHIN).expect("macOS is asked");
    assert_eq!(asked.folder, fixture.path("Documents"));

    let first = wait_for(&snapshots, "the first map", |s| project(s, &app).is_some());
    assert!(
        project(&first, &notes).is_none(),
        "nothing in Documents is read while macOS asks"
    );

    asked.answer.send(Ok(())).unwrap();
    wait_for(&snapshots, "the repository in Documents", |s| {
        access_of(s, &notes) == Some(FolderAccess::Readable)
    });
    engine.stop();
}

#[test]
fn a_worktree_in_a_private_folder_goes_unread_while_macos_asks_and_says_so_when_it_says_no() {
    let fixture = TempDir::new("engine-macos-no");
    let main = fixture.repo("code/app");
    let task = fixture.path("Documents/Codex/task");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "task",
            task.to_str().unwrap(),
        ],
    );
    fixture.write("Documents/Codex/task/node_modules/dep/index.js", "");
    let (probe, questions) = macos_played_by_the_test();
    let (engine, snapshots) = start_asking(roots(&fixture), documents_watched(), probe);
    let asked = questions.recv_timeout(WITHIN).expect("macOS is asked");

    let waiting = Some(FolderAccess::KeptOut(
        PrivateFolder::Documents,
        KeptOutBy::Prompt,
    ));
    // Once the map is sized, nothing else on it is left to change.
    let settled = wait_for(&snapshots, "the map sized, the worktree waiting", |s| {
        access_of(s, &task) == waiting && s.sizing().is_finished()
    });
    assert_eq!(settled.refused_private_folders, []);
    assert!(
        project(&settled, &task).unwrap().artifacts.is_empty(),
        "no build folder is looked for in it, so none is offered for cleaning"
    );
    assert_eq!(
        settled.sizing().total,
        1,
        "only the main worktree's own files are sized"
    );

    asked
        .answer
        .send(Err(io::ErrorKind::PermissionDenied.into()))
        .unwrap();
    let refused = Some(FolderAccess::KeptOut(
        PrivateFolder::Documents,
        KeptOutBy::MacOs,
    ));
    let after = wait_for(&snapshots, "the worktree macOS refused", |s| {
        access_of(s, &task) == refused
    });
    assert_eq!(after.refused_private_folders, [PrivateFolder::Documents]);
    engine.stop();
}

#[test]
fn the_engine_stops_without_waiting_for_a_prompt_still_up() {
    let fixture = TempDir::new("engine-macos-stop");
    let app = fixture.repo("code/app");
    let (probe, questions) = macos_played_by_the_test();
    let (engine, snapshots) = start_asking(roots(&fixture), documents_watched(), probe);
    let _never_answered = questions.recv_timeout(WITHIN).expect("macOS is asked");
    wait_for(&snapshots, "the first map", |s| project(s, &app).is_some());

    let (stopped, stop_returned) = mpsc::channel();
    thread::spawn(move || {
        engine.stop();
        let _ = stopped.send(());
    });

    stop_returned
        .recv_timeout(WITHIN)
        .expect("the engine stops with the prompt still up");
}

#[test]
fn a_private_folder_macos_answers_for_at_once_goes_into_the_first_walk() {
    let fixture = TempDir::new("engine-macos-at-once");
    let notes = fixture.repo("Documents/notes");
    // As macOS answers on every launch after the one where the user answered its prompt.
    let at_once: Probe = Arc::new(|_: &Path| Ok(()));
    let (engine, snapshots) = start_asking(roots(&fixture), documents_watched(), at_once);

    let first = snapshots.recv_timeout(WITHIN).expect("the first map");

    assert_eq!(
        access_of(&first, &notes),
        Some(FolderAccess::Readable),
        "no second walk is needed"
    );
    engine.stop();
}
