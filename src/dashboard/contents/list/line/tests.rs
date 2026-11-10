//! A line as the window receives it: what it names and says of what it holds, its total and the
//! clean it offers, on a group's the choice of cleaning by itself once idle and the mark of it,
//! and what it shows folded shut.

use std::path::Path;

use super::*;
use crate::dashboard::cleaning::CleanWhenIdle;
use crate::dashboard::cleaning::tests::{cleaning, deleted};
use crate::dashboard::contents::list;
use crate::dashboard::listing::fixtures::{
    agent, came_from, linked_worktree, main_worktree, now, project, quiet, running_as,
    scratch_folder, snapshot, subagent_in, upstream,
};
use crate::dashboard::listing::{Arrangement, Folds, Listing};
use crate::dashboard::{AgentLook, ListRow, Place};
use crate::map::docker::Container;
use crate::map::test_support::{cargo_target, measured, node_modules, project_at, usage};
use crate::map::{AgentStatus, Measurement, Snapshot};
use crate::notifications::Notifier;

/// What the cards read, with `cleaning` at work.
fn context<'a>(cleaning: &'a Cleaning, notifier: &'a Notifier) -> CardContext<'a> {
    CardContext {
        notifier,
        cleaning,
        home: Path::new("/Users/dev"),
        now: now(),
    }
}

/// The rows the Activity sort lists for `map`, the lines in `open` opened.
pub(in crate::dashboard::contents) fn rows_of(map: &Snapshot, open: &[LineKey]) -> Vec<ListRow> {
    rows_cleaning_when_idle(map, open, &CleanWhenIdle::default())
}

/// The rows `rows_of` lists, with the groups `clean_when_idle` chose cleaned once idle.
fn rows_cleaning_when_idle(
    map: &Snapshot,
    open: &[LineKey],
    clean_when_idle: &CleanWhenIdle,
) -> Vec<ListRow> {
    let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
    let mut folds = Folds::default();
    for line in open {
        assert!(folds.toggle(line, map), "{line:?} is on the map");
    }
    let listing = Listing::new(map, &Arrangement::default(), &cleaning, now());
    let roots = [std::path::PathBuf::from("/code")];
    let chosen = list::LineChoices {
        folds: &folds,
        clean_when_idle,
    };
    let shown = list::list(
        Some(&listing),
        None,
        &roots,
        None,
        false,
        chosen,
        &context(&cleaning, &notifier),
    );
    let crate::dashboard::List::Rows { rows } = shown else {
        panic!("rows: {shown:?}");
    };
    rows
}

/// The lines among `rows`, with their places.
fn lines(rows: Vec<ListRow>) -> Vec<(Line, Place)> {
    let lines = rows.into_iter().filter_map(|row| match row {
        ListRow::Line { line, place } => Some((*line, place)),
        _ => None,
    });
    lines.collect()
}

fn app_line() -> LineKey {
    LineKey::Group {
        kind: GroupKind::Upstream {
            remote: "github.com/me/app".to_string(),
        },
    }
}

/// A clone of `github.com/me/app` at `/code/<name>`, made from the projects at `made_from`.
fn app(name: &str, made_from: &[&str], project: Project) -> Project {
    let named = Project {
        path: Path::new("/code").join(name),
        name: name.to_string(),
        ..project
    };
    came_from(upstream("github.com/me/app"), made_from, named)
}

#[test]
fn a_groups_line_names_its_upstream_and_adds_up_all_it_holds_on_the_map() {
    let sized = |bytes, project| Project {
        own_files: measured(bytes),
        ..project
    };
    let map = snapshot(
        vec![
            sized(2_000_000_000, app("app", &[], project("", 1))),
            app("app-2", &[], project("", 1)),
            sized(500_000_000, app("app-old", &[], quiet(""))),
        ],
        vec![],
    );
    let [(line, place)] = &lines(rows_of(&map, &[]))[..] else {
        panic!("one line");
    };
    assert_eq!(line.key, app_line());
    assert_eq!(line.name, "app");
    assert_eq!(line.tooltip, "github.com/me/app");
    assert_eq!(
        line.total.as_deref(),
        Some("2.5 GB"),
        "the quiet clone's size too, and nothing yet of the one being sized"
    );
    assert_eq!(
        *place,
        Place {
            depth: 0,
            first: false
        }
    );
}

