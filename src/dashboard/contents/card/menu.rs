//! What a right click on a card offers: everything Sutra can do to the project, so the card itself
//! only has to show how it stands.

use crate::dashboard::cleaning::Cleaning;
use crate::dashboard::{Action, MenuAction, MenuEntry, format};
use crate::map::clean::WORTH_CLEANING_BYTES;
use crate::map::{ArtifactKind, Project};
use crate::model::Environment;
use crate::notifications::Notifier;

/// The menu for `project`. Sections with nothing in them leave no separator behind.
pub(super) fn entries(
    project: &Project,
    cleaning: &Cleaning,
    notifier: &Notifier,
    sandboxed: bool,
) -> Vec<MenuEntry> {
    let path = format::path_text(&project.path);
    let mut sections: Vec<Vec<MenuEntry>> = vec![vec![
        item(
            "Open in Visual Studio Code",
            MenuAction::OpenInVisualStudioCode { path: path.clone() },
        ),
        item(
            "Reveal in Finder",
            MenuAction::RevealInFinder { path: path.clone() },
        ),
        item("Copy Path", MenuAction::CopyPath { path }),
    ]];
    sections.push(
        project
            .artifacts
            .iter()
            .filter(|dir| dir.kind == ArtifactKind::CargoTarget && cleaning.state_of(dir).is_none())
            .filter(|dir| {
                dir.measurement
                    .usage()
                    .is_none_or(|usage| usage.bytes >= WORTH_CLEANING_BYTES)
            })
            .map(|dir| {
                let name = format::artifact_name(dir, &project.path);
                let title = match dir.measurement.usage() {
                    Some(usage) => format!("Clean {name}/ ({})", format::usage_size(usage)),
                    None => format!("Clean {name}/"),
                };
                let folder = format::path_text(&dir.path);
                item(title, dashboard(Action::Clean { folder }))
            })
            .collect(),
    );
    if let Some(env) = &project.environment {
        let mut units = Vec::new();
        for unit in &env.units {
            let (environment, name) = (env.id.clone(), unit.name.clone());
            let muted = notifier.is_unit_muted(&env.id, &unit.name);
            let banners_off = notifier.is_unit_notifications_off(&env.id, &unit.name);
            units.push(item(
                format!("{} {}", if muted { "Unmute" } else { "Mute" }, unit.name),
                dashboard(Action::ToggleUnitMute {
                    environment: environment.clone(),
                    unit: name.clone(),
                }),
            ));
            units.push(item(
                format!(
                    "{} {} Notifications",
                    if banners_off { "Enable" } else { "Disable" },
                    unit.name
                ),
                dashboard(Action::ToggleUnitNotifications {
                    environment,
                    unit: name,
                }),
            ));
        }
        sections.push(units);
        if offers_terminate(env, sandboxed) {
            sections.push(vec![item(
                "Terminate Environment",
                dashboard(Action::TerminateEnvironment { pid: env.pid }),
            )]);
        }
    }
    let mut menu = Vec::new();
    for section in sections.into_iter().filter(|section| !section.is_empty()) {
        if !menu.is_empty() {
            menu.push(MenuEntry::Separator);
        }
        menu.extend(section);
    }
    menu
}

/// Whether the menu offers to terminate `env`: its process lives, and Sutra runs outside the App
/// Sandbox, inside which a signal cannot reach another app's process.
pub(in crate::dashboard) fn offers_terminate(env: &Environment, sandboxed: bool) -> bool {
    env.alive && !sandboxed
}

fn item(title: impl Into<String>, action: MenuAction) -> MenuEntry {
    MenuEntry::Item {
        title: title.into(),
        checked: false,
        action,
    }
}

