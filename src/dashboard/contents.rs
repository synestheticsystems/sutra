//! What the window shows, as the SwiftUI window receives it. The dashboard computes these records
//! from the map and its own state; every word in them is final, so the window only lays them out,
//! picks colours from the states they name, and animates what they mark as live. Each part of the
//! window has a child module that works out its record.

pub(super) mod banner;
pub(super) mod card;
pub(super) mod footer;
pub(super) mod heading;
pub(super) mod list;
pub(super) mod toolbar;

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
    /// How full the disk holding the home folder is; `None` until it is first read, or while the
    /// volume does not answer.
    pub disk: Option<DiskGauge>,
}

/// The gauge at the end of the scope bar: a bar filling with the share of the disk in use, and that
/// share in words.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiskGauge {
    /// The share of the disk in use, from 0 to 1, to whole percents.
    pub used: f32,
    /// That share: `94%`.
    pub label: String,
    /// The disk in bytes: `1.8 TB of 2 TB in use, 160 GB free`.
    pub tooltip: String,
    pub look: GaugeLook,
}

/// How the gauge looks: plain while the disk has room, amber once little is free, red once almost
/// nothing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GaugeLook {
    Roomy,
    Tight,
    Full,
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
    /// Newest activity first, in five-minute slices of the clock, each slice in name order. The
    /// cards of a parent with two or more of them above the quiet row, a git repository's
    /// worktrees or a Claude Code session's scratch folders, stand together under a line naming
    /// it, a repository's main worktree first, and the group takes the place its newest card would.
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
    /// The cards that share a parent, in the Activity sort, where two or more of them are listed
    /// together: a line naming the parent, with the disk its cards take, then the cards, nested
    /// under it, unless the line holds them folded. The line names what the cards share, so each
    /// goes by what sets it apart: a worktree by its branch, or on no branch by its folder, and a
    /// scratch folder by its own name. A click on the line folds the group shut, and another opens
    /// it (`Action::ToggleFold`).
    Group { line: GroupLine, fold: GroupFold },
    /// The agents working outside every project, under `heading`: a line for each session, in the
    /// map's order, then one for each other folder subagents work in, in the order of the first
    /// subagent there. No folder has two lines of subagents, and none moves when a status changes.
    Elsewhere {
        heading: String,
        lines: Vec<ElsewhereLine>,
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

/// The line over a group's cards, which names their parent.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct GroupLine {
    /// What the cards share, which the window knows the group by, and a click on the line names.
    pub parent: Parent,
    /// A repository's main worktree's folder name, or a bare repository's own, less `.git`; a
    /// session's name, as its scratch folders go by alone.
    pub name: String,
    /// Where a repository is, as `~/code/app` reads, which tells two clones of one remote apart;
    /// a session's name and whether it still runs, then its folder.
    pub tooltip: String,
    /// On a session's line, the mark of its scratch, beside its name. While the mark says the
    /// session has ended, the whole line steps back as a robot at rest does.
    pub scratch: Option<ScratchMark>,
    /// The disk the parent's cards on the map take between them, listed or not, as their trays
    /// write it: the sizes measured so far, added up, and `None` before the first.
    pub total: Option<String>,
}

/// What a group's cards share: the parent whose fate decides theirs, which its line stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Parent {
    /// A git repository, by the git folder its worktrees share.
    Repository { common_dir: String },
    /// A Claude Code session, by its folder in the scratch root, which holds its scratch folders.
    Session { folder: String },
}

/// What stands under a group's line: its cards, or, while the group is folded shut, none, the line
/// itself showing the agents in those cards.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum GroupFold {
    /// The cards, nested under the line, a repository's main worktree first.
    Open { cards: Vec<Card> },
    /// Folded shut: at the line's end, a robot for each agent in the cards it holds, every session
    /// before every subagent, each in its card's order.
    Folded {
        sessions: Vec<AgentBadge>,
        subagents: Vec<AgentBadge>,
        /// One of those sessions waits on the user, so the line throbs as its card would.
        waiting: bool,
    },
}

/// One project's card.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Card {
    /// The project's folder, which actions on the card name. Text, as every APFS path is.
    pub path: String,
    /// The folder in full, with `~` for the home folder: shown while the pointer is over the card.
    pub location: String,
    /// The folder's name, or a scratch folder's session's; nested under its parent's line, a
    /// worktree's branch, or a scratch folder's own name.
    pub name: String,
    /// The folder, then when its last activity was and where it came from; nested, a worktree's
    /// branch in full before them.
    pub name_tooltip: String,
    /// The chip beside the name: a worktree's branch, or a scratch folder's place in its session's
    /// scratch. `None` for a plain folder, and for a card nested under its parent's line, which
    /// says what the chip would.
    pub chip: Option<Chip>,
    /// How long ago the last activity was, as `4m` reads.
    pub age: Option<String>,
    /// Why the folder cannot be read, above the units that sources outside it still show.
    pub unreadable: Option<Unreadable>,
    pub units: Vec<UnitLozenge>,
    /// The card's tray along its bottom: the folder's disk use, then the agents in it, sessions
    /// before subagents.
    pub tray: Option<Tray>,
    /// An agent in it waits on the user, so the card throbs.
    pub waiting: bool,
    /// What a right click offers.
    pub menu: Vec<MenuEntry>,
}

