use anyhow::{anyhow, Result};
use golem_events::FailureCode;

/// Browser context used when a step omits `session`.
pub const DEFAULT_CONTEXT: &str = "_default";
/// Tab used when a step omits `session`.
pub const DEFAULT_SESSION: &str = "_default";

/// Where a browser step runs: which context, and which tab inside it.
///
/// Labels are flow-local. Each flowrun owns its own browser process, so two
/// flowruns naming the same context are already separate Chromes — the labels
/// need no hashing or prefixing to stay isolated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    pub context: String,
    pub session: String,
}

impl Default for SessionRef {
    fn default() -> Self {
        Self {
            context: DEFAULT_CONTEXT.to_string(),
            session: DEFAULT_SESSION.to_string(),
        }
    }
}

/// Parse a `session` param: `[context:]session`, split on the FIRST `:`.
///
/// `:` is reserved from day one even though only the default context exists.
/// Accepting `"tenantX:admin"` as a tab literally named `tenantX:admin` would
/// silently do the wrong thing today and change meaning the moment #109 lands,
/// so a context prefix parses and then errors out instead.
pub fn parse_session(raw: Option<&str>) -> Result<SessionRef> {
    let raw = raw.map(str::trim).unwrap_or_default();
    if raw.is_empty() {
        return Ok(SessionRef::default());
    }

    let Some((context, session)) = raw.split_once(':') else {
        return Ok(SessionRef {
            context: DEFAULT_CONTEXT.to_string(),
            session: raw.to_string(),
        });
    };

    let (context, session) = (context.trim(), session.trim());
    if context.is_empty() || session.is_empty() || session.contains(':') {
        return Err(golem_events::coded(
            FailureCode::ParseMissingParam,
            anyhow!(
                "invalid session `{raw}` — expected `session` or `context:session`, \
                 and neither label may contain `:`"
            ),
        ));
    }

    Err(golem_events::coded(
        FailureCode::ParseMissingParam,
        anyhow!(
            "multiple browser contexts not yet supported (session `{raw}` asks for \
             context `{context}`) — see https://github.com/golem-fail/golem/issues/109"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. An omitted session lands on the default context and default tab.
    #[test]
    fn omitted_session_is_default_context_and_tab() {
        let s = parse_session(None).expect("None SHALL parse");
        assert_eq!(s.context, DEFAULT_CONTEXT);
        assert_eq!(s.session, DEFAULT_SESSION);
        assert_eq!(s, SessionRef::default());
    }

    // 2. Blank and whitespace-only values are the same as omitting it.
    #[test]
    fn blank_session_is_treated_as_omitted() {
        for raw in ["", "   "] {
            let s = parse_session(Some(raw)).expect("blank SHALL parse");
            assert_eq!(s, SessionRef::default(), "`{raw}` SHALL mean default");
        }
    }

    // 3. A bare label names a tab in the default context.
    #[test]
    fn bare_label_is_a_tab_in_the_default_context() {
        let s = parse_session(Some("admin")).expect("bare label SHALL parse");
        assert_eq!(s.context, DEFAULT_CONTEXT);
        assert_eq!(s.session, "admin");
    }

    // 4. Surrounding whitespace is not part of the label.
    #[test]
    fn labels_are_trimmed() {
        let s = parse_session(Some("  admin  ")).expect("padded label SHALL parse");
        assert_eq!(s.session, "admin");
    }

    // 5. A context prefix is recognised and rejected, pointing at #109 —
    //    never silently accepted as a tab whose name contains a colon.
    #[test]
    fn context_prefix_errors_with_the_followup_issue() {
        let e = parse_session(Some("tenantX:admin")).expect_err("context prefix SHALL error");
        let msg = format!("{e:#}");
        assert!(
            msg.contains("not yet supported"),
            "message SHALL say contexts are unsupported, got: {msg}"
        );
        assert!(
            msg.contains("109"),
            "message SHALL point at #109, got: {msg}"
        );
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
    }

    // 6. Malformed values are a param error, not a context request.
    #[test]
    fn malformed_session_values_are_param_errors() {
        for raw in [":admin", "tenantX:", "a:b:c", ":"] {
            let e = parse_session(Some(raw)).expect_err("`{raw}` SHALL error");
            let msg = format!("{e:#}");
            assert!(
                msg.contains("invalid session"),
                "`{raw}` SHALL be rejected as malformed, got: {msg}"
            );
            assert_eq!(
                golem_events::extract_code(&e),
                Some(FailureCode::ParseMissingParam)
            );
        }
    }
}
