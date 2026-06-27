//! Geo data loader — embeds and indexes all `data/geo/*.json` files at compile
//! time, exposing them through a single `GeoDatabase` singleton.
//!
//! New country files only require adding one `include_str!` line to the
//! `RAW_ENTRIES` array; every consumer that goes through `geo_database()` will
//! pick it up automatically.

use std::collections::HashMap;
use std::sync::OnceLock;

use rand::Rng;
use serde::Deserialize;

use crate::script::ascii_fold;

// ---------------------------------------------------------------------------
// Compile-time data loading
// ---------------------------------------------------------------------------

/// Each entry is a `(filename, json_str)` pair.  To add a new country, append
/// another tuple here — the ISO code is extracted from the JSON itself.
static RAW_ENTRIES: &[(&str, &str)] = &[
    ("ae.json", include_str!("../../data/geo/ae.json")),
    ("au.json", include_str!("../../data/geo/au.json")),
    ("be.json", include_str!("../../data/geo/be.json")),
    ("br.json", include_str!("../../data/geo/br.json")),
    ("ca.json", include_str!("../../data/geo/ca.json")),
    ("cn.json", include_str!("../../data/geo/cn.json")),
    ("de.json", include_str!("../../data/geo/de.json")),
    ("eg.json", include_str!("../../data/geo/eg.json")),
    ("es.json", include_str!("../../data/geo/es.json")),
    ("fr.json", include_str!("../../data/geo/fr.json")),
    ("gb.json", include_str!("../../data/geo/gb.json")),
    ("ie.json", include_str!("../../data/geo/ie.json")),
    ("il.json", include_str!("../../data/geo/il.json")),
    ("in.json", include_str!("../../data/geo/in.json")),
    ("jp.json", include_str!("../../data/geo/jp.json")),
    ("kr.json", include_str!("../../data/geo/kr.json")),
    ("lt.json", include_str!("../../data/geo/lt.json")),
    ("mx.json", include_str!("../../data/geo/mx.json")),
    ("nl.json", include_str!("../../data/geo/nl.json")),
    ("nz.json", include_str!("../../data/geo/nz.json")),
    ("pl.json", include_str!("../../data/geo/pl.json")),
    ("ru.json", include_str!("../../data/geo/ru.json")),
    ("se.json", include_str!("../../data/geo/se.json")),
    ("sg.json", include_str!("../../data/geo/sg.json")),
    ("th.json", include_str!("../../data/geo/th.json")),
    ("us.json", include_str!("../../data/geo/us.json")),
    ("za.json", include_str!("../../data/geo/za.json")),
];

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct GeoData {
    pub(crate) country: GeoCountry,
    pub(crate) states: Vec<GeoState>,
}

#[derive(Deserialize)]
pub(crate) struct GeoCountry {
    /// Native-script country name (日本, Deutschland, 대한민국).
    pub(crate) name: String,
    /// ASCII romanisation of `name` (Nihon, Deutschland, Daehan-minguk). A
    /// romanisation, never an English exonym; omitted when `name` is already
    /// ASCII (then derived by folding `name` — see [`Self::ascii_name`]).
    #[serde(default)]
    pub(crate) name_ascii: String,
    pub(crate) iso_code: String,
    #[allow(dead_code)]
    pub(crate) phone_prefix: String,
    pub(crate) phone_formats: Vec<String>,
    #[allow(dead_code)]
    pub(crate) postcode_format: String,
    /// Per-country person-name preset: the `name`/`reading` resolution chains
    /// and the `local` accepted repertoire, as repertoire/representation token
    /// names (see `structured::repertoire` / `structured::person`). Empty when
    /// a country file omits the block.
    #[serde(default)]
    pub(crate) person: PersonPreset,
    /// Address structural markers, keyed by the placeholder name a street
    /// `pattern` references (`{chome}`, `{ban}`, `{go}`, …). Each maps to its
    /// native form (rendered into the native address) and its ascii romanisation
    /// (rendered into the `ascii` branch). Defined once per country since the
    /// marker set is small and repeated across that country's postcodes. Empty
    /// for countries whose addresses carry no such markers.
    #[serde(default)]
    pub(crate) markers: HashMap<String, Marker>,
}

