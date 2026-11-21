//! Typed, secret-redacted error taxonomy for `some-payment-core`.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

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

const KNOWN_ERROR_KINDS: &[(&str, ErrorKind)] = &[
    ("validation", ErrorKind::Validation),
    ("configuration", ErrorKind::Configuration),
    ("unsupported", ErrorKind::Unsupported),
    ("card_declined", ErrorKind::CardDeclined),
    ("three_ds_failed", ErrorKind::ThreeDsFailed),
    ("authentication", ErrorKind::Authentication),
    ("rate_limited", ErrorKind::RateLimited),
    ("provider_unavailable", ErrorKind::ProviderUnavailable),
    ("webhook_verification_failed", ErrorKind::WebhookVerificationFailed),
    ("idempotency_conflict", ErrorKind::IdempotencyConflict),
    ("aborted_by_hook", ErrorKind::AbortedByHook),
    ("provider", ErrorKind::Provider),
];

impl ErrorKind {
    /// Parses a snake_case error kind string using a static lookup table.
    pub fn from_slug(slug: &str) -> Option<Self> {
        let normalized = slug.trim().to_ascii_lowercase();
        KNOWN_ERROR_KINDS
            .iter()
            .find(|(k, _)| *k == normalized)
            .map(|(_, v)| *v)
    }
}

/// Canonical error returned by [`crate::PaymentProvider`] and orchestrator operations.
///
/// Implements custom [`fmt::Debug`] formatting that redacts any attached
/// provider diagnostic metadata so secret keys or cardholder data never leak
/// into structured logs or panic traces.
///
/// # Examples
///
/// ```
/// use some_payment_core::{PaymentError, ProviderId};
///
/// let err = PaymentError::unsupported(&ProviderId::new("paymob"), "splits");
/// assert!(!err.is_retryable());
/// ```
#[derive(Clone, Error)]
pub enum PaymentError {
    /// Input failed local validation prior to network I/O.
    #[error("validation error: {message}")]
    ValidationError {
        /// Human-readable validation message (free of secrets/PII).
        message: String,
    },

    /// Missing or invalid provider/orchestrator configuration.
    #[error("configuration error: {message}")]
    ConfigurationError {
        /// Human-readable configuration error.
        message: String,
    },

    /// Provider does not claim the capability required for an operation.
    #[error("provider '{provider}' does not support operation '{operation}'")]
    Unsupported {
        /// Target provider slug.
        provider: ProviderId,
        /// Operation or capability name.
        operation: String,
    },

    /// Customer card or wallet payment was declined.
    #[error("payment declined by '{provider}': {message}")]
    CardDeclined {
        /// Target provider slug.
        provider: ProviderId,
        /// Vendor decline code (e.g., `insufficient_funds`).
        decline_code: Option<String>,
        /// Vendor advice code if available.
        advice_code: Option<String>,
        /// Redacted human-readable message.
        message: String,
    },

    /// Customer failed 3-D Secure authentication (distinct from merchant auth).
    #[error("3DS authentication failed on '{provider}': {message}")]
    ThreeDsFailed {
        /// Target provider slug.
        provider: ProviderId,
        /// Vendor error code (e.g., `3ds_auth_error`).
        code: Option<String>,
        /// Redacted human-readable message.
        message: String,
    },

    /// Merchant credentials were rejected (HTTP 401) or wrong key scope.
    #[error("merchant authentication failed on '{provider}': {message}")]
    AuthenticationError {
        /// Target provider slug.
        provider: ProviderId,
        /// Redacted human-readable message.
        message: String,
    },

    /// Upstream gateway rate-limited the request (HTTP 429).
    #[error("rate limited by '{provider}'")]
    RateLimited {
        /// Target provider slug.
        provider: ProviderId,
        /// Optional `Retry-After` hint in seconds.
        retry_after_secs: Option<u64>,
    },

    /// Upstream gateway is temporarily unavailable or timed out.
    #[error("provider '{provider}' unavailable: {message}")]
    ProviderUnavailable {
        /// Target provider slug.
        provider: ProviderId,
        /// Optional HTTP status code.
        http_status: Option<u16>,
        /// Whether the request may have been processed (e.g. network timeout).
        indeterminate: bool,
        /// Redacted human-readable message.
        message: String,
    },

    /// Webhook signature or secret-token verification failed.
    #[error("webhook verification failed for '{provider}': {reason}")]
    WebhookVerificationFailed {
        /// Target provider slug.
        provider: ProviderId,
        /// Non-sensitive failure reason.
        reason: String,
    },

    /// Another worker currently holds the idempotency lock for this key.
    #[error("idempotency conflict on scope '{scope}'")]
    IdempotencyConflict {
        /// Namespaced idempotency scope.
        scope: String,
    },

    /// A `before` lifecycle hook aborted execution.
    #[error("aborted by lifecycle hook: {reason}")]
    AbortedByHook {
        /// Reason returned by the hook.
        reason: String,
    },

