//! Codex runs each sub-agent as a thread of its own, whose `source` names the thread that spawned
//! it: `{"subagent":{"thread_spawn":{"parent_thread_id":"…","depth":1,…}}}`. A sub-agent's work
//! counts for the user-facing thread it descends from, which is the one the map shows.
//!
//! Codex also starts a guardian thread, `{"subagent":{"other":"guardian"}}`, to review a command a
//! thread wants approved. It counts for nothing here: a review runs inside its parent's open turn,
//! which reads as busy already, and is short (3,375 reviews on Daniel's Mac, the longest 57 s,
//! 2026-11-05).

use std::collections::{HashMap, HashSet};

use super::Thread;

/// Where a thread came from, as its `source` says.
#[derive(Debug, PartialEq, Eq)]
enum Origin {
    /// The terminal UI, the Desktop app (`vscode`) or `codex exec`.
    User,
    /// Spawned as a sub-agent by the thread with this id.
    SpawnedBy(String),
    /// Anything else, such as a review thread or a guardian.
    Other,
}

impl Thread {
    fn origin(&self) -> Origin {
        if ["cli", "vscode", "exec"].contains(&self.source.as_str()) {
            return Origin::User;
        }
        let Ok(source) = serde_json::from_str::<serde_json::Value>(&self.source) else {
            return Origin::Other;
        };
        match source.pointer("/subagent/thread_spawn/parent_thread_id") {
            Some(serde_json::Value::String(parent)) => Origin::SpawnedBy(parent.clone()),
            _ => Origin::Other,
        }
    }
}

/// The threads of one load: the user-facing ones, which the map shows, and the sub-agent threads,
/// each with the user-facing thread it counts for.
#[derive(Debug)]
pub(super) struct Threads {
    pub(super) user_facing: Vec<Thread>,
    pub(super) sub_agents: Vec<(Thread, String)>,
}

impl Threads {
    /// Sort out `recent`, the threads that wrote lately, in the order given. A thread a sub-agent
    /// descends from that `recent` lacks is looked up with `find`, which finds a thread by id
    /// unless it is off the map. A thread waiting on its sub-agents can go unwritten for hours
    /// while they work (two of the ten on Daniel's Mac whose rollouts remain, 2026-11-05), and one
    /// looked up joins the user-facing threads after the recent ones. A sub-agent whose line of
    /// parents ends off the map, or loops, counts for nothing.
    pub(super) fn sort_out<E>(
        recent: Vec<Thread>,
        mut find: impl FnMut(&str) -> Result<Option<Thread>, E>,
    ) -> Result<Threads, E> {
        let mut user_facing = Vec::new();
        let mut spawned = Vec::new();
        for thread in recent {
            match thread.origin() {
                Origin::User => user_facing.push(thread),
                Origin::SpawnedBy(parent) => spawned.push((thread, parent)),
                Origin::Other => {}
            }
        }
        let mut parents: HashMap<String, String> = spawned
            .iter()
            .map(|(thread, parent)| (thread.id.clone(), parent.clone()))
            .collect();
        let mut sub_agents = Vec::new();
        for (sub_agent, mut id) in spawned {
            let mut passed = HashSet::new();
            let root = loop {
                if user_facing.iter().any(|thread| thread.id == id) {
                    break Some(id);
                }
                if !passed.insert(id.clone()) {
                    break None;
                }
                id = match parents.get(&id) {
                    Some(parent) => parent.clone(),
                    None => match find(&id)?.map(|thread| (thread.origin(), thread)) {
                        Some((Origin::User, thread)) => {
                            user_facing.push(thread);
                            break Some(id);
                        }
                        Some((Origin::SpawnedBy(parent), _)) => {
                            parents.insert(id, parent.clone());
                            parent
                        }
                        Some((Origin::Other, _)) | None => break None,
                    },
                };
            };
            sub_agents.extend(root.map(|root| (sub_agent, root)));
        }
        Ok(Threads {
            user_facing,
            sub_agents,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    /// The source of a thread spawned by `parent`, as Codex writes it.
    fn spawned_by(parent: &str) -> String {
        serde_json::json!({"subagent": {"thread_spawn": {
            "parent_thread_id": parent, "depth": 1, "agent_nickname": "Boyle", "agent_role": "explorer",
        }}})
        .to_string()
    }

    fn thread(id: &str, source: &str) -> Thread {
        Thread {
            id: id.to_string(),
            rollout_path: None,
            cwd: "/code/app".to_string(),
            source: source.to_string(),
            updated_at_ms: 0,
        }
    }

    fn ids(threads: &[Thread]) -> Vec<&str> {
        threads.iter().map(|thread| thread.id.as_str()).collect()
    }

    fn pairs(sub_agents: &[(Thread, String)]) -> Vec<(&str, &str)> {
        let pairs = sub_agents.iter();
        pairs
            .map(|(sub_agent, root)| (sub_agent.id.as_str(), root.as_str()))
            .collect()
    }

    #[test]
    fn a_thread_comes_from_the_user_from_a_spawn_or_from_elsewhere() {
        let guardian = r#"{"subagent":{"other":"guardian"}}"#;
        for (source, origin) in [
            ("cli", Origin::User),
            ("vscode", Origin::User),
            ("exec", Origin::User),
            (&spawned_by("p1"), Origin::SpawnedBy("p1".to_string())),
            (guardian, Origin::Other),
            (r#"{"subagent":"review"}"#, Origin::Other),
            (
                r#"{"subagent":{"thread_spawn":{"depth":1}}}"#,
                Origin::Other,
            ),
            (r#"{"subagent":{"thread_spawn":"#, Origin::Other),
            ("thread_spawn", Origin::Other),
        ] {
            assert_eq!(thread("t", source).origin(), origin, "{source}");
        }
    }

    #[test]
    fn a_sub_agent_counts_for_its_user_facing_ancestor_looked_up_only_when_missing() {
        let recent = vec![
            thread("app", "vscode"),
            thread("explorer", &spawned_by("app")),
            thread("nested", &spawned_by("waiting")),
            thread("guardian", r#"{"subagent":{"other":"guardian"}}"#),
        ];
        let mut looked_up = Vec::new();
        let older = |id: &str| -> Result<Option<Thread>, Infallible> {
            looked_up.push(id.to_string());
            Ok(match id {
                "waiting" => Some(thread("waiting", &spawned_by("quiet-app"))),
                "quiet-app" => Some(thread("quiet-app", "cli")),
                _ => None,
            })
        };

        let Ok(threads) = Threads::sort_out(recent, older);

        assert_eq!(ids(&threads.user_facing), ["app", "quiet-app"]);
        assert_eq!(
            pairs(&threads.sub_agents),
            [("explorer", "app"), ("nested", "quiet-app")]
        );
        assert_eq!(looked_up, ["waiting", "quiet-app"]);
    }

    #[test]
    fn a_sub_agent_whose_parents_loop_or_end_off_the_map_counts_for_nothing() {
        let recent = vec![
            thread("a", &spawned_by("b")),
            thread("b", &spawned_by("a")),
            thread("orphan", &spawned_by("archived")),
        ];
        let off_the_map = |_: &str| -> Result<Option<Thread>, Infallible> { Ok(None) };

        // On a thread, so a loop that went round for ever fails the test instead of hanging it.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok(threads) = Threads::sort_out(recent, off_the_map);
            let _ = sender.send(threads);
        });
        let threads = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the sorting returns despite the loop");

        assert!(threads.user_facing.is_empty());
        assert!(threads.sub_agents.is_empty());
    }
}
