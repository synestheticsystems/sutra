//! The build folders a card's tray names after the folder's whole size, largest first: those large
//! enough to matter, those whose measurement failed, and those a clean is at work on or has left
//! large enough to show. Each has its size, or what a clean has left of it, in its tooltip; the
//! ones too small to name are in the tooltip of the total.

use std::cmp::Reverse;
use std::path::Path;

use crate::dashboard::cleaning::CleanState;
use crate::dashboard::format;
use crate::map::{ArtifactDir, Measurement, Project};

/// One entry of a tray's disk use after its total: a build folder, or on Docker's card one of the
/// stores Docker keeps on disk. Its name keeps the word from when every entry was a folder.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TrayFolder {
    /// As `target/` reads, or `volumes`.
    pub label: String,
    /// Its own size, and while cleaning, what is left of it.
    pub tooltip: String,
    /// A clean is at work on it.
    pub cleaning: bool,
}

/// One build folder on the tray.
#[derive(Debug, PartialEq)]
pub(super) enum Entry<'a> {
    /// At rest: measured, or its measurement failed, which its tooltip says.
    Folder(&'a ArtifactDir),
    /// A clean is at work on it, or is worked out, asked about or waiting its turn, holding `left`
    /// if its size was known.
    Cleaning {
        dir: &'a ArtifactDir,
        left: Option<u64>,
    },
    /// Cleaned and still large enough to show, holding `left`, with the map yet to measure it.
    Cleaned { dir: &'a ArtifactDir, left: u64 },
}

/// The least a measured build folder holds to earn a place on the tray, as one of Docker's stores
/// does on its card. A `node_modules` of 100 MB beside a 46 GB `target/` is only noise.
const WORTH_SHOWING_BYTES: u64 = 1 << 30;

pub(in crate::dashboard::contents) fn worth_showing(bytes: u64) -> bool {
    bytes >= WORTH_SHOWING_BYTES
}

pub(super) fn measured_bytes(dir: &ArtifactDir) -> u64 {
    dir.measurement.usage().map_or(0, |usage| usage.bytes)
}

/// The measured build folders that earn no place, for the size's tooltip to name.
pub(super) fn off_the_card(project: &Project) -> Vec<&ArtifactDir> {
    project
        .artifacts
        .iter()
        .filter(|dir| dir.measurement.usage().is_some())
        .filter(|dir| !worth_showing(measured_bytes(dir)))
        .collect()
}

/// The tray's entries, largest first: each build folder large enough to show, whose measurement
/// failed, or with a clean under way. One not yet measured is left out until it has a size, and one
/// just cleaned counts at what the clean left in it.
pub(super) fn entries<'a>(
    project: &'a Project,
    clean_state: impl Fn(&ArtifactDir) -> Option<CleanState>,
) -> Vec<Entry<'a>> {
    let mut entries: Vec<(u64, Entry<'a>)> = project
        .artifacts
        .iter()
        .filter_map(|dir| {
            let state = clean_state(dir);
            let bytes = state
                .and_then(CleanState::left)
                .unwrap_or_else(|| measured_bytes(dir));
            let entry = match (state, &dir.measurement) {
                (Some(CleanState::Deleting { .. }), _) => Some(Entry::Cleaning {
                    dir,
                    left: state.and_then(CleanState::left),
                }),
                (Some(CleanState::Deleted { left }), _) => {
                    worth_showing(left).then_some(Entry::Cleaned { dir, left })
                }
                (None, Measurement::Measured(usage)) => {
                    worth_showing(usage.bytes).then_some(Entry::Folder(dir))
                }
                (None, Measurement::Failed(_)) => Some(Entry::Folder(dir)),
                (None, Measurement::Pending | Measurement::Measuring) => None,
            };
            entry.map(|entry| (bytes, entry))
        })
        .collect();
    entries.sort_by_key(|(bytes, _)| Reverse(*bytes));
    entries.into_iter().map(|(_, entry)| entry).collect()
}

/// An entry: the folder's path, with its size in the tooltip.
pub(super) fn folder(entry: Entry, project: &Path, home: &Path) -> TrayFolder {
    let (dir, tooltip, cleaning) = match entry {
        Entry::Folder(dir) => (dir, artifact_tooltip(dir, home), false),
        Entry::Cleaning { dir, left } => (dir, cleaning_tooltip(dir, left, home), true),
        Entry::Cleaned { dir, left } => (dir, cleaning_tooltip(dir, Some(left), home), false),
    };
    TrayFolder {
        label: format!("{}/", format::artifact_name(dir, project)),
        tooltip,
        cleaning,
    }
}