/// A line the dashboard itself carries out.
fn dashboard(action: Action) -> MenuAction {
    MenuAction::Dashboard { action }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::cleaning::tests::cleaning;
    use crate::map::test_support::{cargo_target, environment_at, measured, project_at};
    use crate::model::{State, UnitStatus};

    fn app() -> Project {
        project_at("/code/app")
    }

    fn notifier() -> Notifier {
        Notifier::new(|_, _| {})
    }

    /// A running or stopped environment publishing the one unit `web`.
    fn environment(alive: bool) -> Environment {
        Environment {
            pid: 4242,
            alive,
            units: vec![UnitStatus {
                name: "web".to_string(),
                state: State::Ready,
                detail: None,
            }],
            ..environment_at("/code/app")
        }
    }

    fn titles(entries: &[MenuEntry]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item { title, .. } => title.as_str(),
                MenuEntry::Separator => "-",
            })
            .collect()
    }

    #[test]
    fn a_plain_folder_offers_to_open_reveal_and_copy_and_no_stray_separator() {
        let entries = entries(&app(), &cleaning(), &notifier(), false);
        assert_eq!(
            titles(&entries),
            [
                "Open in Visual Studio Code",
                "Reveal in Finder",
                "Copy Path"
            ]
        );
        assert_eq!(
            entries[0],
            item(
                "Open in Visual Studio Code",
                MenuAction::OpenInVisualStudioCode {
                    path: "/code/app".to_string()
                }
            )
        );
        assert_eq!(
            entries[1],
            item(
                "Reveal in Finder",
                MenuAction::RevealInFinder {
                    path: "/code/app".to_string()
                }
            )
        );
    }

    #[test]
    fn a_cargo_target_offers_clean_with_its_size_and_a_running_environment_offers_the_rest() {
        let app = Project {
            artifacts: vec![cargo_target("/code/app/target", measured(12_300_000_000))],
            environment: Some(environment(true)),
            ..app()
        };
        let entries = entries(&app, &cleaning(), &notifier(), false);
        assert_eq!(
            titles(&entries),
            [
                "Open in Visual Studio Code",
                "Reveal in Finder",
                "Copy Path",
                "-",
                "Clean target/ (12 GB)",
                "-",
                "Mute web",
                "Disable web Notifications",
                "-",
                "Terminate Environment",
            ]
        );
        let action = |index: usize| match &entries[index] {
            MenuEntry::Item {
                action: MenuAction::Dashboard { action },
                ..
            } => action.clone(),
            entry => panic!("not the dashboard's: {entry:?}"),
        };
        assert_eq!(
            action(4),
            Action::Clean {
                folder: "/code/app/target".to_string()
            }
        );
        assert_eq!(
            action(6),
            Action::ToggleUnitMute {
                environment: "env".to_string(),
                unit: "web".to_string()
            }
        );
        assert_eq!(action(9), Action::TerminateEnvironment { pid: 4242 });
    }

    #[test]
    fn terminate_is_offered_only_for_a_live_environment_outside_the_sandbox() {
        let mut app = app();
        app.environment = Some(environment(true));
        let notifier = notifier();
        let sandboxed = entries(&app, &cleaning(), &notifier, true);
        assert!(!titles(&sandboxed).contains(&"Terminate Environment"));
        app.environment = Some(environment(false));
        let stopped = entries(&app, &cleaning(), &notifier, false);
        assert!(!titles(&stopped).contains(&"Terminate Environment"));
    }

    #[test]
    fn a_build_folder_already_clean_offers_no_clean_and_one_not_yet_sized_does() {
        let cleaning = cleaning();
        let notifier = notifier();
        let project = Project {
            artifacts: vec![
                cargo_target("/code/app/emptied", measured(WORTH_CLEANING_BYTES - 1)),
                cargo_target("/code/app/full", measured(WORTH_CLEANING_BYTES)),
                cargo_target("/code/app/unsized", crate::map::Measurement::Measuring),
            ],
            ..app()
        };
        let clean_titles: Vec<String> = entries(&project, &cleaning, &notifier, false)
            .into_iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item { title, .. } if title.starts_with("Clean") => Some(title),
                _ => None,
            })
            .collect();
        assert_eq!(clean_titles.len(), 2, "{clean_titles:?}");
        assert!(clean_titles[0].starts_with("Clean full/"));
        assert_eq!(clean_titles[1], "Clean unsized/");
    }
}
