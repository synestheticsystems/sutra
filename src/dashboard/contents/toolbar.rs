//! The toolbar: the filter field while ⌘F shows it, the sort, the clean of idle build folders, the
//! sound and notification toggles, in the store edition the folder Sutra watches, and the gauge of
//! how full the disk is.

mod gauge;

pub use gauge::{DiskGauge, GaugeLook};

pub(in crate::dashboard) use gauge::gauge as disk_gauge;

use std::path::{Path, PathBuf};

use crate::dashboard::cleaning::{self, AtRestOffer, IdleOffer};
use crate::dashboard::listing::{Arrangement, Listing};
use crate::dashboard::{Action, MenuAction, MenuEntry, SortOrder, format};
use crate::notifications::Notifier;

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
    /// How full the disk holding the home folder is, or why it could not be read; `None` until it
    /// is first read.
    pub disk: Option<DiskGauge>,
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

/// The toolbar, with `listing` `None` until the first map arrives, `clean` as `clean_control`
/// works it out, and `disk` as `disk_gauge` does, `None` before the disk's first reading.
pub(in crate::dashboard) fn toolbar(
    listing: Option<&Listing>,
    arrangement: &Arrangement,
    clean: CleanControl,
    notifier: &Notifier,
    sandboxed: bool,
    home: &Path,
    disk: Option<DiskGauge>,
) -> Toolbar {
    let map = listing.map(|listing| listing.snapshot);
    Toolbar {
        // Whether the sort control shows depends on every project, not on the ones a filter
        // shows, so it does not jump away while the user types.
        sort: map
            .filter(|map| map.projects.len() > 1)
            .map(|_| sort_control(arrangement.sort())),
        filter: arrangement.filter().map(str::to_string),
        clean,
        muted: notifier.global_mute,
        mute_tooltip: if notifier.global_mute {
            "Unmute all sounds"
        } else {
            "Mute all sounds"
        }
        .to_string(),
        notifications_off: notifier.global_notifications_off,
        notifications_tooltip: if notifier.global_notifications_off {
            "Enable notifications"
        } else {
            "Disable notifications"
        }
        .to_string(),
        // The store edition sees only the folder the user granted, and the user may change it.
        folder: map
            .filter(|_| sandboxed)
            .map(|map| watching(&map.roots, &map.skipped_private_folders, home)),
        disk,
    }
}

/// The toolbar's clean: a click cleans every idle build folder, then asks apart about those kept
/// only by sessions at rest, and a right click sets what Sutra keeps free. `offer` is what is idle
/// and `at_rest` what sessions at rest keep, if anything, when the map `can_tell`; `busy` while a
/// click's clean is still worked out; `floor` the free space Sutra keeps, if the user chose one.
pub(in crate::dashboard) fn clean_control(
    offer: Option<IdleOffer>,
    at_rest: Option<AtRestOffer>,
    can_tell: bool,
    busy: bool,
    floor: Option<u64>,
) -> CleanControl {
    CleanControl {
        offered: offer.is_some() || at_rest.is_some(),
        busy,
        tooltip: offer_tooltip(offer, at_rest, can_tell, floor),
        menu: clean_menu(offer, at_rest, floor),
    }
}

/// The toolbar's words for what is on offer, when `can_tell` what is idle, for what it asks about
/// apart, and for what Sutra keeps free.
fn offer_tooltip(
    offer: Option<IdleOffer>,
    at_rest: Option<AtRestOffer>,
    can_tell: bool,
    floor: Option<u64>,
) -> String {
    let at_rest = at_rest.map(|AtRestOffer { folders, bytes }| {
        let which = match folders {
            1 => "the build folder".to_string(),
            folders => format!("{folders} build folders"),
        };
        format!(
            "Ask about {which} kept by sessions at rest: {}",
            format::size(bytes, false)
        )
    });
    let offer = match offer {
        Some(IdleOffer { folders: 1, bytes }) => Some(format!(
            "Clean the idle build folder: {}",
            format::size(bytes, false)
        )),
        Some(IdleOffer { folders, bytes }) => Some(format!(
            "Clean {folders} idle build folders: {}",
            format::size(bytes, false)
        )),
        // What sessions at rest keep is then all a click asks about.
        None if at_rest.is_some() => None,
        None if can_tell => Some("No idle build folders to clean".to_string()),
        None => Some("Can't tell what is idle right now".to_string()),
    };
    let floor = floor.map(|floor| format!("Keeping {} free", format::size(floor, false)));
    let lines: Vec<String> = [offer, at_rest, floor].into_iter().flatten().collect();
    lines.join("\n")
}

