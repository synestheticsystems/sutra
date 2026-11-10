//! The alerts waiting for the user: notices to read and questions to answer. The window shows the
//! oldest, one at a time, and answers it by its id, so an answer to an alert no longer shown
//! changes nothing. The dashboard goes on meanwhile; a question holds back only what it asks
//! about.

use std::collections::VecDeque;
use std::path::PathBuf;

use crate::dashboard::Dialog;
use crate::map::clean::{AtRestClean, IdleClean, Prepared};

/// A question: what it asks, in a title and the detail under it, and the label of the button that
/// says yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::dashboard) struct Question {
    pub(in crate::dashboard) title: String,
    pub(in crate::dashboard) detail: String,
    pub(in crate::dashboard) action: String,
}

/// What a yes to a question does.
#[derive(Debug)]
pub(in crate::dashboard) enum Proposal {
    /// Clean what `prepared` deletes, as the click on `clicked` asked, though other projects'
    /// builds go with it. Boxed, as it dwarfs every other alert.
    Clean {
        clicked: PathBuf,
        prepared: Box<Prepared>,
    },
    /// Clean each of `cleans`, the idle build folders.
    CleanIdle(Vec<IdleClean>),
    /// Clean each of `cleans`, the build folders kept only by sessions at rest.
    CleanAtRest(Vec<AtRestClean>),
    /// Keep this many bytes free from now on, cleaning by itself.
    KeepFree(u64),
}

impl Proposal {
    /// Whether the yes deletes build folders. Keeping free space only changes a setting, though
    /// cleans follow from it later, each told in a banner.
    fn deletes(&self) -> bool {
        match self {
            Proposal::Clean { .. } | Proposal::CleanIdle(_) | Proposal::CleanAtRest(_) => true,
            Proposal::KeepFree(_) => false,
        }
    }
}

/// Something for the user to read, or to answer.
#[derive(Debug)]
pub(in crate::dashboard) enum Alert {
    /// Why something the user asked for did not happen, or what they should know; read and
    /// dismissed.
    Notice { title: String, detail: String },
    /// A question before something that cannot be undone.
    Question {
        question: Question,
        proposal: Proposal,
    },
}

impl Alert {
    pub(in crate::dashboard) fn notice(
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Alert {
        Alert::Notice {
            title: title.into(),
            detail: detail.into(),
        }
    }

    pub(in crate::dashboard) fn question(question: Question, proposal: Proposal) -> Alert {
        Alert::Question { question, proposal }
    }
}

/// The alerts not yet answered, oldest first.
#[derive(Debug, Default)]
pub(super) struct Dialogs {
    /// Each alert with its id.
    waiting: VecDeque<(u64, Alert)>,
    /// The id the next alert takes, so no two share one.
    next_id: u64,
}

impl Dialogs {
    /// Put `alert` behind those already waiting.
    pub(super) fn push(&mut self, alert: Alert) {
        self.waiting.push_back((self.next_id, alert));
        self.next_id += 1;
    }

    /// The alert the window shows: the oldest.
    pub(super) fn shown(&self) -> Option<Dialog> {
        let (id, alert) = self.waiting.front()?;
        let dialog = match alert {
            Alert::Notice { title, detail } => Dialog {
                id: *id,
                title: title.clone(),
                detail: detail.clone(),
                action: None,
                destructive: false,
            },
            Alert::Question { question, proposal } => Dialog {
                id: *id,
                title: question.title.clone(),
                detail: question.detail.clone(),
                action: Some(question.action.clone()),
                destructive: proposal.deletes(),
            },
        };
        Some(dialog)
    }

    /// The user answered alert `id`, the one shown: it goes, and a question gives back what it
    /// proposed, for the answer to decide. Nothing for a notice, or for an id no longer shown.
    pub(super) fn answer(&mut self, id: u64) -> Option<Proposal> {
        if self.waiting.front().is_none_or(|(shown, _)| *shown != id) {
            return None;
        }
        match self.waiting.pop_front()?.1 {
            Alert::Notice { .. } => None,
            Alert::Question { proposal, .. } => Some(proposal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(title: &str, floor: u64) -> Alert {
        let question = Question {
            title: title.to_string(),
            detail: "detail".to_string(),
            action: "Keep".to_string(),
        };
        Alert::question(question, Proposal::KeepFree(floor))
    }

    #[test]
    fn the_oldest_alert_shows_until_answered_and_then_the_next() {
        let mut dialogs = Dialogs::default();
        assert_eq!(dialogs.shown(), None);
        dialogs.push(Alert::notice("First", "read me"));
        dialogs.push(question("Second?", 7));

        let first = dialogs.shown().unwrap();
        assert_eq!(
            first,
            Dialog {
                id: first.id,
                title: "First".to_string(),
                detail: "read me".to_string(),
                action: None,
                destructive: false,
            }
        );
        assert!(
            dialogs.answer(first.id).is_none(),
            "a notice proposes nothing"
        );

        let second = dialogs.shown().unwrap();
        assert_ne!(second.id, first.id);
        assert_eq!(second.action.as_deref(), Some("Keep"));
        assert!(matches!(
            dialogs.answer(second.id),
            Some(Proposal::KeepFree(7))
        ));
        assert_eq!(dialogs.shown(), None);
    }

    #[test]
    fn only_a_yes_that_deletes_build_folders_is_drawn_as_destructive() {
        let mut dialogs = Dialogs::default();
        let asked = |dialogs: &mut Dialogs, proposal| {
            let question = Question {
                title: "Clean?".to_string(),
                detail: String::new(),
                action: "Clean".to_string(),
            };
            dialogs.push(Alert::question(question, proposal));
            let shown = dialogs.shown().unwrap();
            dialogs.answer(shown.id);
            shown.destructive
        };
        assert!(asked(&mut dialogs, Proposal::CleanIdle(Vec::new())));
        assert!(asked(&mut dialogs, Proposal::CleanAtRest(Vec::new())));
        assert!(!asked(&mut dialogs, Proposal::KeepFree(1)));
    }

    #[test]
    fn an_answer_to_an_alert_no_longer_shown_changes_nothing() {
        let mut dialogs = Dialogs::default();
        dialogs.push(question("Old?", 1));
        let old = dialogs.shown().unwrap().id;
        assert!(dialogs.answer(old).is_some());
        dialogs.push(question("New?", 2));

        assert!(dialogs.answer(old).is_none(), "answered already");
        assert_eq!(dialogs.shown().unwrap().title, "New?");
    }
}
