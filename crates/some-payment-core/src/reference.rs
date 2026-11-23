//! Cross-lifecycle [`ProviderReference`] correlating gateway identifiers.

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::PaymentError;
use crate::provider_id::ProviderId;

/// Maximum allowed byte length for any individual reference identifier.
pub const MAX_REFERENCE_ID_LEN: usize = 255;

/// Correlates all gateway-specific identifiers across create, checkout,
/// webhook, inquiry, capture, void, and refund flows.
///
/// For example, Paymob's Unified Intention returns `intent_id` and `order_id`,
/// while Capture/Void/Refund require the numeric `transaction_id` (`obj.id`)
/// delivered via callback or Transaction Inquiry.
///
/// # Examples
///
/// ```
/// use some_payment_core::{ProviderId, ProviderReference};
///
/// let reference = ProviderReference::builder(ProviderId::new("paymob"))
///     .order_id("ORD-EG-501")
///     .transaction_id("987654321")
///     .build()
///     .unwrap();
/// assert_eq!(reference.transaction_id(), Some("987654321"));
/// ```
///
/// Direct struct construction is disallowed; always use
/// [`ProviderReference::builder`]:
///
/// ```compile_fail
/// use some_payment_core::{ProviderId, ProviderReference};
///
/// let _ = ProviderReference {
///     provider_id: ProviderId::new("stripe"),
///     payment_id: Some("pi_1".into()),
///     transaction_id: None,
///     order_id: None,
///     intent_id: None,
///     checkout_session_id: None,
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct ProviderReference {
    provider_id: ProviderId,
    payment_id: Option<String>,
    transaction_id: Option<String>,
    order_id: Option<String>,
    intent_id: Option<String>,
    checkout_session_id: Option<String>,
}

impl ProviderReference {
    /// Starts a [`ProviderReferenceBuilder`] for the given [`ProviderId`].
    #[must_use]
    pub fn builder(provider_id: ProviderId) -> ProviderReferenceBuilder {
        ProviderReferenceBuilder {
            provider_id,
            payment_id: None,
            transaction_id: None,
            order_id: None,
            intent_id: None,
            checkout_session_id: None,
        }
    }

    /// Target provider slug.
    #[must_use]
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    /// Primary payment identifier (e.g., Moyasar UUID or Stripe `pi_...`).
    #[must_use]
    pub fn payment_id(&self) -> Option<&str> {
        self.payment_id.as_deref()
    }

    /// Gateway transaction identifier (e.g., Paymob numeric `obj.id`).
    #[must_use]
    pub fn transaction_id(&self) -> Option<&str> {
        self.transaction_id.as_deref()
    }

    /// Merchant or gateway order identifier (e.g., Paymob `special_reference`).
    #[must_use]
    pub fn order_id(&self) -> Option<&str> {
        self.order_id.as_deref()
    }

    /// Payment intent identifier (Stripe `pi_...`, Moyasar intent, Paymob intention ID).
    #[must_use]
    pub fn intent_id(&self) -> Option<&str> {
        self.intent_id.as_deref()
    }

    /// Hosted checkout session or invoice identifier (Stripe `cs_...`, Moyasar invoice ID).
    #[must_use]
    pub fn checkout_session_id(&self) -> Option<&str> {
        self.checkout_session_id.as_deref()
    }
}

/// Builder for [`ProviderReference`] enforcing non-empty bounded identifiers.
#[derive(Clone, Debug)]
pub struct ProviderReferenceBuilder {
    provider_id: ProviderId,
    payment_id: Option<String>,
    transaction_id: Option<String>,
    order_id: Option<String>,
    intent_id: Option<String>,
    checkout_session_id: Option<String>,
}

fn is_valid_ref_byte(b: u8) -> bool {
    b.is_ascii_graphic() && !matches!(b, b'/' | b'\\' | b'?' | b'#' | b'%')
}

fn is_valid_ref_id(s: &str) -> bool {
    !s.is_empty() && !s.contains("..") && s.bytes().all(is_valid_ref_byte)
}

fn normalize_ref_id(raw: Option<String>) -> Result<Option<String>, PaymentError> {
    let Some(value) = raw else {
        return Ok(None);
    };
    if value.len() > MAX_REFERENCE_ID_LEN {
        return Err(PaymentError::validation(
            "reference identifiers must not exceed 255 bytes",
        ));
    }
    let trimmed = value.trim();
    if !is_valid_ref_id(trimmed) {
        return Err(PaymentError::validation(
            "reference identifiers must be 1..=255 printable ASCII bytes without path or URL delimiters",
        ));
    }
    Ok(Some(trimmed.to_owned()))
}

impl ProviderReferenceBuilder {
    /// Sets the primary payment ID.
    #[must_use]
    pub fn payment_id(mut self, id: impl Into<String>) -> Self {
        self.payment_id = Some(id.into());
        self
    }

    /// Sets the gateway transaction ID.
    #[must_use]
    pub fn transaction_id(mut self, id: impl Into<String>) -> Self {
        self.transaction_id = Some(id.into());
        self
    }

    /// Sets the merchant/gateway order ID.
    #[must_use]
    pub fn order_id(mut self, id: impl Into<String>) -> Self {
        self.order_id = Some(id.into());
        self
    }

    /// Sets the payment intent ID.
    #[must_use]
    pub fn intent_id(mut self, id: impl Into<String>) -> Self {
        self.intent_id = Some(id.into());
        self
    }

    /// Sets the hosted checkout session or invoice ID.
    #[must_use]
    pub fn checkout_session_id(mut self, id: impl Into<String>) -> Self {
        self.checkout_session_id = Some(id.into());
        self
    }

