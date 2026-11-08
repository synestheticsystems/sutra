//! How the window writes paths, sizes, counts and times, so the cards, the lines around the list
//! and the questions about a clean all write them alike.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::map::{ArtifactDir, DirUsage};

/// `path` as the window receives it: what the window knows a folder by, and what an action on the
/// folder names it back by, so no two folders may read the same. A path that is text reads as
/// itself, as every path does on a volume macOS formats: APFS and ExFAT refuse a name that is not
/// text, and HFS+ and FAT keep its odd bytes as `%FF`. A volume that passes names through as they
/// are, such as an NFS share, can hold one, and each byte of it that is not text reads as a NUL
/// and two hex digits. No path on disk holds a NUL, so that text names no other folder, and an
/// action naming it finds the folder again. Finder and the editors, which take the text as a path,
/// find nothing there.
pub(super) fn path_text(path: &Path) -> String {
    let mut text = String::new();
    for chunk in path.as_os_str().as_encoded_bytes().utf8_chunks() {
        text.push_str(chunk.valid());
        for byte in chunk.invalid() {
            text.push_str(&format!("\0{byte:02x}"));
        }
    }
    text
}

/// `path` with a leading home folder written `~`.
pub(super) fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The watched folders as a list: `~/code, /Volumes/work`.
pub(super) fn roots(roots: &[PathBuf], home: &Path) -> String {
    let roots: Vec<String> = roots.iter().map(|root| tilde(root, home)).collect();
    roots.join(", ")
}

/// A build folder's name: its path inside the project, or its own name when it is the project's
/// folder itself, as a scratch folder `CARGO_TARGET_DIR` named is.
pub(super) fn artifact_name(dir: &ArtifactDir, project: &Path) -> String {
    let inside = dir.path.strip_prefix(project).unwrap_or(&dir.path);
    let name = match inside.file_name() {
        Some(_) => inside.as_os_str(),
        None => dir.path.file_name().unwrap_or(dir.path.as_os_str()),
    };
    Path::new(name).display().to_string()
}

/// A size with one decimal in the largest decimal unit that keeps the number under 1000, the way
/// Finder counts: `999 B`, `1.2 KB`, `12.3 GB`.
pub(super) fn bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["KB", "MB", "GB", "TB", "PB", "EB"];
    if n < 1_000 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1_000.0;
    let mut unit = 0;
    // 999.95 and up would print as 1000.0, so it moves to the next unit.
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1_000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// A size as a card shows it, no finer than the eye needs: a decimal only under 10, and none that
/// says nothing. `≥` when part of what was measured could not be read. `12 GB`, `3.1 GB`, `≥5 MB`.
pub(super) fn size(n: u64, partial: bool) -> String {
    let exact = bytes(n);
    let (number, unit) = exact.split_once(' ').unwrap_or((exact.as_str(), ""));
    let value: f64 = number.parse().unwrap_or(0.0);
    let number = if value >= 10.0 {
        format!("{value:.0}")
    } else {
        number.strip_suffix(".0").unwrap_or(number).to_string()
    };
    let at_least = if partial { "\u{2265}" } else { "" };
    format!("{at_least}{number} {unit}")
}

/// The `size` of a measured folder: `12 GB`, or `≥5 MB` when part of it could not be read.
pub(super) fn usage_size(usage: &DirUsage) -> String {
    size(usage.bytes, usage.unreadable_dirs > 0)
}

