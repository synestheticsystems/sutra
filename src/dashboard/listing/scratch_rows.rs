//! Which of a session's scratch folders have a row on the list. The map holds a scratch folder as
//! soon as it holds a build folder, however small, so that cleaning can reach it and its session's
//! line can count it; but below a gigabyte, the size at which the tray names a build folder, its
//! card would show nothing but a name, an age and a number. So a scratch folder has a row only
//! while it has something to show, and its session's line still counts it in its total.

use super::focus::disk_bytes;
use crate::dashboard::cleaning::Cleaning;
use crate::map::{LARGE_FOLDER_BYTES, Measurement, Project};

/// Whether `project` has a row on the list. Every project has one but a scratch folder, which has
/// one only while an agent or a dev environment works in it, while it holds `LARGE_FOLDER_BYTES`
/// (see `disk_bytes`), while `cleaning` is at work on it, or while a measurement of it failed,
/// which its card says why of.
pub(super) fn has_a_row(project: &Project, cleaning: &Cleaning) -> bool {
    let at_work = || {
        !project.agents.is_empty() || !project.subagents.is_empty() || project.environment.is_some()
    };
    let failed = || {
        let mut measurements = project.measurements();
        measurements.any(|measurement| matches!(measurement, Measurement::Failed(_)))
    };
    project.scratch.is_none()
        || at_work()
        || disk_bytes(project) >= LARGE_FOLDER_BYTES
        || cleaning.is_at_work_in(project)
        || failed()
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::dashboard::cleaning::tests::{cleaning, deleted};
    use crate::dashboard::listing::fixtures::{
        agent, quiet, running_as, scratch_folder, subagent_in,
    };
    use crate::map::AgentStatus;
    use crate::map::test_support::{cargo_target, environment_at, measured};

    /// A scratch folder of an ended session holding `own_files` and a build folder of `target`,
    /// at `/code/app/target`, where `deleted()` has cleaned one.
    fn scratch(own_files: Measurement, target: Measurement) -> Project {
        Project {
            own_files,
            artifacts: vec![cargo_target("/code/app/target", target)],
            ..scratch_folder("e26ca359", "r567", quiet(""))
        }
    }

    #[test]
    fn a_project_that_is_no_scratch_folder_always_has_a_row() {
        assert!(has_a_row(&quiet("notes"), &cleaning()));
    }

    #[test]
    fn a_small_scratch_folder_with_nothing_to_show_has_no_row_even_while_its_session_runs() {
        let small = scratch(measured(LARGE_FOLDER_BYTES - 11), measured(10));
        assert!(!has_a_row(&small, &cleaning()));
        let running = running_as("Foundry", small);
        assert!(
            !has_a_row(&running, &cleaning()),
            "its session's robot stands where it works"
        );
    }

    #[test]
    fn a_scratch_folder_has_a_row_once_it_holds_large_folder_bytes() {
        let large = scratch(measured(LARGE_FOLDER_BYTES - 10), measured(10));
        assert!(has_a_row(&large, &cleaning()));
        let sizing = scratch(Measurement::Measuring, measured(LARGE_FOLDER_BYTES));
        assert!(
            has_a_row(&sizing, &cleaning()),
            "its build folders alone, before its own files are sized"
        );
    }

    #[test]
    fn a_scratch_folder_has_a_row_while_an_agent_or_a_dev_environment_works_in_it() {
        let small = || scratch(measured(0), measured(10));
        let session = Project {
            agents: vec![agent("/code/app/review", AgentStatus::Busy)],
            ..small()
        };
        let subagent = Project {
            subagents: vec![subagent_in("/code/app/review")],
            ..small()
        };
        let environment = Project {
            environment: Some(environment_at("/code/app")),
            ..small()
        };
        for working in [session, subagent, environment] {
            assert!(has_a_row(&working, &cleaning()), "{working:?}");
        }
    }

    #[test]
    fn a_scratch_folder_has_a_row_while_a_clean_is_at_work_on_it_or_its_measurement_failed() {
        let small = scratch(measured(0), measured(10));
        assert!(
            has_a_row(&small, &deleted()),
            "cleaned, not yet measured again"
        );
        let unmeasured = scratch(
            Measurement::Failed(io::ErrorKind::PermissionDenied),
            measured(10),
        );
        assert!(has_a_row(&unmeasured, &cleaning()));
        let target_unmeasured = scratch(
            measured(0),
            Measurement::Failed(io::ErrorKind::PermissionDenied),
        );
        assert!(has_a_row(&target_unmeasured, &cleaning()));
    }
}