/// One address marker's native form and its ascii romanisation (e.g. the
/// Japanese 丁目 → `-chome `).
#[derive(Deserialize)]
pub(crate) struct Marker {
    pub(crate) native: String,
    pub(crate) ascii: String,
}

/// Token-name lists for a country's person-name preset. Parsed into
/// representation/repertoire enums by the person generator.
#[derive(Deserialize, Default)]
pub(crate) struct PersonPreset {
    #[serde(default)]
    pub(crate) name: Vec<String>,
    #[serde(default)]
    pub(crate) reading: Vec<String>,
    #[serde(default)]
    pub(crate) local: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct GeoState {
    /// Native-script state/province name (北海道, Bayern, …).
    pub(crate) name: String,
    /// ASCII romanisation of `name` (Hokkaido, Bayern, …); a romanisation, not
    /// an English exonym (Bayern, never "Bavaria"); omitted when already ASCII.
    #[serde(default)]
    pub(crate) name_ascii: String,
    pub(crate) region_tags: Vec<String>,
    pub(crate) cities: Vec<GeoCity>,
}

#[derive(Deserialize)]
pub(crate) struct GeoCity {
    /// Native-script city name (札幌市, München, …).
    pub(crate) name: String,
    /// ASCII romanisation of `name` (Sapporo, Muenchen, …); a romanisation, not
    /// an English exonym (Muenchen, never "Munich"); omitted when already ASCII.
    #[serde(default)]
    pub(crate) name_ascii: String,
    pub(crate) lat: f64,
    pub(crate) lon: f64,
    pub(crate) postcodes: Vec<GeoPostcode>,
}

#[derive(Deserialize)]
pub(crate) struct GeoPostcode {
    pub(crate) code: String,
    /// Native-script street name (北一条西, Königstraße, …).
    pub(crate) street: String,
    /// ASCII romanisation of `street` (Kita 1-jo Nishi, Koenigstrasse, …);
    /// omitted when `street` is already ASCII (then folded from `street`).
    #[serde(default)]
    pub(crate) street_ascii: String,
    /// Native street pattern — a skeleton of placeholders and `n{min,max}`
    /// house-number token(s): `{street}` for the street name and `{<marker>}`
    /// for each country marker (`{chome}`, `{ban}`, …). Number tokens carry the
    /// native numeral style (full-width for JP, Arabic-Indic for AR). One string,
    /// OR an array of strings to pick one from at random (a finite set of known
    /// addresses — what used to be a separate `fixed` list). The ascii street is
    /// derived from this same skeleton (markers→ascii, `{street}`→the
    /// romanisation, ASCII digits) — see `address::street_pair`.
    #[serde(default)]
    pub(crate) pattern: Option<Pattern>,
}

/// A street pattern: a single skeleton, or a set to choose one from at random.
/// Both go through the identical expansion in `address::street_pair`; the only
/// difference is the array form picks an entry first.
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Pattern {
    One(String),
    Many(Vec<String>),
}

/// The ASCII form of a node: the ASCII fold of its stored romanisation when
/// present, else of the native text (Latin nodes omit the stored field and fold
/// directly). Folding the chosen source guarantees a pure-ASCII result and
/// tolerates a stored romanisation that still carries diacritics (e.g. pinyin
/// tone marks: "Méihuá" → "Meihua"). Single source for every geo `ascii_*`.
fn ascii_or_fold(stored: &str, native: &str) -> String {
    ascii_fold(if stored.is_empty() { native } else { stored })
}

impl GeoCountry {
    /// ASCII country name (stored romanisation, else folded native).
    pub(crate) fn ascii_name(&self) -> String {
        ascii_or_fold(&self.name_ascii, &self.name)
    }
}

impl GeoState {
    /// ASCII state name (stored romanisation, else folded native).
    pub(crate) fn ascii_name(&self) -> String {
        ascii_or_fold(&self.name_ascii, &self.name)
    }
}

impl GeoCity {
    /// ASCII city name (stored romanisation, else folded native).
    pub(crate) fn ascii_name(&self) -> String {
        ascii_or_fold(&self.name_ascii, &self.name)
    }
}

impl GeoPostcode {
    /// ASCII street name (stored romanisation, else folded native street).
    pub(crate) fn ascii_street(&self) -> String {
        ascii_or_fold(&self.street_ascii, &self.street)
    }
}

// ---------------------------------------------------------------------------
// Test-only constructors
// ---------------------------------------------------------------------------
//
// These let in-crate tests (here and in `geo.rs`) hand-build geo structures
// without depending on the embedded database. They only assemble existing
// fields — no parsing, no I/O, no behavior.

#[cfg(test)]
impl GeoData {
    /// Build a `GeoData` from a country and its states (test fixtures only).
    pub(crate) fn for_test(country: GeoCountry, states: Vec<GeoState>) -> Self {
        Self { country, states }
    }
}

#[cfg(test)]
impl GeoCountry {
    /// Build a `GeoCountry` with the given ISO code and phone formats; the
    /// remaining descriptive fields are filled with neutral placeholders.
    pub(crate) fn for_test(iso_code: &str, phone_formats: Vec<String>) -> Self {
        Self {
            name: "Testland".to_string(),
            name_ascii: String::new(),
            iso_code: iso_code.to_string(),
            phone_prefix: String::new(),
            phone_formats,
            postcode_format: String::new(),
            person: PersonPreset::default(),
            markers: HashMap::new(),
        }
    }
}

#[cfg(test)]
impl GeoState {
    /// Build a `GeoState` with the given region tags and cities (test fixtures).
    pub(crate) fn for_test(region_tags: Vec<String>, cities: Vec<GeoCity>) -> Self {
        Self {
            name: "Region".to_string(),
            name_ascii: String::new(),
            region_tags,
            cities,
        }
    }
}

#[cfg(test)]
impl GeoCity {
    /// Build a `GeoCity` with the given (native) name and postcodes (test fixtures).
    pub(crate) fn for_test(name: &str, postcodes: Vec<GeoPostcode>) -> Self {
        Self {
            name: name.to_string(),
            name_ascii: String::new(),
            lat: 0.0,
            lon: 0.0,
            postcodes,
        }
    }
}

#[cfg(test)]
impl GeoPostcode {
    /// Build a `GeoPostcode` from the address-shaping fields (test fixtures).
    /// `street` is the native street name; the ascii form folds from it.
    pub(crate) fn for_test(code: &str, street: &str, pattern: Option<Pattern>) -> Self {
        Self {
            code: code.to_string(),
            street: street.to_string(),
            street_ascii: String::new(),
            pattern,
        }
    }
}

// ---------------------------------------------------------------------------
// GeoDatabase
// ---------------------------------------------------------------------------

/// A collection of parsed `GeoData` entries, keyed by uppercase ISO country
/// code (e.g. `"JP"`, `"GB"`).
pub(crate) struct GeoDatabase {
    map: HashMap<String, GeoData>,
}

impl GeoDatabase {
    /// Parse every embedded JSON blob and index by ISO code.
    fn new() -> Self {
        Self::from_entries(RAW_ENTRIES)
    }

