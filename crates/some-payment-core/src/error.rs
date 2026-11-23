//! Typed, secret-redacted error taxonomy for `some-payment-core`.

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::provider_id::ProviderId;

/// High-level error classification for retryability and telemetry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Local input or invariant validation failure.
    Validation,
    /// Missing or invalid provider/orchestrator configuration.
    Configuration,
    /// Requested operation is not supported by the target provider.
    Unsupported,
    /// Customer payment instrument was declined.
    CardDeclined,
    /// Customer 3-D Secure authentication failed (never merchant auth).
    ThreeDsFailed,
    /// Merchant API credentials were rejected or out of scope.
    Authentication,
    /// Upstream provider rate limit hit (HTTP 429).
    RateLimited,
    /// Upstream provider is temporarily unavailable (HTTP 5xx / transport).
    ProviderUnavailable,
    /// Cryptographic or token webhook verification failed.
    WebhookVerificationFailed,
    /// Concurrent worker holds an active idempotency lock.
    IdempotencyConflict,
    /// A `before` lifecycle hook aborted execution prior to network I/O.
    AbortedByHook,
    /// Unclassified upstream provider error.
    Provider,
}

const ERROR_KIND_META: &[(&str, ErrorKind, &str)] = &[
    ("validation", ErrorKind::Validation, "validation error"),
    (
        "configuration",
        ErrorKind::Configuration,
        "configuration error",
    ),
    (
        "unsupported",
        ErrorKind::Unsupported,
        "unsupported operation",
    ),
    ("card_declined", ErrorKind::CardDeclined, "payment declined"),
    (
        "three_ds_failed",
        ErrorKind::ThreeDsFailed,
        "3DS authentication failed",
    ),
    (
        "authentication",
        ErrorKind::Authentication,
        "merchant authentication failed",
    ),
    ("rate_limited", ErrorKind::RateLimited, "rate limited"),
    (
        "provider_unavailable",
        ErrorKind::ProviderUnavailable,
        "provider unavailable",
    ),
    (
        "webhook_verification_failed",
        ErrorKind::WebhookVerificationFailed,
        "webhook verification failed",
    ),
    (
        "idempotency_conflict",
        ErrorKind::IdempotencyConflict,
        "idempotency conflict",
    ),
    (
        "aborted_by_hook",
        ErrorKind::AbortedByHook,
        "aborted by lifecycle hook",
    ),
    ("provider", ErrorKind::Provider, "provider error"),
];

const RETRYABLE_KINDS: &[ErrorKind] = &[
    ErrorKind::RateLimited,
    ErrorKind::ProviderUnavailable,
    ErrorKind::IdempotencyConflict,
];

const DEFINITIVE_KINDS: &[ErrorKind] = &[
    ErrorKind::Validation,
    ErrorKind::Configuration,
    ErrorKind::Unsupported,
    ErrorKind::Authentication,
    ErrorKind::AbortedByHook,
];

impl ErrorKind {
    /// Parses a snake_case error kind string using a static lookup table.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        let normalized = slug.trim().to_ascii_lowercase();
        ERROR_KIND_META
            .iter()
            .find(|(k, _, _)| *k == normalized)
            .map(|(_, v, _)| *v)
    }

    /// Returns a safe, static human-readable summary for this error kind.
    #[must_use]
    pub fn summary(self) -> &'static str {
        ERROR_KIND_META
            .iter()
            .find(|(_, k, _)| *k == self)
            .map_or("payment error", |(_, _, msg)| *msg)
    }
}

