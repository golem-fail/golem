use std::collections::HashMap;

use anyhow::{anyhow, Result};
use rand::Rng;

use crate::generators::generate_simple;
use crate::structured::generate_structured;
use crate::{GeneratorDef, VarValue};

/// Evaluate all variable declarations, processing generators and cross-references.
///
/// Variables are evaluated in the order provided (which should match declaration order).
/// Values starting with `fake:` are treated as generator definitions and are evaluated.
/// Generator params may reference already-evaluated variables using `${var}` or
/// `${var.field}` syntax. All other values are stored as plain `VarValue::String`.
pub fn evaluate_generators(
    vars: &[(String, String)],
    rng: &mut impl Rng,
) -> Result<HashMap<String, VarValue>> {
    let mut results: HashMap<String, VarValue> = HashMap::new();

    for (name, value) in vars {
        if value.starts_with("fake:") {
            // Resolve any ${var} references in the value string first
            let resolved_value = resolve_references(value, &results)?;
            // Parse the generator def from the resolved string
            let def = GeneratorDef::parse(&resolved_value)
                .ok_or_else(|| anyhow!("failed to parse generator definition: {resolved_value}"))?;
            // Try structured first, then simple
            let generated = match generate_structured(&def, rng) {
                Ok(val) => val,
                Err(_) => generate_simple(&def, rng)?,
            };
            results.insert(name.clone(), generated);
        } else {
            // Plain string value — also resolve references in case someone writes
            // a plain value that references another variable
            let resolved = resolve_references(value, &results)?;
            results.insert(name.clone(), VarValue::String(resolved));
        }
    }

    Ok(results)
}

