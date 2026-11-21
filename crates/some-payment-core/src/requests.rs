//! Canonical request DTOs for [`crate::PaymentProvider`] operations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::PaymentError;
use crate::money::Money;
use crate::provider_id::ProviderId;
use crate::reference::ProviderReference;

/// Maximum metadata entries accepted across gateways (aligned to Moyasar's 30-key cap).
pub const MAX_METADATA_ENTRIES: usize = 30;
/// Maximum metadata key length in bytes.
pub const MAX_METADATA_KEY_LEN: usize = 40;
/// Maximum metadata value length in bytes.
pub const MAX_METADATA_VALUE_LEN: usize = 500;

/// Controls whether a payment captures immediately or places an authorization hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// Automatically capture funds upon authorization.
    #[default]
    Automatic,
    /// Authorize funds only; requires a subsequent [`crate::PaymentProvider::capture`] call.
    Manual,
}

/// Marketplace split settlement instruction for a recipient sub-account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitInstruction {
    /// Provider-specific recipient or connected account identifier.
    pub recipient_id: String,
    /// Positive integer minor-unit amount allocated to this recipient.
    pub amount: u64,
    /// Optional fee bearer flag.
    pub fee_source: bool,
}

/// Canonical request to create a direct payment or embedded payment intent.
///
/// # Examples
///
/// ```
/// use some_payment_core::{CaptureMode, CreatePaymentRequest, Money};
///
/// let money = Money::new(15_000, "SAR").unwrap();
/// let req = CreatePaymentRequest::new(money, "ORD-SA-9901")
///     .with_capture_mode(CaptureMode::Automatic);
/// assert_eq!(req.order_id, "ORD-SA-9901");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePaymentRequest {
    /// Optional explicit provider override; takes precedence over routing policies.
    pub provider_id: Option<ProviderId>,
    /// Positive minor-unit amount and ISO-4217 currency.
    pub money: Money,
    /// Merchant order identifier (mapped to Paymob `special_reference`, Stripe metadata, etc.).
    pub order_id: String,
    /// Optional buyer ISO-3166-1 alpha-2 country code for regional routing (e.g., `"SA"`, `"EG"`).
    pub buyer_country: Option<String>,
    /// Optional human-readable order description.
    pub description: Option<String>,
    /// Automatic vs manual capture mode.
    pub capture_mode: CaptureMode,
    /// Caller-supplied idempotency key (UUID v4 recommended; `<= 255` chars).
    pub idempotency_key: Option<String>,
    /// Optional return/callback URL after 3DS or wallet redirects.
    pub callback_url: Option<String>,
    /// Optional customer email address.
    pub customer_email: Option<String>,
    /// Optional customer phone number (mandatory for Paymob `billing_data`).
    pub customer_phone: Option<String>,
    /// Optional tokenized payment source or method hint (never raw server-side PAN).
    pub payment_token: Option<String>,
    /// Optional marketplace split settlement instructions.
    pub splits: Vec<SplitInstruction>,
    /// Bounded key-value metadata attached to the payment.
    pub metadata: BTreeMap<String, String>,
}

impl CreatePaymentRequest {
    /// Creates a new [`CreatePaymentRequest`] with [`CaptureMode::Automatic`].
    pub fn new(money: Money, order_id: impl Into<String>) -> Self {
        Self {
            provider_id: None,
            money,
            order_id: order_id.into(),
            buyer_country: None,
            description: None,
            capture_mode: CaptureMode::Automatic,
            idempotency_key: None,
            callback_url: None,
            customer_email: None,
            customer_phone: None,
            payment_token: None,
            splits: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    /// Sets the [`CaptureMode`].
    pub fn with_capture_mode(mut self, mode: CaptureMode) -> Self {
        self.capture_mode = mode;
        self
    }

    /// Sets an explicit [`ProviderId`] override.
    pub fn with_provider_id(mut self, provider_id: ProviderId) -> Self {
        self.provider_id = Some(provider_id);
        self
    }

    /// Sets the caller-supplied idempotency key.
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Validates cross-gateway request invariants (order ID, metadata caps, splits sum).
    pub fn validate(&self) -> Result<(), PaymentError> {
        if self.order_id.trim().is_empty() {
            return Err(PaymentError::validation("order_id must not be empty"));
        }
        validate_metadata(&self.metadata)?;
        validate_splits(&self.splits, self.money.amount())
    }
}

/// Canonical request to create a provider-hosted checkout page, session, or invoice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutSessionRequest {
    /// Optional explicit provider override.
    pub provider_id: Option<ProviderId>,
    /// Positive minor-unit amount and currency.
    pub money: Money,
    /// Merchant order identifier.
    pub order_id: String,
    /// Success redirect URL.
    pub success_url: String,
    /// Cancel redirect URL.
    pub cancel_url: Option<String>,
    /// Optional description shown on hosted page.
    pub description: Option<String>,
    /// Optional customer email.
    pub customer_email: Option<String>,
    /// Optional customer phone.
    pub customer_phone: Option<String>,
    /// Automatic vs manual capture mode.
    pub capture_mode: CaptureMode,
    /// Caller-supplied idempotency key.
    pub idempotency_key: Option<String>,
    /// Bounded metadata map.
    pub metadata: BTreeMap<String, String>,
}

/// Request to capture a previously authorized hold (full when `amount` is `None`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRequest {
    /// Gateway reference identifying the authorized payment/transaction.
    pub reference: ProviderReference,
    /// Optional partial capture amount (`None` captures the full authorized amount).
    pub amount: Option<Money>,
    /// Caller-supplied idempotency key for distributed mutation locking.
    pub idempotency_key: Option<String>,
}

