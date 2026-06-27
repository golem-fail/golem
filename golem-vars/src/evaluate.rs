use std::collections::HashMap;

use anyhow::{anyhow, Result};
use rand::Rng;

use crate::generators::generate_simple;
use crate::structured::generate_structured;
use crate::{GeneratorDef, VarError, VarValue};

/// Generate a value from a `fake:NAME(args)` definition string (with NO
/// trailing `.field` — split that off with [`split_fake_ref`] first).
/// Tries structured generators (person/address/credit_card/…) first, then
/// simple ones (email/uuid/…). Public so callers that own an RNG (the
/// runner) can supply generation to the interpolation context.
pub fn generate_fake(def_str: &str, rng: &mut impl Rng) -> Result<VarValue, VarError> {
    let def = GeneratorDef::parse(def_str)
        .ok_or_else(|| VarError::Other(format!("invalid generator: {def_str}")))?;
    match generate_structured(&def, rng) {
        Ok(val) => Ok(val),
        Err(_) => generate_simple(&def, rng),
    }
}

/// Split a `fake:` reference into the generator definition and an optional
/// trailing `.field.path`. Parenthesis-aware so a `.` inside args (e.g.
/// `fake:address(format=a.b)`) is not mistaken for a field separator.
///
/// - `"fake:email"` → (`"fake:email"`, None)
/// - `"fake:person(country=JP)"` → (`"fake:person(country=JP)"`, None)
/// - `"fake:person(country=JP).first"` → (`"fake:person(country=JP)"`, Some("first"))
/// - `"fake:uuid.short"` → (`"fake:uuid"`, Some("short"))
pub fn split_fake_ref(reference: &str) -> (&str, Option<&str>) {
    // Find where the generator definition ends. With args, that's the
    // matching close paren; without, the first '.'.
    if let Some(open) = reference.find('(') {
        let mut depth = 0usize;
        for (i, c) in reference.char_indices().skip(open) {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let def = &reference[..=i];
                        let rest = &reference[i + 1..];
                        return match rest.strip_prefix('.') {
                            Some(field) if !field.is_empty() => (def, Some(field)),
                            _ => (def, None),
                        };
                    }
                }
                _ => {}
            }
        }
        // Unbalanced parens — let the generator parser report it.
        (reference, None)
    } else {
        match reference.split_once('.') {
            Some((def, field)) if !field.is_empty() => (def, Some(field)),
            _ => (reference, None),
        }
    }
}

