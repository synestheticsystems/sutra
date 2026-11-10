//! The build folders kept only by agent sessions at rest: in a project where every session has
//! ended its turn and waited for its next prompt a while, nothing else is at work, and nothing has
//! happened for as long. A session keeps its project's folders from everything Sutra does by
//! itself, however long it rests: it may be waiting for the user's review, when what it built is
//! wanted, and a resume would build it all again. So the window asks about these apart from the
//! idle ones, and only a yes to that question cleans them.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::super::prepare::Builds;
use super::super::{CargoEnv, Prepared};
use super::{
    LeftOut, OfferTimes, can_tell_what_is_at_work, is_at_work_but_for_sessions, is_idle,
    prepare_unless_kept, quiet_for, sessions_on, sharer_keeping, worth_cleaning,
};
use crate::map::{ArtifactDir, FolderAccess, Project, Snapshot};

/// A build folder worth cleaning in a project kept only by sessions at rest.
#[derive(Debug, Clone, Copy)]
pub struct AtRestFolder<'a> {
    pub project: &'a Project,
    pub dir: &'a ArtifactDir,
    pub bytes: u64,
    /// How long the session on the project that rested least had rested.
    pub at_rest_for: Duration,
}

/// How long every session on `project` has been at rest as of `now`, the shortest of their rests:
/// `None` while one of them is not at rest or cannot tell since when, or when none is on it.
fn sessions_at_rest_for(project: &Project, now: SystemTime) -> Option<Duration> {
    let rests: Option<Vec<Duration>> = sessions_on(project)
        .map(|session| session.at_rest_for(now))
        .collect();
    rests?.into_iter().min()
}

/// How long the sessions on `project` have been at rest, as `sessions_at_rest_for` tells it, when
/// they alone keep it: each at rest for `at_least`, nothing else at work there, and the project
/// known to have been quiet as long. `None` when they do not.
fn kept_only_by_sessions_at_rest(
    project: &Project,
    now: SystemTime,
    at_least: Duration,
) -> Option<Duration> {
    let quiet = quiet_for(project, now).is_some_and(|quiet| quiet >= at_least);
    if !quiet || is_at_work_but_for_sessions(project) {
        return None;
    }
    sessions_at_rest_for(project, now).filter(|rest| *rest >= at_least)
}

/// Whether the builds of `project` may go with a clean of folders kept only by sessions at rest,
/// its own or one whose folders it shares: it is idle by `times.quiet`, or kept only by sessions at
/// rest by `times.at_rest`, and so among those the same question asks about.
fn may_go(project: &Project, now: SystemTime, times: OfferTimes) -> bool {
    is_idle(project, now, times.quiet)
        || kept_only_by_sessions_at_rest(project, now, times.at_rest).is_some()
}

/// Every Cargo build folder worth cleaning in a project kept only by sessions at rest for
/// `at_least`, the largest first. None while `map` cannot say what is at work.
pub fn at_rest_folders(
    map: &Snapshot,
    now: SystemTime,
    at_least: Duration,
) -> Vec<AtRestFolder<'_>> {
    if !can_tell_what_is_at_work(map) {
        return Vec::new();
    }
    let mut found: Vec<AtRestFolder> = (map.projects.iter())
        .filter(|project| project.access == FolderAccess::Readable)
        .filter_map(|project| {
            let at_rest_for = kept_only_by_sessions_at_rest(project, now, at_least)?;
            Some((project, at_rest_for))
        })
        .flat_map(|(project, at_rest_for)| {
            worth_cleaning(project).map(move |(dir, bytes)| AtRestFolder {
                project,
                dir,
                bytes,
                at_rest_for,
            })
        })
        .collect();
    found.sort_by_key(|folder| std::cmp::Reverse(folder.bytes));
    found
}

/// The clean of one folder kept only by sessions at rest, ready to ask about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtRestClean {
    /// The build folder on the map that led to this clean.
    pub clicked: PathBuf,
    pub project_name: String,
    pub project_path: PathBuf,
    /// How many sessions were on the project, and how long the one that rested least had rested,
    /// when the clean was worked out, which the question says.
    pub sessions: usize,
    pub at_rest_for: Duration,
    pub prepared: Prepared,
}

/// The cleans of every folder kept only by sessions at rest, the largest first, and the folders
/// that cannot go now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtRestCleans {
    pub cleans: Vec<AtRestClean>,
    pub left_out: Vec<LeftOut>,
}

