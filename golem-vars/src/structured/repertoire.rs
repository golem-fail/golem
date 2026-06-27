//! Named character repertoires and the country-aware `local` representation.
//!
//! A *repertoire* is a named, concrete character set — `kanji`, `hiragana`,
//! `diacritics_fr`, … — NOT a raw Unicode script. The distinction matters: the
//! `kanji` repertoire is JIS X 0208 (the kanji legitimately used in Japanese
//! text), which is *narrower* than the Han block, so a simplified-Chinese-only
//! character such as 张 is rejected even though it is Han.
//!
//! [`local`] keeps a name in its native script iff every character is accepted
//! by the **union** of a country's repertoires; otherwise it yields `""` so a
//! fallback chain (e.g. `[local, ascii]`) falls through to the romanised form.
//! An empty repertoire list means "accept everything" — `local` == native.
//!
//! These are `&str`-generic with no person-specific assumptions, so the geo
//! (address) data can reuse them unchanged.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::script::{classify_char, Script};

/// The chōonpu (long-vowel mark) — script-less in [`classify_char`] but a
/// legitimate member of both kana repertoires.
const CHOONPU: char = 'ー';

/// One named character repertoire (a concrete character set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Repertoire {
    Ascii,
    /// JIS X 0208 kanji — narrower than Han (rejects simplified-Chinese chars).
    Kanji,
    Hiragana,
    Katakana,
    Hangul,
    /// The full Han block — for Chinese (`hanzi`) and Korean (`hanja`) names.
    Han,
    Cyrillic,
    Hebrew,
    Arabic,
    Devanagari,
    Thai,
    /// A language's Latin diacritic set (the accented letters only, no ASCII),
    /// keyed by ISO 639-1 code (`"fr"`, `"de"`, …).
    Diacritics(&'static str),
}

impl Repertoire {
    /// Parse a repertoire token (`"ascii"`, `"kanji"`, `"diacritics_fr"`, …).
    /// `"hanzi"` and `"hanja"` are both spellings of the Han block.
    pub(crate) fn parse(token: &str) -> Option<Self> {
        Some(match token.trim() {
            "ascii" => Self::Ascii,
            "kanji" => Self::Kanji,
            "hiragana" => Self::Hiragana,
            "katakana" => Self::Katakana,
            "hangul" => Self::Hangul,
            "hanzi" | "hanja" => Self::Han,
            "cyrillic" => Self::Cyrillic,
            "hebrew" => Self::Hebrew,
            "arabic" => Self::Arabic,
            "devanagari" => Self::Devanagari,
            "thai" => Self::Thai,
            other => {
                let lang = other.strip_prefix("diacritics_")?;
                Self::Diacritics(intern_lang(lang)?)
            }
        })
    }

    /// True when `c` is a member of this repertoire.
    fn accepts(self, c: char) -> bool {
        match self {
            Self::Ascii => c.is_ascii(),
            Self::Kanji => kanji_set().contains(&c),
            Self::Hiragana => matches!(classify_char(c), Some(Script::Hiragana)) || c == CHOONPU,
            Self::Katakana => matches!(classify_char(c), Some(Script::Katakana)) || c == CHOONPU,
            Self::Hangul => matches!(classify_char(c), Some(Script::Hangul)),
            Self::Han => matches!(classify_char(c), Some(Script::Han)),
            Self::Cyrillic => matches!(classify_char(c), Some(Script::Cyrillic)),
            Self::Hebrew => matches!(classify_char(c), Some(Script::Hebrew)),
            Self::Arabic => matches!(classify_char(c), Some(Script::Arabic)),
            Self::Devanagari => matches!(classify_char(c), Some(Script::Devanagari)),
            Self::Thai => matches!(classify_char(c), Some(Script::Thai)),
            Self::Diacritics(lang) => diacritic_set(lang).is_some_and(|s| s.contains(&c)),
        }
    }
}

