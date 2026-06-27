use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::Result;
use rand::Rng;
use serde::Deserialize;

use crate::geo_loader::geo_database;
use crate::kana::to_katakana;
use crate::script::{classify_char, detect_script, Script};
use crate::structured::repertoire::{local, Repertoire};
use crate::transcribe::transcribe;
use crate::VarValue;

// ---------------------------------------------------------------------------
// Compile-time data loading
// ---------------------------------------------------------------------------

static NAMES_JSON: &str = include_str!("../../../data/names.json");

// ---------------------------------------------------------------------------
// Data models for deserialisation
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct NamesData {
    given_names: Vec<NameEntry>,
    family_names: Vec<NameEntry>,
}

#[derive(Deserialize, Clone)]
struct NameEntry {
    name: String,
    // Romanised, pure-ASCII form. Stored only when it differs from `name`;
    // omitted (and derived as `name`) when the name is already ASCII. Required
    // for non-Latin names, where it is a genuine romanisation, not derivable.
    #[serde(default)]
    ascii: String,
    kana: String,
    // Broad phonemic IPA — the source for the ipa->script engines (hangul,
    // cyrillic, hebrew, arabic). Required by the schema; held to the charset
    // invariant by the data tests below.
    ipa: String,
    // Korean Hanja, where it exists. Empty for the (many) pure-Hangul names and
    // for every non-Korean entry. Not derivable, so stored; defaults to empty
    // until the data pass populates it.
    #[serde(default)]
    hanja: String,
}

impl NameEntry {
    /// The ASCII form: the stored `ascii` when present, otherwise the ASCII
    /// fold of the name. Latin names (with or without diacritics) fold cleanly,
    /// so they omit `ascii`; non-Latin names can't be folded and must store it.
    fn ascii_form(&self) -> String {
        if self.ascii.is_empty() {
            crate::script::ascii_fold(&self.name)
        } else {
            self.ascii.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Lazy-parsed singletons
// ---------------------------------------------------------------------------

fn names_data() -> &'static NamesData {
    static INSTANCE: OnceLock<NamesData> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        serde_json::from_str(NAMES_JSON).expect("data/names.json should be valid JSON")
    })
}

// ---------------------------------------------------------------------------
// Representations and resolution chains
// ---------------------------------------------------------------------------

/// A single *representation* of a name part. Representations are a layer above
/// stored fields: some are stored verbatim (`Native`, `Ascii`, `Kana`,
/// `Hanja`), some are derived (`Katakana`, `Hiragana`, the ipa→script engines),
/// and `Local` is country-parameterised. Every representation can be **empty**
/// for a given person — that is what lets a fallback chain fall through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rep {
    Native,
    Local,
    Ascii,
    Kana,
    Hiragana,
    Katakana,
    Hangul,
    Cyrillic,
    Hebrew,
    Arabic,
    Hanja,
}

impl Rep {
    /// Parse a representation token used in a chain (`name=[local, ascii]`).
    fn parse(token: &str) -> Option<Self> {
        Some(match token.trim() {
            "native" => Self::Native,
            "local" => Self::Local,
            "ascii" => Self::Ascii,
            "kana" => Self::Kana,
            "hiragana" => Self::Hiragana,
            "katakana" => Self::Katakana,
            "hangul" => Self::Hangul,
            "cyrillic" => Self::Cyrillic,
            "hebrew" => Self::Hebrew,
            "arabic" => Self::Arabic,
            "hanja" => Self::Hanja,
            _ => return None,
        })
    }
}

/// True when a stored `kana` reading is written in hiragana (no katakana
/// letters). A native Japanese name's reading is hiragana; a foreign name's is
/// katakana — so this is an exact discriminator, with no lossy fold needed.
fn kana_is_hiragana(kana: &str) -> bool {
    !kana
        .chars()
        .any(|c| matches!(classify_char(c), Some(Script::Katakana)))
}

/// Use the native form when it is already in the target script, otherwise
/// transcribe it from the broad phonemic ipa.
fn render(e: &NameEntry, target: Script) -> String {
    if detect_script(&e.name) == target {
        e.name.clone()
    } else {
        transcribe(&e.ipa, target)
    }
}

