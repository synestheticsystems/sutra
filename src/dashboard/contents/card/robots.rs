//! Which robot stands for an agent: the AI it is, told by the model its transcript names, as
//! `claude-opus-5-5` or `gpt-6-astra` reads, or, while none is known, by the harness it runs in,
//! whose own models it runs unless told otherwise.

use crate::dashboard::Robot;
use crate::map::Harness;

/// The robot of an agent running in `harness` on `model`, when its harness records the model.
pub(in crate::dashboard::contents) fn robot(harness: Harness, model: Option<&str>) -> Robot {
    match model {
        Some(model) => by_model(model),
        None => match harness {
            Harness::ClaudeCode => Robot::Claude,
            Harness::Codex => Robot::Codex,
        },
    }
}

/// The robot of `model`, by the name of its family, with a provider's prefix, as `openai/gpt-5`
/// has, set aside. A family with no robot of its own gets the plain one.
fn by_model(model: &str) -> Robot {
    let name = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let of_family = |prefixes: &[&str]| prefixes.iter().any(|prefix| name.starts_with(prefix));
    if of_family(&["claude"]) {
        Robot::Claude
    } else if of_family(&["gpt", "codex", "o1", "o3", "o4"]) {
        Robot::Codex
    } else if of_family(&["gemini"]) {
        Robot::Gemini
    } else {
        Robot::Plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_named_picks_its_familys_robot_whatever_harness_runs_it() {
        let cases = [
            ("claude-opus-5-5", Robot::Claude),
            ("Claude-Sonnet-5-5", Robot::Claude),
            ("gpt-6-astra", Robot::Codex),
            ("openai/gpt-5.5", Robot::Codex),
            ("o3-mini", Robot::Codex),
            ("gemini-3-pro", Robot::Gemini),
            ("glm-4.6", Robot::Plain),
        ];
        for (model, expected) in cases {
            assert_eq!(robot(Harness::ClaudeCode, Some(model)), expected, "{model}");
            assert_eq!(robot(Harness::Codex, Some(model)), expected, "{model}");
        }
    }

    #[test]
    fn with_no_model_known_the_harness_picks_the_robot_of_its_own_models() {
        assert_eq!(robot(Harness::ClaudeCode, None), Robot::Claude);
        assert_eq!(robot(Harness::Codex, None), Robot::Codex);
    }
}