/// The `local` representation: returns `name` unchanged iff **every** character
/// is accepted by the union of `repertoires`, otherwise `""`. An empty
/// `repertoires` slice accepts everything (`local` == native).
pub(crate) fn local(name: &str, repertoires: &[Repertoire]) -> String {
    if repertoires.is_empty() {
        return name.to_string();
    }
    let ok = name
        .chars()
        .all(|c| repertoires.iter().any(|r| r.accepts(c)));
    if ok {
        name.to_string()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// Kanji repertoire — JIS X 0208, derived exactly from the EUC-JP code table.
// ---------------------------------------------------------------------------

/// The JIS X 0208 kanji set. Derived once from the EUC-JP encoding rather than
/// embedded as a literal list. The kanji occupy ku (rows) 16–84, i.e. EUC lead
/// bytes `0xB0..=0xF4`; each two-byte sequence there is decoded and the
/// Han-script results collected. Restricting to those rows yields exactly the
/// 6 355 level-1+2 kanji and excludes the IBM/NEC extension rows (89+) that the
/// WHATWG EUC-JP index also carries. This is a superset of the jōyō/jinmeiyō
/// name kanji that never wrongly rejects a real Japanese character, yet still
/// excludes simplified-Chinese-only forms.
fn kanji_set() -> &'static HashSet<char> {
    static SET: OnceLock<HashSet<char>> = OnceLock::new();
    SET.get_or_init(|| {
        let euc = encoding_rs::EUC_JP;
        let mut set = HashSet::new();
        for b1 in 0xB0u8..=0xF4 {
            for b2 in 0xA1u8..=0xFE {
                let bytes = [b1, b2];
                let (decoded, had_errors) = euc.decode_without_bom_handling(&bytes);
                if had_errors {
                    continue;
                }
                let mut chars = decoded.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    if matches!(classify_char(c), Some(Script::Han)) {
                        set.insert(c);
                    }
                }
            }
        }
        set
    })
}

// ---------------------------------------------------------------------------
// Latin diacritic sets — per-language accented-letter repertoires.
// ---------------------------------------------------------------------------

/// The lowercase accented letters of each language. Uppercase variants are
/// derived automatically in [`diacritic_set`], so each table is written once.
/// Defined independently per language (no containment de-duplication): the
/// union in [`local`] handles composition and multi-language countries.
fn diacritic_letters(lang: &str) -> &'static str {
    match lang {
        "de" => "äöüß",
        "fr" => "àâæçéèêëîïôœùûüÿ",
        "es" => "áéíóúüñ",
        "pt" => "ãõáàâçéêíóôú",
        "it" => "àèéìíîòóùú",
        "sv" => "åäö",
        "ga" => "áéíóú",
        "mi" => "āēīōū",
        "pl" => "ąćęłńóśźż",
        "lt" => "ąčęėįšųūž",
        "nl" => "ëïéèöü",
        _ => "",
    }
}

/// Every language that has a defined diacritic set. The single source of truth
/// for which `diacritics_<lang>` tokens are valid.
const DIACRITIC_LANGS: &[&str] = &[
    "de", "fr", "es", "pt", "it", "sv", "ga", "mi", "pl", "lt", "nl",
];

/// All language diacritic sets (lower + upper case), built once.
fn diacritic_sets() -> &'static HashMap<&'static str, HashSet<char>> {
    static SETS: OnceLock<HashMap<&'static str, HashSet<char>>> = OnceLock::new();
    SETS.get_or_init(|| {
        DIACRITIC_LANGS
            .iter()
            .map(|&lang| {
                let mut set = HashSet::new();
                for c in diacritic_letters(lang).chars() {
                    set.insert(c);
                    set.extend(c.to_uppercase());
                }
                (lang, set)
            })
            .collect()
    })
}

/// The accented-letter set for a language, or `None` for an unknown code.
fn diacritic_set(lang: &str) -> Option<&'static HashSet<char>> {
    diacritic_sets().get(lang)
}

/// Map a language code to its `'static` spelling, so [`Repertoire::Diacritics`]
/// can hold a `&'static str` without leaking caller input.
fn intern_lang(lang: &str) -> Option<&'static str> {
    DIACRITIC_LANGS.iter().copied().find(|&l| l == lang)
}

