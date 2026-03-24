use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{bail, Result};
use rand::Rng;
use serde::Deserialize;

use crate::geo_loader::{geo_database, GeoData};
use crate::{GeneratorDef, VarValue};

// ---------------------------------------------------------------------------
// Compile-time data loading
// ---------------------------------------------------------------------------

static NAMES_JSON: &str = include_str!("../../data/names.json");

// ---------------------------------------------------------------------------
// Data models for deserialisation
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct NamesData {
    first_names: Vec<NameEntry>,
    last_names: Vec<NameEntry>,
}

#[derive(Deserialize, Clone)]
struct NameEntry {
    #[allow(dead_code)]
    name: String,
    name_en: String,
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

/// Countries that use family-name-first ordering.
fn is_family_first(country: &str) -> bool {
    matches!(country, "JP" | "CN" | "KR")
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Generate a structured (Object) value from a generator definition.
///
/// Supported generators: `person`, `address`, `credit_card`.
pub fn generate_structured(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue> {
    match def.name.as_str() {
        "person" => generate_person(&def.params, rng),
        "address" => generate_address(&def.params, rng),
        "credit_card" => generate_credit_card(&def.params, rng),
        _ => bail!("Unknown structured generator: {}", def.name),
    }
}

// ---------------------------------------------------------------------------
// Person generator
// ---------------------------------------------------------------------------

fn generate_person(params: &HashMap<String, String>, rng: &mut impl Rng) -> Result<VarValue> {
    let data = names_data();
    let country = params.get("country").map(|s| s.as_str());

    // Pick first and last names from the global pool.
    let fi = rng.gen_range(0..data.first_names.len());
    let li = rng.gen_range(0..data.last_names.len());
    let first = data.first_names[fi].name_en.clone();
    let last = data.last_names[li].name_en.clone();

    // Format full name depending on culture.
    let full_name = if country.is_some_and(is_family_first) {
        format!("{last} {first}")
    } else {
        format!("{first} {last}")
    };

    // Derive email from name components.
    let email = format!(
        "{}.{}@example.com",
        first.to_lowercase(),
        last.to_lowercase()
    );

    // Generate phone number.
    let phone = generate_phone(country, rng);

    let map: Vec<(&str, VarValue)> = vec![
        ("first", VarValue::string(&first)),
        ("last", VarValue::string(&last)),
        ("name", VarValue::string(&full_name)),
        ("email", VarValue::string(&email)),
        ("phone", VarValue::string(&phone)),
    ];

    Ok(VarValue::object(map))
}

/// Generate a phone number. Uses geo data phone_formats when available.
fn generate_phone(country: Option<&str>, rng: &mut impl Rng) -> String {
    let geo = country.and_then(|c| geo_database().get(c));

    match geo {
        Some(g) if !g.country.phone_formats.is_empty() => {
            let fmt_idx = rng.gen_range(0..g.country.phone_formats.len());
            let fmt = &g.country.phone_formats[fmt_idx];
            expand_phone_format(fmt, rng)
        }
        _ => {
            // Default US-style phone
            let area: u32 = rng.gen_range(200..999);
            let exchange: u32 = rng.gen_range(200..999);
            let subscriber: u32 = rng.gen_range(1000..9999);
            format!("+1-{area}-{exchange}-{subscriber}")
        }
    }
}

/// Replace '#' characters in a phone format with random digits.
fn expand_phone_format(fmt: &str, rng: &mut impl Rng) -> String {
    fmt.chars()
        .map(|c| {
            if c == '#' {
                char::from(b'0' + rng.gen_range(0..10u8))
            } else {
                c
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Address generator
// ---------------------------------------------------------------------------

fn generate_address(params: &HashMap<String, String>, rng: &mut impl Rng) -> Result<VarValue> {
    let country = params.get("country").map(|s| s.as_str());

    let geo = country.and_then(|c| geo_database().get(c));

    match geo {
        Some(g) => generate_address_from_geo(g, rng),
        None => generate_default_address(rng),
    }
}

fn generate_address_from_geo(geo: &GeoData, rng: &mut impl Rng) -> Result<VarValue> {
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

/// Expand a street pattern like "n{1,221} Baker Street" or "北一条西n{1,20}".
/// The `n{min,max}` is replaced with a random number in that range.
/// If expansion fails, we fall back to a simple format.
fn expand_street_pattern(pattern: &str, street_en: &str, rng: &mut impl Rng) -> String {
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

fn generate_default_address(rng: &mut impl Rng) -> Result<VarValue> {
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

// ---------------------------------------------------------------------------
// Credit card generator
// ---------------------------------------------------------------------------

/// Card brand metadata.
struct CardBrand {
    name: &'static str,
    prefix: &'static str,
    length: usize,
    cvv_len: usize,
}

const CARD_BRANDS: &[CardBrand] = &[
    CardBrand {
        name: "Visa",
        prefix: "4",
        length: 16,
        cvv_len: 3,
    },
    CardBrand {
        name: "Mastercard",
        prefix: "51",
        length: 16,
        cvv_len: 3,
    },
    CardBrand {
        name: "Amex",
        prefix: "34",
        length: 15,
        cvv_len: 4,
    },
    CardBrand {
        name: "Discover",
        prefix: "6011",
        length: 16,
        cvv_len: 3,
    },
];

fn generate_credit_card(
    _params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let brand_idx = rng.gen_range(0..CARD_BRANDS.len());
    let brand = &CARD_BRANDS[brand_idx];

    let number = generate_luhn_number(brand.prefix, brand.length, rng);

    // Generate expiry: random month, 1-5 years in the future from a reference year (2026).
    let month: u32 = rng.gen_range(1..=12);
    let year: u32 = rng.gen_range(27..=31);
    let expiry = format!("{month:02}/{year}");

    // Generate CVV.
    let cvv: String = (0..brand.cvv_len)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect();

    let map: Vec<(&str, VarValue)> = vec![
        ("number", VarValue::string(&number)),
        ("expiry", VarValue::string(&expiry)),
        ("cvv", VarValue::string(&cvv)),
        ("brand", VarValue::string(brand.name)),
    ];

    Ok(VarValue::object(map))
}

/// Generate a Luhn-valid credit card number with the given prefix and total length.
fn generate_luhn_number(prefix: &str, length: usize, rng: &mut impl Rng) -> String {
    // Start with the prefix digits.
    let mut digits: Vec<u8> = prefix
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as u8))
        .collect();

    // Fill random digits up to length - 1 (last digit is the check digit).
    while digits.len() < length - 1 {
        digits.push(rng.gen_range(0..10));
    }

    // Compute the Luhn check digit.
    let check = luhn_check_digit(&digits);
    digits.push(check);

    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// Compute the Luhn check digit for a sequence of digits (the check digit will
/// be appended at the end to make the full number valid).
fn luhn_check_digit(digits: &[u8]) -> u8 {
    let mut sum: u32 = 0;

    for (i, &d) in digits.iter().rev().enumerate() {
        // Position from the right: the check digit is position 0 (even),
        // so the last provided digit is position 1 (odd), etc.
        let pos_from_right = i + 1;
        if pos_from_right % 2 == 1 {
            // Odd position from right (with check digit at 0): double it
            let doubled = (d as u32) * 2;
            sum += if doubled > 9 { doubled - 9 } else { doubled };
        } else {
            sum += d as u32;
        }
    }

    ((10 - (sum % 10)) % 10) as u8
}

/// Validate a number string passes the Luhn check.
#[cfg(test)]
fn luhn_valid(number: &str) -> bool {
    let digits: Vec<u8> = number
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as u8))
        .collect();

    if digits.is_empty() {
        return false;
    }

    let mut sum: u32 = 0;
    for (i, &d) in digits.iter().rev().enumerate() {
        if i % 2 == 1 {
            let doubled = (d as u32) * 2;
            sum += if doubled > 9 { doubled - 9 } else { doubled };
        } else {
            sum += d as u32;
        }
    }

    sum % 10 == 0
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

    // 1. Person produces all expected fields
    #[test]
    fn person_produces_all_fields() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("person"), &mut rng).expect("should generate");
        let obj = result.as_object().expect("should be object");
        assert!(obj.contains_key("first"), "missing 'first'");
        assert!(obj.contains_key("last"), "missing 'last'");
        assert!(obj.contains_key("name"), "missing 'name'");
        assert!(obj.contains_key("email"), "missing 'email'");
        assert!(obj.contains_key("phone"), "missing 'phone'");
    }

    // 2. Person with country=JP uses family_first name order
    #[test]
    fn person_jp_uses_family_first_order() {
        let mut rng = seeded_rng();
        let d = def_with_params("person", &[("country", "JP")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let first = field(&result, "first");
        let last = field(&result, "last");
        let name = field(&result, "name");

        // Family first means "Last First"
        assert_eq!(
            name,
            format!("{last} {first}"),
            "JP name should be in family_first order"
        );
    }

    // 3. Person name and email are consistent
    #[test]
    fn person_name_email_consistent() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("person"), &mut rng).expect("should generate");

        let first = field(&result, "first");
        let last = field(&result, "last");
        let email = field(&result, "email");

        let expected_email = format!(
            "{}.{}@example.com",
            first.to_lowercase(),
            last.to_lowercase()
        );
        assert_eq!(email, expected_email, "email should derive from name");
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

    // 8. Credit card produces all expected fields
    #[test]
    fn credit_card_produces_all_fields() {
        let mut rng = seeded_rng();
        let result =
            generate_structured(&def("credit_card"), &mut rng).expect("should generate");
        let obj = result.as_object().expect("should be object");
        assert!(obj.contains_key("number"), "missing 'number'");
        assert!(obj.contains_key("expiry"), "missing 'expiry'");
        assert!(obj.contains_key("cvv"), "missing 'cvv'");
        assert!(obj.contains_key("brand"), "missing 'brand'");
    }

    // 9. Credit card CVV length matches brand (3 or 4)
    #[test]
    fn credit_card_cvv_length_matches_brand() {
        // Generate many cards to hit different brands.
        for seed in 0u64..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let brand = field(&result, "brand");
            let cvv = field(&result, "cvv");

            let expected_cvv_len = if brand == "Amex" { 4 } else { 3 };
            assert_eq!(
                cvv.len(),
                expected_cvv_len,
                "seed={seed}: brand={brand} should have CVV length {expected_cvv_len}, got {}",
                cvv.len()
            );
        }
    }

    // 10. Deterministic seed produces same output
    #[test]
    fn deterministic_seed_same_output() {
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let person1 = generate_structured(&def("person"), &mut rng1).expect("should generate");
        let person2 = generate_structured(&def("person"), &mut rng2).expect("should generate");
        assert_eq!(person1, person2, "same seed should produce same person");

        // Reset RNGs for address test.
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let addr1 = generate_structured(&def("address"), &mut rng1).expect("should generate");
        let addr2 = generate_structured(&def("address"), &mut rng2).expect("should generate");
        assert_eq!(addr1, addr2, "same seed should produce same address");

        // Reset RNGs for credit card test.
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let cc1 =
            generate_structured(&def("credit_card"), &mut rng1).expect("should generate");
        let cc2 =
            generate_structured(&def("credit_card"), &mut rng2).expect("should generate");
        assert_eq!(
            cc1, cc2,
            "same seed should produce same credit card"
        );
    }

    // 11. Unknown type returns error
    #[test]
    fn unknown_type_returns_error() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("nonexistent"), &mut rng);
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        assert!(
            err.to_string().contains("Unknown structured generator"),
            "expected 'Unknown structured generator' error, got: {err}"
        );
    }

    // 12. Credit card number is Luhn-valid
    #[test]
    fn credit_card_number_is_luhn_valid() {
        for seed in 0u64..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let number = field(&result, "number");
            assert!(
                luhn_valid(&number),
                "seed={seed}: card number {number} should pass Luhn check"
            );
        }
    }

    // 13. Credit card number has correct length for its brand
    #[test]
    fn credit_card_number_correct_length() {
        for seed in 0u64..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let brand = field(&result, "brand");
            let number = field(&result, "number");

            let expected_len = if brand == "Amex" { 15 } else { 16 };
            assert_eq!(
                number.len(),
                expected_len,
                "seed={seed}: brand={brand} number should have length {expected_len}, got {}",
                number.len()
            );
        }
    }

    // 14. Person with country=JP picks from the global name pool
    #[test]
    fn person_jp_picks_from_global_pool() {
        let all_first: Vec<&str> = names_data()
            .first_names
            .iter()
            .map(|n| n.name_en.as_str())
            .collect();
        let all_last: Vec<&str> = names_data()
            .last_names
            .iter()
            .map(|n| n.name_en.as_str())
            .collect();

        for seed in 0u64..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let d = def_with_params("person", &[("country", "JP")]);
            let result = generate_structured(&d, &mut rng).expect("should generate");
            let first = field(&result, "first");
            let last = field(&result, "last");

            assert!(
                all_first.contains(&first.as_str()),
                "seed={seed}: first name '{first}' SHALL be in global pool"
            );
            assert!(
                all_last.contains(&last.as_str()),
                "seed={seed}: last name '{last}' SHALL be in global pool"
            );
        }
    }

