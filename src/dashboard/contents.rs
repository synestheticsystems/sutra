//! What the window shows, as the SwiftUI window receives it. The dashboard computes these records
//! from the map and its own state; every word in them is final, so the window only lays them out,
//! picks colours from the states they name, and animates what they mark as live. Each part of the
//! window has a child module that works out its record.

pub(super) mod banner;
pub(super) mod card;
pub(super) mod footer;
pub(super) mod list;
pub(super) mod toolbar;

pub(super) use card::CardContext;

/// Everything the window shows at one instant.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WindowContents {
    /// `Sutra · 2 waiting` while agents wait on the user, else `Sutra`.
    pub title: String,
    /// How many agents wait on the user, for the Dock badge, which shows none at 0.
    pub waiting: u32,
    pub toolbar: Toolbar,
    /// The strip above the list while a source that alone tells of a kind of thing cannot be read,
    /// so its absence does not pass for nothing running.
    pub banner: Option<Banner>,
    pub list: List,
    /// What shows under the list while there is something to say.
    pub footer: Option<Footer>,
    /// The oldest alert not yet answered. The window shows one at a time and answers it with
    /// `Action::Answer`.
    pub dialog: Option<Dialog>,
    pub settings: Settings,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Toolbar {
    /// The current sort, and the sorts to choose from. `None` while there is at most one project,
    /// when ordering means nothing.
    pub sort: Option<SortControl>,
    /// The filter field's text while the field shows (⌘F).
    pub filter: Option<String>,
    /// The clean of idle build folders.
    pub clean: CleanControl,
    pub muted: bool,
    /// What the sound switch does: `Mute all sounds`, or `Unmute all sounds` while muted.
    pub mute_tooltip: String,
    pub notifications_off: bool,
    /// What the notification switch does: `Disable notifications`, or `Enable notifications`
    /// while they are off.
    pub notifications_tooltip: String,
    /// The store edition's folder button, with what it watches for its tooltip; `None` outside the
    /// App Sandbox, where Sutra reads the whole home folder, and until the first map arrives.
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SortControl {
    pub current: SortOrder,
    pub choices: Vec<SortChoice>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SortChoice {
    pub order: SortOrder,
    pub label: String,
    pub tooltip: String,
}

/// The orders the project list can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum SortOrder {
    /// Newest activity first, in five-minute slices of the clock, each slice in name order.
    #[default]
    Activity,
    /// By folder name, ignoring case.
    Name,
    /// Largest folders first.
    Disk,
}

/// The toolbar's clean: a click cleans every idle build folder, a right click sets what Sutra keeps
/// free.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CleanControl {
    /// Something idle is on offer, so a click does something; the icon fades while nothing is.
    pub offered: bool,
    /// A click's clean is being worked out, which takes seconds when many folders are searched
    /// for locks.
    pub busy: bool,
    pub tooltip: String,
    /// What a right click offers.
    pub menu: Vec<MenuEntry>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Banner {
    /// What Sutra cannot see, as one sentence.
    pub heading: String,
    /// Each unreadable source and why, one line each.
    pub reasons: Vec<String>,
    /// The store edition offers the remedy: granting a larger folder (`MenuAction::ChooseFolder`).
    pub offers_folder: bool,
}

/// The list, or what stands in its place.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum List {
    /// The store edition has no folder to read yet: what Sutra needs and why, with the button that
    /// opens the panel (`MenuAction::ChooseFolder`).
    NoFolder {
        message: String,
        /// Why the folder granted on an earlier launch could not be reopened.
        lost: Option<String>,
    },
    /// The first map is still being made.
    Scanning,
    /// Nothing to list, or nothing the filter admits, or no map ever came because the scanner
    /// stopped, and why, in a sentence or two.
    Empty {
        message: String,
    },
    Rows {
        rows: Vec<ListRow>,
    },
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ListRow {
    Card {
        // Boxed, as a card dwarfs the other rows; the window receives it as a plain card.
        card: Box<Card>,
    },
    /// The agents working outside every project, under `heading`, each beside the folder it works
    /// in, those waiting on the user first.
    Elsewhere {
        heading: String,
        agents: Vec<ElsewhereAgent>,
        /// One of them waits on the user, so the row throbs as a card does.
        waiting: bool,
    },
    /// The row standing for the quiet projects: how many, and whether they are listed below it.
    /// A click opens or closes it (`Action::ToggleQuietProjects`).
    QuietProjects {
        count: u32,
        open: bool,
        label: String,
        tooltip: String,
    },
}

/// One project's card.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Card {
    /// The project's folder, which actions on the card name. Text, as every APFS path is.
    pub path: String,
    /// The folder in full, with `~` for the home folder: shown while the pointer is over the card.
    pub location: String,
    pub name: String,
    /// The folder, then when its last activity was and where it came from.
    pub name_tooltip: String,
    /// The branch chip and its tooltip (which worktree of which repository this is); `None` for a
    /// plain folder.
    pub branch: Option<Chip>,
    /// How long ago the last activity was, as `4m` reads.
    pub age: Option<String>,
    /// Why the folder cannot be read, above the agents and units that sources outside it still
    /// show.
    pub unreadable: Option<Unreadable>,
    pub agents: Vec<AgentLozenge>,
    pub units: Vec<UnitLozenge>,
    pub tray: Option<Tray>,
    /// An agent in it waits on the user, so the card throbs.
    pub waiting: bool,
    /// What a right click offers.
    pub menu: Vec<MenuEntry>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Chip {
    pub text: String,
    pub tooltip: String,
}

/// One agent session, as `(Claude) 4m` reads: the colour says what it is doing.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentLozenge {
    pub harness: String,
    pub look: AgentLook,
    /// How long it has waited or been quiet, from its first whole minute.
    pub beside: Option<String>,
    /// Its status in words, and the session's name or id.
    pub tooltip: String,
    /// Opens the session in the app it runs in, on a click.
    pub link: Option<String>,
}

