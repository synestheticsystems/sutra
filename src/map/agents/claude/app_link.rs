//! Opening a session in the Claude desktop app: the link its handler accepts.

use crate::map::agents::SessionHost;

impl SessionHost {
    /// The link that opens the session in the app it runs in, when there is one.
    pub fn link(&self) -> Option<String> {
        match self {
            SessionHost::ClaudeApp {
                app_session_id: Some(id),
            } => claude_app_link(id),
            SessionHost::ClaudeApp {
                app_session_id: None,
            }
            | SessionHost::Terminal => None,
        }
    }
}

/// `claude://code/continue?session=<id>`, which the Claude desktop app routes to that session.
/// The format is undocumented. It comes from the app's own link handler, in
/// `/Applications/Claude.app/Contents/Resources/app.asar` (read 2026-11-04), which accepts `last`
/// (what the app's Dock menu sends) or an id matching `^local_[A-Za-z0-9-]{1,64}$`. An id of any
/// other form gets no link, because the app would refuse it; if these links stop working after an
/// app update, that handler is the place to look.
fn claude_app_link(app_session_id: &str) -> Option<String> {
    let suffix = app_session_id.strip_prefix("local_")?;
    let well_formed = (1..=64).contains(&suffix.len())
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    well_formed.then(|| format!("claude://code/continue?session={app_session_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_app(app_session_id: Option<&str>) -> SessionHost {
        SessionHost::ClaudeApp {
            app_session_id: app_session_id.map(str::to_string),
        }
    }

    #[test]
    fn a_claude_app_session_with_a_well_formed_id_links_to_itself() {
        let id = "local_2b5d1f5a-66c8-4826-b823-26ed94dd90a3";
        assert_eq!(
            in_app(Some(id)).link().as_deref(),
            Some("claude://code/continue?session=local_2b5d1f5a-66c8-4826-b823-26ed94dd90a3")
        );
        let longest = format!("local_{}", "a".repeat(64));
        assert!(in_app(Some(&longest)).link().is_some());
    }

    #[test]
    fn an_id_the_claude_app_would_refuse_gets_no_link() {
        let too_long = format!("local_{}", "a".repeat(65));
        for refused in [
            "local_",
            "local_has space",
            "local_a&b=c",
            "abc-123",
            "last",
            &too_long,
        ] {
            assert_eq!(in_app(Some(refused)).link(), None, "{refused}");
        }
        assert_eq!(in_app(None).link(), None);
        assert_eq!(SessionHost::Terminal.link(), None);
    }
}
