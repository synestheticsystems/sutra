//! A line's fold. Folded shut, the line stands alone and still says what the rows under it would of
//! their agents and their servers: a robot for each agent in them stands at its end, every session
//! before every subagent; it throbs while one of the sessions waits on the user; and the first
//! servers up or on their way in them stand after its total, with a count of the rest.

use crate::dashboard::contents::card::{CardContext, agents, lozenges, subagents};
use crate::dashboard::listing::Elsewhere;
use crate::dashboard::{AgentBadge, Lozenge, LozengeLook};
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
        /// order, as their cards draw them, so folding a line shut never hides that a server runs
        /// under it.
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

/// The lozenges up or on their way in `cards`, as their cards draw them, in the cards' order: the
/// first `LOZENGES_ON_A_FOLDED_LINE` of them, and what counts the rest.
fn live_lozenges(cards: &[&Project], ctx: &CardContext) -> (Vec<Lozenge>, Option<MoreLozenges>) {
    let all = cards.iter().flat_map(|card| lozenges::lozenges(card, ctx));
    let live =
        all.filter(|lozenge| matches!(lozenge.look, LozengeLook::Up | LozengeLook::Starting));
    first_few(live.collect())
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
