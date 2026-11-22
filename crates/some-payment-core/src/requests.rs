//! Canonical request DTOs for [`crate::PaymentProvider`] operations.

use std::collections::BTreeMap;
use std::fmt;

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
/// Maximum marketplace split instructions per payment request.
pub const MAX_SPLITS: usize = 50;
/// Maximum caller-supplied idempotency key length in bytes.
pub const MAX_IDEMPOTENCY_KEY_LEN: usize = 255;
/// Maximum merchant order identifier length in bytes.
pub const MAX_ORDER_ID_LEN: usize = 255;

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
/// Implements custom [`fmt::Debug`] so `customer_email`, `customer_phone`, and
/// `payment_token` are redacted as `"[REDACTED]"` when present.
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

fn redacted_opt(value: &Option<String>) -> Option<&'static str> {
    value.as_ref().map(|_| "[REDACTED]")
}

impl fmt::Debug for CreatePaymentRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatePaymentRequest")
            .field("provider_id", &self.provider_id)
            .field("money", &self.money)
            .field("order_id", &self.order_id)
            .field("buyer_country", &self.buyer_country)
            .field("description", &self.description)
            .field("capture_mode", &self.capture_mode)
            .field("idempotency_key", &self.idempotency_key)
            .field("callback_url", &self.callback_url)
            .field("customer_email", &redacted_opt(&self.customer_email))
            .field("customer_phone", &redacted_opt(&self.customer_phone))
            .field("payment_token", &redacted_opt(&self.payment_token))
            .field("splits", &self.splits)
            .field("metadata", &self.metadata)
            .finish()
    }
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

    /// Validates cross-gateway request invariants (order ID, country, URLs,
    /// idempotency key, metadata caps, and splits bounds).
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_order_id(&self.order_id)?;
        validate_optional_country(self.buyer_country.as_deref())?;
        validate_optional_idempotency_key(self.idempotency_key.as_deref())?;
        validate_optional_http_url(self.callback_url.as_deref(), "callback_url")?;
        validate_metadata(&self.metadata)?;
        validate_splits(&self.splits, self.money.amount())
    }
}

/// Canonical request to create a provider-hosted checkout page, session, or invoice.
///
/// Implements custom [`fmt::Debug`] so `customer_email` and `customer_phone`
/// are redacted as `"[REDACTED]"` when present.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Debug for CheckoutSessionRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckoutSessionRequest")
            .field("provider_id", &self.provider_id)
            .field("money", &self.money)
            .field("order_id", &self.order_id)
            .field("success_url", &self.success_url)
            .field("cancel_url", &self.cancel_url)
            .field("description", &self.description)
            .field("customer_email", &redacted_opt(&self.customer_email))
            .field("customer_phone", &redacted_opt(&self.customer_phone))
            .field("capture_mode", &self.capture_mode)
            .field("idempotency_key", &self.idempotency_key)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl CheckoutSessionRequest {
    /// Validates cross-gateway checkout session request invariants.
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_order_id(&self.order_id)?;
        validate_http_url(&self.success_url, "success_url")?;
        validate_optional_http_url(self.cancel_url.as_deref(), "cancel_url")?;
        validate_optional_idempotency_key(self.idempotency_key.as_deref())?;
        validate_metadata(&self.metadata)
    }
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

impl CaptureRequest {
    /// Validates capture request invariants (idempotency key bounds).
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_optional_idempotency_key(self.idempotency_key.as_deref())
    }
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

impl VoidRequest {
    /// Validates void request invariants (idempotency key bounds).
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_optional_idempotency_key(self.idempotency_key.as_deref())
    }
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

impl RefundRequest {
    /// Validates refund request invariants (idempotency key bounds).
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_optional_idempotency_key(self.idempotency_key.as_deref())
    }
}

fn validate_order_id(order_id: &str) -> Result<(), PaymentError> {
    let trimmed = order_id.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_ORDER_ID_LEN {
        return Err(PaymentError::validation(
            "order_id must be 1..=255 non-whitespace bytes",
        ));
    }
    Ok(())
}

