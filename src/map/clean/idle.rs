//! Which build folders nobody is using, and the cleans of those that may go: what the window offers
//! to clean in one go, and what it cleans by itself when the disk runs low.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::prepare::{Builds, building_in, prepare_among};
use super::{CargoEnv, PrepareError, Prepared};
use crate::map::{ArtifactDir, ArtifactKind, FolderAccess, Project, Snapshot};

/// The least a build folder holds for a clean to be worth offering. A clean leaves a few empty lock
/// files and a tag behind (see `run`), which must not go on offering themselves.
pub const WORTH_CLEANING_BYTES: u64 = 1 << 20;

/// A build folder worth cleaning in a project nobody is using.
#[derive(Debug, Clone, Copy)]
pub struct IdleFolder<'a> {
    pub project: &'a Project,
    pub dir: &'a ArtifactDir,
    pub bytes: u64,
}

/// Whether something is at work in `project` now: an agent, other than a job that has finished, a
/// subagent at work there, as in a worktree of its own, an environment whose process lives, or the
/// session whose scratch folder it is, while the session runs. Such a session builds into its
/// scratch whenever it likes, and a build's writes date no folder, so its scratch can look quiet
/// between two of its builds.
fn in_use(project: &Project) -> bool {
    let session_runs = project
        .scratch
        .as_ref()
        .is_some_and(|space| space.session.is_some());
    project.agents.iter().any(|agent| !agent.is_finished_job())
        || !project.subagents.is_empty()
        || project.environment.as_ref().is_some_and(|env| env.alive)
        || session_runs
}

/// How long `project` has been quiet. One with no known activity counts as quiet for good, and one
/// whose activity is dated ahead of the clock as just active.
fn quiet_for(project: &Project, now: SystemTime) -> Duration {
    project.last_activity.map_or(Duration::MAX, |at| {
        now.duration_since(at).unwrap_or(Duration::ZERO)
    })
}

/// Whether nothing is at work in `project`, and it has been quiet for `at_least`.
fn is_idle(project: &Project, now: SystemTime, at_least: Duration) -> bool {
    !in_use(project) && quiet_for(project, now) >= at_least
}

/// Whether `map` can say what is at work. It cannot while something that tells of agents,
/// environments or activity goes unread, and then an empty project proves nothing.
pub fn can_tell_what_is_at_work(map: &Snapshot) -> bool {
    !map.problems
        .iter()
        .any(|problem| problem.source.tells_what_is_at_work())
}

/// Every Cargo build folder worth cleaning in a project that is idle, longest quiet first, and the
/// largest first among those quiet as long. None while `map` cannot say what is at work.
pub fn idle_folders(map: &Snapshot, now: SystemTime, at_least: Duration) -> Vec<IdleFolder<'_>> {
    if !can_tell_what_is_at_work(map) {
        return Vec::new();
    }
    let mut found: Vec<IdleFolder> = map
        .projects
        .iter()
        .filter(|project| project.access == FolderAccess::Readable)
        .filter(|project| is_idle(project, now, at_least))
        .flat_map(|project| {
            project
                .artifacts
                .iter()
                .filter(|dir| dir.kind == ArtifactKind::CargoTarget)
                .filter_map(move |dir| {
                    let bytes = dir.measurement.usage()?.bytes;
                    (bytes >= WORTH_CLEANING_BYTES).then_some(IdleFolder {
                        project,
                        dir,
                        bytes,
                    })
                })
        })
        .collect();
    found.sort_by_key(|folder| {
        (
            std::cmp::Reverse(quiet_for(folder.project, now)),
            std::cmp::Reverse(folder.bytes),
        )
    });
    found
}

/// The name of a project that shares `prepared`'s folders and is not idle, whose builds must not go
/// with the clean of an idle one's folder: one that shared them as the clean was worked out for the
/// project at `project`, or one `map` shows building in them now.
pub fn is_blocked_by_a_sharer(
    prepared: &Prepared,
    project: &Path,
    map: &Snapshot,
    now: SystemTime,
    at_least: Duration,
) -> Option<String> {
    let found_then = (prepared.sharing.project_folders.iter())
        .filter_map(|folder| map.projects.iter().find(|sharer| sharer.path == *folder));
    let building_now = building_in(&prepared.plan, project, map);
    let busy = found_then
        .chain(building_now)
        .find(|sharer| !is_idle(sharer, now, at_least))?;
    Some(busy.name.clone())
}

