//! What Sutra says about cleaning the build folders kept only by sessions at rest: the question the
//! toolbar's clean asks about them, apart from the one about the idle folders and after it.

use std::path::Path;
use std::time::Duration;

use super::idle::{NAMED, bytes_of_each, left_out_paragraph, others_building_there};
use crate::dashboard::format;
use crate::dashboard::state::Question;
use crate::map::clean::{AtRestClean, LeftOut, Prepared};

/// The build folders of one project, kept only by its sessions at rest, as the question names them.
struct Kept<'a> {
    name: &'a str,
    path: &'a Path,
    folders: usize,
    sessions: usize,
    at_rest_for: Duration,
    bytes: u64,
}

/// The question before the folders kept only by sessions at rest are cleaned, each project quiet
/// and its sessions at rest for `at_least`: how many projects, how much, then each with how much
/// and how long its sessions have rested, the largest first, and what was left out.
pub(in crate::dashboard::cleaning) fn at_rest_question(
    cleans: &[AtRestClean],
    left_out: &[LeftOut],
    at_least: Duration,
) -> Question {
    let kept = by_project(cleans);
    let total: u64 = kept.iter().map(|kept| kept.bytes).sum();
    let size = format::size(total, false);
    let (title, mut detail) = match kept.as_slice() {
        [one] => {
            let (folder, them) = match one.folders {
                1 => ("build folder", "it"),
                _ => ("build folders", "them"),
            };
            let (session, is, has, or_more, which) = match one.sessions {
                1 => ("session", "is", "has", "", "The"),
                _ => ("sessions", "are", "have", " or more", "A"),
            };
            let title = format!(
                "Clean the {folder} of {}, whose {session} {is} at rest?",
                one.name
            );
            let detail = format!(
                "This frees {size}. Nothing has happened in this project for {} or more, its \
                 {session} {has} been at rest for {}{or_more} and nothing else is running in it. \
                 {which} session may come back to what it built, and cargo fills {them} again the \
                 next time it builds.",
                rest_time(at_least),
                rest_time(one.at_rest_for),
            );
            (title, detail)
        }
        many => {
            let title = format!(
                "Clean the build folders of {} projects whose sessions are at rest?",
                many.len()
            );
            let lines: Vec<String> = (many.iter().take(NAMED))
                .map(|kept| {
                    format!(
                        "{} ({}, at rest for {})",
                        kept.name,
                        format::size(kept.bytes, false),
                        rest_time(kept.at_rest_for)
                    )
                })
                .collect();
            let mut detail = format!(
                "This frees {size}. Nothing has happened in these projects for {} or more, their \
                 sessions have been at rest as long and nothing else is running in them. A \
                 session may come back to what it built, and cargo fills them again the next time \
                 it builds.\n\n{}",
                rest_time(at_least),
                lines.join("\n"),
            );
            if many.len() > NAMED {
                detail.push_str(&format!("\nand {} more", many.len() - NAMED));
            }
            (title, detail)
        }
    };
    let them = if cleans.len() == 1 { "it" } else { "them" };
    let theirs: Vec<(&str, &Prepared)> = (cleans.iter())
        .map(|clean| (clean.project_name.as_str(), &clean.prepared))
        .collect();
    detail.push_str(&others_building_there(&theirs, them));
    detail.push_str(&left_out_paragraph(left_out));
    Question {
        title,
        detail,
        action: "Clean".to_string(),
    }
}

/// `cleans` by the project each is in, the largest first, each counting what its cleans free (see
/// `bytes_of_each`).
fn by_project(cleans: &[AtRestClean]) -> Vec<Kept<'_>> {
    let bytes = bytes_of_each(cleans.iter().map(|clean| &clean.prepared));
    let mut kept: Vec<Kept> = Vec::new();
    for (clean, bytes) in cleans.iter().zip(bytes) {
        match kept.iter_mut().find(|kept| kept.path == clean.project_path) {
            Some(kept) => {
                kept.folders += 1;
                kept.bytes += bytes;
            }
            None => kept.push(Kept {
                name: &clean.project_name,
                path: &clean.project_path,
                folders: 1,
                sessions: clean.sessions,
                at_rest_for: clean.at_rest_for,
                bytes,
            }),
        }
    }
    kept.sort_by_key(|kept| std::cmp::Reverse(kept.bytes));
    kept
}

