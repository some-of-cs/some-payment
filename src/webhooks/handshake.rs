//! [`VerificationHandshakeHandler`] trait and routing for Moyasar
//! `webhook_verification` events.
//!
//! Payment gateways such as Moyasar send a one-shot `webhook_verification`
//! callback containing a `webhook_id` and a 6-digit `verification_code` when
//! registering a new webhook endpoint. This module validates the challenge
//! payload and dispatches it to a merchant-supplied
//! [`VerificationHandshakeHandler`] without touching the order ledger or
//! leaking `verification_code` into logs or error messages.

use async_trait::async_trait;
use some_payment_core::{NormalizedWebhookEvent, PaymentError, ProviderId, VerificationChallenge};

/// Exact number of ASCII digits required for a webhook verification code.
pub const VERIFICATION_CODE_LEN: usize = 6;

/// Maximum allowed byte length for a webhook registration identifier.
pub const MAX_WEBHOOK_ID_LEN: usize = 255;

/// Asynchronous callback invoked when a verified webhook carries a provider
/// verification handshake challenge (such as Moyasar `webhook_verification`).
///
/// Implementations should persist or surface the 6-digit verification code out
/// of band so the webhook endpoint can be confirmed with the provider, while
/// keeping `challenge.verification_code` out of logs and telemetry.
///
/// # Examples
///
/// ```
/// use async_trait::async_trait;
/// use some_payment::webhooks::handshake::VerificationHandshakeHandler;
/// use some_payment_core::{PaymentError, ProviderId, VerificationChallenge};
///
/// struct NoopHandshakeHandler;
///
/// #[async_trait]
/// impl VerificationHandshakeHandler for NoopHandshakeHandler {
///     async fn handle_verification(
///         &self,
///         _provider_id: &ProviderId,
///         _challenge: &VerificationChallenge,
///     ) -> Result<(), PaymentError> {
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait VerificationHandshakeHandler: Send + Sync {
    /// Handles a validated provider webhook verification handshake challenge.
    ///
    /// # Errors
    ///
    /// Returns a [`PaymentError`] if the merchant application fails to process
    /// or persist the verification handshake.
    async fn handle_verification(
        &self,
        provider_id: &ProviderId,
        challenge: &VerificationChallenge,
    ) -> Result<(), PaymentError>;
}

/// Outcome produced by [`route_verification_handshake`].
///
/// Intentionally omits the sensitive `verification_code` so the outcome can be
/// safely logged, serialized, or returned in HTTP handler diagnostics.
///
/// # Examples
///
/// ```
/// use some_payment::webhooks::handshake::HandshakeRouteOutcome;
/// use some_payment_core::ProviderId;
///
/// let handled = HandshakeRouteOutcome::Handled {
///     provider_id: ProviderId::new("moyasar"),
///     webhook_id: "wh_moyasar_01".into(),
/// };
/// assert!(matches!(handled, HandshakeRouteOutcome::Handled { .. }));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandshakeRouteOutcome {
    /// The verified webhook event did not carry a verification challenge and
    /// should continue to normal deduplication and fulfillment processing.
    NotAHandshake,
    /// The verification challenge was validated and dispatched to the
    /// [`VerificationHandshakeHandler`]; callers should respond with HTTP
    /// `200 OK` without mutating the order ledger.
    Handled {
        /// Normalized identifier of the provider that originated the handshake.
        provider_id: ProviderId,
        /// Provider webhook registration identifier from the challenge.
        webhook_id: String,
    },
}