#[test]
fn a_line_standing_for_its_groups_one_checkout_says_where_that_is() {
    let main = main_worktree("/Users/dev/code/notes/.git", "main", {
        Project {
            path: Path::new("/Users/dev/code/notes").to_path_buf(),
            ..project("notes", 1)
        }
    });
    let draft = linked_worktree(
        "/Users/dev/code/notes/.git",
        "draft",
        project("notes-draft", 1),
    );
    let map = snapshot(vec![main, draft], vec![]);
    let [(line, _)] = &lines(rows_of(&map, &[]))[..] else {
        panic!("one line");
    };
    assert_eq!(
        line.tooltip, "~/code/notes\nA repository with no remote",
        "the checkout it stands for is the folder its key names, said once"
    );
}

#[test]
fn a_checkouts_line_goes_by_its_folder_and_holds_its_copies_open_beneath_it() {
    let map = snapshot(
        vec![
            app("app", &[], project("", 1)),
            app("app-wt", &["/code/app"], project("", 1)),
            app("app-2", &[], project("", 30)),
        ],
        vec![],
    );
    let checkout = LineKey::Checkout {
        folder: "/code/app".to_string(),
    };
    let rows = rows_of(&map, &[app_line(), checkout.clone()]);
    let shown: Vec<(String, u32, bool)> = rows
        .iter()
        .map(|row| match row {
            ListRow::Line { line, place } => {
                (format!("line {}", line.name), place.depth, place.first)
            }
            ListRow::Card { card, place } => (card.name.clone(), place.depth, place.first),
            row => panic!("{row:?}"),
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("line app".to_string(), 0, false),
            ("line app".to_string(), 1, true),
            ("app".to_string(), 2, true),
            ("app-wt".to_string(), 2, false),
            ("app-2".to_string(), 1, false),
        ],
        "the group's line, the checkout's under it holding its own card and its copy, then the \
         other clone"
    );
    let [_, (line, _)] = &lines(rows)[..] else {
        panic!("two lines");
    };
    assert_eq!(line.key, checkout);
    assert_eq!(line.tooltip, "/code/app");
}

#[test]
fn a_folded_line_shows_the_robots_of_all_it_holds_sessions_first_and_throbs_for_one_waiting() {
    let map = |second_status| {
        let mut first = app("app", &[], project("", 1));
        first.own_files = measured(2_000_000_000);
        first.agents.push(agent("/code/app", AgentStatus::Busy));
        first.subagents.push(subagent_in("/code/app"));
        let mut second = app("app-2", &[], project("", 1));
        second.agents.push(agent("/code/app-2", second_status));
        snapshot(vec![first, second], vec![])
    };
    let folded = |second_status| {
        let [(line, _)] = &lines(rows_of(&map(second_status), &[]))[..] else {
            panic!("one line");
        };
        let LineFold::Folded {
            sessions,
            subagents,
            waiting,
            ..
        } = &line.fold
        else {
            panic!("folded: {:?}", line.fold);
        };
        let looks: Vec<AgentLook> = sessions.iter().map(|badge| badge.look).collect();
        (line.total.clone(), looks, subagents.len(), *waiting)
    };
    assert_eq!(
        folded(AgentStatus::Waiting),
        (
            Some("2 GB".to_string()),
            vec![AgentLook::Working, AgentLook::Waiting],
            1,
            true
        ),
        "app's session before app-2's, then the subagent, throbbing for app-2's"
    );
    let (.., waiting) = folded(AgentStatus::Busy);
    assert!(!waiting, "no throb while no session waits");
}