/// Canonical error returned by [`crate::PaymentProvider`] and orchestrator operations.
///
/// Implements custom [`fmt::Display`] and [`fmt::Debug`] formatting that never
/// interpolate raw provider-sourced `message` or `reason` payloads, ensuring
/// secret keys, webhook secrets, or cardholder data cannot leak into logs or
/// panic traces. Sensitive string buffers are zeroized on drop.
///
/// # Examples
///
/// ```
/// use some_payment_core::{PaymentError, ProviderId};
///
/// let err = PaymentError::unsupported(&ProviderId::new("paymob"), "splits");
/// assert!(!err.is_retryable());
/// ```
#[derive(Clone)]
pub enum PaymentError {
    /// Input failed local validation prior to network I/O.
    ValidationError {
        /// Human-readable validation message (never interpolated verbatim in `Display`/`Debug`).
        message: String,
    },

    /// Missing or invalid provider/orchestrator configuration.
    ConfigurationError {
        /// Human-readable configuration error.
        message: String,
    },

    /// Provider does not claim the capability required for an operation.
    Unsupported {
        /// Target provider slug.
        provider: ProviderId,
        /// Operation or capability name.
        operation: String,
    },

    /// Customer card or wallet payment was declined.
    CardDeclined {
        /// Target provider slug.
        provider: ProviderId,
        /// Vendor decline code (e.g., `insufficient_funds`).
        decline_code: Option<String>,
        /// Vendor advice code if available.
        advice_code: Option<String>,
        /// Provider message (never interpolated verbatim in `Display`/`Debug`).
        message: String,
    },

    /// Customer failed 3-D Secure authentication (distinct from merchant auth).
    ThreeDsFailed {
        /// Target provider slug.
        provider: ProviderId,
        /// Vendor error code (e.g., `3ds_auth_error`).
        code: Option<String>,
        /// Provider message (never interpolated verbatim in `Display`/`Debug`).
        message: String,
    },

    /// Merchant credentials were rejected (HTTP 401) or wrong key scope.
    AuthenticationError {
        /// Target provider slug.
        provider: ProviderId,
        /// Provider message (never interpolated verbatim in `Display`/`Debug`).
        message: String,
    },

    /// Upstream gateway rate-limited the request (HTTP 429).
    RateLimited {
        /// Target provider slug.
        provider: ProviderId,
        /// Optional `Retry-After` hint in seconds.
        retry_after_secs: Option<u64>,
    },

    /// Upstream gateway is temporarily unavailable or timed out.
    ProviderUnavailable {
        /// Target provider slug.
        provider: ProviderId,
        /// Optional HTTP status code.
        http_status: Option<u16>,
        /// Whether the request may have been processed (e.g. network timeout).
        indeterminate: bool,
        /// Provider message (never interpolated verbatim in `Display`/`Debug`).
        message: String,
    },

    /// Webhook signature or secret-token verification failed.
    WebhookVerificationFailed {
        /// Target provider slug.
        provider: ProviderId,
        /// Failure reason (never interpolated verbatim in `Display`/`Debug`).
        reason: String,
    },

    /// Another worker currently holds the idempotency lock for this key.
    IdempotencyConflict {
        /// Namespaced idempotency scope.
        scope: String,
    },

    /// A `before` lifecycle hook aborted execution.
    AbortedByHook {
        /// Reason returned by the hook (never interpolated verbatim in `Display`/`Debug`).
        reason: String,
    },

    /// Structured provider error preserving HTTP status and vendor code.
    ProviderError {
        /// Target provider slug.
        provider: ProviderId,
        /// HTTP status code if available.
        http_status: Option<u16>,
        /// Vendor-specific error code.
        vendor_code: Option<String>,
        /// Provider message (never interpolated verbatim in `Display`/`Debug`).
        message: String,
    },
}

impl fmt::Display for PaymentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind().summary())?;
        if let Some(provider) = self.provider() {
            write!(f, " (provider={provider})")?;
        }
        if let Some(op) = self.operation() {
            write!(f, " (operation={op})")?;
        }
        if let Some(status) = self.http_status() {
            write!(f, " (status={status})")?;
        }
        Ok(())
    }
}

impl std::error::Error for PaymentError {}

