use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::Result;
use rand::Rng;
use serde::Deserialize;

use crate::geo_loader::geo_database;
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
pub(crate) fn is_family_first(country: &str) -> bool {
    matches!(country, "JP" | "CN" | "KR")
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
/// When no country is specified (or the country has no formats), a random
/// country from the geo database is chosen.
pub(crate) fn generate_phone(country: Option<&str>, rng: &mut impl Rng) -> String {
    let geo = country.and_then(|c| geo_database().get(c));

    let geo = match geo {
        Some(g) if !g.country.phone_formats.is_empty() => g,
        _ => geo_database().random(rng),
    };

    let fmt_idx = rng.gen_range(0..geo.country.phone_formats.len());
    let fmt = &geo.country.phone_formats[fmt_idx];
    expand_phone_format(fmt, rng)
}

/// Replace '#' characters in a phone format with random digits.
pub(crate) fn expand_phone_format(fmt: &str, rng: &mut impl Rng) -> String {
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structured::generate_structured;
    use crate::GeneratorDef;
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
        assert_eq!(email, expected_email, "email SHALL derive from name");
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

    // 16. Default person phone uses a random geo country (not hardcoded US)
    #[test]
    fn person_default_phone_uses_geo_data() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("person"), &mut rng).expect("should generate");
        let phone = field(&result, "phone");
        assert!(
            phone.starts_with('+'),
            "Default phone SHALL start with '+', got: {phone}"
        );
        assert!(
            phone.len() > 5,
            "Default phone SHALL be a plausible length, got: {phone}"
        );
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

    // 24. is_family_first SHALL be true only for JP, CN, KR
    #[test]
    fn is_family_first_true_for_known_countries() {
        assert!(is_family_first("JP"), "JP SHALL be family-first");
        assert!(is_family_first("CN"), "CN SHALL be family-first");
        assert!(is_family_first("KR"), "KR SHALL be family-first");
    }

    // 25. is_family_first SHALL be false for other / empty / unknown countries
    #[test]
    fn is_family_first_false_for_other_countries() {
        assert!(!is_family_first("US"), "US SHALL NOT be family-first");
        assert!(!is_family_first("GB"), "GB SHALL NOT be family-first");
        assert!(!is_family_first(""), "empty SHALL NOT be family-first");
    }

    // 26. is_family_first SHALL be case-sensitive (lowercase does not match)
    #[test]
    fn is_family_first_is_case_sensitive() {
        assert!(
            !is_family_first("jp"),
            "lowercase 'jp' SHALL NOT be family-first"
        );
        assert!(
            !is_family_first("Jp"),
            "mixed-case 'Jp' SHALL NOT be family-first"
        );
    }

    // 27. Non-family-first country SHALL render name as "First Last"
    #[test]
    fn person_us_uses_first_last_order() {
        let mut rng = seeded_rng();
        let d = def_with_params("person", &[("country", "US")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let first = field(&result, "first");
        let last = field(&result, "last");
        let name = field(&result, "name");

        assert_eq!(
            name,
            format!("{first} {last}"),
            "US name SHALL be in First Last order"
        );
    }

    // 28. KR person SHALL render name in family-first ("Last First") order
    #[test]
    fn person_kr_uses_family_first_order() {
        let mut rng = seeded_rng();
        let d = def_with_params("person", &[("country", "KR")]);
        let result = generate_structured(&d, &mut rng).expect("should generate");

        let first = field(&result, "first");
        let last = field(&result, "last");
        let name = field(&result, "name");

        // Pin the seed-42 components so the assertion is not circular with the
        // production format string: the same draw rendered first-last would be
        // "Dolores Adeyemi", but family-first SHALL invert it to "Adeyemi Dolores".
        assert_eq!(first, "Dolores", "seed-42 KR first SHALL be stable");
        assert_eq!(last, "Adeyemi", "seed-42 KR last SHALL be stable");
        assert_eq!(
            name, "Adeyemi Dolores",
            "KR name SHALL be family-first (Last First), not 'Dolores Adeyemi'"
        );
    }

    // 29. expand_phone_format SHALL replace every '#' with a decimal digit
    #[test]
    fn expand_phone_format_replaces_hash_with_digits() {
        let mut rng = seeded_rng();
        let out = expand_phone_format("###", &mut rng);
        assert_eq!(out.len(), 3, "length SHALL be preserved");
        assert!(
            out.chars().all(|c| c.is_ascii_digit()),
            "all '#' SHALL become digits, got: {out}"
        );
    }

    // 30. expand_phone_format SHALL preserve non-'#' characters verbatim
    #[test]
    fn expand_phone_format_preserves_literals() {
        let mut rng = seeded_rng();
        let out = expand_phone_format("+1 (###) ###-####", &mut rng);
        assert!(out.starts_with("+1 ("), "literal prefix SHALL be preserved");
        assert!(out.contains(") "), "literal separators SHALL be preserved");
        assert!(out.contains('-'), "literal dash SHALL be preserved");
        // Every original '#' position is now a digit; literals unchanged.
        assert_eq!(
            out.chars().filter(|c| c.is_ascii_digit()).count(),
            1 + 3 + 3 + 4,
            "digit count SHALL equal literal digits plus expanded '#', got: {out}"
        );
    }

    // 31. expand_phone_format on a format with no '#' SHALL return it unchanged
    #[test]
    fn expand_phone_format_no_hash_is_identity() {
        let mut rng = seeded_rng();
        let out = expand_phone_format("+44-FIXED", &mut rng);
        assert_eq!(out, "+44-FIXED", "no-'#' format SHALL pass through unchanged");
    }

    // 32. expand_phone_format on an empty format SHALL return an empty string
    #[test]
    fn expand_phone_format_empty_is_empty() {
        let mut rng = seeded_rng();
        let out = expand_phone_format("", &mut rng);
        assert_eq!(out, "", "empty format SHALL produce empty string");
    }

    // 33. generate_phone with None country SHALL fall back to a random geo country
    #[test]
    fn generate_phone_none_country_uses_random_geo() {
        let mut rng = seeded_rng();
        let phone = generate_phone(None, &mut rng);
        assert!(
            phone.starts_with('+'),
            "fallback phone SHALL start with '+', got: {phone}"
        );
        assert!(
            phone.len() > 5,
            "fallback phone SHALL be plausibly long, got: {phone}"
        );
    }

    // 34. generate_phone for JP SHALL use the JP +81 format
    #[test]
    fn generate_phone_jp_uses_jp_format() {
        let mut rng = seeded_rng();
        let phone = generate_phone(Some("JP"), &mut rng);
        assert!(
            phone.starts_with("+81"),
            "JP phone SHALL start with +81, got: {phone}"
        );
    }

    // 35. generate_phone for an unknown country SHALL fall back to random geo
    #[test]
    fn generate_phone_unknown_country_falls_back() {
        let mut rng = seeded_rng();
        let phone = generate_phone(Some("ZZ"), &mut rng);
        assert!(
            phone.starts_with('+'),
            "unknown-country phone SHALL fall back to a '+' format, got: {phone}"
        );
        assert!(
            phone.len() > 5,
            "unknown-country phone SHALL be plausibly long, got: {phone}"
        );
    }

    // 36. generate_phone SHALL be deterministic for a fixed seed and country
    #[test]
    fn generate_phone_deterministic() {
        let mut rng1 = ChaCha8Rng::seed_from_u64(7);
        let mut rng2 = ChaCha8Rng::seed_from_u64(7);
        let p1 = generate_phone(Some("JP"), &mut rng1);
        let p2 = generate_phone(Some("JP"), &mut rng2);
        assert_eq!(p1, p2, "same seed+country SHALL produce same phone");
    }
}
