//! The groups' idle build folders cleaned by themselves: when the groups are looked at, what a look
//! queues and what it leaves to a clean or a question already there, how a round ends in one
//! banner, and what a changed choice, a project back at work or a failure does to the cleans.

use std::io;

use super::*;
use crate::dashboard::cleaning::CleanOutcome;
use crate::dashboard::cleaning::queue::Running;
use crate::dashboard::cleaning::tests::{
    cleaning, idle_clean, next_report, prepared_at, reporting,
};
use crate::map::clean::{LeftOut, Progress};
use crate::map::test_support::{
    TempDir, agent_session, at, cargo_target, measured, project_at, snapshot_of,
};
use crate::map::{AgentStatus, Provenance};

const DAY_SECS: u64 = 24 * 60 * 60;
const LARGE: u64 = 100 * clean::WORTH_CLEANING_BYTES;

fn now() -> SystemTime {
    at(1_000 * DAY_SECS)
}

/// The group of the clones of `github.com/me/<name>`.
fn upstream(name: &str) -> GroupKey {
    GroupKey::Upstream(format!("github.com/me/{name}"))
}

fn in_group(group: GroupKey, project: Project) -> Project {
    Project {
        provenance: Provenance {
            group,
            made_from: Vec::new(),
        },
        ..project
    }
}

/// A package `name` in `fixture`, standing in `group`, quiet for `days`, whose build folder
/// holds a file and is measured as large.
fn checkout(fixture: &TempDir, name: &str, group: GroupKey, days: u64) -> Project {
    fixture.package(name);
    fixture.write(&format!("{name}/target/debug/deps/lib"), [1; 64 * 1024]);
    let target = cargo_target(fixture.path(&format!("{name}/target")), measured(LARGE));
    let project = Project {
        last_activity: Some(at((1_000 - days) * DAY_SECS)),
        artifacts: vec![target],
        ..project_at(fixture.path(name))
    };
    in_group(group, project)
}

/// The choice of cleaning `group`'s build folders once idle for `days`, on `map`.
fn chosen(group: &GroupKey, days: u32, map: &Snapshot) -> CleanWhenIdle {
    let mut chosen = CleanWhenIdle::default();
    assert!(chosen.choose(group, Some(days), map));
    chosen
}

#[test]
fn a_groups_idle_folders_go_unasked_and_one_banner_says_what_went_once_the_last_ends() {
    let fixture = TempDir::new("when-idle-round");
    let busy = Project {
        agents: vec![agent_session("s1", AgentStatus::Busy)],
        ..checkout(&fixture, "app-busy", upstream("app"), 9)
    };
    let map = Arc::new(snapshot_of(vec![
        checkout(&fixture, "app", upstream("app"), 9),
        checkout(&fixture, "app-old", upstream("app"), 30),
        busy,
        checkout(&fixture, "site", upstream("site"), 30),
    ]));
    let chosen = chosen(&upstream("app"), 3, &map);
    let (mut cleaning, inbox) = reporting();
    let home = fixture.root();

    cleaning.clean_when_idle(&map, &chosen, Instant::now(), now(), home);
    cleaning.receive(next_report(&inbox), Some(&map), now(), home);
    assert_eq!(cleaning.running.len(), 2, "the idle checkouts' folders");
    assert!(cleaning.take_alerts().is_empty(), "no question asked");

    cleaning.receive(next_report(&inbox), Some(&map), now(), home);
    assert!(cleaning.take_banners().is_empty(), "one clean still runs");
    cleaning.receive(next_report(&inbox), Some(&map), now(), home);

    let banners = cleaning.take_banners();
    assert_eq!(banners.len(), 1, "{banners:?}");
    assert_eq!(banners[0].0, "Cleaned app's idle build folders");
    assert!(
        banners[0]
            .1
            .starts_with("2 folders idle for 3 days or more: "),
        "{}",
        banners[0].1
    );
    let built = |name: &str| fixture.path(&format!("{name}/target/debug/deps/lib"));
    assert!(!built("app").exists());
    assert!(!built("app-old").exists());
    assert!(built("app-busy").exists(), "an agent is at work in it");
    assert!(built("site").exists(), "its group has no time chosen");
}

