//! Geo data loader — embeds and indexes all `data/geo/*.json` files at compile
//! time, exposing them through a single `GeoDatabase` singleton.
//!
//! New country files only require adding one `include_str!` line to the
//! `RAW_ENTRIES` array; every consumer that goes through `geo_database()` will
//! pick it up automatically.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Compile-time data loading
// ---------------------------------------------------------------------------

/// Each entry is a `(filename, json_str)` pair.  To add a new country, append
/// another tuple here — the ISO code is extracted from the JSON itself.
static RAW_ENTRIES: &[(&str, &str)] = &[
    ("jp.json", include_str!("../../data/geo/jp.json")),
    ("gb.json", include_str!("../../data/geo/gb.json")),
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
    #[allow(dead_code)]
    pub(crate) name_en: String,
    pub(crate) iso_code: String,
    #[allow(dead_code)]
    pub(crate) phone_prefix: String,
    pub(crate) phone_formats: Vec<String>,
    #[allow(dead_code)]
    pub(crate) postcode_format: String,
    #[allow(dead_code)]
    pub(crate) name_order: String,
}

#[derive(Deserialize)]
pub(crate) struct GeoState {
    #[allow(dead_code)]
    pub(crate) name: String,
    #[allow(dead_code)]
    pub(crate) name_en: String,
    pub(crate) region_tags: Vec<String>,
    pub(crate) cities: Vec<GeoCity>,
}

#[derive(Deserialize)]
pub(crate) struct GeoCity {
    #[allow(dead_code)]
    pub(crate) name: String,
    pub(crate) name_en: String,
    #[allow(dead_code)]
    pub(crate) lat: f64,
    #[allow(dead_code)]
    pub(crate) lon: f64,
    pub(crate) postcodes: Vec<GeoPostcode>,
}

#[derive(Deserialize)]
pub(crate) struct GeoPostcode {
    pub(crate) code: String,
    #[allow(dead_code)]
    pub(crate) street: String,
    pub(crate) street_en: String,
    pub(crate) pattern: Option<String>,
    pub(crate) fixed: Option<Vec<String>>,
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
        let mut map = HashMap::new();
        for (filename, json) in RAW_ENTRIES {
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

    /// Iterate over all loaded `GeoData` entries.
    pub(crate) fn all(&self) -> impl Iterator<Item = &GeoData> {
        self.map.values()
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

    // 1. JP is present with correct iso_code
    #[test]
    fn geo_database_loads_jp() {
        let db = geo_database();
        let jp = db.get("JP").expect("JP should be loaded");
        assert_eq!(jp.country.iso_code, "JP");
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
        assert!(codes.contains(&"JP"), "countries should include JP");
        assert!(codes.contains(&"GB"), "countries should include GB");
    }

    // 6. JP has non-empty states
    #[test]
    fn geo_database_jp_has_states() {
        let jp = geo_database().get("JP").expect("JP should be loaded");
        assert!(
            !jp.states.is_empty(),
            "JP should have at least one state"
        );
    }

    // 7. GB has non-empty states
    #[test]
    fn geo_database_gb_has_states() {
        let gb = geo_database().get("GB").expect("GB should be loaded");
        assert!(
            !gb.states.is_empty(),
            "GB should have at least one state"
        );
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
        assert!(
            count >= 2,
            "all() should yield at least 2, got {count}"
        );
    }

    // 11. Case-insensitive lookup works
    #[test]
    fn geo_database_case_insensitive_lookup() {
        let db = geo_database();
        assert!(db.get("jp").is_some(), "lowercase 'jp' should resolve");
        assert!(db.get("Gb").is_some(), "mixed-case 'Gb' should resolve");
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
}
