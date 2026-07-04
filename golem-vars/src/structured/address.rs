use std::collections::HashMap;

use anyhow::Result;
use rand::Rng;

use crate::geo::{
    expand_native_tokens, fill_ascii_tokens, fold_fullwidth_digits, tidy_ascii_spacing,
};
use crate::geo_loader::{geo_database, GeoData, GeoPostcode, GeoState, Marker, Pattern};
use crate::script::ascii_fold;
use crate::VarValue;

// ---------------------------------------------------------------------------
// Address generator
// ---------------------------------------------------------------------------

pub(crate) fn generate_address(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let state_filter = params.get("state").map(|s| s.as_str());
    let region_filter = params.get("region").map(|s| s.as_str());

    // Unset country → random; a present-but-unknown code → error (a typo
    // shouldn't silently produce some other country's address).
    let geo = match params.get("country") {
        Some(code) => Some(
            geo_database()
                .get(code)
                .ok_or_else(|| anyhow::anyhow!("unknown country code: {code}"))?,
        ),
        None => None,
    };

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
                // Accept either the native name (東京) or the ascii romanisation
                // (Tokyo), case-insensitively, so a user can pass whichever.
                if !s.name.eq_ignore_ascii_case(sf) && !s.ascii_name().eq_ignore_ascii_case(sf) {
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

pub(crate) fn generate_address_from_geo(geo: &GeoData, rng: &mut impl Rng) -> Result<VarValue> {
    // Pick a random state. Guard the empty pool rather than panicking in
    // `gen_range` — current data is non-empty (enforced by a geo-data
    // validation test), but a future country file shouldn't crash the run.
    if geo.states.is_empty() {
        anyhow::bail!("no states for country {}", geo.country.iso_code);
    }
    let state_idx = rng.gen_range(0..geo.states.len());
    let state = &geo.states[state_idx];
    generate_address_from_state(geo, state, rng)
}

/// Substitute the `{street}` and `{<marker>}` placeholders in a street pattern.
/// `street` is the value for `{street}` (the native name, or its romanisation,
/// per the caller); `ascii` selects each marker's romanised form over its native
/// form. Plain string replacement — placeholders never nest.
fn fill_placeholders(
    pattern: &str,
    street: &str,
    markers: &HashMap<String, Marker>,
    ascii: bool,
) -> String {
    let mut s = pattern.replace("{street}", street);
    for (name, m) in markers {
        let repl = if ascii { &m.ascii } else { &m.native };
        s = s.replace(&format!("{{{name}}}"), repl);
    }
    s
}

/// Render a postcode's street in both native and ascii form, sharing the same
/// drawn house number(s). The pattern is a `{street}`/`{marker}`/`n{}` skeleton
/// (one string, or an array to pick one from): native fills it with the native
/// street + native markers + native numerals; ascii fills the SAME skeleton with
/// the romanised street + ascii markers + ASCII digits, then tidies the
/// letter→digit spacing. With no pattern, a plain `"<num> <street>"` is used.
/// `markers` is the country's marker map.
fn street_pair(
    pc: &GeoPostcode,
    markers: &HashMap<String, Marker>,
    rng: &mut impl Rng,
) -> (String, String) {
    // Resolve the chosen skeleton (one, or a random pick from the set).
    let skeleton = match &pc.pattern {
        Some(Pattern::One(s)) => Some(s.as_str()),
        Some(Pattern::Many(v)) if !v.is_empty() => Some(v[rng.gen_range(0..v.len())].as_str()),
        _ => None,
    };

    match skeleton {
        Some(pattern) => {
            let native_skel = fill_placeholders(pattern, &pc.street, markers, false);
            let (native, nums) = expand_native_tokens(&native_skel, rng);

            let ascii_skel = fill_placeholders(pattern, &pc.ascii_street(), markers, true);
            // Same drawn numbers, ASCII digits; fold for safety, tidy the spacing.
            let ascii = tidy_ascii_spacing(&ascii_fold(&fill_ascii_tokens(&ascii_skel, &nums)));
            (native, ascii)
        }
        None => {
            let num: u32 = rng.gen_range(1..200);
            (
                format!("{num} {}", pc.street),
                format!("{num} {}", pc.ascii_street()),
            )
        }
    }
}

/// Generate an address from a specific state within a country. The top-level
/// fields are NATIVE; an `ascii` sub-object carries the romanised text fields.
/// Script-neutral fields (`country_code`, `lat`, `lon`) appear only top-level.
fn generate_address_from_state(
    geo: &GeoData,
    state: &GeoState,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    // Pick a random city within the state (guard empty pools, don't panic).
    if state.cities.is_empty() {
        anyhow::bail!(
            "no cities for state {} in {}",
            state.ascii_name(),
            geo.country.iso_code
        );
    }
    let city_idx = rng.gen_range(0..state.cities.len());
    let city = &state.cities[city_idx];

    // Pick a random postcode entry.
    if city.postcodes.is_empty() {
        anyhow::bail!(
            "no postcodes for city {} in {}",
            city.ascii_name(),
            geo.country.iso_code
        );
    }
    let pc_idx = rng.gen_range(0..city.postcodes.len());
    let postcode_entry = &city.postcodes[pc_idx];

    let (street_native, street_ascii) = street_pair(postcode_entry, &geo.country.markers, rng);

    // ASCII view: only the script-variant text fields (romanised).
    let ascii = VarValue::object(vec![
        ("street", VarValue::string(street_ascii)),
        ("city", VarValue::string(city.ascii_name())),
        ("state", VarValue::string(state.ascii_name())),
        ("country", VarValue::string(geo.country.ascii_name())),
        (
            "postcode",
            VarValue::string(fold_fullwidth_digits(&postcode_entry.code)),
        ),
    ]);

    // Top-level: native text fields + the script-neutral fields.
    let map: Vec<(&str, VarValue)> = vec![
        ("street", VarValue::string(street_native)),
        ("city", VarValue::string(&city.name)),
        ("state", VarValue::string(&state.name)),
        ("postcode", VarValue::string(&postcode_entry.code)),
        ("country", VarValue::string(&geo.country.name)),
        ("country_code", VarValue::string(&geo.country.iso_code)),
        // Approximate coordinates: the city's centre, not the street point.
        ("lat", VarValue::string(city.lat.to_string())),
        ("lon", VarValue::string(city.lon.to_string())),
        ("ascii", ascii),
    ];

    Ok(VarValue::object(map))
}

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
    use crate::seed::FakeRng;
    use crate::structured::generate_structured;
    use crate::{GeneratorDef, VarValue};
    use std::collections::HashMap;

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
        assert!(obj.contains_key("lat"), "missing 'lat'");
        assert!(obj.contains_key("lon"), "missing 'lon'");
    }

    // 4b. lat/lon are numeric strings in valid coordinate ranges.
    #[test]
    fn address_lat_lon_are_valid_coordinates() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");
        let lat: f64 = field(&result, "lat")
            .parse()
            .expect("lat SHALL be a number");
        let lon: f64 = field(&result, "lon")
            .parse()
            .expect("lon SHALL be a number");
        assert!((-90.0..=90.0).contains(&lat), "lat out of range: {lat}");
        assert!((-180.0..=180.0).contains(&lon), "lon out of range: {lon}");
    }

    // 5. country=JP: top-level country is the NATIVE name (日本); the ascii
    //    branch carries the romanisation; country_code stays the ISO code.
    #[test]
    fn address_jp_returns_japanese_data() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        assert_eq!(
            field(&result, "country"),
            "日本",
            "top-level SHALL be native"
        );
        assert_eq!(field(&result, "country_code"), "JP");
        // The ascii branch romanises the country (pure ASCII, no native script).
        let ascii_country = field(result.get_path("ascii").expect("ascii branch"), "country");
        assert!(
            ascii_country.is_ascii() && !ascii_country.is_empty(),
            "ascii.country SHALL be a non-empty romanisation, got: {ascii_country}"
        );
    }

    // 6. country=GB (Latin): native == ascii for the country name.
    #[test]
    fn address_gb_returns_british_data() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "GB")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        assert_eq!(field(&result, "country"), "United Kingdom");
        assert_eq!(field(&result, "country_code"), "GB");
        let ascii = result.get_path("ascii").expect("ascii branch");
        assert_eq!(
            field(ascii, "country"),
            "United Kingdom",
            "a Latin country folds to itself"
        );
    }

    // 7. Address fields are internally consistent (native top-level).
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

        // Top-level country is the native name; the code is the ISO code.
        assert_eq!(country, "日本");
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

    /// Extract a field from the `ascii` sub-object.
    fn ascii_field(val: &VarValue, key: &str) -> String {
        field(val.get_path("ascii").expect("ascii branch"), key)
    }

    // 22. Address with state=Tokyo constrains to Tokyo (checked on the ascii
    //     branch; top-level state is now native).
    #[test]
    fn address_jp_with_state_tokyo() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("state", "Tokyo")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate JP/Tokyo address");
        let state = ascii_field(&result, "state");
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
        assert!(result.is_err(), "SHALL error for nonexistent state");
    }

    // 25. Address with nonexistent region returns error
    #[test]
    fn address_jp_nonexistent_region_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("region", "Narnia")]);
        let result = generate_structured(&d, &mut rng);
        assert!(result.is_err(), "SHALL error for nonexistent region");
    }

    // 26. A present-but-unknown country code is an error (a typo shouldn't
    //     silently yield some other country's address); an absent country is
    //     fine and picks one at random.
    #[test]
    fn address_unknown_country_errors_unset_is_random() {
        let mut rng = seeded_rng();
        let bad = def_with_params("address", &[("country", "ZZ")]);
        let err =
            generate_structured(&bad, &mut rng).expect_err("unknown country code SHALL error");
        assert!(
            err.to_string().contains("unknown country code: ZZ"),
            "got: {err}"
        );

        // No country param → random country from the database (no error).
        let result = generate_structured(&def("address"), &mut rng).expect("unset SHALL pick one");
        let country_code = field(&result, "country_code");
        let loaded_codes = crate::geo_loader::geo_database().countries();
        assert!(
            loaded_codes.contains(&country_code.as_str()),
            "unset country SHALL pick a loaded country, got: {country_code}"
        );
    }

    // 27. State filter is case-insensitive (lowercase "tokyo" SHALL match
    //     "Tokyo" via eq_ignore_ascii_case).
    #[test]
    fn address_state_filter_is_case_insensitive() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP"), ("state", "tokyo")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL match case-insensitively");
        let state = ascii_field(&result, "state");
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
        let kanto_states: Vec<String> = crate::geo_loader::geo_database()
            .get("JP")
            .expect("JP SHALL be loaded")
            .states
            .iter()
            .filter(|s| {
                s.region_tags
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case("kanto"))
            })
            .map(|s| s.ascii_name())
            .collect();
        assert!(
            !kanto_states.is_empty(),
            "fixture SHALL contain at least one Kanto-tagged state"
        );
        let state = ascii_field(&result, "state");
        assert!(
            kanto_states.contains(&state),
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
        assert_eq!(ascii_field(&result, "state"), "Tokyo");
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
        let err =
            generate_structured(&d, &mut rng).expect_err("conflicting state+region SHALL error");
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

    use crate::geo_loader::{GeoCity, GeoCountry, GeoData, GeoPostcode, GeoState, Marker, Pattern};

    /// Build a single-state, single-city `GeoData` whose one postcode carries
    /// the supplied street-shaping fields. Lets the street-generation branches
    /// of `generate_address_from_state` be exercised in isolation, with no
    /// dependence on the embedded geo database.
    fn synthetic_geo(iso: &str, code: &str, street: &str, pattern: Option<Pattern>) -> GeoData {
        let postcode = GeoPostcode::for_test(code, street, pattern);
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
        let geo = synthetic_geo("ZZ", "12345", "Main St", None);
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate from synthetic state");

        // 1. Geographic fields come straight from the synthetic fixture.
        assert_eq!(field(&result, "city"), "Testville", "city SHALL round-trip");
        assert_eq!(field(&result, "state"), "Region", "state SHALL round-trip");
        assert_eq!(
            field(&result, "postcode"),
            "12345",
            "postcode SHALL round-trip"
        );
        assert_eq!(
            field(&result, "country"),
            "Testland",
            "country SHALL round-trip"
        );
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
        let geo = synthetic_geo("ZZ", "00000", "Cherry Lane", None);
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

    // 34. An empty pattern set falls back to the default "<num> <street>" (an
    //     empty array carries no addresses to pick from).
    #[test]
    fn from_state_empty_pattern_set_uses_default_street() {
        let mut rng = seeded_rng();
        let geo = synthetic_geo("ZZ", "00000", "Oak Road", Some(Pattern::Many(vec![])));
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate from empty pattern set");
        let street = field(&result, "street");
        assert!(
            street.ends_with(" Oak Road"),
            "empty set SHALL fall back to '<num> Oak Road', got: {street}"
        );
    }

    // 35. A pattern array (a set of known addresses) SHALL pick one of its
    //     entries as the street.
    #[test]
    fn from_state_pattern_array_picks_a_listed_entry() {
        let mut rng = seeded_rng();
        let entries = vec![
            "10 Downing Street".to_string(),
            "221B Baker Street".to_string(),
        ];
        let geo = synthetic_geo(
            "ZZ",
            "00000",
            "Ignored",
            Some(Pattern::Many(entries.clone())),
        );
        let state = &geo.states[0];
        let result = super::generate_address_from_state(&geo, state, &mut rng)
            .expect("SHALL generate from pattern array");
        let street = field(&result, "street");
        assert!(
            entries.contains(&street),
            "array street SHALL be one of the listed entries, got: {street}"
        );
    }

    // 36. A pattern with an n{min,max} token SHALL replace it with a number in
    //     range, the {street} placeholder filled with the street name.
    #[test]
    fn from_state_pattern_expands_token_and_includes_street() {
        let mut rng = seeded_rng();
        let geo = synthetic_geo(
            "ZZ",
            "00000",
            "Elm Avenue",
            Some(Pattern::One("n{1,9} {street}".to_string())),
        );
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
        let geo = synthetic_geo(
            "ZZ",
            "00000",
            "Pine Street",
            Some(Pattern::One("n{1,200} {street}".to_string())),
        );
        let state = &geo.states[0];

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();
        let a = super::generate_address_from_state(&geo, state, &mut rng1)
            .expect("SHALL generate (run 1)");
        let b = super::generate_address_from_state(&geo, state, &mut rng2)
            .expect("SHALL generate (run 2)");
        assert_eq!(
            a, b,
            "same seed + same state SHALL produce identical address"
        );
    }

    // 38a. Every real postcode in every country SHALL render an ascii street
    //      that is pure ASCII — the data test on bare names doesn't cover the
    //      rendered pattern, so a missed marker or unsubstituted native literal
    //      in a `pattern` is caught here. Deterministic: visits every postcode.
    #[test]
    fn every_postcode_renders_pure_ascii_street() {
        let mut rng = seeded_rng();
        for g in crate::geo_loader::geo_database().all() {
            let markers = &g.country.markers;
            let cc = &g.country.iso_code;
            for s in &g.states {
                for c in &s.cities {
                    for p in &c.postcodes {
                        let (_native, ascii) = super::street_pair(p, markers, &mut rng);
                        assert!(
                            ascii.is_ascii() && !ascii.is_empty(),
                            "{cc}/{}: ascii street SHALL be non-empty ASCII, got: {ascii}",
                            c.name
                        );
                    }
                }
            }
        }
    }

    // ---- native + ascii model ---------------------------------------------

    /// Parse the integer value out of a string regardless of numeral width
    /// (ASCII or full-width digits) — for comparing house numbers across scripts.
    fn number_value(s: &str) -> Option<u32> {
        let digits: String = s
            .chars()
            .filter_map(|c| match c as u32 {
                0x30..=0x39 => Some(c),
                0xFF10..=0xFF19 => char::from_u32(c as u32 - 0xFF10 + 0x30),
                _ => None,
            })
            .collect();
        digits.parse().ok()
    }

    /// Each maximal run of digits (ASCII or full-width) as its integer value, in
    /// order — for comparing the SEQUENCE of house numbers across scripts.
    fn digit_values(s: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for c in s.chars() {
            match c as u32 {
                0x30..=0x39 => cur.push(c),
                0xFF10..=0xFF19 => cur.push(char::from(b'0' + (c as u32 - 0xFF10) as u8)),
                _ => {
                    if let Ok(v) = cur.parse() {
                        out.push(v);
                    }
                    cur.clear();
                }
            }
        }
        if let Ok(v) = cur.parse() {
            out.push(v);
        }
        out
    }

    // 38. The object is NATIVE at top level with an `ascii` sub-object that
    //     carries only the script-variant text fields. The script-neutral
    //     fields live top-level ONLY (not duplicated into `ascii`).
    #[test]
    fn address_native_top_level_with_ascii_text_branch() {
        let mut rng = seeded_rng();
        let d = def_with_params("address", &[("country", "JP")]);
        let r = generate_structured(&d, &mut rng).expect("should generate");

        let ascii = r
            .get_path("ascii")
            .expect("ascii branch")
            .as_object()
            .expect("value SHALL be present");
        for k in ["street", "city", "state", "country", "postcode"] {
            assert!(ascii.contains_key(k), "ascii SHALL carry text field {k}");
        }
        for k in ["country_code", "lat", "lon"] {
            assert!(
                !ascii.contains_key(k),
                "ascii SHALL NOT duplicate the script-neutral field {k}"
            );
            // …which is present at the top level.
            assert!(!field(&r, k).is_empty(), "top-level SHALL carry {k}");
        }

        // Top-level JP city is native (non-ASCII); the ascii branch romanises it.
        let city = field(&r, "city");
        assert!(
            !city.is_ascii(),
            "JP top-level city SHALL be native: {city}"
        );
        let acity = ascii_field(&r, "city");
        assert!(
            acity.is_ascii() && !acity.is_empty(),
            "ascii.city SHALL be a non-empty romanisation: {acity}"
        );
    }

    // 39. A Latin country (GB) folds to itself: native == ascii for every text
    //     field (the fold is identity / no stored romanisation needed).
    #[test]
    fn address_latin_native_equals_ascii() {
        let mut rng = seeded_rng();
        let r = generate_structured(&def_with_params("address", &[("country", "GB")]), &mut rng)
            .expect("should generate");
        for k in ["street", "city", "state", "country", "postcode"] {
            assert_eq!(
                field(&r, k),
                ascii_field(&r, k),
                "GB is Latin: native SHALL equal ascii for {k}"
            );
        }
    }

    /// A two-marker country map (jp-style 丁目/番) for the placeholder tests.
    fn jp_markers() -> HashMap<String, Marker> {
        HashMap::from([
            (
                "chome".to_string(),
                Marker {
                    native: "丁目".to_string(),
                    ascii: "-chome ".to_string(),
                },
            ),
            (
                "ban".to_string(),
                Marker {
                    native: "番".to_string(),
                    ascii: "-ban".to_string(),
                },
            ),
        ])
    }

    // 40. street_pair shares the drawn house number(s) across scripts and
    //     romanises every marker: a multi-part {street}/{marker} skeleton renders
    //     full-width native and pure-ASCII ascii from the SAME numbers.
    #[test]
    fn street_pair_shares_numbers_and_romanises_markers() {
        let mut rng = seeded_rng();
        let markers = jp_markers();
        // Digit-free romanisation so the only digits are the house numbers.
        let pc = GeoPostcode {
            code: "060-0001".to_string(),
            street: "本町".to_string(),
            street_ascii: "Honmachi".to_string(),
            pattern: Some(Pattern::One(
                "{street}n{１,４}{chome}n{１,１５}{ban}".to_string(),
            )),
        };
        let (native, ascii) = super::street_pair(&pc, &markers, &mut rng);

        // Native keeps script + native markers, full-width digits only.
        assert!(native.starts_with("本町"), "native keeps script: {native}");
        assert!(
            native.contains("丁目") && native.contains("番"),
            "native markers: {native}"
        );
        assert!(
            !native.chars().any(|c| c.is_ascii_digit()),
            "native house numbers SHALL be full-width: {native}"
        );
        // ASCII romanises both markers, is pure ASCII, tidy spacing.
        assert!(
            ascii.is_ascii(),
            "ascii street SHALL be pure ASCII: {ascii}"
        );
        assert!(
            ascii.contains("Honmachi") && ascii.contains("-chome") && ascii.contains("-ban"),
            "ascii romanises street + markers: {ascii}"
        );
        assert!(
            !ascii.contains("Honmachi4"),
            "letter→digit join SHALL be spaced: {ascii}"
        );
        // Both drawn numbers match across scripts (full-width vs ASCII).
        assert_eq!(
            digit_values(&native),
            digit_values(&ascii),
            "native and ascii SHALL share the same numbers ({native} / {ascii})"
        );
    }

    // 41. A Latin street with diacritics folds programmatically (no markers, no
    //     stored romanisation): ascii = the ASCII fold of the native skeleton,
    //     word order preserved, same house number.
    #[test]
    fn street_pair_folds_diacritics_programmatically() {
        let mut rng = seeded_rng();
        let markers: HashMap<String, Marker> = HashMap::new();
        let pc = GeoPostcode {
            code: "70173".to_string(),
            street: "Königstraße".to_string(),
            street_ascii: String::new(), // nothing stored — derive by fold
            pattern: Some(Pattern::One("{street} n{1,120}".to_string())),
        };
        let (native, ascii) = super::street_pair(&pc, &markers, &mut rng);

        assert!(native.starts_with("Königstraße"), "native: {native}");
        assert!(
            ascii.starts_with("Konigstrasse"),
            "diacritics SHALL fold programmatically (ß→ss, ö→o): {ascii}"
        );
        assert!(ascii.is_ascii(), "folded street SHALL be ASCII: {ascii}");
        assert_eq!(
            number_value(&native),
            number_value(&ascii),
            "fold SHALL preserve the same house number"
        );
    }
}
