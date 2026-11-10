//! What the user chose for the list's lines that outlasts the run, kept with the saved settings
//! under words that name each line: its kind, then what it is known by. So that what is kept never
//! outgrows the map, a choice goes once a live map no longer knows what its line stands for: not
//! what one snapshot lists, which leaves out a folder that waits to be sized for its card, but
//! every group and every copied checkout the engine tracks (see `map::Ancestry`). A group whose key
//! changed under all its projects at once keeps its line's choice under the new key.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dashboard::{GroupKind, LineKey, format};
use crate::map::{GroupKey, Snapshot};

/// The choice made for each line, by the words `saved_as` writes its key in. A line the user made
/// no choice for is not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::dashboard) struct ByLine<T>(BTreeMap<String, T>);

impl<T> Default for ByLine<T> {
    fn default() -> Self {
        ByLine(BTreeMap::new())
    }
}

impl<T: PartialEq> ByLine<T> {
    /// The choice made for the line of `line`, if any.
    pub(in crate::dashboard) fn get(&self, line: &LineKey) -> Option<&T> {
        self.0.get(&saved_as(line))
    }

    /// Keep `choice` for the line of `line`, or with `None` let the one made go. A line whose key
    /// `map` no longer knows changes nothing, as it belonged to an older map, and the next would
    /// forget the choice anyway. Returns whether the choices changed.
    pub(in crate::dashboard) fn set(
        &mut self,
        line: &LineKey,
        choice: Option<T>,
        map: &Snapshot,
    ) -> bool {
        let saved = saved_as(line);
        if !known(map).contains(&saved) {
            return false;
        }
        match choice {
            None => self.0.remove(&saved).is_some(),
            Some(choice) if self.0.get(&saved) == Some(&choice) => false,
            Some(choice) => {
                self.0.insert(saved, choice);
                true
            }
        }
    }

    /// A live map arrived: carry the choice made for the line of each group that `moves` says
    /// moved over to the line of the group it moved to (see `Ancestry::moves_since`), then forget
    /// the choice made for every line whose key `map` no longer knows. Returns whether the choices
    /// changed.
    pub(in crate::dashboard) fn keep_up_with(
        &mut self,
        map: &Snapshot,
        moves: &BTreeMap<GroupKey, GroupKey>,
    ) -> bool {
        let mut carried = false;
        for (from, to) in moves {
            if let Some(choice) = self.0.remove(&saved_as(&LineKey::group(from))) {
                self.0.insert(saved_as(&LineKey::group(to)), choice);
                carried = true;
            }
        }
        let known = known(map);
        let before = self.0.len();
        self.0.retain(|saved, _| known.contains(saved));
        carried || self.0.len() < before
    }
}

/// The words a line's choice is kept under in the settings: its kind, then what it is known by.
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
    let groups = map.ancestry.groups.keys().map(LineKey::group);
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
                groups: BTreeMap::from([(GroupKey::Upstream(APP.to_string()), BTreeSet::new())]),
                made_from: BTreeSet::from([PathBuf::from(SITE)]),
            }),
            ..snapshot(vec![], vec![])
        }
    }

    #[test]
    fn a_choice_is_kept_changed_and_let_go_and_says_when_nothing_changed() {
        let mut chosen = ByLine::default();
        assert!(chosen.set(&app(), Some(3), &map()));
        assert!(
            !chosen.set(&app(), Some(3), &map()),
            "the same choice again"
        );
        assert!(chosen.set(&app(), Some(7), &map()));
        assert_eq!(chosen.get(&app()), Some(&7));

        assert!(chosen.set(&app(), None, &map()));
        assert!(!chosen.set(&app(), None, &map()), "nothing left to let go");
        assert_eq!(chosen, ByLine::default());
    }

    #[test]
    fn a_choice_for_a_line_the_engine_no_longer_knows_changes_nothing() {
        let mut chosen = ByLine::default();
        let gone = LineKey::Group {
            kind: GroupKind::Package {
                name: "gone".to_string(),
            },
        };
        assert!(!chosen.set(&gone, Some(1), &map()));
        let notes = LineKey::Checkout {
            folder: "/code/notes".to_string(),
        };
        assert!(
            !chosen.set(&notes, Some(1), &map()),
            "a checkout nothing was copied from"
        );
        assert_eq!(chosen, ByLine::default());
        let other = LineKey::group(&GroupKey::Other);
        assert!(chosen.set(&other, Some(1), &map()), "Other is always known");
    }

    #[test]
    fn dockers_choice_outlasts_a_map_on_which_docker_has_not_answered_yet() {
        let mut chosen = ByLine::default();
        assert!(chosen.set(&LineKey::DOCKER, Some(1), &map()));
        assert_eq!(map().docker, None, "Docker has said nothing");

        assert!(!chosen.keep_up_with(&map(), &BTreeMap::new()));
        assert_eq!(chosen.get(&LineKey::DOCKER), Some(&1), "still kept");
    }

    #[test]
    fn a_choice_lasts_while_the_engine_knows_its_line_with_or_without_a_card_and_goes_with_it() {
        let mut chosen = ByLine::default();
        chosen.set(&app(), Some(1), &map());
        chosen.set(&site_checkout(), Some(2), &map());
        assert!(
            !chosen.keep_up_with(&map(), &BTreeMap::new()),
            "both known, though no card is listed"
        );

        let only_app = Snapshot {
            ancestry: Arc::new(Ancestry {
                groups: BTreeMap::from([(GroupKey::Upstream(APP.to_string()), BTreeSet::new())]),
                made_from: BTreeSet::new(),
            }),
            ..map()
        };
        assert!(chosen.keep_up_with(&only_app, &BTreeMap::new()));
        assert_eq!(chosen.get(&app()), Some(&1), "app's choice stays");
        assert_eq!(chosen.get(&site_checkout()), None);
    }

    #[test]
    fn a_choice_follows_its_group_to_the_key_it_moved_to_and_no_other_choice_moves() {
        let mut chosen = ByLine::default();
        chosen.set(&app(), Some(1), &map());
        chosen.set(&site_checkout(), Some(2), &map());
        let renamed = GroupKey::Upstream("github.com/me/app-renamed".to_string());
        let moved = Snapshot {
            ancestry: Arc::new(Ancestry {
                groups: BTreeMap::from([(renamed.clone(), BTreeSet::new())]),
                made_from: BTreeSet::from([PathBuf::from(SITE)]),
            }),
            ..map()
        };
        let moves = BTreeMap::from([(GroupKey::Upstream(APP.to_string()), renamed.clone())]);

        assert!(chosen.keep_up_with(&moved, &moves));
        assert_eq!(chosen.get(&LineKey::group(&renamed)), Some(&1));
        assert_eq!(chosen.get(&app()), None);
        assert_eq!(chosen.get(&site_checkout()), Some(&2), "a checkout's stays");
        assert!(
            !chosen.keep_up_with(&moved, &moves),
            "nothing left to carry"
        );
    }

    #[test]
    fn choices_are_saved_under_each_lines_kind_and_key() {
        let mut chosen = ByLine::default();
        chosen.set(&app(), Some("open"), &map());
        chosen.set(&site_checkout(), Some("folded"), &map());
        let saved = serde_json::to_value(&chosen).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({
                "upstream github.com/me/app": "open",
                "checkout /code/site": "folded",
            })
        );
        assert_eq!(
            serde_json::from_value::<ByLine<String>>(saved).unwrap(),
            ByLine(BTreeMap::from([
                ("upstream github.com/me/app".to_string(), "open".to_string()),
                ("checkout /code/site".to_string(), "folded".to_string()),
            ]))
        );
    }
}
