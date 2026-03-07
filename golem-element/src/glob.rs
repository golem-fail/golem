/// A compiled glob pattern for matching element text.
///
/// Supports:
/// - `*` matches zero or more of any character
/// - `?` matches exactly one character
/// - `\*` escaped literal asterisk
/// - `\?` escaped literal question mark
/// - All other characters are literal (case-sensitive)
#[derive(Debug, Clone)]
pub struct GlobMatcher {
    /// Compiled segments of the pattern.
    segments: Vec<Segment>,
}

/// A single segment of a compiled glob pattern.
#[derive(Debug, Clone)]
enum Segment {
    /// Match zero or more of any character.
    Star,
    /// Match exactly one character.
    Question,
    /// Match a literal string.
    Literal(String),
}

impl GlobMatcher {
    /// Compile a glob pattern string into a matcher.
    pub fn new(pattern: &str) -> Self {
        let segments = compile(pattern);
        Self { segments }
    }

    /// Test if a string matches this pattern.
    pub fn is_match(&self, text: &str) -> bool {
        matches_segments(&self.segments, text)
    }
}

/// Convenience function to compile and test in one call.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    GlobMatcher::new(pattern).is_match(text)
}

/// Parse a glob pattern string into a list of segments.
fn compile(pattern: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut literal_buf = String::new();
    let mut chars = pattern.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                // Escaped character: next char is literal
                if let Some(next) = chars.next() {
                    literal_buf.push(next);
                } else {
                    // Trailing backslash is kept as literal
                    literal_buf.push('\\');
                }
            }
            '*' => {
                // Flush any accumulated literal
                if !literal_buf.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal_buf)));
                }
                segments.push(Segment::Star);
            }
            '?' => {
                // Flush any accumulated literal
                if !literal_buf.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal_buf)));
                }
                segments.push(Segment::Question);
            }
            _ => {
                literal_buf.push(ch);
            }
        }
    }

    // Flush remaining literal
    if !literal_buf.is_empty() {
        segments.push(Segment::Literal(literal_buf));
    }

    segments
}

/// Recursively match a slice of segments against a text string.
fn matches_segments(segments: &[Segment], text: &str) -> bool {
    match segments.first() {
        None => {
            // No more segments: text must also be empty
            text.is_empty()
        }
        Some(Segment::Literal(lit)) => {
            // Text must start with the literal
            if let Some(rest) = text.strip_prefix(lit.as_str()) {
                matches_segments(&segments[1..], rest)
            } else {
                false
            }
        }
        Some(Segment::Question) => {
            // Must match exactly one character
            let mut chars = text.chars();
            if chars.next().is_some() {
                matches_segments(&segments[1..], chars.as_str())
            } else {
                false
            }
        }
        Some(Segment::Star) => {
            // Try matching zero characters, then one, then two, etc.
            // Iterate over all possible split points (by character boundary).
            let remaining = &segments[1..];
            // Start by trying to consume zero characters
            if matches_segments(remaining, text) {
                return true;
            }
            // Then try consuming one character at a time
            for (i, _) in text.char_indices() {
                // Try matching after consuming up to (and including) this character
                let after = &text[i..];
                // Skip the first character at position `i`
                let mut chars = after.chars();
                chars.next(); // consume one char
                let rest = chars.as_str();
                if matches_segments(remaining, rest) {
                    return true;
                }
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1. Wildcard -- matches anything
    #[test]
    fn wildcard_matches_anything() {
        assert!(glob_match("Item *", "Item 1"));
        assert!(glob_match("Item *", "Item 100 units"));
        assert!(!glob_match("Item *", "Item")); // space required after "Item"
        assert!(!glob_match("Item *", "Other"));
    }

    // 2. Wildcard -- leading
    #[test]
    fn wildcard_leading() {
        assert!(glob_match("* error", "connection error"));
        assert!(glob_match("* error", "timeout error"));
        assert!(!glob_match("* error", "error")); // no leading chars + space
        assert!(!glob_match("* error", "errors"));
    }

    // 3. Wildcard -- middle
    #[test]
    fn wildcard_middle() {
        assert!(glob_match("Order #*!", "Order #123!"));
        assert!(glob_match("Order #*!", "Order #!"));
        assert!(glob_match("Order #*!", "Order #456 pending!"));
    }

    // 4. Wildcard -- multiple
    #[test]
    fn wildcard_multiple() {
        assert!(glob_match("*hello*", "hello"));
        assert!(glob_match("*hello*", "say hello world"));
        assert!(glob_match("*hello*", "hello!"));
        assert!(!glob_match("*hello*", "hey"));
    }

    // 5. Single character -- ?
    #[test]
    fn single_char_question_mark() {
        assert!(glob_match("Tab ?", "Tab 1"));
        assert!(glob_match("Tab ?", "Tab A"));
        assert!(!glob_match("Tab ?", "Tab 10"));
        assert!(!glob_match("Tab ?", "Tab "));
    }

    // 6. Single character -- multiple ?
    #[test]
    fn multiple_question_marks() {
        assert!(glob_match("??-???", "AB-CDE"));
        assert!(!glob_match("??-???", "A-BCD"));
        assert!(!glob_match("??-???", "AB-CD"));
        assert!(!glob_match("??-???", "ABC-DEF"));
    }

    // 7. Exact match -- no wildcards
    #[test]
    fn exact_match_no_wildcards() {
        assert!(glob_match("Submit", "Submit"));
        assert!(!glob_match("Submit", "submit"));
        assert!(!glob_match("Submit", "Submit Order"));
        assert!(!glob_match("Submit", " Submit"));
    }

    // 8. Escaped asterisk
    #[test]
    fn escaped_asterisk() {
        assert!(glob_match("5 \\* 3", "5 * 3"));
        assert!(!glob_match("5 \\* 3", "5 x 3"));
        assert!(!glob_match("5 \\* 3", "5  3"));
    }

    // 9. Escaped question mark
    #[test]
    fn escaped_question_mark() {
        assert!(glob_match("Really\\?", "Really?"));
        assert!(!glob_match("Really\\?", "Reallyx"));
        assert!(!glob_match("Really\\?", "Really"));
    }

    // 10. Empty pattern
    #[test]
    fn empty_pattern() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "something"));
    }

    // 11. Wildcard only
    #[test]
    fn wildcard_only() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", "hello world"));
    }

    // 12. Question mark only
    #[test]
    fn question_mark_only() {
        assert!(!glob_match("?", ""));
        assert!(glob_match("?", "a"));
        assert!(!glob_match("?", "ab"));
    }

    // 13. Special regex characters are not interpreted
    #[test]
    fn special_regex_chars_literal() {
        assert!(glob_match("price: $10.00", "price: $10.00"));
        assert!(!glob_match("price: $10.00", "price: $10x00"));
        assert!(!glob_match("price: $10.00", "price: $10.00 USD"));
    }

    // 14. Case sensitivity
    #[test]
    fn case_sensitive() {
        assert!(glob_match("Submit*", "Submit"));
        assert!(glob_match("Submit*", "Submit Order"));
        assert!(!glob_match("Submit*", "submit"));
        assert!(!glob_match("Submit*", "SUBMIT"));
    }

    // 15. Unicode text
    #[test]
    fn unicode_text() {
        assert!(glob_match("送信*", "送信"));
        assert!(glob_match("送信*", "送信ボタン"));
        assert!(!glob_match("送信*", "キャンセル"));
    }
}
