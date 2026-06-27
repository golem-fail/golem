use crate::seed::FakeRng;
use crate::sentence_loader::sentence_database;
use crate::{GeneratorDef, VarError, VarValue};
use chrono::{DateTime, Duration, Months, Utc};
use rand::Rng;
use uuid::Uuid;

/// Generate a simple (non-structured) fake value from a generator definition.
pub fn generate_simple(def: &GeneratorDef, rng: &mut FakeRng) -> Result<VarValue, VarError> {
    match def.name.as_str() {
        "email" => generate_email(def, rng),
        "password" => generate_password(def, rng),
        "uuid" => generate_uuid(rng),
        "number" => generate_number(def, rng),
        "one_of" => generate_one_of(def, rng),
        "sentence" => generate_sentence(def, rng),
        "timestamp" => generate_timestamp(def, rng),
        "phone" => crate::geo::generate_phone(&def.params, rng),
        _ => Err(VarError::Other(format!("unknown generator: {}", def.name))),
    }
}

/// Generate a random email address.
///
/// Params:
/// - `prefix`: prepended to the random part (e.g. "test+" produces "test+abc123@example.com")
/// - `domain`: replaces "example.com"
fn generate_email(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let charset: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    // 10–14 chars from a 36-char set ≈ 52–72 bits of entropy — comfortably
    // collision-free for golem's scale (well under a few thousand addresses a
    // day, often with test-data cleanup), without producing absurdly long
    // locals. Use `prefix=user+` for real-inbox plus-addressing.
    let len = rng.gen_range(10..=14);
    let random_part: String = (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..charset.len());
            charset[idx] as char
        })
        .collect();

    let prefix = def.params.get("prefix").cloned().unwrap_or_default();
    let domain = def
        .params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "example.com".to_string());

    let email = format!("{prefix}{random_part}@{domain}");
    Ok(VarValue::String(email))
}

/// Generate a random password.
///
/// Params:
/// - `length`: desired length (default 12)
/// - `symbols`: "true" or "false" (default "true") — whether to include symbols
fn generate_password(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let length: usize = def
        .params
        .get("length")
        .map(|v| {
            v.parse::<usize>()
                .map_err(|_| VarError::Other(format!("invalid length: {v}")))
        })
        .transpose()?
        .unwrap_or(12);

    let include_symbols = def
        .params
        .get("symbols")
        .map(|v| v != "false")
        .unwrap_or(true);

    let charset_no_symbols: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let charset_with_symbols: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()-_=+";

    let charset = if include_symbols {
        charset_with_symbols
    } else {
        charset_no_symbols
    };

    let password: String = (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..charset.len());
            charset[idx] as char
        })
        .collect();

    Ok(VarValue::String(password))
}

/// Generate a v4 UUID string from the seeded RNG, so `${fake:uuid}` reproduces
/// under `--seed N` like every other generator — `Uuid::new_v4` would instead
/// draw OS entropy and break determinism.
fn generate_uuid(rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let bytes: [u8; 16] = rng.gen();
    // `from_random_bytes` sets the v4 version and RFC 4122 variant bits.
    let id: Uuid = uuid::Builder::from_random_bytes(bytes).into_uuid();
    Ok(VarValue::String(id.to_string()))
}

/// Generate a random integer in a range.
///
/// Params:
/// - `min`: minimum value (default 0)
/// - `max`: maximum value (default 100)
fn generate_number(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let min: i64 = def
        .params
        .get("min")
        .map(|v| {
            v.parse::<i64>()
                .map_err(|_| VarError::Other(format!("invalid min: {v}")))
        })
        .transpose()?
        .unwrap_or(0);

    let max: i64 = def
        .params
        .get("max")
        .map(|v| {
            v.parse::<i64>()
                .map_err(|_| VarError::Other(format!("invalid max: {v}")))
        })
        .transpose()?
        .unwrap_or(100);

    if min > max {
        return Err(VarError::Other(format!(
            "min ({min}) must be <= max ({max})"
        )));
    }

    let n = rng.gen_range(min..=max);
    Ok(VarValue::String(n.to_string()))
}