/// What a right click on the toolbar's clean offers: the clean itself when anything is idle or
/// kept by sessions at rest, titled by the idle folders when there are any, as its first question
/// asks about them, then how much free space Sutra should keep by itself, with the current choice
/// checked.
fn clean_menu(
    offer: Option<IdleOffer>,
    at_rest: Option<AtRestOffer>,
    floor: Option<u64>,
) -> Vec<MenuEntry> {
    let mut menu = Vec::new();
    let title = match (offer, at_rest) {
        (Some(IdleOffer { folders, bytes }), _) => Some(format!(
            "Clean {} ({})\u{2026}",
            format::plural(folders as u64, "Idle Folder"),
            format::size(bytes, false)
        )),
        (None, Some(AtRestOffer { folders, bytes })) => Some(format!(
            "Clean {} Kept by Sessions at Rest ({})\u{2026}",
            format::plural(folders as u64, "Folder"),
            format::size(bytes, false)
        )),
        (None, None) => None,
    };
    if let Some(title) = title {
        menu.push(MenuEntry::Item {
            title,
            checked: false,
            action: MenuAction::Dashboard {
                action: Action::CleanIdle,
            },
        });
        menu.push(MenuEntry::Separator);
    }
    menu.extend(
        cleaning::keep_free_choices()
            .into_iter()
            .map(|choice| MenuEntry::Item {
                title: choice.label,
                checked: floor == choice.bytes,
                action: MenuAction::Dashboard {
                    action: Action::KeepFree {
                        bytes: choice.bytes,
                    },
                },
            }),
    );
    menu
}

