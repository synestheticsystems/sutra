//! When a file event in a project says someone worked there, and when. Activity on the map means
//! work: a file made, removed, renamed or written. But a file also changes when nobody works on
//! it, and FSEvents reports some of those changes as changes to its contents. When an app opens a
//! file that iCloud Drive had evicted, iCloud brings it back, and the event says the file's
//! contents, attributes, owner and inode all changed (flags 0x1d400, seen on 2026-11-06 for a
//! download in ~/Documents). A download leaves the file's own modification time where it was,
//! because only writing its contents moves that time. So a change to a file counts from that time,
//! not from the moment the event came.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use notify::EventKind;
use notify::event::ModifyKind;

/// When the file event of `kind` at `path` says work was done in the project that holds it, or
/// `None` when it says none. A file made, removed or renamed was worked on `now`. Any other change
/// counts from the file's own modification time, and never later than `now`. In a project Sutra
/// may not read (`readable` false), Sutra never looks at the file, so such a change does not
/// count.
pub(super) fn worked_at(
    kind: &EventKind,
    path: &Path,
    readable: bool,
    now: SystemTime,
) -> Option<SystemTime> {
    match kind {
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)) => {
            Some(now)
        }
        _ if readable => fs::symlink_metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .map(|modified| modified.min(now)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use notify::event::{CreateKind, DataChange, MetadataKind, RemoveKind, RenameMode};

    use super::*;
    use crate::map::test_support::{TempDir, at, set_mtime};

    /// The kinds of the events FSEvents raised when iCloud Drive brought an evicted file back.
    const DOWNLOAD: [EventKind; 4] = [
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Extended)),
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Ownership)),
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
    ];

    #[test]
    fn a_file_made_removed_or_renamed_was_worked_on_now_wherever_it_is() {
        let now = at(2_000_000_000);
        let gone = Path::new("/nowhere/notes.md");
        let kinds = [
            EventKind::Create(CreateKind::File),
            EventKind::Remove(RemoveKind::File),
            EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
        ];
        for kind in kinds {
            assert_eq!(worked_at(&kind, gone, true, now), Some(now), "{kind:?}");
            assert_eq!(worked_at(&kind, gone, false, now), Some(now), "{kind:?}");
        }
    }

    #[test]
    fn a_file_brought_back_from_icloud_counts_from_when_it_was_last_written() {
        let fixture = TempDir::new("worked-download");
        let file = fixture.write("README.md", "evicted, then brought back");
        let written = at(1_760_000_000);
        set_mtime(&file, written);
        let now = at(2_000_000_000);
        for kind in DOWNLOAD {
            assert_eq!(
                worked_at(&kind, &file, true, now),
                Some(written),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_file_written_with_a_time_still_to_come_counts_as_worked_on_now() {
        let fixture = TempDir::new("worked-future");
        let file = fixture.write("README.md", "from a clock ahead of this one");
        let now = at(2_000_000_000);
        set_mtime(&file, now + Duration::from_secs(3_600));
        let written = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        assert_eq!(worked_at(&written, &file, true, now), Some(now));
    }

    #[test]
    fn a_change_to_a_file_sutra_may_not_read_or_one_already_gone_does_not_count() {
        let fixture = TempDir::new("worked-unread");
        let file = fixture.write("README.md", "behind a folder kept out");
        let now = at(2_000_000_000);
        let gone = fixture.path("gone.md");
        for kind in DOWNLOAD {
            assert_eq!(worked_at(&kind, &file, false, now), None, "{kind:?}");
            assert_eq!(worked_at(&kind, &gone, true, now), None, "{kind:?}");
        }
    }
}