fn validate_optional_idempotency_key(key: Option<&str>) -> Result<(), PaymentError> {
    let Some(raw) = key else {
        return Ok(());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_IDEMPOTENCY_KEY_LEN {
        return Err(PaymentError::validation(
            "idempotency_key must be 1..=255 non-whitespace bytes",
        ));
    }
    Ok(())
}

fn validate_optional_country(country: Option<&str>) -> Result<(), PaymentError> {
    let Some(raw) = country else {
        return Ok(());
    };
    let trimmed = raw.trim();
    let is_two_ascii_letters =
        trimmed.len() == 2 && trimmed.bytes().all(|b| b.is_ascii_alphabetic());
    if !is_two_ascii_letters {
        return Err(PaymentError::validation(
            "buyer_country must be a 2-letter ISO-3166-1 alpha-2 ASCII code",
        ));
    }
    Ok(())
}

fn validate_http_url(url: &str, field: &str) -> Result<(), PaymentError> {
    let trimmed = url.trim();
    let has_scheme = trimmed.starts_with("https://") || trimmed.starts_with("http://");
    let has_host = trimmed
        .split_once("://")
        .is_some_and(|(_, rest)| !rest.is_empty());
    if !has_scheme || !has_host {
        return Err(PaymentError::validation(format!(
            "{field} must be a non-empty http:// or https:// URL"
        )));
    }
    Ok(())
}

fn validate_optional_http_url(url: Option<&str>, field: &str) -> Result<(), PaymentError> {
    match url {
        Some(raw) => validate_http_url(raw, field),
        None => Ok(()),
    }
}

fn validate_metadata_entry(key: &str, value: &str) -> Result<(), PaymentError> {
    if key.trim().is_empty() {
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

fn validate_split_item(split: &SplitInstruction) -> Result<u64, PaymentError> {
    if split.amount == 0 || split.recipient_id.trim().is_empty() {
        return Err(PaymentError::validation(
            "split instructions require non-empty recipient_id and amount > 0",
        ));
    }
    Ok(split.amount)
}

fn validate_splits(splits: &[SplitInstruction], total_amount: u64) -> Result<(), PaymentError> {
    if splits.len() > MAX_SPLITS {
        return Err(PaymentError::validation(
            "splits exceed maximum of 50 instructions",
        ));
    }
    let mut sum: u64 = 0;
    for split in splits {
        let amount = validate_split_item(split)?;
        sum = sum
            .checked_add(amount)
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

    fn sample_reference() -> ProviderReference {
        ProviderReference::builder(ProviderId::new("stripe"))
            .payment_id("pi_123")
            .build()
            .unwrap()
    }

    #[test]
    fn builder_helpers_and_capture_mode() {
        assert_eq!(CaptureMode::default(), CaptureMode::Automatic);
        let money = Money::new(5_000, "SAR").unwrap();
        let req = CreatePaymentRequest::new(money, "ORD-1")
            .with_capture_mode(CaptureMode::Manual)
            .with_provider_id(ProviderId::new("moyasar"))
            .with_idempotency_key("idem-key-1");
        assert_eq!(req.capture_mode, CaptureMode::Manual);
        assert_eq!(req.provider_id, Some(ProviderId::new("moyasar")));
        assert_eq!(req.idempotency_key.as_deref(), Some("idem-key-1"));
        assert!(req.validate().is_ok());
    }

    #[test]
    fn debug_redacts_pii_and_tokens() {
        let money = Money::new(5_000, "SAR").unwrap();
        let mut req = CreatePaymentRequest::new(money.clone(), "ORD-PII");
        req.customer_email = Some("buyer@canary-domain.test".into());
        req.customer_phone = Some("+201001234567".into());
        req.payment_token = Some("tok_CANARY_999".into());

        let dbg = format!("{req:?}");
        assert!(!dbg.contains("buyer@canary-domain.test"));
        assert!(!dbg.contains("+201001234567"));
        assert!(!dbg.contains("tok_CANARY_999"));
        assert!(dbg.contains("[REDACTED]"));

        let checkout = CheckoutSessionRequest {
            provider_id: None,
            money,
            order_id: "ORD-CHK".into(),
            success_url: "https://example.com/ok".into(),
            cancel_url: None,
            description: None,
            customer_email: Some("checkout@canary-domain.test".into()),
            customer_phone: Some("+966501234567".into()),
            capture_mode: CaptureMode::Automatic,
            idempotency_key: None,
            metadata: BTreeMap::new(),
        };
        let chk_dbg = format!("{checkout:?}");
        assert!(!chk_dbg.contains("checkout@canary-domain.test"));
        assert!(!chk_dbg.contains("+966501234567"));
        assert!(chk_dbg.contains("[REDACTED]"));
    }

    #[test]
    fn validates_order_id_country_url_and_metadata_bounds() {
        let money = Money::new(10_000, "SAR").unwrap();
        assert!(CreatePaymentRequest::new(money.clone(), "   ").validate().is_err());
        assert!(
            CreatePaymentRequest::new(money.clone(), "x".repeat(256))
                .validate()
                .is_err()
        );

        let mut req = CreatePaymentRequest::new(money, "ORD-1");
        req.buyer_country = Some("SA".into());
        req.callback_url = Some("https://example.com/cb".into());
        assert!(req.validate().is_ok());

        req.buyer_country = Some("USA".into());
        assert!(req.validate().is_err());
        req.buyer_country = Some("12".into());
        assert!(req.validate().is_err());
        req.buyer_country = None;

        req.callback_url = Some("ftp://example.com/cb".into());
        assert!(req.validate().is_err());
        req.callback_url = Some("https://".into());
        assert!(req.validate().is_err());
        req.callback_url = None;

        req.metadata.insert("   ".into(), "v".into());
        assert!(req.validate().is_err());
        req.metadata.clear();
        req.metadata.insert("k".repeat(41), "v".into());
        assert!(req.validate().is_err());
        req.metadata.clear();
        req.metadata.insert("k".into(), "v".repeat(501));
        assert!(req.validate().is_err());
        req.metadata.clear();

        for idx in 0..31 {
            req.metadata.insert(format!("k{idx}"), "v".into());
        }
        assert!(req.validate().is_err());
    }

    #[test]
    fn validates_splits_bounds_and_overflow() {
        let money = Money::new(1_000, "EGP").unwrap();
        let mut req = CreatePaymentRequest::new(money.clone(), "ORD-2");
        req.splits.push(SplitInstruction {
            recipient_id: "acct_1".into(),
            amount: 400,
            fee_source: false,
        });
        req.splits.push(SplitInstruction {
            recipient_id: "acct_2".into(),
            amount: 600,
            fee_source: true,
        });
        assert!(req.validate().is_ok());

        req.splits[0].amount = 0;
        assert!(req.validate().is_err());
        req.splits[0].amount = 400;
        req.splits[0].recipient_id = "   ".into();
        assert!(req.validate().is_err());

        let max_money = Money::new(u64::MAX, "EGP").unwrap();
        let mut overflow_req = CreatePaymentRequest::new(max_money, "O");
        overflow_req.splits.push(SplitInstruction {
            recipient_id: "a".into(),
            amount: u64::MAX,
            fee_source: false,
        });
        overflow_req.splits.push(SplitInstruction {
            recipient_id: "b".into(),
            amount: 1,
            fee_source: false,
        });
        assert!(overflow_req.validate().is_err());

        let mut max_count_req = CreatePaymentRequest::new(money.clone(), "ORD-MAX");
        for idx in 0..51 {
            max_count_req.splits.push(SplitInstruction {
                recipient_id: format!("acct_{idx}"),
                amount: 1,
                fee_source: false,
            });
        }
        assert!(max_count_req.validate().is_err());

        let mut exceed_req = CreatePaymentRequest::new(money, "ORD-3");
        exceed_req.splits.push(SplitInstruction {
            recipient_id: "acct_1".into(),
            amount: 1_001,
            fee_source: false,
        });
        assert!(exceed_req.validate().is_err());
    }

    #[test]
    fn validates_checkout_capture_void_and_refund_requests() {
        let money = Money::new(2_500, "USD").unwrap();
        let mut checkout = CheckoutSessionRequest {
            provider_id: None,
            money: money.clone(),
            order_id: "ORD-CHK".into(),
            success_url: "https://example.com/success".into(),
            cancel_url: Some("http://localhost:3000/cancel".into()),
            description: None,
            customer_email: None,
            customer_phone: None,
            capture_mode: CaptureMode::Automatic,
            idempotency_key: Some("idem-1".into()),
            metadata: BTreeMap::new(),
        };
        assert!(checkout.validate().is_ok());

        checkout.success_url = "javascript:alert(1)".into();
        assert!(checkout.validate().is_err());
        checkout.success_url = "https://example.com/success".into();
        checkout.cancel_url = Some("   ".into());
        assert!(checkout.validate().is_err());

        let cap_ok = CaptureRequest {
            reference: sample_reference(),
            amount: Some(money.clone()),
            idempotency_key: Some("cap-key".into()),
        };
        assert!(cap_ok.validate().is_ok());

        let cap_bad = CaptureRequest {
            reference: sample_reference(),
            amount: None,
            idempotency_key: Some("   ".into()),
        };
        assert!(cap_bad.validate().is_err());

        let void_bad = VoidRequest {
            reference: sample_reference(),
            reason: None,
            idempotency_key: Some("k".repeat(256)),
        };
        assert!(void_bad.validate().is_err());

        let refund_ok = RefundRequest {
            reference: sample_reference(),
            amount: Some(money),
            reason: Some("requested".into()),
            idempotency_key: None,
        };
        assert!(refund_ok.validate().is_ok());
    }
}