#[test]
fn a_folded_line_shows_two_of_the_servers_up_in_what_it_holds_and_counts_the_rest() {
    use crate::map::test_support::environment_at;
    use crate::model::{Environment, UnitStatus};
    // Each unit as `name:status:port`, in the environment of the clone at `path`.
    let serving = |path: &str, units: &[(&str, &str, u16)]| Environment {
        units: units
            .iter()
            .map(|(name, status, _)| UnitStatus::parse(name, status))
            .collect(),
        ports: units
            .iter()
            .map(|(name, _, port)| (name.to_string(), *port))
            .collect(),
        ..environment_at(path)
    };
    let mut first = app("app", &[], project("", 1));
    first.environment = Some(serving(
        "/code/app",
        &[("web", "ready", 5173), ("worker", "stopped", 9000)],
    ));
    let mut second = app("app-2", &[], project("", 1));
    second.environment = Some(serving(
        "/code/app-2",
        &[("api", "starting", 8787), ("docs", "running", 4000)],
    ));
    let map = snapshot(vec![first, second], vec![]);
    let [(line, _)] = &lines(rows_of(&map, &[]))[..] else {
        panic!("one line");
    };
    let LineFold::Folded {
        lozenges,
        more_lozenges,
        ..
    } = &line.fold
    else {
        panic!("folded: {:?}", line.fold);
    };
    let shown: Vec<(&str, Option<&str>)> = lozenges
        .iter()
        .map(|on_line| {
            (
                on_line.lozenge.name.as_deref().unwrap(),
                on_line.lozenge.port.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [("web", Some(":5173")), ("api", Some(":8787"))],
        "up or on its way, in the cards' order; not the stopped worker"
    );
    assert_eq!(
        lozenges[0].lozenge.tooltip, "web in /code/app\nready",
        "the folder it serves in, which the folded line does not show, then its card's tip"
    );
    assert_eq!(
        lozenges[1].lozenge.link.as_deref(),
        Some("http://localhost:8787"),
        "a click opens the port, as on its card"
    );
    let counted = |label: &str, tooltip: &str| MoreLozenges {
        label: label.to_string(),
        tooltip: tooltip.to_string(),
    };
    assert_eq!(*more_lozenges, Some(counted("+1", "docs :4000")));
    let giving_way: Vec<&MoreLozenges> = lozenges.iter().map(|l| &l.gives_way_to).collect();
    assert_eq!(
        giving_way,
        [
            &counted("+3", "web :5173\napi :8787\ndocs :4000"),
            &counted("+2", "api :8787\ndocs :4000"),
        ],
        "each counts itself and every lozenge after it, should the line need its room"
    );
}

#[test]
fn a_folded_line_shows_its_cards_processes_and_containers_and_a_container_on_two_of_them_once() {
    use crate::map::test_support::{container_for, listening_in};
    let both = || Container {
        bound: vec!["/code/app".into(), "/code/app-2".into()],
        ..container_for("/code/app", "app-db-1")
    };
    let mut first = app("app", &[], project("", 1));
    first.listening = vec![listening_in("/code/app", "node", &[5177])];
    first.containers = vec![both()];
    let mut second = app("app-2", &[], project("", 1));
    second.containers = vec![both()];
    let map = snapshot(vec![first, second], vec![]);
    let [(line, _)] = &lines(rows_of(&map, &[]))[..] else {
        panic!("one line");
    };
    let LineFold::Folded {
        lozenges,
        more_lozenges,
        ..
    } = &line.fold
    else {
        panic!("folded: {:?}", line.fold);
    };
    let shown: Vec<(&str, Option<&str>)> = lozenges
        .iter()
        .map(|on_line| {
            (
                on_line.lozenge.name.as_deref().unwrap(),
                on_line.lozenge.port.as_deref(),
            )
        })
        .collect();
    assert_eq!(shown, [("node", Some(":5177")), ("postgres", None)]);
    assert_eq!(*more_lozenges, None, "the container on both cards, once");
    assert_eq!(
        lozenges[1].lozenge.tooltip,
        "postgres in /code/app and /code/app-2\nContainer app-db-1, of postgres:17, publishing \
         no TCP port\n/code/app"
    );
}

#[test]
fn others_folded_line_carries_the_robots_of_the_agents_working_elsewhere_first() {
    let mut notes = project("notes", 1);
    notes.agents.push(agent("/code/notes", AgentStatus::Busy));
    let map = snapshot(
        vec![notes],
        vec![agent("/tmp/review", AgentStatus::Waiting)],
    );
    let [(line, _)] = &lines(rows_of(&map, &[]))[..] else {
        panic!("Other's line alone");
    };
    let LineFold::Folded {
        sessions, waiting, ..
    } = &line.fold
    else {
        panic!("folded: {:?}", line.fold);
    };
    let looks: Vec<AgentLook> = sessions.iter().map(|badge| badge.look).collect();
    assert_eq!(looks, [AgentLook::Waiting, AgentLook::Working]);
    assert!(waiting, "throbbing for the agent waiting elsewhere");
}

#[test]
fn a_line_offers_to_clean_what_it_holds_and_nothing_in_a_folder_it_cannot_read() {
    let scratchpad = "/private/tmp/claude-501/-code-app/e26ca359/scratchpad";
    let with = |folder: &str, artifacts: Vec<ArtifactDir>| {
        let scratch = Project {
            artifacts,
            ..scratch_folder("e26ca359", folder, project("", 1))
        };
        came_from(
            upstream("github.com/me/app"),
            &[],
            running_as("Foundry", scratch),
        )
    };
    let r567 = format!("{scratchpad}/r567");
    let fix = format!("{scratchpad}/target-567-fix");
    let map = |fix_measurement, fix_access| {
        let mut fix_folder = with("target-567-fix", vec![cargo_target(&fix, fix_measurement)]);
        fix_folder.access = fix_access;
        snapshot(
            vec![
                with(
                    "r567",
                    vec![
                        cargo_target(format!("{r567}/target"), measured(60_000_000_000)),
                        node_modules(format!("{r567}/ui/node_modules"), measured(1 << 30)),
                    ],
                ),
                fix_folder,
            ],
            vec![],
        )
    };
    let menu = |map: &Snapshot| {
        let [(line, _)] = &lines(rows_of(map, &[]))[..] else {
            panic!("one line");
        };
        line.menu.clone()
    };

    let readable = map(measured(30_000_000_000), FolderAccess::Readable);
    let [MenuEntry::Item { title, action, .. }, ..] = &menu(&readable)[..] else {
        panic!("the clean first");
    };
    assert_eq!(
        title, "Clean Build Folders (90 GB)",
        "the two Cargo build folders, not the line's whole total"
    );
    assert_eq!(
        *action,
        MenuAction::Dashboard {
            action: Action::CleanBuildFolders { line: app_line() }
        }
    );
    let sizing = map(Measurement::Measuring, FolderAccess::Readable);
    let [MenuEntry::Item { title, .. }, ..] = &menu(&sizing)[..] else {
        panic!("the clean first");
    };
    assert_eq!(
        title, "Clean Build Folders (\u{2265}60 GB)",
        "one not sized yet"
    );
    let sealed = map(measured(30_000_000_000), FolderAccess::Denied);
    let [MenuEntry::Item { title, .. }, ..] = &menu(&sealed)[..] else {
        panic!("the clean first");
    };
    assert_eq!(
        title, "Clean Build Folders (60 GB)",
        "not the one in the folder that cannot be read, where the clean would fail"
    );
}

#[test]
fn a_lines_total_waits_for_a_first_size_and_says_when_it_is_a_lower_bound() {
    let (cleaning, notifier) = (cleaning(), Notifier::new(|_, _| {}));
    let cards = context(&cleaning, &notifier);
    let being_sized = project_at("/code/app");
    let read_in_part = Project {
        own_files: Measurement::Measured(DirUsage {
            unreadable_dirs: 1,
            ..usage(5_000_000)
        }),
        ..project_at("/code/app-fix")
    };
    assert_eq!(total_of(&[&being_sized], &cards), None);
    assert_eq!(
        total_of(&[&being_sized, &read_in_part], &cards).as_deref(),
        Some("\u{2265}5 MB")
    );
}

#[test]
fn a_lines_total_falls_with_what_a_clean_leaves_as_its_cards_trays_do() {
    let (cleaning, notifier) = (deleted(), Notifier::new(|_, _| {}));
    let cards = context(&cleaning, &notifier);
    let app = Project {
        own_files: measured(1_000_000_000),
        artifacts: vec![cargo_target("/code/app/target", measured(12_300_000_000))],
        ..project_at("/code/app")
    };
    assert_eq!(total_of(&[&app], &cards).as_deref(), Some("1 GB"));
}

/// A map of `github.com/me/app`'s clones, one with a copy made from it, so its group's line holds
/// a checkout's line.
fn app_with_a_copy() -> Snapshot {
    snapshot(
        vec![
            app("app", &[], project("", 1)),
            app("app-wt", &["/code/app"], project("", 1)),
            app("app-2", &[], project("", 30)),
        ],
        vec![],
    )
}

/// The titles of a submenu's entries, each checked or not.
fn choices(entries: &[MenuEntry]) -> Vec<(&str, bool)> {
    let each = entries.iter().map(|entry| match entry {
        MenuEntry::Item { title, checked, .. } => (title.as_str(), *checked),
        entry => panic!("only choices: {entry:?}"),
    });
    each.collect()
}

#[test]
fn a_groups_line_offers_how_long_its_build_folders_may_sit_idle_with_the_choice_made_checked() {
    let map = app_with_a_copy();
    let app_group = crate::map::GroupKey::Upstream("github.com/me/app".to_string());
    let mut chosen = CleanWhenIdle::default();
    assert!(chosen.choose(&app_group, Some(3), &map));
    let rows = rows_cleaning_when_idle(&map, &[app_line()], &chosen);
    let [(group, _), (checkout, _)] = &lines(rows)[..] else {
        panic!("the group's line and its checkout's");
    };

    let [MenuEntry::Submenu { title, entries }] = &group.menu[..] else {
        panic!("only the choice, with nothing to clean: {:?}", group.menu);
    };
    assert_eq!(title, "Clean When Idle");
    assert_eq!(
        choices(entries),
        [
            ("Never", false),
            ("For a Day", false),
            ("For 3 Days", true),
            ("For a Week", false),
            ("For 2 Weeks", false),
        ]
    );
    let MenuEntry::Item { action, .. } = &entries[0] else {
        panic!("a choice");
    };
    assert_eq!(
        *action,
        MenuAction::Dashboard {
            action: Action::CleanWhenIdle {
                group: GroupKind::Upstream {
                    remote: "github.com/me/app".to_string()
                },
                days: None,
            }
        }
    );
    assert!(
        checkout.menu.is_empty(),
        "a group's choice, not a checkout's"
    );
}

#[test]
fn a_line_whose_group_cleans_itself_once_idle_says_so_and_another_says_nothing() {
    let map = app_with_a_copy();
    let app_group = crate::map::GroupKey::Upstream("github.com/me/app".to_string());
    let marks = |chosen: &CleanWhenIdle| -> Vec<Option<String>> {
        let rows = rows_cleaning_when_idle(&map, &[app_line()], chosen);
        let lines = lines(rows).into_iter();
        lines.map(|(line, _)| line.cleans_when_idle).collect()
    };

    assert_eq!(marks(&CleanWhenIdle::default()), [None, None]);
    let mut chosen = CleanWhenIdle::default();
    assert!(chosen.choose(&app_group, Some(7), &map));
    assert_eq!(
        marks(&chosen),
        [
            Some("Cleans its build folders once they've sat idle for a week".to_string()),
            None
        ],
        "the group's line alone"
    );
}