/// The size's tooltip: the build folders too small to show on the card, a line each; `None` when
/// there are none.
pub(super) fn off_the_card_tooltip(folders: &[&ArtifactDir], project: &Path) -> Option<String> {
    let lines: Vec<String> = folders
        .iter()
        .filter_map(|dir| {
            let usage = dir.measurement.usage()?;
            Some(format!(
                "{}/ {}",
                format::artifact_name(dir, project),
                format::usage_size(usage)
            ))
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The tooltip of a build folder a clean is at work on or just finished with: what it still holds,
/// if its size was known, then where it is.
fn cleaning_tooltip(dir: &ArtifactDir, left: Option<u64>, home: &Path) -> String {
    let path = format::tilde(&dir.path, home);
    match left {
        Some(left) => format!("{}\n{path}", format::size(left, false)),
        None => path,
    }
}

/// A build folder's tooltip: its size, or why it has none, then where it is, and how many folders
/// inside it could not be read.
fn artifact_tooltip(dir: &ArtifactDir, home: &Path) -> String {
    let path = format::tilde(&dir.path, home);
    match &dir.measurement {
        Measurement::Failed(kind) => format!("Couldn't measure: {kind}\n{path}"),
        Measurement::Measured(usage) if usage.unreadable_dirs > 0 => format!(
            "{}\n{path}\n{} inside could not be read",
            format::usage_size(usage),
            format::plural(usage.unreadable_dirs, "folder")
        ),
        Measurement::Measured(usage) => format!("{}\n{path}", format::usage_size(usage)),
        Measurement::Pending | Measurement::Measuring => path,
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::super::tests::project;
    use super::*;
    use crate::map::DirUsage;
    use crate::map::test_support::{cargo_target, measured, node_modules, usage};

    const HOME: &str = "/Users/dev";

    #[test]
    fn large_build_folders_earn_a_place_on_the_tray() {
        let app = project(vec![
            cargo_target("/code/app/target", measured(12_300_000_000)),
            node_modules("/code/app/web/node_modules", measured(1_200_000_000)),
        ]);
        let entries = entries(&app, |_| None);
        assert_eq!(
            entries,
            [
                Entry::Folder(&app.artifacts[0]),
                Entry::Folder(&app.artifacts[1]),
            ]
        );
    }

    #[test]
    fn the_largest_build_folder_comes_first() {
        let app = project(vec![
            node_modules("/code/app/node_modules", measured(1_500_000_000)),
            cargo_target("/code/app/target", measured(46_400_000_000)),
        ]);
        assert_eq!(
            entries(&app, |_| None),
            [
                Entry::Folder(&app.artifacts[1]),
                Entry::Folder(&app.artifacts[0]),
            ]
        );
    }

    #[test]
    fn a_folder_too_small_to_matter_leaves_the_tray_for_the_tooltip() {
        let app = project(vec![
            node_modules("/code/app/ui/node_modules", measured(168_100_000)),
            cargo_target("/code/app/target", measured(3_000_000)),
        ]);
        assert_eq!(entries(&app, |_| None), []);
        assert_eq!(off_the_card(&app), [&app.artifacts[0], &app.artifacts[1]]);
    }

    #[test]
    fn a_folder_not_yet_measured_waits_off_the_card_and_a_failed_one_says_so() {
        let failed = Measurement::Failed(io::ErrorKind::PermissionDenied);
        let app = project(vec![
            cargo_target("/code/app/target", Measurement::Measuring),
            cargo_target("/code/app/old/target", failed),
        ]);
        assert_eq!(entries(&app, |_| None), [Entry::Folder(&app.artifacts[1])]);
    }

    #[test]
    fn a_folder_being_cleaned_keeps_its_place_on_the_tray_and_says_so() {
        let app = project(vec![cargo_target(
            "/code/app/target",
            measured(12_300_000_000),
        )]);
        let state = CleanState::Deleting {
            before: Some(12_000_000_000),
            freed: 8_000_000_000,
        };
        assert_eq!(
            entries(&app, |_| Some(state)),
            [Entry::Cleaning {
                dir: &app.artifacts[0],
                left: Some(4_000_000_000)
            }]
        );
    }

    #[test]
    fn a_folder_just_cleaned_leaves_the_tray_unless_it_still_holds_enough_to_show() {
        let app = project(vec![cargo_target(
            "/code/app/target",
            measured(12_300_000_000),
        )]);
        let cleaned = |left| move |_: &ArtifactDir| Some(CleanState::Deleted { left });
        assert_eq!(entries(&app, cleaned(4096)), []);
        assert_eq!(
            entries(&app, cleaned(2_000_000_000)),
            [Entry::Cleaned {
                dir: &app.artifacts[0],
                left: 2_000_000_000
            }]
        );
    }

    #[test]
    fn a_partly_read_folders_tooltip_gives_a_lower_bound_and_how_many_folders_it_missed() {
        let partly_read = DirUsage {
            unreadable_dirs: 3,
            ..usage(5_000_000)
        };
        let partial = cargo_target("/code/app/target", Measurement::Measured(partly_read));
        assert_eq!(
            artifact_tooltip(&partial, Path::new(HOME)),
            "\u{2265}5 MB\n/code/app/target\n3 folders inside could not be read"
        );
    }

    #[test]
    fn a_folder_whose_measurement_failed_says_why_in_its_tooltip() {
        let failed = cargo_target(
            "/Users/dev/app/target",
            Measurement::Failed(io::ErrorKind::PermissionDenied),
        );
        assert_eq!(
            artifact_tooltip(&failed, Path::new(HOME)),
            "Couldn't measure: permission denied\n~/app/target"
        );
    }

    #[test]
    fn a_folder_being_cleaned_says_in_its_tooltip_what_it_still_holds() {
        let dir = cargo_target("/Users/dev/app/target", measured(12_000_000_000));
        assert_eq!(
            cleaning_tooltip(&dir, Some(4_000_000_000), Path::new(HOME)),
            "4 GB\n~/app/target"
        );
        assert_eq!(
            cleaning_tooltip(&dir, None, Path::new(HOME)),
            "~/app/target"
        );
    }
}
