//! Secret values that golem masks in everything it writes.
//!
//! A step or a fixture tags a value as secret where the value enters a
//! run ([`register`]). Each output point passes its text through [`mask`],
//! which replaces each secret value with its reference (`${qa.password}`),
//! so the output still says which value stood there.
//!
//! The set is process-wide and never shrinks. The daemon runs many flows,
//! and a secret of one run that is masked in another run's output hides
//! text, but never shows a secret.

use std::borrow::Cow;
use std::sync::RwLock;

/// Shown for a secret whose reference is unknown.
pub const MASK: &str = "•••";

/// `(form, shown)`, the longest form first, so a secret that contains
/// another is replaced whole.
static SECRETS: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

/// Mask `value` from now on, shown as `reference` (`${qa.password}`), or as
/// [`MASK`] when `reference` is empty. An empty value is ignored: it would
/// mask nothing, or everything.
pub fn register(value: &str, reference: &str) {
    if value.is_empty() {
        return;
    }
    let shown = if reference.is_empty() {
        MASK.to_string()
    } else {
        reference.to_string()
    };
    let mut forms = vec![(value.to_string(), shown.clone())];
    // JSON, TOON and `{:?}` escape quotes and backslashes; JUnit escapes
    // XML. A secret that holds one of these shows in that output escaped.
    for escaped in [json_escaped(value), xml_escaped(value)] {
        if escaped != value {
            forms.push((escaped, shown.clone()));
        }
    }
    let mut secrets = SECRETS.write().unwrap_or_else(|e| e.into_inner());
    for form in forms {
        if !secrets.iter().any(|(v, _)| *v == form.0) {
            secrets.push(form);
        }
    }
    secrets.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
}

/// `text` with each secret value replaced by its reference.
pub fn mask(text: &str) -> Cow<'_, str> {
    let secrets = SECRETS.read().unwrap_or_else(|e| e.into_inner());
    let mut out = Cow::Borrowed(text);
    for (value, shown) in secrets.iter() {
        if out.contains(value.as_str()) {
            out = Cow::Owned(out.replace(value.as_str(), shown));
        }
    }
    out
}

/// Whether any secret is registered.
pub fn any() -> bool {
    !SECRETS.read().unwrap_or_else(|e| e.into_inner()).is_empty()
}

fn json_escaped(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn xml_escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    // nextest runs each test in its own process, so the set starts empty.

    #[test]
    fn a_registered_value_shows_as_its_reference() {
        assert_eq!(mask("t:\"s3cret\""), "t:\"s3cret\"");
        register("s3cret", "${qa.password}");
        assert_eq!(mask("t:\"s3cret\""), "t:\"${qa.password}\"");
        assert_eq!(
            mask("Authorization: Bearer s3cret"),
            "Authorization: Bearer ${qa.password}",
            "a string made from a secret SHALL be masked where the secret stands"
        );
        assert!(any());
    }

    #[test]
    fn a_secret_with_no_reference_shows_as_dots() {
        register("tok-123", "");
        assert_eq!(mask("got tok-123"), format!("got {MASK}"));
    }

    #[test]
    fn the_longer_of_two_overlapping_secrets_is_replaced_whole() {
        register("abcd", "${short}");
        register("abcdefgh", "${long}");
        assert_eq!(mask("abcdefgh abcd"), "${long} ${short}");
    }

    #[test]
    fn a_secret_is_masked_in_its_escaped_forms() {
        register(r#"pa"ss\word<1>"#, "${pw}");
        assert_eq!(
            mask(r#"{"input":"pa\"ss\\word<1>"}"#),
            r#"{"input":"${pw}"}"#
        );
        assert_eq!(
            mask("<failure message=\"pa&quot;ss\\word&lt;1&gt;\"/>"),
            "<failure message=\"${pw}\"/>"
        );
    }

    #[test]
    fn an_empty_value_masks_nothing() {
        register("", "${empty}");
        assert!(!any());
        assert_eq!(mask("abc"), "abc");
    }
}