/// Validates a [`VerificationChallenge`] prior to invoking a handshake handler.
///
/// Enforces two strict invariants:
/// - `webhook_id` is 1..=[`MAX_WEBHOOK_ID_LEN`] printable ASCII bytes with no
///   whitespace or control characters (equivalent to non-empty trimmed
///   graphic ASCII).
/// - `verification_code` is strictly [`VERIFICATION_CODE_LEN`] (6) ASCII
///   digits (`'0'..='9'`).
///
/// Neither `verification_code` nor any sensitive input is interpolated into
/// returned error messages.
///
/// # Errors
///
/// Returns [`PaymentError::ValidationError`] if `webhook_id` or
/// `verification_code` violates the invariants above.
///
/// # Examples
///
/// ```
/// use some_payment::webhooks::handshake::validate_verification_challenge;
/// use some_payment_core::VerificationChallenge;
///
/// let valid = VerificationChallenge {
///     webhook_id: "wh_moyasar_01".into(),
///     verification_code: "482910".into(),
/// };
/// assert!(validate_verification_challenge(&valid).is_ok());
///
/// let invalid = VerificationChallenge {
///     webhook_id: "wh_moyasar_01".into(),
///     verification_code: "12345".into(),
/// };
/// assert!(validate_verification_challenge(&invalid).is_err());
/// ```
pub fn validate_verification_challenge(
    challenge: &VerificationChallenge,
) -> Result<(), PaymentError> {
    validate_webhook_id(&challenge.webhook_id)?;
    validate_verification_code(&challenge.verification_code)?;
    Ok(())
}

/// Routes a verified [`NormalizedWebhookEvent`] through the verification
/// handshake handler when [`NormalizedWebhookEvent::verification_challenge`]
/// is populated.
///
/// - When `event.verification_challenge` is `None`, returns
///   [`HandshakeRouteOutcome::NotAHandshake`] without invoking `handler`.
/// - When `Some(challenge)`, validates the challenge via
///   [`validate_verification_challenge`], invokes
///   [`VerificationHandshakeHandler::handle_verification`], emits a
///   non-secret `tracing::info!` log containing only `provider_id` and
///   `webhook_id`, and returns [`HandshakeRouteOutcome::Handled`].
///
/// # Errors
///
/// Returns [`PaymentError::ValidationError`] if the embedded challenge fails
/// validation (without invoking `handler`), or propagates any [`PaymentError`]
/// returned by `handler`.
///
/// # Examples
///
/// ```
/// use async_trait::async_trait;
/// use serde_json::json;
/// use some_payment::webhooks::handshake::{
///     HandshakeRouteOutcome, VerificationHandshakeHandler, route_verification_handshake,
/// };
/// use some_payment_core::{
///     NormalizedWebhookEvent, PaymentError, ProviderId, VerificationChallenge,
/// };
///
/// struct AcceptHandler;
///
/// #[async_trait]
/// impl VerificationHandshakeHandler for AcceptHandler {
///     async fn handle_verification(
///         &self,
///         _provider_id: &ProviderId,
///         _challenge: &VerificationChallenge,
///     ) -> Result<(), PaymentError> {
///         Ok(())
///     }
/// }
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), PaymentError> {
/// let event = NormalizedWebhookEvent {
///     provider_id: ProviderId::new("moyasar"),
///     event_id: "evt_verify_001".into(),
///     event_type: "webhook_verification".into(),
///     livemode: false,
///     requires_hydration: false,
///     reference: None,
///     status: None,
///     money: None,
///     verification_challenge: Some(VerificationChallenge {
///         webhook_id: "wh_moyasar_01".into(),
///         verification_code: "482910".into(),
///     }),
///     raw_redacted: json!({}),
/// };
///
/// let outcome = route_verification_handshake(&event, &AcceptHandler).await?;
/// assert_eq!(
///     outcome,
///     HandshakeRouteOutcome::Handled {
///         provider_id: ProviderId::new("moyasar"),
///         webhook_id: "wh_moyasar_01".into(),
///     }
/// );
/// # Ok(())
/// # }
/// ```
pub async fn route_verification_handshake(
    event: &NormalizedWebhookEvent,
    handler: &dyn VerificationHandshakeHandler,
) -> Result<HandshakeRouteOutcome, PaymentError> {
    let Some(challenge) = event.verification_challenge.as_ref() else {
        return Ok(HandshakeRouteOutcome::NotAHandshake);
    };

    validate_verification_challenge(challenge)?;
    handler.handle_verification(&event.provider_id, challenge).await?;

    tracing::info!(
        provider_id = %event.provider_id,
        webhook_id = %challenge.webhook_id,
        "webhook verification handshake handled"
    );

    Ok(HandshakeRouteOutcome::Handled {
        provider_id: event.provider_id.clone(),
        webhook_id: challenge.webhook_id.clone(),
    })
}

fn validate_webhook_id(webhook_id: &str) -> Result<(), PaymentError> {
    if webhook_id.is_empty() || webhook_id.len() > MAX_WEBHOOK_ID_LEN {
        return Err(PaymentError::validation(
            "webhook_id must be between 1 and 255 printable ASCII characters",
        ));
    }
    if !webhook_id.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(PaymentError::validation(
            "webhook_id must contain only printable ASCII characters without whitespace or control characters",
        ));
    }
    Ok(())
}

