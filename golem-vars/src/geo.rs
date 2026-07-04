//! Geo-aware generators for phone, city, postcode, and street.
//!
//! These produce scalar `VarValue::String` results and are wired into
//! `generate_simple()` via the `phone`, `city`, `postcode`, and `street`
//! generator names.

use std::collections::HashMap;

use rand::Rng;

use crate::geo_loader::geo_database;
use crate::{VarError, VarValue};

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

    // Unset country → random; a present-but-unknown code → error (a typo
    // shouldn't silently produce some other country's number).
    let geo = match params.get("country") {
        Some(code) => geo_database()
            .get(code)
            .ok_or_else(|| VarError::Other(format!("unknown country code: {code}")))?,
        None => geo_database().random(rng),
    };

    if geo.country.phone_formats.is_empty() {
        return Err(VarError::Other(format!(
            "no phone formats for country {}",
            geo.country.iso_code
        )));
    }

    let idx = rng.gen_range(0..geo.country.phone_formats.len());
    let fmt = &geo.country.phone_formats[idx];
    Ok(VarValue::String(expand_format(fmt, rng)))
}

/// Expand a native street `pattern` containing zero or more `n{min,max}`
/// house-number tokens, drawing each number once. Returns the rendered native
/// string (numbers in the pattern's own numeral style — full-width for JP,
/// Arabic-Indic for AR, …) AND the list of drawn integer VALUES in token order,
/// so an ascii rendering can reuse the SAME house number(s).
///
/// The numeral system is detected per token from the min value's digits (ASCII
/// `0-9` default, full-width `０-９`, Arabic-Indic `٠-٩`). A malformed token
/// (no `}`, no comma, non-numeric bound) stops expansion, leaving the rest of
/// the pattern verbatim — current data is well-formed, this only avoids panics.
pub(crate) fn expand_native_tokens(pattern: &str, rng: &mut impl Rng) -> (String, Vec<u32>) {
    let mut result = pattern.to_string();
    let mut nums = Vec::new();
    let mut from = 0usize;

    while let Some(rel) = result[from..].find("n{") {
        let start = from + rel;
        let Some(close_rel) = result[start..].find('}') else {
            break;
        };
        let close = start + close_rel;
        let range_str = &result[start + 2..close];
        let Some((min_s, max_s)) = range_str.split_once(',') else {
            break;
        };
        let (min_s, max_s) = (min_s.trim(), max_s.trim());

        let style = detect_numeral_style(min_s);
        let (Some(min), Some(max)) = (parse_numerals(min_s), parse_numerals(max_s)) else {
            break;
        };
        // Tolerate reversed bounds (e.g. `n{5,1}`) by swapping rather than
        // panicking in `gen_range(min..=max)`.
        let (min, max) = if min > max { (max, min) } else { (min, max) };

        let num = rng.gen_range(min..=max);
        nums.push(num);
        let num_str = format_numerals(num, style);

        let prefix = &result[..start];
        let suffix = &result[close + 1..];
        result = format!("{prefix}{num_str}{suffix}");
        from = start + num_str.len();
    }

    (result, nums)
}

/// Fill an ASCII street `pattern`'s `n{…}` tokens with pre-drawn house numbers
/// (ASCII digits), reusing `nums` in token order so the ascii street names the
/// SAME building as its native counterpart. The tokens' own ranges are ignored
/// (the value is already chosen); a token past the end of `nums` yields `0`.
pub(crate) fn fill_ascii_tokens(pattern: &str, nums: &[u32]) -> String {
    let mut result = pattern.to_string();
    let mut idx = 0usize;
    let mut from = 0usize;

    while let Some(rel) = result[from..].find("n{") {
        let start = from + rel;
        let Some(close_rel) = result[start..].find('}') else {
            break;
        };
        let close = start + close_rel;
        let num_str = nums.get(idx).copied().unwrap_or(0).to_string();
        idx += 1;

        let prefix = &result[..start];
        let suffix = &result[close + 1..];
        result = format!("{prefix}{num_str}{suffix}");
        from = start + num_str.len();
    }

    result
}