/// Whether `prepared`, worked out for a build folder of the idle project at `project`, may still go
/// as `map` shows things now. Its turn can come many minutes after it was worked out, and by then
/// an agent or an environment may have started there, or in a project sharing its folders.
pub fn is_still_idle(
    prepared: &Prepared,
    project: &Path,
    map: &Snapshot,
    now: SystemTime,
    at_least: Duration,
) -> bool {
    can_tell_what_is_at_work(map)
        && map
            .projects
            .iter()
            .find(|candidate| candidate.path == project)
            .is_some_and(|project| is_idle(project, now, at_least))
        && is_blocked_by_a_sharer(prepared, project, map, now, at_least).is_none()
}

/// One idle folder's clean, ready to ask about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleClean {
    /// The build folder on the map that led to this clean.
    pub clicked: PathBuf,
    pub project_name: String,
    pub project_path: PathBuf,
    pub prepared: Prepared,
}

/// An idle folder left out of the cleans, by the project it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub project_name: String,
    pub why: Why,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// Sutra will not clean it now.
    Refused(PrepareError),
    /// This project builds there too, and is not idle.
    SharedWith(String),
}

/// The cleans of every idle folder, longest quiet first, and the folders that cannot go now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdleCleans {
    pub cleans: Vec<IdleClean>,
    pub left_out: Vec<LeftOut>,
}

