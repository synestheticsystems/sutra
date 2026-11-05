//! Which projects the list keeps in focus. Discovery casts a wide net, every worktree under the
//! roots, which on a busy machine is hundreds of rows; focus keeps the ones worth a look, and the
//! rest wait behind the quiet row.

use std::time::{Duration, SystemTime};

use crate::map::Project;

/// How recent a project's last activity must be to keep it in focus. A first guess.
pub(in crate::dashboard) const FOCUS_RECENT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How much disk a project's folder must hold to bring it into focus on size alone. A first
/// guess.
pub(in crate::dashboard) const FOCUS_LARGE_BYTES: u64 = 1 << 30;

/// Whether the project belongs in the focused view. A project is in focus when any of these holds:
///   - a coding agent has a session in it;
///   - a dev-runner environment describes it, from `~/.dev-runner/` or its own `.dev-runner/`;
///   - its last activity lies within `FOCUS_RECENT` of `now`;
///   - its folder holds at least `FOCUS_LARGE_BYTES` (see `disk_bytes`), so a forgotten folder
///     that fills the disk is not hidden. A large project joins the focus when the
///     measurement that shows it lands.
///
/// Both thresholds are first guesses, to tune once the focused view has been lived with.
pub(super) fn in_focus(project: &Project, now: SystemTime) -> bool {
    !project.agents.is_empty()
        || project.environment.is_some()
        || project.last_activity.is_some_and(|at| {
            // A time ahead of the clock (a file touched with a future date) is recent.
            now.duration_since(at).unwrap_or(Duration::ZERO) <= FOCUS_RECENT
        })
        || disk_bytes(project) >= FOCUS_LARGE_BYTES
}

/// What the Disk sort and the focus rule weigh: the whole folder once it is measured, and until
/// then the build folders measured so far.
pub(super) fn disk_bytes(project: &Project) -> u64 {
    project
        .folder_size()
        .map_or_else(|| project.measured_build_bytes(), |size| size.bytes)
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::map::test_support::{
        agent_session, at, cargo_target, environment_at, measured, node_modules, project_at,
    };
    use crate::map::{AgentStatus, ArtifactDir, Measurement};

    fn now() -> SystemTime {
        at(1_000_000_000)
    }

    fn target(measurement: Measurement) -> ArtifactDir {
        cargo_target("/code/app/target", measurement)
    }

    /// A project that misses every reason, each by the smallest margin: no agent, no environment,
    /// last active one second before `FOCUS_RECENT`, and build folders one byte short of
    /// `FOCUS_LARGE_BYTES`, beside one still measuring and one whose measurement failed. Its own
    /// files are not measured yet, so its build folders are all its size is known by.
    fn out_of_focus() -> Project {
        Project {
            last_activity: Some(now() - FOCUS_RECENT - Duration::from_secs(1)),
            artifacts: vec![
                target(measured(FOCUS_LARGE_BYTES - 1)),
                node_modules("/code/app/web/node_modules", Measurement::Measuring),
                cargo_target(
                    "/code/app/old/target",
                    Measurement::Failed(io::ErrorKind::PermissionDenied),
                ),
            ],
            own_files: Measurement::Measuring,
            ..project_at("/code/app")
        }
    }

    /// A project whose own files and only build folder are both as given.
    fn sized(own_files: Measurement, build_folder: Measurement) -> Project {
        Project {
            artifacts: vec![target(build_folder)],
            own_files,
            ..out_of_focus()
        }
    }

    #[test]
    fn a_project_that_misses_every_reason_is_out_of_focus() {
        assert!(!in_focus(&out_of_focus(), now()));
        let never_active = Project {
            last_activity: None,
            ..out_of_focus()
        };
        assert!(!in_focus(&never_active, now()));
    }

    #[test]
    fn an_agent_session_brings_a_project_into_focus() {
        let working = Project {
            agents: vec![agent_session("thread-1", AgentStatus::Idle)],
            ..out_of_focus()
        };
        assert!(in_focus(&working, now()));
    }

    #[test]
    fn a_dev_runner_environment_brings_a_project_into_focus() {
        let described = Project {
            environment: Some(environment_at("/code/app")),
            ..out_of_focus()
        };
        assert!(in_focus(&described, now()));
    }

    #[test]
    fn activity_within_focus_recent_brings_a_project_into_focus() {
        let at_the_edge = Project {
            last_activity: Some(now() - FOCUS_RECENT),
            ..out_of_focus()
        };
        assert!(in_focus(&at_the_edge, now()));
        let ahead_of_the_clock = Project {
            last_activity: Some(now() + Duration::from_secs(60)),
            ..out_of_focus()
        };
        assert!(in_focus(&ahead_of_the_clock, now()));
    }

    #[test]
    fn build_folders_that_reach_focus_large_bytes_between_them_bring_a_project_into_focus() {
        let mut project = out_of_focus();
        project
            .artifacts
            .push(cargo_target("/code/app/crates/cli/target", measured(1)));
        assert!(in_focus(&project, now()));
    }

    #[test]
    fn a_whole_folder_that_reaches_focus_large_bytes_brings_a_project_into_focus() {
        let small_target_large_folder = sized(measured(FOCUS_LARGE_BYTES - 10), measured(10));
        assert!(in_focus(&small_target_large_folder, now()));
        let one_byte_short = sized(measured(FOCUS_LARGE_BYTES - 11), measured(10));
        assert!(!in_focus(&one_byte_short, now()));
    }

    #[test]
    fn disk_bytes_weigh_the_whole_folder_once_known_and_the_build_folders_until_then() {
        assert_eq!(disk_bytes(&sized(measured(300), measured(700))), 1_000);
        assert_eq!(
            disk_bytes(&sized(Measurement::Measuring, measured(700))),
            700
        );
    }
}