#[test]
fn the_groups_are_looked_at_once_a_time_is_chosen_then_hourly_and_soon_after_a_change() {
    let map = Arc::new(snapshot_of(vec![in_group(
        upstream("app"),
        project_at("/code/app"),
    )]));
    let home = Path::new("/nowhere");
    let look = |cleaning: &mut Cleaning, chosen: &CleanWhenIdle, at| {
        cleaning.clean_when_idle(&map, chosen, at, now(), home);
        let looked = cleaning.when_idle.preparing;
        if looked {
            let found = Some(IdleCleans::default());
            cleaning.when_idle_prepared(found, Some(&map), now(), home);
        }
        looked
    };
    let mut cleaning = cleaning();
    let start = Instant::now();

    let nothing = CleanWhenIdle::default();
    assert!(!look(&mut cleaning, &nothing, start), "no time chosen");
    let chosen = chosen(&upstream("app"), 3, &map);
    assert!(look(&mut cleaning, &chosen, start));
    assert!(!look(&mut cleaning, &chosen, start + CHECK_EVERY / 2));
    assert!(look(&mut cleaning, &chosen, start + CHECK_EVERY));
    cleaning.when_idle_changed(&upstream("app"));
    let soon = start + CHECK_EVERY + Duration::from_secs(2);
    assert!(look(&mut cleaning, &chosen, soon));
}

#[test]
fn no_look_starts_while_a_clean_is_under_way() {
    let fixture = TempDir::new("when-idle-busy");
    let target = fixture.dir("card/target");
    let map = Arc::new(snapshot_of(vec![in_group(
        upstream("app"),
        project_at("/code/app"),
    )]));
    let mut cleaning = cleaning();
    cleaning.queue(target.clone(), prepared_at(&target), By::User);

    let chosen = chosen(&upstream("app"), 3, &map);
    cleaning.clean_when_idle(&map, &chosen, Instant::now(), now(), fixture.root());
    assert!(!cleaning.when_idle.preparing);
    assert_eq!(cleaning.when_idle.next_check, None, "it looks once free");
}

#[test]
fn what_a_look_found_before_the_user_changed_a_groups_time_is_dropped_and_looked_for_again() {
    let fixture = TempDir::new("when-idle-outdated");
    let app = checkout(&fixture, "app", upstream("app"), 9);
    let target = app.artifacts[0].path.clone();
    let map = Arc::new(snapshot_of(vec![app]));
    let chosen = chosen(&upstream("app"), 3, &map);
    let mut cleaning = cleaning();
    cleaning.clean_when_idle(&map, &chosen, Instant::now(), now(), fixture.root());

    cleaning.when_idle_changed(&upstream("app"));
    let found = IdleCleans {
        cleans: vec![idle_clean(&[&target])],
        left_out: Vec::new(),
    };
    cleaning.when_idle_prepared(Some(found), Some(&map), now(), fixture.root());

    assert!(cleaning.waiting.is_empty() && cleaning.running.is_empty());
    assert!(!cleaning.is_busy());
    assert_eq!(cleaning.when_idle.next_check, None, "at the next tick");
    assert!(fixture.path("app/target/debug/deps/lib").exists());
}

/// `WhenIdle` for the clean of the project at `project`, in `group`, once idle for 3 days.
fn when_idle(project: &Path, group: GroupKey) -> By {
    By::WhenIdle {
        project: project.to_path_buf(),
        group,
        quiet_at_least: DAY * 3,
    }
}

#[test]
fn the_cleans_waiting_go_back_when_the_user_changes_their_groups_time() {
    let fixture = TempDir::new("when-idle-changed");
    let app = fixture.dir("app/target");
    let site = fixture.dir("site/target");
    let mut cleaning = cleaning();
    let by_app = when_idle(&fixture.path("app"), upstream("app"));
    cleaning.queue(app.clone(), prepared_at(&app), by_app);
    let by_site = when_idle(&fixture.path("site"), upstream("site"));
    cleaning.queue(site.clone(), prepared_at(&site), by_site);

    cleaning.when_idle_changed(&upstream("app"));

    let waiting: Vec<&PathBuf> = cleaning.waiting.iter().map(|w| &w.clicked).collect();
    assert_eq!(waiting, [&site]);
    let app_card = cargo_target(app, measured(LARGE));
    assert_eq!(
        cleaning.state_of(&app_card),
        None,
        "its card shows its size"
    );
}

#[test]
fn a_clean_goes_at_its_turn_only_while_its_project_is_still_idle_as_long_as_its_group_says() {
    let fixture = TempDir::new("when-idle-turn");
    let target = fixture.dir("app/target");
    let project = fixture.path("app");
    let quiet_for = |days: u64| {
        snapshot_of(vec![Project {
            last_activity: Some(at((1_000 - days) * DAY_SECS)),
            artifacts: vec![cargo_target(target.clone(), measured(LARGE))],
            ..in_group(upstream("app"), project_at(project.clone()))
        }])
    };
    let started = |map: &Snapshot| {
        let mut cleaning = cleaning();
        let by = when_idle(&project, upstream("app"));
        cleaning.queue(target.clone(), prepared_at(&target), by);
        cleaning.start_next(Some(map), now(), fixture.root());
        !cleaning.running.is_empty()
    };

    assert!(!started(&quiet_for(1)), "active since the look");
    assert!(started(&quiet_for(9)));
}