    /// Structured provider error preserving HTTP status and vendor code.
    #[error("provider '{provider}' error (status={http_status:?}): {message}")]
    ProviderError {
        /// Target provider slug.
        provider: ProviderId,
        /// HTTP status code if available.
        http_status: Option<u16>,
        /// Vendor-specific error code.
        vendor_code: Option<String>,
        /// Redacted message.
        message: String,
        /// Optional redacted diagnostic details (never printed in `Debug`).
        redacted_context: Option<String>,
    },
}

impl fmt::Debug for PaymentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentError")
            .field("kind", &self.kind())
            .field("http_status", &self.http_status())
            .field("retryable", &self.is_retryable())
            .field("decline_code", &self.decline_code())
            .field("message", &"[REDACTED]")
            .finish()
    }
}

impl PaymentError {
    /// Constructs a [`PaymentError::ValidationError`].
    pub fn validation(message: impl Into<String>) -> Self {
        Self::ValidationError {
            message: message.into(),
        }
    }

    /// Constructs a [`PaymentError::ConfigurationError`].
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::ConfigurationError {
            message: message.into(),
        }
    }

    /// Constructs a [`PaymentError::Unsupported`] error.
    pub fn unsupported(provider: &ProviderId, operation: impl Into<String>) -> Self {
        Self::Unsupported {
            provider: provider.clone(),
            operation: operation.into(),
        }
    }

    /// Returns the coarse [`ErrorKind`] for telemetry and classification.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::ValidationError { .. } => ErrorKind::Validation,
            Self::ConfigurationError { .. } => ErrorKind::Configuration,
            Self::Unsupported { .. } => ErrorKind::Unsupported,
            Self::CardDeclined { .. } => ErrorKind::CardDeclined,
            Self::ThreeDsFailed { .. } => ErrorKind::ThreeDsFailed,
            Self::AuthenticationError { .. } => ErrorKind::Authentication,
            Self::RateLimited { .. } => ErrorKind::RateLimited,
            Self::ProviderUnavailable { .. } => ErrorKind::ProviderUnavailable,
            Self::WebhookVerificationFailed { .. } => ErrorKind::WebhookVerificationFailed,
            Self::IdempotencyConflict { .. } => ErrorKind::IdempotencyConflict,
            Self::AbortedByHook { .. } => ErrorKind::AbortedByHook,
            Self::ProviderError { .. } => ErrorKind::Provider,
        }
    }

    /// Returns `true` if the operation may safely be retried with the same
    /// idempotency key.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. }
                | Self::ProviderUnavailable { .. }
                | Self::IdempotencyConflict { .. }
        )
    }

    /// Returns `true` if this error represents a definitive local or 4xx client
    /// rejection where an idempotency lock may be safely released.
    pub fn is_definitive_client_rejection(&self) -> bool {
        matches!(
            self,
            Self::ValidationError { .. }
                | Self::ConfigurationError { .. }
                | Self::Unsupported { .. }
                | Self::AuthenticationError { .. }
                | Self::AbortedByHook { .. }
        ) || matches!(self.http_status(), Some(400..=499) if self.http_status() != Some(429))
    }

    /// Returns the upstream HTTP status code if available.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::RateLimited { .. } => Some(429),
            Self::AuthenticationError { .. } => Some(401),
            Self::ProviderUnavailable { http_status, .. }
            | Self::ProviderError { http_status, .. } => *http_status,
            _ => None,
        }
    }

    /// Returns the vendor decline code if this is a [`PaymentError::CardDeclined`].
    pub fn decline_code(&self) -> Option<&str> {
        match self {
            Self::CardDeclined { decline_code, .. } => decline_code.as_deref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_secret_payloads() {
        let err = PaymentError::ProviderError {
            provider: ProviderId::new("stripe"),
            http_status: Some(500),
            vendor_code: Some("api_error".into()),
            message: "sk_live_CANARY_SECRET_999".into(),
            redacted_context: Some("whsec_CANARY_SECRET_999".into()),
        };
        let debug_str = format!("{err:?}");
        assert!(!debug_str.contains("CANARY_SECRET_999"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn retryability_and_3ds_classification() {
        let three_ds = PaymentError::ThreeDsFailed {
            provider: ProviderId::new("moyasar"),
            code: Some("3ds_auth_error".into()),
            message: "3DS challenge failed".into(),
        };
        assert_eq!(three_ds.kind(), ErrorKind::ThreeDsFailed);
        assert!(!three_ds.is_retryable());

        let rl = PaymentError::RateLimited {
            provider: ProviderId::new("stripe"),
            retry_after_secs: Some(5),
        };
        assert!(rl.is_retryable());
        assert!(!rl.is_definitive_client_rejection());
        assert_eq!(rl.http_status(), Some(429));
    }

    #[test]
    fn error_kind_static_lookup() {
        assert_eq!(
            ErrorKind::from_slug("three_ds_failed"),
            Some(ErrorKind::ThreeDsFailed)
        );
        assert_eq!(ErrorKind::from_slug("unknown"), None);
    }
}
