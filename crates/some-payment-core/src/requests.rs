//! Canonical request DTOs for [`crate::PaymentProvider`] operations.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

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
/// Maximum recipient identifier length in bytes for split instructions.
pub const MAX_RECIPIENT_ID_LEN: usize = 255;
/// Maximum URL length in bytes accepted for callback and redirect URLs.
pub const MAX_URL_LEN: usize = 2048;
/// Maximum order description length in bytes.
pub const MAX_DESCRIPTION_LEN: usize = 1000;
/// Maximum cancellation or refund reason length in bytes.
pub const MAX_REASON_LEN: usize = 500;

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
/// Implements custom [`fmt::Debug`] so `customer_email`, `customer_phone`,
/// `payment_token`, and any query/fragment parameters on `callback_url` are
/// redacted as `"[REDACTED]"` when present.
///
/// Callers MUST invoke [`Self::validate`] after deserializing untrusted
/// request payloads before passing them to a [`crate::PaymentProvider`].
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

/// Strips any `?query` or `#fragment` portion from a URL for safe `Debug` logging.
pub(crate) fn redact_url_query(url: &str) -> String {
    match url.find(['?', '#']) {
        Some(idx) => format!("{}?[REDACTED]", &url[..idx]),
        None => url.to_owned(),
    }
}

/// Redacts query/fragment parameters on an optional URL for safe `Debug` logging.
pub(crate) fn redact_opt_url(url: &Option<String>) -> Option<String> {
    url.as_deref().map(redact_url_query)
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
            .field("callback_url", &redact_opt_url(&self.callback_url))
            .field("customer_email", &redacted_opt(&self.customer_email))
            .field("customer_phone", &redacted_opt(&self.customer_phone))
            .field("payment_token", &redacted_opt(&self.payment_token))
            .field("splits", &self.splits)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl Drop for CreatePaymentRequest {
    fn drop(&mut self) {
        self.payment_token.zeroize();
        self.customer_email.zeroize();
        self.customer_phone.zeroize();
    }
}

impl CreatePaymentRequest {
    /// Creates a new [`CreatePaymentRequest`] with [`CaptureMode::Automatic`].
    #[must_use]
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
    #[must_use]
    pub fn with_capture_mode(mut self, mode: CaptureMode) -> Self {
        self.capture_mode = mode;
        self
    }

    /// Sets an explicit [`ProviderId`] override.
    #[must_use]
    pub fn with_provider_id(mut self, provider_id: ProviderId) -> Self {
        self.provider_id = Some(provider_id);
        self
    }

    /// Sets the caller-supplied idempotency key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Validates cross-gateway request invariants (order ID, country, description,
    /// URLs, idempotency key, metadata caps, and splits bounds).
    ///
    /// Callers MUST call this method after deserializing a [`CreatePaymentRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if any field violates length,
    /// character, URL scheme/host, metadata, or split-settlement invariants.
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_order_id(&self.order_id)?;
        validate_optional_country(self.buyer_country.as_deref())?;
        validate_optional_bounded_text(
            self.description.as_deref(),
            MAX_DESCRIPTION_LEN,
            "description",
        )?;
        validate_optional_idempotency_key(self.idempotency_key.as_deref())?;
        validate_optional_http_url(self.callback_url.as_deref(), "callback_url")?;
        validate_metadata(&self.metadata)?;
        validate_splits(&self.splits, self.money.amount())
    }
}

/// Canonical request to create a provider-hosted checkout page, session, or invoice.
///
/// Implements custom [`fmt::Debug`] so `customer_email`, `customer_phone`, and
/// any query/fragment parameters on `success_url` and `cancel_url` are redacted
/// as `"[REDACTED]"` when present.
///
/// Callers MUST invoke [`Self::validate`] after deserializing untrusted
/// request payloads before passing them to a [`crate::PaymentProvider`].
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
            .field("success_url", &redact_url_query(&self.success_url))
            .field("cancel_url", &redact_opt_url(&self.cancel_url))
            .field("description", &self.description)
            .field("customer_email", &redacted_opt(&self.customer_email))
            .field("customer_phone", &redacted_opt(&self.customer_phone))
            .field("capture_mode", &self.capture_mode)
            .field("idempotency_key", &self.idempotency_key)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl Drop for CheckoutSessionRequest {
    fn drop(&mut self) {
        self.customer_email.zeroize();
        self.customer_phone.zeroize();
    }
}

