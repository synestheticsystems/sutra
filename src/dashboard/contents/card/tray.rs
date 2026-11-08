//! A card's tray, all on one line: the folder's disk use at its start, and at its end the agents in
//! the folder, sessions before subagents. The disk use, as `32 GB  target/  node_modules/` reads,
//! is the folder's whole size, then the build folders large enough to matter, largest first (see
//! `folders`). Each folder's own size is in its tooltip, and the ones too small to name are in the
//! total's. Cleaning is in the card's right-click menu, and a bar along the top of the tray shows
//! how far a clean has come.

mod folders;

pub use folders::TrayEntry;
pub(in crate::dashboard::contents) use folders::worth_showing;

use std::path::Path;

use crate::dashboard::cleaning::CleanState;
use crate::dashboard::{AgentBadge, format};
use crate::map::{ArtifactDir, FolderAccess, FolderSize, Measurement, Project};
use folders::{build_folders, entry, measured_bytes, off_the_card, off_the_card_tooltip};

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
    /// The build folders too small to name, for the total's tooltip; on Docker's card, its stores
    /// too small to name, and when Docker said what it takes.
    pub total_tooltip: Option<String>,
    /// Why the folder has no total, when measuring its own files failed: an alert sign's tooltip.
    pub warning: Option<String>,
    /// The build folders large enough to matter, largest first; on Docker's card, its stores.
    pub entries: Vec<TrayEntry>,
    /// How far the clean of its build folders has come, from 0 to 1, by the space given back:
    /// the bar along the seam above the tray. Empty while a clean is worked out or waits its turn.
    pub progress: Option<f32>,
}

/// The tray of `project`, with its `sessions` and `subagents` at its end; `None` while it would
/// hold nothing: no agent, and nothing about the folder's size known yet, which the bar under the
/// list shows is still being worked out. `clean_state` says what a clean means for each build
/// folder.
pub(super) fn tray(
    project: &Project,
    sessions: Vec<AgentBadge>,
    subagents: Vec<AgentBadge>,
    clean_state: impl Fn(&ArtifactDir) -> Option<CleanState>,
    home: &Path,
) -> Option<Tray> {
    let disk = disk_use(project, clean_state, home);
    let holds_any = disk.is_some() || !sessions.is_empty() || !subagents.is_empty();
    holds_any.then_some(Tray {
        disk,
        sessions,
        subagents,
    })
}

/// The folder's disk use, `None` while nothing about its size is known yet.
fn disk_use(
    project: &Project,
    clean_state: impl Fn(&ArtifactDir) -> Option<CleanState>,
    home: &Path,
) -> Option<DiskUse> {
    // The card already says why a folder that cannot be read has no size.
    if project.access != FolderAccess::Readable {
        return None;
    }
    let folders = build_folders(project, &clean_state);
    let total = total_on_card(project, &clean_state);
    let warning = own_files_failure(project);
    if total.is_none() && folders.is_empty() && warning.is_none() {
        return None;
    }
    Some(DiskUse {
        total: total.map(|total| format::size(total.bytes, total.partial)),
        // The total's tooltip names what it holds besides the folders shown, so it has nothing to
        // say without a total.
        total_tooltip: total
            .and_then(|_| off_the_card_tooltip(&off_the_card(project), &project.path)),
        warning,
        entries: folders
            .into_iter()
            .map(|folder| entry(folder, &project.path, home))
            .collect(),
        progress: clean_progress(project, &clean_state),
    })
}

/// The folder's whole size as the card shows it: what the map measured, with each build folder a
/// clean is at work on counted at what the clean has left of it. The map only learns that from its
/// next walk, and a walk landing during the clean changes nothing here.
pub(super) fn total_on_card(
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

#[cfg(test)]
mod tests {
    use std::io;

    use std::path::PathBuf;

    use super::*;
    use crate::dashboard::{AgentLook, Robot};
    use crate::map::test_support::{cargo_target, measured, node_modules, project_at};

    const HOME: &str = "/Users/dev";

    /// The project at `/code/app`, holding the build folders `artifacts`.
    pub(super) fn project(artifacts: Vec<ArtifactDir>) -> Project {
        Project {
            artifacts,
            ..project_at("/code/app")
        }
    }

    #[test]
    fn the_disk_use_is_the_total_then_the_large_folders_and_marks_the_one_being_cleaned() {
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

        let disk = disk_use(&app, cleaning_target, Path::new(HOME)).unwrap();

        assert_eq!(disk.total.as_deref(), Some("12 GB"));
        assert_eq!(disk.total_tooltip.as_deref(), Some("ui/node_modules/ 5 MB"));
        assert_eq!(disk.warning, None);
        let folders: Vec<(&str, bool)> = disk
            .entries
            .iter()
            .map(|folder| (folder.label.as_str(), folder.cleaning))
            .collect();
        assert_eq!(folders, [("target/", true), ("node_modules/", false)]);
        assert_eq!(disk.entries[0].tooltip, "9 GB\n~/app/target");
        assert_eq!(disk.progress, Some(0.25));
    }

    /// A folder whose size is not known yet, and one that cannot be read, though Sutra once
    /// measured it.
    fn unsized_and_unreadable() -> [Project; 2] {
        let not_sized = project(vec![cargo_target(
            "/code/app/target",
            Measurement::Measuring,
        )]);
        let denied = Project {
            access: FolderAccess::Denied,
            own_files: measured(1_000),
            ..project(Vec::new())
        };
        [not_sized, denied]
    }

    #[test]
    fn a_folder_that_cannot_be_read_or_has_nothing_sized_yet_has_no_tray_while_no_agent_is_in_it() {
        for project in unsized_and_unreadable() {
            let tray = tray(&project, Vec::new(), Vec::new(), |_| None, Path::new(HOME));
            assert_eq!(tray, None);
        }
    }

    #[test]
    fn an_agent_gives_its_folder_a_tray_whatever_is_known_of_the_folders_size() {
        let badge = |tooltip: &str| AgentBadge {
            robot: Robot::Codex,
            look: AgentLook::Working,
            tooltip: tooltip.to_string(),
            link: None,
        };
        let (session, subagent) = (badge("Busy\nabc-123\nCodex"), badge("Busy\nfor abc-123"));
        for project in unsized_and_unreadable() {
            let only_a_session = tray(
                &project,
                vec![session.clone()],
                Vec::new(),
                |_| None,
                Path::new(HOME),
            );
            assert_eq!(
                only_a_session,
                Some(Tray {
                    disk: None,
                    sessions: vec![session.clone()],
                    subagents: Vec::new(),
                })
            );
            let only_a_subagent = tray(
                &project,
                Vec::new(),
                vec![subagent.clone()],
                |_| None,
                Path::new(HOME),
            );
            assert_eq!(
                only_a_subagent,
                Some(Tray {
                    disk: None,
                    sessions: Vec::new(),
                    subagents: vec![subagent.clone()],
                })
            );
        }

        let sized = Project {
            own_files: measured(5_000_000),
            ..project(Vec::new())
        };
        let tray = tray(
            &sized,
            vec![session.clone()],
            vec![subagent.clone()],
            |_| None,
            Path::new(HOME),
        )
        .unwrap();
        assert_eq!(
            tray.disk.and_then(|disk| disk.total).as_deref(),
            Some("5 MB")
        );
        assert_eq!(
            (tray.sessions, tray.subagents),
            (vec![session], vec![subagent]),
            "beside the disk use"
        );
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
}
