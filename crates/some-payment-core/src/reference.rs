//! Cross-lifecycle [`ProviderReference`] correlating gateway identifiers.

use serde::{Deserialize, Serialize};

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    /// Primary payment identifier (e.g., Moyasar UUID or Stripe `pi_...`).
    pub fn payment_id(&self) -> Option<&str> {
        self.payment_id.as_deref()
    }

    /// Gateway transaction identifier (e.g., Paymob numeric `obj.id`).
    pub fn transaction_id(&self) -> Option<&str> {
        self.transaction_id.as_deref()
    }

    /// Merchant or gateway order identifier (e.g., Paymob `special_reference`).
    pub fn order_id(&self) -> Option<&str> {
        self.order_id.as_deref()
    }

    /// Payment intent identifier (Stripe `pi_...`, Moyasar intent, Paymob intention ID).
    pub fn intent_id(&self) -> Option<&str> {
        self.intent_id.as_deref()
    }

    /// Hosted checkout session or invoice identifier (Stripe `cs_...`, Moyasar invoice ID).
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

impl ProviderReferenceBuilder {
    /// Sets the primary payment ID.
    pub fn payment_id(mut self, id: impl Into<String>) -> Self {
        self.payment_id = Some(id.into());
        self
    }

    /// Sets the gateway transaction ID.
    pub fn transaction_id(mut self, id: impl Into<String>) -> Self {
        self.transaction_id = Some(id.into());
        self
    }

    /// Sets the merchant/gateway order ID.
    pub fn order_id(mut self, id: impl Into<String>) -> Self {
        self.order_id = Some(id.into());
        self
    }

    /// Sets the payment intent ID.
    pub fn intent_id(mut self, id: impl Into<String>) -> Self {
        self.intent_id = Some(id.into());
        self
    }

    /// Sets the hosted checkout session or invoice ID.
    pub fn checkout_session_id(mut self, id: impl Into<String>) -> Self {
        self.checkout_session_id = Some(id.into());
        self
    }

    /// Validates that at least one identifier is populated and within length bounds.
    pub fn build(self) -> Result<ProviderReference, PaymentError> {
        let fields = [
            self.payment_id.as_deref(),
            self.transaction_id.as_deref(),
            self.order_id.as_deref(),
            self.intent_id.as_deref(),
            self.checkout_session_id.as_deref(),
        ];
        let mut any_present = false;
        for field in fields.into_iter().flatten() {
            let trimmed = field.trim();
            if trimmed.is_empty() || trimmed.len() > MAX_REFERENCE_ID_LEN {
                return Err(PaymentError::validation(
                    "reference identifiers must be 1..=255 non-whitespace bytes",
                ));
            }
            any_present = true;
        }
        if !any_present {
            return Err(PaymentError::validation(
                "ProviderReference requires at least one populated identifier",
            ));
        }
        Ok(ProviderReference {
            provider_id: self.provider_id,
            payment_id: self.payment_id,
            transaction_id: self.transaction_id,
            order_id: self.order_id,
            intent_id: self.intent_id,
            checkout_session_id: self.checkout_session_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_validates_bounds() {
        let reference = ProviderReference::builder(ProviderId::new("paymob"))
            .order_id("ORD-123")
            .transaction_id("456789")
            .build()
            .unwrap();
        let json = serde_json::to_string(&reference).unwrap();
        let decoded: ProviderReference = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, reference);

        assert!(ProviderReference::builder(ProviderId::new("stripe"))
            .build()
            .is_err());
        assert!(ProviderReference::builder(ProviderId::new("stripe"))
            .payment_id("x".repeat(256))
            .build()
            .is_err());
    }
}