    // 15. Person phone has country prefix for JP
    #[test]
    fn person_jp_phone_has_correct_prefix() {
        let mut rng = seeded_rng();
        let d = def_with_params("person", &[("country", "JP")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");
        let phone = field(&result, "phone");
        assert!(
            phone.starts_with("+81"),
            "JP phone should start with +81, got: {phone}"
        );
    }

    // 16. Default person phone is US-style
    #[test]
    fn person_default_phone_us_style() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("person"), &mut rng).expect("should generate");
        let phone = field(&result, "phone");
        assert!(
            phone.starts_with("+1-"),
            "Default phone should start with +1-, got: {phone}"
        );
    }

    // 17. Credit card expiry format is MM/YY
    #[test]
    fn credit_card_expiry_format() {
        let mut rng = seeded_rng();
        let result =
            generate_structured(&def("credit_card"), &mut rng).expect("should generate");
        let expiry = field(&result, "expiry");
        assert_eq!(expiry.len(), 5, "expiry should be 5 chars (MM/YY)");
        assert_eq!(
            expiry.chars().nth(2),
            Some('/'),
            "expiry should have / at position 2"
        );
        let month: u32 = expiry[..2]
            .parse()
            .expect("month part should parse as u32");
        assert!((1..=12).contains(&month), "month should be 1-12, got {month}");
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

    // 19. Person with country=CN uses family_first order
    #[test]
    fn person_cn_uses_family_first_order() {
        let mut rng = seeded_rng();
        let d = def_with_params("person", &[("country", "CN")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let first = field(&result, "first");
        let last = field(&result, "last");
        let name = field(&result, "name");

        assert_eq!(
            name,
            format!("{last} {first}"),
            "CN name should be in family_first order"
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

    // 22. Person with country=KR phone starts with +82 (Issue 9)
    #[test]
    fn person_kr_phone_has_correct_prefix() {
        let mut rng = seeded_rng();
        let d = def_with_params("person", &[("country", "KR")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate KR person");
        let phone = field(&result, "phone");
        assert!(
            phone.starts_with("+82"),
            "SHALL start KR phone with +82, got: {phone}"
        );
    }

    // 23. first_name pool diversity: >20 unique names from 100 draws (Issue 13)
    #[test]
    fn person_name_pool_diversity() {
        let mut unique_first: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for seed in 0u64..100 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("person"), &mut rng).expect("SHALL generate person");
            unique_first.insert(field(&result, "first"));
        }
        assert!(
            unique_first.len() > 20,
            "SHALL draw from pool >20 unique first names in 100 draws, got {}",
            unique_first.len()
        );
    }
}
