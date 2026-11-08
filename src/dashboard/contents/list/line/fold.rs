//! A line's fold. Folded shut, the line stands alone and still says what the rows under it would of
//! their agents and their servers: a robot for each agent in them stands at its end, every session
//! before every subagent; it throbs while one of the sessions waits on the user; and the first
//! servers up or on their way in them stand after its total, with a count of the rest. Each of
//! those lozenges opens its port on a click, as on its card, and names in its tip the folder it
//! serves in, which the folded line no longer shows.

use std::path::Path;

use crate::dashboard::contents::card::lozenges::{self, Placed};
use crate::dashboard::contents::card::{CardContext, agents, subagents};
use crate::dashboard::listing::Elsewhere;
use crate::dashboard::{AgentBadge, Lozenge, LozengeLook, format};
use crate::map::Project;

/// Whether a line's rows show under it.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum LineFold {
    /// Open: the rows it holds follow it on the list.
    Open,
    /// Folded shut: at the line's end, a robot for each agent in the rows it holds, every session
    /// before every subagent, each in its row's order.
    Folded {
        sessions: Vec<AgentBadge>,
        subagents: Vec<AgentBadge>,
        /// One of those sessions waits on the user, so the line throbs as its card would.
        waiting: bool,
        /// After the line's total, the first two lozenges up or on their way in the cards it
        /// holds, dev-runner units, ports processes listen on and containers alike, in the cards'
        /// order, so folding a line shut never hides that a server runs under it. Each opens its
        /// port as on its card, and its tip names the folders it serves in first.
        lozenges: Vec<Lozenge>,
        /// The lozenges up or on their way past those two; `None` while there are none.
        more_lozenges: Option<MoreLozenges>,
    },
}

/// The lozenges a folded line counts past the ones it draws.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MoreLozenges {
    /// How many, as `+2` reads.
    pub label: String,
    /// Each by its name and port, a line each, as `api :8787` reads.
    pub tooltip: String,
}

/// What a folded line shows of `cards`, the cards it holds, and of the agents working `elsewhere`
/// when it is Other's: a robot for each agent, every session before every subagent, each in the
/// order their rows would stand them, and whether one of the sessions waits on the user, as would
/// make its card throb; and the servers at work in the cards (see `live_lozenges`).
pub(in crate::dashboard::contents) fn folded(
    cards: &[&Project],
    elsewhere: Option<&Elsewhere>,
    ctx: &CardContext,
) -> LineFold {
    let elsewhere_sessions = elsewhere.into_iter().flat_map(|row| &row.sessions);
    let sessions = elsewhere_sessions
        .clone()
        .map(|session| agents::badge(session, ctx.now))
        .chain(
            cards
                .iter()
                .flat_map(|card| agents::badges(&card.agents, ctx.now)),
        );
    let elsewhere_subagents = elsewhere
        .into_iter()
        .flat_map(|row| row.subagents.iter().copied());
    let at_work = elsewhere_subagents.chain(cards.iter().flat_map(|card| &card.subagents));
    let in_cards = cards.iter().flat_map(|card| &card.agents);
    let waiting = agents::any_waiting(elsewhere_sessions.copied().chain(in_cards));
    let (lozenges, more_lozenges) = live_lozenges(cards, ctx);
    LineFold::Folded {
        sessions: sessions.collect(),
        subagents: subagents::badges(at_work, ctx.now),
        waiting,
        lozenges,
        more_lozenges,
    }
}

/// How many lozenges a folded line draws: enough to say that a server runs, and which, and few
/// enough to leave the line its name.
const LOZENGES_ON_A_FOLDED_LINE: usize = 2;

/// The lozenges up or on their way in `cards`, in the cards' order (see `lozenges::on_cards`), as a
/// folded line shows them (see `on_the_line`): the first `LOZENGES_ON_A_FOLDED_LINE` of them, and
/// what counts the rest.
fn live_lozenges(cards: &[&Project], ctx: &CardContext) -> (Vec<Lozenge>, Option<MoreLozenges>) {
    let placed = lozenges::on_cards(cards, ctx).into_iter();
    let live = placed
        .filter(|placed| matches!(placed.lozenge.look, LozengeLook::Up | LozengeLook::Starting));
    first_few(live.map(|placed| on_the_line(placed, ctx.home)).collect())
}

/// `placed` as a folded line shows it: its capsule alone, as the line has room for little beside
/// its name, and a tip that names first what serves and the folders of the cards it stands on, as
/// `web in ~/code/app` reads, then says what its card's would.
fn on_the_line(placed: Placed, home: &Path) -> Lozenge {
    let folders: Vec<String> = placed
        .cards
        .iter()
        .map(|card| format::tilde(&card.path, home))
        .collect();
    let folders = format::all(&folders.iter().map(String::as_str).collect::<Vec<_>>());
    let lozenge = placed.lozenge;
    Lozenge {
        tooltip: format!("{} in {folders}\n{}", lozenge.name, lozenge.tooltip),
        beside: None,
        muted: false,
        notifications_off: false,
        ..lozenge
    }
}

/// The first `LOZENGES_ON_A_FOLDED_LINE` of `lozenges`, and what counts the rest.
pub(in crate::dashboard::contents) fn first_few(
    mut lozenges: Vec<Lozenge>,
) -> (Vec<Lozenge>, Option<MoreLozenges>) {
    let rest = lozenges.split_off(LOZENGES_ON_A_FOLDED_LINE.min(lozenges.len()));
    let more = (!rest.is_empty()).then(|| {
        let named = rest.iter().map(|lozenge| match &lozenge.port {
            Some(port) => format!("{} {port}", lozenge.name),
            None => lozenge.name.clone(),
        });
        MoreLozenges {
            label: format!("+{}", rest.len()),
            tooltip: named.collect::<Vec<_>>().join("\n"),
        }
    });
    (lozenges, more)
}
