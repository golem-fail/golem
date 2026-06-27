//! Sentence data loader — embeds and indexes all `data/sentences/*.json` files
//! at compile time, exposing them through a single `SentenceDatabase` singleton.
//!
//! Each file is one language (keyed by its ISO 639-1 code, or `lorem` for the
//! language-neutral default). Adding a language only requires a new JSON file
//! and one `include_str!` line in `RAW_ENTRIES`; the generator picks it up
//! automatically. See `generators.rs::generate_sentence` for the consumer.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Compile-time data loading
// ---------------------------------------------------------------------------

/// `(language_key, json_str)` for every embedded sentence file.
const RAW_ENTRIES: &[(&str, &str)] = &[
    ("lorem", include_str!("../../data/sentences/lorem.json")),
    ("af", include_str!("../../data/sentences/af.json")),
    ("ak", include_str!("../../data/sentences/ak.json")),
    ("am", include_str!("../../data/sentences/am.json")),
    ("ar", include_str!("../../data/sentences/ar.json")),
    ("as", include_str!("../../data/sentences/as.json")),
    ("ay", include_str!("../../data/sentences/ay.json")),
    ("az", include_str!("../../data/sentences/az.json")),
    ("be", include_str!("../../data/sentences/be.json")),
    ("bg", include_str!("../../data/sentences/bg.json")),
    ("bm", include_str!("../../data/sentences/bm.json")),
    ("bn", include_str!("../../data/sentences/bn.json")),
    ("bo", include_str!("../../data/sentences/bo.json")),
    ("bs", include_str!("../../data/sentences/bs.json")),
    ("ca", include_str!("../../data/sentences/ca.json")),
    ("cs", include_str!("../../data/sentences/cs.json")),
    ("da", include_str!("../../data/sentences/da.json")),
    ("de", include_str!("../../data/sentences/de.json")),
    ("ee", include_str!("../../data/sentences/ee.json")),
    ("el", include_str!("../../data/sentences/el.json")),
    ("en", include_str!("../../data/sentences/en.json")),
    ("es", include_str!("../../data/sentences/es.json")),
    ("fa", include_str!("../../data/sentences/fa.json")),
    ("ff", include_str!("../../data/sentences/ff.json")),
    ("fi", include_str!("../../data/sentences/fi.json")),
    ("fr", include_str!("../../data/sentences/fr.json")),
    ("gl", include_str!("../../data/sentences/gl.json")),
    ("gn", include_str!("../../data/sentences/gn.json")),
    ("gu", include_str!("../../data/sentences/gu.json")),
    ("ha", include_str!("../../data/sentences/ha.json")),
    ("he", include_str!("../../data/sentences/he.json")),
    ("hi", include_str!("../../data/sentences/hi.json")),
    ("hr", include_str!("../../data/sentences/hr.json")),
    ("ht", include_str!("../../data/sentences/ht.json")),
    ("hu", include_str!("../../data/sentences/hu.json")),
    ("hy", include_str!("../../data/sentences/hy.json")),
    ("id", include_str!("../../data/sentences/id.json")),
    ("ig", include_str!("../../data/sentences/ig.json")),
    ("ii", include_str!("../../data/sentences/ii.json")),
    ("it", include_str!("../../data/sentences/it.json")),
    ("ja", include_str!("../../data/sentences/ja.json")),
    ("jv", include_str!("../../data/sentences/jv.json")),
    ("ka", include_str!("../../data/sentences/ka.json")),
    ("kg", include_str!("../../data/sentences/kg.json")),
    ("ki", include_str!("../../data/sentences/ki.json")),
    ("kk", include_str!("../../data/sentences/kk.json")),
    ("km", include_str!("../../data/sentences/km.json")),
    ("kn", include_str!("../../data/sentences/kn.json")),
    ("ko", include_str!("../../data/sentences/ko.json")),
    ("kr", include_str!("../../data/sentences/kr.json")),
    ("ks", include_str!("../../data/sentences/ks.json")),
    ("ku", include_str!("../../data/sentences/ku.json")),
    ("ky", include_str!("../../data/sentences/ky.json")),
    ("lg", include_str!("../../data/sentences/lg.json")),
    ("ln", include_str!("../../data/sentences/ln.json")),
    ("lo", include_str!("../../data/sentences/lo.json")),
    ("lt", include_str!("../../data/sentences/lt.json")),
    ("mg", include_str!("../../data/sentences/mg.json")),
    ("mk", include_str!("../../data/sentences/mk.json")),
    ("ml", include_str!("../../data/sentences/ml.json")),
    ("mn", include_str!("../../data/sentences/mn.json")),
    ("mr", include_str!("../../data/sentences/mr.json")),
    ("ms", include_str!("../../data/sentences/ms.json")),
    ("my", include_str!("../../data/sentences/my.json")),
    ("ne", include_str!("../../data/sentences/ne.json")),
    ("nl", include_str!("../../data/sentences/nl.json")),
    ("no", include_str!("../../data/sentences/no.json")),
    ("ny", include_str!("../../data/sentences/ny.json")),
    ("om", include_str!("../../data/sentences/om.json")),
    ("or", include_str!("../../data/sentences/or.json")),
    ("pa", include_str!("../../data/sentences/pa.json")),
    ("pl", include_str!("../../data/sentences/pl.json")),
    ("ps", include_str!("../../data/sentences/ps.json")),
    ("pt", include_str!("../../data/sentences/pt.json")),
    ("qu", include_str!("../../data/sentences/qu.json")),
    ("rn", include_str!("../../data/sentences/rn.json")),
    ("ro", include_str!("../../data/sentences/ro.json")),
    ("ru", include_str!("../../data/sentences/ru.json")),
    ("rw", include_str!("../../data/sentences/rw.json")),
    ("sd", include_str!("../../data/sentences/sd.json")),
    ("sg", include_str!("../../data/sentences/sg.json")),
    ("si", include_str!("../../data/sentences/si.json")),
    ("sk", include_str!("../../data/sentences/sk.json")),
    ("sl", include_str!("../../data/sentences/sl.json")),
    ("sn", include_str!("../../data/sentences/sn.json")),
    ("so", include_str!("../../data/sentences/so.json")),
    ("sq", include_str!("../../data/sentences/sq.json")),
    ("sr", include_str!("../../data/sentences/sr.json")),
    ("ss", include_str!("../../data/sentences/ss.json")),
    ("st", include_str!("../../data/sentences/st.json")),
    ("su", include_str!("../../data/sentences/su.json")),
    ("sv", include_str!("../../data/sentences/sv.json")),
    ("sw", include_str!("../../data/sentences/sw.json")),
    ("ta", include_str!("../../data/sentences/ta.json")),
    ("te", include_str!("../../data/sentences/te.json")),
    ("tg", include_str!("../../data/sentences/tg.json")),
    ("th", include_str!("../../data/sentences/th.json")),
    ("ti", include_str!("../../data/sentences/ti.json")),
    ("tk", include_str!("../../data/sentences/tk.json")),
    ("tl", include_str!("../../data/sentences/tl.json")),
    ("tn", include_str!("../../data/sentences/tn.json")),
    ("tr", include_str!("../../data/sentences/tr.json")),
    ("ts", include_str!("../../data/sentences/ts.json")),
    ("tt", include_str!("../../data/sentences/tt.json")),
    ("ug", include_str!("../../data/sentences/ug.json")),
    ("uk", include_str!("../../data/sentences/uk.json")),
    ("ur", include_str!("../../data/sentences/ur.json")),
    ("uz", include_str!("../../data/sentences/uz.json")),
    ("vi", include_str!("../../data/sentences/vi.json")),
    ("wo", include_str!("../../data/sentences/wo.json")),
    ("xh", include_str!("../../data/sentences/xh.json")),
    ("yo", include_str!("../../data/sentences/yo.json")),
    ("za", include_str!("../../data/sentences/za.json")),
    ("zh", include_str!("../../data/sentences/zh.json")),
    ("zu", include_str!("../../data/sentences/zu.json")),
];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Sentence templates and word lists for one language.
///
/// A pattern carries the literal connective text (spaces, particles,
/// punctuation, leading article) plus `{slot}` placeholders; each placeholder
/// is filled from `slots[name]`. All script/joining/RTL behaviour is therefore
/// expressed in the data, not in code.
#[derive(Deserialize)]
pub(crate) struct SentenceData {
    pub(crate) patterns: Vec<String>,
    pub(crate) slots: HashMap<String, Vec<String>>,
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// Parsed `SentenceData` keyed by language (`"lorem"`, `"en"`, `"fr"`, …).
pub(crate) struct SentenceDatabase {
    map: HashMap<String, SentenceData>,
}

impl SentenceDatabase {
    fn new() -> Self {
        let mut map = HashMap::new();
        for (lang, json) in RAW_ENTRIES {
            let data: SentenceData = serde_json::from_str(json).unwrap_or_else(|e| {
                panic!("failed to parse sentence data for {lang}: {e}");
            });
            map.insert((*lang).to_string(), data);
        }
        Self { map }
    }

