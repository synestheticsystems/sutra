//! The lines of the window's right-click menus, which the toolbar's clean, each card and each of
//! the list's lines offer, and what each line does.

use crate::dashboard::Action;

/// One line of a right-click menu.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MenuEntry {
    Item {
        title: String,
        /// Whether a check mark shows beside it.
        checked: bool,
        action: MenuAction,
    },
    Separator,
}

/// What a menu line does: the window's own platform services, or something the dashboard does.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MenuAction {
    RevealInFinder {
        path: String,
    },
    CopyPath {
        path: String,
    },
    /// The store edition's open panel, whose folder the window passes on with
    /// `Action::FolderGranted`.
    ChooseFolder,
    Dashboard {
        action: Action,
    },
}
