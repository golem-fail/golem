use crate::{GeneratorDef, VarError, VarValue};
use chrono::{Duration, Utc};
use rand::Rng;
use uuid::Uuid;

/// Generate a simple (non-structured) fake value from a generator definition.
pub fn generate_simple(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue, VarError> {
    match def.name.as_str() {
        "email" => generate_email(def, rng),
        "first_name" => generate_first_name(rng),
        "last_name" => generate_last_name(rng),
        "password" => generate_password(def, rng),
        "uuid" => generate_uuid(),
        "number" => generate_number(def, rng),
        "sentence" => generate_sentence(rng),
        "timestamp" => generate_timestamp(rng),
        "phone" => crate::geo::generate_phone(&def.params, rng),
        "city" => crate::geo::generate_city(&def.params, rng),
        "postcode" => crate::geo::generate_postcode(&def.params, rng),
        "street" => crate::geo::generate_street(&def.params, rng),
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
    let len = rng.gen_range(4..=6);
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

const FIRST_NAMES: &[&str] = &[
    "Alice", "Bob", "Carol", "David", "Emma", "Frank", "Grace", "Henry", "Iris", "Jack", "Karen",
    "Leo", "Mia", "Noah", "Olivia", "Paul", "Quinn", "Rachel", "Sam", "Tara", "Uma", "Victor",
    "Wendy", "Xavier", "Yara", "Zach",
];

/// Pick a random first name from a hardcoded list.
fn generate_first_name(rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let idx = rng.gen_range(0..FIRST_NAMES.len());
    Ok(VarValue::String(FIRST_NAMES[idx].to_string()))
}

const LAST_NAMES: &[&str] = &[
    "Adams", "Baker", "Clark", "Davis", "Evans", "Fisher", "Garcia", "Harris", "Irwin", "Jones",
    "Kim", "Lee", "Moore", "Nelson", "Owens", "Patel", "Quinn", "Reyes", "Smith", "Taylor",
    "Upton", "Vega", "Wang", "Young", "Zhang",
];

/// Pick a random last name from a hardcoded list.
fn generate_last_name(rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let idx = rng.gen_range(0..LAST_NAMES.len());
    Ok(VarValue::String(LAST_NAMES[idx].to_string()))
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

/// Generate a v4 UUID string.
fn generate_uuid() -> Result<VarValue, VarError> {
    let id = Uuid::new_v4();
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

const ADJECTIVES: &[&str] = &[
    "quick", "lazy", "happy", "sad", "bright", "dark", "warm", "cold", "tall", "small",
];

const NOUNS: &[&str] = &[
    "fox", "dog", "cat", "bird", "tree", "river", "cloud", "mountain", "garden", "bridge",
];

const VERBS: &[&str] = &[
    "runs", "jumps", "sleeps", "flies", "grows", "sings", "dances", "hides", "watches", "waits",
];

/// Generate a simple sentence using "The [adj] [noun] [verb]." pattern.
fn generate_sentence(rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    let verb = VERBS[rng.gen_range(0..VERBS.len())];
    Ok(VarValue::String(format!("The {adj} {noun} {verb}.")))
}

/// Generate a random ISO 8601 timestamp within the last year.
fn generate_timestamp(rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let now = Utc::now();
    let one_year_secs = 365 * 24 * 3600;
    let offset_secs = rng.gen_range(0..one_year_secs);
    let ts = now - Duration::seconds(offset_secs);
    Ok(VarValue::String(ts.to_rfc3339()))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // 1. email generation matches *@example.com pattern
    #[test]
    fn email_matches_example_com_pattern() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("email"), &mut rng).expect("should generate");
        let email = result.as_str().expect("should be string");
        assert!(email.ends_with("@example.com"), "got: {email}");
        assert!(email.len() > "@example.com".len(), "should have local part");
    }

    // 2. email with prefix/domain
    #[test]
    fn email_with_prefix_and_domain() {
        let mut rng = seeded_rng();
        let d = def_with_params("email", &[("prefix", "test"), ("domain", "acme.com")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let email = result.as_str().expect("should be string");
        assert!(email.starts_with("test"), "should start with prefix, got: {email}");
        assert!(email.ends_with("@acme.com"), "should end with @acme.com, got: {email}");
    }

    // 3. email with plus addressing
    #[test]
    fn email_with_plus_addressing() {
        let mut rng = seeded_rng();
        let d = def_with_params("email", &[("prefix", "user+tag")]);
        let result = generate_simple(&d, &mut rng).expect("should generate");
        let email = result.as_str().expect("should be string");
        assert!(email.starts_with("user+tag"), "should start with user+tag, got: {email}");
        assert!(email.contains('+'), "should contain +, got: {email}");
    }

    // 4. first_name returns non-empty string
    #[test]
    fn first_name_returns_non_empty() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("first_name"), &mut rng).expect("should generate");
        let name = result.as_str().expect("should be string");
        assert!(!name.is_empty());
    }

    // 5. last_name returns non-empty string
    #[test]
    fn last_name_returns_non_empty() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("last_name"), &mut rng).expect("should generate");
        let name = result.as_str().expect("should be string");
        assert!(!name.is_empty());
    }

    // 6. password default — 12+ chars, contains letters and digits
    #[test]
    fn password_default_length_and_content() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("password"), &mut rng).expect("should generate");
        let pw = result.as_str().expect("should be string");
        assert!(pw.len() >= 12, "expected >= 12 chars, got {}", pw.len());
        assert!(pw.chars().any(|c| c.is_ascii_alphabetic()), "should contain letters");
        assert!(pw.chars().any(|c| c.is_ascii_digit()), "should contain digits");
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
        assert!(parsed.is_ok(), "should parse as UUID, got: {id}");
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

    // 11. timestamp matches ISO 8601 format
    #[test]
    fn timestamp_matches_iso8601() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("timestamp"), &mut rng).expect("should generate");
        let ts = result.as_str().expect("should be string");
        let parsed = chrono::DateTime::parse_from_rfc3339(ts);
        assert!(parsed.is_ok(), "should parse as ISO 8601, got: {ts}");
    }

    // 12. sentence — non-empty string
    #[test]
    fn sentence_non_empty() {
        let mut rng = seeded_rng();
        let result = generate_simple(&def("sentence"), &mut rng).expect("should generate");
        let s = result.as_str().expect("should be string");
        assert!(!s.is_empty());
        assert!(s.ends_with('.'), "sentence should end with period, got: {s}");
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
        assert_eq!(email1, email2, "same seed should produce same output");
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
}
