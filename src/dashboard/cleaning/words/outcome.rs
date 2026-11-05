//! What Sutra says when a clean stops early.

use std::io;
use std::path::Path;

use crate::dashboard::format;
use crate::map::clean::CleanError;

/// One alert for everything in `stopped`, each `(title, detail)` as `stopped` words it.
pub(in crate::dashboard::cleaning) fn all_stopped(
    stopped: &[(String, String)],
) -> (String, String) {
    match stopped {
        [(title, detail)] => (title.clone(), detail.clone()),
        many => {
            let lines: Vec<String> = many
                .iter()
                .map(|(title, detail)| format!("{title}: {detail}"))
                .collect();
            (
                format!("Couldn't finish {} cleans", many.len()),
                lines.join("\n\n"),
            )
        }
    }
}

/// The alert after a clean that started from `target` stopped early: because of `failure`, or
/// with no word from its thread. `files_deleted` is how far it came.
pub(in crate::dashboard::cleaning) fn stopped(
    target: &Path,
    failure_seen: Option<&CleanError>,
    files_deleted: u64,
    home: &Path,
) -> (String, String) {
    let show = |path: &Path| format::tilde(path, home);
    let mut detail = match failure_seen {
        Some(err) => failure(err, &show),
        None => "The cleaning stopped before it finished.".to_string(),
    };
    if files_deleted == 0 {
        detail.push_str(" Nothing was deleted.");
    } else {
        let deleted = format::plural(files_deleted, "file");
        detail.push_str(&format!(" It had deleted {deleted} by then."));
    }
    (format!("Couldn't finish cleaning {}", show(target)), detail)
}

/// Why a clean could not lock or delete, each path shown by `show`.
pub(super) fn failure(err: &CleanError, show: &dyn Fn(&Path) -> String) -> String {
    match err {
        CleanError::BuildRunning { lock } => format!(
            "A build is using {}. Try again once it finishes.",
            show(lock.parent().unwrap_or(lock))
        ),
        CleanError::Changed { folder } => {
            format!("{} changed before the clean could start.", show(folder))
        }
        CleanError::Io { path, kind } if *kind == io::ErrorKind::Unsupported => format!(
            "{} can't be locked, so Sutra can't tell whether a build is using it.",
            show(path)
        ),
        CleanError::Io { path, kind } => format!("{}: {kind}", show(path)),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const HOME: &str = "/Users/dev";

    #[test]
    fn several_stops_make_one_alert() {
        let one = (
            "Couldn't finish cleaning a".to_string(),
            "denied".to_string(),
        );
        assert_eq!(all_stopped(std::slice::from_ref(&one)), one);
        let two = ("Couldn't finish cleaning b".to_string(), "busy".to_string());
        let (title, detail) = all_stopped(&[one, two]);
        assert_eq!(title, "Couldn't finish 2 cleans");
        assert_eq!(
            detail,
            "Couldn't finish cleaning a: denied\n\nCouldn't finish cleaning b: busy"
        );
    }

    #[test]
    fn a_clean_that_stopped_says_why_and_how_far_it_came() {
        let home = Path::new(HOME);
        let target = Path::new("/Users/dev/code/app/target");
        let running = CleanError::BuildRunning {
            lock: PathBuf::from("/Users/dev/code/app/target/release/.cargo-build-lock"),
        };
        assert_eq!(
            stopped(target, Some(&running), 0, home),
            (
                "Couldn't finish cleaning ~/code/app/target".to_string(),
                "A build is using ~/code/app/target/release. Try again once it finishes. \
                 Nothing was deleted."
                    .to_string()
            )
        );
        assert_eq!(
            stopped(target, None, 1_204, home).1,
            "The cleaning stopped before it finished. It had deleted 1204 files by then."
        );
    }
}
