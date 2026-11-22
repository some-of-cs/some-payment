//! Canonical payment results, [`PaymentStatus`], [`NextAction`], and webhook DTOs.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::money::Money;
use crate::provider_id::ProviderId;
use crate::reference::ProviderReference;

/// Canonical cross-gateway payment status.
///
/// # Examples
///
/// ```
/// use some_payment_core::PaymentStatus;
///
/// assert_eq!(PaymentStatus::from_slug("paid"), Some(PaymentStatus::Paid));
/// assert_eq!(PaymentStatus::from_slug("unknown"), None);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    /// Payment created and awaiting customer or gateway action.
    Initiated,
    /// Customer must complete 3DS, OTP, redirect, or wallet confirmation.
    RequiresAction,
    /// Gateway is asynchronously processing the payment.
    Processing,
    /// Funds are authorized/held awaiting capture.
    Authorized,
    /// Part of the authorized hold has been captured.
    PartiallyCaptured,
    /// Authorized hold has been captured.
    Captured,
    /// Payment succeeded and settled.
    Paid,
    /// Part of the settled payment has been refunded.
    PartiallyRefunded,
    /// Full settled amount has been refunded.
    Refunded,
    /// Payment or authorization was voided prior to settlement.
    Voided,
    /// Payment or checkout session was canceled/expired.
    Canceled,
    /// Payment attempt failed or was declined.
    Failed,
    /// Card verification / zero-value setup succeeded (Moyasar `verified`).
    SetupCompleted,
}

const KNOWN_PAYMENT_STATUSES: &[(&str, PaymentStatus)] = &[
    ("initiated", PaymentStatus::Initiated),
    ("requires_action", PaymentStatus::RequiresAction),
    ("processing", PaymentStatus::Processing),
    ("authorized", PaymentStatus::Authorized),
    ("partially_captured", PaymentStatus::PartiallyCaptured),
    ("captured", PaymentStatus::Captured),
    ("paid", PaymentStatus::Paid),
    ("partially_refunded", PaymentStatus::PartiallyRefunded),
    ("refunded", PaymentStatus::Refunded),
    ("voided", PaymentStatus::Voided),
    ("canceled", PaymentStatus::Canceled),
    ("failed", PaymentStatus::Failed),
    ("setup_completed", PaymentStatus::SetupCompleted),
];

impl PaymentStatus {
    /// Parses a snake_case status string via a static lookup table (`CC <= 3`).
    pub fn from_slug(slug: &str) -> Option<Self> {
        let normalized = slug.trim().to_ascii_lowercase();
        KNOWN_PAYMENT_STATUSES
            .iter()
            .find(|(k, _)| *k == normalized)
            .map(|(_, v)| *v)
    }
}

/// Canonical refund lifecycle status (aligned with Stripe's 5-state refund model).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    /// Refund is pending settlement.
    Pending,
    /// Refund requires customer action (e.g., bank details).
    RequiresAction,
    /// Refund succeeded.
    Succeeded,
    /// Refund failed.
    Failed,
    /// Refund was canceled.
    Canceled,
}

