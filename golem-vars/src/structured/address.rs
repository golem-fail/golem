use std::collections::HashMap;

use anyhow::Result;
use rand::Rng;

use crate::geo_loader::{geo_database, GeoData, GeoState};
use crate::VarValue;

// ---------------------------------------------------------------------------
// Address generator
// ---------------------------------------------------------------------------

pub(crate) fn generate_address(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let country = params.get("country").map(|s| s.as_str());
    let state_filter = params.get("state").map(|s| s.as_str());
    let region_filter = params.get("region").map(|s| s.as_str());

    let geo = country.and_then(|c| geo_database().get(c));

    match geo {
        Some(g) => {
            if state_filter.is_some() || region_filter.is_some() {
                generate_address_filtered(g, state_filter, region_filter, rng)
            } else {
                generate_address_from_geo(g, rng)
            }
        }
        None => generate_default_address(rng),
    }
}

/// Generate an address filtering states by name and/or region tag.
fn generate_address_filtered(
    geo: &GeoData,
    state_filter: Option<&str>,
    region_filter: Option<&str>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let matching: Vec<&GeoState> = geo
        .states
        .iter()
        .filter(|s| {
            if let Some(sf) = state_filter {
                if !s.name_en.eq_ignore_ascii_case(sf) {
                    return false;
                }
            }
            if let Some(rf) = region_filter {
                if !s.region_tags.iter().any(|t| t.eq_ignore_ascii_case(rf)) {
                    return false;
                }
            }
            true
        })
        .collect();

    if matching.is_empty() {
        let filters: Vec<String> = [
            state_filter.map(|s| format!("state={s}")),
            region_filter.map(|r| format!("region={r}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        anyhow::bail!(
            "no states match {} for country {}",
            filters.join(", "),
            geo.country.iso_code
        );
    }

    let state = matching[rng.gen_range(0..matching.len())];
    generate_address_from_state(geo, state, rng)
}

pub(crate) fn generate_address_from_geo(
    geo: &GeoData,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    // Pick a random state.
    let state_idx = rng.gen_range(0..geo.states.len());
    let state = &geo.states[state_idx];
    generate_address_from_state(geo, state, rng)
}

/// Generate an address from a specific state within a country.
fn generate_address_from_state(
    geo: &GeoData,
    state: &GeoState,
    rng: &mut impl Rng,
) -> Result<VarValue> {

    // Pick a random city within the state.
    let city_idx = rng.gen_range(0..state.cities.len());
    let city = &state.cities[city_idx];

    // Pick a random postcode entry.
    let pc_idx = rng.gen_range(0..city.postcodes.len());
    let postcode_entry = &city.postcodes[pc_idx];

    // Generate street address from pattern or fixed list.
    let street = if let Some(ref pattern) = postcode_entry.pattern {
        expand_street_pattern(pattern, &postcode_entry.street_en, rng)
    } else if let Some(ref fixed) = postcode_entry.fixed {
        if fixed.is_empty() {
            format!("1 {}", postcode_entry.street_en)
        } else {
            let idx = rng.gen_range(0..fixed.len());
            fixed[idx].clone()
        }
    } else {
        let num: u32 = rng.gen_range(1..200);
        format!("{num} {}", postcode_entry.street_en)
    };

    let map: Vec<(&str, VarValue)> = vec![
        ("street", VarValue::string(&street)),
        ("city", VarValue::string(&city.name_en)),
        ("state", VarValue::string(&state.name_en)),
        ("postcode", VarValue::string(&postcode_entry.code)),
        ("country", VarValue::string(&geo.country.name_en)),
        ("country_code", VarValue::string(&geo.country.iso_code)),
    ];

    Ok(VarValue::object(map))
}

// Re-export from geo module — single implementation for both simple and structured generators.
pub(crate) use crate::geo::expand_street_pattern;

/// Generate an address by picking a random country from the geo database.
pub(crate) fn generate_default_address(rng: &mut impl Rng) -> Result<VarValue> {
    let geo = geo_database().random(rng);
    generate_address_from_geo(geo, rng)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use crate::structured::generate_structured;
    use crate::{GeneratorDef, VarValue};
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::HashMap;

    fn seeded_rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(42)
    }

    fn def(name: &str) -> GeneratorDef {
        GeneratorDef {
            name: name.to_string(),
            params: HashMap::new(),
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

    // 4. Address produces all expected fields
    #[test]
    fn address_produces_all_fields() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("address"), &mut rng).expect("should generate");
        let obj = result.as_object().expect("should be object");
        assert!(obj.contains_key("street"), "missing 'street'");
        assert!(obj.contains_key("city"), "missing 'city'");
        assert!(obj.contains_key("state"), "missing 'state'");
        assert!(obj.contains_key("postcode"), "missing 'postcode'");
        assert!(obj.contains_key("country"), "missing 'country'");
        assert!(obj.contains_key("country_code"), "missing 'country_code'");
    }

    // 5. Address with country=JP returns Japanese data
    #[test]
    fn address_jp_returns_japanese_data() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let country = field(&result, "country");
        let country_code = field(&result, "country_code");
        assert_eq!(country, "Japan");
        assert_eq!(country_code, "JP");
    }

    // 6. Address with country=GB returns British data
    #[test]
    fn address_gb_returns_british_data() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "GB")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let country = field(&result, "country");
        let country_code = field(&result, "country_code");
        assert_eq!(country, "United Kingdom");
        assert_eq!(country_code, "GB");
    }

    // 7. Address fields are internally consistent
    #[test]
    fn address_fields_internally_consistent() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let country = field(&result, "country");
        let country_code = field(&result, "country_code");
        let city = field(&result, "city");
        let state = field(&result, "state");
        let postcode = field(&result, "postcode");

        // All fields should be non-empty.
        assert!(!country.is_empty());
        assert!(!country_code.is_empty());
        assert!(!city.is_empty());
        assert!(!state.is_empty());
        assert!(!postcode.is_empty());

        // Country and country_code should match.
        assert_eq!(country, "Japan");
        assert_eq!(country_code, "JP");

        // Postcode format for JP: ###-####
        assert!(
            postcode.contains('-'),
            "JP postcode should contain hyphen, got: {postcode}"
        );
    }

    // 10. Deterministic seed produces same output (address portion)
    #[test]
    fn deterministic_seed_same_address() {
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let addr1 = generate_structured(&def("address"), &mut rng1).expect("should generate");
        let addr2 = generate_structured(&def("address"), &mut rng2).expect("should generate");
        assert_eq!(addr1, addr2, "same seed SHALL produce same address");
    }

    // 18. Default address picks a random geo country (not hardcoded US)
    #[test]
    fn default_address_uses_random_geo_country() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("address"), &mut rng).expect("should generate");
        let country_code = field(&result, "country_code");
        let loaded_codes = crate::geo_loader::geo_database().countries();
        assert!(
            loaded_codes.contains(&country_code.as_str()),
            "Default address country_code SHALL be from geo database, got: {country_code}"
        );
    }

    // 20. Address with country=KR returns Korean data (Issue 9)
    #[test]
    fn address_kr_returns_korean_data() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "KR")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate KR address");
        let country_code = field(&result, "country_code");
        assert_eq!(
            country_code, "KR",
            "SHALL return country_code KR for address(country=KR)"
        );
    }

    // 21. Address with country=FR returns French data (Issue 9)
    #[test]
    fn address_fr_returns_french_data() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "FR")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate FR address");
        let country_code = field(&result, "country_code");
        assert_eq!(
            country_code, "FR",
            "SHALL return country_code FR for address(country=FR)"
        );
    }

    // 22. Address with state=Tokyo constrains to Tokyo
    #[test]
    fn address_jp_with_state_tokyo() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("state", "Tokyo")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate JP/Tokyo address");
        let state = field(&result, "state");
        assert!(
            state.to_lowercase().contains("tokyo"),
            "SHALL return Tokyo state, got: {state}"
        );
    }

    // 23. Address with region=Kansai constrains to Kansai cities
    #[test]
    fn address_jp_with_region_kansai() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("region", "Kansai")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate JP/Kansai address");
        let country_code = field(&result, "country_code");
        assert_eq!(country_code, "JP", "SHALL be JP address");
        // The state should be one that has a Kansai region tag.
        let state = field(&result, "state");
        assert!(
            !state.is_empty(),
            "SHALL have a non-empty state in Kansai region"
        );
    }

    // 24. Address with nonexistent state returns error
    #[test]
    fn address_jp_nonexistent_state_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("state", "Narnia")]);
        let result = generate_structured(&d, &mut rng);
        assert!(
            result.is_err(),
            "SHALL error for nonexistent state"
        );
    }

    // 25. Address with nonexistent region returns error
    #[test]
    fn address_jp_nonexistent_region_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("region", "Narnia")]);
        let result = generate_structured(&d, &mut rng);
        assert!(
            result.is_err(),
            "SHALL error for nonexistent region"
        );
    }

    // 26. Unknown country code falls back to a random geo country (None branch
    //     of geo lookup when a country param IS supplied but not in the DB).
    #[test]
    fn address_unknown_country_falls_back_to_geo() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "ZZ")]);
        let result =
            generate_structured(&d, &mut rng).expect("SHALL fall back for unknown country");
        let country_code = field(&result, "country_code");
        let loaded_codes = crate::geo_loader::geo_database().countries();
        assert!(
            loaded_codes.contains(&country_code.as_str()),
            "Unknown country SHALL fall back to a geo-database country, got: {country_code}"
        );
    }

    // 27. State filter is case-insensitive (lowercase "tokyo" SHALL match
    //     "Tokyo" via eq_ignore_ascii_case).
    #[test]
    fn address_state_filter_is_case_insensitive() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("state", "tokyo")]);
        let result =
            generate_structured(&d, &mut rng).expect("SHALL match case-insensitively");
        let state = field(&result, "state");
        assert_eq!(
            state, "Tokyo",
            "lowercase state filter SHALL match Tokyo, got: {state}"
        );
    }

    // 28. Region filter is case-insensitive (lowercase "kanto" SHALL match
    //     region tag "Kanto").
    #[test]
    fn address_region_filter_is_case_insensitive() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("region", "kanto")]);
        let result =
            generate_structured(&d, &mut rng).expect("SHALL match region case-insensitively");
        assert_eq!(
            field(&result, "country_code"),
            "JP",
            "SHALL be a JP address"
        );
        // 1. Build the set of JP states tagged "Kanto" (case-insensitive) straight
        //    from the geo DB, then assert the generated state is one of them. This
        //    proves the lowercase "kanto" filter actually matched the "Kanto" tag,
        //    rather than merely returning some arbitrary JP state.
        let kanto_states: Vec<&str> = crate::geo_loader::geo_database()
            .get("JP")
            .expect("JP SHALL be loaded")
            .states
            .iter()
            .filter(|s| s.region_tags.iter().any(|t| t.eq_ignore_ascii_case("kanto")))
            .map(|s| s.name_en.as_str())
            .collect();
        assert!(
            !kanto_states.is_empty(),
            "fixture SHALL contain at least one Kanto-tagged state"
        );
        let state = field(&result, "state");
        assert!(
            kanto_states.contains(&state.as_str()),
            "lowercase region filter SHALL constrain to a Kanto-tagged state, got: {state} (expected one of {kanto_states:?})"
        );
    }

    // 29. Combined state + region filters that both match the same state
    //     (Tokyo is tagged Kanto) SHALL succeed and constrain to that state.
    #[test]
    fn address_combined_state_and_region_match() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "address",
            &[("country", "JP"), ("state", "Tokyo"), ("region", "Kanto")],
        );
        let result = generate_structured(&d, &mut rng)
            .expect("SHALL match when both state and region apply");
        assert_eq!(field(&result, "state"), "Tokyo");
        assert_eq!(field(&result, "country_code"), "JP");
    }

    // 30. Combined state + region filters that conflict (Tokyo is not in the
    //     Kansai region) SHALL error, and the message SHALL name both filters.
    #[test]
    fn address_combined_state_and_region_conflict_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "address",
            &[("country", "JP"), ("state", "Tokyo"), ("region", "Kansai")],
        );
        let err = generate_structured(&d, &mut rng)
            .expect_err("conflicting state+region SHALL error");
        let msg = err.to_string();
        assert!(
            msg.contains("state=Tokyo") && msg.contains("region=Kansai"),
            "error SHALL name both filters, got: {msg}"
        );
        assert!(
            msg.contains("JP"),
            "error SHALL name the country, got: {msg}"
        );
    }

    // 31. generate_address_from_geo (pub(crate)) directly yields a fully
    //     consistent address object for a specific country's geo data.
    #[test]
    fn generate_from_geo_directly_produces_full_address() {
        let mut rng = seeded_rng();
        let geo = crate::geo_loader::geo_database()
            .get("JP")
            .expect("JP SHALL be loaded");
        let result =
            super::generate_address_from_geo(geo, &mut rng).expect("SHALL generate from geo");
        assert_eq!(field(&result, "country_code"), "JP");
        assert!(!field(&result, "street").is_empty());
        assert!(!field(&result, "city").is_empty());
        assert!(!field(&result, "postcode").is_empty());
    }

    use crate::geo_loader::{GeoCity, GeoCountry, GeoData, GeoPostcode, GeoState};

    /// Build a single-state, single-city `GeoData` whose one postcode carries
    /// the supplied street-shaping fields. Lets the street-generation branches
    /// of `generate_address_from_state` be exercised in isolation, with no
    /// dependence on the embedded geo database.
    fn synthetic_geo(
        iso: &str,
        code: &str,
        street_en: &str,
        pattern: Option<&str>,
        fixed: Option<Vec<String>>,
    ) -> GeoData {
        let postcode = GeoPostcode::for_test(code, street_en, pattern, fixed);
        let city = GeoCity::for_test("Testville", vec![postcode]);
        let state = GeoState::for_test(vec!["r1".to_string()], vec![city]);
        GeoData::for_test(GeoCountry::for_test(iso, vec![]), vec![state])
    }

    // 32. generate_address_from_state maps every geo field straight onto the
    //     output object (state name, city name, postcode code, country name and
    //     ISO code all round-trip verbatim).
    #[test]
    fn from_state_maps_all_geo_fields_onto_output() {
        let mut rng = seeded_rng();
        let geo = synthetic_geo("ZZ", "12345", "Main St", None, None);
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate from synthetic state");

        // 1. Geographic fields come straight from the synthetic fixture.
        assert_eq!(field(&result, "city"), "Testville", "city SHALL round-trip");
        assert_eq!(field(&result, "state"), "Region", "state SHALL round-trip");
        assert_eq!(field(&result, "postcode"), "12345", "postcode SHALL round-trip");
        assert_eq!(field(&result, "country"), "Testland", "country SHALL round-trip");
        assert_eq!(
            field(&result, "country_code"),
            "ZZ",
            "country_code SHALL round-trip"
        );
    }

    // 33. With no pattern and no fixed list, the street SHALL be "<num> <street_en>"
    //     where num is in the documented 1..200 range.
    #[test]
    fn from_state_default_street_is_number_plus_street_en() {
        let mut rng = seeded_rng();
        let geo = synthetic_geo("ZZ", "00000", "Cherry Lane", None, None);
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate default street");
        let street = field(&result, "street");

        // 1. Format SHALL be "<num> Cherry Lane".
        assert!(
            street.ends_with(" Cherry Lane"),
            "default street SHALL end with the street name, got: {street}"
        );
        let num: u32 = street
            .split_whitespace()
            .next()
            .expect("SHALL have a leading token")
            .parse()
            .expect("leading token SHALL be a number");
        assert!(
            (1..200).contains(&num),
            "default street number SHALL be in 1..200, got: {num}"
        );
    }

    // 34. An empty fixed list SHALL produce exactly "1 <street_en>" (the
    //     documented empty-fixed fallback), not a random number.
    #[test]
    fn from_state_empty_fixed_yields_one_plus_street_en() {
        let mut rng = seeded_rng();
        let geo = synthetic_geo("ZZ", "00000", "Oak Road", None, Some(vec![]));
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate from empty fixed list");
        assert_eq!(
            field(&result, "street"),
            "1 Oak Road",
            "empty fixed list SHALL yield '1 <street_en>'"
        );
    }

    // 35. A non-empty fixed list SHALL pick one of its verbatim entries as the
    //     street (no numbering applied).
    #[test]
    fn from_state_nonempty_fixed_picks_a_listed_entry() {
        let mut rng = seeded_rng();
        let entries = vec!["10 Downing Street".to_string(), "221B Baker Street".to_string()];
        let geo = synthetic_geo("ZZ", "00000", "Ignored", None, Some(entries.clone()));
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate from fixed list");
        let street = field(&result, "street");
        assert!(
            entries.contains(&street),
            "fixed street SHALL be one of the listed entries, got: {street}"
        );
    }

    // 36. A pattern delegates to expand_street_pattern: an n{min,max} token SHALL
    //     be replaced by a number in range and the street_en SHALL be present.
    #[test]
    fn from_state_pattern_expands_token_and_includes_street_en() {
        let mut rng = seeded_rng();
        let geo = synthetic_geo("ZZ", "00000", "Elm Avenue", Some("n{1,9} Elm Avenue"), None);
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate from pattern");
        let street = field(&result, "street");

        // 1. The literal street name SHALL survive expansion.
        assert!(
            street.ends_with(" Elm Avenue"),
            "pattern street SHALL retain the street name, got: {street}"
        );
        // 2. The n{1,9} token SHALL be replaced by a single-digit number in range.
        let num: u32 = street
            .split_whitespace()
            .next()
            .expect("SHALL have a leading token")
            .parse()
            .expect("leading token SHALL be a number");
        assert!(
            (1..=9).contains(&num),
            "expanded n{{1,9}} number SHALL be in 1..=9, got: {num}"
        );
    }

    // 37. generate_address_from_state is deterministic for a fixed seed and the
    //     same synthetic state (same street, same all-derived fields).
    #[test]
    fn from_state_is_deterministic_for_same_seed() {
        let geo = synthetic_geo("ZZ", "00000", "Pine Street", Some("n{1,200} Pine Street"), None);
        let state = &geo.states[0];

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();
        let a = super::generate_address_from_state(&geo, state, &mut rng1)
            .expect("SHALL generate (run 1)");
        let b = super::generate_address_from_state(&geo, state, &mut rng2)
            .expect("SHALL generate (run 2)");
        assert_eq!(a, b, "same seed + same state SHALL produce identical address");
    }
}