    /// Parse each `(filename, json_str)` entry and index by uppercase ISO code.
    /// The `filename` is used only for panic context on parse failure.
    fn from_entries(entries: &[(&str, &str)]) -> Self {
        let mut map = HashMap::new();
        for (filename, json) in entries {
            let data: GeoData = serde_json::from_str(json).unwrap_or_else(|e| {
                panic!("failed to parse geo data from {filename}: {e}");
            });
            let key = data.country.iso_code.to_uppercase();
            map.insert(key, data);
        }
        Self { map }
    }

    /// Look up a country by ISO code (case-insensitive).
    pub(crate) fn get(&self, iso_code: &str) -> Option<&GeoData> {
        self.map.get(&iso_code.to_uppercase())
    }

    /// Return a sorted list of all loaded country ISO codes.
    #[allow(dead_code)]
    pub(crate) fn countries(&self) -> Vec<&str> {
        let mut codes: Vec<&str> = self.map.keys().map(|s| s.as_str()).collect();
        codes.sort();
        codes
    }

    /// Iterate over all loaded `GeoData` entries. Test-only since the scalar
    /// city/postcode generators (its last non-test callers) were removed.
    #[cfg(test)]
    pub(crate) fn all(&self) -> impl Iterator<Item = &GeoData> {
        self.map.values()
    }