/// Next customer-facing step required to complete a payment.
///
/// Implements custom [`fmt::Debug`] so embedded `client_secret` tokens are
/// always rendered as `[REDACTED]`, and derives [`ZeroizeOnDrop`] so secret
/// buffers are scrubbed from memory on drop.
///
/// # Examples
///
/// ```
/// use some_payment_core::NextAction;
///
/// let action = NextAction::EmbeddedClientSecret {
///     client_secret: "pi_secret_example".into(),
/// };
/// let dbg = format!("{action:?}");
/// assert!(dbg.contains("[REDACTED]"));
/// assert!(!dbg.contains("pi_secret_example"));
/// ```
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NextAction {
    /// Browser redirect to 3DS challenge, hosted checkout, or invoice URL.
    Redirect {
        /// Target HTTPS URL.
        url: String,
    },
    /// Embedded client-side SDK secret (Stripe PaymentIntent, Moyasar PI, Paymob Pixel).
    EmbeddedClientSecret {
        /// Single-use or session client secret (redacted in `Debug`, zeroized on drop).
        client_secret: String,
    },
    /// Server/API-driven OTP challenge (e.g., Moyasar STC Pay — never browser-redirected).
    OtpChallenge {
        /// Endpoint URL used to submit the customer's OTP.
        submission_url: String,
    },
    /// Offline bill or kiosk payment instructions (e.g., Moyasar SADAD, Fawry Kiosk).
    OfflineBill {
        /// Reference number or bill code displayed to the customer.
        bill_reference: String,
        /// Optional biller/kiosk instructions URL.
        instructions_url: Option<String>,
    },
}

impl fmt::Debug for NextAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Redirect { url } => f.debug_struct("Redirect").field("url", url).finish(),
            Self::EmbeddedClientSecret { .. } => f
                .debug_struct("EmbeddedClientSecret")
                .field("client_secret", &"[REDACTED]")
                .finish(),
            Self::OtpChallenge { submission_url } => f
                .debug_struct("OtpChallenge")
                .field("submission_url", submission_url)
                .finish(),
            Self::OfflineBill {
                bill_reference,
                instructions_url,
            } => f
                .debug_struct("OfflineBill")
                .field("bill_reference", bill_reference)
                .field("instructions_url", instructions_url)
                .finish(),
        }
    }
}

/// Normalized result returned by create, checkout, fetch, capture, and void calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentResult {
    /// Gateway correlation identifiers.
    pub reference: ProviderReference,
    /// Canonical payment status.
    pub status: PaymentStatus,
    /// Total payment amount and currency.
    pub money: Money,
    /// Captured amount in minor units (if known).
    pub captured_amount: Option<u64>,
    /// Refunded amount in minor units (if known).
    pub refunded_amount: Option<u64>,
    /// Optional next customer-facing action.
    pub next_action: Option<NextAction>,
    /// Optional hosted checkout/invoice URL.
    pub checkout_url: Option<String>,
    /// Whether the transaction ran in live mode (`false` for sandbox/test).
    pub livemode: bool,
}

/// Normalized result returned by [`crate::PaymentProvider::refund`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundResult {
    /// Gateway refund identifier (e.g., Stripe `re_...`, Paymob child `transaction_id`).
    pub refund_id: String,
    /// Parent payment reference.
    pub payment_reference: ProviderReference,
    /// Canonical refund lifecycle status.
    pub status: RefundStatus,
    /// Refunded amount and currency.
    pub money: Money,
    /// Optional pending or failure reason from the gateway.
    pub reason_detail: Option<String>,
}

/// HTTP method of an incoming webhook or redirect callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HttpMethod {
    /// HTTP `GET` (e.g., Paymob customer return callback).
    Get,
    /// HTTP `POST` (authoritative server-to-server webhook delivery).
    Post,
}

/// Unparsed incoming webhook request preserving exact raw bytes for HMAC verification.
///
/// Implements custom [`fmt::Debug`] so raw webhook bytes, header values, and
/// query parameter values are never logged in plaintext.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use some_payment_core::{HttpMethod, RawWebhookRequest};
///
/// let mut headers = BTreeMap::new();
/// headers.insert("Stripe-Signature".into(), "t=1,v1=sig".into());
/// let req = RawWebhookRequest {
///     raw_body: b"{}".to_vec(),
///     headers,
///     query_params: BTreeMap::new(),
///     method: HttpMethod::Post,
/// };
/// assert_eq!(req.header("stripe-signature"), Some("t=1,v1=sig"));
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct RawWebhookRequest {
    /// Exact unparsed HTTP body bytes (never pre-parsed/re-serialized JSON).
    pub raw_body: Vec<u8>,
    /// HTTP headers (looked up case-insensitively via [`Self::header`]).
    pub headers: BTreeMap<String, String>,
    /// URL query parameters (e.g., Paymob `?hmac=...`).
    pub query_params: BTreeMap<String, String>,
    /// HTTP method (`GET` or `POST`).
    pub method: HttpMethod,
}

