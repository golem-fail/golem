//! Script detection and text-validation primitives shared across the data
//! pools.
//!
//! Today these back the `names.json` data-validation tests; next they back the
//! per-script person resolver, and later the geo (address) data once it gains
//! `kana`/`ipa` fields. They are deliberately generic over `&str` with no
//! name- or geo-specific assumptions, so geo can reuse them unchanged.
//!
//! `detect_script` is total and never panics: bad data is caught by the
//! data-validation tests, so runtime code can trust the pools.
//!
//! `Script` / `classify_char` / `detect_script` are used in production (the
//! per-script person resolver); the text validators are currently only used by
//! the data-validation tests, so they are `#[cfg(test)]` until geo (or other
//! callers) need them at runtime.

use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;

use unicode_normalization::UnicodeNormalization;

/// Fold a string to a pure-ASCII form: map the handful of non-decomposable
/// Latin letters (ø ł æ ß œ …) via a fixed table, then NFD-decompose and drop
/// the combining diacritical marks (U+0300–U+036F). ASCII characters
/// (including `'` and `-`) pass through unchanged. Characters that do not
/// reduce to ASCII — CJK, Hangul, Cyrillic, Arabic, … — pass through as-is, so
/// the result is NOT guaranteed ASCII for non-Latin input; such names must
/// carry an explicit romanisation rather than relying on this fold.
pub(crate) fn ascii_fold(s: &str) -> String {
    let mut pre = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'ø' => pre.push('o'),
            'Ø' => pre.push('O'),
            'ł' => pre.push('l'),
            'Ł' => pre.push('L'),
            'đ' | 'ð' => pre.push('d'),
            'Đ' | 'Ð' => pre.push('D'),
            'ı' => pre.push('i'),
            'İ' => pre.push('I'),
            'æ' => pre.push_str("ae"),
            'Æ' => pre.push_str("Ae"),
            'œ' => pre.push_str("oe"),
            'Œ' => pre.push_str("Oe"),
            'ß' => pre.push_str("ss"),
            'þ' => pre.push_str("th"),
            'Þ' => pre.push_str("Th"),
            _ => pre.push(c),
        }
    }
    pre.nfd()
        .filter(|c| !matches!(*c as u32, 0x0300..=0x036F))
        .collect()
}

/// A writing system a string can belong to. Only scripts that appear (or are
/// expected) in the data pools are represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Script {
    Latin,
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Cyrillic,
    Hebrew,
    Arabic,
    Devanagari,
    Thai,
}

/// Classify a single character into its [`Script`], or `None` when it carries
/// no script identity: ASCII punctuation/space/digits, combining diacritics,
/// spacing modifiers, IPA-extension and Greek phonetic letters (θ/β/χ …), and
/// the shared kana marks ー (chōonpu) and ・ (nakaguro).
pub(crate) fn classify_char(c: char) -> Option<Script> {
    match c as u32 {
        // Latin: ASCII letters, Latin-1 letters (skipping × U+00D7 and ÷ U+00F7),
        // Latin Extended-A/B, and Latin Extended Additional (Vietnamese forms).
        0x41..=0x5A | 0x61..=0x7A => Some(Script::Latin),
        0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0xFF => Some(Script::Latin),
        0x100..=0x24F | 0x1E00..=0x1EFF => Some(Script::Latin),
        // Han (CJK Unified Ideographs + Extension A).
        0x3400..=0x4DBF | 0x4E00..=0x9FFF => Some(Script::Han),
        // Kana — excluding ・ (U+30FB) and ー (U+30FC), which are script-less marks.
        0x3041..=0x309F => Some(Script::Hiragana),
        0x30A1..=0x30FA | 0x30FD..=0x30FF => Some(Script::Katakana),
        // Other scripts present (or expected) in the pools.
        0xAC00..=0xD7A3 => Some(Script::Hangul),
        0x400..=0x4FF => Some(Script::Cyrillic),
        0x590..=0x5FF => Some(Script::Hebrew),
        0x600..=0x6FF => Some(Script::Arabic),
        0x900..=0x97F => Some(Script::Devanagari),
        0xE00..=0xE7F => Some(Script::Thai),
        _ => None,
    }
}

/// The set of scripts present in a string (script-less characters ignored).
#[cfg(test)]
pub(crate) fn script_set(s: &str) -> BTreeSet<Script> {
    s.chars().filter_map(classify_char).collect()
}

/// The dominant script of a string: the script covering the most characters,
/// ties broken by [`Script`] order for determinism. Falls back to `Latin` when
/// the string has no script-bearing characters. Total — never panics.
pub(crate) fn detect_script(s: &str) -> Script {
    let mut counts: BTreeMap<Script, usize> = BTreeMap::new();
    for sc in s.chars().filter_map(classify_char) {
        *counts.entry(sc).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(sc, _)| sc)
        .unwrap_or(Script::Latin)
}

/// True when every character is ASCII (letters, digits, and ASCII punctuation
/// such as the apostrophe/hyphen/space used in some romanisations).
#[cfg(test)]
pub(crate) fn is_ascii_text(s: &str) -> bool {
    s.is_ascii()
}