/// Fold full-width digits (U+FF10–FF19) to ASCII, leaving everything else
/// untouched. Used for the ascii postcode, since [`crate::script::ascii_fold`]
/// (NFD) does not decompose full-width digits (that is NFKD).
pub(crate) fn fold_fullwidth_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c as u32 {
            0xFF10..=0xFF19 => char::from(b'0' + (c as u32 - 0xFF10) as u8),
            _ => c,
        })
        .collect()
}

/// Tidy a derived ascii street: insert a space at letter→digit joins (the
/// native scripts run a street name straight into its house number, e.g.
/// `Kiyota 1-jo4` → `Kiyota 1-jo 4`) and collapse any resulting double spaces.
/// ASCII-only — the native string keeps its run-together form (`清田一条４`).
pub(crate) fn tidy_ascii_spacing(s: &str) -> String {
    let mut spaced = String::with_capacity(s.len() + 4);
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if prev.is_some_and(|p| p.is_alphabetic()) && c.is_ascii_digit() {
            spaced.push(' ');
        }
        spaced.push(c);
        prev = Some(c);
    }
    // Collapse runs of spaces and trim (markers may introduce an extra space).
    let mut out = String::with_capacity(spaced.len());
    let mut last_space = false;
    for c in spaced.chars() {
        if c == ' ' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
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
    let ascii: String = s
        .chars()
        .map(|ch| {
            if ('０'..='９').contains(&ch) {
                (b'0' + (ch as u32 - '０' as u32) as u8) as char
            } else if ('٠'..='٩').contains(&ch) {
                (b'0' + (ch as u32 - '٠' as u32) as u8) as char
            } else {
                ch
            }
        })
        .collect();
    ascii.parse().ok()
}