/// A count as the window receives it. No count here comes near `u32::MAX`, and one that did would
/// read as the most the window can hold rather than wrap round to a few.
pub(super) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `1 project`, `2 projects`.
pub(super) fn plural(n: u64, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// `a`, `a or b`, `a, b or c`.
pub(super) fn either(items: &[&str]) -> String {
    joined(items, "or")
}

/// `a`, `a and b`, `a, b and c`.
pub(super) fn all(items: &[&str]) -> String {
    joined(items, "and")
}

/// `items` in a list whose last two `word` joins.
fn joined(items: &[&str], word: &str) -> String {
    match items {
        [] => String::new(),
        [only] => only.to_string(),
        [rest @ .., last] => format!("{} {word} {last}", rest.join(", ")),
    }
}

/// How long ago `then` was, in the largest whole unit: `now`, `3m`, `2h`, `5d`. A time in the future
/// (a clock that disagrees with ours) reads as `now`.
pub(super) fn age(then: SystemTime, now: SystemTime) -> String {
    whole_units(then, now).unwrap_or_else(|| "now".to_string())
}

/// How long ago `then` was in the largest whole unit, `None` under a minute.
pub(super) fn whole_units(then: SystemTime, now: SystemTime) -> Option<String> {
    let secs = now.duration_since(then).unwrap_or(Duration::ZERO).as_secs();
    match secs {
        0..60 => None,
        60..3_600 => Some(format!("{}m", secs / 60)),
        3_600..86_400 => Some(format!("{}h", secs / 3_600)),
        _ => Some(format!("{}d", secs / 86_400)),
    }
}

/// A moment as the user's clock reads it, to the minute: `2026-11-04 10:23`. `None` when the
/// system cannot place it in local time.
pub(super) fn timestamp(at: SystemTime) -> Option<String> {
    local_time(at).map(|time| time.to_string())
}

/// A moment on the user's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalTime {
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
}

impl fmt::Display for LocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }
}

/// `at` in the local time zone, from the C library, which knows the user's zone and its rules.
#[cfg(unix)]
fn local_time(at: SystemTime) -> Option<LocalTime> {
    use nix::libc;
    let secs = at.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let time = libc::time_t::try_from(secs).ok()?;
    let mut parts = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: `localtime_r` reads `time` and, on success, fills `parts` and returns a pointer to
    // it; on failure it returns null and `parts` stays unread.
    let parts = unsafe {
        if libc::localtime_r(&time, parts.as_mut_ptr()).is_null() {
            return None;
        }
        parts.assume_init()
    };
    Some(LocalTime {
        year: parts.tm_year + 1900,
        month: parts.tm_mon + 1,
        day: parts.tm_mday,
        hour: parts.tm_hour,
        minute: parts.tm_min,
    })
}