/// Evaluate variable declarations, processing `${…}` references and
/// `${fake:…}` generators, in the order provided (which should match
/// declaration order so cross-references resolve).
///
/// A whole-value reference keeps its shape — `card = "${fake:credit_card()}"`
/// or `alias = "${card}"` stores the object. Embedded references
/// (`id = "u-${fake:uuid}"`) and plain values stringify; an object used in a
/// string context is an error. Generators draw from `rng`; `${var}` inside
/// generator args resolves against already-evaluated vars (correlated data).
pub fn evaluate_generators(
    vars: &[(String, String)],
    rng: &mut impl Rng,
) -> Result<HashMap<String, VarValue>> {
    use crate::interpolation::{evaluate_value, GeneratorResolver, InterpolationContext};
    use crate::{Scope, ScopeLevel, VariableStore};
    use std::cell::RefCell;

    // The generator callback owns the RNG via a RefCell so it can be a plain
    // `Fn` inside the (immutable) interpolation context.
    let rng_cell = RefCell::new(rng);
    let gen = |def: &str| generate_fake(def, &mut **rng_cell.borrow_mut());
    let gen_ref: &GeneratorResolver = &gen;

    // Evaluate each var against a store of those already evaluated, so later
    // declarations can reference earlier ones (incl. object fields).
    let mut store = VariableStore::new();
    store.push_scope(Scope::new(ScopeLevel::Generator));
    let mut results: HashMap<String, VarValue> = HashMap::new();

    for (name, value) in vars {
        let evaluated = {
            let mut ctx = InterpolationContext::new(&store);
            ctx.generator = Some(gen_ref);
            evaluate_value(value, &ctx)
                .map_err(|e| anyhow!("evaluating variable \"{name}\": {e}"))?
        };
        store.set_in_scope(ScopeLevel::Generator, name.clone(), evaluated.clone());
        results.insert(name.clone(), evaluated);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn seeded_rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(42)
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

    // 1. Simple non-generator vars are stored as strings
    #[test]
    fn plain_vars_stored_as_strings() {
        let mut rng = seeded_rng();
        let vars = vec![
            ("greeting".to_string(), "hello world".to_string()),
            ("name".to_string(), "Alice".to_string()),
        ];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        assert_eq!(
            result.get("greeting"),
            Some(&VarValue::string("hello world"))
        );
        assert_eq!(result.get("name"), Some(&VarValue::string("Alice")));
    }

    // 2. Generator vars are evaluated and stored
    #[test]
    fn generator_vars_are_evaluated() {
        let mut rng = seeded_rng();
        let vars = vec![("email".to_string(), "${fake:email}".to_string())];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        let email_val = result.get("email").expect("should have email");
        let email_str = email_val.as_str().expect("should be string");
        assert!(
            email_str.contains('@'),
            "SHALL be an email, got: {email_str}"
        );
    }

    // 3. Cross-reference: generator param references prior variable
    #[test]
    fn cross_reference_prior_string_variable() {
        let mut rng = seeded_rng();
        let vars = vec![
            ("domain".to_string(), "acme.com".to_string()),
            (
                "email".to_string(),
                "${fake:email(domain=${domain})}".to_string(),
            ),
        ];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        let email_val = result.get("email").expect("should have email");
        let email_str = email_val.as_str().expect("should be string");
        assert!(
            email_str.ends_with("@acme.com"),
            "should use resolved domain, got: {email_str}"
        );
    }

    // 4. Cross-reference: generator param references prior object field (dot-path)
    #[test]
    fn cross_reference_object_field_dot_path() {
        let mut rng = seeded_rng();
        let vars = vec![
            (
                "addr".to_string(),
                "${fake:address(country=JP)}".to_string(),
            ),
            (
                "phone".to_string(),
                "${fake:phone(country=${addr.country_code})}".to_string(),
            ),
        ];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        // addr should have country_code=JP
        let addr = result.get("addr").expect("should have addr");
        let cc = field(addr, "country_code");
        assert_eq!(cc, "JP");

        // phone should have been generated with country=JP
        let phone_val = result.get("phone").expect("should have phone");
        let phone_str = phone_val.as_str().expect("should be string");
        assert!(
            phone_str.starts_with("+81"),
            "JP phone should start with +81, got: {phone_str}"
        );
    }

    // 5. Cross-reference: multiple generators chained
    #[test]
    fn multiple_generators_chained() {
        let mut rng = seeded_rng();
        let vars = vec![
            (
                "addr".to_string(),
                "${fake:address(country=JP)}".to_string(),
            ),
            (
                "phone".to_string(),
                "${fake:phone(country=${addr.country_code})}".to_string(),
            ),
            (
                "city2".to_string(),
                "${fake:city(country=${addr.country_code})}".to_string(),
            ),
            (
                "person".to_string(),
                "${fake:person(country=${addr.country_code})}".to_string(),
            ),
        ];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        assert_eq!(result.len(), 4);

        // Phone should be JP
        let phone = result.get("phone").expect("should have phone");
        let phone_str = phone.as_str().expect("should be string");
        assert!(
            phone_str.starts_with("+81"),
            "phone should be JP, got: {phone_str}"
        );

        // city2 should be a non-empty string
        let city2 = result.get("city2").expect("should have city2");
        let city_str = city2.as_str().expect("should be string");
        assert!(!city_str.is_empty(), "city2 SHALL NOT be empty");

        // person should be an object with JP name ordering
        let person = result.get("person").expect("should have person");
        assert!(person.as_object().is_some(), "person SHALL be an object");
    }

    // 6. Error when referencing undefined variable
    #[test]
    fn error_on_undefined_variable() {
        let mut rng = seeded_rng();
        let vars = vec![(
            "email".to_string(),
            "${fake:email(domain=${missing_var})}".to_string(),
        )];
        let result = evaluate_generators(&vars, &mut rng);

        assert!(result.is_err());
        let err = result.expect_err("should be error");
        assert!(
            err.to_string().contains("undefined variable"),
            "expected 'undefined variable' error, got: {err}"
        );
    }

    // 7. Error when referencing not-yet-evaluated variable (forward reference)
    #[test]
    fn error_on_forward_reference() {
        let mut rng = seeded_rng();
        // phone references addr, but addr is declared after phone
        let vars = vec![
            (
                "phone".to_string(),
                "${fake:phone(country=${addr.country_code})}".to_string(),
            ),
            (
                "addr".to_string(),
                "${fake:address(country=JP)}".to_string(),
            ),
        ];
        let result = evaluate_generators(&vars, &mut rng);

        assert!(result.is_err());
        let err = result.expect_err("should be error");
        assert!(
            err.to_string().contains("undefined variable"),
            "expected 'undefined variable' error for forward reference, got: {err}"
        );
    }

    // 8. Mixed generators and plain vars in correct order
    #[test]
    fn mixed_generators_and_plain_vars() {
        let mut rng = seeded_rng();
        let vars = vec![
            ("base_url".to_string(), "https://example.com".to_string()),
            ("user_email".to_string(), "${fake:email}".to_string()),
            ("greeting".to_string(), "Hello!".to_string()),
            (
                "addr".to_string(),
                "${fake:address(country=GB)}".to_string(),
            ),
        ];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        assert_eq!(result.len(), 4);

        // Plain vars
        assert_eq!(
            result.get("base_url"),
            Some(&VarValue::string("https://example.com"))
        );
        assert_eq!(result.get("greeting"), Some(&VarValue::string("Hello!")));

        // Generator vars
        let email = result.get("user_email").expect("should have email");
        assert!(email.as_str().is_some(), "email SHALL be a string");

        let addr = result.get("addr").expect("should have addr");
        assert!(addr.as_object().is_some(), "addr SHALL be an object");
        assert_eq!(field(addr, "country_code"), "GB");
    }

    // 9. Deterministic with same seed
    #[test]
    fn deterministic_with_same_seed() {
        let vars = vec![
            (
                "addr".to_string(),
                "${fake:address(country=JP)}".to_string(),
            ),
            ("email".to_string(), "${fake:email}".to_string()),
            ("name".to_string(), "${fake:city}".to_string()),
        ];

        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let result1 = evaluate_generators(&vars, &mut rng1).expect("should succeed");
        let result2 = evaluate_generators(&vars, &mut rng2).expect("should succeed");

        assert_eq!(result1, result2, "same seed SHALL produce same results");
    }

    // 10. Empty vars list returns empty map
    #[test]
    fn empty_vars_returns_empty_map() {
        let mut rng = seeded_rng();
        let vars: Vec<(String, String)> = vec![];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");
        assert!(result.is_empty(), "empty input SHALL produce empty output");
    }

    // 11. Unclosed reference returns error
    #[test]
    fn error_on_unclosed_reference() {
        let mut rng = seeded_rng();
        let vars = vec![(
            "email".to_string(),
            "${fake:email(domain=${missing}".to_string(),
        )];
        let result = evaluate_generators(&vars, &mut rng);

        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("unclosed variable reference"),
            "expected 'unclosed variable reference' error, got: {msg}"
        );
    }

    // 12. Plain var referencing another plain var via ${} works
    #[test]
    fn plain_var_references_another_plain_var() {
        let mut rng = seeded_rng();
        let vars = vec![
            ("host".to_string(), "example.com".to_string()),
            ("url".to_string(), "https://${host}/api".to_string()),
        ];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        assert_eq!(
            result.get("url"),
            Some(&VarValue::string("https://example.com/api"))
        );
    }

    // 15. Attempting to reference an object variable without dot-path yields error
    #[test]
    fn error_referencing_object_without_dot_path() {
        let mut rng = seeded_rng();
        let vars = vec![
            (
                "addr".to_string(),
                "${fake:address(country=JP)}".to_string(),
            ),
            ("bad".to_string(), "value=${addr}".to_string()),
        ];
        let result = evaluate_generators(&vars, &mut rng);

        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("not a structured object"),
            "expected object-in-string error, got: {msg}"
        );
    }

    // 20. Malformed generator def (unterminated parens) yields a parse error
    #[test]
    fn error_on_unparseable_generator_def() {
        let mut rng = seeded_rng();
        // Opening paren but no closing paren -> GeneratorDef::parse returns None.
        let vars = vec![("bad".to_string(), "${fake:email(domain=x}".to_string())];
        let result = evaluate_generators(&vars, &mut rng);

        assert!(result.is_err());
        let err = result.expect_err("should be error");
        assert!(
            err.to_string().contains("invalid generator"),
            "expected parse-failure error, got: {err}"
        );
    }
}
