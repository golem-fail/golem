mod address;
mod credit_card;
mod person;

use anyhow::Result;
use rand::Rng;

use crate::{GeneratorDef, VarValue};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Generate a structured (Object) value from a generator definition.
///
/// Supported generators: `person`, `address`, `credit_card`.
pub fn generate_structured(def: &GeneratorDef, rng: &mut impl Rng) -> Result<VarValue> {
    match def.name.as_str() {
        "person" => person::generate_person(&def.params, rng),
        "address" => address::generate_address(&def.params, rng),
        "credit_card" => credit_card::generate_credit_card(&def.params, rng),
        _ => Err(golem_events::coded(
            golem_events::FailureCode::ParseVariable,
            anyhow::anyhow!("Unknown structured generator: {}", def.name),
        )),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

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

    // 11. Unknown type returns error
    #[test]
    fn unknown_type_returns_error() {
        let mut rng = seeded_rng();
        let result = generate_structured(&def("nonexistent"), &mut rng);
        assert!(result.is_err());
        let err = result.expect_err("should be error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("Unknown structured generator"),
            "expected 'Unknown structured generator' error, got: {msg}"
        );
    }

    // 12. Unknown type error echoes the offending generator name.
    #[test]
    fn unknown_type_error_includes_name() {
        let mut rng = seeded_rng();
        let err = generate_structured(&def("wobble"), &mut rng)
            .expect_err("unknown generator SHALL error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("wobble"),
            "error SHALL name the unknown generator, got: {msg}"
        );
    }

    // 13. Unknown type error carries the ParseVariable failure code.
    #[test]
    fn unknown_type_error_carries_parse_variable_code() {
        let mut rng = seeded_rng();
        let err = generate_structured(&def("nonexistent"), &mut rng)
            .expect_err("unknown generator SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseVariable),
            "unknown generator error SHALL carry ParseVariable code"
        );
    }

    // 14. The empty generator name is treated as unknown (no implicit default).
    #[test]
    fn empty_name_is_unknown() {
        let mut rng = seeded_rng();
        let err = generate_structured(&def(""), &mut rng)
            .expect_err("empty generator name SHALL error");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("Unknown structured generator"),
            "empty name SHALL be rejected as unknown, got: {msg}"
        );
    }

    // 15. The `person` dispatch arm routes to the person generator and yields an object.
    #[test]
    fn person_dispatches_to_object() {
        let mut rng = seeded_rng();
        let value = generate_structured(&def("person"), &mut rng)
            .expect("person generator SHALL succeed");
        assert!(
            value.as_object().is_some(),
            "person SHALL produce an Object value"
        );
    }

    // 16. The `address` dispatch arm routes to the address generator and yields an object.
    #[test]
    fn address_dispatches_to_object() {
        let mut rng = seeded_rng();
        let value = generate_structured(&def("address"), &mut rng)
            .expect("address generator SHALL succeed");
        assert!(
            value.as_object().is_some(),
            "address SHALL produce an Object value"
        );
    }

    // 17. The `credit_card` dispatch arm routes to the credit_card generator and yields an object.
    #[test]
    fn credit_card_dispatches_to_object() {
        let mut rng = seeded_rng();
        let value = generate_structured(&def("credit_card"), &mut rng)
            .expect("credit_card generator SHALL succeed");
        assert!(
            value.as_object().is_some(),
            "credit_card SHALL produce an Object value"
        );
    }

    // 18. Dispatch is case-sensitive: a capitalized known name is unknown.
    #[test]
    fn dispatch_is_case_sensitive() {
        let mut rng = seeded_rng();
        let err = generate_structured(&def("Person"), &mut rng)
            .expect_err("capitalized name SHALL not match the lowercase arm");
        let msg = golem_events::clean_msg(&err);
        assert!(
            msg.contains("Unknown structured generator"),
            "dispatch SHALL be case-sensitive, got: {msg}"
        );
    }
}
