//! Card data loader — embeds and indexes all `data/cards/*.json` files at
//! compile time, exposing them through a single `CardDatabase` singleton.
//!
//! New provider files only require adding one `include_str!` line to the
//! `RAW_ENTRIES` array.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Compile-time data loading
// ---------------------------------------------------------------------------

static RAW_ENTRIES: &[(&str, &str)] = &[
    // Global / North America
    ("stripe", include_str!("../../data/cards/stripe.json")),
    ("adyen", include_str!("../../data/cards/adyen.json")),
    ("braintree", include_str!("../../data/cards/braintree.json")),
    ("checkout_com", include_str!("../../data/cards/checkout_com.json")),
    ("worldpay", include_str!("../../data/cards/worldpay.json")),
    ("authorize_net", include_str!("../../data/cards/authorize_net.json")),
    ("cybersource", include_str!("../../data/cards/cybersource.json")),
    ("square", include_str!("../../data/cards/square.json")),
    ("nmi", include_str!("../../data/cards/nmi.json")),
    ("spreedly", include_str!("../../data/cards/spreedly.json")),
    ("paypal", include_str!("../../data/cards/paypal.json")),
    // Europe
    ("mollie", include_str!("../../data/cards/mollie.json")),
    ("klarna", include_str!("../../data/cards/klarna.json")),
    ("praxis", include_str!("../../data/cards/praxis.json")),
    ("worldline", include_str!("../../data/cards/worldline.json")),
    ("opayo", include_str!("../../data/cards/opayo.json")),
    ("unzer", include_str!("../../data/cards/unzer.json")),
    ("nets_nexi", include_str!("../../data/cards/nets_nexi.json")),
    ("multisafepay", include_str!("../../data/cards/multisafepay.json")),
    ("buckaroo", include_str!("../../data/cards/buckaroo.json")),
    ("paysafe", include_str!("../../data/cards/paysafe.json")),
    ("payoneer", include_str!("../../data/cards/payoneer.json")),
    ("nuvei", include_str!("../../data/cards/nuvei.json")),
    ("two_checkout", include_str!("../../data/cards/two_checkout.json")),
    ("bambora", include_str!("../../data/cards/bambora.json")),
    // Africa
    ("flutterwave", include_str!("../../data/cards/flutterwave.json")),
    ("paystack", include_str!("../../data/cards/paystack.json")),
    ("interswitch", include_str!("../../data/cards/interswitch.json")),
    ("peach_payments", include_str!("../../data/cards/peach_payments.json")),
    // Asia-Pacific
    ("payu", include_str!("../../data/cards/payu.json")),
    ("komoju", include_str!("../../data/cards/komoju.json")),
    ("pay_jp", include_str!("../../data/cards/pay_jp.json")),
    ("univapay", include_str!("../../data/cards/univapay.json")),
    ("omise", include_str!("../../data/cards/omise.json")),
    ("xendit", include_str!("../../data/cards/xendit.json")),
    ("two_c2p", include_str!("../../data/cards/two_c2p.json")),
    ("pin_payments", include_str!("../../data/cards/pin_payments.json")),
    ("windcave", include_str!("../../data/cards/windcave.json")),
    // Latin America
    ("mercado_pago", include_str!("../../data/cards/mercado_pago.json")),
    ("ebanx", include_str!("../../data/cards/ebanx.json")),
    ("pagseguro", include_str!("../../data/cards/pagseguro.json")),
    ("conekta", include_str!("../../data/cards/conekta.json")),
    ("dlocal", include_str!("../../data/cards/dlocal.json")),
    // Middle East
    ("tap_payments", include_str!("../../data/cards/tap_payments.json")),
    ("amazon_ps", include_str!("../../data/cards/amazon_ps.json")),
    ("myfatoorah", include_str!("../../data/cards/myfatoorah.json")),
    ("telr", include_str!("../../data/cards/telr.json")),
    ("paytabs", include_str!("../../data/cards/paytabs.json")),
    // Multi-region
    ("global_payments", include_str!("../../data/cards/global_payments.json")),
    ("rapyd", include_str!("../../data/cards/rapyd.json")),
    ("bluesnap", include_str!("../../data/cards/bluesnap.json")),
];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct ProviderFile {
    #[allow(dead_code)]
    pub(crate) id: String,
    #[allow(dead_code)]
    pub(crate) name: String,
    pub(crate) defaults: CardDefaults,
    pub(crate) statuses: HashMap<String, Vec<CardConfig>>,
}