/// Off Unix there is no C library call to ask, so no moment is placed.
#[cfg(not(unix))]
fn local_time(_at: SystemTime) -> Option<LocalTime> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Measurement;
    use crate::map::test_support::{at, cargo_target, node_modules, usage};

    const HOME: &str = "/Users/dev";

    #[test]
    fn a_path_that_is_not_text_keeps_its_odd_bytes_so_no_other_folder_reads_the_same() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        assert_eq!(path_text(Path::new("/code/app")), "/code/app");
        let odd = Path::new(OsStr::from_bytes(b"/code/\xffapp"));
        assert_eq!(path_text(odd), "/code/\0ffapp");
        let shown_alike = Path::new("/code/\u{fffd}app");
        assert_eq!(path_text(shown_alike), "/code/\u{fffd}app");
    }

    #[test]
    fn a_count_past_what_the_window_holds_reads_as_the_most_it_holds() {
        assert_eq!(count(764), 764);
        assert_eq!(count(usize::MAX), u32::MAX);
    }

    #[test]
    fn tilde_abbreviates_only_the_home_folder() {
        let home = Path::new(HOME);
        assert_eq!(tilde(Path::new(HOME), home), "~");
        assert_eq!(tilde(Path::new("/Users/dev/code"), home), "~/code");
        assert_eq!(tilde(Path::new("/Users/devops"), home), "/Users/devops");
    }

    #[test]
    fn roots_lists_every_root() {
        let dirs = [
            PathBuf::from("/Users/dev/code"),
            PathBuf::from("/Volumes/work"),
        ];
        assert_eq!(roots(&dirs, Path::new(HOME)), "~/code, /Volumes/work");
    }

    #[test]
    fn a_build_folder_is_named_by_its_path_inside_the_project() {
        let nested = node_modules("/code/app/web/node_modules", Measurement::Pending);
        assert_eq!(
            artifact_name(&nested, Path::new("/code/app")),
            "web/node_modules"
        );
        let itself = cargo_target(
            "/tmp/claude-501/s/scratchpad/target-567",
            Measurement::Pending,
        );
        assert_eq!(
            artifact_name(
                &itself,
                Path::new("/tmp/claude-501/s/scratchpad/target-567")
            ),
            "target-567",
            "the project's folder itself goes by its own name"
        );
    }

    #[test]
    fn bytes_under_a_kilobyte_have_no_decimal() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
    }

    #[test]
    fn bytes_show_one_decimal_in_the_largest_unit() {
        assert_eq!(bytes(1_234), "1.2 KB");
        assert_eq!(bytes(1_200_000_000), "1.2 GB");
        assert_eq!(bytes(12_345_678_901), "12.3 GB");
    }

    #[test]
    fn bytes_that_would_round_to_a_thousand_move_to_the_next_unit() {
        assert_eq!(bytes(999_949), "999.9 KB");
        assert_eq!(bytes(999_950), "1.0 MB");
    }

    #[test]
    fn bytes_cover_the_whole_u64_range() {
        assert_eq!(bytes(u64::MAX), "18.4 EB");
    }

    #[test]
    fn a_partial_size_is_a_lower_bound() {
        assert_eq!(size(5_000_000, false), "5 MB");
        assert_eq!(size(3_100_000_000, false), "3.1 GB");
        assert_eq!(size(12_345_678_901, false), "12 GB");
        assert_eq!(size(5_000_000, true), "\u{2265}5 MB");
    }

    #[test]
    fn a_measured_folder_gives_its_size_and_a_partly_read_one_a_lower_bound() {
        assert_eq!(usage_size(&usage(12_300_000_000)), "12 GB");
        let partly_read = DirUsage {
            unreadable_dirs: 3,
            ..usage(5_000_000)
        };
        assert_eq!(usage_size(&partly_read), "\u{2265}5 MB");
    }

    #[test]
    fn plural_adds_an_s_except_for_one() {
        assert_eq!(plural(0, "project"), "0 projects");
        assert_eq!(plural(1, "project"), "1 project");
        assert_eq!(plural(2, "project"), "2 projects");
    }

    #[test]
    fn either_joins_with_commas_and_a_last_or() {
        assert_eq!(either(&["a"]), "a");
        assert_eq!(either(&["a", "b"]), "a or b");
        assert_eq!(either(&["a", "b", "c"]), "a, b or c");
    }

    #[test]
    fn all_joins_with_commas_and_a_last_and() {
        assert_eq!(all(&["a"]), "a");
        assert_eq!(all(&["a", "b"]), "a and b");
        assert_eq!(all(&["a", "b", "c"]), "a, b and c");
    }

    #[test]
    fn age_reads_the_largest_whole_unit() {
        let now = at(1_000_000);
        let ago = |secs| age(now - Duration::from_secs(secs), now);
        assert_eq!(ago(0), "now");
        assert_eq!(ago(59), "now");
        assert_eq!(ago(60), "1m");
        assert_eq!(ago(3_599), "59m");
        assert_eq!(ago(3_600), "1h");
        assert_eq!(ago(86_399), "23h");
        assert_eq!(ago(5 * 86_400), "5d");
    }

    #[test]
    fn age_of_a_future_time_is_just_now() {
        let now = at(1_000);
        assert_eq!(age(now + Duration::from_secs(600), now), "now");
    }

    #[test]
    fn a_local_time_reads_to_the_minute_with_padded_fields() {
        let time = LocalTime {
            year: 2026,
            month: 11,
            day: 4,
            hour: 9,
            minute: 5,
        };
        assert_eq!(time.to_string(), "2026-11-04 09:05");
    }

    #[cfg(unix)]
    #[test]
    fn the_c_library_places_a_moment_on_the_local_clock() {
        // 2001-09-09 01:46:40 UTC: whatever the zone, still 2001 and a real clock reading.
        let time = local_time(at(1_000_000_000)).expect("a local time");
        assert_eq!(time.year, 2001);
        assert!((1..=12).contains(&time.month) && (1..=31).contains(&time.day));
        assert!((0..24).contains(&time.hour) && (0..60).contains(&time.minute));
    }
}