/// Work out the clean of every folder idle for `at_least`, but for those already being cleaned.
pub fn prepare_idle(
    map: &Snapshot,
    now: SystemTime,
    at_least: Duration,
    being_cleaned: &HashSet<PathBuf>,
    env: &CargoEnv,
) -> IdleCleans {
    let builds = Builds::of(map, env);
    let mut found = IdleCleans::default();
    let idle = idle_folders(map, now, at_least);
    let idle = idle
        .iter()
        .filter(|folder| !being_cleaned.contains(&folder.dir.path));
    for folder in idle {
        let project = folder.project;
        let left_out = |why| LeftOut {
            project_name: project.name.clone(),
            why,
        };
        match prepare_among(&folder.dir.path, &project.path, map, env, &builds) {
            Err(err) => found.left_out.push(left_out(Why::Refused(err))),
            Ok(prepared) => {
                match is_blocked_by_a_sharer(&prepared, &project.path, map, now, at_least) {
                    Some(sharer) => found.left_out.push(left_out(Why::SharedWith(sharer))),
                    None => found.cleans.push(IdleClean {
                        clicked: folder.dir.path.clone(),
                        project_name: project.name.clone(),
                        project_path: project.path.clone(),
                        prepared,
                    }),
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::clean::CleanError;
    use crate::map::clean::locks::tests::hold_like_cargo;
    use crate::map::test_support::{
        CACHEDIR_TAG, TempDir, agent_session, at, cargo_target, environment_at, measured,
        node_modules, project_at, snapshot_of, subagent_at_work,
    };
    use crate::map::{
        AgentSession, AgentStatus, Harness, KeptOutBy, Measurement, PrivateFolder, ScratchSpace,
        Source, SourceProblem, SubagentWithParent,
    };
    use std::io;

    const DAY: u64 = 86_400;
    const THREE_DAYS: Duration = Duration::from_secs(3 * DAY);
    const LARGE: u64 = 10 * WORTH_CLEANING_BYTES;

    fn now() -> SystemTime {
        at(1_000 * DAY)
    }

    /// A project at `/code/<name>` quiet for `days`, with a `target` of `bytes`.
    fn quiet_project(name: &str, days: u64, bytes: u64) -> Project {
        let path = format!("/code/{name}");
        Project {
            last_activity: Some(at((1_000 - days) * DAY)),
            artifacts: vec![cargo_target(format!("{path}/target"), measured(bytes))],
            ..project_at(path)
        }
    }

    fn names(map: &Snapshot) -> Vec<String> {
        idle_folders(map, now(), THREE_DAYS)
            .iter()
            .map(|folder| folder.project.name.clone())
            .collect()
    }

    #[test]
    fn longest_quiet_comes_first_and_the_largest_first_among_equals() {
        let map = snapshot_of(vec![
            quiet_project("recent", 3, LARGE),
            quiet_project("old-small", 9, LARGE),
            quiet_project("old-large", 9, 2 * LARGE),
            quiet_project("oldest", 20, LARGE),
        ]);
        assert_eq!(names(&map), ["oldest", "old-large", "old-small", "recent"]);
    }

    #[test]
    fn a_project_active_within_the_time_is_not_idle() {
        let map = snapshot_of(vec![
            quiet_project("yesterday", 1, LARGE),
            Project {
                last_activity: Some(at(2_000 * DAY)),
                ..quiet_project("dated-ahead-of-the-clock", 9, LARGE)
            },
        ]);
        assert!(names(&map).is_empty());
    }

    #[test]
    fn a_project_with_no_known_activity_counts_as_quiet() {
        let map = snapshot_of(vec![Project {
            last_activity: None,
            ..quiet_project("unknown", 0, LARGE)
        }]);
        assert_eq!(names(&map), ["unknown"]);
    }

    #[test]
    fn an_agent_or_a_live_environment_keeps_a_project_out_and_a_finished_job_or_dead_one_does_not()
    {
        let session = |harness, kind: &str, status| crate::map::AgentSession {
            harness,
            kind: Some(kind.to_string()),
            ..agent_session("/code/x", status)
        };
        let environment = |alive| {
            Some(crate::model::Environment {
                alive,
                ..environment_at("/code/x")
            })
        };
        let map = snapshot_of(vec![
            Project {
                agents: vec![session(Harness::Codex, "cli", AgentStatus::Idle)],
                ..quiet_project("agent-at-rest", 9, LARGE)
            },
            Project {
                environment: environment(true),
                ..quiet_project("environment-running", 9, LARGE)
            },
            Project {
                agents: vec![session(Harness::Codex, "exec", AgentStatus::Idle)],
                ..quiet_project("finished-job", 9, LARGE)
            },
            Project {
                environment: environment(false),
                ..quiet_project("environment-stopped", 9, LARGE)
            },
        ]);
        let mut found = names(&map);
        found.sort();
        assert_eq!(found, ["environment-stopped", "finished-job"]);
    }

    #[test]
    fn a_subagent_at_work_for_a_session_elsewhere_keeps_its_worktree_out() {
        let session = agent_session("/code/app", AgentStatus::Idle);
        let map = snapshot_of(vec![Project {
            subagents: vec![SubagentWithParent::new(subagent_at_work("a1"), &session)],
            ..quiet_project("agent-a1", 9, LARGE)
        }]);
        assert_eq!(names(&map), [""; 0]);
    }

    #[test]
    fn a_running_sessions_scratch_folder_is_kept_out_and_a_gone_ones_is_not() {
        let scratch = |name: &str, session: Option<crate::map::AgentSession>| Project {
            scratch: Some(crate::map::ScratchSpace {
                session_folder: PathBuf::from("/private/tmp/claude-501/-code-app").join(name),
                session_id: name.to_string(),
                started_in: "app".to_string(),
                session,
            }),
            ..quiet_project(name, 9, LARGE)
        };
        let running = agent_session("/code/app", AgentStatus::Idle);
        let map = snapshot_of(vec![
            scratch("e26ca359", Some(running)),
            scratch("d5fa0639", None),
        ]);
        assert_eq!(names(&map), ["d5fa0639"]);
    }

    #[test]
    fn only_a_cargo_build_folder_worth_cleaning_counts() {
        let map = snapshot_of(vec![
            quiet_project("emptied", 9, WORTH_CLEANING_BYTES - 1),
            Project {
                artifacts: vec![node_modules("/code/js/node_modules", measured(LARGE))],
                ..quiet_project("js", 9, LARGE)
            },
            Project {
                artifacts: vec![cargo_target("/code/unsized/target", Measurement::Measuring)],
                ..quiet_project("unsized", 9, LARGE)
            },
            Project {
                access: FolderAccess::KeptOut(PrivateFolder::Desktop, KeptOutBy::Settings),
                ..quiet_project("private", 9, LARGE)
            },
            quiet_project("worth-it", 9, WORTH_CLEANING_BYTES),
        ]);
        assert_eq!(names(&map), ["worth-it"]);
    }

    /// A built package in `fixture` at `name`, on the map as idle.
    fn idle_package(fixture: &TempDir, name: &str) -> Project {
        fixture.package(name);
        fixture.write(&format!("{name}/target/debug/deps/lib"), [1; 64 * 1024]);
        let path = fixture.path(name);
        let target = path.join("target");
        let usage = crate::map::disk::measure(&target).unwrap();
        Project {
            last_activity: Some(at((1_000 - 9) * DAY)),
            artifacts: vec![cargo_target(
                target,
                Measurement::Measured(crate::map::DirUsage {
                    bytes: LARGE,
                    ..usage
                }),
            )],
            ..project_at(path)
        }
    }

    fn cleaned_folders(found: &IdleCleans) -> Vec<PathBuf> {
        found
            .cleans
            .iter()
            .flat_map(|clean| clean.prepared.plan.folders())
            .map(std::path::Path::to_path_buf)
            .collect()
    }

    #[test]
    fn each_idle_folder_is_prepared_and_one_being_cleaned_or_used_by_a_build_is_not() {
        let fixture = TempDir::new("idle-prepare");
        let map = snapshot_of(vec![
            idle_package(&fixture, "one"),
            idle_package(&fixture, "two"),
            idle_package(&fixture, "three"),
        ]);
        let busy = fixture.write("three/target/debug/.cargo-build-lock", "");
        let _building = hold_like_cargo(&busy);
        let being_cleaned = HashSet::from([fixture.path("two/target")]);

        let found = prepare_idle(
            &map,
            now(),
            THREE_DAYS,
            &being_cleaned,
            &fixture.cargo_env(),
        );

        assert_eq!(cleaned_folders(&found), [fixture.path("one/target")]);
        assert_eq!(
            found.left_out,
            [LeftOut {
                project_name: "three".to_string(),
                why: Why::Refused(PrepareError::Locks(CleanError::BuildRunning { lock: busy })),
            }]
        );
    }

    /// An idle project `app` with a build folder, and a package `other` that builds in it too.
    fn app_and_a_package_building_in_it(fixture: &TempDir) -> (Project, Project) {
        let app = idle_package(fixture, "app");
        fixture.package("other");
        fixture.write(
            "other/.cargo/config.toml",
            "[build]\ntarget-dir = \"../app/target\"\n",
        );
        let other = Project {
            last_activity: Some(at((1_000 - 9) * DAY)),
            ..project_at(fixture.path("other"))
        };
        (app, other)
    }

    #[test]
    fn a_folder_an_idle_project_shares_is_cleaned_and_names_it() {
        let fixture = TempDir::new("idle-shared-idle");
        let (app, other) = app_and_a_package_building_in_it(&fixture);

        let found = prepare_idle(
            &snapshot_of(vec![app, other]),
            now(),
            THREE_DAYS,
            &HashSet::new(),
            &fixture.cargo_env(),
        );

        assert_eq!(cleaned_folders(&found), [fixture.path("app/target")]);
        assert_eq!(found.cleans[0].prepared.sharing.projects, ["other"]);
        assert!(found.left_out.is_empty());
    }

    #[test]
    fn a_member_registered_on_its_own_keeps_its_workspaces_folder_while_it_is_at_work() {
        let fixture = TempDir::new("idle-member");
        fixture.write(
            "ws/Cargo.toml",
            "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n",
        );
        fixture.package("ws/member");
        fixture.write("ws/target/CACHEDIR.TAG", CACHEDIR_TAG);
        let target = fixture.path("ws/target");
        let root = Project {
            last_activity: Some(at((1_000 - 9) * DAY)),
            artifacts: vec![cargo_target(target, measured(LARGE))],
            ..project_at(fixture.path("ws"))
        };
        let member = Project {
            last_activity: Some(at((1_000 - 9) * DAY)),
            ..project_at(fixture.path("ws/member"))
        };
        let prepared_with = |member: &Project| {
            prepare_idle(
                &snapshot_of(vec![root.clone(), member.clone()]),
                now(),
                THREE_DAYS,
                &HashSet::new(),
                &fixture.cargo_env(),
            )
        };

        let found = prepared_with(&member);
        assert_eq!(found.cleans.len(), 1, "{:?}", found.left_out);
        assert_eq!(
            found.cleans[0].prepared.sharing.projects,
            Vec::<String>::new(),
            "its own workspace is no news to the question"
        );

        let found = prepared_with(&Project {
            agents: vec![agent_session("/code/x", AgentStatus::Busy)],
            ..member.clone()
        });
        assert!(found.cleans.is_empty(), "{:?}", found.cleans);
        assert_eq!(
            found.left_out,
            [LeftOut {
                project_name: "ws".to_string(),
                why: Why::SharedWith("member".to_string()),
            }]
        );
    }

    #[test]
    fn a_project_at_work_whose_settings_cannot_be_read_may_share_the_folder_and_keeps_it() {
        let fixture = TempDir::new("idle-sharer-unknown");
        let app = idle_package(&fixture, "app");
        fixture.package("sealed");
        fixture.restrict(&fixture.dir("sealed/.cargo"), 0o000);
        let quiet = Project {
            last_activity: Some(at((1_000 - 9) * DAY)),
            ..project_at(fixture.path("sealed"))
        };
        let prepared_with = |sealed: &Project| {
            prepare_idle(
                &snapshot_of(vec![app.clone(), sealed.clone()]),
                now(),
                THREE_DAYS,
                &HashSet::new(),
                &fixture.cargo_env(),
            )
        };

        let found = prepared_with(&quiet);
        assert_eq!(found.cleans.len(), 1, "{:?}", found.left_out);
        assert_eq!(found.cleans[0].prepared.sharing.unknown, ["sealed"]);

        let found = prepared_with(&Project {
            agents: vec![agent_session("/code/x", AgentStatus::Busy)],
            ..quiet.clone()
        });
        assert!(found.cleans.is_empty(), "{:?}", found.cleans);
        assert_eq!(found.left_out[0].why, Why::SharedWith("sealed".to_string()));
    }

    #[test]
    fn a_clean_is_no_longer_idle_once_its_project_or_a_sharer_is_at_work() {
        let fixture = TempDir::new("idle-still");
        let (app, other) = app_and_a_package_building_in_it(&fixture);
        let quiet = snapshot_of(vec![app.clone(), other.clone()]);
        let found = prepare_idle(
            &quiet,
            now(),
            THREE_DAYS,
            &HashSet::new(),
            &fixture.cargo_env(),
        );
        let prepared = &found.cleans[0].prepared;
        let still_idle =
            |map: &Snapshot| is_still_idle(prepared, &app.path, map, now(), THREE_DAYS);
        let at_work = |project: &Project| Project {
            agents: vec![agent_session("/code/x", AgentStatus::Busy)],
            ..project.clone()
        };

        assert!(still_idle(&quiet));
        assert!(!still_idle(&snapshot_of(vec![
            at_work(&app),
            other.clone()
        ])));
        assert!(!still_idle(&snapshot_of(vec![
            app.clone(),
            at_work(&other)
        ])));
        assert!(
            !still_idle(&snapshot_of(vec![other.clone()])),
            "it left the map"
        );
        let cannot_tell = Snapshot {
            problems: vec![session_file_unread()],
            ..quiet.clone()
        };
        assert!(!still_idle(&cannot_tell));
    }

    fn session_file_unread() -> SourceProblem {
        SourceProblem::new(
            Source::ClaudeSessionFile,
            PathBuf::from("/Users/dev/.claude/sessions/1.json"),
            io::ErrorKind::PermissionDenied,
        )
    }

    #[test]
    fn nothing_is_idle_while_what_tells_of_work_goes_unread() {
        let map_with = |source| Snapshot {
            problems: vec![SourceProblem::new(
                source,
                PathBuf::from("/somewhere"),
                io::ErrorKind::PermissionDenied,
            )],
            ..snapshot_of(vec![quiet_project("app", 9, LARGE)])
        };
        for source in [
            Source::Registry,
            Source::EnvironmentState,
            Source::ClaudeSessions,
            Source::ClaudeSessionFile,
            Source::CodexDatabase,
            Source::Watcher,
        ] {
            assert_eq!(names(&map_with(source)), [""; 0], "{source:?}");
        }
        assert_eq!(
            names(&map_with(Source::Scan)),
            ["app"],
            "a folder discovery could not list hides no agent"
        );
        assert_eq!(
            names(&map_with(Source::ClaudeSubagentMeta)),
            ["app"],
            "a subagent's meta file only names it"
        );
    }

    #[test]
    fn a_folder_a_project_at_work_or_recently_active_shares_is_left_out() {
        let fixture = TempDir::new("idle-shared-busy");
        let (app, other) = app_and_a_package_building_in_it(&fixture);
        let at_work = Project {
            agents: vec![agent_session("/code/x", AgentStatus::Busy)],
            ..other.clone()
        };
        let recently_active = Project {
            last_activity: Some(at(1_000 * DAY - 60)),
            ..other
        };

        for sharer in [at_work, recently_active] {
            let found = prepare_idle(
                &snapshot_of(vec![app.clone(), sharer]),
                now(),
                THREE_DAYS,
                &HashSet::new(),
                &fixture.cargo_env(),
            );

            assert!(found.cleans.is_empty(), "{:?}", found.cleans);
            assert_eq!(
                found.left_out,
                [LeftOut {
                    project_name: "app".to_string(),
                    why: Why::SharedWith("other".to_string()),
                }]
            );
        }
    }

    /// An idle package `app` whose build-dir is a session's scratch folder, which `CARGO_TARGET_DIR`
    /// named: the session builds there too, and the scratch folder, quiet as long as `app`, is on
    /// the map with `session` running, or none.
    fn app_building_in_a_scratch_folder(
        fixture: &TempDir,
        session: Option<AgentSession>,
    ) -> (Project, Project) {
        let app = idle_package(fixture, "app");
        let session_folder = fixture.path("claude-501/-Users-me/e26ca359");
        let tag = session_folder.join("scratchpad/target-567-fix/CACHEDIR.TAG");
        let scratch = tag.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(&tag, CACHEDIR_TAG).unwrap();
        fixture.write(
            "app/.cargo/config.toml",
            format!("[build]\nbuild-dir = \"{}\"\n", scratch.display()),
        );
        let space = ScratchSpace {
            session_folder,
            session_id: "e26ca359".to_string(),
            started_in: "foundry".to_string(),
            session,
        };
        let builder = Project {
            name: space.name(),
            scratch: Some(space),
            last_activity: Some(at((1_000 - 9) * DAY)),
            own_files: measured(0),
            artifacts: vec![cargo_target(scratch.clone(), measured(LARGE))],
            ..project_at(&scratch)
        };
        (app, builder)
    }

    #[test]
    fn a_folder_a_running_sessions_scratch_folder_builds_in_is_left_out_though_no_workspace_says_so()
     {
        let fixture = TempDir::new("idle-shared-scratch");
        let running = agent_session("e26ca359", AgentStatus::Idle);
        let (app, live) = app_building_in_a_scratch_folder(&fixture, Some(running));
        let env = fixture.cargo_env();
        let prepared_with = |builder: &Project| {
            let map = snapshot_of(vec![app.clone(), builder.clone()]);
            prepare_idle(&map, now(), THREE_DAYS, &HashSet::new(), &env)
        };

        let found = prepared_with(&live);
        assert!(found.cleans.is_empty(), "{:?}", found.cleans);
        assert_eq!(
            found.left_out,
            [LeftOut {
                project_name: "app".to_string(),
                why: Why::SharedWith(live.name.clone()),
            }]
        );

        let ended = Project {
            scratch: live.scratch.clone().map(|space| ScratchSpace {
                session: None,
                ..space
            }),
            ..live.clone()
        };
        let found = prepared_with(&ended);
        let of_app = found
            .cleans
            .iter()
            .find(|clean| clean.project_path == app.path)
            .expect("the session ended, so the clean of app goes");
        assert_eq!(of_app.prepared.sharing.projects, [ended.name.as_str()]);
        assert!(
            !is_still_idle(
                &of_app.prepared,
                &app.path,
                &snapshot_of(vec![app.clone(), live.clone()]),
                now(),
                THREE_DAYS
            ),
            "a session running there again by its turn keeps it"
        );
    }

    #[test]
    fn a_session_building_in_an_idle_folder_since_its_clean_was_worked_out_keeps_it() {
        let fixture = TempDir::new("idle-shared-scratch-later");
        let running = agent_session("e26ca359", AgentStatus::Idle);
        let (app, live) = app_building_in_a_scratch_folder(&fixture, Some(running));
        let before = snapshot_of(vec![app.clone()]);
        let found = prepare_idle(
            &before,
            now(),
            THREE_DAYS,
            &HashSet::new(),
            &fixture.cargo_env(),
        );
        let prepared = &found.cleans[0].prepared;

        assert!(is_still_idle(
            prepared,
            &app.path,
            &before,
            now(),
            THREE_DAYS
        ));
        let now_on_map = snapshot_of(vec![app.clone(), live]);
        assert!(!is_still_idle(
            prepared,
            &app.path,
            &now_on_map,
            now(),
            THREE_DAYS
        ));
    }
}
