use std::collections::HashMap;

use anyhow::{bail, Result};
use rand::Rng;

use crate::card_loader::{card_database, find_cards, CardConfig, ProviderFile};
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
        name: "visa",
        prefix: "4",
        length: 16,
        cvv_len: 3,
    },
    CardBrand {
        name: "mastercard",
        prefix: "51",
        length: 16,
        cvv_len: 3,
    },
    CardBrand {
        name: "amex",
        prefix: "34",
        length: 15,
        cvv_len: 4,
    },
    CardBrand {
        name: "discover",
        prefix: "6011",
        length: 16,
        cvv_len: 3,
    },
];

/// Find card brand metadata by name (case-insensitive).
fn brand_by_name(name: &str) -> Option<&'static CardBrand> {
    let lower = name.to_lowercase();
    CARD_BRANDS.iter().find(|b| b.name == lower)
}

// ---------------------------------------------------------------------------
// Generic statuses (no provider)
// ---------------------------------------------------------------------------

/// Statuses that can be generated without a provider.
const GENERIC_STATUSES: &[&str] = &[
    "approved",
    "declined:invalid_number",
    "declined:expired",
    "declined:invalid_cvv",
    "threeds",
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

/// Generate a random Luhn-valid card (no provider), with optional generic status.
fn generate_random_card(
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let status = params.get("status").map(|s| s.as_str());
    let brand_param = params.get("brand").map(|s| s.as_str());

    // Validate status if given
    if let Some(s) = status {
        if s != "approved" && !GENERIC_STATUSES.contains(&s) {
            bail!("status \"{s}\" requires a provider");
        }
    }

    let brand = if let Some(b) = brand_param {
        brand_by_name(b).ok_or_else(|| anyhow::anyhow!("unknown brand: {b}"))?
    } else {
        let idx = rng.gen_range(0..CARD_BRANDS.len());
        &CARD_BRANDS[idx]
    };

    let status = status.unwrap_or("approved");

    let number = match status {
        "declined:invalid_number" => {
            // Generate Luhn-valid then flip check digit
            let valid = generate_luhn_number(brand.prefix, brand.length, rng);
            let mut chars: Vec<u8> = valid.bytes().collect();
            let last = chars.len() - 1;
            let orig = chars[last] - b'0';
            chars[last] = b'0' + ((orig + 1) % 10);
            String::from_utf8(chars).unwrap_or_default()
        }
        _ => generate_luhn_number(brand.prefix, brand.length, rng),
    };

    let expiry = match status {
        "declined:expired" => "01/20".to_string(),
        _ => random_future_expiry(rng),
    };

    let cvv = match status {
        "declined:invalid_cvv" => random_digits(2, rng),
        _ => random_digits(brand.cvv_len, rng),
    };

    let mut map: Vec<(&str, VarValue)> = vec![
        ("number", VarValue::string(&number)),
        ("expiry", VarValue::string(&expiry)),
        ("cvv", VarValue::string(&cvv)),
        ("brand", VarValue::string(brand.name)),
        ("provider", VarValue::string("")),
        ("status", VarValue::string(if status == "approved" { "" } else { status })),
    ];

    if status == "threeds" {
        map.push(("threeds", VarValue::string("true")));
    }

    Ok(VarValue::object(map))
}

/// Generate a card from a specific provider's test card set.
fn generate_provider_card(
    provider: &str,
    params: &HashMap<String, String>,
    rng: &mut impl Rng,
) -> Result<VarValue> {
    let db = card_database();
    let pf = db.get(provider).ok_or_else(|| {
        let valid = db.provider_ids().join(", ");
        anyhow::anyhow!("unknown credit card provider: {provider} (valid: {valid})")
    })?;

    let status = params.get("status").map(|s| s.as_str()).unwrap_or("approved");
    let brand = params.get("brand").map(|s| s.as_str());

    let matching = find_cards(pf, status, brand);
    if matching.is_empty() {
        bail!("no {provider} test card matches status: {status}");
    }

    let card = matching[rng.gen_range(0..matching.len())];

    let resolved = resolve_card(card, pf, rng);

    build_output(&resolved, provider, status)
}

// ---------------------------------------------------------------------------
// Card resolution: merge card config with provider defaults
// ---------------------------------------------------------------------------

struct ResolvedCard {
    number: String,
    brand: String,
    cvv: String,
    expiry: String,
    name: Option<String>,
    amount: Option<f64>,
    amount_currency: Option<String>,
    email: Option<String>,
    description: Option<String>,
    postal_code: Option<String>,
    pin: Option<String>,
    otp: Option<String>,
}

fn resolve_card(card: &CardConfig, pf: &ProviderFile, rng: &mut impl Rng) -> ResolvedCard {
    let brand_name = card.brand.clone().unwrap_or_else(|| "visa".to_string());
    let is_amex = brand_name.to_lowercase() == "amex";

    // Resolve number
    let number = if let Some(n) = &card.number {
        n.clone()
    } else {
        let default_num = pf.defaults.number.as_deref().unwrap_or("random_luhn");
        resolve_string(default_num, &brand_name, rng)
    };

    // Resolve CVV
    let cvv = if let Some(c) = &card.cvv {
        c.clone()
    } else if is_amex {
        let default_cvv = pf
            .defaults
            .cvv_amex
            .as_deref()
            .or(pf.defaults.cvv.as_deref())
            .unwrap_or("random:4");
        resolve_string(default_cvv, &brand_name, rng)
    } else {
        let default_cvv = pf.defaults.cvv.as_deref().unwrap_or("random:3");
        resolve_string(default_cvv, &brand_name, rng)
    };

    // Resolve expiry
    let expiry = if let Some(e) = &card.expiry {
        e.clone()
    } else {
        let default_exp = pf.defaults.expiry.as_deref().unwrap_or("random_future");
        resolve_string(default_exp, &brand_name, rng)
    };

    // Resolve name
    let name = if card.name.is_some() {
        card.name.clone()
    } else {
        match pf.defaults.name.as_deref() {
            Some("random") | None => None,
            Some(literal) => Some(literal.to_string()),
        }
    };

    ResolvedCard {
        number,
        brand: brand_name,
        cvv,
        expiry,
        name,
        amount: card.amount,
        amount_currency: card.amount_currency.clone(),
        email: card.email.clone(),
        description: card.description.clone(),
        postal_code: card.postal_code.clone(),
        pin: card.pin.clone(),
        otp: card.otp.clone(),
    }
}

/// Resolve a default string value, handling magic values.
fn resolve_string(value: &str, brand_name: &str, rng: &mut impl Rng) -> String {
    if value == "random_luhn" {
        let brand = brand_by_name(brand_name).unwrap_or(&CARD_BRANDS[0]);
        generate_luhn_number(brand.prefix, brand.length, rng)
    } else if value == "random_future" {
        random_future_expiry(rng)
    } else if let Some(n) = value.strip_prefix("random:") {
        let len: usize = n.parse().unwrap_or(3);
        random_digits(len, rng)
    } else if value == "random" {
        // For name fields — return a test name
        "Test User".to_string()
    } else {
        value.to_string()
    }
}

fn build_output(card: &ResolvedCard, provider: &str, status: &str) -> Result<VarValue> {
    let mut map: Vec<(&str, VarValue)> = vec![
        ("number", VarValue::string(&card.number)),
        ("expiry", VarValue::string(&card.expiry)),
        ("cvv", VarValue::string(&card.cvv)),
        ("brand", VarValue::string(&card.brand)),
        ("provider", VarValue::string(provider)),
        ("status", VarValue::string(status)),
    ];

    if let Some(ref name) = card.name {
        map.push(("name", VarValue::string(name)));
    }
    if let Some(amount) = card.amount {
        map.push(("amount", VarValue::string(format!("{amount}"))));
    }
    if let Some(ref currency) = card.amount_currency {
        map.push(("amount_currency", VarValue::string(currency)));
    }
    if let Some(ref email) = card.email {
        map.push(("email", VarValue::string(email)));
    }
    if let Some(ref desc) = card.description {
        map.push(("description", VarValue::string(desc)));
    }
    if let Some(ref postal) = card.postal_code {
        map.push(("postal_code", VarValue::string(postal)));
    }
    if let Some(ref pin) = card.pin {
        map.push(("pin", VarValue::string(pin)));
    }
    if let Some(ref otp) = card.otp {
        map.push(("otp", VarValue::string(otp)));
    }
    if status.starts_with("threeds") {
        map.push(("threeds", VarValue::string("true")));
    }

    Ok(VarValue::object(map))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn random_future_expiry(rng: &mut impl Rng) -> String {
    let month: u32 = rng.gen_range(1..=12);
    let year: u32 = rng.gen_range(27..=31);
    format!("{month:02}/{year}")
}

fn random_digits(len: usize, rng: &mut impl Rng) -> String {
    (0..len)
        .map(|_| char::from(b'0' + rng.gen_range(0..10u8)))
        .collect()
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

    fn has_field(val: &VarValue, key: &str) -> bool {
        val.as_object().expect("should be object").contains_key(key)
    }

    // --- No-provider tests (random card) ---

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

    #[test]
    fn credit_card_cvv_length_matches_brand() {
        for seed in 0u64..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let brand = field(&result, "brand");
            let cvv = field(&result, "cvv");

            let expected_cvv_len = if brand == "amex" { 4 } else { 3 };
            assert_eq!(
                cvv.len(),
                expected_cvv_len,
                "seed={seed}: brand={brand} should have CVV length {expected_cvv_len}, got {}",
                cvv.len()
            );
        }
    }

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

    #[test]
    fn credit_card_number_correct_length() {
        for seed in 0u64..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let result =
                generate_structured(&def("credit_card"), &mut rng).expect("should generate");
            let brand = field(&result, "brand");
            let number = field(&result, "number");

            let expected_len = if brand == "amex" { 15 } else { 16 };
            assert_eq!(
                number.len(),
                expected_len,
                "seed={seed}: brand={brand} number should have length {expected_len}, got {}",
                number.len()
            );
        }
    }

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

    // --- Provider tests (Stripe) ---

    #[test]
    fn credit_card_stripe_approved() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "approved")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe card");
        let number = field(&result, "number");
        let approved_numbers = [
            "4242424242424242",
            "5555555555554444",
            "378282246310005",
            "6011111111111117",
        ];
        assert!(
            approved_numbers.contains(&number.as_str()),
            "SHALL return a known Stripe approved card, got: {number}"
        );
    }

    #[test]
    fn credit_card_stripe_declined_insufficient_funds() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "stripe"), ("status", "declined:insufficient_funds")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe card");
        let number = field(&result, "number");
        let valid = ["4000000000009995", "5000000000000019"];
        assert!(
            valid.contains(&number.as_str()),
            "SHALL return Stripe insufficient_funds card, got: {number}"
        );
    }

    #[test]
    fn credit_card_stripe_threeds() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "threeds")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe 3DS card");
        let number = field(&result, "number");
        let threeds_numbers = ["4000000000003220", "5200000000000007"];
        assert!(
            threeds_numbers.contains(&number.as_str()),
            "SHALL return a Stripe 3DS card, got: {number}"
        );
        assert_eq!(field(&result, "threeds"), "true", "SHALL have threeds flag");
    }

    #[test]
    fn credit_card_stripe_default_is_approved() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate stripe card");
        let status = field(&result, "status");
        assert_eq!(status, "approved", "SHALL default to approved status");
    }

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

    #[test]
    fn credit_card_unknown_provider_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "unknown")]);
        let result = generate_structured(&d, &mut rng);
        assert!(result.is_err(), "SHALL error for unknown provider");
    }

    #[test]
    fn credit_card_unknown_status_errors() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "bogus")]);
        let result = generate_structured(&d, &mut rng);
        assert!(result.is_err(), "SHALL error for unknown status");
    }

    #[test]
    fn credit_card_stripe_declined_prefix_matches() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "stripe"), ("status", "declined")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate declined card");
        let status = field(&result, "status");
        // When using bare "declined", the status echoed back is "declined"
        assert_eq!(status, "declined", "SHALL echo back the requested status");
    }

    // --- Brand filter tests ---

    #[test]
    fn credit_card_stripe_brand_filter() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "stripe"), ("status", "approved"), ("brand", "amex")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate amex card");
        let number = field(&result, "number");
        assert_eq!(number, "378282246310005", "SHALL return Amex approved card");
        let brand = field(&result, "brand");
        assert_eq!(brand, "amex", "SHALL have amex brand");
    }

    #[test]
    fn credit_card_random_brand_filter() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("brand", "amex")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate amex card");
        let brand = field(&result, "brand");
        assert_eq!(brand, "amex", "SHALL respect brand filter");
        let number = field(&result, "number");
        assert_eq!(number.len(), 15, "Amex SHALL be 15 digits");
    }

    // --- Generic status tests ---

    #[test]
    fn credit_card_generic_declined_invalid_number() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("status", "declined:invalid_number")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate");
        let number = field(&result, "number");
        assert!(!luhn_valid(&number), "SHALL fail Luhn check");
        let status = field(&result, "status");
        assert_eq!(status, "declined:invalid_number");
    }

    #[test]
    fn credit_card_generic_declined_expired() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("status", "declined:expired")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate");
        let expiry = field(&result, "expiry");
        assert_eq!(expiry, "01/20", "SHALL have past expiry");
        let status = field(&result, "status");
        assert_eq!(status, "declined:expired");
    }

    #[test]
    fn credit_card_generic_declined_invalid_cvv() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("status", "declined:invalid_cvv")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate");
        let cvv = field(&result, "cvv");
        assert_eq!(cvv.len(), 2, "SHALL have 2-digit CVV");
        let status = field(&result, "status");
        assert_eq!(status, "declined:invalid_cvv");
    }

    #[test]
    fn credit_card_generic_threeds() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("status", "threeds")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate");
        assert!(has_field(&result, "threeds"), "SHALL have threeds field");
        assert_eq!(field(&result, "threeds"), "true");
        assert!(luhn_valid(&field(&result, "number")), "SHALL be Luhn-valid");
    }

    #[test]
    fn credit_card_generic_status_requires_provider() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("status", "declined:insufficient_funds")]);
        let result = generate_structured(&d, &mut rng);
        assert!(result.is_err(), "SHALL error: status requires provider");
    }

    // --- Provider mechanism tests ---

    #[test]
    fn credit_card_praxis_cvv_controlled() {
        let mut rng = seeded_rng();
        let d = def_with_params("credit_card", &[("provider", "praxis"), ("status", "approved")]);
        let result = generate_structured(&d, &mut rng).expect("SHALL generate praxis card");
        let cvv = field(&result, "cvv");
        assert!(
            cvv == "568" || cvv == "5681",
            "Praxis approved SHALL have CVV 568 or 5681, got: {cvv}"
        );
        let number = field(&result, "number");
        assert!(luhn_valid(&number), "Praxis card number SHALL be Luhn-valid");
    }

    #[test]
    fn credit_card_mollie_amount_controlled() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "mollie"), ("status", "declined:insufficient_funds")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate mollie card");
        assert!(has_field(&result, "amount"), "SHALL have amount field");
        assert_eq!(field(&result, "amount"), "1007");
        assert_eq!(field(&result, "amount_currency"), "EUR");
    }

    #[test]
    fn credit_card_mercado_pago_name_controlled() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "mercado_pago"), ("status", "approved")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate mercado_pago card");
        assert!(has_field(&result, "name"), "SHALL have name field");
        let name = field(&result, "name");
        assert_eq!(name, "APRO", "SHALL have APRO name trigger");
    }

    #[test]
    fn credit_card_klarna_email_controlled() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "klarna"), ("status", "declined")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate klarna card");
        assert!(has_field(&result, "email"), "SHALL have email field");
        assert_eq!(
            field(&result, "email"),
            "customer+cc+denied@klarna.com"
        );
    }

    #[test]
    fn credit_card_paypal_name_trigger() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "paypal"), ("status", "declined:insufficient_funds")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate paypal card");
        assert!(has_field(&result, "name"), "SHALL have name field");
        assert_eq!(field(&result, "name"), "CCREJECT-IF");
    }

    #[test]
    fn credit_card_authorize_net_postal_trigger() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "authorize_net"), ("status", "declined")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate authorize_net card");
        assert!(has_field(&result, "postal_code"), "SHALL have postal_code field");
        assert_eq!(field(&result, "postal_code"), "46282");
    }

    #[test]
    fn credit_card_adyen_fixed_cvv() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "adyen"), ("status", "approved"), ("brand", "visa")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate adyen card");
        let cvv = field(&result, "cvv");
        assert_eq!(cvv, "737", "Adyen Visa SHALL have CVV 737");
    }

    #[test]
    fn credit_card_adyen_amex_cvv() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "adyen"), ("status", "approved"), ("brand", "amex")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate adyen amex card");
        let cvv = field(&result, "cvv");
        assert_eq!(cvv, "7373", "Adyen Amex SHALL have CVV 7373");
    }

    #[test]
    fn credit_card_braintree_amount() {
        let mut rng = seeded_rng();
        let d = def_with_params(
            "credit_card",
            &[("provider", "braintree"), ("status", "declined")],
        );
        let result = generate_structured(&d, &mut rng).expect("SHALL generate braintree card");
        assert!(has_field(&result, "amount"), "SHALL have amount field");
        assert_eq!(field(&result, "amount"), "2000");
    }
}