/// An agent working outside every project, on the row of those.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ElsewhereAgent {
    pub lozenge: AgentLozenge,
    /// The folder it works in, with `~` for the home folder.
    pub folder: String,
}

/// Why a card shows nothing from inside its folder, as `in Desktop, not watched` reads.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Unreadable {
    pub reason: String,
    pub look: UnreadableLook,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnreadableLook {
    /// Kept out by the user's own choice, in Settings or in the folder granted: a quiet sign, since
    /// nothing went wrong.
    KeptOut,
    /// Missing or refused: the warning sign.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentLook {
    /// Waiting on the user: amber, throbbing with its card.
    Waiting,
    /// At work: green, pulsing slowly.
    Working,
    /// Quiet mid-turn, idle, or with no status: grey and still.
    Still,
}

/// One dev-runner unit, as `(web :3000)` reads.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UnitLozenge {
    pub name: String,
    /// The port as the window draws it after the name, colon included: `:3000`.
    pub port: Option<String>,
    pub look: UnitLook,
    /// What a unit under way or in trouble says of itself.
    pub beside: Option<String>,
    pub muted: bool,
    pub notifications_off: bool,
    pub tooltip: String,
    /// Opens the unit in the browser, on a click.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnitLook {
    /// Up: green.
    Up,
    /// On the way: blue, so amber stays for what needs the user.
    Starting,
    /// Failed: red.
    Failed,
    /// Stopped, unknown, or its environment's process has gone: grey.
    Still,
}

/// The card's disk tray, all on one line, as `32 GB  target/  node_modules/` reads.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Tray {
    /// The folder's whole size, with what a clean has left of the folders it is at work on.
    pub total: Option<String>,
    /// The build folders too small to name, for the total's tooltip.
    pub total_tooltip: Option<String>,
    /// Why the folder has no total, when measuring its own files failed: an alert sign's tooltip.
    pub warning: Option<String>,
    /// The build folders large enough to matter, largest first.
    pub folders: Vec<TrayFolder>,
    /// How far the clean of its build folders has come, from 0 to 1, by the space given back:
    /// the bar along the seam above the tray. Empty while a clean is worked out or waits its turn.
    pub progress: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TrayFolder {
    /// As `target/` reads.
    pub label: String,
    /// Its own size, and while cleaning, what is left of it.
    pub tooltip: String,
    /// A clean is at work on it.
    pub cleaning: bool,
}

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

/// What the user asks the dashboard to do (`Dashboard::perform`).
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum Action {
    /// Clean the build folder at `folder`.
    Clean {
        folder: String,
    },
    /// Clean every idle build folder, after one question.
    CleanIdle,
    /// Keep this much free space from now on, cleaning by itself; `None` leaves cleaning to the
    /// user. Asks first when it turns cleaning on.
    KeepFree {
        bytes: Option<u64>,
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

/// What shows under the list.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Footer {
    /// How far the first sizing has come, from 0 to 1, while it runs.
    pub sizing: Option<f32>,
    /// Sources whose failure loses only some folders or some changes, one line each, for an alert
    /// sign's tooltip.
    pub problems: Vec<String>,
    /// The scanner stopped, which only a relaunch mends.
    pub stopped: Option<String>,
}

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

/// What the Settings window shows.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Settings {
    /// The free space Sutra keeps by cleaning by itself, and the choices.
    pub keep_free: Option<u64>,
    pub keep_free_choices: Vec<KeepFreeChoice>,
    /// The folders macOS guards with a privacy prompt, each with whether Sutra watches it.
    pub private_folders: Vec<PrivateFolderSwitch>,
}

/// One of the folders macOS guards, and whether Sutra watches it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PrivateFolderSwitch {
    /// `Documents`: the switch's label, and the folder `Action::WatchPrivateFolder` names.
    pub folder: String,
    pub watched: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct KeepFreeChoice {
    pub bytes: Option<u64>,
    pub label: String,
}