    /// Look up a language by key (lower-cased, e.g. `"fr"`).
    pub(crate) fn get(&self, lang: &str) -> Option<&SentenceData> {
        self.map.get(&lang.to_lowercase())
    }

    /// All loaded language keys (test-only — used by the data-validation test).
    #[cfg(test)]
    pub(crate) fn languages(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self.map.keys().map(|s| s.as_str()).collect();
        keys.sort();
        keys
    }
}

/// Returns the global, lazily-initialised `SentenceDatabase`.
pub(crate) fn sentence_database() -> &'static SentenceDatabase {
    static INSTANCE: OnceLock<SentenceDatabase> = OnceLock::new();
    INSTANCE.get_or_init(SentenceDatabase::new)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Floor on distinct sentences per language (the project standard).
    const MIN_DISTINCT: u64 = 20_000;

    // Every loaded language has at least one pattern, every `{slot}` referenced
    // by any pattern resolves to a non-empty word list (so the generator never
    // indexes into an empty slice or hits a missing slot), and each language
    // yields at least `MIN_DISTINCT` distinct sentences.
    #[test]
    fn every_language_has_patterns_resolvable_slots_and_enough_variety() {
        let db = sentence_database();
        for lang in db.languages() {
            let data = db.get(lang).expect("language SHALL load");
            assert!(!data.patterns.is_empty(), "{lang}: no patterns");

            // distinct = Σ over patterns of (∏ sizes of referenced slot lists).
            let mut distinct: u64 = 0;
            for pattern in &data.patterns {
                let mut combos: u64 = 1;
                for slot in slot_names(pattern) {
                    let words = data.slots.get(&slot).unwrap_or_else(|| {
                        panic!("{lang}: pattern references unknown slot {{{slot}}}")
                    });
                    assert!(!words.is_empty(), "{lang}: slot {{{slot}}} has no words");
                    combos = combos.saturating_mul(words.len() as u64);
                }
                distinct = distinct.saturating_add(combos);
            }
            assert!(
                distinct >= MIN_DISTINCT,
                "{lang}: only {distinct} distinct sentences (< {MIN_DISTINCT})"
            );
        }
    }

    /// Extract the `{slot}` names referenced in a pattern.
    fn slot_names(pattern: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = pattern;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            names.push(rest[open + 1..open + close].to_string());
            rest = &rest[open + close + 1..];
        }
        names
    }
}