/// Every lowercase accented letter across all language diacritic sets — used by
/// the name-pool coverage test to assert each diacritic is exercised by data.
#[cfg(test)]
pub(crate) fn all_diacritic_chars() -> Vec<char> {
    let mut chars: Vec<char> = DIACRITIC_LANGS
        .iter()
        .flat_map(|l| diacritic_letters(l).chars())
        .collect();
    chars.sort_unstable();
    chars.dedup();
    chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kanji_set_is_jis_x_0208_sized() {
        // JIS X 0208 defines 6 355 level-1+2 kanji; the derivation SHALL recover
        // exactly that count (a drift would mean an encoding-table change).
        assert_eq!(kanji_set().len(), 6355, "JIS X 0208 kanji count");
    }

    #[test]
    fn kanji_accepts_japanese_rejects_simplified_chinese() {
        // Common Japanese name kanji are present.
        assert!(Repertoire::Kanji.accepts('太'));
        assert!(Repertoire::Kanji.accepts('郎'));
        assert!(Repertoire::Kanji.accepts('子'));
        // Simplified-Chinese-only forms are NOT in JIS X 0208.
        assert!(!Repertoire::Kanji.accepts('张')); // simplified 張
        assert!(!Repertoire::Kanji.accepts('伟')); // simplified 偉
                                                   // …but the Han repertoire (hanzi/hanja) does accept them.
        assert!(Repertoire::Han.accepts('张'));
    }

    #[test]
    fn local_keeps_native_when_all_chars_accepted() {
        // Pure kanji + the mixed kanji/hiragana given name both fit [kanji,hira].
        let jp = [Repertoire::Kanji, Repertoire::Hiragana];
        assert_eq!(local("太郎", &jp), "太郎");
        assert_eq!(local("ゆき子", &jp), "ゆき子");
    }

    #[test]
    fn local_empties_when_any_char_rejected() {
        let jp = [Repertoire::Kanji, Repertoire::Hiragana];
        // A foreign Latin name has no kanji/hiragana → empty (falls through).
        assert_eq!(local("Pierre", &jp), "");
        // A simplified-Chinese name → empty on a JP form.
        assert_eq!(local("张伟", &jp), "");
    }

    #[test]
    fn local_empty_repertoire_accepts_everything() {
        assert_eq!(local("anything 太郎 ☃", &[]), "anything 太郎 ☃");
    }

    #[test]
    fn diacritics_are_language_specific() {
        let de = [Repertoire::Ascii, Repertoire::Diacritics("de")];
        let es = [Repertoire::Ascii, Repertoire::Diacritics("es")];
        // German keeps Müller (ü ∈ de) but folds André (é ∉ de) to empty.
        assert_eq!(local("Müller", &de), "Müller");
        assert_eq!(local("André", &de), "");
        // Spanish keeps the accented ó/í that French/German lack.
        assert_eq!(local("Asunción", &es), "Asunción");
    }

    #[test]
    fn ascii_repertoire_accepts_name_punctuation() {
        let en = [Repertoire::Ascii];
        assert_eq!(local("O'Brien-Smith", &en), "O'Brien-Smith");
        assert_eq!(local("De Luca", &en), "De Luca");
        assert_eq!(local("García", &en), ""); // í ∉ ascii
    }

    #[test]
    fn parse_round_trips_known_tokens() {
        assert_eq!(Repertoire::parse("ascii"), Some(Repertoire::Ascii));
        assert_eq!(Repertoire::parse("kanji"), Some(Repertoire::Kanji));
        assert_eq!(Repertoire::parse("hanzi"), Some(Repertoire::Han));
        assert_eq!(Repertoire::parse("hanja"), Some(Repertoire::Han));
        assert_eq!(
            Repertoire::parse(" diacritics_fr "),
            Some(Repertoire::Diacritics("fr"))
        );
        assert_eq!(Repertoire::parse("klingon"), None);
        assert_eq!(Repertoire::parse("diacritics_zz"), None);
    }

    #[test]
    fn katakana_and_hiragana_admit_choonpu() {
        assert!(Repertoire::Katakana.accepts(CHOONPU));
        assert!(Repertoire::Hiragana.accepts(CHOONPU));
        assert!(Repertoire::Katakana.accepts('ピ'));
        assert!(!Repertoire::Hiragana.accepts('ピ')); // katakana ∉ hiragana
    }
}
