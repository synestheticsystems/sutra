//! The toolbar: the filter field while ⌘F shows it, the sort, the clean of idle build folders, the
//! sound and notification toggles, and in the store edition the folder Sutra watches.

use std::path::{Path, PathBuf};

use crate::dashboard::cleaning::{self, IdleOffer};
use crate::dashboard::listing::{Arrangement, Listing};
use crate::dashboard::{
    Action, CleanControl, MenuAction, MenuEntry, SortChoice, SortControl, SortOrder, Toolbar,
    format,
};
use crate::notifications::Notifier;

/// The toolbar, with `listing` `None` until the first map arrives, and `clean` as
/// `clean_control` works it out.
pub(in crate::dashboard) fn toolbar(
    listing: Option<&Listing>,
    arrangement: &Arrangement,
    clean: CleanControl,
    notifier: &Notifier,
    sandboxed: bool,
    home: &Path,
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
    }
}

/// The toolbar's clean: a click cleans every idle build folder, and a right click sets what Sutra
/// keeps free. `offer` is what is idle, if anything, when the map `can_tell`; `busy` while a
/// click's clean is still worked out; `floor` the free space Sutra keeps, if the user chose one.
pub(in crate::dashboard) fn clean_control(
    offer: Option<IdleOffer>,
    can_tell: bool,
    busy: bool,
    floor: Option<u64>,
) -> CleanControl {
    CleanControl {
        offered: offer.is_some(),
        busy,
        tooltip: offer_tooltip(offer, can_tell, floor),
        menu: clean_menu(offer, floor),
    }
}

/// The toolbar's words for what is on offer, when `can_tell` what is idle, and for what Sutra keeps
/// free.
fn offer_tooltip(offer: Option<IdleOffer>, can_tell: bool, floor: Option<u64>) -> String {
    let offer = match offer {
        Some(IdleOffer { folders: 1, bytes }) => {
            format!(
                "Clean the idle build folder: {}",
                format::size(bytes, false)
            )
        }
        Some(IdleOffer { folders, bytes }) => {
            format!(
                "Clean {folders} idle build folders: {}",
                format::size(bytes, false)
            )
        }
        None if can_tell => "No idle build folders to clean".to_string(),
        None => "Can't tell what is idle right now".to_string(),
    };
    match floor {
        Some(floor) => format!("{offer}\nKeeping {} free", format::size(floor, false)),
        None => offer,
    }
}

/// What a right click on the toolbar's clean offers: the clean itself when anything is idle, then
/// how much free space Sutra should keep by itself, with the current choice checked.
fn clean_menu(offer: Option<IdleOffer>, floor: Option<u64>) -> Vec<MenuEntry> {
    let mut menu = Vec::new();
    if let Some(IdleOffer { folders, bytes }) = offer {
        menu.push(MenuEntry::Item {
            title: format!(
                "Clean {} ({})\u{2026}",
                format::plural(folders as u64, "Idle Folder"),
                format::size(bytes, false)
            ),
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
        let menu = clean_menu(Some(offer), Some(100_000_000_000));
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
        let menu = clean_menu(None, None);
        assert_eq!(
            titles(&menu)[0],
            (Some("Never Clean by Itself".to_string()), true)
        );
        assert_eq!(menu.len(), 5);
    }

    #[test]
    fn the_toolbar_says_what_is_on_offer_and_what_is_kept_free() {
        assert_eq!(
            offer_tooltip(None, true, None),
            "No idle build folders to clean"
        );
        assert_eq!(
            offer_tooltip(None, false, None),
            "Can't tell what is idle right now"
        );
        let one = IdleOffer {
            folders: 1,
            bytes: 4_000_000_000,
        };
        assert_eq!(
            offer_tooltip(Some(one), true, None),
            "Clean the idle build folder: 4 GB"
        );
        let many = IdleOffer {
            folders: 12,
            bytes: 282_000_000_000,
        };
        assert_eq!(
            offer_tooltip(Some(many), true, Some(100_000_000_000)),
            "Clean 12 idle build folders: 282 GB\nKeeping 100 GB free"
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
        let clean = clean_control(None, true, false, None);
        let arrangement = Arrangement::default();
        let toolbar_over = |map, sandboxed| {
            let listing = Listing::new(map, &arrangement, now());
            let home = Path::new("/Users/dev");
            toolbar(
                Some(&listing),
                &arrangement,
                clean.clone(),
                &notifier,
                sandboxed,
                home,
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
