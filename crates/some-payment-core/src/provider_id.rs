//! Open, normalized lowercase provider identifier (`ProviderId`).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::PaymentError;

/// Maximum allowed length for a [`ProviderId`] slug.
pub const MAX_PROVIDER_ID_LEN: usize = 64;

/// Unique, lowercase ASCII provider slug (e.g., `"stripe"`, `"moyasar"`,
/// `"paymob"`, `"tap"`).
///
/// Using an open string newtype rather than a closed enum allows third-party
/// and future providers to plug into `some-payment` with zero core edits.
///
/// # Examples
///
/// ```
/// use some_payment_core::ProviderId;
///
/// let id = ProviderId::try_new("  Tap-Payments  ").unwrap();
/// assert_eq!(id.as_str(), "tap-payments");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    /// Creates a [`ProviderId`] from a trusted static/known slug, normalizing
    /// whitespace and ASCII case.
    ///
    /// Prefer [`ProviderId::try_new`] for untrusted external inputs.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not a valid provider slug (must be 1..=64 ASCII
    /// characters starting with `[a-z0-9]` and containing only `[a-z0-9_-]`).
    pub fn new(id: impl Into<String>) -> Self {
        Self::try_new(id.into()).expect("invalid static ProviderId slug")
    }

    /// Validates and constructs a [`ProviderId`] from untrusted input.
    ///
    /// Valid slugs are 1..=64 ASCII characters starting with `[a-z0-9]` and
    /// containing only `[a-z0-9_-]`.
    pub fn try_new(id: impl AsRef<str>) -> Result<Self, PaymentError> {
        let normalized = id.as_ref().trim().to_ascii_lowercase();
        validate_slug(&normalized)?;
        Ok(Self(normalized))
    }

    /// Returns the normalized slug string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_slug_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_')
}

fn validate_slug(slug: &str) -> Result<(), PaymentError> {
    if slug.is_empty() || slug.len() > MAX_PROVIDER_ID_LEN {
        return Err(PaymentError::validation(
            "provider_id must be between 1 and 64 ASCII characters",
        ));
    }
    let bytes = slug.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err(PaymentError::validation(
            "provider_id must start with an ASCII letter or digit",
        ));
    }
    if !bytes.iter().copied().all(is_valid_slug_byte) {
        return Err(PaymentError::validation(
            "provider_id may only contain lowercase ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ProviderId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ProviderId {
    type Error = PaymentError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::try_new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_valid_slugs_and_exact_boundary() {
        let id = ProviderId::try_new("  Stripe ").unwrap();
        assert_eq!(id.as_str(), "stripe");
        let custom = ProviderId::try_new("tap_payments-v2").unwrap();
        assert_eq!(custom.to_string(), "tap_payments-v2");
        let from_new = ProviderId::new("  Paymob ");
        assert_eq!(from_new.as_str(), "paymob");
        let from_try = ProviderId::try_from("moyasar").unwrap();
        assert_eq!(from_try.as_str(), "moyasar");
        assert!(ProviderId::try_from("../bad").is_err());

        let exact_max = "a".repeat(MAX_PROVIDER_ID_LEN);
        assert_eq!(ProviderId::try_new(&exact_max).unwrap().as_str(), exact_max);
    }

    #[test]
    #[should_panic(expected = "invalid static ProviderId slug")]
    fn new_panics_on_invalid_slug() {
        let _ = ProviderId::new("../evil");
    }

    #[test]
    fn rejects_empty_leading_punctuation_traversal_and_oversized_slugs() {
        assert!(ProviderId::try_new("").is_err());
        assert!(ProviderId::try_new("   ").is_err());
        assert!(ProviderId::try_new("-stripe").is_err());
        assert!(ProviderId::try_new("_stripe").is_err());
        assert!(ProviderId::try_new("../stripe").is_err());
        assert!(ProviderId::try_new("bad slug!").is_err());
        assert!(ProviderId::try_new("a".repeat(65)).is_err());
    }

    #[test]
    fn serde_validates_on_deserialize() {
        let ok: ProviderId = serde_json::from_str(r#""Moyasar""#).unwrap();
        assert_eq!(ok.as_str(), "moyasar");
        let err: Result<ProviderId, _> = serde_json::from_str(r#""../evil""#);
        assert!(err.is_err());
    }
}
