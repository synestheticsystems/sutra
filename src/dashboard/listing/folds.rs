//! The folds the user chose from the list's lines. Every line starts folded: a group's, so the list
//! opens on one line per upstream, and a checkout's, so a group opens on its clones before their
//! copies. A click on a line turns its fold, and the choice outlasts the run, kept with the saved
//! settings. So that what is kept never outgrows the map, a choice goes once a live map no longer
//! knows what its line stands for: not what one snapshot lists, which leaves out a folder that
//! waits to be sized for its card, but every group and every copied checkout the engine tracks
//! (see `map::Ancestry`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dashboard::{GroupKind, LineKey, format};
use crate::map::{GroupKey, Snapshot};

/// The fold the user left each line in, by the words `saved_as` writes its key in. A line never
/// clicked is not here, and starts folded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::dashboard) struct Folds(BTreeMap<String, Fold>);

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
        self.0.get(&saved_as(line)) != Some(&Fold::Open)
    }

    /// The user clicked the line of `line`: fold it shut, or open it. A line whose key `map` no
    /// longer knows changes nothing, as it belonged to an older map, and the next would forget its
    /// fold anyway. Returns whether the folds changed.
    pub(in crate::dashboard) fn toggle(&mut self, line: &LineKey, map: &Snapshot) -> bool {
        let saved = saved_as(line);
        if !known(map).contains(&saved) {
            return false;
        }
        let turned = if self.is_folded(line) {
            Fold::Open
        } else {
            Fold::Folded
        };
        self.0.insert(saved, turned);
        true
    }

    /// A live map arrived: forget the fold of every line whose key `map` no longer knows. Returns
    /// whether any went.
    pub(in crate::dashboard) fn forget_gone(&mut self, map: &Snapshot) -> bool {
        let known = known(map);
        let before = self.0.len();
        self.0.retain(|saved, _| known.contains(saved));
        self.0.len() < before
    }
}

/// The words a line's fold is kept under in the settings: its kind, then what it is known by.
fn saved_as(line: &LineKey) -> String {
    match line {
        LineKey::Group {
            kind: GroupKind::Upstream { remote },
        } => format!("upstream {remote}"),
        LineKey::Group {
            kind: GroupKind::LocalRepository { folder },
        } => format!("local repository {folder}"),
        LineKey::Group {
            kind: GroupKind::Package { name },
        } => format!("package {name}"),
        LineKey::Group {
            kind: GroupKind::Other,
        } => "other".to_string(),
        LineKey::Group {
            kind: GroupKind::Docker,
        } => "docker".to_string(),
        LineKey::Checkout { folder } => format!("checkout {folder}"),
    }
}

/// The words of the key of every line `map` may show, as `saved_as` writes them: every group its
/// projects stand in, and every checkout copies were made from; and always Other's and Docker's,
/// one line each, which may stand whatever the map holds: Other's with the agents working
/// elsewhere alone, and Docker's whenever Docker answers, which it may not have yet.
fn known(map: &Snapshot) -> Vec<String> {
    let groups = map.ancestry.groups.iter().map(LineKey::group);
    let always = [LineKey::group(&GroupKey::Other), LineKey::DOCKER];
    let checkouts = map
        .ancestry
        .made_from
        .iter()
        .map(|folder| LineKey::Checkout {
            folder: format::path_text(folder),
        });
    let mut known: Vec<String> = groups
        .chain(always)
        .chain(checkouts)
        .map(|line| saved_as(&line))
        .collect();
    known.sort();
    known.dedup();
    known
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
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
                groups: BTreeSet::from([GroupKey::Upstream(APP.to_string())]),
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
    fn a_click_on_a_line_the_engine_no_longer_knows_changes_nothing() {
        let mut folds = Folds::default();
        let gone = LineKey::Group {
            kind: GroupKind::Package {
                name: "gone".to_string(),
            },
        };
        assert!(!folds.toggle(&gone, &map()));
        let notes = LineKey::Checkout {
            folder: "/code/notes".to_string(),
        };
        assert!(
            !folds.toggle(&notes, &map()),
            "a checkout nothing was copied from"
        );
        assert_eq!(folds, Folds::default());
        let other = LineKey::group(&GroupKey::Other);
        assert!(folds.toggle(&other, &map()), "Other is always known");
    }

    #[test]
    fn dockers_fold_outlasts_a_map_on_which_docker_has_not_answered_yet() {
        let mut folds = Folds::default();
        assert!(folds.toggle(&LineKey::DOCKER, &map()));
        assert_eq!(map().docker, None, "Docker has said nothing");

        assert!(!folds.forget_gone(&map()));
        assert!(!folds.is_folded(&LineKey::DOCKER), "still open");
    }

    #[test]
    fn a_fold_lasts_while_the_engine_knows_its_line_with_or_without_a_card_and_goes_with_it() {
        let mut folds = Folds::default();
        folds.toggle(&app(), &map());
        folds.toggle(&site_checkout(), &map());
        assert!(
            !folds.forget_gone(&map()),
            "both known, though no card is listed"
        );

        let only_app = Snapshot {
            ancestry: Arc::new(Ancestry {
                groups: BTreeSet::from([GroupKey::Upstream(APP.to_string())]),
                made_from: BTreeSet::new(),
            }),
            ..map()
        };
        assert!(folds.forget_gone(&only_app));
        assert!(!folds.is_folded(&app()), "app's opening stays");
        assert!(
            folds.is_folded(&site_checkout()),
            "folded, as it starts again"
        );
    }

    #[test]
    fn folds_are_saved_as_each_lines_choice_under_its_kind_and_key() {
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
