//! The strip at the top of the list when Sutra cannot read a source that alone tells of a kind of
//! thing (agent sessions, registered environments), so their absence does not pass for none
//! running. In the store edition it offers the remedy: granting a larger folder. And how any source
//! that could not be read is worded, which the footer's lines share.

use std::io;
use std::path::Path;

use crate::dashboard::format;
use crate::map::{Source, SourceProblem};

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Banner {
    /// What Sutra cannot see, as one sentence.
    pub heading: String,
    /// Each unreadable source and why, one line each.
    pub reasons: Vec<String>,
    /// The store edition offers the remedy: granting a larger folder (`MenuAction::ChooseFolder`).
    pub offers_folder: bool,
}

/// The banner over the map's `problems`, `None` while every source that alone tells of a kind of
/// thing reads.
pub(in crate::dashboard) fn banner(
    problems: &[SourceProblem],
    sandboxed: bool,
    home: &Path,
) -> Option<Banner> {
    let heading = hidden_sources(problems)?;
    let reasons = problems
        .iter()
        .filter(|problem| problem.source.is_sole_source())
        .map(|problem| problem_line(problem, home))
        .collect();
    Some(Banner {
        heading,
        reasons,
        offers_folder: sandboxed,
    })
}

/// What the map cannot show because a source that alone tells of it could not be read
/// (`Source::is_sole_source`): `Sutra can't see your Claude Code sessions or dev environments in
/// ~/.dev-runner.` `None` when every such source reads.
fn hidden_sources(problems: &[SourceProblem]) -> Option<String> {
    let hides = |source| problems.iter().any(|problem| problem.source == source);
    let hidden: Vec<&str> = [
        (Source::ClaudeSessions, "Claude Code sessions"),
        (Source::CodexDatabase, "Codex threads"),
        (Source::Registry, "dev environments in ~/.dev-runner"),
        (Source::Processes, "processes' ports"),
        (Source::Docker, "Docker containers"),
    ]
    .into_iter()
    .filter(|(source, _)| hides(*source))
    .map(|(_, what)| what)
    .collect();
    (!hidden.is_empty()).then(|| format!("Sutra can't see your {}.", format::either(&hidden)))
}