fn validate_verification_code(code: &str) -> Result<(), PaymentError> {
    if code.len() != VERIFICATION_CODE_LEN || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PaymentError::validation("verification_code must be exactly 6 ASCII digits"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use serde_json::json;
    use some_payment_core::{
        ErrorKind, NormalizedWebhookEvent, PaymentError, ProviderId, VerificationChallenge,
    };

    use super::*;

    struct RecordingHandler {
        calls: AtomicUsize,
        recorded: Mutex<Vec<(ProviderId, String, String)>>,
        fail_with: Option<PaymentError>,
    }

    impl RecordingHandler {
        fn accepting() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                recorded: Mutex::new(Vec::new()),
                fail_with: None,
            }
        }

        fn failing(err: PaymentError) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                recorded: Mutex::new(Vec::new()),
                fail_with: Some(err),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl VerificationHandshakeHandler for RecordingHandler {
        async fn handle_verification(
            &self,
            provider_id: &ProviderId,
            challenge: &VerificationChallenge,
        ) -> Result<(), PaymentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(err) = &self.fail_with {
                return Err(err.clone());
            }
            let mut guard = self.recorded.lock().expect("recording mutex poisoned");
            guard.push((
                provider_id.clone(),
                challenge.webhook_id.clone(),
                challenge.verification_code.clone(),
            ));
            Ok(())
        }
    }

    fn make_event(
        provider: &str,
        event_type: &str,
        challenge: Option<VerificationChallenge>,
    ) -> NormalizedWebhookEvent {
        NormalizedWebhookEvent {
            provider_id: ProviderId::new(provider),
            event_id: "evt_test_001".into(),
            event_type: event_type.into(),
            livemode: false,
            requires_hydration: false,
            reference: None,
            status: None,
            money: None,
            verification_challenge: challenge,
            raw_redacted: json!({}),
        }
    }

    fn make_challenge(webhook_id: &str, verification_code: &str) -> VerificationChallenge {
        VerificationChallenge {
            webhook_id: webhook_id.into(),
            verification_code: verification_code.into(),
        }
    }

    #[test]
    fn constants_match_contract() {
        assert_eq!(VERIFICATION_CODE_LEN, 6);
        assert_eq!(MAX_WEBHOOK_ID_LEN, 255);
    }

    #[tokio::test]
    async fn tc_3_2_3_routes_valid_moyasar_verification_challenge() {
        let handler = RecordingHandler::accepting();
        let event = make_event(
            "moyasar",
            "webhook_verification",
            Some(make_challenge("wh_moyasar_01", "482910")),
        );

        let result = route_verification_handshake(&event, &handler).await;
        let outcome = result.expect("valid handshake must succeed");

        assert_eq!(
            outcome,
            HandshakeRouteOutcome::Handled {
                provider_id: ProviderId::new("moyasar"),
                webhook_id: "wh_moyasar_01".into(),
            }
        );
        assert_eq!(handler.call_count(), 1);

        let recorded = handler.recorded.lock().expect("recording mutex poisoned");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, ProviderId::new("moyasar"));
        assert_eq!(recorded[0].1, "wh_moyasar_01");
        assert_eq!(recorded[0].2, "482910");

        let debug_outcome = format!("{outcome:?}");
        assert!(debug_outcome.contains("wh_moyasar_01"));
        assert!(!debug_outcome.contains("482910"));
    }

    #[tokio::test]
    async fn ac_3_non_handshake_event_returns_not_a_handshake() {
        let handler = RecordingHandler::accepting();
        let event = make_event("moyasar", "payment_paid", None);

        let result = route_verification_handshake(&event, &handler).await;
        let outcome = result.expect("non-handshake event must return Ok");

        assert_eq!(outcome, HandshakeRouteOutcome::NotAHandshake);
        assert_eq!(handler.call_count(), 0);
    }

    #[test]
    fn accepts_max_length_webhook_id_and_boundary_codes() {
        let max_id = "w".repeat(MAX_WEBHOOK_ID_LEN);
        assert!(validate_verification_challenge(&make_challenge(&max_id, "000000")).is_ok());
        assert!(validate_verification_challenge(&make_challenge("wh-1_2.3", "999999")).is_ok());
    }

    #[tokio::test]
    async fn rejects_invalid_webhook_ids_before_invoking_handler() {
        let oversized = "a".repeat(MAX_WEBHOOK_ID_LEN + 1);
        let invalid_ids = [
            "",
            "   ",
            " wh_01",
            "wh_01 ",
            "wh 01",
            "wh\t01",
            "wh\n01",
            "wh\x0001",
            "wh\x7f01",
            "wh_🚀",
            oversized.as_str(),
        ];

        let handler = RecordingHandler::accepting();
        for bad_id in invalid_ids {
            let event = make_event(
                "moyasar",
                "webhook_verification",
                Some(make_challenge(bad_id, "123456")),
            );
            let result = route_verification_handshake(&event, &handler).await;
            let err = result.expect_err("invalid webhook_id must fail validation");
            assert_eq!(err.kind(), ErrorKind::Validation);
        }
        assert_eq!(handler.call_count(), 0);
    }

    #[tokio::test]
    async fn rejects_invalid_verification_codes_without_leaking_secret() {
        let invalid_codes = [
            "",
            "12345",
            "1234567",
            "12345a",
            "abcdef",
            " 12345",
            "12345 ",
            "123 45",
            "١٢٣٤٥٦",
        ];

        let handler = RecordingHandler::accepting();
        for bad_code in invalid_codes {
            let event = make_event(
                "moyasar",
                "webhook_verification",
                Some(make_challenge("wh_moyasar_01", bad_code)),
            );
            let result = route_verification_handshake(&event, &handler).await;
            let err = result.expect_err("invalid verification_code must fail validation");
            assert_eq!(err.kind(), ErrorKind::Validation);
            if !bad_code.is_empty() {
                assert!(!format!("{err}").contains(bad_code));
                assert!(!format!("{err:?}").contains(bad_code));
            }
        }
        assert_eq!(handler.call_count(), 0);
    }

    #[tokio::test]
    async fn propagates_handler_errors() {
        let provider = ProviderId::new("moyasar");
        let handler = RecordingHandler::failing(PaymentError::ProviderUnavailable {
            provider: provider.clone(),
            http_status: Some(503),
            indeterminate: false,
            message: "handshake store offline".into(),
        });
        let event = make_event(
            "moyasar",
            "webhook_verification",
            Some(make_challenge("wh_moyasar_01", "654321")),
        );

        let result = route_verification_handshake(&event, &handler).await;
        let err = result.expect_err("handler error must propagate");

        assert_eq!(err.kind(), ErrorKind::ProviderUnavailable);
        assert_eq!(err.provider(), Some(&provider));
        assert_eq!(handler.call_count(), 1);
    }
}
