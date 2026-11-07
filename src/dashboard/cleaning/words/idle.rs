//! What Sutra says about cleaning every idle build folder at once: the one question, and why there
//! is nothing to ask.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use super::{list, unknown_sharers};
use crate::dashboard::format;
use crate::dashboard::state::Question;
use crate::map::clean::{CleanError, IdleClean, IdleCleans, LeftOut, PrepareError, Sharing, Why};

/// How many folders the question about idle folders names before it counts the rest.
const IDLE_NAMED: usize = 8;

/// The one question before every idle folder is cleaned: how many, how much, whose, and what was
/// left out.
pub(in crate::dashboard::cleaning) fn idle_question(
    cleans: &[IdleClean],
    left_out: &[LeftOut],
    quiet_at_least: Duration,
) -> Question {
    let bytes = bytes_of_each(cleans);
    let total: u64 = bytes.iter().sum();
    let title = match cleans {
        [one] => format!("Clean the idle build folder in {}?", one.project_name),
        many => format!("Clean {} idle build folders?", many.len()),
    };
    let (them, these) = match cleans {
        [_] => ("it", "this project"),
        _ => ("them", "these projects"),
    };
    let days = quiet_at_least.as_secs() / (24 * 60 * 60);
    let mut detail = format!(
        "This frees {}. Nothing has happened in {these} for {days} days or more and nothing is \
         running in {them}, and cargo fills {them} again the next time it builds.",
        format::size(total, false)
    );
    if cleans.len() > 1 {
        let mut largest: Vec<(&IdleClean, u64)> = cleans.iter().zip(bytes).collect();
        largest.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        let lines: Vec<String> = largest
            .iter()
            .take(IDLE_NAMED)
            .map(|(clean, bytes)| {
                format!("{} ({})", name_of_clean(clean), format::size(*bytes, false))
            })
            .collect();
        detail.push_str(&format!("\n\n{}", lines.join("\n")));
        if cleans.len() > IDLE_NAMED {
            detail.push_str(&format!("\nand {} more", cleans.len() - IDLE_NAMED));
        }
    }
    let among_them = |name: &String| cleans.iter().any(|clean| clean.project_name == *name);
    let mut sharing = Sharing::default();
    for clean in cleans {
        sharing
            .projects
            .extend(clean.prepared.sharing.projects.iter().cloned());
        sharing
            .unknown
            .extend(clean.prepared.sharing.unknown.iter().cloned());
    }
    sharing.projects.retain(|name| !among_them(name));
    sharing.projects.sort();
    sharing.projects.dedup();
    sharing.unknown.sort();
    sharing.unknown.dedup();
    if !sharing.projects.is_empty() {
        detail.push_str(&format!(
            "\n\nBuilds of {} are in there too, and go with {them}.",
            list(&sharing.projects)
        ));
    }
    detail.push_str(&unknown_sharers(&sharing));
    if !left_out.is_empty() {
        detail.push_str(&format!("\n\n{}", left_out_line(left_out)));
    }
    Question {
        title,
        detail,
        action: "Clean".to_string(),
    }
}

/// The alert when the search behind the toolbar's clean cannot finish.
pub(in crate::dashboard::cleaning) const IDLE_SEARCH_STOPPED: &str =
    "Couldn't work out what to clean";

/// The alert in place of the toolbar's question when there is nothing to ask about, `None` when
/// there is. `found` is what the search found, `None` when it ended with no word.
pub(in crate::dashboard::cleaning) fn nothing_to_ask(
    found: Option<&IdleCleans>,
) -> Option<(String, String)> {
    match found {
        None => Some((
            IDLE_SEARCH_STOPPED.to_string(),
            "The search stopped before it finished. Nothing was deleted.".to_string(),
        )),
        Some(found) if found.cleans.is_empty() => Some(nothing_idle_to_clean(&found.left_out)),
        Some(_) => None,
    }
}