impl CheckoutSessionRequest {
    /// Creates a new [`CheckoutSessionRequest`] with [`CaptureMode::Automatic`].
    #[must_use]
    pub fn new(money: Money, order_id: impl Into<String>, success_url: impl Into<String>) -> Self {
        Self {
            provider_id: None,
            money,
            order_id: order_id.into(),
            success_url: success_url.into(),
            cancel_url: None,
            description: None,
            customer_email: None,
            customer_phone: None,
            capture_mode: CaptureMode::Automatic,
            idempotency_key: None,
            metadata: BTreeMap::new(),
        }
    }

    /// Sets an explicit [`ProviderId`] override.
    #[must_use]
    pub fn with_provider_id(mut self, provider_id: ProviderId) -> Self {
        self.provider_id = Some(provider_id);
        self
    }

    /// Sets the cancel redirect URL.
    #[must_use]
    pub fn with_cancel_url(mut self, cancel_url: impl Into<String>) -> Self {
        self.cancel_url = Some(cancel_url.into());
        self
    }

    /// Sets the order description shown on the hosted checkout page.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the customer email address.
    #[must_use]
    pub fn with_customer_email(mut self, email: impl Into<String>) -> Self {
        self.customer_email = Some(email.into());
        self
    }

    /// Sets the customer phone number.
    #[must_use]
    pub fn with_customer_phone(mut self, phone: impl Into<String>) -> Self {
        self.customer_phone = Some(phone.into());
        self
    }

    /// Sets the [`CaptureMode`].
    #[must_use]
    pub fn with_capture_mode(mut self, mode: CaptureMode) -> Self {
        self.capture_mode = mode;
        self
    }

    /// Sets the caller-supplied idempotency key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Sets the bounded metadata map.
    #[must_use]
    pub fn with_metadata(mut self, metadata: BTreeMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Validates cross-gateway checkout session request invariants.
    ///
    /// Callers MUST call this method after deserializing a [`CheckoutSessionRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `order_id`, `success_url`,
    /// `cancel_url`, `description`, `idempotency_key`, or `metadata` violate
    /// cross-gateway validation invariants.
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_order_id(&self.order_id)?;
        validate_http_url(&self.success_url, "success_url")?;
        validate_optional_http_url(self.cancel_url.as_deref(), "cancel_url")?;
        validate_optional_bounded_text(
            self.description.as_deref(),
            MAX_DESCRIPTION_LEN,
            "description",
        )?;
        validate_optional_idempotency_key(self.idempotency_key.as_deref())?;
        validate_metadata(&self.metadata)
    }
}

/// Request to capture a previously authorized hold (full when `amount` is `None`).
///
/// Callers MUST invoke [`Self::validate`] after deserializing untrusted
/// request payloads.
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
    /// Creates a new full-capture [`CaptureRequest`] for `reference`.
    #[must_use]
    pub fn new(reference: ProviderReference) -> Self {
        Self {
            reference,
            amount: None,
            idempotency_key: None,
        }
    }

    /// Sets an explicit partial capture amount.
    #[must_use]
    pub fn with_amount(mut self, amount: Money) -> Self {
        self.amount = Some(amount);
        self
    }

    /// Sets the caller-supplied idempotency key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Validates capture request invariants (idempotency key bounds).
    ///
    /// Callers MUST call this method after deserializing a [`CaptureRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `idempotency_key` is
    /// malformed or exceeds [`MAX_IDEMPOTENCY_KEY_LEN`] bytes.
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_optional_idempotency_key(self.idempotency_key.as_deref())
    }
}

/// Request to void/cancel an authorized hold or unsettled payment.
///
/// Callers MUST invoke [`Self::validate`] after deserializing untrusted
/// request payloads.
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
    /// Creates a new [`VoidRequest`] for `reference`.
    #[must_use]
    pub fn new(reference: ProviderReference) -> Self {
        Self {
            reference,
            reason: None,
            idempotency_key: None,
        }
    }

    /// Sets an optional cancellation reason.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Sets the caller-supplied idempotency key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Validates void request invariants (reason and idempotency key bounds).
    ///
    /// Callers MUST call this method after deserializing a [`VoidRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `reason` contains control
    /// characters or exceeds [`MAX_REASON_LEN`] bytes, or if `idempotency_key`
    /// is invalid.
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_optional_bounded_text(self.reason.as_deref(), MAX_REASON_LEN, "reason")?;
        validate_optional_idempotency_key(self.idempotency_key.as_deref())
    }
}

