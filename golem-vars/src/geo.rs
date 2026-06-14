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

/// Expand a street pattern containing one or more `n{min,max}` tokens.
///
/// Each `n{min,max}` is replaced with a random integer in `[min, max]`.
/// Supports multiple tokens per pattern, e.g. `"清田一条n{１,４}丁目n{１,１５}番"`.
///
/// The numeral system is detected from the min value's digits:
/// - ASCII `0-9` → ASCII output (default)
/// - Full-width `０-９` → full-width output (Japanese addresses)
/// - Arabic-Indic `٠-٩` → Arabic-Indic output
/// - Hebrew numerals not yet supported (Hebrew addresses use ASCII)
///
/// If `street_en` is non-empty and the expanded result doesn't contain it,
/// falls back to `"NUM street_en"` format using the first generated number.
pub(crate) fn expand_street_pattern(pattern: &str, street_en: &str, rng: &mut impl Rng) -> String {
    let mut result = pattern.to_string();
    let mut first_num: Option<String> = None;

    // Replace all n{min,max} tokens left-to-right.
    while let Some(start) = result.find("n{") {
        let Some(close) = result[start..].find('}') else { break };
        let range_str = &result[start + 2..start + close];
        let Some((min_s, max_s)) = range_str.split_once(',') else { break };
        let min_s = min_s.trim();
        let max_s = max_s.trim();

        // Detect numeral system from min value.
        let style = detect_numeral_style(min_s);

        let Some(min) = parse_numerals(min_s) else { break };
        let Some(max) = parse_numerals(max_s) else { break };

        let num = rng.gen_range(min..=max);
        let num_str = format_numerals(num, style);
        if first_num.is_none() {
            first_num = Some(num_str.clone());
        }

        let prefix = &result[..start];
        let suffix = &result[start + close + 1..];
        result = format!("{prefix}{num_str}{suffix}");
    }

    // If expanded text contains the English street name (or no English name), return as-is.
    if result.contains(street_en) || street_en.is_empty() {
        return result;
    }

    // Fallback: "NUM street_en" using the first number we generated.
    let num_str = first_num.unwrap_or_else(|| rng.gen_range(1..200u32).to_string());
    format!("{num_str} {street_en}")
}

/// Numeral systems supported in street patterns.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NumeralStyle {
    Ascii,
    FullWidth,
    ArabicIndic,
}

/// Detect numeral style from the first digit character in a string.
fn detect_numeral_style(s: &str) -> NumeralStyle {
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            return NumeralStyle::Ascii;
        }
        if ('０'..='９').contains(&ch) {
            return NumeralStyle::FullWidth;
        }
        if ('٠'..='٩').contains(&ch) {
            return NumeralStyle::ArabicIndic;
        }
    }
    NumeralStyle::Ascii
}

/// Parse a numeral string (any supported style) to u32.
fn parse_numerals(s: &str) -> Option<u32> {
    let ascii: String = s.chars().map(|ch| {
        if ('０'..='９').contains(&ch) {
            (b'0' + (ch as u32 - '０' as u32) as u8) as char
        } else if ('٠'..='٩').contains(&ch) {
            (b'0' + (ch as u32 - '٠' as u32) as u8) as char
        } else {
            ch
        }
    }).collect();
    ascii.parse().ok()
}

