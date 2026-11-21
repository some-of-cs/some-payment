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
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into().trim().to_ascii_lowercase())
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
    let valid_chars = bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_');
    if !valid_chars {
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
    fn normalizes_valid_slugs() {
        let id = ProviderId::try_new("  Stripe ").unwrap();
        assert_eq!(id.as_str(), "stripe");
        let custom = ProviderId::try_new("tap_payments-v2").unwrap();
        assert_eq!(custom.to_string(), "tap_payments-v2");
    }

    #[test]
    fn rejects_empty_traversal_and_oversized_slugs() {
        assert!(ProviderId::try_new("").is_err());
        assert!(ProviderId::try_new("   ").is_err());
        assert!(ProviderId::try_new("../stripe").is_err());
        assert!(ProviderId::try_new("bad slug!").is_err());
        assert!(ProviderId::try_new("a".repeat(65)).is_err());
    }

    #[test]
    fn serde_validates_on_deserialize() {
        let ok: ProviderId = serde_json::from_str(""Moyasar"").unwrap();
        assert_eq!(ok.as_str(), "moyasar");
        let err: Result<ProviderId, _> = serde_json::from_str(""../evil"");
        assert!(err.is_err());
    }
}
