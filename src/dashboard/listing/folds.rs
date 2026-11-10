//! The folds the user chose from the list's lines. Every line starts folded: a group's, so the list
//! opens on one line per upstream, and a checkout's, so a group opens on its clones before their
//! copies. A click on a line turns its fold, and the choice outlasts the run, kept with the saved
//! settings for as long as the engine knows the line (see `ByLine`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ByLine;
use crate::dashboard::LineKey;
use crate::map::{GroupKey, Snapshot};

/// The fold the user left each line in. A line never clicked is not here, and starts folded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::dashboard) struct Folds(ByLine<Fold>);

/// Whether the rows under a line show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Fold {
    Open,
    Folded,
}

impl Folds {
    /// Whether the line of `line` is folded shut.
    pub(in crate::dashboard) fn is_folded(&self, line: &LineKey) -> bool {
        self.0.get(line) != Some(&Fold::Open)
    }

    /// The user clicked the line of `line`: fold it shut, or open it. A line whose key `map` no
    /// longer knows changes nothing (see `ByLine::set`). Returns whether the folds changed.
    pub(in crate::dashboard) fn toggle(&mut self, line: &LineKey, map: &Snapshot) -> bool {
        let turned = if self.is_folded(line) {
            Fold::Open
        } else {
            Fold::Folded
        };
        self.0.set(line, Some(turned), map)
    }

    /// A live map arrived: carry the fold of each group that `moves` says moved to its new key,
    /// and forget the fold of every line whose key `map` no longer knows (see
    /// `ByLine::keep_up_with`). Returns whether the folds changed.
    pub(in crate::dashboard) fn keep_up_with(
        &mut self,
        map: &Snapshot,
        moves: &BTreeMap<GroupKey, GroupKey>,
    ) -> bool {
        self.0.keep_up_with(map, moves)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::dashboard::GroupKind;
    use crate::dashboard::listing::fixtures::snapshot;
    use crate::map::Ancestry;

    const APP: &str = "github.com/me/app";
    const SITE: &str = "/code/site";

    fn app() -> LineKey {
        LineKey::Group {
            kind: GroupKind::Upstream {
                remote: APP.to_string(),
            },
        }
    }

    fn site_checkout() -> LineKey {
        LineKey::Checkout {
            folder: SITE.to_string(),
        }
    }

    /// A map whose engine knows the group of `APP`'s clones, and that copies were made from the
    /// checkout at `SITE`, whatever cards it lists: none here.
    fn map() -> Snapshot {
        Snapshot {
            ancestry: Arc::new(Ancestry {
                groups: BTreeMap::from([(GroupKey::Upstream(APP.to_string()), BTreeSet::new())]),
                made_from: BTreeSet::from([PathBuf::from(SITE)]),
            }),
            ..snapshot(vec![], vec![])
        }
    }

    #[test]
    fn every_line_starts_folded_and_a_click_opens_it_and_another_folds_it_again() {
        let mut folds = Folds::default();
        assert!(folds.is_folded(&app()));
        assert!(folds.is_folded(&site_checkout()));

        assert!(folds.toggle(&app(), &map()));
        assert!(folds.toggle(&site_checkout(), &map()));
        assert!(!folds.is_folded(&app()));
        assert!(!folds.is_folded(&site_checkout()));

        assert!(folds.toggle(&app(), &map()));
        assert!(folds.is_folded(&app()));
    }

    #[test]
    fn folds_are_saved_as_open_or_folded_under_each_lines_kind_and_key() {
        let mut folds = Folds::default();
        folds.toggle(&app(), &map());
        folds.toggle(&site_checkout(), &map());
        folds.toggle(&site_checkout(), &map());
        let saved = serde_json::to_value(&folds).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({
                "upstream github.com/me/app": "open",
                "checkout /code/site": "folded",
            })
        );
        assert_eq!(serde_json::from_value::<Folds>(saved).unwrap(), folds);
    }
}