    /// Pick a random `GeoData` entry. Entries are sorted by ISO code for
    /// deterministic results with seeded RNGs.
    pub(crate) fn random(&self, rng: &mut impl Rng) -> &GeoData {
        let mut entries: Vec<&GeoData> = self.map.values().collect();
        entries.sort_by(|a, b| a.country.iso_code.cmp(&b.country.iso_code));
        entries[rng.gen_range(0..entries.len())]
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

/// Returns the global, lazily-initialised `GeoDatabase`.
pub(crate) fn geo_database() -> &'static GeoDatabase {
    static INSTANCE: OnceLock<GeoDatabase> = OnceLock::new();
    INSTANCE.get_or_init(GeoDatabase::new)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn seeded_rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(42)
    }

    // 1. JP is present with correct iso_code
    #[test]
    fn geo_database_loads_jp() {
        let db = geo_database();
        let jp = db.get("JP").expect("JP should be loaded");
        assert_eq!(jp.country.iso_code, "JP");
    }

    // 1b. Every loaded country has non-empty generation pools, so the address
    //     and phone generators never index into an empty slice. Cheap (27
    //     countries) and the backstop for the runtime `bail!` guards.
    #[test]
    fn all_countries_have_non_empty_pools() {
        for g in geo_database().all() {
            let cc = &g.country.iso_code;
            assert!(
                !g.country.phone_formats.is_empty(),
                "{cc}: empty phone_formats"
            );
            assert!(!g.states.is_empty(), "{cc}: empty states");
            for s in &g.states {
                assert!(!s.cities.is_empty(), "{cc}/{}: empty cities", s.name);
                for c in &s.cities {
                    assert!(
                        !c.postcodes.is_empty(),
                        "{cc}/{}/{}: empty postcodes",
                        s.name,
                        c.name
                    );
                }
            }
        }
    }

    // 1c. Every node's ASCII form (stored romanisation, else folded native) is
    //     pure ASCII. This is what guarantees the `address.ascii` branch never
    //     leaks native script: a non-Latin node that forgets its romanisation
    //     would fold to non-ASCII and fail here.
    #[test]
    fn all_ascii_forms_are_pure_ascii() {
        for g in geo_database().all() {
            let cc = &g.country.iso_code;
            let ca = g.country.ascii_name();
            assert!(ca.is_ascii(), "{cc}: country ascii_name not ASCII: {ca}");
            for s in &g.states {
                let sa = s.ascii_name();
                assert!(
                    sa.is_ascii(),
                    "{cc}/{}: state ascii not ASCII: {sa}",
                    s.name
                );
                for c in &s.cities {
                    let cia = c.ascii_name();
                    assert!(
                        cia.is_ascii(),
                        "{cc}/{}: city ascii not ASCII: {cia}",
                        c.name
                    );
                    for p in &c.postcodes {
                        let st = p.ascii_street();
                        assert!(
                            st.is_ascii(),
                            "{cc}/{}/{}: street ascii not ASCII: {st}",
                            s.name,
                            c.name
                        );
                    }
                }
            }
        }
    }

