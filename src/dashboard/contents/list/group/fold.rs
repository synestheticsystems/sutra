//! A group's fold. Folded shut, the line stands alone and still says what its cards would of their
//! agents and their dev servers: a robot for each agent in them stands at its end, every session
//! before every subagent; it throbs while one of the sessions waits on the user; and the first
//! servers up or on their way in them stand after its total, with a count of the rest.

use crate::dashboard::contents::card::{CardContext, agents, subagents, units};
use crate::dashboard::{GroupFold, MoreUnits, UnitLook, UnitLozenge};
use crate::map::Project;
use crate::notifications::Notifier;

/// What the line of a folded group shows of `cards`, the cards it holds: a robot for each agent in
/// them, every session before every subagent, each in the order their cards would stand them, and
/// whether one of the sessions waits on the user, as would make its card throb; and the dev
/// servers at work in them (see `live_units`).
pub(super) fn folded(cards: &[&Project], ctx: &CardContext) -> GroupFold {
    let sessions = cards
        .iter()
        .flat_map(|card| agents::badges(&card.agents, ctx.now));
    let at_work = cards.iter().flat_map(|card| &card.subagents);
    let waiting = agents::any_waiting(cards.iter().flat_map(|card| &card.agents));
    let (units, more_units) = live_units(cards, ctx.notifier);
    GroupFold::Folded {
        sessions: sessions.collect(),
        subagents: subagents::badges(at_work, ctx.now),
        waiting,
        units,
        more_units,
    }
}

/// How many units a folded line draws as lozenges: enough to say that a server runs, and which,
/// and few enough to leave the line its name.
const UNITS_ON_A_FOLDED_LINE: usize = 2;

/// The dev-runner units up or on their way in `cards`, as their cards draw them, in the cards'
/// order: the first `UNITS_ON_A_FOLDED_LINE` of them, and what counts the rest.
fn live_units(cards: &[&Project], notifier: &Notifier) -> (Vec<UnitLozenge>, Option<MoreUnits>) {
    let environments = cards.iter().filter_map(|card| card.environment.as_ref());
    let lozenges = environments.flat_map(|environment| units::lozenges(environment, notifier));
    let live = lozenges.filter(|unit| matches!(unit.look, UnitLook::Up | UnitLook::Starting));
    let mut shown: Vec<UnitLozenge> = live.collect();
    let rest = shown.split_off(UNITS_ON_A_FOLDED_LINE.min(shown.len()));
    let more = (!rest.is_empty()).then(|| {
        let named = rest.iter().map(|unit| match &unit.port {
            Some(port) => format!("{} {port}", unit.name),
            None => unit.name.clone(),
        });
        MoreUnits {
            label: format!("+{}", rest.len()),
            tooltip: named.collect::<Vec<_>>().join("\n"),
        }
    });
    (shown, more)
}

#[cfg(test)]
mod tests {
    use super::super::tests::first_group;
    use super::*;
    use crate::dashboard::listing::Folds;
    use crate::dashboard::listing::fixtures::{
        agent, linked_worktree, main_worktree, project, snapshot, subagent_in,
    };
    use crate::dashboard::{AgentLook, Parent};
    use crate::map::AgentStatus;
    use crate::map::test_support::measured;

    #[test]
    fn a_folded_group_keeps_its_name_and_disk_and_shows_its_cards_robots_sessions_first() {
        let app = "/code/app/.git";
        let map = |fix_status| {
            let mut main = main_worktree(app, "main", project("app", 1));
            main.own_files = measured(2_000_000_000);
            main.agents.push(agent("/code/app", AgentStatus::Busy));
            main.subagents.push(subagent_in("/code/app"));
            let mut fix = linked_worktree(app, "fix", project("app-fix", 1));
            fix.agents.push(agent("/code/app-fix", fix_status));
            snapshot(vec![main, fix], vec![])
        };
        let mut folds = Folds::default();
        let parent = Parent::Repository {
            common_dir: app.to_string(),
        };
        folds.toggle(&parent, &map(AgentStatus::Waiting));
        let folded = |fix_status| {
            let (line, fold) = first_group(&map(fix_status), &folds);
            let GroupFold::Folded {
                sessions,
                subagents,
                waiting,
                ..
            } = fold
            else {
                panic!("folded: {fold:?}");
            };
            let looks: Vec<AgentLook> = sessions.iter().map(|badge| badge.look).collect();
            (line.name, line.total, looks, subagents.len(), waiting)
        };

        assert_eq!(
            folded(AgentStatus::Waiting),
            (
                "app".to_string(),
                Some("2 GB".to_string()),
                vec![AgentLook::Working, AgentLook::Waiting],
                1,
                true
            ),
            "app's session before app-fix's, then the subagent, throbbing for app-fix's"
        );
        let (.., waiting) = folded(AgentStatus::Busy);
        assert!(!waiting, "no throb while no session waits");
    }

    #[test]
    fn a_folded_group_shows_two_of_the_servers_up_in_its_cards_and_counts_the_rest() {
        use crate::map::test_support::environment_at;
        use crate::model::{Environment, UnitStatus};
        let app = "/code/app/.git";
        // Each unit as `name:status:port`, in the environment of the worktree at `path`.
        let serving = |path: &str, units: &[(&str, &str, u16)]| Environment {
            units: units
                .iter()
                .map(|(name, status, _)| UnitStatus::parse(name, status))
                .collect(),
            ports: units
                .iter()
                .map(|(name, _, port)| (name.to_string(), *port))
                .collect(),
            ..environment_at(path)
        };
        let mut main = main_worktree(app, "main", project("app", 1));
        main.environment = Some(serving(
            "/code/app",
            &[("web", "ready", 5173), ("worker", "stopped", 9000)],
        ));
        let mut fix = linked_worktree(app, "fix", project("app-fix", 1));
        fix.environment = Some(serving(
            "/code/app-fix",
            &[("api", "starting", 8787), ("docs", "running", 4000)],
        ));
        let map = snapshot(vec![main, fix], vec![]);
        let mut folds = Folds::default();
        let parent = Parent::Repository {
            common_dir: app.to_string(),
        };
        folds.toggle(&parent, &map);

        let (_, fold) = first_group(&map, &folds);
        let GroupFold::Folded {
            units, more_units, ..
        } = fold
        else {
            panic!("folded: {fold:?}");
        };
        let shown: Vec<(&str, Option<&str>)> = units
            .iter()
            .map(|unit| (unit.name.as_str(), unit.port.as_deref()))
            .collect();
        assert_eq!(
            shown,
            [("web", Some(":5173")), ("api", Some(":8787"))],
            "up or on its way, in the cards' order; not the stopped worker"
        );
        assert_eq!(
            more_units,
            Some(MoreUnits {
                label: "+1".to_string(),
                tooltip: "docs :4000".to_string(),
            })
        );
    }
}