/// Request to refund a settled payment (full when `amount` is `None`).
///
/// Callers MUST invoke [`Self::validate`] after deserializing untrusted
/// request payloads.
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
    /// Creates a new full-refund [`RefundRequest`] for `reference`.
    #[must_use]
    pub fn new(reference: ProviderReference) -> Self {
        Self {
            reference,
            amount: None,
            reason: None,
            idempotency_key: None,
        }
    }

    /// Sets an explicit partial refund amount.
    #[must_use]
    pub fn with_amount(mut self, amount: Money) -> Self {
        self.amount = Some(amount);
        self
    }

    /// Sets an optional refund reason.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Sets the caller-supplied idempotency key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Validates refund request invariants (reason and idempotency key bounds).
    ///
    /// Callers MUST call this method after deserializing a [`RefundRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `reason` contains control
    /// characters or exceeds [`MAX_REASON_LEN`] bytes, or if `idempotency_key`
    /// is invalid.
    pub fn validate(&self) -> Result<(), PaymentError> {
        validate_optional_bounded_text(self.reason.as_deref(), MAX_REASON_LEN, "reason")?;
        validate_optional_idempotency_key(self.idempotency_key.as_deref())
    }
}

fn has_control_chars(value: &str) -> bool {
    value.bytes().any(|b| b.is_ascii_control())
}

fn validate_optional_bounded_text(
    value: Option<&str>,
    max_len: usize,
    field: &str,
) -> Result<(), PaymentError> {
    let Some(text) = value else {
        return Ok(());
    };
    if text.len() > max_len || has_control_chars(text) {
        return Err(PaymentError::validation(format!(
            "{field} must be <= {max_len} bytes without control characters"
        )));
    }
    Ok(())
}

fn validate_order_id(order_id: &str) -> Result<(), PaymentError> {
    if order_id.trim().is_empty()
        || order_id.len() > MAX_ORDER_ID_LEN
        || has_control_chars(order_id)
    {
        return Err(PaymentError::validation(
            "order_id must be 1..=255 non-whitespace bytes without control characters",
        ));
    }
    Ok(())
}

fn is_valid_idempotency_key(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= MAX_IDEMPOTENCY_KEY_LEN
        && raw == raw.trim()
        && raw.bytes().all(|b| b.is_ascii_graphic())
}

fn validate_optional_idempotency_key(key: Option<&str>) -> Result<(), PaymentError> {
    let Some(raw) = key else {
        return Ok(());
    };
    if !is_valid_idempotency_key(raw) {
        return Err(PaymentError::validation(
            "idempotency_key must be 1..=255 printable non-whitespace ASCII bytes",
        ));
    }
    Ok(())
}

fn validate_optional_country(country: Option<&str>) -> Result<(), PaymentError> {
    let Some(raw) = country else {
        return Ok(());
    };
    let is_two_ascii_letters = raw.len() == 2 && raw.bytes().all(|b| b.is_ascii_alphabetic());
    if !is_two_ascii_letters {
        return Err(PaymentError::validation(
            "buyer_country must be a 2-letter ISO-3166-1 alpha-2 ASCII code",
        ));
    }
    Ok(())
}

fn is_printable_ascii_url(rest: &str) -> bool {
    rest.chars()
        .all(|ch| ch.is_ascii() && !ch.is_ascii_control() && !ch.is_ascii_whitespace())
}

fn is_valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|p| p > 0)
}

fn is_valid_host_label(label: &str) -> bool {
    !label.is_empty()
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn is_valid_host(host: &str) -> bool {
    !host.is_empty() && host.split('.').all(is_valid_host_label)
}

fn is_valid_ipv6_authority(authority: &str) -> bool {
    let Some(rest) = authority.strip_prefix('[') else {
        return false;
    };
    let Some((host, suffix)) = rest.split_once(']') else {
        return false;
    };
    if host.parse::<std::net::Ipv6Addr>().is_err() {
        return false;
    }
    suffix.is_empty() || suffix.strip_prefix(':').is_some_and(is_valid_port)
}

fn is_valid_host_port(authority: &str) -> bool {
    if authority.contains('[') || authority.contains(']') {
        return false;
    }
    match authority.split_once(':') {
        Some((host, port)) => is_valid_host(host) && is_valid_port(port),
        None => is_valid_host(authority),
    }
}

fn is_valid_authority(authority: &str) -> bool {
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return false;
    }
    if authority.starts_with('[') {
        return is_valid_ipv6_authority(authority);
    }
    is_valid_host_port(authority)
}