/// The chip beside a card's name, whose look says what it marks before its words do.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum Chip {
    /// A worktree's branch, a filled label: the branch, `detached abc1234` or `no commits`, and in
    /// its tooltip the branch in full and which worktree of which repository this is.
    Branch { text: String, tooltip: String },
    /// A scratch folder's place in its session's scratch.
    Scratch { mark: ScratchMark },
}

/// The mark of Claude Code's scratch space: outlined in dashes, behind a scribble, so that nothing
/// about it reads as a branch, which is kept, where scratch is thrown away. It fades, as a still
/// robot does, once its session has ended and will build there no more.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ScratchMark {
    /// Where the folder it marks lies in its session's folder, as `scratchpad/r567fixture` reads;
    /// `None` on the session's line, which marks the whole space.
    pub place: Option<String>,
    /// `Claude Code's scratch space`, and the session's whole id.
    pub tooltip: String,
    /// The session runs, and may build in its scratch again.
    pub session_runs: bool,
}

/// One agent, as one of the robots on its card's tray: the robot says which AI it is, and the light
/// of its face what it is doing. A session's face moves with its look's rhythm, a subagent's holds
/// still, and where each stands, before the tray's divider or after it, says which it is.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentBadge {
    pub robot: Robot,
    pub look: AgentLook,
    /// What the robot says only in its light, in words, then who the agent is. A session's: its
    /// status, with how long it has waited and how many of its subagents are at work; its name or
    /// id; its harness and the model it works with, as `Claude · claude-opus-5-5` reads. A
    /// subagent's: its status; its name and task, when its harness records them; what kind of
    /// agent it is, the model it works with and the session it works for, as `general-purpose ·
    /// claude-opus-5-5 · for Sutra launch` reads.
    pub tooltip: String,
    /// Opens the session in the app it runs in, on a click: for a subagent, the session it works
    /// for.
    pub link: Option<String>,
}

/// Which AI an agent is, as its robot shows it: by the model its transcript names, or, while none
/// is known, by the harness it runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Robot {
    Claude,
    /// An OpenAI model, as Codex runs.
    Codex,
    Gemini,
    /// A model with no robot of its own.
    Plain,
}

/// One line of the agents working outside every project, each with the folder it is about, with
/// `~` for the home folder.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ElsewhereLine {
    /// A session's robot beside the folder it works in, and at the line's end, as on a card's
    /// tray, the subagents at work in that same folder.
    Session {
        badge: AgentBadge,
        folder: String,
        subagents: Vec<AgentBadge>,
    },
    /// The subagents at work in a folder that no session on the row works in: their robots, then
    /// the folder.
    Subagents {
        subagents: Vec<AgentBadge>,
        folder: String,
    },
}

/// Why a card shows nothing from inside its folder, as `in Desktop, not watched` reads.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Unreadable {
    pub reason: String,
    pub look: UnreadableLook,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnreadableLook {
    /// Kept out by the user's own choice, in Settings, in the folder granted or in answer to
    /// macOS's prompt, or until they answer it: a quiet sign, since nothing went wrong.
    KeptOut,
    /// Missing, or locked by its own permissions: the warning sign.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentLook {
    /// Waiting on the user: an amber face, a session's throbbing with its card.
    Waiting,
    /// At work: a green face, a session's pulsing slowly.
    Working,
    /// Quiet mid-turn, idle, or with no status: the face unlit and the robot dimmed.
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

/// The card's tray, all on one line: the folder's disk use at its start, and at its end the agents
/// in the folder, its sessions, a divider, and the subagents at work in it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Tray {
    /// The folder's disk use, at the tray's start: `None` for a folder that cannot be read, which
    /// the card says why of, and while nothing about its size is known yet.
    pub disk: Option<DiskUse>,
    /// The sessions in the folder, in the map's order, so none moves when its status changes.
    pub sessions: Vec<AgentBadge>,
    /// The subagents at work in the folder, whichever session they work for: by session, each
    /// session's in its own order.
    pub subagents: Vec<AgentBadge>,
}

/// The folder's disk use, as `32 GB  target/  node_modules/` reads.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DiskUse {
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
    /// Fold shut the group of `parent`, as its line names it, or open it again. The fold is kept
    /// for the next run.
    ToggleFold {
        parent: Parent,
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

/// What shows under the list.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Footer {
    /// How far the first sizing has come, from 0 to 1, while it runs.
    pub sizing: Option<f32>,
    /// Sources whose failure loses only some folders or some changes, one line each, for an alert
    /// sign's tooltip.
    pub problems: Vec<String>,
    /// The private folders macOS keeps Sutra out of, and where the user can let it in, beside the
    /// quiet sign the cards in them show.
    pub refused: Option<String>,
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