/// Pick one value at random from a caller-supplied set:
/// `${fake:one_of(free|pro|enterprise)}`. Choices are the generator's
/// positional args, each further split on `|`, so both `one_of(a|b|c)` and
/// `one_of(a, b, c)` work. The pick is seeded like every other generator.
fn generate_one_of(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let choices: Vec<&str> = def
        .positional
        .iter()
        .flat_map(|arg| arg.split('|'))
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();

    if choices.is_empty() {
        return Err(VarError::Other(
            "one_of requires at least one choice, e.g. one_of(a|b|c)".to_string(),
        ));
    }

    let pick = choices[rng.gen_range(0..choices.len())];
    Ok(VarValue::String(pick.to_string()))
}

/// Generate a single sentence in the requested language.
///
/// Param `language` (ISO 639-1, e.g. `fr`/`ja`/`ar`) selects a per-language
/// template set; omitting it uses the language-neutral `lorem` default. An
/// unrecognised language is an error (like `country=`). A random pattern is
/// chosen and its `{slot}` placeholders filled with seeded random words — all
/// joining / punctuation / script behaviour lives in the data
/// (`data/sentences/*.json`), so this engine is language-agnostic.
fn generate_sentence(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let lang = def
        .params
        .get("language")
        .map(|s| s.as_str())
        .unwrap_or("lorem");

    let data = sentence_database()
        .get(lang)
        .ok_or_else(|| VarError::Other(format!("unsupported language: {lang}")))?;

    if data.patterns.is_empty() {
        return Err(VarError::Other(format!("no sentence patterns for {lang}")));
    }
    let pattern = &data.patterns[rng.gen_range(0..data.patterns.len())];

    // Fill each `{slot}` with a random word from that slot's list.
    let mut result = String::with_capacity(pattern.len());
    let mut rest = pattern.as_str();
    while let Some(open) = rest.find('{') {
        result.push_str(&rest[..open]);
        let close = rest[open..]
            .find('}')
            .ok_or_else(|| VarError::Other(format!("malformed sentence pattern: {pattern}")))?;
        let slot = &rest[open + 1..open + close];
        let words = data
            .slots
            .get(slot)
            .filter(|w| !w.is_empty())
            .ok_or_else(|| {
                VarError::Other(format!("sentence slot {{{slot}}} has no words ({lang})"))
            })?;
        result.push_str(&words[rng.gen_range(0..words.len())]);
        rest = &rest[open + close + 1..];
    }
    result.push_str(rest);

    Ok(VarValue::String(capitalize_first(&result)))
}

/// Capitalise the first character (a no-op for caseless scripts like CJK and
/// Arabic), so authored data can store leading articles lower-case
/// (`le golem` → `Le golem`).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Subtract `years` whole calendar years from `dt` (clamping Feb-29 → Feb-28).
fn sub_years(dt: DateTime<Utc>, years: u32) -> DateTime<Utc> {
    dt.checked_sub_months(Months::new(years.saturating_mul(12)))
        .unwrap_or(dt)
}