/// Request to void/cancel an authorized hold or unsettled payment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoidRequest {
    /// Gateway reference identifying the payment/transaction.
    pub reference: ProviderReference,
    /// Optional cancellation reason.
    pub reason: Option<String>,
    /// Caller-supplied idempotency key.
    pub idempotency_key: Option<String>,
}

/// Request to refund a settled payment (full when `amount` is `None`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundRequest {
    /// Gateway reference identifying the settled payment/transaction.
    pub reference: ProviderReference,
    /// Optional partial refund amount (`None` requests a full remaining refund).
    pub amount: Option<Money>,
    /// Optional refund reason (silently ignored by gateways without a reason field).
    pub reason: Option<String>,
    /// Caller-supplied idempotency key for distributed mutation locking.
    pub idempotency_key: Option<String>,
}

fn validate_metadata_entry(key: &str, value: &str) -> Result<(), PaymentError> {
    if key.is_empty() {
        return Err(PaymentError::validation("metadata key must not be empty"));
    }
    if key.len() > MAX_METADATA_KEY_LEN {
        return Err(PaymentError::validation("metadata key exceeds 40 bytes"));
    }
    if value.len() > MAX_METADATA_VALUE_LEN {
        return Err(PaymentError::validation("metadata value exceeds 500 bytes"));
    }
    Ok(())
}

fn validate_metadata(metadata: &BTreeMap<String, String>) -> Result<(), PaymentError> {
    if metadata.len() > MAX_METADATA_ENTRIES {
        return Err(PaymentError::validation(
            "metadata exceeds maximum of 30 entries",
        ));
    }
    for (k, v) in metadata {
        validate_metadata_entry(k, v)?;
    }
    Ok(())
}

fn validate_splits(splits: &[SplitInstruction], total_amount: u64) -> Result<(), PaymentError> {
    if splits.is_empty() {
        return Ok(());
    }
    let mut sum: u64 = 0;
    for split in splits {
        if split.amount == 0 || split.recipient_id.trim().is_empty() {
            return Err(PaymentError::validation(
                "split instructions require non-empty recipient_id and amount > 0",
            ));
        }
        sum = sum
            .checked_add(split.amount)
            .ok_or_else(|| PaymentError::validation("split amounts overflow u64"))?;
    }
    if sum > total_amount {
        return Err(PaymentError::validation(
            "sum of split amounts exceeds total payment amount",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_metadata_and_splits_bounds() {
        let money = Money::new(10_000, "SAR").unwrap();
        let mut req = CreatePaymentRequest::new(money, "ORD-1");
        assert!(req.validate().is_ok());

        for idx in 0..31 {
            req.metadata.insert(format!("k{idx}"), "v".into());
        }
        assert!(req.validate().is_err());
    }

    #[test]
    fn rejects_splits_exceeding_total() {
        let money = Money::new(1_000, "EGP").unwrap();
        let mut req = CreatePaymentRequest::new(money, "ORD-2");
        req.splits.push(SplitInstruction {
            recipient_id: "acct_1".into(),
            amount: 1_001,
            fee_source: false,
        });
        assert!(req.validate().is_err());
    }
}
