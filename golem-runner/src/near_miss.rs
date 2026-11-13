//! Hints for a selector that matched nothing: the visible texts it nearly
//! matched, so the failure points at the fix. Read only after a timeout, so a
//! passing step pays nothing.

/// At most this many hints, best first.
const MAX_HINTS: usize = 3;

/// Normalized Damerau-Levenshtein similarity (a swap of two letters is one
/// edit) a candidate needs to be "similar".
const SIMILAR: f64 = 0.75;

/// Lines that suggest how `wanted` (a selector's text, matched whole) could
/// match one of the `visible` texts. Empty when nothing is close, or when
/// `wanted` is already a pattern.
pub(crate) fn hints(wanted: &str, visible: &[&str]) -> Vec<String> {
    if wanted.is_empty() || wanted.contains(['*', '?']) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut push = |cand: &str, line: String, out: &mut Vec<String>| {
        if out.len() < MAX_HINTS && !seen.iter().any(|s| s == cand) {
            seen.push(cand.to_string());
            out.push(line);
        }
    };
    let mut unique: Vec<&str> = Vec::new();
    for v in visible {
        let v = v.trim();
        if !v.is_empty() && v != wanted && !unique.contains(&v) {
            unique.push(v);
        }
    }
    // 1. The selector's text inside a longer text.
    for cand in &unique {
        if cand.contains(wanted) {
            let pattern = if cand.starts_with(wanted) {
                format!("{wanted}*")
            } else if cand.ends_with(wanted) {
                format!("*{wanted}")
            } else {
                format!("*{wanted}*")
            };
            push(
                cand,
                format!("visible text {cand:?} contains it → try {pattern:?}"),
                &mut out,
            );
        }
    }
    // 2. The same text with other numbers.
    let shape = digits_as_one(wanted);
    if shape != wanted {
        for cand in &unique {
            if digits_as_one(cand) == shape {
                push(
                    cand,
                    format!(
                        "visible text {cand:?} differs only in numbers → try {:?}",
                        digits_as_star(wanted)
                    ),
                    &mut out,
                );
            }
        }
    }
    // 3. A similar text, most similar first.
    let mut similar: Vec<(f64, &str)> = unique
        .iter()
        .map(|c| {
            (
                strsim::normalized_damerau_levenshtein(&wanted.to_lowercase(), &c.to_lowercase()),
                *c,
            )
        })
        .filter(|(score, _)| *score >= SIMILAR)
        .collect();
    similar.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, cand) in similar {
        push(cand, format!("visible text {cand:?} is similar"), &mut out);
    }
    out
}

/// `text` with each run of digits as one `#`.
fn digits_as_one(text: &str) -> String {
    collapse_digits(text, "#")
}

/// `text` with each run of digits as `*`, a pattern that matches any number.
fn digits_as_star(text: &str) -> String {
    collapse_digits(text, "*")
}

fn collapse_digits(text: &str, with: &str) -> String {
    let mut out = String::new();
    let mut in_digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push_str(with);
            }
            in_digits = true;
        } else {
            out.push(c);
            in_digits = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_longer_text_holding_the_selector_suggests_a_pattern() {
        let h = hints("依頼", &["ホーム", "依頼, tab, 2 of 3"]);
        assert_eq!(
            h,
            [r#"visible text "依頼, tab, 2 of 3" contains it → try "依頼*""#]
        );
        let h = hints("Save", &["Click to Save"]);
        assert_eq!(
            h,
            [r#"visible text "Click to Save" contains it → try "*Save""#]
        );
        let h = hints("Save", &["Do Save now"]);
        assert_eq!(
            h,
            [r#"visible text "Do Save now" contains it → try "*Save*""#]
        );
    }

    #[test]
    fn other_numbers_suggest_a_pattern_for_any_number() {
        let h = hints("サービス600種類以上", &["サービス700種類以上"]);
        assert_eq!(
            h,
            [
                r#"visible text "サービス700種類以上" differs only in numbers → try "サービス*種類以上""#
            ]
        );
    }

    #[test]
    fn a_similar_text_is_named() {
        let h = hints("Sumbit", &["Submit", "Cancel"]);
        assert_eq!(h, [r#"visible text "Submit" is similar"#]);
        let h = hints("submit", &["Submit"]);
        assert_eq!(h, [r#"visible text "Submit" is similar"#]);
    }

    #[test]
    fn nothing_close_gives_no_hint() {
        assert!(hints("Checkout", &["Home", "Profile", "Settings"]).is_empty());
        assert!(hints("", &["Home"]).is_empty());
    }

    #[test]
    fn a_pattern_gets_no_hint() {
        assert!(hints("Sub*", &["Submit"]).is_empty());
    }

    #[test]
    fn at_most_three_hints_one_per_text() {
        let h = hints("Item", &["Item 1", "Item 2", "Item 3", "Item 4", "Item 1"]);
        assert_eq!(h.len(), 3);
        assert!(h[0].contains("Item 1"));
    }
}
