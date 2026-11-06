//! Asking macOS whether Sutra may read each private folder the walk would enter. The first read
//! inside one raises macOS's privacy prompt and waits for the user to answer it, which can take
//! minutes, or forever when the prompt opens behind other windows. So each folder is listed once,
//! on a thread of its own, while the boundary keeps it out of every other read (see
//! `Boundary::keep_out_until_macos_answers`), and the engine never waits on a prompt.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use super::live::Message;
use crate::map::PrivateFolder;
use crate::map::scan::MacOsAnswer;

/// Lists a folder once, as the first read inside a private folder does: `list`, or for a test, a
/// stand-in that returns when the test says.
pub(super) type Probe = Arc<dyn Fn(&Path) -> io::Result<()> + Send + Sync>;

/// List `folder`, which raises macOS's privacy prompt when the user has not answered it yet, and
/// waits for the answer.
pub(super) fn list(folder: &Path) -> io::Result<()> {
    fs::read_dir(folder).map(drop)
}

/// Have `probe` list each of `folders` on a thread of its own, and tell the engine through
/// `messages` what macOS answered for each. A thread whose prompt is still up when the engine stops
/// is left to end when the user answers, and its answer goes nowhere.
pub(super) fn ask(
    folders: Vec<(PrivateFolder, PathBuf)>,
    probe: &Probe,
    messages: &Sender<Message>,
) {
    for (folder, path) in folders {
        let probe = Arc::clone(probe);
        let messages = messages.clone();
        thread::Builder::new()
            .name(format!("sutra-ask-{}", folder.name()))
            .spawn(move || {
                let answer = answer_of(probe(&path));
                // Fails only once the engine has stopped, with no map left to let the folder into.
                let _ = messages.send(Message::MacOsAnswered(folder, answer));
            })
            .expect("spawn a thread to ask macOS about a private folder");
    }
}

/// What macOS answered, as the first listing of a private folder met it. Only a refusal keeps
/// Sutra out: a folder that is not there has nothing for macOS to guard, and any other failure is
/// the walk's to meet and report.
fn answer_of(listing: io::Result<()>) -> MacOsAnswer {
    match listing {
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => MacOsAnswer::Refused,
        Ok(()) | Err(_) => MacOsAnswer::LetIn,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Receiver};

    use super::*;

    #[test]
    fn a_listing_refused_is_macos_saying_no_and_any_other_ending_lets_the_folder_in() {
        let failed = |kind: io::ErrorKind| answer_of(Err(kind.into()));
        assert_eq!(
            failed(io::ErrorKind::PermissionDenied),
            MacOsAnswer::Refused
        );
        assert_eq!(answer_of(Ok(())), MacOsAnswer::LetIn);
        assert_eq!(
            failed(io::ErrorKind::NotFound),
            MacOsAnswer::LetIn,
            "no folder to guard"
        );
        assert_eq!(failed(io::ErrorKind::Other), MacOsAnswer::LetIn);
    }

    /// The answers that reach the engine, as `ask` sends them.
    fn answers(inbox: &Receiver<Message>) -> impl Iterator<Item = (PrivateFolder, MacOsAnswer)> {
        inbox.iter().filter_map(|message| match message {
            Message::MacOsAnswered(folder, answer) => Some((folder, answer)),
            _ => None,
        })
    }

    #[test]
    fn each_folder_is_asked_on_a_thread_of_its_own_so_a_prompt_left_up_holds_back_no_other() {
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let probe: Probe = Arc::new(move |folder: &Path| {
            if folder.ends_with("Documents") {
                // The prompt stays up until the test answers it.
                let _ = released.lock().unwrap().recv();
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            Ok(())
        });
        let (messages, inbox) = mpsc::channel();
        let home = PathBuf::from("/Users/me");
        let folders = vec![
            (PrivateFolder::Documents, home.join("Documents")),
            (PrivateFolder::Desktop, home.join("Desktop")),
        ];

        ask(folders, &probe, &messages);
        drop(messages);

        let mut answers = answers(&inbox);
        assert_eq!(
            answers.next(),
            Some((PrivateFolder::Desktop, MacOsAnswer::LetIn))
        );
        release.send(()).unwrap();
        assert_eq!(
            answers.next(),
            Some((PrivateFolder::Documents, MacOsAnswer::Refused))
        );
        assert_eq!(answers.next(), None, "one answer each");
    }
}
