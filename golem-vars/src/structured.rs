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
}