/// True when every character lies in the hiragana/katakana blocks (including
/// the chōonpu ー): a pure-kana reading with no stray script, latin, or
/// punctuation.
#[cfg(test)]
pub(crate) fn is_kana_text(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| matches!(c as u32, 0x3040..=0x30FF))
}

/// True when a string is free of natural-script contamination for use as an
/// IPA value: every character must be Latin or script-less (IPA-extension and
/// Greek θ/β/χ phonetic letters, combining diacritics, stress/length marks).
/// Catches e.g. Cyrillic homoglyphs (о/а/т) mistyped for Latin o/a/t.
#[cfg(test)]
pub(crate) fn ipa_is_clean(s: &str) -> bool {
    s.chars()
        .all(|c| matches!(classify_char(c), None | Some(Script::Latin)))
}

/// True when a `name` uses a single script, or the one legitimate multi-script
/// combination: a Japanese given name written as kanji + hiragana (e.g. ゆき子).
/// Kanji + katakana is NOT a real given-name form, and any other mix (e.g.
/// Latin + Cyrillic) indicates bad/contaminated data.
#[cfg(test)]
pub(crate) fn name_scripts_ok(s: &str) -> bool {
    let set = script_set(s);
    if set.len() <= 1 {
        return true;
    }
    let kanji_hiragana: BTreeSet<Script> = [Script::Han, Script::Hiragana].into_iter().collect();
    set.is_subset(&kanji_hiragana)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_script_picks_the_right_system() {
        assert_eq!(detect_script("Pierre"), Script::Latin);
        assert_eq!(detect_script("García"), Script::Latin); // accented latin
        assert_eq!(detect_script("太郎"), Script::Han);
        assert_eq!(detect_script("김민준"), Script::Hangul);
        assert_eq!(detect_script("Иванов"), Script::Cyrillic);
        assert_eq!(detect_script("محمد"), Script::Arabic);
        assert_eq!(detect_script("כהן"), Script::Hebrew);
        assert_eq!(detect_script("さくら"), Script::Hiragana);
        assert_eq!(detect_script("ジャン"), Script::Katakana);
    }

    #[test]
    fn detect_script_is_total_on_scriptless_input() {
        assert_eq!(detect_script(""), Script::Latin);
        assert_eq!(detect_script("123 -'"), Script::Latin);
    }

    #[test]
    fn name_scripts_ok_allows_japanese_mix_rejects_contamination() {
        assert!(name_scripts_ok("ゆき子")); // hiragana + kanji — legitimate given name
        assert!(name_scripts_ok("Pierre"));
        assert!(name_scripts_ok("太郎"));
        // Kanji + katakana is NOT a real given-name form.
        assert!(!name_scripts_ok("トシ子"));
        // Latin string with a Cyrillic homoglyph 'е' — contamination.
        assert!(!name_scripts_ok("Piеrre"));
        // Han + Hangul is not a legitimate combination.
        assert!(!name_scripts_ok("王김"));
    }

    #[test]
    fn ipa_is_clean_allows_ipa_rejects_cyrillic() {
        assert!(ipa_is_clean("ʒɑ̃")); // nasal vowel + combining
        assert!(ipa_is_clean("ɡaɾˈθia")); // IPA-ext letters + Greek θ + stress
        assert!(ipa_is_clean("ˈmʏlɐ"));
        assert!(!ipa_is_clean("jamamoто")); // Cyrillic т/о homoglyphs
    }

    #[test]
    fn ascii_fold_derives_latin_keeps_ascii_passes_through_nonlatin() {
        // NFD-decomposable accents fold to their base letter.
        assert_eq!(ascii_fold("Woźniak"), "Wozniak");
        assert_eq!(ascii_fold("André"), "Andre");
        assert_eq!(ascii_fold("Ngāhuia"), "Ngahuia");
        assert_eq!(ascii_fold("Wałęsa"), "Walesa");
        // Non-decomposable letters fold via the fixed table.
        assert_eq!(ascii_fold("Søren"), "Soren");
        assert_eq!(ascii_fold("Łukasz"), "Lukasz");
        assert_eq!(ascii_fold("Lætitia"), "Laetitia");
        assert_eq!(ascii_fold("Strauß"), "Strauss");
        assert_eq!(ascii_fold("Lecœur"), "Lecoeur");
        // ASCII passes through verbatim — apostrophes and hyphens are kept.
        assert_eq!(ascii_fold("O'Brien"), "O'Brien");
        assert_eq!(ascii_fold("Jean-Pierre"), "Jean-Pierre");
        // Non-Latin scripts don't reduce to ASCII (caller must store a
        // romanisation); the fold leaves them unchanged.
        assert_eq!(ascii_fold("太郎"), "太郎");
        assert!(!ascii_fold("김").is_ascii());
    }

    #[test]
    fn kana_and_ascii_text_predicates() {
        assert!(is_kana_text("ゆきこ"));
        assert!(is_kana_text("ジャン"));
        assert!(is_kana_text("ピエール")); // includes ー
        assert!(!is_kana_text("Jean"));
        assert!(!is_kana_text(""));
        assert!(is_ascii_text("O'Brien"));
        assert!(is_ascii_text("De Luca"));
        assert!(!is_ascii_text("García"));
    }
}
