//! The list's rows, as flat as the window lays them out, one row per line and per card: each
//! group's line, then, unless it is folded shut, what hangs from it, down through each checkout's
//! line to its cards, each row with its place in the group's tree (see `Place`); and the row
//! standing for the quiet groups.

use super::{docker, elsewhere, how_many, line, what_makes_one_quiet};
use crate::dashboard::contents::card::{self, CardContext, agents};
use crate::dashboard::listing::{Folds, Group, Listing, Node, Row};
use crate::dashboard::{Card, DockerCard, ElsewhereLine, Line, LineFold, LineKey, format};
use crate::map::Snapshot;

/// One row of the list.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum ListRow {
    /// A line: a group's, which names the upstream its cards came from, or within a group a
    /// checkout's, which holds its own card first, then the copies made from it. A click on it
    /// folds what it holds shut, and another opens it (`Action::ToggleFold`).
    Line {
        // Boxed, as a line and what it shows folded dwarf the other rows; the window receives it
        // as a plain line.
        line: Box<Line>,
        place: Place,
    },
    /// A project's card, under its line. Every card stands in a group, going by what sets it apart
    /// from the rows beside it: under its repository's own line a worktree by its branch, and any
    /// other card by its folder's name.
    Card {
        // Boxed, as a card dwarfs the other rows; the window receives it as a plain card.
        card: Box<Card>,
        place: Place,
    },
    /// Docker's card, alone under its group's line: the containers running for no project, and
    /// the disk Docker takes. The window knows it as the one Docker card there is.
    Docker {
        // Boxed, as a card dwarfs the other rows; the window receives it as a plain card.
        card: Box<DockerCard>,
        place: Place,
    },
    /// The agents working outside every project, which stand in Other, under `heading`: a line
    /// for each session, in the map's order, then one for each other folder subagents work in, in
    /// the order of the first subagent there. No folder has two lines of subagents, and none moves
    /// when a status changes.
    Elsewhere {
        heading: String,
        lines: Vec<ElsewhereLine>,
        /// One of them waits on the user, so the row throbs as a card does.
        waiting: bool,
        place: Place,
    },
    /// The row standing for the quiet groups: how many projects they hold, and whether they are
    /// listed below it. A click opens or closes it (`Action::ToggleQuietProjects`).
    QuietProjects {
        count: u32,
        open: bool,
        label: String,
        tooltip: String,
    },
}

/// Where a row stands in its group's tree, which the window draws as its indent, and as its stretch
/// of the guide that runs down from the chevron of each line it hangs under, past every row that
/// line holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct Place {
    /// How many lines it hangs under: none for a group's line, one for a row under it, and one
    /// more for each checkout's line in between.
    pub depth: u32,
    /// It hangs first under its line, so that line's guide reaches up into the line, to its
    /// chevron.
    pub first: bool,
}

/// The rows of `row`, one of the listing's.
pub(super) fn of(row: &Row, listing: &Listing, folds: &Folds, cards: &CardContext) -> Vec<ListRow> {
    match row {
        Row::Group(group) => Flattening {
            map: listing.snapshot,
            folds,
            cards,
            rows: Vec::new(),
        }
        .group(group),
        Row::Docker(docker) => {
            let folded = folds.is_folded(&LineKey::DOCKER);
            let line = ListRow::Line {
                line: Box::new(docker::line(docker, folded, cards)),
                place: Place {
                    depth: 0,
                    first: false,
                },
            };
            let card = (!folded).then(|| ListRow::Docker {
                card: Box::new(docker::card(docker, cards)),
                place: Place {
                    depth: 1,
                    first: true,
                },
            });
            std::iter::once(line).chain(card).collect()
        }
        Row::QuietProjects { count, shown } => vec![ListRow::QuietProjects {
            count: format::count(*count),
            open: *shown,
            label: how_many(*count),
            tooltip: what_makes_one_quiet(listing.sort),
        }],
    }
}

/// The rows of one group as they are laid out, a line's after it.
struct Flattening<'a> {
    map: &'a Snapshot,
    folds: &'a Folds,
    cards: &'a CardContext<'a>,
    rows: Vec<ListRow>,
}

impl Flattening<'_> {
    /// The rows of `group`: its line, then, while it is open, what hangs from it.
    fn group(mut self, group: &Group) -> Vec<ListRow> {
        let open = !self.folds.is_folded(&LineKey::group(group.key));
        let fold = if open {
            LineFold::Open
        } else {
            let elsewhere = group.children.iter().find_map(|node| match node {
                Node::Elsewhere(elsewhere) => Some(elsewhere),
                _ => None,
            });
            line::folded(&group.cards(), elsewhere, self.cards)
        };
        let line = line::group_line(group, fold, self.cards);
        let place = Place {
            depth: 0,
            first: false,
        };
        self.rows.push(ListRow::Line {
            line: Box::new(line),
            place,
        });
        if open {
            self.nodes(&group.children, 1);
        }
        self.rows
    }

    /// The rows of `nodes`, the rows under one line, each `depth` lines deep.
    fn nodes(&mut self, nodes: &[Node], depth: u32) {
        for (index, node) in nodes.iter().enumerate() {
            let place = Place {
                depth,
                first: index == 0,
            };
            match node {
                Node::Card { project, naming } => {
                    let card = card::card(project, naming, self.map, self.cards);
                    self.rows.push(ListRow::Card {
                        card: Box::new(card),
                        place,
                    });
                }
                Node::Elsewhere(working) => self.rows.push(ListRow::Elsewhere {
                    heading: "Elsewhere".to_string(),
                    lines: elsewhere::lines(working, self.cards),
                    waiting: agents::any_waiting(working.sessions.iter().copied()),
                    place,
                }),
                Node::Checkout(checkout) => {
                    let open = !self
                        .folds
                        .is_folded(&LineKey::checkout(&checkout.project.path));
                    let fold = if open {
                        LineFold::Open
                    } else {
                        line::folded(&node.cards(), None, self.cards)
                    };
                    let line = line::checkout_line(checkout, fold, self.cards);
                    self.rows.push(ListRow::Line {
                        line: Box::new(line),
                        place,
                    });
                    if open {
                        self.nodes(&checkout.children, depth + 1);
                    }
                }
            }
        }
    }
}