    /// Validates that at least one identifier is populated and within length bounds,
    /// storing normalized trimmed identifiers.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if no identifier is provided,
    /// if any raw identifier exceeds [`MAX_REFERENCE_ID_LEN`] bytes before
    /// trimming, or if any trimmed identifier is empty, contains `..`, or
    /// contains non-graphic ASCII, whitespace, control, or URL/path delimiter
    /// bytes (`/`, `\`, `?`, `#`, `%`).
    pub fn build(self) -> Result<ProviderReference, PaymentError> {
        let payment_id = normalize_ref_id(self.payment_id)?;
        let transaction_id = normalize_ref_id(self.transaction_id)?;
        let order_id = normalize_ref_id(self.order_id)?;
        let intent_id = normalize_ref_id(self.intent_id)?;
        let checkout_session_id = normalize_ref_id(self.checkout_session_id)?;

        let any_present = [
            &payment_id,
            &transaction_id,
            &order_id,
            &intent_id,
            &checkout_session_id,
        ]
        .iter()
        .any(|field| field.is_some());

        if !any_present {
            return Err(PaymentError::validation(
                "ProviderReference requires at least one populated identifier",
            ));
        }

        Ok(ProviderReference {
            provider_id: self.provider_id,
            payment_id,
            transaction_id,
            order_id,
            intent_id,
            checkout_session_id,
        })
    }
}

#[derive(Deserialize)]
struct RawProviderReference {
    provider_id: ProviderId,
    #[serde(default)]
    payment_id: Option<String>,
    #[serde(default)]
    transaction_id: Option<String>,
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default)]
    intent_id: Option<String>,
    #[serde(default)]
    checkout_session_id: Option<String>,
}

impl<'de> Deserialize<'de> for ProviderReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawProviderReference::deserialize(deserializer)?;
        ProviderReferenceBuilder {
            provider_id: raw.provider_id,
            payment_id: raw.payment_id,
            transaction_id: raw.transaction_id,
            order_id: raw.order_id,
            intent_id: raw.intent_id,
            checkout_session_id: raw.checkout_session_id,
        }
        .build()
        .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn round_trips_trims_hashes_and_exposes_all_accessors() {
        let reference = ProviderReference::builder(ProviderId::new("paymob"))
            .payment_id("  pay_1  ")
            .transaction_id("  456789  ")
            .order_id("  ORD-123  ")
            .intent_id("  pi_999  ")
            .checkout_session_id("  cs_777  ")
            .build()
            .unwrap();
        assert_eq!(reference.provider_id().as_str(), "paymob");
        assert_eq!(reference.payment_id(), Some("pay_1"));
        assert_eq!(reference.transaction_id(), Some("456789"));
        assert_eq!(reference.order_id(), Some("ORD-123"));
        assert_eq!(reference.intent_id(), Some("pi_999"));
        assert_eq!(reference.checkout_session_id(), Some("cs_777"));

        let mut set = HashSet::new();
        assert!(set.insert(reference.clone()));
        assert!(!set.insert(reference.clone()));

        let json = serde_json::to_string(&reference).unwrap();
        let decoded: ProviderReference = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, reference);
    }

    #[test]
    fn accepts_exact_255_byte_boundary_and_rejects_oversized_or_padded() {
        let exact = "x".repeat(MAX_REFERENCE_ID_LEN);
        let built = ProviderReference::builder(ProviderId::new("stripe"))
            .payment_id(exact.clone())
            .build()
            .unwrap();
        assert_eq!(built.payment_id(), Some(exact.as_str()));

        assert!(
            ProviderReference::builder(ProviderId::new("stripe"))
                .build()
                .is_err()
        );
        assert!(
            ProviderReference::builder(ProviderId::new("stripe"))
                .payment_id("   ")
                .build()
                .is_err()
        );
        assert!(
            ProviderReference::builder(ProviderId::new("stripe"))
                .payment_id("x".repeat(256))
                .build()
                .is_err()
        );
        let padded_oversized = format!("{}ok", " ".repeat(254));
        assert_eq!(padded_oversized.len(), 256);
        assert!(
            ProviderReference::builder(ProviderId::new("stripe"))
                .payment_id(padded_oversized)
                .build()
                .is_err()
        );
    }

    #[test]
    fn rejects_traversal_delimiters_and_control_chars() {
        let bad_ids = [
            "../admin",
            "pay..123",
            "pay/123",
            "pay\\123",
            "pay?foo=1",
            "pay#frag",
            "pay%2f123",
            "pay\r123",
            "pay\n123",
            "pay\0123",
            "pay 123",
            "pay\u{202E}123",
        ];
        for bad in bad_ids {
            assert!(
                ProviderReference::builder(ProviderId::new("stripe"))
                    .payment_id(bad)
                    .build()
                    .is_err(),
                "expected error for {bad:?}"
            );
        }
    }

    #[test]
    fn serde_rejects_invalid_payloads() {
        let empty: Result<ProviderReference, _> =
            serde_json::from_str(r#"{"provider_id":"stripe"}"#);
        assert!(empty.is_err());

        let whitespace: Result<ProviderReference, _> =
            serde_json::from_str(r#"{"provider_id":"stripe","payment_id":"   "}"#);
        assert!(whitespace.is_err());

        let traversal: Result<ProviderReference, _> =
            serde_json::from_str(r#"{"provider_id":"stripe","payment_id":"../evil"}"#);
        assert!(traversal.is_err());

        let oversized_json = format!(
            r#"{{"provider_id":"stripe","order_id":"{}"}}"#,
            "a".repeat(256)
        );
        let oversized: Result<ProviderReference, _> = serde_json::from_str(&oversized_json);
        assert!(oversized.is_err());
    }
}