/// Generate a timestamp **object** anchored on the run's reference instant
/// (`rng.anchor()` — see [`FakeRng`]), so it is seed-reproducible yet tracks
/// real "now" on a no-`--seed` run.
///
/// Fields: `datetime` (ISO 8601), `date` (`YYYY-MM-DD`), `time` (`HH:MM`), and
/// the zero-padded parts `year` / `month` / `day` — the parts suit forms with
/// separate date inputs (e.g. a three-field date of birth).
///
/// Window params, both counted **back from the anchor in whole years**:
/// - `max_years` — the oldest the date may be (far edge; default 1).
/// - `min_years` — the youngest (near edge; default 0, i.e. the anchor).
///
/// The date is drawn uniformly from `[anchor - max_years, anchor - min_years]`.
/// Default (no params) → within the last year. A date of birth is just a window
/// pushed back: `min_years=18, max_years=90`. Reversed bounds are swapped.
fn generate_timestamp(def: &GeneratorDef, rng: &mut FakeRng) -> Result<VarValue, VarError> {
    let anchor = rng.anchor();

    let parse_years = |key: &str, default: u32| -> Result<u32, VarError> {
        def.params
            .get(key)
            .map(|v| {
                v.parse::<u32>()
                    .map_err(|_| VarError::Other(format!("invalid {key}: {v}")))
            })
            .transpose()
            .map(|opt| opt.unwrap_or(default))
    };

    let min_years = parse_years("min_years", 0)?;
    let max_years = parse_years("max_years", 1)?;
    let (min_years, max_years) = if min_years > max_years {
        (max_years, min_years)
    } else {
        (min_years, max_years)
    };

    // [start, end] is the window a date is drawn from, uniformly: `max_years`
    // ago (oldest) up to `min_years` ago (youngest).
    let start = sub_years(anchor, max_years);
    let end = sub_years(anchor, min_years);

    let span_secs = (end - start).num_seconds().max(1);
    let offset_secs = rng.gen_range(0..span_secs);
    let ts = start + Duration::seconds(offset_secs);

    Ok(VarValue::object(vec![
        ("datetime", VarValue::string(ts.to_rfc3339())),
        ("date", VarValue::string(ts.format("%Y-%m-%d").to_string())),
        ("time", VarValue::string(ts.format("%H:%M").to_string())),
        ("year", VarValue::string(ts.format("%Y").to_string())),
        ("month", VarValue::string(ts.format("%m").to_string())),
        ("day", VarValue::string(ts.format("%d").to_string())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // 1. email generation matches *@example.com pattern
    #[test]
    fn email_matches_example_com_pattern() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("email"), &mut rng).expect("should generate");
        let email = result.as_str().expect("should be string");
        assert!(email.ends_with("@example.com"), "got: {email}");
        assert!(email.len() > "@example.com".len(), "SHALL have local part");
    }

    // 2. email with prefix/domain
    #[test]
    fn email_with_prefix_and_domain() {
        let mut rng = seeded_rng();
        let d = def_with_params("email", &[("prefix", "test"), ("domain", "acme.com")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let email = result.as_str().expect("should be string");
        assert!(
            email.starts_with("test"),
            "SHALL start with prefix, got: {email}"
        );
        assert!(
            email.ends_with("@acme.com"),
            "SHALL end with @acme.com, got: {email}"
        );
    }

    // 3. email with plus addressing
    #[test]
    fn email_with_plus_addressing() {
        let mut rng = seeded_rng();
        let d = def_with_params("email", &[("prefix", "user+tag")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let email = result.as_str().expect("should be string");
        assert!(
            email.starts_with("user+tag"),
            "SHALL start with user+tag, got: {email}"
        );
        assert!(email.contains('+'), "SHALL contain +, got: {email}");
    }

    // 6. password default — 12+ chars, contains letters and digits
    #[test]
    fn password_default_length_and_content() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("password"), &mut rng).expect("should generate");
        let pw = result.as_str().expect("should be string");
        assert!(pw.len() >= 12, "expected >= 12 chars, got {}", pw.len());
        assert!(
            pw.chars().any(|c| c.is_ascii_alphabetic()),
            "SHALL contain letters"
        );
        assert!(
            pw.chars().any(|c| c.is_ascii_digit()),
            "SHALL contain digits"
        );
    }

    // 7. password custom length
    #[test]
    fn password_custom_length() {
        let mut rng = seeded_rng();
        let d = def_with_params("password", &[("length", "20")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let pw = result.as_str().expect("should be string");
        assert_eq!(pw.len(), 20, "expected 20 chars, got {}", pw.len());
    }

    // 8. password no symbols — only letters and digits
    #[test]
    fn password_no_symbols() {
        let mut rng = seeded_rng();
        let d = def_with_params("password", &[("symbols", "false")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let pw = result.as_str().expect("should be string");
        assert!(
            pw.chars().all(|c| c.is_ascii_alphanumeric()),
            "should only contain alphanumeric chars, got: {pw}"
        );
    }

    // 9. uuid matches UUID v4 format
    #[test]
    fn uuid_matches_v4_format() {
        let result = generate_simple(&def("uuid"), &mut seeded_rng()).expect("should generate");
        let id = result.as_str().expect("should be string");
        // UUID v4 format: 8-4-4-4-12 hex digits
        let parsed = Uuid::parse_str(id);
        assert!(parsed.is_ok(), "SHALL parse as UUID, got: {id}");
        let parsed = parsed.expect("already checked");
        assert_eq!(
            parsed.get_version(),
            Some(uuid::Version::Random),
            "should be v4"
        );
    }

    // 10. number range — within specified range
    #[test]
    fn number_within_range() {
        let mut rng = seeded_rng();
        let d = def_with_params("number", &[("min", "10"), ("max", "20")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let s = result.as_str().expect("should be string");
        let n: i64 = s.parse().expect("should parse as integer");
        assert!((10..=20).contains(&n), "expected 10..=20, got {n}");
    }

    /// Read a string field from a timestamp/object value.
    fn ts_field(val: &VarValue, key: &str) -> String {
        val.as_object()
            .unwrap_or_else(|| panic!("timestamp SHALL be an object"))
            .get(key)
            .unwrap_or_else(|| panic!("missing field: {key}"))
            .as_str()
            .unwrap_or_else(|| panic!("field {key} SHALL be a string"))
            .to_string()
    }

    // 11. timestamp is an object whose datetime/date/time/parts agree and parse.
    #[test]
    fn timestamp_object_fields_are_consistent() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("timestamp"), &mut rng).expect("should generate");

        let datetime = ts_field(&result, "datetime");
        let parsed = chrono::DateTime::parse_from_rfc3339(&datetime)
            .expect("datetime SHALL parse as ISO 8601");

        let date = ts_field(&result, "date");
        let (year, month, day) = (
            ts_field(&result, "year"),
            ts_field(&result, "month"),
            ts_field(&result, "day"),
        );
        assert_eq!(date, format!("{year}-{month}-{day}"), "parts SHALL build date");
        assert_eq!(month.len(), 2, "month SHALL be zero-padded, got: {month}");
        assert_eq!(day.len(), 2, "day SHALL be zero-padded, got: {day}");
        assert_eq!(
            date,
            parsed.format("%Y-%m-%d").to_string(),
            "date SHALL match datetime"
        );
        assert_eq!(
            ts_field(&result, "time"),
            parsed.format("%H:%M").to_string(),
            "time SHALL match datetime"
        );
    }

    // 11b. min_years/max_years push the window back (date-of-birth use).
    #[test]
    fn timestamp_year_window_pushed_back() {
        use chrono::Datelike;
        let mut rng = seeded_rng();
        let d = def_with_params("timestamp", &[("min_years", "18"), ("max_years", "40")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let birth_year: i32 = ts_field(&result, "year").parse().expect("year");
        let anchor_year = seeded_rng().anchor().year();
        let age = anchor_year - birth_year;
        // Whole-year-bucketed window, so allow the boundary years inclusively.
        assert!(
            (17..=41).contains(&age),
            "date SHALL be ~[18,40] years before anchor {anchor_year}, birth {birth_year}"
        );
    }

    // 12. sentence — non-empty string
    #[test]
    fn sentence_non_empty() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("sentence"), &mut rng).expect("should generate");
        let s = result.as_str().expect("should be string");
        assert!(!s.is_empty());
        assert!(s.ends_with('.'), "sentence SHALL end with period, got: {s}");
    }

    // 13. unknown generator — error
    #[test]
    fn unknown_generator_returns_error() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("nonexistent"), &mut rng);
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        assert!(
            err.to_string().contains("unknown generator"),
            "expected 'unknown generator' error, got: {err}"
        );
    }

    // 14. Two calls with same RNG seed produce same email (determinism)
    #[test]
    fn deterministic_with_same_seed() {
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();
        let email1 = generate_simple(&def("email"), &mut rng1).expect("should generate");
        let email2 = generate_simple(&def("email"), &mut rng2).expect("should generate");
        assert_eq!(email1, email2, "same seed SHALL produce same output");
    }

    // 15. Two calls advance RNG (different values)
    #[test]
    fn successive_calls_produce_different_values() {
        let mut rng = seeded_rng();
        let email1 = generate_simple(&def("email"), &mut rng).expect("should generate");
        let email2 = generate_simple(&def("email"), &mut rng).expect("should generate");
        assert_ne!(
            email1, email2,
            "successive calls should produce different values"
        );
    }

    // 18. email local part (before prefix/domain) uses only [a-z0-9] charset
    #[test]
    fn email_random_part_charset() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("email"), &mut rng).expect("should generate");
        let email = result.as_str().expect("should be string");
        let local = email
            .strip_suffix("@example.com")
            .expect("SHALL end with @example.com");
        assert!(!local.is_empty(), "SHALL have a non-empty local part");
        assert!(
            local
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "random part SHALL be lowercase letters and digits only, got: {local}"
        );
    }

    // 19. password with explicit symbols=true draws from a charset that includes symbols
    #[test]
    fn password_with_symbols_can_include_symbols() {
        // Probe many seeds: at least one SHALL produce a symbol when symbols enabled.
        let symbols = "!@#$%^&*()-_=+";
        let mut saw_symbol = false;
        for seed in 0u64..50 {
            let mut rng = FakeRng::from_seed(seed);
            let d = def_with_params("password", &[("symbols", "true"), ("length", "20")]);
            let result = generate_simple(&d, &mut rng).expect("should generate");
            let pw = result.as_str().expect("should be string").to_string();
            if pw.chars().any(|c| symbols.contains(c)) {
                saw_symbol = true;
                break;
            }
        }
        assert!(
            saw_symbol,
            "symbols=true SHALL be able to emit symbol characters"
        );
    }

    // 20. password symbols param treats any non-"false" value as enabling symbols
    #[test]
    fn password_symbols_non_false_enables_symbols() {
        // Only the literal "false" disables symbols; e.g. "no" keeps them on.
        let symbols = "!@#$%^&*()-_=+";
        let mut saw_symbol = false;
        for seed in 0u64..50 {
            let mut r = FakeRng::from_seed(seed);
            let d = def_with_params("password", &[("symbols", "no"), ("length", "20")]);
            let result = generate_simple(&d, &mut r).expect("should generate");
            let pw = result.as_str().expect("should be string").to_string();
            if pw.chars().any(|c| symbols.contains(c)) {
                saw_symbol = true;
                break;
            }
        }
        assert!(
            saw_symbol,
            "symbols param other than \"false\" SHALL keep symbols enabled"
        );
    }

    // 21. password length=0 yields an empty string
    #[test]
    fn password_zero_length_is_empty() {
        let mut rng = seeded_rng();
        let d = def_with_params("password", &[("length", "0")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let pw = result.as_str().expect("should be string");
        assert!(
            pw.is_empty(),
            "length=0 SHALL produce empty password, got: {pw}"
        );
    }

    // 22. password invalid (non-numeric) length is an error mentioning length
    #[test]
    fn password_invalid_length_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("password", &[("length", "abc")]);
        let err = generate_simple(&d, &mut rng).expect_err("SHALL error on bad length");
        assert!(
            err.to_string().contains("invalid length"),
            "SHALL report invalid length, got: {err}"
        );
    }

    // 23. number with no params uses default range 0..=100
    #[test]
    fn number_default_range() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("number"), &mut rng).expect("should generate");
        let n: i64 = result
            .as_str()
            .expect("should be string")
            .parse()
            .expect("should parse as integer");
        assert!((0..=100).contains(&n), "default SHALL be 0..=100, got {n}");
    }

    // 24. number with min == max always returns that exact value
    #[test]
    fn number_min_equals_max() {
        let mut rng = seeded_rng();
        let d = def_with_params("number", &[("min", "7"), ("max", "7")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        assert_eq!(
            result.as_str().expect("should be string"),
            "7",
            "min==max SHALL pin the value"
        );
    }

    // 25. number supports negative ranges
    #[test]
    fn number_negative_range() {
        let mut rng = seeded_rng();
        let d = def_with_params("number", &[("min", "-10"), ("max", "-5")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let n: i64 = result
            .as_str()
            .expect("should be string")
            .parse()
            .expect("should parse as integer");
        assert!((-10..=-5).contains(&n), "expected -10..=-5, got {n}");
    }

    // 26. number with min > max is an error
    #[test]
    fn number_min_greater_than_max_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("number", &[("min", "50"), ("max", "10")]);
        let err = generate_simple(&d, &mut rng).expect_err("SHALL error when min > max");
        assert!(
            err.to_string().contains("must be <= max"),
            "SHALL report ordering constraint, got: {err}"
        );
    }

    // 27. number with invalid (non-numeric) min is an error
    #[test]
    fn number_invalid_min_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("number", &[("min", "xyz")]);
        let err = generate_simple(&d, &mut rng).expect_err("SHALL error on bad min");
        assert!(
            err.to_string().contains("invalid min"),
            "SHALL report invalid min, got: {err}"
        );
    }

    // 28. number with invalid (non-numeric) max is an error
    #[test]
    fn number_invalid_max_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("number", &[("max", "1.5")]);
        let err = generate_simple(&d, &mut rng).expect_err("SHALL error on bad max");
        assert!(
            err.to_string().contains("invalid max"),
            "SHALL report invalid max, got: {err}"
        );
    }

    // 29. sentence: default is lorem; English is capitalised and ends with '.'.
    #[test]
    fn sentence_default_lorem_and_english() {
        let mut rng = seeded_rng();
        let lorem = generate_simple(&def("sentence"), &mut rng).expect("should generate");
        let s = lorem.as_str().expect("string");
        assert!(s.starts_with("Lorem") || s.starts_with("Sed") || s.starts_with("Dolor"));
        assert!(s.ends_with('.'), "lorem SHALL end with '.', got: {s}");

        let en = def_with_params("sentence", &[("language", "en")]);
        let result = generate_simple(&en, &mut rng).expect("should generate");
        let s = result.as_str().expect("string");
        assert!(s.ends_with('.'), "en SHALL end with '.', got: {s}");
        let first = s.chars().next().expect("non-empty");
        assert!(first.is_uppercase(), "en SHALL be capitalised, got: {s}");
    }

    // 29b. ja joins with no ASCII spaces and uses the CJK full-stop.
    #[test]
    fn sentence_ja_no_spaces_cjk() {
        let mut rng = seeded_rng();
        let d = def_with_params("sentence", &[("language", "ja")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let s = result.as_str().expect("string");
        assert!(!s.contains(' '), "ja SHALL have no ASCII spaces, got: {s}");
        assert!(s.ends_with('。'), "ja SHALL end with '。', got: {s}");
        assert!(
            s.chars().any(|c| ('\u{3040}'..='\u{9fff}').contains(&c)),
            "ja SHALL contain kana/kanji, got: {s}"
        );
    }

    // 29c. ar produces right-to-left Arabic-block text.
    #[test]
    fn sentence_ar_is_arabic_script() {
        let mut rng = seeded_rng();
        let d = def_with_params("sentence", &[("language", "ar")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let s = result.as_str().expect("string");
        assert!(
            s.chars().any(|c| ('\u{0600}'..='\u{06ff}').contains(&c)),
            "ar SHALL contain Arabic-block characters, got: {s}"
        );
    }

    // 29d. fr is capitalised (article baked lower-case, capitalised in code).
    #[test]
    fn sentence_fr_capitalised() {
        let mut rng = seeded_rng();
        let d = def_with_params("sentence", &[("language", "fr")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let s = result.as_str().expect("string");
        let first = s.chars().next().expect("non-empty");
        assert!(first.is_uppercase(), "fr SHALL be capitalised, got: {s}");
    }

    // 29e. unknown language errors; same seed reproduces the same sentence.
    #[test]
    fn sentence_unknown_language_errors_and_is_seeded() {
        let mut rng = seeded_rng();
        let bad = def_with_params("sentence", &[("language", "zz")]);
        let err = generate_simple(&bad, &mut rng).expect_err("unknown language SHALL error");
        assert!(
            err.to_string().contains("unsupported language: zz"),
            "got: {err}"
        );

        let a = generate_simple(&def("sentence"), &mut seeded_rng()).expect("gen");
        let b = generate_simple(&def("sentence"), &mut seeded_rng()).expect("gen");
        assert_eq!(a, b, "same seed SHALL reproduce the same sentence");
    }

    // 30. timestamp is seed-reproducible and falls within the year before the
    //     run's anchor instant (`rng.anchor()`) — NOT relative to wall-clock
    //     `now`. The dependence on `Utc::now()` was the determinism bug; the
    //     anchor now rides inside the seed (see `FakeRng`).
    #[test]
    fn timestamp_seeded_and_within_anchor_year() {
        // Same seed SHALL reproduce the same timestamp.
        let a = generate_simple(&def("timestamp"), &mut seeded_rng()).expect("should generate");
        let b = generate_simple(&def("timestamp"), &mut seeded_rng()).expect("should generate");
        assert_eq!(a, b, "same seed SHALL reproduce the same timestamp");

        let ts = ts_field(&a, "datetime");
        let parsed = chrono::DateTime::parse_from_rfc3339(&ts).expect("SHALL parse as RFC3339");
        let anchor = seeded_rng().anchor().fixed_offset();
        assert!(
            parsed <= anchor,
            "timestamp SHALL be at or before the anchor, got: {ts}"
        );
        let one_year_before = anchor - Duration::seconds(365 * 24 * 3600);
        assert!(
            parsed >= one_year_before,
            "timestamp SHALL be within the year before the anchor, got: {ts}"
        );
    }

    // 31. uuid is drawn from the seeded RNG: reproducible across same-seed runs,
    //     distinct on successive draws, and a valid v4 string.
    #[test]
    fn uuid_is_seeded_and_valid_v4() {
        // 1. Same seed SHALL reproduce the same UUID (the determinism fix —
        //    Uuid::new_v4 would draw OS entropy and fail this).
        let a = generate_simple(&def("uuid"), &mut seeded_rng()).expect("should generate");
        let b = generate_simple(&def("uuid"), &mut seeded_rng()).expect("should generate");
        assert_eq!(a, b, "same seed SHALL reproduce the same UUID");

        // 2. Successive draws from one RNG SHALL differ (the stream advances).
        let mut rng = seeded_rng();
        let c = generate_simple(&def("uuid"), &mut rng).expect("should generate");
        let d = generate_simple(&def("uuid"), &mut rng).expect("should generate");
        assert_ne!(c, d, "successive draws SHALL produce distinct UUIDs");

        // 3. Each output SHALL be the hyphenated v4 string form.
        for value in [&a, &c, &d] {
            let id = value.as_str().expect("uuid SHALL be a string");
            let parsed = Uuid::parse_str(id).expect("uuid output SHALL parse as a UUID");
            assert_eq!(
                parsed.get_version(),
                Some(uuid::Version::Random),
                "uuid output SHALL be v4, got: {id}"
            );
            assert_eq!(id.len(), 36, "hyphenated v4 SHALL be 36 chars, got: {id}");
            assert_eq!(
                id.matches('-').count(),
                4,
                "hyphenated v4 SHALL have four hyphens, got: {id}"
            );
        }
    }

    // 32. dispatch routes "phone" to geo::generate_phone, producing a phone-shaped value.
    #[test]
    fn dispatch_routes_phone() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("phone"), &mut rng).expect("phone SHALL generate");
        let s = result.as_str().expect("should be string");
        // 1. Every phone format expands at least one '#' into a digit, so the routed
        //    phone arm SHALL yield a value containing digits (a wrong arm — e.g. city —
        //    would not be guaranteed to).
        assert!(
            s.chars().any(|c| c.is_ascii_digit()),
            "phone SHALL contain digits, got: {s}"
        );
    }

    // 32b. phone with a present-but-unknown country code errors; an absent
    //      country picks one at random (succeeds).
    #[test]
    fn phone_unknown_country_errors_unset_is_random() {
        let mut rng = seeded_rng();
        let bad = def_with_params("phone", &[("country", "ZZ")]);
        let err = generate_simple(&bad, &mut rng).expect_err("unknown country SHALL error");
        assert!(
            err.to_string().contains("unknown country code: ZZ"),
            "got: {err}"
        );
        generate_simple(&def("phone"), &mut rng).expect("unset country SHALL generate");
    }

    // 33. city/postcode/street were removed — use fake:address dot-notation.
    //     Their names now fall through to the unknown-generator arm.
    #[test]
    fn removed_geo_scalars_are_unknown() {
        let mut rng = seeded_rng();
        for name in ["city", "postcode", "street"] {
            let err = generate_simple(&def(name), &mut rng)
                .expect_err("removed generator SHALL be unknown");
            assert!(
                err.to_string().contains("unknown generator"),
                "{name}: got {err}"
            );
        }
    }

    // 34. one_of picks a member of the set (pipe syntax), seed-reproducibly.
    #[test]
    fn one_of_picks_from_set_and_reproduces() {
        let parsed = GeneratorDef::parse("fake:one_of(free|pro|enterprise)").expect("parse");
        let a = generate_simple(&parsed, &mut seeded_rng()).expect("generate");
        let b = generate_simple(&parsed, &mut seeded_rng()).expect("generate");
        assert_eq!(a, b, "same seed SHALL reproduce the same pick");
        let picked = a.as_str().expect("string");
        assert!(
            ["free", "pro", "enterprise"].contains(&picked),
            "SHALL pick a set member, got: {picked}"
        );
    }

    // 34b. Comma-separated choices work too (no key=value), and trim.
    #[test]
    fn one_of_accepts_comma_list() {
        let parsed = GeneratorDef::parse("fake:one_of(red, green, blue)").expect("parse");
        let mut rng = seeded_rng();
        let picked = generate_simple(&parsed, &mut rng).expect("generate");
        assert!(
            ["red", "green", "blue"].contains(&picked.as_str().expect("string")),
            "SHALL pick a trimmed comma-list member, got: {picked:?}"
        );
    }

    // 34c. one_of with no choices is an error, not a panic.
    #[test]
    fn one_of_empty_errors() {
        let parsed = GeneratorDef::parse("fake:one_of()").expect("parse");
        let mut rng = seeded_rng();
        let err = generate_simple(&parsed, &mut rng).expect_err("empty one_of SHALL error");
        assert!(
            err.to_string().contains("one_of requires at least one choice"),
            "got: {err}"
        );
    }
}