impl fmt::Debug for RawWebhookRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_headers: BTreeMap<&str, &str> = self
            .headers
            .keys()
            .map(|k| (k.as_str(), "[REDACTED]"))
            .collect();
        let redacted_query: BTreeMap<&str, &str> = self
            .query_params
            .keys()
            .map(|k| (k.as_str(), "[REDACTED]"))
            .collect();
        f.debug_struct("RawWebhookRequest")
            .field("method", &self.method)
            .field("raw_body_len", &self.raw_body.len())
            .field("headers", &redacted_headers)
            .field("query_params", &redacted_query)
            .finish()
    }
}

impl Drop for RawWebhookRequest {
    fn drop(&mut self) {
        self.raw_body.zeroize();
        for value in self.headers.values_mut() {
            value.zeroize();
        }
        for value in self.query_params.values_mut() {
            value.zeroize();
        }
    }
}

impl RawWebhookRequest {
    /// Performs a case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Looks up a query parameter by exact key.
    pub fn query(&self, key: &str) -> Option<&str> {
        self.query_params.get(key).map(String::as_str)
    }
}

/// One-shot webhook verification handshake challenge (e.g., Moyasar `webhook_verification`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct VerificationChallenge {
    /// Provider webhook registration identifier.
    pub webhook_id: String,
    /// 6-digit verification code (zeroized on drop, redacted in `Debug`).
    pub verification_code: String,
}

impl fmt::Debug for VerificationChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerificationChallenge")
            .field("webhook_id", &self.webhook_id)
            .field("verification_code", &"[REDACTED]")
            .finish()
    }
}

