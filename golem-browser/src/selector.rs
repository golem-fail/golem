use anyhow::{anyhow, Result};
use golem_events::FailureCode;
use golem_parser::Step;

/// What a browser step acts on.
///
/// Browser targeting is CSS, and only CSS. golem's mobile selectors (text,
/// accessibility label, type, the relational `below`/`above`/`child_of`
/// anchors) describe a native view tree and have no meaning in a DOM, so a
/// browser step ignores them rather than pretending to honour them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserTarget {
    /// A raw CSS selector, passed to the page verbatim.
    pub selector: String,
    /// Which match to act on, 0-based — the same numbering as mobile
    /// `on_index` (see `docs/selectors.md`). A selector that matches several
    /// elements takes the first unless a step says otherwise, which is what
    /// mobile golem does with an ambiguous selector.
    pub index: usize,
}

/// Resolve what a browser step targets from its params.
///
/// The selector is trimmed but never parsed or rewritten: CSS semantics belong
/// to the page's engine, and a golem-side parser would eventually reject valid
/// selectors it hadn't been taught about.
pub fn resolve_target(step: &Step) -> Result<BrowserTarget> {
    let selector = step
        .params
        .get("selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            golem_events::coded(
                FailureCode::ParseMissingParam,
                anyhow!(
                    "browser steps target by CSS — `selector` is required. golem's mobile \
                     selectors (text, type, below/above/child_of) don't apply in a browser."
                ),
            )
        })?;

    Ok(BrowserTarget {
        selector: selector.to_string(),
        index: resolve_index(step)?,
    })
}

fn resolve_index(step: &Step) -> Result<usize> {
    let Some(raw) = step.params.get("index") else {
        return Ok(0);
    };
    let invalid = |detail: String| {
        golem_events::coded(
            FailureCode::ParseMissingParam,
            anyhow!("browser step `index` {detail} — expected a 0-based whole number"),
        )
    };
    let n = raw
        .as_integer()
        .ok_or_else(|| invalid(format!("must be a number, got `{raw}`")))?;
    usize::try_from(n).map_err(|_| invalid(format!("cannot be negative, got `{n}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(toml_src: &str) -> Step {
        toml::from_str(toml_src).expect("fixture SHALL parse")
    }

    // 1. A selector reaches the page exactly as written — no parsing, no
    //    rewriting, however exotic the CSS.
    #[test]
    fn selector_passes_through_verbatim() {
        for css in [
            "#submit",
            "button.primary[data-state='ready']",
            "form > input:nth-of-type(2)",
            "li:has(> a[href^='/orders']) span",
        ] {
            let s = step(&format!("action = \"browse_tap\"\nselector = \"{css}\""));
            let target = resolve_target(&s).expect("selector SHALL resolve");
            assert_eq!(target.selector, css);
        }
    }

    // 2. Surrounding whitespace is not part of the selector.
    #[test]
    fn selector_is_trimmed() {
        let s = step("action = \"browse_tap\"\nselector = \"  #submit  \"");
        assert_eq!(
            resolve_target(&s)
                .expect("padded selector SHALL resolve")
                .selector,
            "#submit"
        );
    }

    // 3. A selector matching several elements takes the first by default.
    #[test]
    fn index_defaults_to_the_first_match() {
        let s = step("action = \"browse_tap\"\nselector = \".row\"");
        assert_eq!(
            resolve_target(&s).expect("selector SHALL resolve").index,
            0,
            "an ambiguous selector SHALL take the first match, like mobile golem"
        );
    }

    // 4. `index` picks a later match, 0-based like mobile `on_index`.
    #[test]
    fn index_selects_the_nth_match() {
        let s = step("action = \"browse_tap\"\nselector = \".row\"\nindex = 2");
        assert_eq!(
            resolve_target(&s)
                .expect("indexed selector SHALL resolve")
                .index,
            2
        );
    }

    // 5. A missing selector is a param error that says what browser targeting
    //    actually needs — this is what a step written with mobile selectors hits.
    #[test]
    fn missing_selector_explains_that_targeting_is_css() {
        let s = step("action = \"browse_tap\"\non_text = \"Login\"");
        let e = resolve_target(&s).expect_err("a browser step SHALL require a selector");
        let msg = format!("{e:#}");
        assert!(
            msg.contains("CSS") && msg.contains("selector"),
            "message SHALL point at CSS, got: {msg}"
        );
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
    }

    // 6. An empty or whitespace-only selector is missing, not a match-nothing
    //    selector — the alternative is a step that silently never finds anything.
    #[test]
    fn blank_selector_is_treated_as_missing() {
        for css in ["", "   "] {
            let s = step(&format!("action = \"browse_tap\"\nselector = \"{css}\""));
            let e = resolve_target(&s).expect_err("blank selector SHALL be rejected");
            assert_eq!(
                golem_events::extract_code(&e),
                Some(FailureCode::ParseMissingParam)
            );
        }
    }

    // 7. A negative index is rejected here rather than silently clamped.
    #[test]
    fn negative_index_is_rejected() {
        let s = step("action = \"browse_tap\"\nselector = \".row\"\nindex = -1");
        let e = resolve_target(&s).expect_err("a negative index SHALL be rejected");
        let msg = format!("{e:#}");
        assert!(
            msg.contains("negative"),
            "message SHALL name the problem, got: {msg}"
        );
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
    }

    // 8. A non-numeric index is a param error, not a silent default to 0.
    #[test]
    fn non_numeric_index_is_rejected() {
        let s = step("action = \"browse_tap\"\nselector = \".row\"\nindex = \"second\"");
        let e = resolve_target(&s).expect_err("a non-numeric index SHALL be rejected");
        assert!(format!("{e:#}").contains("must be a number"));
        assert_eq!(
            golem_events::extract_code(&e),
            Some(FailureCode::ParseMissingParam)
        );
    }

    // 9. Mobile selector fields alongside a CSS selector are ignored, not
    //    merged — a browser step is targeted by its CSS alone.
    #[test]
    fn mobile_selector_fields_are_ignored() {
        let s = step(
            r##"
            action = "browse_tap"
            selector = "#submit"
            on_text = "Login"
            on_index = 7
            on_below = "Header"
            "##,
        );
        let target = resolve_target(&s).expect("CSS selector SHALL win");
        assert_eq!(target.selector, "#submit");
        assert_eq!(
            target.index, 0,
            "mobile `on_index` SHALL NOT feed the browser index"
        );
    }
}
