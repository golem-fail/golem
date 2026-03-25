use std::collections::HashMap;

use anyhow::Result;
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

pub(crate) fn generate_credit_card(
    _params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let brand_idx = rng.gen_range(0..CARD_BRANDS.len());
    let brand = &CARD_BRANDS[brand_idx];

    let number = generate_luhn_number(brand.prefix, brand.length, rng);

    // Generate expiry: random month, 1-5 years in the future from a reference year (2026).
    let month: u32 = rng.gen_range(1..=12);
    let year: u32 = rng.gen_range(27..=31);
    let expiry = format!("{month:02}/{year}");

    // Generate CVV.
    let cvv: String = (0..brand.cvv_len)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect();

    let map: Vec<(&str, VarValue)> = vec![
        ("number", VarValue::string(&number)),
        ("expiry", VarValue::string(&expiry)),
        ("cvv", VarValue::string(&cvv)),
        ("brand", VarValue::string(brand.name)),
    ];

    Ok(VarValue::object(map))
}

/// Generate a Luhn-valid credit card number with the given prefix and total length.
pub(crate) fn generate_luhn_number(prefix: &str, length: usize, rng: &mut impl Rng) -> String {
    // Start with the prefix digits.
    let mut digits: Vec<u8> = prefix
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as u8))
        .collect();

    // Fill random digits up to length - 1 (last digit is the check digit).
    while digits.len() < length - 1 {
        digits.push(rng.gen_range(0..10));
    }

    // Compute the Luhn check digit.
    let check = luhn_check_digit(&digits);
    digits.push(check);

    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// Compute the Luhn check digit for a sequence of digits (the check digit will
/// be appended at the end to make the full number valid).
pub(crate) fn luhn_check_digit(digits: &[u8]) -> u8 {
    let mut sum: u32 = 0;

    for (i, &d) in digits.iter().rev().enumerate() {
        // Position from the right: the check digit is position 0 (even),
        // so the last provided digit is position 1 (odd), etc.
        let pos_from_right = i + 1;
        if pos_from_right % 2 == 1 {
            // Odd position from right (with check digit at 0): double it
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
    }

    // 9. Credit card CVV length matches brand (3 or 4)
    #[test]
    fn credit_card_cvv_length_matches_brand() {
        // Generate many cards to hit different brands.
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
        assert_eq!(expiry.len(), 5, "expiry should be 5 chars (MM/YY)");
        assert_eq!(
            expiry.chars().nth(2),
            Some('/'),
            "expiry should have / at position 2"
        );
        let month: u32 = expiry[..2]
            .parse()
            .expect("month part should parse as u32");
        assert!((1..=12).contains(&month), "month should be 1-12, got {month}");
    }
}
