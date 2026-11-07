//! The alert over the window: the oldest of the notices and questions waiting for the user, which
//! the dashboard's state keeps (`state::dialogs`), and which the window answers by its id.

/// An alert: a notice to read, or a question to answer.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Dialog {
    pub id: u64,
    pub title: String,
    pub detail: String,
    /// The button that says yes, for a question; `None` for a notice, which has only OK.
    pub action: Option<String>,
    /// The yes deletes build folders, so its button is drawn as destructive. A notice's OK and a
    /// yes that only changes a setting are not.
    pub destructive: bool,
}