/// Work out the clean of each of `folders`, among other cleans that share what `builds` worked out.
/// A project sharing a folder must be idle, or kept only by sessions at rest too (see `may_go`).
pub(super) fn prepare_among(
    folders: Vec<AtRestFolder>,
    map: &Snapshot,
    now: SystemTime,
    times: OfferTimes,
    env: &CargoEnv,
    builds: &Builds,
) -> AtRestCleans {
    let mut found = AtRestCleans::default();
    let may_share = |sharer: &Project| may_go(sharer, now, times);
    for folder in folders {
        let project = folder.project;
        match prepare_unless_kept(project, folder.dir, map, env, builds, may_share) {
            Err(left_out) => found.left_out.push(left_out),
            Ok(prepared) => found.cleans.push(AtRestClean {
                clicked: folder.dir.path.clone(),
                project_name: project.name.clone(),
                project_path: project.path.clone(),
                sessions: sessions_on(project).count(),
                at_rest_for: folder.at_rest_for,
                prepared,
            }),
        }
    }
    found
}

/// Whether `prepared`, worked out for a build folder of the project at `project`, kept only by
/// sessions at rest, may still go as `map` shows things now: the project and each sharer still
/// kept only by sessions at rest, or idle (see `may_go`). Its turn can come many minutes after it
/// was worked out, and by then a session there may have taken a turn, or something else started.
pub fn is_still_at_rest(
    prepared: &Prepared,
    project: &Path,
    map: &Snapshot,
    now: SystemTime,
    times: OfferTimes,
) -> bool {
    can_tell_what_is_at_work(map)
        && (map.projects.iter())
            .find(|candidate| candidate.path == project)
            .is_some_and(|project| may_go(project, now, times))
        && sharer_keeping(prepared, project, map, |sharer| may_go(sharer, now, times)).is_none()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::tests::{
        DAY, LARGE, THREE_DAYS, app_and_a_package_building_in_it, idle_package, now, quiet_project,
        scratch_folder, session_file_unread,
    };
    use super::super::{Why, prepare_offer};
    use super::*;
    use crate::map::clean::locks::tests::hold_like_cargo;
    use crate::map::clean::{CleanError, PrepareError};
    use crate::map::test_support::{
        TempDir, agent_session, at, container_for, environment_at, listening_in, measured,
        snapshot_of, subagent_at_work,
    };
    use crate::map::{AgentSession, AgentStatus, Harness, SubagentWithParent};

    const TIMES: OfferTimes = OfferTimes {
        quiet: THREE_DAYS,
        at_rest: Duration::from_secs(DAY),
    };

    /// A session `id` whose status, `status`, was set `secs` seconds before `now()`.
    fn session_since(id: &str, status: AgentStatus, secs: u64) -> AgentSession {
        AgentSession {
            since: Some(now() - Duration::from_secs(secs)),
            ..agent_session(id, status)
        }
    }

    /// A session `id` at rest for `days`.
    fn resting(id: &str, days: u64) -> AgentSession {
        session_since(id, AgentStatus::Idle, days * DAY)
    }

    /// `project` with `agents` on it.
    fn with(agents: Vec<AgentSession>, project: Project) -> Project {
        Project { agents, ..project }
    }

    /// Each project `at_rest_folders` finds on `map`, in its order, and how many whole days its
    /// sessions have rested.
    fn found(map: &Snapshot) -> Vec<(String, u64)> {
        (at_rest_folders(map, now(), TIMES.at_rest).iter())
            .map(|folder| {
                let days = folder.at_rest_for.as_secs() / DAY;
                (folder.project.name.clone(), days)
            })
            .collect()
    }

    #[test]
    fn a_project_whose_every_session_rested_a_day_is_found_largest_first_by_its_least_rest() {
        let map = snapshot_of(vec![
            with(vec![resting("a", 2)], quiet_project("resting", 2, LARGE)),
            with(
                vec![resting("b", 5), resting("c", 3)],
                quiet_project("two-resting", 5, 2 * LARGE),
            ),
            with(
                vec![resting("d", 5), session_since("e", AgentStatus::Busy, 0)],
                quiet_project("one-busy", 5, LARGE),
            ),
            with(
                vec![session_since("f", AgentStatus::Waiting, 5 * DAY)],
                quiet_project("waiting-on-a-prompt", 5, LARGE),
            ),
            with(
                vec![session_since("g", AgentStatus::Quiet, 5 * DAY)],
                quiet_project("silent-mid-turn", 5, LARGE),
            ),
            with(
                vec![session_since("h", AgentStatus::Unknown, 5 * DAY)],
                quiet_project("status-unknown", 5, LARGE),
            ),
            with(
                vec![session_since("i", AgentStatus::Idle, DAY / 2)],
                quiet_project("rested-half-a-day", 5, LARGE),
            ),
            with(
                vec![resting("j", 5)],
                quiet_project("active-today", 0, LARGE),
            ),
            with(
                vec![agent_session("k", AgentStatus::Idle)],
                quiet_project("rest-undated", 5, LARGE),
            ),
            quiet_project("no-session", 9, LARGE),
        ]);

        assert_eq!(
            found(&map),
            [("two-resting".to_string(), 3), ("resting".to_string(), 2)]
        );
    }

    #[test]
    fn a_finished_job_is_no_session_on_a_project_and_keeps_none_of_its_folders() {
        let finished = AgentSession {
            harness: Harness::Codex,
            kind: Some("exec".to_string()),
            ..agent_session("job", AgentStatus::Idle)
        };
        let map = snapshot_of(vec![
            with(
                vec![finished.clone(), resting("a", 2)],
                quiet_project("resting", 2, LARGE),
            ),
            with(vec![finished], quiet_project("job-done", 2, LARGE)),
        ]);
        assert_eq!(found(&map), [("resting".to_string(), 2)]);
    }

    #[test]
    fn anything_else_at_work_keeps_a_project_whose_sessions_rest() {
        let at_rest = |name: &str| with(vec![resting("a", 5)], quiet_project(name, 5, LARGE));
        let session = agent_session("elsewhere", AgentStatus::Busy);
        let map = snapshot_of(vec![
            Project {
                subagents: vec![SubagentWithParent::new(subagent_at_work("a1"), &session)],
                ..at_rest("subagent")
            },
            Project {
                environment: Some(crate::model::Environment {
                    alive: true,
                    ..environment_at("/code/environment")
                }),
                ..at_rest("environment")
            },
            Project {
                listening: vec![listening_in("/code/server", "server", &[8080])],
                ..at_rest("server")
            },
            Project {
                containers: vec![container_for("/code/container/data", "pg")],
                ..at_rest("container")
            },
            // A scratch folder another session works in, while its own session runs elsewhere.
            with(
                vec![resting("a", 5)],
                scratch_folder("scratch", Some(resting("its-own", 5)), measured(1_000)),
            ),
        ]);
        assert_eq!(found(&map), []);
    }

    #[test]
    fn nothing_is_kept_by_sessions_at_rest_while_what_tells_of_work_goes_unread() {
        let map = Snapshot {
            problems: vec![session_file_unread()],
            ..snapshot_of(vec![with(
                vec![resting("a", 5)],
                quiet_project("resting", 5, LARGE),
            )])
        };
        assert_eq!(found(&map), []);
    }

    /// `idle_package`'s project at `name` in `fixture`, with one session on it at rest for `days`.
    fn resting_package(fixture: &TempDir, name: &str, days: u64) -> Project {
        with(vec![resting("s", days)], idle_package(fixture, name))
    }

    fn at_rest_cleaned(found: &AtRestCleans) -> Vec<PathBuf> {
        (found.cleans.iter())
            .map(|clean| clean.clicked.clone())
            .collect()
    }

    #[test]
    fn the_offer_prepares_the_idle_folders_and_apart_those_kept_by_sessions_at_rest() {
        let fixture = TempDir::new("at-rest-offer");
        let map = snapshot_of(vec![
            idle_package(&fixture, "idle"),
            resting_package(&fixture, "resting", 2),
            resting_package(&fixture, "building", 2),
            resting_package(&fixture, "being-cleaned", 2),
        ]);
        let busy = fixture.write("building/target/debug/.cargo-build-lock", "");
        let _building = hold_like_cargo(&busy);
        let being_cleaned = HashSet::from([fixture.path("being-cleaned/target")]);

        let found = prepare_offer(&map, now(), TIMES, &being_cleaned, &fixture.cargo_env());

        let idle: Vec<&PathBuf> = (found.idle.cleans.iter())
            .map(|clean| &clean.clicked)
            .collect();
        assert_eq!(idle, [&fixture.path("idle/target")]);
        assert_eq!(
            at_rest_cleaned(&found.at_rest),
            [fixture.path("resting/target")]
        );
        let clean = &found.at_rest.cleans[0];
        assert_eq!(
            (clean.sessions, clean.at_rest_for),
            (1, Duration::from_secs(2 * DAY))
        );
        assert_eq!(
            found.at_rest.left_out,
            [LeftOut {
                project_name: "building".to_string(),
                folder: fixture.path("building/target"),
                why: Why::Refused(PrepareError::Locks(CleanError::BuildRunning { lock: busy })),
            }]
        );
    }

    #[test]
    fn a_sharer_must_be_idle_or_kept_by_sessions_at_rest_too_when_worked_out_and_at_the_turn() {
        let fixture = TempDir::new("at-rest-shared");
        let (app, other) = app_and_a_package_building_in_it(&fixture);
        let app = with(vec![resting("s", 2)], app);
        let env = fixture.cargo_env();
        // `other` builds in `app`'s folder, as it is: quiet 9 days with nothing on it, or as `be`
        // makes it.
        let left_out_by = |be: &dyn Fn(Project) -> Project| {
            let map = snapshot_of(vec![app.clone(), be(other.clone())]);
            let found = prepare_offer(&map, now(), TIMES, &HashSet::new(), &env).at_rest;
            let still = (found.cleans.first()).is_some_and(|clean| {
                is_still_at_rest(&clean.prepared, &app.path, &map, now(), TIMES)
            });
            match found.left_out.as_slice() {
                [] => {
                    assert_eq!(at_rest_cleaned(&found), [fixture.path("app/target")]);
                    assert!(still, "and still at its turn");
                    None
                }
                [left_out] => Some(left_out.why.clone()),
                many => panic!("one folder, left out {many:?}"),
            }
        };
        let shared_with = Some(Why::SharedWith("other".to_string()));

        assert_eq!(left_out_by(&|other| other), None, "other is idle");
        assert_eq!(
            left_out_by(&|other| with(vec![resting("t", 2)], other)),
            None,
            "other is kept only by a session at rest too"
        );
        let quiet_two_days = |other: Project| Project {
            last_activity: Some(at((1_000 - 2) * DAY)),
            ..other
        };
        assert_eq!(
            left_out_by(&quiet_two_days),
            shared_with,
            "other is neither idle its 3 days nor kept by a session at rest"
        );
        assert_eq!(
            left_out_by(&|other| with(vec![session_since("t", AgentStatus::Busy, 0)], other)),
            shared_with,
            "other has a session at work"
        );
    }

    #[test]
    fn an_idle_folder_a_project_with_a_session_at_rest_builds_in_is_never_offered_as_idle() {
        let fixture = TempDir::new("at-rest-not-idle");
        let (app, other) = app_and_a_package_building_in_it(&fixture);
        let map = snapshot_of(vec![app, with(vec![resting("s", 30)], other)]);

        let found = prepare_offer(&map, now(), TIMES, &HashSet::new(), &fixture.cargo_env());

        assert!(found.idle.cleans.is_empty());
        assert_eq!(
            found.idle.left_out[0].why,
            Why::SharedWith("other".to_string())
        );
    }

    #[test]
    fn a_clean_goes_at_its_turn_only_while_its_sessions_still_rest_and_nothing_else_started() {
        let prepared = crate::map::clean::Prepared::for_test(
            crate::map::clean::Plan::CargoClean(crate::map::clean::CleanPlan {
                workspace: PathBuf::from("/code/app"),
                folders: Vec::new(),
            }),
            Vec::new(),
            crate::map::clean::Sharing::default(),
        );
        let app = |agents, days| with(agents, quiet_project("app", days, LARGE));
        let still =
            |map: &Snapshot| is_still_at_rest(&prepared, Path::new("/code/app"), map, now(), TIMES);

        assert!(still(&snapshot_of(vec![app(vec![resting("s", 2)], 2)])));
        let took_a_turn = session_since("s", AgentStatus::Busy, 0);
        assert!(!still(&snapshot_of(vec![app(vec![took_a_turn], 2)])));
        let rests_again = session_since("s", AgentStatus::Idle, DAY / 2);
        assert!(!still(&snapshot_of(vec![app(vec![rests_again], 2)])));
        let server = Project {
            listening: vec![listening_in("/code/app", "server", &[8080])],
            ..app(vec![resting("s", 2)], 2)
        };
        assert!(!still(&snapshot_of(vec![server])), "a server started");
        assert!(
            still(&snapshot_of(vec![app(Vec::new(), 9)])),
            "its session ended, and it is idle"
        );
        assert!(
            !still(&snapshot_of(vec![app(Vec::new(), 2)])),
            "its session ended, and it is not idle its 3 days"
        );
        assert!(!still(&snapshot_of(Vec::new())), "it left the map");
        let cannot_tell = Snapshot {
            problems: vec![session_file_unread()],
            ..snapshot_of(vec![app(vec![resting("s", 2)], 2)])
        };
        assert!(!still(&cannot_tell));
    }
}