/// `Watching ~ (not Desktop or Downloads)`: the roots, and the private folders inside them that
/// Settings keeps the scan out of.
fn watching(roots: &[PathBuf], skipped: &[PathBuf], home: &Path) -> String {
    let watched = format::roots(roots, home);
    let names: Vec<String> = skipped
        .iter()
        .filter_map(|folder| folder.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    if names.is_empty() {
        return format!("Watching {watched}");
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    format!("Watching {watched} (not {})", format::either(&names))
}

/// One choice per order, the current one marked.
fn sort_control(current: SortOrder) -> SortControl {
    SortControl {
        current,
        choices: SortOrder::ALL
            .map(|order| SortChoice {
                order,
                label: order.label().to_string(),
                tooltip: order.tooltip().to_string(),
            })
            .to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::listing::fixtures::{now, project, snapshot};

    fn titles(menu: &[MenuEntry]) -> Vec<(Option<String>, bool)> {
        menu.iter()
            .map(|entry| match entry {
                MenuEntry::Item { title, checked, .. } => (Some(title.clone()), *checked),
                MenuEntry::Submenu { title, .. } => (Some(title.clone()), false),
                MenuEntry::Separator => (None, false),
            })
            .collect()
    }

    #[test]
    fn the_clean_menu_offers_the_clean_when_anything_is_idle_and_checks_what_is_kept_free() {
        let offer = IdleOffer {
            folders: 12,
            bytes: 282_000_000_000,
        };
        let menu = clean_menu(Some(offer), None, Some(100_000_000_000));
        let title = |title: &str, checked| (Some(title.to_string()), checked);
        assert_eq!(
            titles(&menu),
            [
                title("Clean 12 Idle Folders (282 GB)\u{2026}", false),
                (None, false),
                title("Never Clean by Itself", false),
                title("Keep 25 GB Free", false),
                title("Keep 50 GB Free", false),
                title("Keep 100 GB Free", true),
                title("Keep 200 GB Free", false),
            ]
        );
        let action = |entry: &MenuEntry| match entry {
            MenuEntry::Item {
                action: MenuAction::Dashboard { action },
                ..
            } => action.clone(),
            entry => panic!("not the dashboard's: {entry:?}"),
        };
        assert_eq!(action(&menu[0]), Action::CleanIdle);
        assert_eq!(
            action(&menu[5]),
            Action::KeepFree {
                bytes: Some(100_000_000_000)
            }
        );
    }

    #[test]
    fn with_nothing_idle_the_clean_menu_only_sets_what_is_kept_free() {
        let menu = clean_menu(None, None, None);
        assert_eq!(
            titles(&menu)[0],
            (Some("Never Clean by Itself".to_string()), true)
        );
        assert_eq!(menu.len(), 5);
    }

    #[test]
    fn the_toolbar_says_what_is_on_offer_and_what_is_kept_free() {
        assert_eq!(
            offer_tooltip(None, None, true, None),
            "No idle build folders to clean"
        );
        assert_eq!(
            offer_tooltip(None, None, false, None),
            "Can't tell what is idle right now"
        );
        let one = IdleOffer {
            folders: 1,
            bytes: 4_000_000_000,
        };
        assert_eq!(
            offer_tooltip(Some(one), None, true, None),
            "Clean the idle build folder: 4 GB"
        );
        let many = IdleOffer {
            folders: 12,
            bytes: 282_000_000_000,
        };
        assert_eq!(
            offer_tooltip(Some(many), None, true, Some(100_000_000_000)),
            "Clean 12 idle build folders: 282 GB\nKeeping 100 GB free"
        );
    }

    #[test]
    fn the_toolbar_says_what_it_asks_about_apart_after_what_is_idle_or_alone() {
        let idle = IdleOffer {
            folders: 12,
            bytes: 282_000_000_000,
        };
        let at_rest = |folders| AtRestOffer {
            folders,
            bytes: 180_000_000_000,
        };
        assert_eq!(
            offer_tooltip(Some(idle), Some(at_rest(3)), true, Some(100_000_000_000)),
            "Clean 12 idle build folders: 282 GB\nAsk about 3 build folders kept by sessions at \
             rest: 180 GB\nKeeping 100 GB free"
        );
        assert_eq!(
            offer_tooltip(None, Some(at_rest(1)), true, None),
            "Ask about the build folder kept by sessions at rest: 180 GB"
        );

        let control = clean_control(None, Some(at_rest(3)), true, false, None);
        assert!(control.offered, "a click asks about them");
        assert_eq!(
            titles(&control.menu)[0],
            (
                Some("Clean 3 Folders Kept by Sessions at Rest (180 GB)\u{2026}".to_string()),
                false
            )
        );
        let both = clean_menu(Some(idle), Some(at_rest(3)), None);
        assert_eq!(
            titles(&both)[0],
            (
                Some("Clean 12 Idle Folders (282 GB)\u{2026}".to_string()),
                false
            ),
            "titled by the first question"
        );
    }

    #[test]
    fn watching_names_the_private_folders_it_skips_and_nothing_when_there_are_none() {
        let home = Path::new("/Users/dev");
        let private: Vec<PathBuf> = ["Desktop", "Documents", "Downloads"]
            .iter()
            .map(|name| home.join(name))
            .collect();
        assert_eq!(
            watching(&[home.to_path_buf()], &private, home),
            "Watching ~ (not Desktop, Documents or Downloads)"
        );
        assert_eq!(watching(&[home.join("code")], &[], home), "Watching ~/code");
    }

    #[test]
    fn the_sort_shows_with_more_than_one_project_and_the_folder_only_in_the_store_edition() {
        let notifier = Notifier::new(|_, _| {});
        let clean = clean_control(None, None, true, false, None);
        let arrangement = Arrangement::default();
        let toolbar_over = |map, sandboxed| {
            let listing = Listing::new(map, &arrangement, &cleaning::tests::cleaning(), now());
            let home = Path::new("/Users/dev");
            toolbar(
                Some(&listing),
                &arrangement,
                clean.clone(),
                &notifier,
                sandboxed,
                home,
                None,
            )
        };
        let one = snapshot(vec![project("a", 1)], vec![]);
        let two = snapshot(vec![project("a", 1), project("b", 2)], vec![]);

        assert_eq!(toolbar_over(&one, false).sort, None);
        let sort = toolbar_over(&two, false).sort.unwrap();
        assert_eq!(sort.current, SortOrder::Activity);
        let labels: Vec<&str> = sort.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Activity", "Name", "Disk"]);
        assert_eq!(toolbar_over(&two, false).folder, None);
        assert_eq!(
            toolbar_over(&two, true).folder.as_deref(),
            Some("Watching /code")
        );
    }
}