/// Verified, secret-scrubbed, provider-agnostic webhook event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedWebhookEvent {
    /// Provider that originated the event.
    pub provider_id: ProviderId,
    /// Provider-unique event identifier used for `(provider, event_id)` deduplication.
    pub event_id: String,
    /// Raw vendor event type string (e.g., `payment_intent.succeeded`, `payment_paid`).
    pub event_type: String,
    /// Whether the event originated in live mode (`false` for test/sandbox).
    pub livemode: bool,
    /// Whether this event is a thin event requiring API hydration (Stripe v2 / thin events).
    pub requires_hydration: bool,
    /// Optional correlated gateway reference.
    pub reference: Option<ProviderReference>,
    /// Optional normalized payment status.
    pub status: Option<PaymentStatus>,
    /// Optional payment amount and currency for order-total verification.
    pub money: Option<Money>,
    /// Populated when the provider requires a webhook verification handshake.
    pub verification_challenge: Option<VerificationChallenge>,
    /// Secret-scrubbed JSON payload (`secret_token` and credentials MUST be stripped).
    pub raw_redacted: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_embedded_client_secret_and_verification_code() {
        let action = NextAction::EmbeddedClientSecret {
            client_secret: "pi_secret_CANARY_123".into(),
        };
        let dbg = format!("{action:?}");
        assert!(!dbg.contains("CANARY_123"));
        assert!(dbg.contains("[REDACTED]"));

        let ch = VerificationChallenge {
            webhook_id: "wh_1".into(),
            verification_code: "654321".into(),
        };
        let ch_dbg = format!("{ch:?}");
        assert!(!ch_dbg.contains("654321"));
        assert!(ch_dbg.contains("[REDACTED]"));
    }

    #[test]
    fn raw_webhook_request_redacts_body_headers_and_query() {
        let mut headers = BTreeMap::new();
        headers.insert("Stripe-Signature".into(), "t=1,v1=HEADER_CANARY_456".into());
        let mut query_params = BTreeMap::new();
        query_params.insert("hmac".into(), "QUERY_CANARY_789".into());
        let raw = RawWebhookRequest {
            raw_body: b"{\"secret_token\":\"BODY_CANARY_123\"}".to_vec(),
            headers,
            query_params,
            method: HttpMethod::Post,
        };
        assert_eq!(
            raw.header("stripe-signature"),
            Some("t=1,v1=HEADER_CANARY_456")
        );
        assert_eq!(raw.query("hmac"), Some("QUERY_CANARY_789"));
        assert_eq!(raw.query("missing"), None);

        let dbg = format!("{raw:?}");
        assert!(!dbg.contains("BODY_CANARY_123"));
        assert!(!dbg.contains("HEADER_CANARY_456"));
        assert!(!dbg.contains("QUERY_CANARY_789"));
        assert!(dbg.contains("raw_body_len"));
        assert!(dbg.contains("[REDACTED]"));
    }

    #[test]
    fn status_lookup_and_next_action_variants() {
        assert_eq!(
            PaymentStatus::from_slug("setup_completed"),
            Some(PaymentStatus::SetupCompleted)
        );
        assert_eq!(PaymentStatus::from_slug("unknown"), None);

        let redirect = NextAction::Redirect {
            url: "https://3ds.example.com/challenge".into(),
        };
        assert!(format!("{redirect:?}").contains("https://3ds.example.com/challenge"));

        let otp = NextAction::OtpChallenge {
            submission_url: "https://api.moyasar.com/otp".into(),
        };
        assert!(format!("{otp:?}").contains("https://api.moyasar.com/otp"));

        let bill = NextAction::OfflineBill {
            bill_reference: "BILL-12345".into(),
            instructions_url: Some("https://fawry.example.com/help".into()),
        };
        let bill_dbg = format!("{bill:?}");
        assert!(bill_dbg.contains("BILL-12345"));
        assert!(bill_dbg.contains("https://fawry.example.com/help"));
    }

    #[test]
    fn result_and_webhook_dtos_serde_round_trip() {
        let reference = ProviderReference::builder(ProviderId::new("stripe"))
            .payment_id("pi_1")
            .build()
            .unwrap();
        let money = Money::new(10_000, "USD").unwrap();

        let payment = PaymentResult {
            reference: reference.clone(),
            status: PaymentStatus::RequiresAction,
            money: money.clone(),
            captured_amount: None,
            refunded_amount: Some(0),
            next_action: Some(NextAction::Redirect {
                url: "https://example.com/3ds".into(),
            }),
            checkout_url: None,
            livemode: false,
        };
        let p_json = serde_json::to_string(&payment).unwrap();
        let p_back: PaymentResult = serde_json::from_str(&p_json).unwrap();
        assert_eq!(p_back, payment);

        let refund = RefundResult {
            refund_id: "re_1".into(),
            payment_reference: reference.clone(),
            status: RefundStatus::Succeeded,
            money: money.clone(),
            reason_detail: Some("duplicate".into()),
        };
        let r_json = serde_json::to_string(&refund).unwrap();
        let r_back: RefundResult = serde_json::from_str(&r_json).unwrap();
        assert_eq!(r_back, refund);

        let event = NormalizedWebhookEvent {
            provider_id: ProviderId::new("stripe"),
            event_id: "evt_1".into(),
            event_type: "payment_intent.succeeded".into(),
            livemode: false,
            requires_hydration: false,
            reference: Some(reference),
            status: Some(PaymentStatus::Paid),
            money: Some(money),
            verification_challenge: None,
            raw_redacted: serde_json::json!({"id": "evt_1"}),
        };
        let e_json = serde_json::to_string(&event).unwrap();
        let e_back: NormalizedWebhookEvent = serde_json::from_str(&e_json).unwrap();
        assert_eq!(e_back, event);
    }
}