#[derive(Deserialize, Default)]
pub(crate) struct CardDefaults {
    #[serde(default)]
    pub(crate) cvv: Option<String>,
    #[serde(default)]
    pub(crate) cvv_amex: Option<String>,
    #[serde(default)]
    pub(crate) expiry: Option<String>,
    #[serde(default)]
    pub(crate) number: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct CardConfig {
    #[serde(default)]
    pub(crate) number: Option<String>,
    #[serde(default)]
    pub(crate) brand: Option<String>,
    #[serde(default)]
    pub(crate) cvv: Option<String>,
    #[serde(default)]
    pub(crate) expiry: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) amount: Option<f64>,
    #[serde(default)]
    pub(crate) amount_currency: Option<String>,
    #[serde(default)]
    pub(crate) email: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) postal_code: Option<String>,
    #[serde(default)]
    pub(crate) pin: Option<String>,
    #[serde(default)]
    pub(crate) otp: Option<String>,
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

pub(crate) struct CardDatabase {
    providers: HashMap<String, ProviderFile>,
}

impl CardDatabase {
    fn new() -> Self {
        let mut providers = HashMap::new();
        for (id, json) in RAW_ENTRIES {
            let provider: ProviderFile =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("bad card JSON {id}: {e}"));
            providers.insert(id.to_string(), provider);
        }
        Self { providers }
    }

    pub(crate) fn get(&self, provider_id: &str) -> Option<&ProviderFile> {
        self.providers.get(provider_id)
    }

    pub(crate) fn provider_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.providers.keys().map(|s| s.as_str()).collect();
        ids.sort();
        ids
    }
}

pub(crate) fn card_database() -> &'static CardDatabase {
    static INSTANCE: OnceLock<CardDatabase> = OnceLock::new();
    INSTANCE.get_or_init(CardDatabase::new)
}

// ---------------------------------------------------------------------------
// Status lookup
// ---------------------------------------------------------------------------