    // 1d. A node whose native folds to pure ASCII (Latin, diacritics included)
    //     SHALL NOT store a romanisation — the fold derives it. Mirrors the
    //     person pool's "omit a redundant ascii" rule; keeps Latin-script geo
    //     data from re-storing what the fold already produces.
    #[test]
    fn no_redundant_stored_ascii_on_latin_nodes() {
        fn check(stored: &str, native: &str, what: &str) {
            if ascii_fold(native).is_ascii() {
                assert!(
                    stored.is_empty(),
                    "{what}: native '{native}' folds to ASCII — drop the stored '{stored}'"
                );
            }
        }
        for g in geo_database().all() {
            let cc = &g.country.iso_code;
            check(
                &g.country.name_ascii,
                &g.country.name,
                &format!("{cc} country"),
            );
            for s in &g.states {
                check(&s.name_ascii, &s.name, &format!("{cc}/{}", s.name));
                for c in &s.cities {
                    check(&c.name_ascii, &c.name, &format!("{cc}/{}", c.name));
                    for p in &c.postcodes {
                        check(
                            &p.street_ascii,
                            &p.street,
                            &format!("{cc}/{} street", c.name),
                        );
                    }
                }
            }
        }
    }

    // 2. GB is present with correct iso_code
    #[test]
    fn geo_database_loads_gb() {
        let db = geo_database();
        let gb = db.get("GB").expect("GB should be loaded");
        assert_eq!(gb.country.iso_code, "GB");
    }

    // 3. At least 2 countries loaded
    #[test]
    fn geo_database_country_count() {
        let db = geo_database();
        assert!(
            db.countries().len() >= 2,
            "expected at least 2 countries, got {}",
            db.countries().len()
        );
    }

    // 4. Unknown code returns None
    #[test]
    fn geo_database_get_unknown_returns_none() {
        let db = geo_database();
        assert!(db.get("XX").is_none());
    }

    // 5. countries() contains both JP and GB
    #[test]
    fn geo_database_countries_contains_jp_and_gb() {
        let codes = geo_database().countries();
        assert!(codes.contains(&"JP"), "countries SHALL include JP");
        assert!(codes.contains(&"GB"), "countries SHALL include GB");
    }

    // 6. JP has non-empty states
    #[test]
    fn geo_database_jp_has_states() {
        let jp = geo_database().get("JP").expect("JP should be loaded");
        assert!(!jp.states.is_empty(), "JP should have at least one state");
    }

    // 7. GB has non-empty states
    #[test]
    fn geo_database_gb_has_states() {
        let gb = geo_database().get("GB").expect("GB should be loaded");
        assert!(!gb.states.is_empty(), "GB should have at least one state");
    }

    // 8. JP phone formats start with +81
    #[test]
    fn geo_database_jp_phone_format() {
        let jp = geo_database().get("JP").expect("JP should be loaded");
        assert!(
            !jp.country.phone_formats.is_empty(),
            "JP should have phone formats"
        );
        for fmt in &jp.country.phone_formats {
            assert!(
                fmt.starts_with("+81"),
                "JP phone format should start with +81, got: {fmt}"
            );
        }
    }

    // 9. GB phone formats start with +44
    #[test]
    fn geo_database_gb_phone_format() {
        let gb = geo_database().get("GB").expect("GB should be loaded");
        assert!(
            !gb.country.phone_formats.is_empty(),
            "GB should have phone formats"
        );
        for fmt in &gb.country.phone_formats {
            assert!(
                fmt.starts_with("+44"),
                "GB phone format should start with +44, got: {fmt}"
            );
        }
    }

    // 10. all() yields at least 2 entries
    #[test]
    fn geo_database_all_iterates() {
        let count = geo_database().all().count();
        assert!(count >= 2, "all() should yield at least 2, got {count}");
    }

    // 11. Case-insensitive lookup works
    #[test]
    fn geo_database_case_insensitive_lookup() {
        let db = geo_database();
        assert!(db.get("jp").is_some(), "lowercase 'jp' SHALL resolve");
        assert!(db.get("Gb").is_some(), "mixed-case 'Gb' SHALL resolve");
    }