/// Resolve `${var}` and `${var.field}` references in a string using already-evaluated variables.
///
/// Returns an error if:
/// - A referenced variable has not been evaluated yet (including forward references)
/// - A dot-path navigates through a non-object or into a missing field
/// - A `${...}` reference is unclosed
fn resolve_references(template: &str, vars: &HashMap<String, VarValue>) -> Result<String> {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            // Consume the '{'
            chars.next();

            // Read until '}'
            let mut ref_name = String::new();
            let mut found_close = false;
            for ch in chars.by_ref() {
                if ch == '}' {
                    found_close = true;
                    break;
                }
                ref_name.push(ch);
            }

            if !found_close {
                return Err(golem_events::coded(
                    golem_events::FailureCode::ParseVariable,
                    anyhow!("unclosed variable reference: ${{{ref_name}"),
                ));
            }

            // Resolve the reference: split on first '.' to get var name and optional path
            let resolved = resolve_var_path(&ref_name, vars)?;
            result.push_str(&resolved);
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

/// Resolve a variable reference like `"addr"` or `"addr.country_code"`.
///
/// For simple references (`addr`), returns the string representation of the variable.
/// For dot-path references (`addr.country_code`), navigates into the object.
fn resolve_var_path(ref_path: &str, vars: &HashMap<String, VarValue>) -> Result<String> {
    let (var_name, field_path) = match ref_path.split_once('.') {
        Some((name, path)) => (name, Some(path)),
        None => (ref_path, None),
    };

    let var_value = vars
        .get(var_name)
        .ok_or_else(|| golem_events::coded(
            golem_events::FailureCode::ParseVariable,
            anyhow!("undefined variable: {var_name}"),
        ))?;

    match field_path {
        Some(path) => {
            // Navigate the dot-path within the variable value
            let target = var_value
                .get_path(path)
                .ok_or_else(|| golem_events::coded(
                    golem_events::FailureCode::ParseVariable,
                    anyhow!("path \"{path}\" not found on variable \"{var_name}\""),
                ))?;
            match target {
                VarValue::String(s) => Ok(s.clone()),
                VarValue::Object(_) => {
                    return Err(golem_events::coded(
                        golem_events::FailureCode::ParseVariable,
                        anyhow!("path \"{path}\" on variable \"{var_name}\" resolved to an object, not a string"),
                    ))
                }
            }
        }
        None => match var_value {
            VarValue::String(s) => Ok(s.clone()),
            VarValue::Object(_) => {
                return Err(golem_events::coded(
                    golem_events::FailureCode::ParseVariable,
                    anyhow!("variable \"{var_name}\" is an object; use dot-path syntax like ${{{var_name}.field}}"),
                ))
            }
        },
    }
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

        assert_eq!(result.get("greeting"), Some(&VarValue::string("hello world")));
        assert_eq!(result.get("name"), Some(&VarValue::string("Alice")));
    }

    // 2. Generator vars are evaluated and stored
    #[test]
    fn generator_vars_are_evaluated() {
        let mut rng = seeded_rng();
        let vars = vec![("email".to_string(), "fake:email".to_string())];
        let result = evaluate_generators(&vars, &mut rng).expect("should succeed");

        let email_val = result.get("email").expect("should have email");
        let email_str = email_val.as_str().expect("should be string");
        assert!(email_str.contains('@'), "SHALL be an email, got: {email_str}");
    }

    // 3. Cross-reference: generator param references prior variable
    #[test]
    fn cross_reference_prior_string_variable() {
        let mut rng = seeded_rng();
        let vars = vec![
            ("domain".to_string(), "acme.com".to_string()),
            ("email".to_string(), "fake:email(domain=${domain})".to_string()),
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
            ("addr".to_string(), "fake:address(country=JP)".to_string()),
            (
                "phone".to_string(),
                "fake:phone(country=${addr.country_code})".to_string(),
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
            ("addr".to_string(), "fake:address(country=JP)".to_string()),
            (
                "phone".to_string(),
                "fake:phone(country=${addr.country_code})".to_string(),
            ),
            (
                "city2".to_string(),
                "fake:city(country=${addr.country_code})".to_string(),
            ),
            (
                "person".to_string(),
                "fake:person(country=${addr.country_code})".to_string(),
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
            "fake:email(domain=${missing_var})".to_string(),
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
                "fake:phone(country=${addr.country_code})".to_string(),
            ),
            ("addr".to_string(), "fake:address(country=JP)".to_string()),
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
            ("user_email".to_string(), "fake:email".to_string()),
            ("greeting".to_string(), "Hello!".to_string()),
            ("addr".to_string(), "fake:address(country=GB)".to_string()),
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
            ("addr".to_string(), "fake:address(country=JP)".to_string()),
            ("email".to_string(), "fake:email".to_string()),
            ("name".to_string(), "fake:first_name".to_string()),
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
            "fake:email(domain=${missing".to_string(),
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

    // 13. resolve_references with no references returns string unchanged
    #[test]
    fn resolve_references_no_refs_unchanged() {
        let vars: HashMap<String, VarValue> = HashMap::new();
        let result = resolve_references("plain string with no refs", &vars)
            .expect("should succeed");
        assert_eq!(result, "plain string with no refs");
    }

    // 14. resolve_references with multiple references in one string
    #[test]
    fn resolve_references_multiple_refs() {
        let mut vars: HashMap<String, VarValue> = HashMap::new();
        vars.insert("a".to_string(), VarValue::string("AAA"));
        vars.insert("b".to_string(), VarValue::string("BBB"));

        let result = resolve_references("${a} and ${b}", &vars).expect("should succeed");
        assert_eq!(result, "AAA and BBB");
    }

    // 15. Attempting to reference an object variable without dot-path yields error
    #[test]
    fn error_referencing_object_without_dot_path() {
        let mut rng = seeded_rng();
        let vars = vec![
            ("addr".to_string(), "fake:address(country=JP)".to_string()),
            ("bad".to_string(), "value=${addr}".to_string()),
        ];
        let result = evaluate_generators(&vars, &mut rng);

        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("is an object"),
            "expected 'is an object' error, got: {msg}"
        );
    }

    // 16. Dot-path that resolves to a nested object (not a string) yields error
    #[test]
    fn error_dot_path_resolves_to_object() {
        let mut vars: HashMap<String, VarValue> = HashMap::new();
        vars.insert(
            "addr".to_string(),
            VarValue::object(vec![(
                "geo",
                VarValue::object(vec![("lat", VarValue::string("35.6"))]),
            )]),
        );

        let result = resolve_var_path("addr.geo", &vars);
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("resolved to an object, not a string"),
            "expected object-not-string error, got: {msg}"
        );
    }

    // 17. Dot-path to a missing field yields a "path not found" error
    #[test]
    fn error_dot_path_field_not_found() {
        let mut vars: HashMap<String, VarValue> = HashMap::new();
        vars.insert(
            "addr".to_string(),
            VarValue::object(vec![("country_code", VarValue::string("JP"))]),
        );

        let result = resolve_var_path("addr.nonexistent", &vars);
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("not found") && msg.contains("addr"),
            "expected 'path not found' error mentioning the variable, got: {msg}"
        );
    }

    // 18. Dot-path on a string variable navigates into a non-object and is not found
    #[test]
    fn error_dot_path_through_string_variable() {
        let mut vars: HashMap<String, VarValue> = HashMap::new();
        vars.insert("name".to_string(), VarValue::string("Alice"));

        let result = resolve_var_path("name.first", &vars);
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("not found"),
            "expected 'path not found' error for path into a string, got: {msg}"
        );
    }

    // 19. Dot-path navigating multiple segments down to a leaf string succeeds
    #[test]
    fn dot_path_multi_segment_leaf_string() {
        let mut vars: HashMap<String, VarValue> = HashMap::new();
        vars.insert(
            "user".to_string(),
            VarValue::object(vec![(
                "addr",
                VarValue::object(vec![("city", VarValue::string("Tokyo"))]),
            )]),
        );

        let resolved = resolve_var_path("user.addr.city", &vars).expect("should succeed");
        assert_eq!(resolved, "Tokyo", "deep dot-path SHALL resolve to leaf string");
    }

    // 20. Malformed generator def (unterminated parens) yields a parse error
    #[test]
    fn error_on_unparseable_generator_def() {
        let mut rng = seeded_rng();
        // Opening paren but no closing paren -> GeneratorDef::parse returns None.
        let vars = vec![("bad".to_string(), "fake:email(domain=x".to_string())];
        let result = evaluate_generators(&vars, &mut rng);

        assert!(result.is_err());
        let err = result.expect_err("should be error");
        assert!(
            err.to_string().contains("failed to parse generator definition"),
            "expected parse-failure error, got: {err}"
        );
    }

    // 21. Empty reference ${} resolves the empty-named variable, which is undefined
    #[test]
    fn error_on_empty_reference() {
        let vars: HashMap<String, VarValue> = HashMap::new();
        let result = resolve_references("prefix-${}-suffix", &vars);
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("undefined variable"),
            "empty reference SHALL be treated as an undefined variable, got: {msg}"
        );
    }

    // 22. A '$' not followed by '{' is emitted literally
    #[test]
    fn lone_dollar_is_literal() {
        let vars: HashMap<String, VarValue> = HashMap::new();
        let result = resolve_references("cost is $5 and $ alone", &vars)
            .expect("should succeed");
        assert_eq!(result, "cost is $5 and $ alone");
    }

    // 23. Resolved reference value is inserted literally (no recursive re-resolution)
    #[test]
    fn resolved_value_is_not_re_resolved() {
        let mut vars: HashMap<String, VarValue> = HashMap::new();
        // 'a' itself contains a ${b} sequence; resolution does not recurse into it.
        vars.insert("a".to_string(), VarValue::string("${b}"));

        let result = resolve_references("${a}", &vars).expect("should succeed");
        assert_eq!(
            result, "${b}",
            "resolved value SHALL be inserted literally without re-resolution"
        );
    }
}
