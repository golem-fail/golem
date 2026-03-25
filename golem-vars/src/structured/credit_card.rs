use std::collections::HashMap;

use anyhow::{bail, Result};
use rand::Rng;

use crate::VarValue;

// ---------------------------------------------------------------------------
// Credit card generator
// ---------------------------------------------------------------------------

/// Card brand metadata.
struct CardBrand {
    name: &'static str,
    prefix: &'static str,
    length: usize,
    cvv_len: usize,
}

const CARD_BRANDS: &[CardBrand] = &[
    CardBrand {
        name: "Visa",
        prefix: "4",
        length: 16,
        cvv_len: 3,
    },
    CardBrand {
        name: "Mastercard",
        prefix: "51",
        length: 16,
        cvv_len: 3,
    },
    CardBrand {
        name: "Amex",
        prefix: "34",
        length: 15,
        cvv_len: 4,
    },
    CardBrand {
        name: "Discover",
        prefix: "6011",
        length: 16,
        cvv_len: 3,
    },
];

// ---------------------------------------------------------------------------
// Provider test cards
// ---------------------------------------------------------------------------

struct TestCard {
    number: &'static str,
    brand: &'static str,
    cvv_len: usize,
    status: &'static str,
}

const STRIPE_CARDS: &[TestCard] = &[
    TestCard { number: "4242424242424242", brand: "Visa", cvv_len: 3, status: "approved" },
    TestCard { number: "5555555555554444", brand: "Mastercard", cvv_len: 3, status: "approved" },
    TestCard { number: "378282246310005", brand: "Amex", cvv_len: 4, status: "approved" },
    TestCard { number: "4000000000000002", brand: "Visa", cvv_len: 3, status: "declined:card_declined" },
    TestCard { number: "4000000000009995", brand: "Visa", cvv_len: 3, status: "declined:insufficient_funds" },
    TestCard { number: "4000000000000069", brand: "Visa", cvv_len: 3, status: "declined:expired_card" },
    TestCard { number: "4000000000000127", brand: "Visa", cvv_len: 3, status: "declined:incorrect_cvc" },
    TestCard { number: "4000000000003220", brand: "Visa", cvv_len: 3, status: "threeds:required" },
    TestCard { number: "4000000000003063", brand: "Visa", cvv_len: 3, status: "threeds:challenge" },
];

// ---------------------------------------------------------------------------
// Generator entry point
// ---------------------------------------------------------------------------

pub(crate) fn generate_credit_card(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    match params.get("provider").map(|s| s.as_str()) {
        Some(provider) => generate_provider_card(provider, params, rng),
        None => generate_random_card(params, rng),
    }
}

/// Generate a random Luhn-valid card (no provider).
fn generate_random_card(
    _params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let brand_idx = rng.gen_range(0..CARD_BRANDS.len());
    let brand = &CARD_BRANDS[brand_idx];

    let number = generate_luhn_number(brand.prefix, brand.length, rng);

    let month: u32 = rng.gen_range(1..=12);
    let year: u32 = rng.gen_range(27..=31);
    let expiry = format!("{month:02}/{year}");

    let cvv: String = (0..brand.cvv_len)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect();

    let map: Vec<(&str, VarValue)> = vec![
        ("number", VarValue::string(&number)),
        ("expiry", VarValue::string(&expiry)),
        ("cvv", VarValue::string(&cvv)),
        ("brand", VarValue::string(brand.name)),
        ("provider", VarValue::string("")),
        ("status", VarValue::string("")),
    ];

    Ok(VarValue::object(map))
}

