//! What the window shows, as the SwiftUI window receives it. The dashboard computes these records
//! from the map and its own state; every word in them is final, so the window only lays them out,
//! picks colours from the states they name, and animates what they mark as live. Each part of the
//! window has a child module that defines its record's types and works the record out, but the
//! dashboard's state works out the alert and the Settings window itself, so their modules only
//! define their records. The lines of the menus several parts offer (`menu`) and what the user asks
//! the dashboard to do (`action`) have modules of their own. This module defines the record of the
//! whole window, and the orders the list can take, which the toolbar offers, an action chooses and
//! the listing follows.

mod action;
pub(super) mod banner;
pub(super) mod card;
mod dialog;
pub(super) mod footer;
pub(super) mod heading;
pub(super) mod list;
mod menu;
mod settings;
pub(super) mod toolbar;

pub use action::Action;
pub use banner::Banner;
pub use card::{
    AgentBadge, AgentLook, Card, Chip, DiskUse, Lozenge, LozengeLook, Robot, ScratchMark, Tray,
    TrayFolder, Unreadable, UnreadableLook,
};
pub use dialog::Dialog;
pub use footer::Footer;
pub use list::{
    DockerCard, ElsewhereLine, GroupFold, GroupLine, List, ListRow, MoreLozenges, Parent,
};
pub use menu::{MenuAction, MenuEntry};
pub use settings::{KeepFreeChoice, PrivateFolderSwitch, Settings};
pub use toolbar::{CleanControl, DiskGauge, GaugeLook, SortChoice, SortControl, Toolbar};

pub(super) use card::CardContext;

/// Everything the window shows at one instant.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WindowContents {
    /// `Sutra · 2 waiting` while agents wait on the user, `Sutra · stopped` once the map no longer
    /// updates, else `Sutra`.
    pub title: String,
    /// The Dock badge: how many agents wait on the user, none at 0, or `!` once the map no longer
    /// updates.
    pub dock_badge: Option<String>,
    pub toolbar: Toolbar,
    /// The strip above the list while a source that alone tells of a kind of thing cannot be read,
    /// so its absence does not pass for nothing running.
    pub banner: Option<Banner>,
    /// Once the engine has stopped, which only a relaunch mends, the line over the list saying it
    /// shows how things last stood. Nothing in the window moves meanwhile: what moves claims news.
    pub stopped: Option<String>,
    pub list: List,
    /// What shows under the list while there is something to say.
    pub footer: Option<Footer>,
    /// The oldest alert not yet answered. The window shows one at a time and answers it with
    /// `Action::Answer`.
    pub dialog: Option<Dialog>,
    pub settings: Settings,
}

/// The orders the project list can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum SortOrder {
    /// Newest activity first, in five-minute slices of the clock, each slice in name order. Above
    /// the quiet row, a git repository's worktrees, two or more, and a Claude Code session's
    /// scratch folders, however few, stand together under a line naming their parent, a
    /// repository's main worktree first, and the group takes the place its newest card would.
    #[default]
    Activity,
    /// By folder name, ignoring case.
    Name,
    /// Largest folders first.
    Disk,
}
