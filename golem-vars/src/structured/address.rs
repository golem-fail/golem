use std::collections::HashMap;

use anyhow::Result;
use rand::Rng;

use crate::geo_loader::{geo_database, GeoData};
use crate::VarValue;

// ---------------------------------------------------------------------------
// Address generator
// ---------------------------------------------------------------------------

pub(crate) fn generate_address(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let country = params.get("country").map(|s| s.as_str());

    let geo = country.and_then(|c| geo_database().get(c));

    match geo {
        Some(g) => generate_address_from_geo(g, rng),
        None => generate_default_address(rng),
    }
}

pub(crate) fn generate_address_from_geo(
    geo: &GeoData,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    // Pick a random state.
    let state_idx = rng.gen_range(0..geo.states.len());
    let state = &geo.states[state_idx];

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

/// Expand a street pattern like "n{1,221} Baker Street" or "north-one-west n{1,20}".
/// The `n{min,max}` is replaced with a random number in that range.
/// If expansion fails, we fall back to a simple format.
pub(crate) fn expand_street_pattern(
    pattern: &str,
    street_en: &str,
    rng: &mut impl Rng,
) -> String {
    // Look for the n{min,max} pattern.
    if let Some(start) = pattern.find("n{") {
        if let Some(end) = pattern[start..].find('}') {
            let range_str = &pattern[start + 2..start + end];
            if let Some((min_s, max_s)) = range_str.split_once(',') {
                if let (Ok(min), Ok(max)) = (min_s.parse::<u32>(), max_s.parse::<u32>()) {
                    let num = rng.gen_range(min..=max);
                    // Replace the pattern portion with the number.
                    let prefix = &pattern[..start];
                    let suffix = &pattern[start + end + 1..];
                    let expanded = format!("{prefix}{num}{suffix}");
                    // If the expanded text still has the original street_en, return as-is.
                    // Otherwise return "num street_en" style.
                    if expanded.contains(street_en) || street_en.is_empty() {
                        return expanded;
                    }
                    return format!("{num} {street_en}");
                }
            }
        }
    }

    // Fallback: random number + street name.
    let num: u32 = rng.gen_range(1..200);
    format!("{num} {street_en}")
}

pub(crate) fn generate_default_address(rng: &mut impl Rng) -> Result<VarValue> {
    const STREETS: &[&str] = &[
        "Main Street",
        "Oak Avenue",
        "Elm Drive",
        "Maple Lane",
        "Cedar Boulevard",
        "Pine Road",
        "Birch Way",
        "Walnut Court",
        "Willow Place",
        "Spruce Terrace",
    ];
    const CITIES: &[&str] = &[
        "Springfield",
        "Riverside",
        "Fairview",
        "Madison",
        "Georgetown",
        "Clinton",
        "Salem",
        "Franklin",
        "Arlington",
        "Chester",
    ];
    const STATES: &[&str] = &[
        "California",
        "Texas",
        "New York",
        "Florida",
        "Illinois",
        "Pennsylvania",
        "Ohio",
        "Georgia",
        "Michigan",
        "Virginia",
    ];

    let street_num: u32 = rng.gen_range(1..9999);
    let street_name = STREETS[rng.gen_range(0..STREETS.len())];
    let city = CITIES[rng.gen_range(0..CITIES.len())];
    let state = STATES[rng.gen_range(0..STATES.len())];
    let zip: u32 = rng.gen_range(10000..99999);

    let map: Vec<(&str, VarValue)> = vec![
        (
            "street",
            VarValue::string(format!("{street_num} {street_name}")),
        ),
        ("city", VarValue::string(city)),
        ("state", VarValue::string(state)),
        ("postcode", VarValue::string(format!("{zip}"))),
        ("country", VarValue::string("United States")),
        ("country_code", VarValue::string("US")),
    ];

    Ok(VarValue::object(map))
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
        assert_eq!(addr1, addr2, "same seed should produce same address");
    }

    // 18. Default address returns US data
    #[test]
    fn default_address_returns_us() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("address"), &mut rng).expect("should generate");
        let country = field(&result, "country");
        let country_code = field(&result, "country_code");
        assert_eq!(country, "United States");
        assert_eq!(country_code, "US");
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
}