/// Generate a card from a specific provider's test card set.
fn generate_provider_card(
    provider: &str,
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let cards = match provider {
        "stripe" => STRIPE_CARDS,
        _ => bail!("unknown credit card provider: {provider}"),
    };

    let status = params.get("status").map(|s| s.as_str());

    let matching: Vec<&TestCard> = match status {
        None | Some("approved") => {
            cards.iter().filter(|c| c.status == "approved").collect()
        }
        Some(s) if !s.contains(':') => {
            // Prefix match: "declined" matches any "declined:*"
            let prefix = format!("{s}:");
            cards
                .iter()
                .filter(|c| c.status.starts_with(&prefix))
                .collect()
        }
        Some(s) => {
            // Exact match: "declined:insufficient_funds"
            cards.iter().filter(|c| c.status == s).collect()
        }
    };

    if matching.is_empty() {
        let status_str = status.unwrap_or("(none)");
        bail!("no {provider} test card matches status: {status_str}");
    }

    let card = matching[rng.gen_range(0..matching.len())];

    let month: u32 = rng.gen_range(1..=12);
    let year: u32 = rng.gen_range(27..=31);
    let expiry = format!("{month:02}/{year}");

    let cvv: String = (0..card.cvv_len)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect();

    let map: Vec<(&str, VarValue)> = vec![
        ("number", VarValue::string(card.number)),
        ("expiry", VarValue::string(&expiry)),
        ("cvv", VarValue::string(&cvv)),
        ("brand", VarValue::string(card.brand)),
        ("provider", VarValue::string(provider)),
        ("status", VarValue::string(card.status)),
    ];

    Ok(VarValue::object(map))
}

/// Generate a Luhn-valid credit card number with the given prefix and total length.
pub(crate) fn generate_luhn_number(prefix: &str, length: usize, rng: &mut impl Rng) -> String {
    let mut digits: Vec<u8> = prefix
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as u8))
        .collect();

    while digits.len() < length - 1 {
        digits.push(rng.gen_range(0..10));
    }

    let check = luhn_check_digit(&digits);
    digits.push(check);

    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// Compute the Luhn check digit for a sequence of digits.
pub(crate) fn luhn_check_digit(digits: &[u8]) -> u8 {
    let mut sum: u32 = 0;

    for (i, &d) in digits.iter().rev().enumerate() {
        let pos_from_right = i + 1;
        if pos_from_right % 2 == 1 {
            let doubled = (d as u32) * 2;
            sum += if doubled > 9 { doubled - 9 } else { doubled };
        } else {
            sum += d as u32;
        }
    }

    ((10 - (sum % 10)) % 10) as u8
}