fn extract_authority_host(authority: &str) -> Option<&str> {
    if authority.starts_with('[') {
        return authority
            .split_once(']')
            .map(|(host, _)| &authority[..=host.len()]);
    }
    Some(authority.split_once(':').map_or(authority, |(h, _)| h))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]"
}

fn is_scheme_allowed(is_https: bool, authority: &str) -> bool {
    is_https || extract_authority_host(authority).is_some_and(is_loopback_host)
}

fn parse_url_scheme(url: &str) -> Option<(bool, &str)> {
    if let Some(rest) = url.strip_prefix("https://") {
        return Some((true, rest));
    }
    url.strip_prefix("http://").map(|rest| (false, rest))
}

fn has_valid_host_and_chars(rest: &str) -> bool {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    is_printable_ascii_url(rest) && is_valid_authority(authority)
}

fn validate_http_url(url: &str, field: &str) -> Result<(), PaymentError> {
    if url.len() > MAX_URL_LEN {
        return Err(PaymentError::validation(format!(
            "{field} exceeds maximum length of {MAX_URL_LEN} bytes"
        )));
    }
    let Some((is_https, rest)) = parse_url_scheme(url) else {
        return Err(PaymentError::validation(format!(
            "{field} must use https:// (or http:// for loopback only)"
        )));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if !has_valid_host_and_chars(rest) {
        return Err(PaymentError::validation(format!(
            "{field} must be a valid URL"
        )));
    }
    if !is_scheme_allowed(is_https, authority) {
        return Err(PaymentError::validation(format!(
            "{field} must use https:// for non-loopback hosts"
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
    let recipient = split.recipient_id.as_str();
    if split.amount == 0
        || recipient.trim().is_empty()
        || recipient.len() > MAX_RECIPIENT_ID_LEN
        || has_control_chars(recipient)
    {
        return Err(PaymentError::validation(
            "split instructions require amount > 0 and recipient_id of 1..=255 bytes without control characters",
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
        let req = CreatePaymentRequest::new(money.clone(), "ORD-1")
            .with_capture_mode(CaptureMode::Manual)
            .with_provider_id(ProviderId::new("moyasar"))
            .with_idempotency_key("idem-key-1");
        assert_eq!(req.capture_mode, CaptureMode::Manual);
        assert_eq!(req.provider_id, Some(ProviderId::new("moyasar")));
        assert_eq!(req.idempotency_key.as_deref(), Some("idem-key-1"));
        assert!(req.validate().is_ok());

        let mut meta = BTreeMap::new();
        meta.insert("env".into(), "test".into());
        let checkout =
            CheckoutSessionRequest::new(money.clone(), "ORD-CHK-1", "https://example.com/ok")
                .with_provider_id(ProviderId::new("stripe"))
                .with_cancel_url("https://example.com/cancel")
                .with_description("Order 1")
                .with_customer_email("buyer@example.com")
                .with_customer_phone("+966501234567")
                .with_capture_mode(CaptureMode::Manual)
                .with_idempotency_key("chk-idem-1")
                .with_metadata(meta);
        assert!(checkout.validate().is_ok());
        assert_eq!(checkout.capture_mode, CaptureMode::Manual);

        let cap = CaptureRequest::new(sample_reference())
            .with_amount(money.clone())
            .with_idempotency_key("cap-1");
        assert!(cap.validate().is_ok());

        let void = VoidRequest::new(sample_reference())
            .with_reason("customer_canceled")
            .with_idempotency_key("void-1");
        assert!(void.validate().is_ok());

        let refund = RefundRequest::new(sample_reference())
            .with_amount(money)
            .with_reason("duplicate")
            .with_idempotency_key("ref-1");
        assert!(refund.validate().is_ok());
    }

    #[test]
    fn debug_redacts_pii_tokens_and_url_queries() {
        let money = Money::new(5_000, "SAR").unwrap();
        let mut req = CreatePaymentRequest::new(money.clone(), "ORD-PII");
        req.customer_email = Some("buyer@canary-domain.test".into());
        req.customer_phone = Some("+201001234567".into());
        req.payment_token = Some("tok_CANARY_999".into());
        req.callback_url = Some("https://example.com/cb?token=CB_QUERY_CANARY#frag".into());

        let dbg = format!("{req:?}");
        assert!(!dbg.contains("buyer@canary-domain.test"));
        assert!(!dbg.contains("+201001234567"));
        assert!(!dbg.contains("tok_CANARY_999"));
        assert!(!dbg.contains("CB_QUERY_CANARY"));
        assert!(dbg.contains("https://example.com/cb?[REDACTED]"));

        let checkout = CheckoutSessionRequest::new(
            money,
            "ORD-CHK",
            "https://example.com/ok?session=OK_CANARY_1",
        )
        .with_cancel_url("https://example.com/cancel#CANCEL_CANARY_2")
        .with_customer_email("checkout@canary-domain.test")
        .with_customer_phone("+966501234567");
        let chk_dbg = format!("{checkout:?}");
        assert!(!chk_dbg.contains("checkout@canary-domain.test"));
        assert!(!chk_dbg.contains("+966501234567"));
        assert!(!chk_dbg.contains("OK_CANARY_1"));
        assert!(!chk_dbg.contains("CANCEL_CANARY_2"));
        assert!(chk_dbg.contains("https://example.com/ok?[REDACTED]"));
        assert!(chk_dbg.contains("https://example.com/cancel?[REDACTED]"));
    }

    #[test]
    fn validates_order_id_country_url_and_metadata_bounds() {
        let money = Money::new(10_000, "SAR").unwrap();
        assert!(
            CreatePaymentRequest::new(money.clone(), "   ")
                .validate()
                .is_err()
        );
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
        let mut checkout =
            CheckoutSessionRequest::new(money.clone(), "ORD-CHK", "https://example.com/success")
                .with_cancel_url("http://localhost:3000/cancel")
                .with_idempotency_key("idem-1");
        assert!(checkout.validate().is_ok());

        checkout.success_url = "javascript:alert(1)".into();
        assert!(checkout.validate().is_err());
        checkout.success_url = "https://example.com/success".into();
        checkout.cancel_url = Some("   ".into());
        assert!(checkout.validate().is_err());

        let cap_ok = CaptureRequest::new(sample_reference())
            .with_amount(money.clone())
            .with_idempotency_key("cap-key");
        assert!(cap_ok.validate().is_ok());

        let cap_bad = CaptureRequest::new(sample_reference()).with_idempotency_key("   ");
        assert!(cap_bad.validate().is_err());

        let void_bad = VoidRequest::new(sample_reference()).with_idempotency_key("k".repeat(256));
        assert!(void_bad.validate().is_err());

        let refund_ok = RefundRequest::new(sample_reference())
            .with_amount(money)
            .with_reason("requested");
        assert!(refund_ok.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_idempotency_keys_control_chars_and_text_caps() {
        let money = Money::new(1_000, "SAR").unwrap();

        for bad_key in [
            " leading",
            "trailing ",
            "has space",
            "line\nbreak",
            "cr\rkey",
            "null\0key",
            "non-ascii-\u{00E9}",
            &format!("{}K", " ".repeat(256)),
        ] {
            let req =
                CreatePaymentRequest::new(money.clone(), "ORD-1").with_idempotency_key(bad_key);
            assert!(
                req.validate().is_err(),
                "expected error for key {bad_key:?}"
            );
        }

        for bad_order in [
            "ORD\r1",
            "ORD\n1",
            "ORD\x001",
            &format!("{}O", " ".repeat(256)),
        ] {
            let req = CreatePaymentRequest::new(money.clone(), bad_order);
            assert!(
                req.validate().is_err(),
                "expected error for order {bad_order:?}"
            );
        }

        let mut split_req = CreatePaymentRequest::new(money.clone(), "ORD-S");
        split_req.splits.push(SplitInstruction {
            recipient_id: "r".repeat(MAX_RECIPIENT_ID_LEN + 1),
            amount: 100,
            fee_source: false,
        });
        assert!(split_req.validate().is_err());

        split_req.splits[0].recipient_id = "acct\n1".into();
        assert!(split_req.validate().is_err());

        let mut desc_req = CreatePaymentRequest::new(money.clone(), "ORD-D");
        desc_req.description = Some("d".repeat(MAX_DESCRIPTION_LEN + 1));
        assert!(desc_req.validate().is_err());
        desc_req.description = Some("bad\0desc".into());
        assert!(desc_req.validate().is_err());

        let void_long =
            VoidRequest::new(sample_reference()).with_reason("r".repeat(MAX_REASON_LEN + 1));
        assert!(void_long.validate().is_err());

        let refund_ctrl = RefundRequest::new(sample_reference()).with_reason("bad\r\nreason");
        assert!(refund_ctrl.validate().is_err());

        let mut country_req = CreatePaymentRequest::new(money, "ORD-2");
        country_req.buyer_country = Some(" SA ".into());
        assert!(country_req.validate().is_err());
    }

    #[test]
    fn enforces_strict_url_length_port_host_and_https_rules() {
        let money = Money::new(1_000, "USD").unwrap();
        let oversized_url = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        let bad_urls = [
            oversized_url.as_str(),
            "https://?next=done",
            "https:///path",
            "https://#frag",
            "https://example.com/a b",
            "https://example.com/a\nb",
            "https://user:pass@example.com/cb",
            "https://@evil.com",
            "https://:443",
            "https://:",
            "https://evil.com\\@trusted.com",
            "https://example.com/\u{202E}cb",
            "https://[not-an-ip]/cb",
            "https://example.com:0/cb",
            "https://[::1]:0/cb",
            "https://-bad.example.com/cb",
            "https://bad-.example.com/cb",
            "https://a..b.com/cb",
            "https://bad_host.com/cb",
            "http://example.com/cb",
            "http://10.0.0.1/cb",
            "http://[2001:db8::1]/cb",
        ];
        for url in bad_urls {
            let mut req = CreatePaymentRequest::new(money.clone(), "ORD-URL");
            req.callback_url = Some(url.into());
            assert!(req.validate().is_err(), "expected error for {url}");
        }

        let good_urls = [
            "https://[::1]:8443/cb",
            "https://[::1]/cb",
            "https://example.com:8443/cb",
            "http://localhost:3000/cb",
            "http://LOCALHOST/cb",
            "http://127.0.0.1:8080/cb",
            "http://[::1]:8080/cb",
        ];
        for url in good_urls {
            let mut req = CreatePaymentRequest::new(money.clone(), "ORD-URL-OK");
            req.callback_url = Some(url.into());
            assert!(req.validate().is_ok(), "expected ok for {url}");
        }
    }

    #[test]
    fn exhaustive_request_dtos_serde_round_trip() {
        let money = Money::new(9_900, "SAR").unwrap();
        let mut create = CreatePaymentRequest::new(money.clone(), "ORD-SERDE-1")
            .with_provider_id(ProviderId::new("moyasar"))
            .with_capture_mode(CaptureMode::Manual)
            .with_idempotency_key("idem-serde-1");
        create.buyer_country = Some("SA".into());
        create.description = Some("Order desc".into());
        create.callback_url = Some("https://example.com/cb".into());
        create.customer_email = Some("buyer@example.com".into());
        create.customer_phone = Some("+966501234567".into());
        create.payment_token = Some("tok_1".into());
        create.splits.push(SplitInstruction {
            recipient_id: "acct_1".into(),
            amount: 5_000,
            fee_source: true,
        });
        create.metadata.insert("k".into(), "v".into());

        let c_json = serde_json::to_string(&create).unwrap();
        let c_back: CreatePaymentRequest = serde_json::from_str(&c_json).unwrap();
        assert_eq!(c_back, create);

        let checkout =
            CheckoutSessionRequest::new(money.clone(), "ORD-SERDE-2", "https://example.com/ok")
                .with_cancel_url("https://example.com/cancel")
                .with_description("Hosted checkout")
                .with_idempotency_key("idem-serde-2");
        let chk_json = serde_json::to_string(&checkout).unwrap();
        let chk_back: CheckoutSessionRequest = serde_json::from_str(&chk_json).unwrap();
        assert_eq!(chk_back, checkout);

        let capture = CaptureRequest::new(sample_reference())
            .with_amount(money.clone())
            .with_idempotency_key("idem-cap");
        let cap_json = serde_json::to_string(&capture).unwrap();
        let cap_back: CaptureRequest = serde_json::from_str(&cap_json).unwrap();
        assert_eq!(cap_back, capture);

        let void = VoidRequest::new(sample_reference())
            .with_reason("expired")
            .with_idempotency_key("idem-void");
        let v_json = serde_json::to_string(&void).unwrap();
        let v_back: VoidRequest = serde_json::from_str(&v_json).unwrap();
        assert_eq!(v_back, void);

        let refund = RefundRequest::new(sample_reference())
            .with_amount(money)
            .with_reason("customer_request")
            .with_idempotency_key("idem-refund");
        let r_json = serde_json::to_string(&refund).unwrap();
        let r_back: RefundRequest = serde_json::from_str(&r_json).unwrap();
        assert_eq!(r_back, refund);
    }
}