    // 12. JP has cities with postcodes
    #[test]
    fn geo_database_jp_has_postcodes() {
        let jp = geo_database().get("JP").expect("JP should be loaded");
        let postcode_count: usize = jp
            .states
            .iter()
            .flat_map(|s| &s.cities)
            .map(|c| c.postcodes.len())
            .sum();
        assert!(
            postcode_count > 0,
            "JP should have at least one postcode entry"
        );
    }

    // 13. All 27 countries SHALL be loaded
    #[test]
    fn geo_database_loads_all_27_countries() {
        let db = geo_database();
        let count = db.countries().len();
        assert!(count >= 27, "SHALL load at least 27 countries, got {count}");
    }

    // 14. Every expected ISO code SHALL be present
    #[test]
    fn geo_database_all_expected_codes_present() {
        let db = geo_database();
        let expected = [
            "AE", "AU", "BE", "BR", "CA", "CN", "DE", "EG", "ES", "FR", "GB", "IE", "IL", "IN",
            "JP", "KR", "LT", "MX", "NL", "NZ", "PL", "RU", "SE", "SG", "TH", "US", "ZA",
        ];
        for code in &expected {
            assert!(db.get(code).is_some(), "SHALL load country {code}");
        }
    }

    // 15. Every loaded country SHALL have non-empty states
    #[test]
    fn geo_database_every_country_has_states() {
        for geo in geo_database().all() {
            assert!(
                !geo.states.is_empty(),
                "SHALL have states for {}",
                geo.country.iso_code
            );
        }
    }

    // 16. Every loaded country SHALL have phone formats
    #[test]
    fn geo_database_every_country_has_phone_formats() {
        for geo in geo_database().all() {
            assert!(
                !geo.country.phone_formats.is_empty(),
                "SHALL have phone formats for {}",
                geo.country.iso_code
            );
        }
    }

    // 17. countries() SHALL return ISO codes in ascending sorted order
    #[test]
    fn geo_database_countries_is_sorted() {
        let codes = geo_database().countries();
        // 1. Assert each adjacent pair is ascending, independently of the
        //    implementation's own .sort() call (no re-sorting to derive expected).
        assert!(
            codes.windows(2).all(|w| w[0] <= w[1]),
            "countries() SHALL be sorted ascending, got {codes:?}"
        );
    }

    // 19. random() with the same seed SHALL produce the same country
    #[test]
    fn geo_database_random_is_deterministic_for_same_seed() {
        let db = geo_database();
        let first = db.random(&mut seeded_rng()).country.iso_code.clone();
        let second = db.random(&mut seeded_rng()).country.iso_code.clone();
        assert_eq!(
            first, second,
            "same seed SHALL produce the same random country"
        );
    }

    // 20. random() SHALL always return a country that is actually loaded
    #[test]
    fn geo_database_random_returns_loaded_country() {
        let db = geo_database();
        let mut rng = seeded_rng();
        for _ in 0..50 {
            let picked = &db.random(&mut rng).country.iso_code;
            assert!(
                db.get(picked).is_some(),
                "random() SHALL return a loaded country, got {picked}"
            );
        }
    }

    // 21. Empty-string lookup SHALL return None
    #[test]
    fn geo_database_get_empty_returns_none() {
        assert!(
            geo_database().get("").is_none(),
            "empty code SHALL resolve to None"
        );
    }

