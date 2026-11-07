//! What the Settings window shows of the choices the dashboard keeps for the user: how much free
//! space Sutra keeps by cleaning by itself, and which of the folders macOS guards it watches. The
//! dashboard's state works it out (`State::contents`).

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