/// Find cards matching the given status, with parent fallback.
///
/// Lookup order:
/// 1. Exact match (e.g. `declined:insufficient_funds`)
/// 2. Parent fallback (e.g. `declined`)
/// 3. Prefix match for bare status (e.g. `declined` matches all `declined:*`)
pub(crate) fn find_cards<'a>(
    provider: &'a ProviderFile,
    status: &str,
    brand: Option<&str>,
) -> Vec<&'a CardConfig> {
    let candidates: Vec<&CardConfig> = if let Some(cards) = provider.statuses.get(status) {
        // Exact match
        cards.iter().collect()
    } else if let Some(colon_pos) = status.find(':') {
        // Try parent: "declined:insufficient_funds" -> "declined"
        let parent = &status[..colon_pos];
        provider
            .statuses
            .get(parent)
            .map(|cards| cards.iter().collect())
            .unwrap_or_default()
    } else {
        // Bare status like "declined" — prefix-match all "declined:*" keys
        provider
            .statuses
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{status}:")))
            .flat_map(|(_, cards)| cards.iter())
            .collect()
    };

    if let Some(b) = brand {
        let b_lower = b.to_lowercase();
        candidates
            .into_iter()
            .filter(|c| {
                c.brand
                    .as_ref()
                    .map(|cb| cb.to_lowercase() == b_lower)
                    .unwrap_or(false)
            })
            .collect()
    } else {
        candidates
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_providers() {
        let db = card_database();
        let ids = db.provider_ids();
        assert!(ids.contains(&"stripe"), "SHALL load stripe");
        assert!(ids.contains(&"adyen"), "SHALL load adyen");
        assert!(ids.contains(&"praxis"), "SHALL load praxis");
        assert!(ids.contains(&"mollie"), "SHALL load mollie");
        assert!(ids.contains(&"klarna"), "SHALL load klarna");
        assert!(ids.contains(&"mercado_pago"), "SHALL load mercado_pago");
        assert!(ids.contains(&"worldpay"), "SHALL load worldpay");
        assert!(ids.contains(&"flutterwave"), "SHALL load flutterwave");
        assert!(ids.contains(&"xendit"), "SHALL load xendit");
        assert!(ids.contains(&"tap_payments"), "SHALL load tap_payments");
        assert!(ids.contains(&"windcave"), "SHALL load windcave");
        assert_eq!(ids.len(), 51, "SHALL have 51 providers");
    }

    #[test]
    fn stripe_has_approved_cards() {
        let db = card_database();
        let stripe = db.get("stripe").expect("SHALL find stripe");
        let cards = find_cards(stripe, "approved", None);
        assert!(!cards.is_empty(), "SHALL have approved cards");
        let numbers: Vec<&str> = cards.iter().filter_map(|c| c.number.as_deref()).collect();
        assert!(
            numbers.contains(&"4242424242424242"),
            "SHALL contain Visa approved card"
        );
    }

    #[test]
    fn status_exact_match() {
        let db = card_database();
        let stripe = db.get("stripe").unwrap();
        let cards = find_cards(stripe, "declined:insufficient_funds", None);
        assert!(!cards.is_empty());
        let numbers: Vec<&str> = cards.iter().filter_map(|c| c.number.as_deref()).collect();
        assert!(numbers.contains(&"4000000000009995"));
    }

    #[test]
    fn status_parent_fallback() {
        let db = card_database();
        let stripe = db.get("stripe").unwrap();
        // "declined:some_nonexistent" should fall back to "declined"
        let cards = find_cards(stripe, "declined:some_nonexistent", None);
        assert!(!cards.is_empty(), "SHALL fall back to parent 'declined'");
    }

    #[test]
    fn status_prefix_match() {
        let db = card_database();
        let stripe = db.get("stripe").unwrap();
        // Bare "declined" should match all "declined:*" plus "declined"
        let cards = find_cards(stripe, "declined", None);
        assert!(cards.len() > 1, "SHALL match multiple declined statuses");
    }

    #[test]
    fn brand_filter() {
        let db = card_database();
        let stripe = db.get("stripe").unwrap();
        let cards = find_cards(stripe, "approved", Some("amex"));
        assert_eq!(cards.len(), 1, "SHALL filter to one amex card");
        assert_eq!(cards[0].number.as_deref(), Some("378282246310005"));
    }

    #[test]
    fn unknown_provider_returns_none() {
        let db = card_database();
        assert!(db.get("nonexistent").is_none());
    }

    #[test]
    fn praxis_cvv_controlled() {
        let db = card_database();
        let praxis = db.get("praxis").unwrap();
        let cards = find_cards(praxis, "approved", None);
        assert!(!cards.is_empty());
        // Praxis cards have CVV set, number is None (resolved from defaults)
        let card = &cards[0];
        assert!(card.number.is_none(), "Praxis card number SHALL be None (random_luhn default)");
        assert!(card.cvv.is_some(), "Praxis card SHALL have explicit CVV");
    }

    #[test]
    fn mollie_amount_controlled() {
        let db = card_database();
        let mollie = db.get("mollie").unwrap();
        let cards = find_cards(mollie, "declined:insufficient_funds", None);
        assert!(!cards.is_empty());
        let card = &cards[0];
        assert_eq!(card.amount, Some(1007.0));
        assert_eq!(card.amount_currency.as_deref(), Some("EUR"));
    }

    #[test]
    fn klarna_email_controlled() {
        let db = card_database();
        let klarna = db.get("klarna").unwrap();
        let cards = find_cards(klarna, "declined", None);
        assert!(!cards.is_empty());
        let card = &cards[0];
        assert_eq!(
            card.email.as_deref(),
            Some("customer+cc+denied@klarna.com")
        );
    }

    #[test]
    fn mercado_pago_name_controlled() {
        let db = card_database();
        let mp = db.get("mercado_pago").unwrap();
        let cards = find_cards(mp, "approved", None);
        assert!(!cards.is_empty());
        assert!(
            cards.iter().any(|c| c.name.as_deref() == Some("APRO")),
            "SHALL have APRO name trigger"
        );
    }

    // 1. Bare status with no exact key falls through to the prefix branch and
    //    collects every "status:*" child (stripe has no bare "dispute" key but
    //    has "dispute:fraudulent" and "dispute:product_not_received").
    #[test]
    fn status_pure_prefix_match_no_exact_key() {
        let db = card_database();
        let stripe = db.get("stripe").expect("SHALL find stripe");
        assert!(
            !stripe.statuses.contains_key("dispute"),
            "precondition: no bare 'dispute' key"
        );
        let cards = find_cards(stripe, "dispute", None);
        assert_eq!(
            cards.len(),
            2,
            "SHALL collect both dispute:* children via prefix match"
        );
    }

    // 2. Bare status that matches neither an exact key nor any "status:*" prefix
    //    SHALL produce an empty result.
    #[test]
    fn bare_status_no_match_is_empty() {
        let db = card_database();
        let stripe = db.get("stripe").expect("SHALL find stripe");
        let cards = find_cards(stripe, "totally_unknown_status", None);
        assert!(cards.is_empty(), "unknown bare status SHALL yield no cards");
    }

    // 3. A colon status whose parent does not exist SHALL fall back to an empty
    //    result (the unwrap_or_default arm), not panic.
    #[test]
    fn colon_status_missing_parent_is_empty() {
        let db = card_database();
        let stripe = db.get("stripe").expect("SHALL find stripe");
        assert!(
            !stripe.statuses.contains_key("ghost"),
            "precondition: no 'ghost' parent key"
        );
        let cards = find_cards(stripe, "ghost:variant", None);
        assert!(
            cards.is_empty(),
            "colon status with missing parent SHALL yield no cards"
        );
    }

    // 4. Brand filtering SHALL be case-insensitive: an uppercase query matches a
    //    lowercase stored brand.
    #[test]
    fn brand_filter_is_case_insensitive() {
        let db = card_database();
        let stripe = db.get("stripe").expect("SHALL find stripe");
        let cards = find_cards(stripe, "approved", Some("AMEX"));
        assert_eq!(cards.len(), 1, "uppercase AMEX SHALL match lowercase brand");
        assert_eq!(cards[0].number.as_deref(), Some("378282246310005"));
    }

    // 5. A brand filter that matches no card SHALL produce an empty result.
    #[test]
    fn brand_filter_no_match_is_empty() {
        let db = card_database();
        let stripe = db.get("stripe").expect("SHALL find stripe");
        let cards = find_cards(stripe, "approved", Some("jcb"));
        assert!(
            cards.is_empty(),
            "brand absent from approved set SHALL yield no cards"
        );
    }

    // 6. Cards without a brand SHALL be excluded by a brand filter (the
    //    unwrap_or(false) arm). Praxis approved holds one brandless card and one
    //    amex card: filtering by amex SHALL drop the brandless one.
    #[test]
    fn brand_filter_excludes_cards_without_brand() {
        let db = card_database();
        let praxis = db.get("praxis").expect("SHALL find praxis");
        let unfiltered = find_cards(praxis, "approved", None);
        assert!(
            unfiltered.iter().any(|c| c.brand.is_none()),
            "precondition: a praxis approved card carries no brand"
        );
        let filtered = find_cards(praxis, "approved", Some("amex"));
        assert!(
            filtered.iter().all(|c| c.brand.is_some()),
            "brandless cards SHALL be excluded by a brand filter"
        );
        assert!(
            filtered.len() < unfiltered.len(),
            "filtering by amex SHALL drop the brandless card"
        );
    }

    // 7. provider_ids() SHALL return its ids in ascending sorted order. Checked
    //    independently of the production sort: a strictly-monotonic windows()
    //    scan plus a known out-of-insertion-order pair ("adyen" before "stripe",
    //    even though "stripe" is inserted first in RAW_ENTRIES).
    #[test]
    fn provider_ids_are_sorted() {
        let db = card_database();
        let ids = db.provider_ids();
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "provider_ids SHALL be strictly ascending: {ids:?}"
        );
        let adyen = ids
            .iter()
            .position(|id| *id == "adyen")
            .expect("SHALL contain adyen");
        let stripe = ids
            .iter()
            .position(|id| *id == "stripe")
            .expect("SHALL contain stripe");
        assert!(
            adyen < stripe,
            "adyen SHALL sort before stripe despite later insertion order"
        );
    }

    // 8. An empty brand string filters out every card (no stored brand equals "").
    #[test]
    fn empty_brand_filter_excludes_all() {
        let db = card_database();
        let stripe = db.get("stripe").expect("SHALL find stripe");
        let cards = find_cards(stripe, "approved", Some(""));
        assert!(
            cards.is_empty(),
            "empty brand string SHALL match no stored brand"
        );
    }
}