/// A source the map could not read: `Couldn't read ~/.claude/sessions: permission denied`, with
/// `and 3 more` when discovery met the same error in several folders. The processes have no path:
/// `Couldn't list your processes: permission denied`. Docker is read through its socket: `Couldn't
/// read Docker at ~/.docker/run/docker.sock: timed out`, or, when its engine answered with an error
/// status, `Docker at ~/.docker/run/docker.sock answered with an error`, and alike for its disk
/// use.
pub(super) fn problem_line(problem: &SourceProblem, home: &Path) -> String {
    let verb = match problem.source {
        Source::Processes => return format!("Couldn't list your processes: {}", problem.kind),
        Source::Docker | Source::DockerDiskUse => {
            let socket = format::tilde(&problem.path, home);
            let disk_use = problem.source == Source::DockerDiskUse;
            return match (problem.kind, disk_use) {
                (io::ErrorKind::Other, false) => {
                    format!("Docker at {socket} answered with an error")
                }
                (io::ErrorKind::Other, true) => {
                    format!("Docker at {socket} answered with an error when asked for its disk use")
                }
                (kind, false) => format!("Couldn't read Docker at {socket}: {kind}"),
                (kind, true) => format!("Couldn't read Docker's disk use at {socket}: {kind}"),
            };
        }
        Source::Watcher => "watch",
        Source::Scan
        | Source::Registry
        | Source::EnvironmentState
        | Source::ClaudeSessions
        | Source::ClaudeSessionFile
        | Source::ClaudeSubagentMeta
        | Source::ClaudeSessionTranscript
        | Source::CodexDatabase => "read",
    };
    let more = match problem.count {
        0 | 1 => String::new(),
        n => format!(" and {} more", n - 1),
    };
    format!(
        "Couldn't {verb} {}{more}: {}",
        format::tilde(&problem.path, home),
        problem.kind
    )
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;

    use super::*;

    const HOME: &str = "/Users/dev";

    fn denied(source: Source, path: &str) -> SourceProblem {
        SourceProblem::new(source, PathBuf::from(path), io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn hidden_sources_name_what_the_user_cannot_see() {
        let problems = [
            denied(Source::Registry, "/Users/dev/.dev-runner"),
            denied(Source::ClaudeSessions, "/Users/dev/.claude/sessions"),
        ];
        assert_eq!(
            hidden_sources(&problems).as_deref(),
            Some("Sutra can't see your Claude Code sessions or dev environments in ~/.dev-runner.")
        );
        let unrelated = [
            denied(Source::Scan, "/Users/dev/sealed"),
            denied(Source::Watcher, "/Users/dev"),
        ];
        assert_eq!(hidden_sources(&unrelated), None);
    }

    #[test]
    fn the_banner_names_what_each_sole_source_hides_and_only_those() {
        let sources = [
            Source::Scan,
            Source::Registry,
            Source::ClaudeSessions,
            Source::CodexDatabase,
            Source::Processes,
            Source::Docker,
            Source::Watcher,
        ];
        for source in sources {
            let named = hidden_sources(&[denied(source, "/x")]).is_some();
            assert_eq!(named, source.is_sole_source(), "{source:?}");
        }
    }

    #[test]
    fn the_banner_gives_a_line_for_each_sole_source_and_offers_a_folder_in_the_store_edition() {
        let problems = [
            denied(Source::ClaudeSessions, "/Users/dev/.claude/sessions"),
            denied(Source::Scan, "/Users/dev/sealed"),
        ];
        let shown = banner(&problems, true, Path::new(HOME)).unwrap();
        assert_eq!(
            shown.reasons,
            ["Couldn't read ~/.claude/sessions: permission denied"]
        );
        assert!(shown.offers_folder);
        assert_eq!(banner(&problems[1..], true, Path::new(HOME)), None);
    }

    #[test]
    fn problem_names_the_path_and_the_error() {
        let sessions = denied(Source::ClaudeSessions, "/Users/dev/.claude/sessions");
        assert_eq!(
            problem_line(&sessions, Path::new(HOME)),
            "Couldn't read ~/.claude/sessions: permission denied"
        );
    }

    #[test]
    fn processes_that_cannot_be_listed_hide_their_ports_and_say_so_with_no_path() {
        let unlisted = SourceProblem::new(
            Source::Processes,
            PathBuf::new(),
            io::ErrorKind::PermissionDenied,
        );
        assert_eq!(
            problem_line(&unlisted, Path::new(HOME)),
            "Couldn't list your processes: permission denied"
        );
        assert_eq!(
            hidden_sources(&[unlisted]).as_deref(),
            Some("Sutra can't see your processes' ports.")
        );
    }

    #[test]
    fn docker_that_cannot_be_read_hides_its_containers_and_says_how_its_engine_failed() {
        let socket = "/Users/dev/.docker/run/docker.sock";
        let failed = |kind| SourceProblem::new(Source::Docker, PathBuf::from(socket), kind);
        let home = Path::new(HOME);
        assert_eq!(
            problem_line(&failed(io::ErrorKind::TimedOut), home),
            "Couldn't read Docker at ~/.docker/run/docker.sock: timed out"
        );
        assert_eq!(
            problem_line(&failed(io::ErrorKind::Other), home),
            "Docker at ~/.docker/run/docker.sock answered with an error"
        );
        assert_eq!(
            hidden_sources(&[failed(io::ErrorKind::TimedOut)]).as_deref(),
            Some("Sutra can't see your Docker containers.")
        );
        let disk_use = SourceProblem {
            source: Source::DockerDiskUse,
            ..failed(io::ErrorKind::TimedOut)
        };
        assert_eq!(
            problem_line(&disk_use, home),
            "Couldn't read Docker's disk use at ~/.docker/run/docker.sock: timed out"
        );
        assert_eq!(
            hidden_sources(&[disk_use]),
            None,
            "its containers still show"
        );
    }

    #[test]
    fn problem_of_the_watcher_says_watch_and_one_for_many_folders_counts_the_rest() {
        let home = Path::new(HOME);
        let watcher = SourceProblem::new(
            Source::Watcher,
            PathBuf::from(HOME),
            io::ErrorKind::NotFound,
        );
        assert_eq!(
            problem_line(&watcher, home),
            "Couldn't watch ~: entity not found"
        );
        let scan = SourceProblem {
            count: 4,
            ..denied(Source::Scan, "/Users/dev/sealed")
        };
        assert_eq!(
            problem_line(&scan, home),
            "Couldn't read ~/sealed and 3 more: permission denied"
        );
    }
}
