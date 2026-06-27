//! Hiragana → katakana folding.
//!
//! The stored `kana` reading is a natural mix (hiragana for native parts,
//! katakana for foreign parts). `to_katakana` folds it to katakana — the exact,
//! lossless direction, which is what furigana fields overwhelmingly want. The
//! reverse (katakana→hiragana) is deliberately not provided: the `hiragana`
//! representation is the stored reading *iff* it is already hiragana, so the
//! lossy chōonpu-expansion fold is never needed. Shared and `&str`-generic, so
//! the geo address `kana` can reuse it later.

/// Fold any hiragana in `s` to katakana. Katakana, the chōonpu ー, and any
/// non-kana characters pass through unchanged.
pub(crate) fn to_katakana(s: &str) -> String {
    s.chars()
        .map(|c| match c as u32 {
            o @ 0x3041..=0x3096 => char::from_u32(o + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katakana_fold_converts_hiragana_keeps_katakana() {
        assert_eq!(to_katakana("ゆきこ"), "ユキコ");
        assert_eq!(to_katakana("ピエール"), "ピエール"); // already katakana, incl. ー
        assert_eq!(to_katakana("たろう"), "タロウ");
    }
}