impl fmt::Debug for PaymentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentError")
            .field("kind", &self.kind())
            .field("provider", &self.provider())
            .field("operation", &self.operation())
            .field("http_status", &self.http_status())
            .field("retryable", &self.is_retryable())
            .field("decline_code", &self.decline_code())
            .field("vendor_code", &self.vendor_code())
            .field("retry_after_secs", &self.retry_after_secs())
            .field("indeterminate", &self.is_indeterminate())
            .field("message", &"[REDACTED]")
            .finish()
    }
}

impl PaymentError {
    /// Constructs a [`PaymentError::ValidationError`].
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::ValidationError {
            message: message.into(),
        }
    }

    /// Constructs a [`PaymentError::ConfigurationError`].
    #[must_use]
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::ConfigurationError {
            message: message.into(),
        }
    }

    /// Constructs a [`PaymentError::Unsupported`] error.
    #[must_use]
    pub fn unsupported(provider: &ProviderId, operation: impl Into<String>) -> Self {
        Self::Unsupported {
            provider: provider.clone(),
            operation: operation.into(),
        }
    }

    fn client_kind(&self) -> Option<ErrorKind> {
        match self {
            Self::ValidationError { .. } => Some(ErrorKind::Validation),
            Self::ConfigurationError { .. } => Some(ErrorKind::Configuration),
            Self::Unsupported { .. } => Some(ErrorKind::Unsupported),
            Self::AbortedByHook { .. } => Some(ErrorKind::AbortedByHook),
            _ => None,
        }
    }

    fn payment_flow_kind(&self) -> Option<ErrorKind> {
        match self {
            Self::CardDeclined { .. } => Some(ErrorKind::CardDeclined),
            Self::ThreeDsFailed { .. } => Some(ErrorKind::ThreeDsFailed),
            Self::AuthenticationError { .. } => Some(ErrorKind::Authentication),
            Self::WebhookVerificationFailed { .. } => Some(ErrorKind::WebhookVerificationFailed),
            _ => None,
        }
    }

    fn transport_kind(&self) -> ErrorKind {
        match self {
            Self::RateLimited { .. } => ErrorKind::RateLimited,
            Self::ProviderUnavailable { .. } => ErrorKind::ProviderUnavailable,
            Self::IdempotencyConflict { .. } => ErrorKind::IdempotencyConflict,
            _ => ErrorKind::Provider,
        }
    }

    /// Returns the coarse [`ErrorKind`] for telemetry and classification.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.client_kind()
            .or_else(|| self.payment_flow_kind())
            .unwrap_or_else(|| self.transport_kind())
    }

    fn primary_provider(&self) -> Option<&ProviderId> {
        match self {
            Self::Unsupported { provider, .. } => Some(provider),
            Self::CardDeclined { provider, .. } => Some(provider),
            Self::ThreeDsFailed { provider, .. } => Some(provider),
            Self::AuthenticationError { provider, .. } => Some(provider),
            _ => None,
        }
    }

    fn secondary_provider(&self) -> Option<&ProviderId> {
        match self {
            Self::RateLimited { provider, .. } => Some(provider),
            Self::ProviderUnavailable { provider, .. } => Some(provider),
            Self::WebhookVerificationFailed { provider, .. } => Some(provider),
            Self::ProviderError { provider, .. } => Some(provider),
            _ => None,
        }
    }

    /// Returns the associated [`ProviderId`] if this error originated from a provider.
    #[must_use]
    pub fn provider(&self) -> Option<&ProviderId> {
        self.primary_provider()
            .or_else(|| self.secondary_provider())
    }

    fn operation(&self) -> Option<&str> {
        match self {
            Self::Unsupported { operation, .. } => Some(operation.as_str()),
            _ => None,
        }
    }

    /// Returns `true` if the operation may safely be retried with the same
    /// idempotency key.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        RETRYABLE_KINDS.contains(&self.kind())
    }

    /// Returns `true` if this error represents a definitive local or 4xx client
    /// rejection where an idempotency lock may be safely released.
    ///
    /// Ambiguous client statuses (`408 Request Timeout`, `409 Conflict`,
    /// `425 Too Early`, `429 Too Many Requests`, and `499 Client Closed Request`)
    /// intentionally return `false` because the upstream mutation may still be
    /// in flight or already committed.
    ///
    /// [`PaymentError::CardDeclined`] and [`PaymentError::ThreeDsFailed`] also
    /// intentionally return `false`: the request already reached the gateway
    /// and issuer network, so callers should initiate a new payment attempt
    /// with a fresh idempotency key rather than releasing and reusing the lock.
    #[must_use]
    pub fn is_definitive_client_rejection(&self) -> bool {
        if matches!(self, Self::ProviderUnavailable { .. }) {
            return false;
        }
        if DEFINITIVE_KINDS.contains(&self.kind()) {
            return true;
        }
        matches!(
            self.http_status(),
            Some(400..=407 | 410..=424 | 426..=428 | 431 | 451)
        )
    }

    /// Returns the upstream HTTP status code if available.
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::RateLimited { .. } => Some(429),
            Self::AuthenticationError { .. } => Some(401),
            Self::ProviderUnavailable { http_status, .. } => *http_status,
            Self::ProviderError { http_status, .. } => *http_status,
            _ => None,
        }
    }

    /// Returns the vendor decline code if this is a [`PaymentError::CardDeclined`].
    #[must_use]
    pub fn decline_code(&self) -> Option<&str> {
        match self {
            Self::CardDeclined { decline_code, .. } => decline_code.as_deref(),
            _ => None,
        }
    }

    /// Returns the `Retry-After` hint in seconds if this is a [`PaymentError::RateLimited`].
    #[must_use]
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::RateLimited {
                retry_after_secs, ..
            } => *retry_after_secs,
            _ => None,
        }
    }

    /// Returns `true` if this is a [`PaymentError::ProviderUnavailable`] whose
    /// upstream execution outcome is indeterminate (e.g., network timeout).
    #[must_use]
    pub fn is_indeterminate(&self) -> bool {
        matches!(
            self,
            Self::ProviderUnavailable {
                indeterminate: true,
                ..
            }
        )
    }

    /// Returns the non-secret vendor error code when present on
    /// [`PaymentError::ProviderError`] or [`PaymentError::ThreeDsFailed`].
    #[must_use]
    pub fn vendor_code(&self) -> Option<&str> {
        match self {
            Self::ProviderError { vendor_code, .. } => vendor_code.as_deref(),
            Self::ThreeDsFailed { code, .. } => code.as_deref(),
            _ => None,
        }
    }

    fn zeroize_primary_buffers(&mut self) {
        match self {
            Self::ValidationError { message }
            | Self::ConfigurationError { message }
            | Self::AuthenticationError { message, .. }
            | Self::ProviderUnavailable { message, .. } => message.zeroize(),
            Self::WebhookVerificationFailed { reason, .. } => reason.zeroize(),
            Self::AbortedByHook { reason } => reason.zeroize(),
            _ => {}
        }
    }

    fn zeroize_coded_buffers(&mut self) {
        match self {
            Self::CardDeclined {
                decline_code,
                advice_code,
                message,
                ..
            } => {
                decline_code.zeroize();
                advice_code.zeroize();
                message.zeroize();
            }
            Self::ThreeDsFailed { code, message, .. } => {
                code.zeroize();
                message.zeroize();
            }
            Self::ProviderError {
                vendor_code,
                message,
                ..
            } => {
                vendor_code.zeroize();
                message.zeroize();
            }
            _ => {}
        }
    }
}

