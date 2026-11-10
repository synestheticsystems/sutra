//! What the user asks the dashboard to do, which the window passes to `Dashboard::perform`: the
//! action of a line of one of its menus (`MenuAction::Dashboard`), or what one of the window's own
//! controls asks for.

use crate::dashboard::cleaning::WhenIdle;
use crate::dashboard::{GroupKind, LineKey, SortOrder};

/// What the user asks the dashboard to do (`Dashboard::perform`).
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum Action {
    /// Clean the build folder at `folder`.
    Clean {
        folder: String,
    },
    /// Clean every build folder in what the line of `line` holds on the map that its menu
    /// offered, as a click on each one's card would, so each goes only where it would go alone.
    CleanBuildFolders {
        line: LineKey,
    },
    /// Clean what the toolbar's clean offers: every idle build folder, after one question, and
    /// then the folders kept only by agent sessions at rest, after a question of their own.
    CleanOffered,
    /// Keep this much free space from now on, cleaning by itself; `None` leaves cleaning to the
    /// user. Asks first when it turns cleaning on.
    KeepFree {
        bytes: Option<u64>,
    },
    /// Clean the build folders of every group that made no choice of its own by itself from now
    /// on as `choice` says, each once it has sat idle that long, or never. The choice is the
    /// user's say so, so no question follows, and it is kept for the next run.
    CleanEveryGroupWhenIdle {
        choice: WhenIdle,
    },
    /// Clean the build folders of `group` by itself from now on as `own` says, its own choice, or
    /// with `None`, as the choice for every group says. No question follows, and it is kept for
    /// the next run.
    CleanWhenIdle {
        group: GroupKind,
        own: Option<WhenIdle>,
    },
    SetSort {
        order: SortOrder,
    },
    /// Show the filter field (⌘F).
    ShowFilter,
    FilterChanged {
        text: String,
    },
    HideFilter,
    ToggleQuietProjects,
    /// Fold shut what the line of `line` holds, or open it again. The fold is kept for the next
    /// run.
    ToggleFold {
        line: LineKey,
    },
    /// The pointer entered or left the list, which holds the cards in place while it is there.
    PointerEnteredList,
    PointerLeftList,
    ToggleGlobalMute,
    ToggleGlobalNotifications,
    ToggleUnitMute {
        environment: String,
        unit: String,
    },
    ToggleUnitNotifications {
        environment: String,
        unit: String,
    },
    TerminateEnvironment {
        pid: u32,
    },
    /// The user answered the dialog `dialog`: `yes` for its action button, `false` for Cancel or
    /// for a notice's OK.
    Answer {
        dialog: u64,
        yes: bool,
    },
    /// The store edition's user granted `folder` in the open panel.
    FolderGranted {
        folder: String,
    },
    /// The user chose whether Sutra watches the private folder named `folder` (`Documents`).
    WatchPrivateFolder {
        folder: String,
        watch: bool,
    },
}