/// Format a u32 in the given numeral style.
fn format_numerals(n: u32, style: NumeralStyle) -> String {
    let ascii = n.to_string();
    match style {
        NumeralStyle::Ascii => ascii,
        NumeralStyle::FullWidth => ascii.chars().map(|ch| {
            if ch.is_ascii_digit() {
                char::from_u32('０' as u32 + (ch as u32 - '0' as u32)).unwrap_or(ch)
            } else {
                ch
            }
        }).collect(),
        NumeralStyle::ArabicIndic => ascii.chars().map(|ch| {
            if ch.is_ascii_digit() {
                char::from_u32('٠' as u32 + (ch as u32 - '0' as u32)).unwrap_or(ch)
            } else {
                ch
            }
        }).collect(),
    }
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
    use crate::geo_loader::{GeoCity, GeoCountry, GeoState};
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

    // -----------------------------------------------------------------------
    // 18. expand_street_pattern: single token
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_single_token() {
        let mut rng = seeded_rng();
        let result = expand_street_pattern("n{1,100} Baker Street", "Baker Street", &mut rng);
        assert!(
            result.ends_with("Baker Street"),
            "SHALL contain street name, got: {result}"
        );
        let num: u32 = result.split_whitespace().next().unwrap().parse().unwrap();
        assert!((1..=100).contains(&num), "number SHALL be in range, got: {num}");
    }

    // -----------------------------------------------------------------------
    // 19. expand_street_pattern: multiple tokens (JP style)
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_multiple_tokens() {
        let mut rng = seeded_rng();
        let result = expand_street_pattern("清田一条n{1,4}-n{1,15}", "", &mut rng);
        assert!(
            result.starts_with("清田一条"),
            "SHALL keep prefix, got: {result}"
        );
        assert!(
            !result.contains("n{"),
            "SHALL replace all tokens, got: {result}"
        );
        // Should match pattern like "清田一条2-7"
        let suffix = &result["清田一条".len()..];
        let parts: Vec<&str> = suffix.split('-').collect();
        assert_eq!(parts.len(), 2, "SHALL have two numeric parts, got: {result}");
        let n1: u32 = parts[0].parse().expect("first part SHALL be numeric");
        let n2: u32 = parts[1].parse().expect("second part SHALL be numeric");
        assert!((1..=4).contains(&n1), "first num SHALL be 1-4, got: {n1}");
        assert!((1..=15).contains(&n2), "second num SHALL be 1-15, got: {n2}");
    }

    // -----------------------------------------------------------------------
    // 20. expand_street_pattern: deterministic with seed
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_deterministic() {
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();
        let a = expand_street_pattern("新町n{1,2}-n{1,50}", "", &mut rng1);
        let b = expand_street_pattern("新町n{1,2}-n{1,50}", "", &mut rng2);
        assert_eq!(a, b, "same seed SHALL produce same result");
    }

    // -----------------------------------------------------------------------
    // 21. expand_street_pattern: no tokens falls back
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_no_tokens_returns_as_is() {
        let mut rng = seeded_rng();
        let result = expand_street_pattern("Fixed Street Name", "Fixed Street Name", &mut rng);
        assert_eq!(result, "Fixed Street Name", "no tokens SHALL return pattern unchanged");
    }

    // -----------------------------------------------------------------------
    // 22. expand_street_pattern: full-width numerals (JP)
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_fullwidth_numerals() {
        let mut rng = seeded_rng();
        let result = expand_street_pattern("清田一条n{１,４}丁目n{１,１５}番", "", &mut rng);
        assert!(
            result.starts_with("清田一条"),
            "SHALL keep prefix, got: {result}"
        );
        assert!(
            !result.contains("n{"),
            "SHALL replace all tokens, got: {result}"
        );
        // Should contain full-width digits, not ASCII
        assert!(
            !result.chars().any(|c| c.is_ascii_digit()),
            "SHALL use full-width digits not ASCII, got: {result}"
        );
        // Should contain 丁目 and 番
        assert!(result.contains("丁目"), "SHALL keep 丁目 delimiter, got: {result}");
        assert!(result.contains("番"), "SHALL keep 番 delimiter, got: {result}");
    }

    // -----------------------------------------------------------------------
    // 23. expand_street_pattern: mixed styles use min's style
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_mixed_uses_min_style() {
        let mut rng = seeded_rng();
        // Full-width min, ASCII max — should output full-width
        let result = expand_street_pattern("町n{１,20}", "", &mut rng);
        assert!(
            !result.chars().any(|c| c.is_ascii_digit()),
            "SHALL use full-width (from min), got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 24. parse_numerals: full-width and Arabic-Indic
    // -----------------------------------------------------------------------
    #[test]
    fn parse_numerals_all_styles() {
        assert_eq!(parse_numerals("42"), Some(42));
        assert_eq!(parse_numerals("４２"), Some(42));
        assert_eq!(parse_numerals("٤٢"), Some(42));
        assert_eq!(parse_numerals(""), None);
        assert_eq!(parse_numerals("abc"), None);
    }

    // -----------------------------------------------------------------------
    // 25. format_numerals round-trips
    // -----------------------------------------------------------------------
    #[test]
    fn format_numerals_round_trip() {
        assert_eq!(format_numerals(123, NumeralStyle::Ascii), "123");
        assert_eq!(format_numerals(123, NumeralStyle::FullWidth), "１２３");
        assert_eq!(format_numerals(7, NumeralStyle::ArabicIndic), "٧");
    }

    /// Convenience: build a `GeoPostcode` for entry-expansion tests.
    fn postcode_entry(
        street_en: &str,
        pattern: Option<&str>,
        fixed: Option<Vec<&str>>,
    ) -> GeoPostcode {
        GeoPostcode {
            code: "0000-000".to_string(),
            street: String::new(),
            street_en: street_en.to_string(),
            pattern: pattern.map(|s| s.to_string()),
            fixed: fixed.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
        }
    }

    // -----------------------------------------------------------------------
    // 26. expand_street_from_entry: pattern branch delegates to pattern expansion
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_from_entry_pattern_branch() {
        let mut rng = seeded_rng();
        let entry = postcode_entry("Baker Street", Some("n{1,9} Baker Street"), None);
        let result = expand_street_from_entry(&entry, &mut rng);
        assert!(
            result.ends_with("Baker Street"),
            "pattern branch SHALL expand to street name, got: {result}"
        );
        assert!(
            !result.contains("n{"),
            "pattern branch SHALL replace token, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 27. expand_street_from_entry: empty fixed list uses "1 street_en"
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_from_entry_empty_fixed_uses_one() {
        let mut rng = seeded_rng();
        let entry = postcode_entry("High Street", None, Some(vec![]));
        let result = expand_street_from_entry(&entry, &mut rng);
        assert_eq!(
            result, "1 High Street",
            "empty fixed list SHALL produce '1 street_en', got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 28. expand_street_from_entry: non-empty fixed list picks an entry verbatim
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_from_entry_fixed_picks_verbatim() {
        let mut rng = seeded_rng();
        let entry = postcode_entry("ignored", None, Some(vec!["Only One Address"]));
        let result = expand_street_from_entry(&entry, &mut rng);
        assert_eq!(
            result, "Only One Address",
            "single-element fixed list SHALL be returned verbatim, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 29. expand_street_from_entry: no pattern, no fixed → "NUM street_en"
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_from_entry_default_numeric_prefix() {
        let entry = postcode_entry("Main Road", None, None);
        // Cover the random-number range with several seeds.
        for seed in 0u64..10 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result = expand_street_from_entry(&entry, &mut rng);
            assert!(
                result.ends_with(" Main Road"),
                "default branch SHALL end with street name, got: {result} (seed={seed})"
            );
            let num: u32 = result
                .split_whitespace()
                .next()
                .expect("SHALL have a leading number")
                .parse()
                .expect("leading token SHALL be numeric");
            assert!(
                (1..200).contains(&num),
                "default number SHALL be in 1..200, got: {num} (seed={seed})"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 30. expand_street_pattern: tokenless pattern with non-matching street_en
    //     falls back to "NUM street_en" using a freshly generated number.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_fallback_when_name_absent() {
        for seed in 0u64..10 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            // Pattern has no token and does not contain the (non-empty) street_en.
            let result = expand_street_pattern("Some Lane", "Baker Street", &mut rng);
            assert!(
                result.ends_with("Baker Street"),
                "fallback SHALL append street_en, got: {result} (seed={seed})"
            );
            let num: u32 = result
                .split_whitespace()
                .next()
                .expect("SHALL have a leading number")
                .parse()
                .expect("leading token SHALL be numeric");
            assert!(
                (1..200).contains(&num),
                "fallback number SHALL be in 1..200, got: {num} (seed={seed})"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 31. expand_street_pattern: malformed token (no closing brace) breaks loop
    //     and the fallback appends the absent street name.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_unclosed_token_breaks() {
        let mut rng = seeded_rng();
        // "n{1,5 Foo" has no closing brace → loop breaks, no token replaced.
        // The unchanged pattern lacks the non-empty street_en, so the fallback
        // builds "NUM street_en" with a freshly generated number.
        let result = expand_street_pattern("n{1,5 Foo", "Bar", &mut rng);
        assert!(
            result.ends_with(" Bar"),
            "unclosed token SHALL fall back to 'NUM street_en', got: {result}"
        );
        let num: u32 = result
            .split_whitespace()
            .next()
            .expect("SHALL have a leading number")
            .parse()
            .expect("leading token SHALL be numeric");
        assert!(
            (1..200).contains(&num),
            "fallback number SHALL be in 1..200, got: {num}"
        );
    }

    // -----------------------------------------------------------------------
    // 32. expand_street_pattern: token without comma breaks loop, returns as-is.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_token_without_comma_returns_as_is() {
        let mut rng = seeded_rng();
        // No comma inside braces → split_once fails → break; street_en empty → return as-is.
        let result = expand_street_pattern("Road n{5}", "", &mut rng);
        assert_eq!(
            result, "Road n{5}",
            "comma-less token SHALL leave pattern unchanged, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 33. expand_street_pattern: non-numeric min breaks loop, returns as-is.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_non_numeric_bounds_returns_as_is() {
        let mut rng = seeded_rng();
        let result = expand_street_pattern("Road n{a,b}", "", &mut rng);
        assert_eq!(
            result, "Road n{a,b}",
            "non-numeric bounds SHALL leave pattern unchanged, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 34. expand_street_pattern: bounds are trimmed of whitespace.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_trims_bound_whitespace() {
        let mut rng = seeded_rng();
        let result = expand_street_pattern("n{ 3 , 3 } Way", "Way", &mut rng);
        assert_eq!(
            result, "3 Way",
            "trimmed bounds SHALL parse and equal-range yields the value, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 35. expand_street_pattern: Arabic-Indic min yields Arabic-Indic output.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_street_pattern_arabic_indic_output() {
        let mut rng = seeded_rng();
        // Equal range so output is deterministic: ٧ (Arabic-Indic 7).
        let result = expand_street_pattern("شارع n{٧,٧}", "", &mut rng);
        assert!(
            result.contains('٧'),
            "Arabic-Indic min SHALL produce Arabic-Indic digit, got: {result}"
        );
        assert!(
            !result.chars().any(|c| c.is_ascii_digit()),
            "Arabic-Indic output SHALL NOT contain ASCII digits, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 36. detect_numeral_style: each branch and the no-digit default.
    // -----------------------------------------------------------------------
    #[test]
    fn detect_numeral_style_branches() {
        assert_eq!(detect_numeral_style("12"), NumeralStyle::Ascii);
        assert_eq!(detect_numeral_style("１２"), NumeralStyle::FullWidth);
        assert_eq!(detect_numeral_style("٧"), NumeralStyle::ArabicIndic);
        // Leading non-digit chars are skipped until the first digit is found.
        assert_eq!(detect_numeral_style("abc５"), NumeralStyle::FullWidth);
        // No digit at all defaults to Ascii.
        assert_eq!(detect_numeral_style("no digits"), NumeralStyle::Ascii);
        assert_eq!(detect_numeral_style(""), NumeralStyle::Ascii);
    }

    // -----------------------------------------------------------------------
    // 37. expand_format: '#' becomes a digit, other chars pass through.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_format_preserves_non_hash_chars() {
        let mut rng = seeded_rng();
        let result = expand_format("AB-##-(x)", &mut rng);
        assert_eq!(
            result.len(),
            "AB-##-(x)".len(),
            "expand_format SHALL preserve length, got: {result}"
        );
        assert!(
            result.starts_with("AB-") && result.ends_with("-(x)"),
            "literal chars SHALL pass through, got: {result}"
        );
        assert!(
            !result.contains('#'),
            "all '#' SHALL be replaced, got: {result}"
        );
        // The two replaced positions SHALL be ASCII digits.
        let digits: Vec<char> = result.chars().filter(|c| c.is_ascii_digit()).collect();
        assert_eq!(digits.len(), 2, "two '#' SHALL become two digits, got: {result}");
    }

    // -----------------------------------------------------------------------
    // 38. expand_format: empty string yields empty string.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_format_empty_is_empty() {
        let mut rng = seeded_rng();
        assert_eq!(expand_format("", &mut rng), "", "empty format SHALL stay empty");
    }

    // -----------------------------------------------------------------------
    // 39. parse_numerals: mixed full-width/ASCII digits parse together.
    // -----------------------------------------------------------------------
    #[test]
    fn parse_numerals_mixed_digits() {
        // Full-width '４' followed by ASCII '2' → 42.
        assert_eq!(parse_numerals("４2"), Some(42));
        // Trailing non-digit makes the whole parse fail.
        assert_eq!(parse_numerals("4x"), None);
    }

    /// Convenience: build a `GeoData` fixture with one state holding the given
    /// region tags and one city per (name, postcode-codes) pair supplied.
    fn geo_fixture(region_tags: &[&str], cities: &[(&str, &[&str])]) -> GeoData {
        let cities: Vec<GeoCity> = cities
            .iter()
            .map(|(name, codes)| {
                let postcodes: Vec<GeoPostcode> = codes
                    .iter()
                    .map(|code| GeoPostcode::for_test(code, "St", None, None))
                    .collect();
                GeoCity::for_test(name, postcodes)
            })
            .collect();
        let state = GeoState::for_test(
            region_tags.iter().map(|s| s.to_string()).collect(),
            cities,
        );
        let country = GeoCountry::for_test("ZZ", vec!["+99 ### ####".to_string()]);
        GeoData::for_test(country, vec![state])
    }

    // -----------------------------------------------------------------------
    // 40. collect_city_names with a hand-built GeoData returns every city name.
    // -----------------------------------------------------------------------
    #[test]
    fn collect_city_names_from_fixture_returns_all() {
        let geo = geo_fixture(&["r1"], &[("Alpha", &["1"]), ("Beta", &["2"])]);
        let names = collect_city_names(Some(&geo), None);
        assert_eq!(
            names,
            vec!["Alpha".to_string(), "Beta".to_string()],
            "SHALL return every city name in order, got: {names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 41. collect_city_names region filter is case-insensitive and excludes
    //     states whose region_tags do not contain the requested tag.
    // -----------------------------------------------------------------------
    #[test]
    fn collect_city_names_region_filter_case_insensitive() {
        let geo = geo_fixture(&["Kansai"], &[("Osaka", &["1"])]);
        // 1. Matching tag with different casing SHALL still match.
        let matched = collect_city_names(Some(&geo), Some("kansai"));
        assert_eq!(
            matched,
            vec!["Osaka".to_string()],
            "case-insensitive region match SHALL include the city, got: {matched:?}"
        );
        // 2. A non-matching tag SHALL exclude the state entirely.
        let missed = collect_city_names(Some(&geo), Some("Narnia"));
        assert!(
            missed.is_empty(),
            "non-matching region SHALL yield no cities, got: {missed:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 42. collect_postcodes flattens every postcode code across cities.
    // -----------------------------------------------------------------------
    #[test]
    fn collect_postcodes_from_fixture_flattens_codes() {
        let geo = geo_fixture(&["r1"], &[("Alpha", &["100", "101"]), ("Beta", &["200"])]);
        let codes = collect_postcodes(Some(&geo));
        assert_eq!(
            codes,
            vec!["100".to_string(), "101".to_string(), "200".to_string()],
            "SHALL flatten every postcode code, got: {codes:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 43. pick_random_postcode_entry returns one of the fixture's entries and
    //     errors when the fixture has no postcodes.
    // -----------------------------------------------------------------------
    #[test]
    fn pick_random_postcode_entry_returns_member_or_errors() {
        // 1. With entries, the picked code SHALL be one we put in.
        let geo = geo_fixture(&["r1"], &[("Alpha", &["100", "101"])]);
        let mut rng = seeded_rng();
        let entry = pick_random_postcode_entry(&geo, &mut rng).expect("SHALL pick an entry");
        assert!(
            entry.code == "100" || entry.code == "101",
            "picked code SHALL be a fixture entry, got: {}",
            entry.code
        );

        // 2. With no postcodes at all, it SHALL error.
        let empty = geo_fixture(&["r1"], &[("Alpha", &[])]);
        let mut rng = seeded_rng();
        let result = pick_random_postcode_entry(&empty, &mut rng);
        assert!(
            result.is_err(),
            "empty geo data SHALL error from pick_random_postcode_entry"
        );
    }
}