/// Compute one representation of one name entry. `local_set` is the accepted
/// repertoire for the `Local` representation.
fn rep_value(rep: Rep, e: &NameEntry, local_set: &[Repertoire]) -> String {
    match rep {
        Rep::Native => e.name.clone(),
        Rep::Local => local(&e.name, local_set),
        Rep::Ascii => e.ascii_form(),
        Rep::Kana => e.kana.clone(),
        // hiragana exists only for names whose reading is already hiragana
        // (i.e. Japanese names); foreign/katakana names have no hiragana form.
        Rep::Hiragana => {
            if kana_is_hiragana(&e.kana) {
                e.kana.clone()
            } else {
                String::new()
            }
        }
        // Always derivable: hiragana→katakana is the clean direction.
        Rep::Katakana => to_katakana(&e.kana),
        Rep::Hangul => render(e, Script::Hangul),
        Rep::Cyrillic => render(e, Script::Cyrillic),
        Rep::Hebrew => render(e, Script::Hebrew),
        Rep::Arabic => render(e, Script::Arabic),
        Rep::Hanja => e.hanja.clone(),
    }
}

/// Resolve a fallback chain: the first representation that yields a non-empty
/// value wins; an exhausted chain yields `""` (never undefined).
fn resolve(chain: &[Rep], e: &NameEntry, local_set: &[Repertoire]) -> String {
    chain
        .iter()
        .map(|&rep| rep_value(rep, e, local_set))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// Build a `{given, family}` object from two already-resolved part strings. No
/// full name is assembled: a form decides the order and separator (Western uses
/// a space; JP/KR/CN run the parts together), so the caller concatenates the
/// parts it needs rather than receiving a guessed join.
fn branch(given: &str, family: &str) -> VarValue {
    VarValue::object(vec![
        ("given", VarValue::string(given)),
        ("family", VarValue::string(family)),
    ])
}

// ---------------------------------------------------------------------------
// Country presets
// ---------------------------------------------------------------------------

/// The configurable triple a country preset bundles: the `local` accepted
/// repertoire, the primary `name` chain, and the `reading` chain. Each is
/// independently overridable by a `fake:person(...)` param.
struct Preset {
    local: Vec<Repertoire>,
    name: Vec<Rep>,
    reading: Vec<Rep>,
}

/// Per-country person-name preset, sourced from the `person.{name, reading,
/// local}` token lists in the country's `data/geo/*.json` block. An
/// unknown/absent country accepts everything and renders the raw native name
/// (`name=[native]`). Token definitions (what `kanji`, `diacritics_fr`, … mean)
/// are shared and live in [`crate::structured::repertoire`]; only the *choice*
/// of tokens is per-country data here.
fn country_preset(country: &str) -> Preset {
    let Some(geo) = geo_database().get(country) else {
        return Preset {
            local: vec![],
            name: vec![Rep::Native],
            reading: vec![],
        };
    };
    let p = &geo.country.person;
    // Invalid tokens are dropped here but rejected by `geo_presets_all_parse`,
    // so a malformed preset fails the test suite rather than silently degrading.
    let local = p
        .local
        .iter()
        .filter_map(|t| Repertoire::parse(t))
        .collect();
    let reading = p.reading.iter().filter_map(|t| Rep::parse(t)).collect();
    let name: Vec<Rep> = p.name.iter().filter_map(|t| Rep::parse(t)).collect();
    Preset {
        local,
        name: if name.is_empty() {
            vec![Rep::Native]
        } else {
            name
        },
        reading,
    }
}

/// Parse a (possibly bracketed) comma list into its trimmed, non-empty tokens.
fn parse_token_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse_reps(value: &str) -> Result<Vec<Rep>> {
    parse_token_list(value)
        .map(|t| Rep::parse(t).ok_or_else(|| anyhow::anyhow!("unknown name representation: {t}")))
        .collect()
}

fn parse_repertoires(value: &str) -> Result<Vec<Repertoire>> {
    parse_token_list(value)
        .map(|t| Repertoire::parse(t).ok_or_else(|| anyhow::anyhow!("unknown repertoire: {t}")))
        .collect()
}

// ---------------------------------------------------------------------------
// Person generator
// ---------------------------------------------------------------------------

pub(crate) fn generate_person(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let data = names_data();
    let country = params.get("country").map(|s| s.as_str());

    // Pick a given and a family name from the global pools.
    let gi = rng.gen_range(0..data.given_names.len());
    let fi = rng.gen_range(0..data.family_names.len());
    let given = &data.given_names[gi];
    let family = &data.family_names[fi];

    // Country preset, with per-field param overrides (precedence: param > country).
    let preset = country_preset(country.unwrap_or(""));
    let local_set = match params.get("local") {
        Some(v) => parse_repertoires(v)?,
        None => preset.local,
    };
    let name_chain = match params.get("name") {
        Some(v) => parse_reps(v)?,
        None => preset.name,
    };
    let reading_chain = match params.get("reading") {
        Some(v) => parse_reps(v)?,
        None => preset.reading,
    };

    // A raw representation branch — every script is always exposed under its
    // own key, independent of the resolved semantic fields.
    let raw = |rep: Rep| {
        branch(
            &rep_value(rep, given, &local_set),
            &rep_value(rep, family, &local_set),
        )
    };

    // Semantic fields: each name part resolved through its chain. `person` is
    // names only — no full name (the form decides order + separator) and no
    // email/phone (use the `fake:email` / `fake:phone` generators).
    let name_given = resolve(&name_chain, given, &local_set);
    let name_family = resolve(&name_chain, family, &local_set);
    let read_given = resolve(&reading_chain, given, &local_set);
    let read_family = resolve(&reading_chain, family, &local_set);

    let map: Vec<(&str, VarValue)> = vec![
        // Primary fields, resolved through the country-aware `name` chain.
        ("given", VarValue::string(&name_given)),
        ("family", VarValue::string(&name_family)),
        // Reading / furigana field (empty branch when the country declares no
        // reading chain).
        ("reading", branch(&read_given, &read_family)),
        // Raw representation branches — every script, always present.
        ("native", raw(Rep::Native)),
        ("ascii", raw(Rep::Ascii)),
        ("kana", raw(Rep::Kana)),
        ("hiragana", raw(Rep::Hiragana)),
        ("katakana", raw(Rep::Katakana)),
        ("hangul", raw(Rep::Hangul)),
        ("cyrillic", raw(Rep::Cyrillic)),
        ("hebrew", raw(Rep::Hebrew)),
        ("arabic", raw(Rep::Arabic)),
        ("hanja", raw(Rep::Hanja)),
    ];

    Ok(VarValue::object(map))
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::FakeRng;
    use crate::structured::generate_structured;
    use crate::GeneratorDef;

    fn seeded_rng() -> FakeRng {
        FakeRng::from_seed(42)
    }

    fn def(name: &str) -> GeneratorDef {
        GeneratorDef {
            name: name.to_string(),
            params: HashMap::new(),
            positional: Vec::new(),
        }
    }

    fn def_with_params(name: &str, params: &[(&str, &str)]) -> GeneratorDef {
        let params = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        GeneratorDef {
            name: name.to_string(),
            params,
            positional: Vec::new(),
        }
    }

    /// Helper: extract a string field from a VarValue::Object.
    fn field(val: &VarValue, key: &str) -> String {
        val.as_object()
            .expect("should be object")
            .get(key)
            .unwrap_or_else(|| panic!("missing field: {key}"))
            .as_str()
            .unwrap_or_else(|| panic!("field {key} should be string"))
            .to_string()
    }

    // 1. Person produces all expected fields
    #[test]
    fn person_produces_all_fields() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("person"), &mut rng).expect("should generate");
        let obj = result.as_object().expect("should be object");
        assert!(obj.contains_key("given"), "missing 'given'");
        assert!(obj.contains_key("family"), "missing 'family'");
        assert!(obj.contains_key("reading"), "missing 'reading' branch");
        assert!(obj.contains_key("native"), "missing 'native' branch");
        assert!(obj.contains_key("ascii"), "missing 'ascii' branch");
    }

    // 10. Deterministic seed produces same output (person portion)
    #[test]
    fn deterministic_seed_same_person() {
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let person1 = generate_structured(&def("person"), &mut rng1).expect("should generate");
        let person2 = generate_structured(&def("person"), &mut rng2).expect("should generate");
        assert_eq!(person1, person2, "same seed SHALL produce same person");
    }

    // 14. Person with country=JP picks from the global name pools. The drawn
    //     entry is the NATIVE name (the primary field may be romanised when the
    //     name doesn't fit the JP repertoire), so this checks the native branch.
    #[test]
    fn person_jp_picks_from_global_pool() {
        let all_given: Vec<&str> = names_data()
            .given_names
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        let all_family: Vec<&str> = names_data()
            .family_names
            .iter()
            .map(|n| n.name.as_str())
            .collect();

        for seed in 0u64..20 {
            let mut rng = FakeRng::from_seed(seed);
            let d = def_with_params("person", &[("country", "JP")]);
            let result = generate_structured(&d, &mut rng).expect("should generate");
            let native = result.get_path("native").expect("native branch");
            let given = field(native, "given");
            let family = field(native, "family");

            assert!(
                all_given.contains(&given.as_str()),
                "seed={seed}: given name '{given}' SHALL be in global pool"
            );
            assert!(
                all_family.contains(&family.as_str()),
                "seed={seed}: family name '{family}' SHALL be in global pool"
            );
        }
    }

    // 23. given-name pool diversity: >20 unique names from 100 draws (Issue 13)
    #[test]
    fn person_name_pool_diversity() {
        let mut unique_given: std::collections::HashSet<String> = std::collections::HashSet::new();
        for seed in 0u64..100 {
            let mut rng = FakeRng::from_seed(seed);
            let result =
                generate_structured(&def("person"), &mut rng).expect("SHALL generate person");
            unique_given.insert(field(&result, "given"));
        }
        assert!(
            unique_given.len() > 20,
            "SHALL draw from pool >20 unique given names in 100 draws, got {}",
            unique_given.len()
        );
    }

    // 28. On a KR form the primary parts follow the [local=hangul, ascii] chain:
    //     a Korean (hangul) name stays native, any other name has no hangul so
    //     local is empty and it falls through to the ascii fold (the regression
    //     this guards: a default that wrongly returned native). No full name /
    //     ordering is involved — order is the form's concern.
    #[test]
    fn person_kr_given_is_local_or_ascii() {
        let mut rng = seeded_rng();
        let d = def_with_params("person", &[("country", "KR")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let given = field(&result, "given");
        let ascii = result.get_path("ascii").expect("ascii branch");
        let native = result.get_path("native").expect("native branch");

        let ng = field(native, "given");
        let is_hangul =
            !ng.is_empty() && ng.chars().all(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
        if is_hangul {
            assert_eq!(given, ng, "a hangul name SHALL stay native on a KR form");
        } else {
            assert_eq!(
                given,
                field(ascii, "given"),
                "a foreign name SHALL fall to the ascii fold on a KR form"
            );
            assert!(
                given.is_ascii(),
                "the ascii fold SHALL be pure ASCII, got {given:?}"
            );
        }
    }

    // ---- names.json data-validation suite ---------------------------------
    // These reuse the production script primitives (crate::script) so the name
    // pool is held to the exact invariants the per-script person resolver
    // relies on. detect_script et al. never panic at runtime; this suite is
    // what guarantees the data they will run against is well-formed.

    fn all_entries() -> impl Iterator<Item = &'static NameEntry> {
        let d = names_data();
        d.given_names.iter().chain(d.family_names.iter())
    }

    // 37. Every entry SHALL carry non-empty name/kana/ipa. `ascii` is optional:
    //     omitted when the name is already ASCII (then it derives from name).
    #[test]
    fn data_all_entries_have_required_nonempty_fields() {
        for e in all_entries() {
            assert!(
                !e.name.is_empty() && !e.kana.is_empty() && !e.ipa.is_empty(),
                "entry '{}' has an empty required field",
                e.ascii_form()
            );
        }
    }

    // 38. The ASCII form (stored, or folded from the name) SHALL be pure ASCII.
    //     A stored `ascii` is only for names the fold can't derive (non-Latin):
    //     if the fold already yields it, the field is redundant — omit it.
    #[test]
    fn data_ascii_form_is_ascii() {
        for e in all_entries() {
            let ascii = e.ascii_form();
            assert!(
                crate::script::is_ascii_text(&ascii),
                "ascii form '{ascii}' SHALL be pure ASCII"
            );
            if !e.ascii.is_empty() {
                assert_ne!(
                    e.ascii,
                    crate::script::ascii_fold(&e.name),
                    "'{}' stores an ascii the fold already derives — omit it",
                    e.name
                );
            }
        }
    }

    // 39. kana SHALL contain only hiragana/katakana (no latin, punctuation, ・).
    #[test]
    fn data_kana_is_kana_text() {
        for e in all_entries() {
            assert!(
                crate::script::is_kana_text(&e.kana),
                "kana '{}' for '{}' SHALL be hiragana/katakana only",
                e.kana,
                e.ascii_form()
            );
        }
    }

    // 40. ipa SHALL be free of foreign-script contamination (e.g. Cyrillic
    //     homoglyphs); Latin + IPA-extension + Greek θ/β/χ + marks are allowed.
    #[test]
    fn data_ipa_is_clean() {
        for e in all_entries() {
            assert!(
                crate::script::ipa_is_clean(&e.ipa),
                "ipa '{}' for '{}' SHALL contain no non-Latin script contamination",
                e.ipa,
                e.ascii_form()
            );
        }
    }

    // 41. name SHALL be single-script or the legitimate Japanese kanji+kana mix.
    #[test]
    fn data_name_scripts_are_valid() {
        for e in all_entries() {
            assert!(
                crate::script::name_scripts_ok(&e.name),
                "name '{}' ({}) mixes scripts illegitimately",
                e.name,
                e.ascii_form()
            );
        }
    }

    // 42. Reliable-source invariant: the resolution ladder SHALL never
    //     dead-end. Every entry has an ascii source (ascii), a kana-family
    //     source (kana), and a phonemic source (ipa) for derived scripts.
    #[test]
    fn data_every_entry_has_reliable_sources() {
        for e in all_entries() {
            assert!(
                crate::script::is_ascii_text(&e.ascii_form()),
                "{}: missing ascii source",
                e.ascii_form()
            );
            assert!(
                crate::script::is_kana_text(&e.kana),
                "{}: missing kana-family source",
                e.ascii_form()
            );
            assert!(
                !e.ipa.is_empty() && crate::script::ipa_is_clean(&e.ipa),
                "{}: missing phonemic source",
                e.ascii_form()
            );
        }
    }

    // 43. The pool SHALL contain at least one mixed kanji+kana given name, so
    //     name_scripts_ok's Japanese-mix allowance stays exercised by real data.
    #[test]
    fn data_contains_a_mixed_kanji_kana_name() {
        use crate::script::{script_set, Script};
        let found = all_entries().any(|e| {
            let s = script_set(&e.name);
            s.contains(&Script::Han)
                && (s.contains(&Script::Hiragana) || s.contains(&Script::Katakana))
        });
        assert!(
            found,
            "pool SHALL contain a mixed kanji+kana given name (e.g. ゆき子)"
        );
    }

    // 43b. The pool SHALL contain Devanagari and Thai names so the IN/TH
    //      `local` repertoires are exercised by real data.
    #[test]
    fn data_contains_devanagari_and_thai_names() {
        use crate::script::{detect_script, Script};
        assert!(
            all_entries().any(|e| detect_script(&e.name) == Script::Devanagari),
            "pool SHALL contain a Devanagari name"
        );
        assert!(
            all_entries().any(|e| detect_script(&e.name) == Script::Thai),
            "pool SHALL contain a Thai name"
        );
    }

    // 43c. hanja, where present, SHALL be Han-only with one Hanja per Hangul
    //      syllable; and the pool SHALL keep a realistic MIX (some Korean names
    //      with hanja, some without).
    #[test]
    fn data_hanja_is_well_formed_and_mixed() {
        use crate::script::{classify_char, detect_script, Script};
        let mut has_hanja = false;
        let mut korean_without_hanja = false;
        for e in all_entries() {
            if !e.hanja.is_empty() {
                has_hanja = true;
                assert!(
                    e.hanja
                        .chars()
                        .all(|c| matches!(classify_char(c), Some(Script::Han))),
                    "hanja '{}' for '{}' SHALL be Han characters only",
                    e.hanja,
                    e.ascii
                );
                assert_eq!(
                    e.hanja.chars().count(),
                    e.name.chars().count(),
                    "hanja SHALL be one character per Hangul syllable for '{}'",
                    e.ascii
                );
            } else if detect_script(&e.name) == Script::Hangul {
                korean_without_hanja = true;
            }
        }
        assert!(has_hanja, "pool SHALL contain populated hanja");
        assert!(
            korean_without_hanja,
            "pool SHALL keep some Korean names without hanja (realistic mix)"
        );
    }

    // 43d. Special-character coverage: the pool SHALL contain the characters
    //      that break naive name validators — apostrophe, hyphen, ß.
    #[test]
    fn data_contains_special_character_names() {
        assert!(
            all_entries().any(|e| e.name.contains('\'')),
            "pool SHALL contain an apostrophe name (e.g. O'Brien)"
        );
        assert!(
            all_entries().any(|e| e.name.contains('-')),
            "pool SHALL contain a hyphenated name (e.g. Jean-Pierre)"
        );
        assert!(
            all_entries().any(|e| e.name.contains('ß')),
            "pool SHALL contain an eszett name (e.g. Weiß)"
        );
    }

    // 43e. Diacritic coverage: every accented letter in a `diacritics_*`
    //      repertoire SHALL appear in at least one name, so a random draw can
    //      exercise it — except the few with no common personal name.
    #[test]
    fn data_covers_every_diacritic_with_a_known_name() {
        // No common personal name (in any covered language) uses these in their
        // base nominative form: û (French), į/ų (Lithuanian — grammatical only).
        const NO_KNOWN_NAME: &[char] = &['û', 'į', 'ų'];
        let pool: String = all_entries()
            .flat_map(|e| e.name.chars())
            .flat_map(char::to_lowercase)
            .collect();
        for c in crate::structured::repertoire::all_diacritic_chars() {
            if NO_KNOWN_NAME.contains(&c) {
                assert!(
                    !pool.contains(c),
                    "'{c}' is allowlisted as having no name but one now exists — remove it from NO_KNOWN_NAME"
                );
                continue;
            }
            assert!(
                pool.contains(c),
                "no name in the pool uses the diacritic '{c}' — add a name or allowlist it"
            );
        }
    }

    // 43f. Every loaded geo country's person preset SHALL use only valid
    //      representation/repertoire tokens (no silent drops in country_preset).
    #[test]
    fn geo_presets_all_parse() {
        for geo in crate::geo_loader::geo_database().all() {
            let p = &geo.country.person;
            let iso = &geo.country.iso_code;
            for t in &p.local {
                assert!(
                    crate::structured::repertoire::Repertoire::parse(t).is_some(),
                    "{iso}: invalid local repertoire token {t:?}"
                );
            }
            for t in p.name.iter().chain(p.reading.iter()) {
                assert!(
                    Rep::parse(t).is_some(),
                    "{iso}: invalid representation token {t:?}"
                );
            }
        }
    }

    // ---- Phase A: per-script resolver (person.<script>.<part>) -------------

    // 44. person SHALL expose the always-present script branches carrying
    //     non-empty given/family. (hiragana is omitted: it is empty for foreign
    //     names by design — see person_hiragana_branch_has_no_katakana.)
    #[test]
    fn person_exposes_all_stored_script_branches() {
        let mut rng = seeded_rng();
        let r = generate_structured(&def("person"), &mut rng).expect("should generate");
        for script in ["native", "ascii", "kana", "katakana"] {
            for part in ["given", "family"] {
                let path = format!("{script}.{part}");
                let val = r.get_path(&path).and_then(|v| v.as_str());
                assert!(
                    val.is_some_and(|s| !s.is_empty()),
                    "person.{path} SHALL be a non-empty string"
                );
            }
        }
    }

    // 45. The bare default SHALL mirror the native branch.
    #[test]
    fn person_default_mirrors_native_branch() {
        let mut rng = seeded_rng();
        let r = generate_structured(&def("person"), &mut rng).expect("should generate");
        for part in ["given", "family"] {
            assert_eq!(
                r.get_path(part).and_then(|v| v.as_str()),
                r.get_path(&format!("native.{part}"))
                    .and_then(|v| v.as_str()),
                "bare person.{part} SHALL equal person.native.{part}"
            );
        }
    }

    // 47. The katakana branch SHALL contain only katakana (and the ー mark).
    #[test]
    fn person_katakana_branch_is_katakana_only() {
        for seed in 0u64..30 {
            let mut rng = FakeRng::from_seed(seed);
            let r = generate_structured(&def("person"), &mut rng).expect("should generate");
            let f = r
                .get_path("katakana.given")
                .and_then(|v| v.as_str())
                .expect("given");
            assert!(
                !f.is_empty() && f.chars().all(|c| matches!(c as u32, 0x30A0..=0x30FF)),
                "seed={seed}: katakana.given '{f}' SHALL be katakana only"
            );
        }
    }

    // 48. The hiragana branch SHALL contain no katakana letters.
    #[test]
    fn person_hiragana_branch_has_no_katakana() {
        for seed in 0u64..30 {
            let mut rng = FakeRng::from_seed(seed);
            let r = generate_structured(&def("person"), &mut rng).expect("should generate");
            let f = r
                .get_path("hiragana.given")
                .and_then(|v| v.as_str())
                .expect("given");
            assert!(
                f.chars().all(|c| !matches!(c as u32, 0x30A1..=0x30FA)),
                "seed={seed}: hiragana.given '{f}' SHALL have no katakana letters"
            );
        }
    }

    // ---- Target model: representations, chains, presets, params ------------

    fn entry(name: &str, ascii: &str, kana: &str, ipa: &str, hanja: &str) -> NameEntry {
        NameEntry {
            name: name.to_string(),
            ascii: ascii.to_string(),
            kana: kana.to_string(),
            ipa: ipa.to_string(),
            hanja: hanja.to_string(),
        }
    }

    fn taro() -> NameEntry {
        entry("太郎", "Taro", "たろう", "taɾoː", "")
    }
    fn pierre() -> NameEntry {
        entry("Pierre", "Pierre", "ピエール", "pjɛʁ", "")
    }

    // 49. resolve SHALL return the first non-empty representation in the chain.
    #[test]
    fn resolve_first_nonempty_wins() {
        let jp = [Repertoire::Kanji, Repertoire::Hiragana];
        assert_eq!(resolve(&[Rep::Local, Rep::Ascii], &taro(), &jp), "太郎");
        assert_eq!(resolve(&[Rep::Ascii, Rep::Local], &taro(), &jp), "Taro");
    }

    // 50. An exhausted chain SHALL resolve to the empty string (never panic).
    #[test]
    fn resolve_exhausted_chain_is_empty() {
        // A non-Korean entry has no hanja, so a [hanja] chain exhausts.
        assert_eq!(resolve(&[Rep::Hanja], &pierre(), &[]), "");
        assert_eq!(resolve(&[], &taro(), &[]), "");
    }

    // 51. `local` SHALL keep a fitting native name and empty a non-fitting one.
    #[test]
    fn local_rep_keeps_fit_empties_misfit() {
        let jp = [Repertoire::Kanji, Repertoire::Hiragana];
        assert_eq!(rep_value(Rep::Local, &taro(), &jp), "太郎");
        assert_eq!(rep_value(Rep::Local, &pierre(), &jp), "");
        // Empty repertoire = accept-all = native.
        assert_eq!(rep_value(Rep::Local, &pierre(), &[]), "Pierre");
    }

    // 52. The hiragana representation SHALL be the reading for a Japanese name
    //     and empty for a foreign (katakana-reading) name.
    #[test]
    fn hiragana_rep_present_for_jp_empty_for_foreign() {
        assert_eq!(rep_value(Rep::Hiragana, &taro(), &[]), "たろう");
        assert_eq!(rep_value(Rep::Hiragana, &pierre(), &[]), "");
        // Katakana is always derivable (clean direction).
        assert_eq!(rep_value(Rep::Katakana, &taro(), &[]), "タロウ");
        assert_eq!(rep_value(Rep::Katakana, &pierre(), &[]), "ピエール");
    }

    // 54. country=JP `reading` SHALL be katakana; the default (no country)
    //     `reading` SHALL be an all-empty branch.
    #[test]
    fn reading_field_is_katakana_for_jp_empty_by_default() {
        for seed in 0u64..20 {
            let mut rng = FakeRng::from_seed(seed);
            let jp =
                generate_structured(&def_with_params("person", &[("country", "JP")]), &mut rng)
                    .expect("should generate");
            let reading = jp.get_path("reading").expect("reading branch");
            let rf = field(reading, "given");
            assert!(
                !rf.is_empty() && rf.chars().all(|c| matches!(c as u32, 0x30A0..=0x30FF)),
                "seed={seed}: JP reading.given '{rf}' SHALL be non-empty katakana"
            );

            let mut rng2 = FakeRng::from_seed(seed);
            let dflt = generate_structured(&def("person"), &mut rng2).expect("should generate");
            let dr = dflt.get_path("reading").expect("reading branch");
            assert_eq!(
                field(dr, "given"),
                "",
                "seed={seed}: default reading.given SHALL be empty"
            );
        }
    }

    // 55. The `name` param SHALL override the country preset's name chain.
    #[test]
    fn name_param_overrides_country_chain() {
        let mut rng = seeded_rng();
        // country=JP would romanise the seed-42 Latin name; name=[native] keeps
        // it, proving the param wins over the country preset.
        let d = def_with_params("person", &[("country", "JP"), ("name", "[native]")]);
        let r = generate_structured(&d, &mut rng).expect("should generate");
        assert_eq!(
            field(&r, "given"),
            field(r.get_path("native").expect("native"), "given"),
            "name=[native] SHALL force the native form despite country=JP"
        );
    }

    // 56. An unknown representation or repertoire token SHALL be a clear error.
    #[test]
    fn unknown_tokens_error() {
        let mut rng = seeded_rng();
        let bad_rep = def_with_params("person", &[("name", "[wobble]")]);
        let err = generate_structured(&bad_rep, &mut rng).expect_err("SHALL error");
        assert!(
            err.to_string().contains("unknown name representation"),
            "got: {err}"
        );
        let bad_rep2 = def_with_params("person", &[("local", "[klingon]")]);
        let err2 = generate_structured(&bad_rep2, &mut rng).expect_err("SHALL error");
        assert!(
            err2.to_string().contains("unknown repertoire"),
            "got: {err2}"
        );
    }

    // 57. Every person branch part SHALL be a present, string-typed value
    //     (the empty-string guarantee) across seeds and countries.
    #[test]
    fn every_branch_part_is_a_string() {
        let branches = [
            "native", "ascii", "kana", "hiragana", "katakana", "hangul", "cyrillic", "hebrew",
            "arabic", "hanja", "reading",
        ];
        for country in ["", "JP", "KR", "FR", "RU"] {
            for seed in 0u64..15 {
                let mut rng = FakeRng::from_seed(seed);
                let params: &[(&str, &str)] = if country.is_empty() {
                    &[]
                } else {
                    &[("country", country)]
                };
                let d = def_with_params("person", params);
                let r = generate_structured(&d, &mut rng).expect("should generate");
                for b in branches {
                    for part in ["given", "family"] {
                        let path = format!("{b}.{part}");
                        assert!(
                            r.get_path(&path).and_then(|v| v.as_str()).is_some(),
                            "country={country} seed={seed}: person.{path} SHALL be a string"
                        );
                    }
                }
            }
        }
    }
}
