//! A card's disk tray, all on one line, as `32 GB  target/  node_modules/` reads: the folder's whole
//! size, then the build folders large enough to matter, largest first. Each folder's own size is in
//! its tooltip, and the ones too small to name are in the total's. Cleaning is in the card's
//! right-click menu, and a bar along the top of the tray shows how far a clean has come.

use std::cmp::Reverse;
use std::path::Path;

use crate::dashboard::cleaning::CleanState;
use crate::dashboard::{Tray, TrayFolder, format};
use crate::map::{ArtifactDir, FolderAccess, FolderSize, Measurement, Project};

/// What goes on the tray, `None` while nothing about the folder's size is known yet: the bar under
/// the list shows that sizing is still running. `clean_state` says what a clean means for each
/// build folder.
pub(super) fn tray(
    project: &Project,
    clean_state: impl Fn(&ArtifactDir) -> Option<CleanState>,
    home: &Path,
) -> Option<Tray> {
    // The card already says why a folder that cannot be read has no size.
    if project.access != FolderAccess::Readable {
        return None;
    }
    let entries = entries(project, &clean_state);
    let total = total_on_card(project, &clean_state);
    let warning = own_files_failure(project);
    if total.is_none() && entries.is_empty() && warning.is_none() {
        return None;
    }
    Some(Tray {
        total: total.map(|total| format::size(total.bytes, total.partial)),
        // The total's tooltip names what it holds besides the folders shown, so it has nothing to
        // say without a total.
        total_tooltip: total
            .and_then(|_| off_the_card_tooltip(&off_the_card(project), &project.path)),
        warning,
        folders: entries
            .into_iter()
            .map(|entry| folder(entry, &project.path, home))
            .collect(),
        progress: clean_progress(project, &clean_state),
    })
}

/// The folder's whole size as the card shows it: what the map measured, with each build folder a
/// clean is at work on counted at what the clean has left of it. The map only learns that from its
/// next walk, and a walk landing during the clean changes nothing here.
fn total_on_card(
    project: &Project,
    clean_state: impl Fn(&ArtifactDir) -> Option<CleanState>,
) -> Option<FolderSize> {
    let mut total = project.folder_size()?;
    for dir in &project.artifacts {
        if let Some(left) = clean_state(dir).and_then(CleanState::left) {
            total.bytes = total.bytes.saturating_sub(measured_bytes(dir)) + left;
        }
    }
    Some(total)
}

/// How far the clean of `project`'s build folders has come, from 0 to 1, by the space given back:
/// `None` when no clean is at work on them. One still being worked out, asked about or waiting its
/// turn, or whose size was not known, has come no way, so its bar shows empty.
fn clean_progress(
    project: &Project,
    clean_state: impl Fn(&ArtifactDir) -> Option<CleanState>,
) -> Option<f32> {
    let cleaning: Vec<(u64, u64)> = project
        .artifacts
        .iter()
        .filter_map(|dir| match clean_state(dir) {
            Some(CleanState::Deleting { before, freed }) => Some((before.unwrap_or(0), freed)),
            _ => None,
        })
        .collect();
    if cleaning.is_empty() {
        return None;
    }
    let before: u64 = cleaning.iter().map(|(before, _)| before).sum();
    let freed: u64 = cleaning.iter().map(|(_, freed)| freed).sum();
    Some(if before == 0 {
        0.0
    } else {
        (freed as f64 / before as f64).min(1.0) as f32
    })
}

/// Why the folder has no total, when measuring its own files failed.
fn own_files_failure(project: &Project) -> Option<String> {
    match project.own_files {
        Measurement::Failed(kind) => Some(format!("Couldn't size this folder: {kind}")),
        _ => None,
    }
}

/// One build folder on the tray.
#[derive(Debug, PartialEq)]
enum Entry<'a> {
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

/// The least a measured build folder holds to earn a place on the tray. A `node_modules` of 100 MB
/// beside a 46 GB `target/` is only noise.
const WORTH_SHOWING_BYTES: u64 = 1 << 30;

fn worth_showing(bytes: u64) -> bool {
    bytes >= WORTH_SHOWING_BYTES
}

fn measured_bytes(dir: &ArtifactDir) -> u64 {
    dir.measurement.usage().map_or(0, |usage| usage.bytes)
}