impl Drop for PaymentError {
    fn drop(&mut self) {
        self.zeroize_primary_buffers();
        self.zeroize_coded_buffers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_debug_redact_secret_payloads() {
        let canaries = [
            "sk_live_CANARY_SECRET_999",
            "whsec_CANARY_WEBHOOK_888",
            "egypt_sk_CANARY_PAYMOB_777",
            "4111111111111111",
        ];
        let errors = [
            PaymentError::ProviderError {
                provider: ProviderId::new("stripe"),
                http_status: Some(500),
                vendor_code: Some("api_error".into()),
                message: canaries[0].into(),
            },
            PaymentError::WebhookVerificationFailed {
                provider: ProviderId::new("stripe"),
                reason: canaries[1].into(),
            },
            PaymentError::AuthenticationError {
                provider: ProviderId::new("paymob"),
                message: canaries[2].into(),
            },
            PaymentError::CardDeclined {
                provider: ProviderId::new("moyasar"),
                decline_code: Some("insufficient_funds".into()),
                advice_code: None,
                message: canaries[3].into(),
            },
            PaymentError::ThreeDsFailed {
                provider: ProviderId::new("moyasar"),
                code: Some("3ds_auth_error".into()),
                message: canaries[0].into(),
            },
            PaymentError::ValidationError {
                message: canaries[0].into(),
            },
            PaymentError::ConfigurationError {
                message: canaries[1].into(),
            },
            PaymentError::ProviderUnavailable {
                provider: ProviderId::new("paymob"),
                http_status: Some(503),
                indeterminate: true,
                message: canaries[2].into(),
            },
            PaymentError::AbortedByHook {
                reason: canaries[3].into(),
            },
        ];

        for err in &errors {
            let display = format!("{err}");
            let debug = format!("{err:?}");
            for canary in &canaries {
                assert!(!display.contains(canary));
                assert!(!debug.contains(canary));
            }
            assert!(debug.contains("[REDACTED]"));
        }
    }

    #[test]
    fn debug_includes_non_secret_diagnostic_fields() {
        let provider_err = PaymentError::ProviderError {
            provider: ProviderId::new("stripe"),
            http_status: Some(400),
            vendor_code: Some("invalid_request".into()),
            message: "secret_canary".into(),
        };
        let dbg = format!("{provider_err:?}");
        assert!(dbg.contains("stripe"));
        assert!(dbg.contains("invalid_request"));
        assert!(!dbg.contains("secret_canary"));
        assert_eq!(provider_err.vendor_code(), Some("invalid_request"));

        let rl = PaymentError::RateLimited {
            provider: ProviderId::new("moyasar"),
            retry_after_secs: Some(42),
        };
        let rl_dbg = format!("{rl:?}");
        assert!(rl_dbg.contains("moyasar"));
        assert!(rl_dbg.contains("42"));
        assert_eq!(rl.retry_after_secs(), Some(42));
        assert!(!rl.is_indeterminate());

        let unavail = PaymentError::ProviderUnavailable {
            provider: ProviderId::new("paymob"),
            http_status: Some(504),
            indeterminate: true,
            message: "timeout".into(),
        };
        let u_dbg = format!("{unavail:?}");
        assert!(u_dbg.contains("paymob"));
        assert!(u_dbg.contains("indeterminate: true"));
        assert!(unavail.is_indeterminate());

        let unsup = PaymentError::unsupported(&ProviderId::new("tap"), "splits");
        let unsup_dbg = format!("{unsup:?}");
        assert!(unsup_dbg.contains("tap"));
        assert!(unsup_dbg.contains("splits"));
    }

    #[test]
    fn all_variants_classification_and_accessors() {
        let val = PaymentError::validation("bad input");
        assert_eq!(val.kind(), ErrorKind::Validation);
        assert!(val.is_definitive_client_rejection());
        assert!(!val.is_retryable());
        assert_eq!(val.decline_code(), None);
        assert_eq!(val.vendor_code(), None);
        assert_eq!(val.retry_after_secs(), None);
        assert!(!val.is_indeterminate());
        assert_eq!(val.provider(), None);

        let cfg = PaymentError::configuration("missing key");
        assert_eq!(cfg.kind(), ErrorKind::Configuration);
        assert!(cfg.is_definitive_client_rejection());

        let unsup = PaymentError::unsupported(&ProviderId::new("paymob"), "splits");
        assert_eq!(unsup.kind(), ErrorKind::Unsupported);
        assert!(unsup.is_definitive_client_rejection());
        assert!(format!("{unsup}").contains("provider=paymob"));
        assert!(format!("{unsup}").contains("operation=splits"));

        let declined = PaymentError::CardDeclined {
            provider: ProviderId::new("moyasar"),
            decline_code: Some("insufficient_funds".into()),
            advice_code: Some("retry_later".into()),
            message: "declined".into(),
        };
        assert_eq!(declined.kind(), ErrorKind::CardDeclined);
        assert_eq!(declined.decline_code(), Some("insufficient_funds"));
        assert!(!declined.is_retryable());
        assert!(!declined.is_definitive_client_rejection());

        let auth = PaymentError::AuthenticationError {
            provider: ProviderId::new("stripe"),
            message: "bad key".into(),
        };
        assert_eq!(auth.kind(), ErrorKind::Authentication);
        assert_eq!(auth.http_status(), Some(401));
        assert!(auth.is_definitive_client_rejection());

        let unavail = PaymentError::ProviderUnavailable {
            provider: ProviderId::new("paymob"),
            http_status: Some(503),
            indeterminate: true,
            message: "upstream down".into(),
        };
        assert_eq!(unavail.kind(), ErrorKind::ProviderUnavailable);
        assert_eq!(unavail.http_status(), Some(503));
        assert!(unavail.is_retryable());
        assert!(unavail.is_indeterminate());
        assert!(!unavail.is_definitive_client_rejection());

        let wh = PaymentError::WebhookVerificationFailed {
            provider: ProviderId::new("stripe"),
            reason: "bad sig".into(),
        };
        assert_eq!(wh.kind(), ErrorKind::WebhookVerificationFailed);
        assert!(!wh.is_retryable());

        let conflict = PaymentError::IdempotencyConflict {
            scope: "refund:1".into(),
        };
        assert_eq!(conflict.kind(), ErrorKind::IdempotencyConflict);
        assert!(conflict.is_retryable());
        assert!(!conflict.is_definitive_client_rejection());

        let aborted = PaymentError::AbortedByHook {
            reason: "blocked".into(),
        };
        assert_eq!(aborted.kind(), ErrorKind::AbortedByHook);
        assert!(aborted.is_definitive_client_rejection());
    }

    #[test]
    fn definitive_client_rejection_http_status_boundaries() {
        for status in [400, 404, 422, 431, 451] {
            let err = PaymentError::ProviderError {
                provider: ProviderId::new("stripe"),
                http_status: Some(status),
                vendor_code: None,
                message: "client error".into(),
            };
            assert!(
                err.is_definitive_client_rejection(),
                "expected definitive rejection for {status}"
            );
        }

        for status in [408, 409, 425, 429, 499, 500, 503] {
            let err = PaymentError::ProviderError {
                provider: ProviderId::new("stripe"),
                http_status: Some(status),
                vendor_code: None,
                message: "non-definitive".into(),
            };
            assert!(
                !err.is_definitive_client_rejection(),
                "expected lock retention for {status}"
            );
        }
    }

    #[test]
    fn retryability_and_3ds_classification() {
        let three_ds = PaymentError::ThreeDsFailed {
            provider: ProviderId::new("moyasar"),
            code: Some("3ds_auth_error".into()),
            message: "3DS challenge failed".into(),
        };
        assert_eq!(three_ds.kind(), ErrorKind::ThreeDsFailed);
        assert_eq!(three_ds.vendor_code(), Some("3ds_auth_error"));
        assert!(!three_ds.is_retryable());
        assert!(!three_ds.is_definitive_client_rejection());

        let rl = PaymentError::RateLimited {
            provider: ProviderId::new("stripe"),
            retry_after_secs: Some(5),
        };
        assert!(rl.is_retryable());
        assert!(!rl.is_definitive_client_rejection());
        assert_eq!(rl.http_status(), Some(429));
        assert_eq!(rl.retry_after_secs(), Some(5));
    }

    #[test]
    fn error_kind_all_twelve_variants_lookup_summary_and_serde() {
        let expected = [
            ("validation", ErrorKind::Validation, "validation error"),
            (
                "configuration",
                ErrorKind::Configuration,
                "configuration error",
            ),
            (
                "unsupported",
                ErrorKind::Unsupported,
                "unsupported operation",
            ),
            ("card_declined", ErrorKind::CardDeclined, "payment declined"),
            (
                "three_ds_failed",
                ErrorKind::ThreeDsFailed,
                "3DS authentication failed",
            ),
            (
                "authentication",
                ErrorKind::Authentication,
                "merchant authentication failed",
            ),
            ("rate_limited", ErrorKind::RateLimited, "rate limited"),
            (
                "provider_unavailable",
                ErrorKind::ProviderUnavailable,
                "provider unavailable",
            ),
            (
                "webhook_verification_failed",
                ErrorKind::WebhookVerificationFailed,
                "webhook verification failed",
            ),
            (
                "idempotency_conflict",
                ErrorKind::IdempotencyConflict,
                "idempotency conflict",
            ),
            (
                "aborted_by_hook",
                ErrorKind::AbortedByHook,
                "aborted by lifecycle hook",
            ),
            ("provider", ErrorKind::Provider, "provider error"),
        ];
        assert_eq!(expected.len(), ERROR_KIND_META.len());
        for (slug, kind, summary) in expected {
            assert_eq!(ErrorKind::from_slug(slug), Some(kind));
            assert_eq!(kind.summary(), summary);
            let json = serde_json::to_string(&kind).unwrap();
            let decoded: ErrorKind = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, kind);
        }
        assert_eq!(ErrorKind::from_slug("unknown"), None);
    }