/// Validate a number string passes the Luhn check.
#[cfg(test)]
fn luhn_valid(number: &str) -> bool {
    let digits: Vec<u8> = number
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as u8))
        .collect();

    if digits.is_empty() {
        return false;
    }

    let mut sum: u32 = 0;
    for (i, &d) in digits.iter().rev().enumerate() {
        if i % 2 == 1 {
            let doubled = (d as u32) * 2;
            sum += if doubled > 9 { doubled - 9 } else { doubled };
        } else {
            sum += d as u32;
        }
    }

    sum % 10 == 0
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

    // 8. Credit card produces all expected fields
    #[test]
    fn credit_card_produces_all_fields() {
        let mut rng = seeded_rng();
        let result =
            generate_structured(&def("credit_card"), &mut rng).expect("should generate");
        let obj = result.as_object().expect("should be object");
        assert!(obj.contains_key("number"), "missing 'number'");
        assert!(obj.contains_key("expiry"), "missing 'expiry'");
        assert!(obj.contains_key("cvv"), "missing 'cvv'");
        assert!(obj.contains_key("brand"), "missing 'brand'");
        assert!(obj.contains_key("provider"), "missing 'provider'");
        assert!(obj.contains_key("status"), "missing 'status'");
    }

    // 9. Credit card CVV length matches brand (3 or 4)
    #[test]
    fn credit_card_cvv_length_matches_brand() {
        for seed in 0u64..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let brand = field(&result, "brand");
            let cvv = field(&result, "cvv");

            let expected_cvv_len = if brand == "Amex" { 4 } else { 3 };
            assert_eq!(
                cvv.len(),
                expected_cvv_len,
                "seed={seed}: brand={brand} should have CVV length {expected_cvv_len}, got {}",
                cvv.len()
            );
        }
    }

    // 10. Deterministic seed produces same output (credit card portion)
    #[test]
    fn deterministic_seed_same_credit_card() {
        let mut rng1 = seeded_rng();
        let mut rng2 = seeded_rng();

        let cc1 =
            generate_structured(&def("credit_card"), &mut rng1).expect("should generate");
        let cc2 =
            generate_structured(&def("credit_card"), &mut rng2).expect("should generate");
        assert_eq!(
            cc1, cc2,
            "same seed should produce same credit card"
        );
    }

    // 12. Credit card number is Luhn-valid
    #[test]
    fn credit_card_number_is_luhn_valid() {
        for seed in 0u64..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let number = field(&result, "number");
            assert!(
                luhn_valid(&number),
                "seed={seed}: card number {number} should pass Luhn check"
            );
        }
    }

    // 13. Credit card number has correct length for its brand
    #[test]
    fn credit_card_number_correct_length() {
        for seed in 0u64..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let brand = field(&result, "brand");
            let number = field(&result, "number");

            let expected_len = if brand == "Amex" { 15 } else { 16 };
            assert_eq!(
                number.len(),
                expected_len,
                "seed={seed}: brand={brand} number should have length {expected_len}, got {}",
                number.len()
            );
        }
    }

    // 17. Credit card expiry format is MM/YY
    #[test]
    fn credit_card_expiry_format() {
        let mut rng = seeded_rng();
        let result =
            generate_structured(&def("credit_card"), &mut rng).expect("should generate");
        let expiry = field(&result, "expiry");
        assert_eq!(expiry.len(), 5, "expiry SHALL be 5 chars (MM/YY)");
        assert_eq!(
            expiry.chars().nth(2),
            Some('/'),
            "expiry should have / at position 2"
        );
        let month: u32 = expiry[..2]
            .parse()
            .expect("month part should parse as u32");
        assert!((1..=12).contains(&month), "month SHALL be 1-12, got {month}");
    }

    // 24. No-provider card has empty provider and status fields
    #[test]
    fn credit_card_no_provider_has_empty_fields() {
        let mut rng = seeded_rng();
        let result =
            generate_structured(&def("credit_card"), &mut rng).expect("SHALL generate");
        let provider = field(&result, "provider");
        let status = field(&result, "status");
        assert!(provider.is_empty(), "SHALL have empty provider when none specified");
        assert!(status.is_empty(), "SHALL have empty status when none specified");
    }

    // 25. Stripe approved card returns known test number
    #[test]
    fn credit_card_stripe_approved() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "approved")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe card");
        let number = field(&result, "number");
        let approved_numbers = ["4242424242424242", "5555555555554444", "378282246310005"];
        assert!(
            approved_numbers.contains(&number.as_str()),
            "SHALL return a known Stripe approved card, got: {number}"
        );
    }

    // 26. Stripe declined:insufficient_funds returns specific card
    #[test]
    fn credit_card_stripe_declined_insufficient_funds() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "stripe"), ("status", "declined:insufficient_funds")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe card");
        let number = field(&result, "number");
        assert_eq!(
            number, "4000000000009995",
            "SHALL return Stripe insufficient_funds card"
        );
    }

    // 27. Stripe threeds returns a 3DS card
    #[test]
    fn credit_card_stripe_threeds() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "threeds")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe 3DS card");
        let number = field(&result, "number");
        let threeds_numbers = ["4000000000003220", "4000000000003063"];
        assert!(
            threeds_numbers.contains(&number.as_str()),
            "SHALL return a Stripe 3DS card, got: {number}"
        );
    }

    // 28. Stripe default (no status) returns approved card
    #[test]
    fn credit_card_stripe_default_is_approved() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe card");
        let status = field(&result, "status");
        assert_eq!(status, "approved", "SHALL default to approved status");
    }

    // 29. Stripe card has provider and status fields
    #[test]
    fn credit_card_stripe_has_provider_field() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "approved")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe card");
        let provider = field(&result, "provider");
        let status = field(&result, "status");
        assert_eq!(provider, "stripe", "SHALL have provider=stripe");
        assert_eq!(status, "approved", "SHALL have status=approved");
    }

    // 30. Unknown provider returns error
    #[test]
    fn credit_card_unknown_provider_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "unknown")]);
        let result = generate_structured(&d, &mut rng);
        assert!(result.is_err(), "SHALL error for unknown provider");
    }

    // 31. Unknown status for known provider returns error
    #[test]
    fn credit_card_unknown_status_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "bogus")]);
        let result = generate_structured(&d, &mut rng);
        assert!(result.is_err(), "SHALL error for unknown status");
    }

    // 32. Declined prefix matches any declined:* card
    #[test]
    fn credit_card_stripe_declined_prefix_matches() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "declined")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate declined card");
        let status = field(&result, "status");
        assert!(
            status.starts_with("declined:"),
            "SHALL return a declined:* card, got: {status}"
        );
    }
}