/// How long `rest` is, in whole days as a sentence says it: `a day`, `3 days`, and under a day,
/// `less than a day`.
fn rest_time(rest: Duration) -> String {
    match rest.as_secs() / (24 * 60 * 60) {
        0 => "less than a day".to_string(),
        1 => "a day".to_string(),
        days => format!("{days} days"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::map::clean::{CleanPlan, Origin, Plan, PlannedFolder, Sharing, Why};
    use crate::map::test_support::usage;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    /// The clean of `folder` in `/code/<name>`, holding `bytes`, kept by `sessions` at rest for
    /// `days` and some hours.
    fn kept(name: &str, folder: &str, bytes: u64, sessions: usize, days: u32) -> AtRestClean {
        let project = PathBuf::from(format!("/code/{name}"));
        let folder = project.join(folder);
        let plan = Plan::CargoClean(CleanPlan {
            workspace: project.clone(),
            folders: vec![PlannedFolder {
                path: folder.clone(),
                origin: Origin::Default,
            }],
        });
        AtRestClean {
            clicked: folder,
            project_name: name.to_string(),
            project_path: project,
            sessions,
            at_rest_for: days * DAY + DAY / 3,
            prepared: crate::map::clean::Prepared::for_test(
                plan,
                vec![Some(usage(bytes))],
                Sharing::default(),
            ),
        }
    }

    #[test]
    fn the_question_names_how_many_projects_how_much_and_each_with_its_rest_largest_first() {
        let cleans = [
            kept("gamma", "target", 30_000_000_000, 1, 1),
            kept("alpha", "target", 60_000_000_000, 2, 3),
            kept("alpha", "tools/target", 30_000_000_000, 2, 3),
            kept("beta", "target", 60_000_000_000, 1, 2),
        ];

        let question = at_rest_question(&cleans, &[], DAY);

        assert_eq!(
            question.title,
            "Clean the build folders of 3 projects whose sessions are at rest?"
        );
        assert_eq!(question.action, "Clean");
        assert_eq!(
            question.detail,
            "This frees 180 GB. Nothing has happened in these projects for a day or more, their \
             sessions have been at rest as long and nothing else is running in them. A session \
             may come back to what it built, and cargo fills them again the next time it \
             builds.\n\nalpha (90 GB, at rest for 3 days)\nbeta (60 GB, at rest for 2 days)\n\
             gamma (30 GB, at rest for a day)"
        );
    }

    #[test]
    fn the_question_about_one_project_names_it_and_how_long_its_sessions_rested() {
        let one = at_rest_question(&[kept("app", "target", 4_000_000_000, 1, 3)], &[], DAY);
        assert_eq!(
            one.title,
            "Clean the build folder of app, whose session is at rest?"
        );
        assert_eq!(
            one.detail,
            "This frees 4 GB. Nothing has happened in this project for a day or more, its session \
             has been at rest for 3 days and nothing else is running in it. The session may come \
             back to what it built, and cargo fills it again the next time it builds."
        );

        let two_of_each = [
            kept("app", "target", 4_000_000_000, 2, 3),
            kept("app", "tools/target", 1_000_000_000, 2, 3),
        ];
        let many = at_rest_question(&two_of_each, &[], DAY);
        assert_eq!(
            many.title,
            "Clean the build folders of app, whose sessions are at rest?"
        );
        assert_eq!(
            many.detail,
            "This frees 5 GB. Nothing has happened in this project for a day or more, its \
             sessions have been at rest for 3 days or more and nothing else is running in it. A \
             session may come back to what it built, and cargo fills them again the next time it \
             builds."
        );
    }

    #[test]
    fn the_question_names_past_eight_projects_as_a_count_and_says_who_shares_and_what_was_left_out()
    {
        let mut cleans: Vec<AtRestClean> = (1..=10)
            .map(|n| kept(&format!("p{n}"), "target", n * 1_000_000_000, 1, 2))
            .collect();
        cleans[0].prepared.sharing = Sharing {
            projects: vec!["outside".to_string(), "p2".to_string()],
            project_folders: Vec::new(),
            unknown: vec!["sealed".to_string()],
        };
        let left_out = [LeftOut {
            project_name: "tied".to_string(),
            folder: PathBuf::from("/code/tied/target"),
            why: Why::SharedWith("working".to_string()),
        }];

        let detail = at_rest_question(&cleans, &left_out, DAY).detail;

        assert!(
            detail.contains("p3 (3 GB, at rest for 2 days)\nand 2 more"),
            "{detail}"
        );
        assert!(detail.contains("Builds of outside are in there too, and go with them."));
        assert!(detail.contains("Sutra can't read the Cargo settings of sealed"));
        assert!(detail.ends_with("Left out for now: tied (working builds there and isn't idle)."));
    }

    #[test]
    fn a_rest_is_told_in_whole_days() {
        assert_eq!(rest_time(DAY / 2), "less than a day");
        assert_eq!(rest_time(DAY + DAY / 2), "a day");
        assert_eq!(rest_time(9 * DAY), "9 days");
    }
}