#[test]
fn a_folder_the_user_cleans_or_a_question_asks_about_meanwhile_is_left_to_them() {
    let fixture = TempDir::new("when-idle-taken");
    let asked = checkout(&fixture, "asked", upstream("app"), 9);
    let clicked = checkout(&fixture, "clicked", upstream("app"), 9);
    let (asked_target, clicked_target) = (&asked.artifacts[0].path, &clicked.artifacts[0].path);
    let map = Arc::new(snapshot_of(vec![asked.clone(), clicked.clone()]));
    let home = fixture.root();
    let mut cleaning = cleaning();
    cleaning.start(&asked, &asked.artifacts[0], Arc::clone(&map), home);
    let mut reaching = prepared_at(asked_target);
    reaching.sharing.projects = vec!["fwd".to_string()];
    cleaning.asking(asked_target, Some(Ok(reaching)), None, now(), home);
    assert_eq!(cleaning.take_alerts().len(), 1, "the question is open");
    let by_user = prepared_at(clicked_target);
    cleaning.queue(clicked_target.clone(), by_user, By::User);

    let found = IdleCleans {
        cleans: vec![idle_clean(&[asked_target]), idle_clean(&[clicked_target])],
        left_out: Vec::new(),
    };
    cleaning.when_idle_prepared(Some(found), Some(&map), now(), home);

    let wanted: Vec<&By> = (cleaning.waiting.iter().map(|waiting| &waiting.by))
        .chain(cleaning.running.iter().map(|running| &running.by))
        .collect();
    assert_eq!(wanted, [&By::User], "the user's clean alone");
}

#[test]
fn a_clean_that_failed_is_told_and_its_folder_passed_over_for_a_day_and_a_busy_build_is_not() {
    let fixture = TempDir::new("when-idle-failed");
    let app = checkout(&fixture, "app", upstream("app"), 9);
    let target = app.artifacts[0].path.clone();
    let deleting = fixture.path("old/target");
    let building = fixture.path("busy/target");
    let map = Arc::new(snapshot_of(vec![app]));
    let home = fixture.root();
    let mut cleaning = cleaning();
    let left_out = |folder: &Path, err| LeftOut {
        project_name: "app".to_string(),
        folder: folder.to_path_buf(),
        why: Why::Refused(PrepareError::Locks(err)),
    };
    let cannot_lock = CleanError::Io {
        path: target.join("debug/.cargo-lock"),
        kind: io::ErrorKind::PermissionDenied,
    };
    let build_running = CleanError::BuildRunning {
        lock: building.join("debug/.cargo-lock"),
    };
    let found = IdleCleans {
        cleans: Vec::new(),
        left_out: vec![
            left_out(&target, cannot_lock),
            left_out(&building, build_running),
        ],
    };
    cleaning.when_idle_prepared(Some(found), Some(&map), now(), home);
    cleaning.running.push(Running {
        clicked: deleting.clone(),
        folders: vec![deleting.clone()],
        by: when_idle(&fixture.path("old"), upstream("app")),
        progress: Arc::new(Progress::default()),
    });
    let failed = CleanOutcome::Failed(CleanError::Io {
        path: deleting.join("debug"),
        kind: io::ErrorKind::PermissionDenied,
    });
    cleaning.finished(&deleting, failed, Some(&map), now(), home);
    cleaning.when_idle_prepared(None, Some(&map), now(), home);

    let titles: Vec<String> = cleaning.take_banners().into_iter().map(|b| b.0).collect();
    assert_eq!(titles.len(), 3, "{titles:?}");
    assert!(
        titles[2].starts_with("Couldn't look"),
        "a look that ended with no word"
    );
    let passed_over = |folder: &Path| cleaning.when_idle.passed_over.contains_key(folder);
    assert!(passed_over(&target) && passed_over(&deleting));
    assert!(!passed_over(&building), "a build at work passes");

    let chosen = chosen(&upstream("app"), 3, &map);
    let a_day_on = now() + PASS_OVER_FOR;
    cleaning.clean_when_idle(&map, &chosen, Instant::now(), a_day_on, home);
    assert!(
        cleaning.when_idle.passed_over.is_empty(),
        "each gets another try"
    );
    assert!(cleaning.when_idle.preparing);
}