/// What the toolbar's clean says when it finds nothing it can do: what stopped it, when anything
/// did.
fn nothing_idle_to_clean(left_out: &[LeftOut]) -> (String, String) {
    let detail = if left_out.is_empty() {
        "Every build folder is in use, recently active or already clean.".to_string()
    } else {
        left_out_line(left_out)
    };
    ("Nothing to clean right now".to_string(), detail)
}

/// What each clean frees at most, counting a folder two cleans share for the first of them only.
fn bytes_of_each(cleans: &[IdleClean]) -> Vec<u64> {
    let mut counted = HashSet::new();
    cleans
        .iter()
        .map(|clean| {
            let folders = clean.prepared.plan().folders();
            folders
                .into_iter()
                .zip(&clean.prepared.sizes)
                .filter(|(folder, _)| counted.insert(*folder))
                .filter_map(|(_, usage)| usage.map(|usage| usage.bytes))
                .sum()
        })
        .collect()
}

/// The project's name, and the folder's path in it when it is not the usual `target`.
fn name_of_clean(clean: &IdleClean) -> String {
    match clean.clicked.strip_prefix(&clean.project_path) {
        Ok(inside) if inside != Path::new("target") => {
            format!("{} {}", clean.project_name, inside.display())
        }
        _ => clean.project_name.clone(),
    }
}