    #[test]
    fn rejects_lock_release_on_408_and_provider_unavailable_4xx() {
        let unavail_408 = PaymentError::ProviderUnavailable {
            provider: ProviderId::new("stripe"),
            http_status: Some(408),
            indeterminate: true,
            message: "request timed out".into(),
        };
        assert!(!unavail_408.is_definitive_client_rejection());

        let unavail_400 = PaymentError::ProviderUnavailable {
            provider: ProviderId::new("paymob"),
            http_status: Some(400),
            indeterminate: false,
            message: "ambiguous gateway outage".into(),
        };
        assert!(!unavail_400.is_definitive_client_rejection());

        let provider_408 = PaymentError::ProviderError {
            provider: ProviderId::new("moyasar"),
            http_status: Some(408),
            vendor_code: Some("timeout".into()),
            message: "request timeout".into(),
        };
        assert!(!provider_408.is_definitive_client_rejection());
    }

    #[test]
    fn zeroize_buffers_scrubs_sensitive_error_strings_and_clone_works() {
        let mut declined = PaymentError::CardDeclined {
            provider: ProviderId::new("stripe"),
            decline_code: Some("insufficient_funds".into()),
            advice_code: Some("try_again_later".into()),
            message: "secret_canary_decline".into(),
        };
        let cloned = declined.clone();
        assert_eq!(cloned.decline_code(), Some("insufficient_funds"));

        declined.zeroize_primary_buffers();
        declined.zeroize_coded_buffers();
        if let PaymentError::CardDeclined {
            decline_code,
            advice_code,
            message,
            ..
        } = &declined
        {
            assert!(decline_code.as_deref().is_none_or(str::is_empty));
            assert!(advice_code.as_deref().is_none_or(str::is_empty));
            assert!(message.is_empty());
        } else {
            panic!("expected CardDeclined");
        }
    }
}
