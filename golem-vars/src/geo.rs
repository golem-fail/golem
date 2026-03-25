//! Geo-aware generators for phone, city, postcode, and street.
//!
//! These produce scalar `VarValue::String` results and are wired into
//! `generate_simple()` via the `phone`, `city`, `postcode`, and `street`
//! generator names.

use std::collections::HashMap;

use rand::Rng;

use crate::geo_loader::{geo_database, GeoData, GeoPostcode};
use crate::{VarError, VarValue};

// ---------------------------------------------------------------------------
// Country resolution
// ---------------------------------------------------------------------------

/// Resolve an optional country param to a `&GeoData`, or `None` if unset/unknown.
fn resolve_geo(params: &HashMap<String, String>) -> Option<&'static GeoData> {
    let code = params.get("country")?;
    geo_database().get(code)
}

// ---------------------------------------------------------------------------
// Phone generator
// ---------------------------------------------------------------------------

/// Generate a phone number string.
///
/// Params:
/// - `country`: "JP" | "GB" etc. (default: random from geo database)
/// - `format`: custom format where `#` is replaced by a random digit
pub fn generate_phone(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue, VarError> {
    // Custom format takes precedence.
    if let Some(fmt) = params.get("format") {
        return Ok(VarValue::String(expand_format(fmt, rng)));
    }

    let geo = resolve_geo(params);
    let geo = match geo {
        Some(g) if !g.country.phone_formats.is_empty() => g,
        _ => geo_database().random(rng),
    };

    let idx = rng.gen_range(0..geo.country.phone_formats.len());
    let fmt = &geo.country.phone_formats[idx];
    Ok(VarValue::String(expand_format(fmt, rng)))
}

// ---------------------------------------------------------------------------
// City generator
// ---------------------------------------------------------------------------

/// Generate a city name.
///
/// Params:
/// - `country`: "JP" | "GB" etc. (default: picks from any loaded geo data)
/// - `region`: narrows city pool to states whose `region_tags` contain this value
pub fn generate_city(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue, VarError> {
    let geo = resolve_geo(params);
    let region = params.get("region").map(|s| s.as_str());

    let cities = collect_city_names(geo, region);

    if cities.is_empty() {
        return Err(VarError::Other(
            "no cities found for the given country/region combination".to_string(),
        ));
    }

    let idx = rng.gen_range(0..cities.len());
    Ok(VarValue::String(cities[idx].clone()))
}

/// Collect city names, optionally filtered by geo data and region tag.
fn collect_city_names(geo: Option<&GeoData>, region: Option<&str>) -> Vec<String> {
    match geo {
        Some(g) => {
            let mut names = Vec::new();
            for state in &g.states {
                if let Some(r) = region {
                    if !state.region_tags.iter().any(|t| t.eq_ignore_ascii_case(r)) {
                        continue;
                    }
                }
                for city in &state.cities {
                    names.push(city.name_en.clone());
                }
            }
            names
        }
        None => {
            // No country specified: pull from all loaded geo data.
            let mut names = Vec::new();
            for g in geo_database().all() {
                for state in &g.states {
                    if let Some(r) = region {
                        if !state.region_tags.iter().any(|t| t.eq_ignore_ascii_case(r)) {
                            continue;
                        }
                    }
                    for city in &state.cities {
                        names.push(city.name_en.clone());
                    }
                }
            }
            names
        }
    }
}

// ---------------------------------------------------------------------------
// Postcode generator
// ---------------------------------------------------------------------------

/// Generate a postcode string.
///
/// Params:
/// - `country`: "JP" | "GB" etc. (default: picks from any loaded geo data)
pub fn generate_postcode(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue, VarError> {
    let geo = resolve_geo(params);

    let postcodes = collect_postcodes(geo);

    if postcodes.is_empty() {
        return Err(VarError::Other(
            "no postcodes found for the given country".to_string(),
        ));
    }

    let idx = rng.gen_range(0..postcodes.len());
    Ok(VarValue::String(postcodes[idx].clone()))
}

/// Collect all postcode codes from geo data.
fn collect_postcodes(geo: Option<&GeoData>) -> Vec<String> {
    match geo {
        Some(g) => {
            let mut codes = Vec::new();
            for state in &g.states {
                for city in &state.cities {
                    for pc in &city.postcodes {
                        codes.push(pc.code.clone());
                    }
                }
            }
            codes
        }
        None => {
            let mut codes = Vec::new();
            for g in geo_database().all() {
                for state in &g.states {
                    for city in &state.cities {
                        for pc in &city.postcodes {
                            codes.push(pc.code.clone());
                        }
                    }
                }
            }
            codes
        }
    }
}

// ---------------------------------------------------------------------------
// Street generator
// ---------------------------------------------------------------------------

/// Generate a street address string.
///
/// Params:
/// - `country`: "JP" | "GB" etc. (default: random from geo database)
pub fn generate_street(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue, VarError> {
    let geo = resolve_geo(params);
    let geo = match geo {
        Some(g) => g,
        None => geo_database().random(rng),
    };

    let entry = pick_random_postcode_entry(geo, rng)?;
    let street = expand_street_from_entry(entry, rng);
    Ok(VarValue::String(street))
}

/// Pick a random postcode entry from a `GeoData`.
fn pick_random_postcode_entry<'a>(
    geo: &'a GeoData,
    rng: &mut impl Rng,
) -> Result<&'a GeoPostcode, VarError> {
    // Flatten all postcode entries and pick one.
    let total: usize = geo
        .states
        .iter()
        .flat_map(|s| &s.cities)
        .map(|c| c.postcodes.len())
        .sum();

    if total == 0 {
        return Err(VarError::Other(
            "no postcode entries in geo data".to_string(),
        ));
    }

    let mut target = rng.gen_range(0..total);
    for state in &geo.states {
        for city in &state.cities {
            for pc in &city.postcodes {
                if target == 0 {
                    return Ok(pc);
                }
                target -= 1;
            }
        }
    }

    // Should not reach here, but satisfy the compiler.
    Err(VarError::Other(
        "failed to pick postcode entry".to_string(),
    ))
}

/// Expand a `GeoPostcode` entry into a street address string.
fn expand_street_from_entry(entry: &GeoPostcode, rng: &mut impl Rng) -> String {
    if let Some(ref pattern) = entry.pattern {
        expand_street_pattern(pattern, &entry.street_en, rng)
    } else if let Some(ref fixed) = entry.fixed {
        if fixed.is_empty() {
            format!("1 {}", entry.street_en)
        } else {
            let idx = rng.gen_range(0..fixed.len());
            fixed[idx].clone()
        }
    } else {
        let num: u32 = rng.gen_range(1..200);
        format!("{num} {}", entry.street_en)
    }
}

/// Expand a street pattern like `"n{1,221} Baker Street"` or `"北一条西n{1,20}"`.
/// The `n{min,max}` token is replaced with a random number in that range.
fn expand_street_pattern(pattern: &str, street_en: &str, rng: &mut impl Rng) -> String {
    if let Some(start) = pattern.find("n{") {
        if let Some(end) = pattern[start..].find('}') {
            let range_str = &pattern[start + 2..start + end];
            if let Some((min_s, max_s)) = range_str.split_once(',') {
                if let (Ok(min), Ok(max)) = (min_s.parse::<u32>(), max_s.parse::<u32>()) {
                    let num = rng.gen_range(min..=max);
                    let prefix = &pattern[..start];
                    let suffix = &pattern[start + end + 1..];
                    let expanded = format!("{prefix}{num}{suffix}");
                    // If the expanded already contains the English street name, return as-is.
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

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Replace every `#` in a format string with a random digit 0-9.
fn expand_format(fmt: &str, rng: &mut impl Rng) -> String {
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
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn seeded_rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(42)
    }

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn empty_params() -> HashMap<String, String> {
        HashMap::new()
    }

    /// Convenience: get JP GeoData from the database.
    fn geo_jp() -> &'static GeoData {
        geo_database().get("JP").expect("JP should be loaded")
    }

    /// Convenience: get GB GeoData from the database.
    fn geo_gb() -> &'static GeoData {
        geo_database().get("GB").expect("GB should be loaded")
    }

    // -----------------------------------------------------------------------
    // 1. Phone default picks a random geo country
    // -----------------------------------------------------------------------
    #[test]
    fn phone_default_uses_random_geo_data() {
        let mut rng = seeded_rng();
        let result = generate_phone(&empty_params(), &mut rng).expect("should generate");
        let phone = result.as_str().expect("should be string");
        assert!(
            phone.starts_with('+'),
            "default phone SHALL start with '+', got: {phone}"
        );
        assert!(
            phone.len() > 5,
            "default phone SHALL be a plausible length, got: {phone}"
        );
    }

    // -----------------------------------------------------------------------
    // 2. Phone country=JP
    // -----------------------------------------------------------------------
    #[test]
    fn phone_jp_starts_with_81() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "JP")]);
        let result = generate_phone(&p, &mut rng).expect("should generate");
        let phone = result.as_str().expect("should be string");
        assert!(
            phone.starts_with("+81-"),
            "JP phone should start with +81-, got: {phone}"
        );
    }

    // -----------------------------------------------------------------------
    // 3. Phone country=GB
    // -----------------------------------------------------------------------
    #[test]
    fn phone_gb_starts_with_44() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "GB")]);
        let result = generate_phone(&p, &mut rng).expect("should generate");
        let phone = result.as_str().expect("should be string");
        assert!(
            phone.starts_with("+44-"),
            "GB phone should start with +44-, got: {phone}"
        );
    }

    // -----------------------------------------------------------------------
    // 4. Phone with custom format
    // -----------------------------------------------------------------------
    #[test]
    fn phone_custom_format_replaces_hashes() {
        let mut rng = seeded_rng();
        let p = params(&[("format", "+81-###-####-####")]);
        let result = generate_phone(&p, &mut rng).expect("should generate");
        let phone = result.as_str().expect("should be string");
        assert!(
            phone.starts_with("+81-"),
            "custom format phone should start with +81-, got: {phone}"
        );
        assert_eq!(
            phone.len(),
            "+81-###-####-####".len(),
            "length should match format, got: {phone}"
        );
        // No '#' should remain.
        assert!(
            !phone.contains('#'),
            "no # should remain, got: {phone}"
        );
    }

    // -----------------------------------------------------------------------
    // 5. City country=JP returns a Japanese city
    // -----------------------------------------------------------------------
    #[test]
    fn city_jp_returns_japanese_city() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "JP")]);
        let result = generate_city(&p, &mut rng).expect("should generate");
        let city = result.as_str().expect("should be string");

        // Verify it is among JP geo data cities.
        let jp_cities = collect_city_names(Some(geo_jp()), None);
        assert!(
            jp_cities.contains(&city.to_string()),
            "city '{city}' should be a JP city"
        );
    }

    // -----------------------------------------------------------------------
    // 6. City country=GB returns a British city
    // -----------------------------------------------------------------------
    #[test]
    fn city_gb_returns_british_city() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "GB")]);
        let result = generate_city(&p, &mut rng).expect("should generate");
        let city = result.as_str().expect("should be string");

        let gb_cities = collect_city_names(Some(geo_gb()), None);
        assert!(
            gb_cities.contains(&city.to_string()),
            "city '{city}' should be a GB city"
        );
    }

    // -----------------------------------------------------------------------
    // 7. City with region filter narrows results
    // -----------------------------------------------------------------------
    #[test]
    fn city_region_filter_narrows_results() {
        let all_jp = collect_city_names(Some(geo_jp()), None);
        let kansai_only = collect_city_names(Some(geo_jp()), Some("Kansai"));

        assert!(
            !kansai_only.is_empty(),
            "should have Kansai cities"
        );
        assert!(
            kansai_only.len() < all_jp.len(),
            "Kansai subset ({}) should be smaller than all JP ({})",
            kansai_only.len(),
            all_jp.len()
        );

        // Actually generate one with region filter.
        let mut rng = seeded_rng();
        let p = params(&[("country", "JP"), ("region", "Kansai")]);
        let result = generate_city(&p, &mut rng).expect("should generate");
        let city = result.as_str().expect("should be string");
        assert!(
            kansai_only.contains(&city.to_string()),
            "city '{city}' should be in Kansai region"
        );
    }

    // -----------------------------------------------------------------------
    // 8. Postcode country=JP returns valid JP postcode
    // -----------------------------------------------------------------------
    #[test]
    fn postcode_jp_has_hyphen_format() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "JP")]);
        let result = generate_postcode(&p, &mut rng).expect("should generate");
        let postcode = result.as_str().expect("should be string");

        // JP postcodes are in ###-#### format.
        assert!(
            postcode.contains('-'),
            "JP postcode should contain hyphen, got: {postcode}"
        );
        let parts: Vec<&str> = postcode.split('-').collect();
        assert_eq!(
            parts.len(),
            2,
            "JP postcode should have 2 parts, got: {postcode}"
        );
        assert_eq!(
            parts[0].len(),
            3,
            "JP postcode first part should be 3 digits, got: {postcode}"
        );
        assert_eq!(
            parts[1].len(),
            4,
            "JP postcode second part should be 4 digits, got: {postcode}"
        );
    }

    // -----------------------------------------------------------------------
    // 9. Postcode country=GB returns valid GB postcode
    // -----------------------------------------------------------------------
    #[test]
    fn postcode_gb_has_space_format() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "GB")]);
        let result = generate_postcode(&p, &mut rng).expect("should generate");
        let postcode = result.as_str().expect("should be string");

        // GB postcodes have a space between outward and inward parts.
        assert!(
            postcode.contains(' '),
            "GB postcode should contain space, got: {postcode}"
        );

        // Verify it is a real postcode from gb.json.
        let all_gb_codes = collect_postcodes(Some(geo_gb()));
        assert!(
            all_gb_codes.contains(&postcode.to_string()),
            "postcode '{postcode}' should be from GB geo data"
        );
    }

    // -----------------------------------------------------------------------
    // 10. Street country=JP returns chome-ban-go style
    // -----------------------------------------------------------------------
    #[test]
    fn street_jp_returns_japanese_style() {
        // Run with multiple seeds to cover different patterns.
        for seed in 0u64..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let p = params(&[("country", "JP")]);
            let result = generate_street(&p, &mut rng).expect("should generate");
            let street = result.as_str().expect("should be string");
            assert!(
                !street.is_empty(),
                "JP street should not be empty (seed={seed})"
            );
            // JP streets are typically CJK text with numbers, or English fallbacks
            // containing the street_en. They should not start with a '+' (phone-like).
            assert!(
                !street.starts_with('+'),
                "JP street should not look like a phone, got: {street} (seed={seed})"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 11. Street country=GB returns number + street name
    // -----------------------------------------------------------------------
    #[test]
    fn street_gb_returns_number_and_name() {
        for seed in 0u64..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let p = params(&[("country", "GB")]);
            let result = generate_street(&p, &mut rng).expect("should generate");
            let street = result.as_str().expect("should be string");
            assert!(
                !street.is_empty(),
                "GB street should not be empty (seed={seed})"
            );
            // GB streets should have a numeric part followed by a name.
            // Pattern is "n{...} Name" so the expanded form is "NUM Name".
            let first_char = street.chars().next().expect("non-empty");
            assert!(
                first_char.is_ascii_digit(),
                "GB street should start with digit, got: '{street}' (seed={seed})"
            );
            assert!(
                street.contains(' '),
                "GB street should contain a space between number and name, got: '{street}' (seed={seed})"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 12. Deterministic seed produces same output
    // -----------------------------------------------------------------------
    #[test]
    fn deterministic_seed_same_output() {
        let p_jp = params(&[("country", "JP")]);

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let phone1 = generate_phone(&p_jp, &mut rng1).expect("should generate");
        let phone2 = generate_phone(&p_jp, &mut rng2).expect("should generate");
        assert_eq!(phone1, phone2, "same seed SHALL produce same phone");

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let city1 = generate_city(&p_jp, &mut rng1).expect("should generate");
        let city2 = generate_city(&p_jp, &mut rng2).expect("should generate");
        assert_eq!(city1, city2, "same seed SHALL produce same city");

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let pc1 = generate_postcode(&p_jp, &mut rng1).expect("should generate");
        let pc2 = generate_postcode(&p_jp, &mut rng2).expect("should generate");
        assert_eq!(pc1, pc2, "same seed SHALL produce same postcode");

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let st1 = generate_street(&p_jp, &mut rng1).expect("should generate");
        let st2 = generate_street(&p_jp, &mut rng2).expect("should generate");
        assert_eq!(st1, st2, "same seed SHALL produce same street");
    }

    // -----------------------------------------------------------------------
    // 13. Unknown country falls back to random geo data
    // -----------------------------------------------------------------------
    #[test]
    fn unknown_country_falls_back() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "XX")]);

        // Phone falls back to a random geo country.
        let phone = generate_phone(&p, &mut rng).expect("should generate");
        let phone_str = phone.as_str().expect("should be string");
        assert!(
            phone_str.starts_with('+'),
            "unknown country phone SHALL fall back to geo data, got: {phone_str}"
        );

        // City falls back to all loaded data.
        let mut rng = seeded_rng();
        let city = generate_city(&p, &mut rng).expect("should generate");
        let city_str = city.as_str().expect("should be string");
        assert!(!city_str.is_empty(), "fallback city SHALL NOT be empty");

        // Postcode falls back to all loaded data.
        let mut rng = seeded_rng();
        let pc = generate_postcode(&p, &mut rng).expect("should generate");
        let pc_str = pc.as_str().expect("should be string");
        assert!(!pc_str.is_empty(), "fallback postcode SHALL NOT be empty");

        // Street falls back to a random geo country.
        let mut rng = seeded_rng();
        let st = generate_street(&p, &mut rng).expect("should generate");
        let st_str = st.as_str().expect("should be string");
        assert!(!st_str.is_empty(), "fallback street SHALL NOT be empty");
    }

    // -----------------------------------------------------------------------
    // 14. City with no matching region returns error
    // -----------------------------------------------------------------------
    #[test]
    fn city_nonexistent_region_returns_error() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "JP"), ("region", "Narnia")]);
        let result = generate_city(&p, &mut rng);
        assert!(result.is_err(), "SHALL error for nonexistent region");
    }

    // -----------------------------------------------------------------------
    // 15. Phone format param overrides country
    // -----------------------------------------------------------------------
    #[test]
    fn phone_format_overrides_country() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "JP"), ("format", "+99-###-####")]);
        let result = generate_phone(&p, &mut rng).expect("should generate");
        let phone = result.as_str().expect("should be string");
        assert!(
            phone.starts_with("+99-"),
            "format param should override country, got: {phone}"
        );
    }

    // -----------------------------------------------------------------------
    // 16. Default city (no params) returns a city from combined data
    // -----------------------------------------------------------------------
    #[test]
    fn default_city_returns_from_combined_data() {
        let mut rng = seeded_rng();
        let result = generate_city(&empty_params(), &mut rng).expect("should generate");
        let city = result.as_str().expect("should be string");

        let all = collect_city_names(None, None);
        assert!(
            all.contains(&city.to_string()),
            "default city '{city}' should be from combined JP+GB data"
        );
    }

    // -----------------------------------------------------------------------
    // 17. GB region filter works
    // -----------------------------------------------------------------------
    #[test]
    fn city_gb_scotland_region_filter() {
        let scotland = collect_city_names(Some(geo_gb()), Some("Scotland"));
        assert!(
            !scotland.is_empty(),
            "should have Scottish cities"
        );

        let mut rng = seeded_rng();
        let p = params(&[("country", "GB"), ("region", "Scotland")]);
        let result = generate_city(&p, &mut rng).expect("should generate");
        let city = result.as_str().expect("should be string");
        assert!(
            scotland.contains(&city.to_string()),
            "city '{city}' should be in Scotland region"
        );
    }
}