/// Format a u32 in the given numeral style.
fn format_numerals(n: u32, style: NumeralStyle) -> String {
    let ascii = n.to_string();
    match style {
        NumeralStyle::Ascii => ascii,
        NumeralStyle::FullWidth => ascii
            .chars()
            .map(|ch| {
                if ch.is_ascii_digit() {
                    char::from_u32('０' as u32 + (ch as u32 - '0' as u32)).unwrap_or(ch)
                } else {
                    ch
                }
            })
            .collect(),
        NumeralStyle::ArabicIndic => ascii
            .chars()
            .map(|ch| {
                if ch.is_ascii_digit() {
                    char::from_u32('٠' as u32 + (ch as u32 - '0' as u32)).unwrap_or(ch)
                } else {
                    ch
                }
            })
            .collect(),
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
        assert!(!phone.contains('#'), "no # should remain, got: {phone}");
    }

    // -----------------------------------------------------------------------
    // 12. Deterministic seed produces same phone output
    // -----------------------------------------------------------------------
    #[test]
    fn deterministic_seed_same_output() {
        let p_jp = params(&[("country", "JP")]);

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let phone1 = generate_phone(&p_jp, &mut rng1).expect("should generate");
        let phone2 = generate_phone(&p_jp, &mut rng2).expect("should generate");
        assert_eq!(phone1, phone2, "same seed SHALL produce same phone");
    }

    // -----------------------------------------------------------------------
    // 13. Unknown country: phone errors (a typo shouldn't silently yield
    //     another country's number); an unset country picks one at random.
    // -----------------------------------------------------------------------
    #[test]
    fn unknown_country_phone_errors() {
        let mut rng = seeded_rng();
        let p = params(&[("country", "XX")]);

        let err = generate_phone(&p, &mut rng).expect_err("unknown country SHALL error");
        assert!(
            err.to_string().contains("unknown country code: XX"),
            "got: {err}"
        );

        // No country param → random country (no error).
        generate_phone(&empty_params(), &mut rng).expect("unset country SHALL generate");
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
    // 18. expand_native_tokens: single token returns the rendered string AND the
    //     drawn value (so an ascii rendering can reuse the same number).
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_single_token() {
        let mut rng = seeded_rng();
        let (result, nums) = expand_native_tokens("n{1,100} Baker Street", &mut rng);
        assert!(
            result.ends_with("Baker Street"),
            "SHALL contain street name, got: {result}"
        );
        assert_eq!(nums.len(), 1, "SHALL record one drawn number");
        let num: u32 = result.split_whitespace().next().expect("next() SHALL succeed").parse().expect("parse() SHALL succeed");
        assert!(
            (1..=100).contains(&num),
            "number SHALL be in range, got: {num}"
        );
        assert_eq!(num, nums[0], "rendered number SHALL equal recorded value");
    }

    // Reversed bounds (`n{max,min}`) SHALL be tolerated by swapping, not panic
    // in `gen_range(min..=max)`.
    #[test]
    fn expand_native_tokens_reversed_bounds_no_panic() {
        let mut rng = seeded_rng();
        let (result, _) = expand_native_tokens("n{100,1} Baker Street", &mut rng);
        assert!(
            result.ends_with("Baker Street"),
            "SHALL contain street name, got: {result}"
        );
        let num: u32 = result.split_whitespace().next().expect("next() SHALL succeed").parse().expect("parse() SHALL succeed");
        assert!(
            (1..=100).contains(&num),
            "swapped range SHALL yield a number in [1,100], got: {num}"
        );
    }

    // -----------------------------------------------------------------------
    // 19. expand_native_tokens: multiple tokens (JP style), values recorded in
    //     order.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_multiple_tokens() {
        let mut rng = seeded_rng();
        let (result, nums) = expand_native_tokens("清田一条n{1,4}-n{1,15}", &mut rng);
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
        assert_eq!(
            parts.len(),
            2,
            "SHALL have two numeric parts, got: {result}"
        );
        let n1: u32 = parts[0].parse().expect("first part SHALL be numeric");
        let n2: u32 = parts[1].parse().expect("second part SHALL be numeric");
        assert!((1..=4).contains(&n1), "first num SHALL be 1-4, got: {n1}");
        assert!(
            (1..=15).contains(&n2),
            "second num SHALL be 1-15, got: {n2}"
        );
        assert_eq!(nums, vec![n1, n2], "recorded values SHALL match, in order");
    }

    // -----------------------------------------------------------------------
    // 20. expand_native_tokens: deterministic with seed
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_deterministic() {
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();
        let a = expand_native_tokens("新町n{1,2}-n{1,50}", &mut rng1);
        let b = expand_native_tokens("新町n{1,2}-n{1,50}", &mut rng2);
        assert_eq!(a, b, "same seed SHALL produce same result");
    }

    // -----------------------------------------------------------------------
    // 21. expand_native_tokens: no tokens returns the pattern unchanged, no nums
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_no_tokens_returns_as_is() {
        let mut rng = seeded_rng();
        let (result, nums) = expand_native_tokens("Fixed Street Name", &mut rng);
        assert_eq!(
            result, "Fixed Street Name",
            "no tokens SHALL return pattern unchanged"
        );
        assert!(nums.is_empty(), "no tokens SHALL record no numbers");
    }

    // -----------------------------------------------------------------------
    // 22. expand_native_tokens: full-width numerals (JP)
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_fullwidth_numerals() {
        let mut rng = seeded_rng();
        let (result, _) = expand_native_tokens("清田一条n{１,４}丁目n{１,１５}番", &mut rng);
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
        assert!(
            result.contains("丁目"),
            "SHALL keep 丁目 delimiter, got: {result}"
        );
        assert!(
            result.contains("番"),
            "SHALL keep 番 delimiter, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 23. expand_native_tokens: mixed styles use min's style
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_mixed_uses_min_style() {
        let mut rng = seeded_rng();
        // Full-width min, ASCII max — should output full-width
        let (result, _) = expand_native_tokens("町n{１,20}", &mut rng);
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

    // -----------------------------------------------------------------------
    // 30. fill_ascii_tokens: reuses the pre-drawn house numbers (ASCII digits),
    //     in token order, so the ascii street names the SAME building.
    // -----------------------------------------------------------------------
    #[test]
    fn fill_ascii_tokens_reuses_drawn_numbers() {
        // Two tokens, two recorded values → filled in order, ASCII digits.
        let filled = fill_ascii_tokens("Kiyota 1-jo n{1,4}-n{1,15}", &[3, 12]);
        assert_eq!(
            filled, "Kiyota 1-jo 3-12",
            "tokens SHALL be filled with the recorded numbers in order"
        );
        // A token past the end of the recorded values yields 0.
        assert_eq!(fill_ascii_tokens("Rd n{1,9}", &[]), "Rd 0");
        // No tokens → returned unchanged.
        assert_eq!(fill_ascii_tokens("Plain Road", &[5]), "Plain Road");
    }

    // -----------------------------------------------------------------------
    // 30b. fold_fullwidth_digits: full-width digits → ASCII, rest untouched.
    // -----------------------------------------------------------------------
    #[test]
    fn fold_fullwidth_digits_maps_only_fullwidth() {
        assert_eq!(fold_fullwidth_digits("０６０-０００１"), "060-0001");
        assert_eq!(fold_fullwidth_digits("12345"), "12345");
        // Non-digit native characters pass through.
        assert_eq!(fold_fullwidth_digits("〒１２３"), "〒123");
    }

    // -----------------------------------------------------------------------
    // 31. expand_native_tokens: malformed token (no closing brace) breaks the
    //     loop, leaving the rest verbatim and recording no number.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_unclosed_token_breaks() {
        let mut rng = seeded_rng();
        let (result, nums) = expand_native_tokens("n{1,5 Foo", &mut rng);
        assert_eq!(
            result, "n{1,5 Foo",
            "unclosed token SHALL leave the pattern unchanged, got: {result}"
        );
        assert!(nums.is_empty(), "no number SHALL be recorded");
    }

    // -----------------------------------------------------------------------
    // 32. expand_native_tokens: token without comma breaks loop, returns as-is.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_token_without_comma_returns_as_is() {
        let mut rng = seeded_rng();
        let (result, nums) = expand_native_tokens("Road n{5}", &mut rng);
        assert_eq!(
            result, "Road n{5}",
            "comma-less token SHALL leave pattern unchanged, got: {result}"
        );
        assert!(nums.is_empty());
    }

    // -----------------------------------------------------------------------
    // 33. expand_native_tokens: non-numeric min breaks loop, returns as-is.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_non_numeric_bounds_returns_as_is() {
        let mut rng = seeded_rng();
        let (result, _) = expand_native_tokens("Road n{a,b}", &mut rng);
        assert_eq!(
            result, "Road n{a,b}",
            "non-numeric bounds SHALL leave pattern unchanged, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 34. expand_native_tokens: bounds are trimmed of whitespace.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_trims_bound_whitespace() {
        let mut rng = seeded_rng();
        let (result, _) = expand_native_tokens("n{ 3 , 3 } Way", &mut rng);
        assert_eq!(
            result, "3 Way",
            "trimmed bounds SHALL parse and equal-range yields the value, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 35. expand_native_tokens: Arabic-Indic min yields Arabic-Indic output.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_native_tokens_arabic_indic_output() {
        let mut rng = seeded_rng();
        // Equal range so output is deterministic: ٧ (Arabic-Indic 7).
        let (result, _) = expand_native_tokens("شارع n{٧,٧}", &mut rng);
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
        assert_eq!(
            digits.len(),
            2,
            "two '#' SHALL become two digits, got: {result}"
        );
    }

    // -----------------------------------------------------------------------
    // 38. expand_format: empty string yields empty string.
    // -----------------------------------------------------------------------
    #[test]
    fn expand_format_empty_is_empty() {
        let mut rng = seeded_rng();
        assert_eq!(
            expand_format("", &mut rng),
            "",
            "empty format SHALL stay empty"
        );
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
}