fn left_out_line(left_out: &[LeftOut]) -> String {
    let reasons: Vec<String> = left_out
        .iter()
        .map(|left_out| {
            let why = match &left_out.why {
                Why::Refused(PrepareError::Locks(CleanError::BuildRunning { .. })) => {
                    "a build is using it".to_string()
                }
                Why::Refused(_) => "Sutra can't clean it".to_string(),
                Why::SharedWith(project) => format!("{project} builds there and isn't idle"),
            };
            format!("{} ({why})", left_out.project_name)
        })
        .collect();
    format!("Left out for now: {}.", list(&reasons))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::clean::{CleanPlan, Origin, Plan, PlannedFolder};
    use crate::map::test_support::usage;

    fn planned(path: &str, origin: Origin) -> PlannedFolder {
        PlannedFolder {
            path: PathBuf::from(path),
            origin,
        }
    }

    fn idle_clean(name: &str, bytes: u64) -> IdleClean {
        let target = format!("/code/{name}/target");
        let plan = Plan::CargoClean(CleanPlan {
            workspace: PathBuf::from(format!("/code/{name}")),
            folders: vec![planned(&target, Origin::Default)],
        });
        IdleClean {
            clicked: PathBuf::from(target),
            project_name: name.to_string(),
            project_path: PathBuf::from(format!("/code/{name}")),
            prepared: crate::map::clean::Prepared::for_test(
                plan,
                vec![Some(usage(bytes))],
                Sharing::default(),
            ),
        }
    }

    const THREE_DAYS: Duration = Duration::from_secs(3 * 24 * 60 * 60);

    #[test]
    fn the_idle_question_names_how_many_how_much_and_the_largest_first() {
        let cleans: Vec<IdleClean> = (1..=10)
            .map(|n| idle_clean(&format!("p{n}"), n * 1_000_000_000))
            .collect();

        let question = idle_question(&cleans, &[], THREE_DAYS);

        assert_eq!(question.title, "Clean 10 idle build folders?");
        assert_eq!(question.action, "Clean");
        assert_eq!(
            question.detail,
            "This frees 55 GB. Nothing has happened in these projects for 3 days or more and \
             nothing is running in them, and cargo fills them again the next time it builds.\n\n\
             p10 (10 GB)\np9 (9 GB)\np8 (8 GB)\np7 (7 GB)\np6 (6 GB)\np5 (5 GB)\np4 (4 GB)\n\
             p3 (3 GB)\nand 2 more"
        );
    }

    #[test]
    fn a_folder_two_cleans_share_counts_once_towards_the_total() {
        let both = [
            planned("/code/one/target", Origin::Default),
            planned("/cache/shared", Origin::Default),
        ];
        let one = IdleClean {
            prepared: crate::map::clean::Prepared::for_test(
                Plan::CargoClean(CleanPlan {
                    workspace: PathBuf::from("/code/one"),
                    folders: both.to_vec(),
                }),
                vec![Some(usage(1_000_000_000)), Some(usage(5_000_000_000))],
                Sharing::default(),
            ),
            ..idle_clean("one", 0)
        };
        let two = IdleClean {
            prepared: crate::map::clean::Prepared::for_test(
                Plan::CargoClean(CleanPlan {
                    workspace: PathBuf::from("/code/two"),
                    folders: vec![
                        planned("/code/two/target", Origin::Default),
                        planned("/cache/shared", Origin::Default),
                    ],
                }),
                vec![Some(usage(2_000_000_000)), Some(usage(5_000_000_000))],
                Sharing::default(),
            ),
            ..idle_clean("two", 0)
        };

        assert_eq!(bytes_of_each(&[one, two]), [6_000_000_000, 2_000_000_000]);
    }

    #[test]
    fn the_idle_question_for_one_folder_names_its_project() {
        let question = idle_question(&[idle_clean("app", 4_000_000_000)], &[], THREE_DAYS);

        assert_eq!(question.title, "Clean the idle build folder in app?");
        assert_eq!(
            question.detail,
            "This frees 4 GB. Nothing has happened in this project for 3 days or more and nothing \
             is running in it, and cargo fills it again the next time it builds."
        );
    }

    #[test]
    fn the_idle_question_says_who_shares_and_what_was_left_out() {
        let mut one = idle_clean("one", 1_000_000_000);
        one.prepared.sharing = Sharing {
            projects: vec!["outside".to_string(), "two".to_string()],
            project_folders: Vec::new(),
            unknown: vec!["sealed".to_string()],
        };
        let two = idle_clean("two", 1_000_000_000);
        let left_out = [
            LeftOut {
                project_name: "busy".to_string(),
                why: Why::Refused(PrepareError::Locks(CleanError::BuildRunning {
                    lock: PathBuf::from("/code/busy/target/debug/.cargo-lock"),
                })),
            },
            LeftOut {
                project_name: "tied".to_string(),
                why: Why::SharedWith("working".to_string()),
            },
        ];

        let detail = idle_question(&[one, two], &left_out, THREE_DAYS).detail;

        assert!(detail.contains("Builds of outside are in there too, and go with them."));
        assert!(detail.contains("Sutra can't read the Cargo settings of sealed"));
        assert!(detail.ends_with(
            "Left out for now: busy (a build is using it) and tied (working builds there and \
             isn't idle)."
        ));
    }

    #[test]
    fn nothing_to_clean_says_why_when_something_stopped_it() {
        assert_eq!(
            nothing_idle_to_clean(&[]).1,
            "Every build folder is in use, recently active or already clean."
        );
        let left_out = [LeftOut {
            project_name: "busy".to_string(),
            why: Why::Refused(PrepareError::Locks(CleanError::BuildRunning {
                lock: PathBuf::from("/x"),
            })),
        }];
        assert_eq!(
            nothing_idle_to_clean(&left_out).1,
            "Left out for now: busy (a build is using it)."
        );
    }

    #[test]
    fn a_search_that_ended_with_no_word_says_so_and_does_not_claim_nothing_is_idle() {
        let (title, detail) = nothing_to_ask(None).unwrap();
        assert_eq!(title, "Couldn't work out what to clean");
        assert_eq!(
            detail,
            "The search stopped before it finished. Nothing was deleted."
        );
        let (title, _) = nothing_to_ask(Some(&IdleCleans::default())).unwrap();
        assert_eq!(title, "Nothing to clean right now");
    }
}