/// The measured build folders that earn no place, for the size's tooltip to name.
fn off_the_card(project: &Project) -> Vec<&ArtifactDir> {
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
fn entries<'a>(
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
fn folder(entry: Entry, project: &Path, home: &Path) -> TrayFolder {
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
fn off_the_card_tooltip(folders: &[&ArtifactDir], project: &Path) -> Option<String> {
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

    use std::path::PathBuf;

    use super::*;
    use crate::map::DirUsage;
    use crate::map::test_support::{cargo_target, measured, node_modules, project_at, usage};

    const HOME: &str = "/Users/dev";

    fn project(artifacts: Vec<ArtifactDir>) -> Project {
        Project {
            artifacts,
            ..project_at("/code/app")
        }
    }

    #[test]
    fn the_tray_shows_the_total_then_the_large_folders_and_marks_the_one_being_cleaned() {
        let mut app = project(vec![
            cargo_target("/Users/dev/app/target", measured(12_000_000_000)),
            node_modules("/Users/dev/app/node_modules", measured(2_000_000_000)),
            node_modules("/Users/dev/app/ui/node_modules", measured(5_000_000)),
        ]);
        app.path = PathBuf::from("/Users/dev/app");
        app.own_files = measured(1_000_000_000);
        let cleaning_target = |dir: &ArtifactDir| {
            dir.path
                .ends_with("target")
                .then_some(CleanState::Deleting {
                    before: Some(12_000_000_000),
                    freed: 3_000_000_000,
                })
        };

        let tray = tray(&app, cleaning_target, Path::new(HOME)).unwrap();

        assert_eq!(tray.total.as_deref(), Some("12 GB"));
        assert_eq!(tray.total_tooltip.as_deref(), Some("ui/node_modules/ 5 MB"));
        assert_eq!(tray.warning, None);
        let folders: Vec<(&str, bool)> = tray
            .folders
            .iter()
            .map(|folder| (folder.label.as_str(), folder.cleaning))
            .collect();
        assert_eq!(folders, [("target/", true), ("node_modules/", false)]);
        assert_eq!(tray.folders[0].tooltip, "9 GB\n~/app/target");
        assert_eq!(tray.progress, Some(0.25));
    }

    #[test]
    fn a_folder_that_cannot_be_read_or_has_nothing_sized_yet_has_no_tray() {
        let not_sized = project(vec![cargo_target(
            "/code/app/target",
            Measurement::Measuring,
        )]);
        assert_eq!(tray(&not_sized, |_| None, Path::new(HOME)), None);
        let denied = Project {
            access: FolderAccess::Denied,
            own_files: measured(1_000),
            ..project(Vec::new())
        };
        assert_eq!(tray(&denied, |_| None, Path::new(HOME)), None);
    }

    #[test]
    fn a_folder_whose_own_files_could_not_be_measured_says_why() {
        let mut app = project(Vec::new());
        assert_eq!(own_files_failure(&app), None);
        app.own_files = Measurement::Failed(io::ErrorKind::PermissionDenied);
        assert_eq!(
            own_files_failure(&app).as_deref(),
            Some("Couldn't size this folder: permission denied")
        );
    }

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
    fn the_bar_shows_how_much_of_what_is_being_cleaned_has_gone() {
        let app = project(vec![
            cargo_target("/code/app/target", measured(12_000_000_000)),
            cargo_target("/code/app/other", measured(4_000_000_000)),
        ]);
        assert_eq!(clean_progress(&app, |_| None), None, "no clean at work");

        let deleting = |target_freed: u64, other_freed: u64| {
            move |dir: &ArtifactDir| {
                let (before, freed) = if dir.path.ends_with("target") {
                    (12_000_000_000, target_freed)
                } else {
                    (4_000_000_000, other_freed)
                };
                Some(CleanState::Deleting {
                    before: Some(before),
                    freed,
                })
            }
        };
        assert_eq!(clean_progress(&app, deleting(0, 0)), Some(0.0), "waiting");
        assert_eq!(
            clean_progress(&app, deleting(8_000_000_000, 0)),
            Some(0.5),
            "both folders count"
        );
        assert_eq!(
            clean_progress(&app, deleting(13_000_000_000, 4_000_000_000)),
            Some(1.0),
            "never past the end, whatever the count says"
        );

        let size_unknown = |_: &ArtifactDir| {
            Some(CleanState::Deleting {
                before: None,
                freed: 5,
            })
        };
        assert_eq!(clean_progress(&app, size_unknown), Some(0.0));
        let finished = |_: &ArtifactDir| Some(CleanState::Deleted { left: 4096 });
        assert_eq!(clean_progress(&app, finished), None, "nothing left to show");
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
    fn the_total_falls_with_what_a_clean_has_left_whatever_the_map_measured_meanwhile() {
        let mut app = project(vec![
            cargo_target("/code/app/target", measured(12_000_000_000)),
            node_modules("/code/app/node_modules", measured(1_000_000_000)),
        ]);
        app.own_files = measured(500_000_000);
        let total = |clean_state: &dyn Fn(&ArtifactDir) -> Option<CleanState>| {
            total_on_card(&app, clean_state).map(|total| total.bytes)
        };
        assert_eq!(total(&|_| None), Some(13_500_000_000));

        let being_cleaned = |before: Option<u64>| {
            move |dir: &ArtifactDir| {
                (dir.path.ends_with("target")).then_some(CleanState::Deleting {
                    before,
                    freed: 7_000_000_000,
                })
            }
        };
        assert_eq!(
            total(&being_cleaned(Some(12_000_000_000))),
            Some(6_500_000_000)
        );
        assert_eq!(
            total(&being_cleaned(None)),
            Some(13_500_000_000),
            "with no size to start from, the map's stands"
        );

        // A walk lands during the clean and finds part of the folder gone.
        app.artifacts[0].measurement = measured(9_000_000_000);
        let total = total_on_card(&app, being_cleaned(Some(12_000_000_000)));
        assert_eq!(total.map(|total| total.bytes), Some(6_500_000_000));

        let total = total_on_card(&app, |dir: &ArtifactDir| {
            (dir.path.ends_with("target")).then_some(CleanState::Deleted { left: 4096 })
        });
        assert_eq!(total.map(|total| total.bytes), Some(1_500_004_096));
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