    // A minimal, self-contained geo JSON blob for the given lowercase ISO code,
    // used to exercise GeoDatabase::from_entries() without the embedded data.
    fn fixture_json(iso_code: &str) -> String {
        // Built via concatenation rather than a raw string so the JSON can carry
        // literal '#' placeholders (phone/postcode formats) without colliding
        // with the raw-string hash delimiter.
        // Only native fields + the required ones are emitted; the optional
        // `*_ascii` / `pattern_ascii` are omitted to exercise their serde
        // defaults (an absent romanisation folds from the native text).
        let mut json = String::from("{\n");
        json.push_str("  \"country\": {\n");
        json.push_str("    \"name\": \"Testland\",\n");
        json.push_str(&format!("    \"iso_code\": \"{iso_code}\",\n"));
        json.push_str("    \"phone_prefix\": \"+99\",\n");
        json.push_str("    \"phone_formats\": [\"+99 ### ####\"],\n");
        json.push_str("    \"postcode_format\": \"#####\"\n");
        json.push_str("  },\n");
        json.push_str("  \"states\": [{\n");
        json.push_str("    \"name\": \"Region\",\n");
        json.push_str("    \"region_tags\": [\"r1\"],\n");
        json.push_str("    \"cities\": [{\n");
        json.push_str("      \"name\": \"City\",\n");
        json.push_str("      \"lat\": 1.0,\n");
        json.push_str("      \"lon\": 2.0,\n");
        json.push_str("      \"postcodes\": [{\n");
        json.push_str("        \"code\": \"00000\",\n");
        json.push_str("        \"street\": \"Street\",\n");
        json.push_str("        \"pattern\": null\n");
        json.push_str("      }]\n");
        json.push_str("    }]\n");
        json.push_str("  }]\n");
        json.push_str("}\n");
        json
    }

    // 22. from_entries() SHALL parse each blob and key it by uppercase ISO code.
    #[test]
    fn from_entries_indexes_by_uppercase_iso_code() {
        // 1. Provide a lowercase iso_code to confirm the key is upper-cased.
        let json = fixture_json("zz");
        let entries = [("zz.json", json.as_str())];
        let db = GeoDatabase::from_entries(&entries);
        // 2. Lookup SHALL succeed via the upper-cased key.
        assert!(
            db.get("ZZ").is_some(),
            "from_entries SHALL index by upper-cased ISO code"
        );
        // 3. And only that one entry SHALL be present.
        assert_eq!(
            db.countries(),
            vec!["ZZ"],
            "from_entries SHALL load exactly the supplied entries"
        );
    }

    // 23. from_entries() with an empty slice SHALL yield an empty database.
    #[test]
    fn from_entries_empty_slice_yields_empty_db() {
        let db = GeoDatabase::from_entries(&[]);
        assert!(
            db.countries().is_empty(),
            "empty input SHALL produce no countries"
        );
        assert_eq!(db.all().count(), 0, "empty input SHALL yield no entries");
    }

    // 24. from_entries() SHALL preserve the parsed nested data verbatim.
    #[test]
    fn from_entries_preserves_parsed_fields() {
        let json = fixture_json("zz");
        let entries = [("zz.json", json.as_str())];
        let db = GeoDatabase::from_entries(&entries);
        let data = db.get("ZZ").expect("ZZ SHALL be loaded");
        // 1. Country-level fields round-trip from the JSON.
        assert_eq!(data.country.iso_code, "zz");
        assert_eq!(data.country.phone_formats, vec!["+99 ### ####"]);
        // 2. Nested state/city/postcode structure round-trips.
        let postcode = &data.states[0].cities[0].postcodes[0];
        assert_eq!(postcode.code, "00000");
        assert_eq!(postcode.street, "Street");
        // The omitted romanisation folds from the native street.
        assert_eq!(postcode.ascii_street(), "Street");
        assert!(postcode.pattern.is_none());
    }

    // 25. A later entry with a duplicate ISO code SHALL overwrite the earlier one
    //     (HashMap::insert semantics — documents existing behavior).
    #[test]
    fn from_entries_duplicate_iso_code_last_wins() {
        let first = fixture_json("zz");
        let second = fixture_json("zz");
        let entries = [("a.json", first.as_str()), ("b.json", second.as_str())];
        let db = GeoDatabase::from_entries(&entries);
        assert_eq!(
            db.countries(),
            vec!["ZZ"],
            "duplicate ISO codes SHALL collapse to a single keyed entry"
        );
    }

    // 26. A malformed blob SHALL panic with the filename for context.
    #[test]
    #[should_panic(expected = "bad.json")]
    fn from_entries_malformed_json_panics_with_filename() {
        let entries = [("bad.json", "{ not valid json ")];
        let _ = GeoDatabase::from_entries(&entries);
    }
}
