//! Which projects the list keeps in focus, in each sort. Discovery casts a wide net, every worktree
//! under the roots, which on a busy machine is hundreds of rows; focus keeps the ones worth a look,
//! and the rest wait behind the quiet row. What is worth a look depends on what the sort is for:
//! the Activity sort shows the work going on, so what only takes disk waits there, as the gauge of
//! the disk and the Disk sort show it.

use std::time::{Duration, SystemTime};

use crate::dashboard::SortOrder;
use crate::map::{LARGE_FOLDER_BYTES, Project};

/// How recent a project's last activity must be to keep it in focus. A first guess.
pub(in crate::dashboard) const FOCUS_RECENT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Whether the project belongs in the focused view of `sort`. In every sort, a project is in focus
/// while a coding agent has a session or a subagent at work in it, or a dev-runner environment
/// describes it, from `~/.dev-runner/` or its own `.dev-runner/`. Past that:
///   - in the Activity sort, while its last activity lies within `FOCUS_RECENT` of `now`, unless it
///     is the scratch of a session that has ended: that is disk news, not activity news, whatever
///     its age or size;
///   - in the Name and Disk sorts, while its last activity lies within `FOCUS_RECENT` of `now`, or
///     its folder holds at least `LARGE_FOLDER_BYTES` (see `disk_bytes`), so a forgotten folder
///     that fills the disk is not hidden. A large project joins the focus when the measurement
///     that shows it lands.
///
/// Both thresholds are first guesses, to tune once the focused view has been lived with.
pub(super) fn in_focus(project: &Project, sort: SortOrder, now: SystemTime) -> bool {
    let at_work = !project.agents.is_empty()
        || !project.subagents.is_empty()
        || project.environment.is_some();
    let recent = project.last_activity.is_some_and(|at| {
        // A time ahead of the clock (a file touched with a future date) is recent.
        now.duration_since(at).unwrap_or(Duration::ZERO) <= FOCUS_RECENT
    });
    at_work
        || match sort {
            SortOrder::Activity => recent && !is_ended_scratch(project),
            SortOrder::Name | SortOrder::Disk => {
                recent || disk_bytes(project) >= LARGE_FOLDER_BYTES
            }
        }
}

/// Whether `project` is a scratch folder whose session has ended, so that nothing will build in it
/// again.
fn is_ended_scratch(project: &Project) -> bool {
    let space = project.scratch.as_ref();
    space.is_some_and(|space| space.session.is_none())
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
        subagent_at_work,
    };
    use crate::map::{AgentStatus, ArtifactDir, Measurement, SubagentWithParent};

    fn now() -> SystemTime {
        at(1_000_000_000)
    }

    fn target(measurement: Measurement) -> ArtifactDir {
        cargo_target("/code/app/target", measurement)
    }

    /// A project that misses every reason, each by the smallest margin: no agent, no environment,
    /// last active one second before `FOCUS_RECENT`, and build folders one byte short of
    /// `LARGE_FOLDER_BYTES`, beside one still measuring and one whose measurement failed. Its own
    /// files are not measured yet, so its build folders are all its size is known by.
    fn out_of_focus() -> Project {
        Project {
            last_activity: Some(now() - FOCUS_RECENT - Duration::from_secs(1)),
            artifacts: vec![
                target(measured(LARGE_FOLDER_BYTES - 1)),
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

    /// The sorts in which `project` is in focus.
    fn focused_in(project: &Project) -> Vec<SortOrder> {
        let sorts = SortOrder::ALL.into_iter();
        sorts
            .filter(|sort| in_focus(project, *sort, now()))
            .collect()
    }

    #[test]
    fn a_project_that_misses_every_reason_is_out_of_focus_in_every_sort() {
        assert_eq!(focused_in(&out_of_focus()), []);
        let never_active = Project {
            last_activity: None,
            ..out_of_focus()
        };
        assert_eq!(focused_in(&never_active), []);
    }

    #[test]
    fn an_agent_session_brings_a_project_into_focus_in_every_sort() {
        let working = Project {
            agents: vec![agent_session("thread-1", AgentStatus::Idle)],
            ..out_of_focus()
        };
        assert_eq!(focused_in(&working), SortOrder::ALL);
    }

    #[test]
    fn a_subagent_at_work_for_a_session_elsewhere_brings_a_project_into_focus_in_every_sort() {
        let session = agent_session("elsewhere", AgentStatus::Idle);
        let working = Project {
            subagents: vec![SubagentWithParent::new(subagent_at_work("a1"), &session)],
            ..out_of_focus()
        };
        assert_eq!(focused_in(&working), SortOrder::ALL);
    }

    #[test]
    fn a_dev_runner_environment_brings_a_project_into_focus_in_every_sort() {
        let described = Project {
            environment: Some(environment_at("/code/app")),
            ..out_of_focus()
        };
        assert_eq!(focused_in(&described), SortOrder::ALL);
    }

    #[test]
    fn activity_within_focus_recent_brings_a_project_into_focus_in_every_sort() {
        let at_the_edge = Project {
            last_activity: Some(now() - FOCUS_RECENT),
            ..out_of_focus()
        };
        assert_eq!(focused_in(&at_the_edge), SortOrder::ALL);
        let ahead_of_the_clock = Project {
            last_activity: Some(now() + Duration::from_secs(60)),
            ..out_of_focus()
        };
        assert_eq!(focused_in(&ahead_of_the_clock), SortOrder::ALL);
    }

    #[test]
    fn build_folders_that_reach_large_folder_bytes_bring_a_project_into_focus_by_name_and_disk() {
        let mut project = out_of_focus();
        project
            .artifacts
            .push(cargo_target("/code/app/crates/cli/target", measured(1)));
        assert_eq!(focused_in(&project), [SortOrder::Name, SortOrder::Disk]);
    }

    #[test]
    fn a_whole_folder_that_reaches_large_folder_bytes_brings_a_project_into_focus_by_name_and_disk()
    {
        let small_target_large_folder = sized(measured(LARGE_FOLDER_BYTES - 10), measured(10));
        assert_eq!(
            focused_in(&small_target_large_folder),
            [SortOrder::Name, SortOrder::Disk],
            "the Activity sort leaves size to the gauge and the Disk sort"
        );
        let one_byte_short = sized(measured(LARGE_FOLDER_BYTES - 11), measured(10));
        assert_eq!(focused_in(&one_byte_short), []);
    }

    #[test]
    fn the_scratch_of_a_session_that_ended_is_quiet_in_the_activity_sort_whatever_its_age_or_size()
    {
        use crate::dashboard::listing::fixtures::{running_as, scratch_folder};
        let recent_and_large = Project {
            last_activity: Some(now()),
            ..sized(measured(LARGE_FOLDER_BYTES), measured(10))
        };
        let ended = scratch_folder("e26ca359", "r567", recent_and_large);
        assert_eq!(focused_in(&ended), [SortOrder::Name, SortOrder::Disk]);
        let running = running_as("Foundry", ended.clone());
        assert_eq!(
            focused_in(&running),
            SortOrder::ALL,
            "a running one's is news"
        );
        let reviewed = Project {
            agents: vec![agent_session("thread-1", AgentStatus::Busy)],
            ..ended
        };
        assert_eq!(
            focused_in(&reviewed),
            SortOrder::ALL,
            "an agent at work in it is news"
        );
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
