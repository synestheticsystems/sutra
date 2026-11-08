//! The dashboard's state driven event by event as its thread drives it, with no thread loop: the
//! engine's maps, what the user asks for, what the cleaning threads report, and what the window
//! then shows. Every folder is a fixture, and the notifier is muted throughout.

mod cards;
mod folds;
mod gauge;
mod maps;
mod settings;
mod when_idle;

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use super::*;
use crate::dashboard::live;
use crate::dashboard::{Card, Dialog, LineFold, List, ListRow};
use crate::map::Project;
use crate::map::test_support::{TempDir, project_at};

/// A dashboard watching `root`, with `home` for the home folder, keeping its settings in
/// `settings_file`, before any engine starts. It makes no sound, and what the threads it starts
/// report arrives in the receiver.
fn dashboard_at(
    home: &Path,
    root: &Path,
    settings_file: Option<PathBuf>,
) -> (State, Receiver<Event>) {
    let (events, inbox) = mpsc::channel();
    let roots = Roots {
        home: home.to_path_buf(),
        dirs: vec![root.to_path_buf()],
        scratch: None,
        docker: Vec::new(),
        sandboxed: false,
    };
    let mut notifier = Notifier::new(|_, _| {});
    notifier.toggle_global_mute();
    let state = State::new(
        roots,
        None,
        SavedSettings::default(),
        settings_file,
        notifier,
        events,
    );
    (state, inbox)
}

/// A dashboard on `fixture`, its home folder and only root, which keeps its settings there too.
fn dashboard(fixture: &TempDir) -> (State, Receiver<Event>) {
    dashboard_at(fixture.root(), fixture.root(), Some(settings_in(fixture)))
}

/// Where `dashboard` keeps the settings of `fixture`.
fn settings_in(fixture: &TempDir) -> PathBuf {
    fixture.path("Support/Sutra/settings.json")
}

/// Hand `state` what reaches `inbox`, as its thread would, until `done` holds.
fn drive_until(
    state: &mut State,
    inbox: &Receiver<Event>,
    what: &str,
    done: impl Fn(&State) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(state) {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = inbox
            .recv_timeout(left)
            .unwrap_or_else(|_| panic!("waited too long for {what}"));
        let _ = live::handle(state, event);
    }
}

/// A project at `path` active just now, so the list shows its card, holding `artifacts`.
fn active_at(path: PathBuf, artifacts: Vec<crate::map::ArtifactDir>) -> Project {
    Project {
        last_activity: Some(now()),
        artifacts,
        ..project_at(path)
    }
}

/// The card `contents` shows for the project at `path`, which the window knows by its text.
fn card_at<'a>(contents: &'a WindowContents, path: &Path) -> &'a Card {
    let List::Rows { rows } = &contents.list else {
        panic!("no rows: {:?}", contents.list);
    };
    let path = format::path_text(path);
    rows.iter()
        .find_map(|row| match row {
            ListRow::Card { card, .. } if card.path == path => Some(card.as_ref()),
            _ => None,
        })
        .expect("a card for the project")
}

/// What the window shows once the user has opened every line on the list, as they would to reach
/// its cards, every line starting folded.
fn with_every_line_open(state: &mut State) -> WindowContents {
    loop {
        let contents = state.contents(now());
        let List::Rows { rows } = &contents.list else {
            return contents;
        };
        let folded = rows.iter().find_map(|row| match row {
            ListRow::Line { line, .. } if matches!(line.fold, LineFold::Folded { .. }) => {
                Some(line.key.clone())
            }
            _ => None,
        });
        match folded {
            Some(line) => state.perform(Action::ToggleFold { line }, now()),
            None => return contents,
        }
    }
}

fn now() -> SystemTime {
    SystemTime::now()
}

/// Answer the alert `shown`, `yes` to what it proposes or not, as the window would.
fn answer(state: &mut State, shown: &Dialog, yes: bool) {
    let answer = Action::Answer {
        dialog: shown.id,
        yes,
    };
    state.perform(answer, now());
}
