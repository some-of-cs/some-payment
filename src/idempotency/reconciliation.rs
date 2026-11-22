//! Background reconciliation helper checking [`PaymentProvider::get_payment`]
//! to resolve indeterminate idempotency locks.
//!
//! When a non-idempotent gateway mutation (`refund`, `capture`, or `void_payment`)
//! fails with an indeterminate 5xx response or network timeout, the idempotency
//! lock MUST remain held until authoritative state is fetched from the provider.
//! [`reconcile_indeterminate_mutation`] queries [`PaymentProvider::get_payment`]
//! and either completes the lock with a redacted [`PaymentResult`], releases the
//! lock when the mutation is confirmed absent, or retains the lock when the
//! payment is still in a transitional state.

use some_payment_core::{
    IdempotencyEntry, IdempotencyStore, NextAction, PaymentError, PaymentProvider, PaymentResult,
    PaymentStatus, ProviderReference,
};

/// Mutation kind being reconciled against authoritative provider state.
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::reconciliation::ReconciliationKind;
///
/// let refund = ReconciliationKind::Refund {
///     expected_min_refunded_amount: 500,
/// };
/// let capture = ReconciliationKind::Capture {
///     expected_min_captured_amount: 1_000,
/// };
/// let void = ReconciliationKind::Void;
///
/// assert_ne!(refund, capture);
/// assert_eq!(void, ReconciliationKind::Void);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReconciliationKind {
    /// Reconcile an indeterminate refund mutation.
    Refund {
        /// Minimum cumulative `refunded_amount` (in minor units) proving the refund executed.
        expected_min_refunded_amount: u64,
    },
    /// Reconcile an indeterminate capture mutation.
    Capture {
        /// Minimum cumulative `captured_amount` (in minor units) proving the capture executed.
        expected_min_captured_amount: u64,
    },
    /// Reconcile an indeterminate void/cancellation mutation.
    Void,
}

/// Input descriptor for reconciling an indeterminate idempotency lock.
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::reconciliation::{ReconciliationKind, ReconciliationRequest};
/// use some_payment_core::{IdempotencyEntry, ProviderId, ProviderReference};
///
/// let entry = IdempotencyEntry::new("moyasar:refund", "ord-42", 300);
/// let reference = ProviderReference::builder(ProviderId::new("moyasar"))
///     .payment_id("pay_42")
///     .build()
///     .unwrap();
/// let request = ReconciliationRequest::new(
///     entry,
///     &reference,
///     ReconciliationKind::Refund {
///         expected_min_refunded_amount: 500,
///     },
/// );
///
/// assert_eq!(request.entry.scope, "moyasar:refund");
/// assert_eq!(request.reference.payment_id(), Some("pay_42"));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconciliationRequest<'a> {
    /// Locked idempotency entry awaiting resolution.
    pub entry: IdempotencyEntry<'a>,
    /// Provider correlation reference used to query [`PaymentProvider::get_payment`].
    pub reference: &'a ProviderReference,
    /// Mutation kind and verification threshold to evaluate.
    pub kind: ReconciliationKind,
}

impl<'a> ReconciliationRequest<'a> {
    /// Creates a new [`ReconciliationRequest`].
    pub const fn new(
        entry: IdempotencyEntry<'a>,
        reference: &'a ProviderReference,
        kind: ReconciliationKind,
    ) -> Self {
        Self {
            entry,
            reference,
            kind,
        }
    }
}

/// Resolution outcome returned by [`reconcile_indeterminate_mutation`].
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::reconciliation::ReconciliationOutcome;
/// use some_payment_core::{Money, PaymentResult, PaymentStatus, ProviderId, ProviderReference};
///
/// let reference = ProviderReference::builder(ProviderId::new("moyasar"))
///     .payment_id("pay_1")
///     .build()
///     .unwrap();
/// let payment = PaymentResult {
///     reference,
///     status: PaymentStatus::Voided,
///     money: Money::new(1_000, "SAR").unwrap(),
///     captured_amount: None,
///     refunded_amount: None,
///     next_action: None,
///     checkout_url: None,
///     livemode: false,
/// };
/// let outcome = ReconciliationOutcome::Completed(payment);
/// assert!(matches!(outcome, ReconciliationOutcome::Completed(_)));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    /// Authoritative provider state confirmed the mutation executed; lock completed.
    Completed(PaymentResult),
    /// Authoritative provider state confirmed the mutation did not execute; lock released.
    Released(PaymentResult),
    /// Authoritative provider state is still transitional; lock retained for retry.
    StillIndeterminate(PaymentResult),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReconciliationDecision {
    Complete,
    Release,
    RetainLock,
}

/// Reconciles an indeterminate idempotency lock by fetching authoritative payment
/// state via [`PaymentProvider::get_payment`].
///
/// Depending on [`ReconciliationKind`] and the fetched [`PaymentResult`]:
///
/// - Calls [`IdempotencyStore::complete`] with a secret-redacted serialized
///   [`PaymentResult`] and returns [`ReconciliationOutcome::Completed`] when the
///   mutation is verified executed.
/// - Calls [`IdempotencyStore::release`] and returns
///   [`ReconciliationOutcome::Released`] when the gateway state confirms the
///   mutation did not execute.
/// - Retains the lock without touching `store` and returns
///   [`ReconciliationOutcome::StillIndeterminate`] when the payment remains in a
///   transitional state (e.g., [`PaymentStatus::Processing`],
///   [`PaymentStatus::Initiated`], or [`PaymentStatus::RequiresAction`]).
///
/// # Errors
///
/// Propagates any [`PaymentError`] returned by [`PaymentProvider::get_payment`],
/// [`IdempotencyStore::complete`], or [`IdempotencyStore::release`].
///
/// # Examples
///
/// ```
/// use async_trait::async_trait;
/// use some_payment::idempotency::reconciliation::{
///     ReconciliationKind, ReconciliationOutcome, ReconciliationRequest,
///     reconcile_indeterminate_mutation,
/// };
/// use some_payment_core::{
///     CreatePaymentRequest, IdempotencyEntry, IdempotencyStore, Money,
///     NormalizedWebhookEvent, PaymentCapabilities, PaymentError, PaymentProvider,
///     PaymentResult, PaymentStatus, ProviderId, ProviderReference, RawWebhookRequest,
///     ReserveOutcome,
/// };
///
/// #[derive(Debug)]
/// struct StubProvider(ProviderId, PaymentResult);
///
/// #[async_trait]
/// impl PaymentProvider for StubProvider {
///     fn id(&self) -> &ProviderId { &self.0 }
///     fn capabilities(&self) -> PaymentCapabilities { PaymentCapabilities::empty() }
///     async fn create_payment(&self, _: &CreatePaymentRequest) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "create_payment"))
///     }
///     async fn get_payment(&self, _: &ProviderReference) -> Result<PaymentResult, PaymentError> {
///         Ok(self.1.clone())
///     }
///     async fn parse_and_verify_webhook(&self, _: &RawWebhookRequest) -> Result<NormalizedWebhookEvent, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "parse_and_verify_webhook"))
///     }
/// }
///
/// struct NoopStore;
///
/// #[async_trait]
/// impl IdempotencyStore for NoopStore {
///     async fn reserve(&self, _: &IdempotencyEntry<'_>) -> Result<ReserveOutcome, PaymentError> {
///         Ok(ReserveOutcome::InProgress)
///     }
///     async fn complete(&self, _: &IdempotencyEntry<'_>, _: &serde_json::Value) -> Result<(), PaymentError> {
///         Ok(())
///     }
///     async fn release(&self, _: &str, _: &str) -> Result<(), PaymentError> {
///         Ok(())
///     }
/// }
///
/// let provider_id = ProviderId::new("moyasar");
/// let reference = ProviderReference::builder(provider_id.clone())
///     .payment_id("pay_doc_1")
///     .build()
///     .unwrap();
/// let payment = PaymentResult {
///     reference: reference.clone(),
///     status: PaymentStatus::PartiallyRefunded,
///     money: Money::new(10_000, "SAR").unwrap(),
///     captured_amount: Some(10_000),
///     refunded_amount: Some(2_500),
///     next_action: None,
///     checkout_url: None,
///     livemode: false,
/// };
/// let provider = StubProvider(provider_id, payment);
/// let store = NoopStore;
/// let entry = IdempotencyEntry::new("moyasar:refund", "ord-doc-1", 300);
/// let request = ReconciliationRequest::new(
///     entry,
///     &reference,
///     ReconciliationKind::Refund {
///         expected_min_refunded_amount: 2_500,
///     },
/// );
///
/// let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
/// let outcome = rt
///     .block_on(reconcile_indeterminate_mutation(&provider, &store, &request))
///     .unwrap();
/// assert!(matches!(outcome, ReconciliationOutcome::Completed(_)));
/// ```
pub async fn reconcile_indeterminate_mutation(
    provider: &dyn PaymentProvider,
    store: &dyn IdempotencyStore,
    request: &ReconciliationRequest<'_>,
) -> Result<ReconciliationOutcome, PaymentError> {
    let payment = provider.get_payment(request.reference).await?;
    let decision = evaluate_decision(&payment, request.kind);
    apply_decision(provider, store, request, payment, decision).await
}

async fn apply_decision(
    provider: &dyn PaymentProvider,
    store: &dyn IdempotencyStore,
    request: &ReconciliationRequest<'_>,
    payment: PaymentResult,
    decision: ReconciliationDecision,
) -> Result<ReconciliationOutcome, PaymentError> {
    match decision {
        ReconciliationDecision::Complete => {
            complete_reconciliation(provider, store, request, payment).await
        }
        ReconciliationDecision::Release => {
            release_reconciliation(provider, store, request, payment).await
        }
        ReconciliationDecision::RetainLock => {
            tracing::warn!(
                provider = provider.id().as_str(),
                scope = request.entry.scope,
                status = ?payment.status,
                "reconciliation observed transitional payment state; retaining idempotency lock"
            );
            Ok(ReconciliationOutcome::StillIndeterminate(payment))
        }
    }
}

async fn complete_reconciliation(
    provider: &dyn PaymentProvider,
    store: &dyn IdempotencyStore,
    request: &ReconciliationRequest<'_>,
    payment: PaymentResult,
) -> Result<ReconciliationOutcome, PaymentError> {
    let payload = serialize_cached_payment(&payment)?;
    store.complete(&request.entry, &payload).await?;
    tracing::info!(
        provider = provider.id().as_str(),
        scope = request.entry.scope,
        status = ?payment.status,
        "reconciliation verified mutation execution; idempotency lock completed"
    );
    Ok(ReconciliationOutcome::Completed(payment))
}

async fn release_reconciliation(
    provider: &dyn PaymentProvider,
    store: &dyn IdempotencyStore,
    request: &ReconciliationRequest<'_>,
    payment: PaymentResult,
) -> Result<ReconciliationOutcome, PaymentError> {
    store
        .release(request.entry.scope, request.entry.key)
        .await?;
    tracing::info!(
        provider = provider.id().as_str(),
        scope = request.entry.scope,
        status = ?payment.status,
        "reconciliation confirmed mutation did not execute; idempotency lock released"
    );
    Ok(ReconciliationOutcome::Released(payment))
}

fn evaluate_decision(payment: &PaymentResult, kind: ReconciliationKind) -> ReconciliationDecision {
    match kind {
        ReconciliationKind::Refund {
            expected_min_refunded_amount,
        } => evaluate_refund(payment, expected_min_refunded_amount),
        ReconciliationKind::Capture {
            expected_min_captured_amount,
        } => evaluate_capture(payment, expected_min_captured_amount),
        ReconciliationKind::Void => evaluate_void(payment),
    }
}

fn evaluate_refund(
    payment: &PaymentResult,
    expected_min_refunded_amount: u64,
) -> ReconciliationDecision {
    let refunded = payment.refunded_amount.unwrap_or(0);
    let verified = if expected_min_refunded_amount > 0 {
        refunded >= expected_min_refunded_amount
    } else {
        matches!(
            payment.status,
            PaymentStatus::Refunded | PaymentStatus::PartiallyRefunded
        )
    };

    if verified {
        return ReconciliationDecision::Complete;
    }

    if matches!(
        payment.status,
        PaymentStatus::Paid
            | PaymentStatus::Captured
            | PaymentStatus::Failed
            | PaymentStatus::Canceled
    ) {
        return ReconciliationDecision::Release;
    }

    ReconciliationDecision::RetainLock
}

fn evaluate_capture(
    payment: &PaymentResult,
    expected_min_captured_amount: u64,
) -> ReconciliationDecision {
    let captured = payment.captured_amount.unwrap_or(0);
    let captured_status = matches!(
        payment.status,
        PaymentStatus::Captured | PaymentStatus::PartiallyCaptured | PaymentStatus::Paid
    );

    if captured_status && captured >= expected_min_captured_amount {
        return ReconciliationDecision::Complete;
    }

    if matches!(
        payment.status,
        PaymentStatus::Authorized
            | PaymentStatus::Voided
            | PaymentStatus::Canceled
            | PaymentStatus::Failed
    ) {
        return ReconciliationDecision::Release;
    }

    ReconciliationDecision::RetainLock
}

fn evaluate_void(payment: &PaymentResult) -> ReconciliationDecision {
    if matches!(
        payment.status,
        PaymentStatus::Voided | PaymentStatus::Canceled
    ) {
        return ReconciliationDecision::Complete;
    }

    if matches!(
        payment.status,
        PaymentStatus::Authorized | PaymentStatus::Paid | PaymentStatus::Captured
    ) {
        return ReconciliationDecision::Release;
    }

    ReconciliationDecision::RetainLock
}

fn redact_for_cache(payment: &PaymentResult) -> PaymentResult {
    let mut sanitized = payment.clone();
    if matches!(
        sanitized.next_action,
        Some(NextAction::EmbeddedClientSecret { .. })
    ) {
        sanitized.next_action = Some(NextAction::EmbeddedClientSecret {
            client_secret: "[REDACTED]".to_owned(),
        });
    }
    sanitized
}

fn serialize_cached_payment(payment: &PaymentResult) -> Result<serde_json::Value, PaymentError> {
    let sanitized = redact_for_cache(payment);
    serde_json::to_value(&sanitized).map_err(|_| {
        PaymentError::configuration("failed to serialize reconciled PaymentResult payload")
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, Money, NormalizedWebhookEvent, PaymentCapabilities, ProviderId,
        RawWebhookRequest, ReserveOutcome,
    };

    use super::*;

    #[derive(Debug)]
    struct MockProvider {
        id: ProviderId,
        outcome: Result<PaymentResult, PaymentError>,
    }

    impl MockProvider {
        fn returning(payment: PaymentResult) -> Self {
            Self {
                id: payment.reference.provider_id().clone(),
                outcome: Ok(payment),
            }
        }

        fn failing(provider: &str, err: PaymentError) -> Self {
            Self {
                id: ProviderId::new(provider),
                outcome: Err(err),
            }
        }
    }

    #[async_trait]
    impl PaymentProvider for MockProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn capabilities(&self) -> PaymentCapabilities {
            PaymentCapabilities::empty()
        }

        async fn create_payment(
            &self,
            _req: &CreatePaymentRequest,
        ) -> Result<PaymentResult, PaymentError> {
            Err(PaymentError::unsupported(&self.id, "create_payment"))
        }

        async fn get_payment(
            &self,
            _reference: &ProviderReference,
        ) -> Result<PaymentResult, PaymentError> {
            self.outcome.clone()
        }

        async fn parse_and_verify_webhook(
            &self,
            _raw: &RawWebhookRequest,
        ) -> Result<NormalizedWebhookEvent, PaymentError> {
            Err(PaymentError::unsupported(
                &self.id,
                "parse_and_verify_webhook",
            ))
        }
    }

    #[derive(Default)]
    struct RecordingStore {
        complete_calls: AtomicUsize,
        release_calls: AtomicUsize,
        completed_payload: Mutex<Option<serde_json::Value>>,
        released_target: Mutex<Option<(String, String)>>,
    }

    #[async_trait]
    impl IdempotencyStore for RecordingStore {
        async fn reserve(
            &self,
            _entry: &IdempotencyEntry<'_>,
        ) -> Result<ReserveOutcome, PaymentError> {
            Ok(ReserveOutcome::InProgress)
        }

        async fn complete(
            &self,
            _entry: &IdempotencyEntry<'_>,
            payload: &serde_json::Value,
        ) -> Result<(), PaymentError> {
            self.complete_calls.fetch_add(1, Ordering::SeqCst);
            *self.completed_payload.lock().unwrap() = Some(payload.clone());
            Ok(())
        }

        async fn release(&self, scope: &str, key: &str) -> Result<(), PaymentError> {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            *self.released_target.lock().unwrap() = Some((scope.to_owned(), key.to_owned()));
            Ok(())
        }
    }

    fn sample_reference() -> ProviderReference {
        ProviderReference::builder(ProviderId::new("moyasar"))
            .payment_id("pay_314")
            .build()
            .unwrap()
    }

    fn sample_payment(
        status: PaymentStatus,
        captured_amount: Option<u64>,
        refunded_amount: Option<u64>,
    ) -> PaymentResult {
        PaymentResult {
            reference: sample_reference(),
            status,
            money: Money::new(10_000, "SAR").unwrap(),
            captured_amount,
            refunded_amount,
            next_action: None,
            checkout_url: None,
            livemode: false,
        }
    }

    #[tokio::test]
    async fn tc_3_1_4_refund_verified_by_refunded_amount_completes_lock() {
        let reference = sample_reference();
        let payment = sample_payment(PaymentStatus::PartiallyRefunded, Some(10_000), Some(5_000));
        let provider = MockProvider::returning(payment.clone());
        let store = RecordingStore::default();
        let entry = IdempotencyEntry::new("moyasar:refund", "ord-314", 300);
        let request = ReconciliationRequest::new(
            entry,
            &reference,
            ReconciliationKind::Refund {
                expected_min_refunded_amount: 5_000,
            },
        );

        let res = reconcile_indeterminate_mutation(&provider, &store, &request).await;
        assert_eq!(
            res.unwrap(),
            ReconciliationOutcome::Completed(payment.clone())
        );
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);

        let cached = store.completed_payload.lock().unwrap().clone().unwrap();
        let decoded: PaymentResult = serde_json::from_value(cached).unwrap();
        assert_eq!(decoded, payment);
    }

    #[tokio::test]
    async fn tc_3_1_4_refund_absent_on_paid_payment_releases_lock() {
        let reference = sample_reference();
        let payment = sample_payment(PaymentStatus::Paid, Some(10_000), Some(1_000));
        let provider = MockProvider::returning(payment.clone());
        let store = RecordingStore::default();
        let entry = IdempotencyEntry::new("moyasar:refund", "ord-315", 300);
        let request = ReconciliationRequest::new(
            entry,
            &reference,
            ReconciliationKind::Refund {
                expected_min_refunded_amount: 5_000,
            },
        );

        let res = reconcile_indeterminate_mutation(&provider, &store, &request).await;
        assert_eq!(res.unwrap(), ReconciliationOutcome::Released(payment));
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.released_target.lock().unwrap().as_ref(),
            Some(&("moyasar:refund".to_owned(), "ord-315".to_owned()))
        );
    }

    #[tokio::test]
    async fn tc_3_1_4_refund_on_processing_payment_retains_lock() {
        let reference = sample_reference();
        let payment = sample_payment(PaymentStatus::Processing, None, None);
        let provider = MockProvider::returning(payment.clone());
        let store = RecordingStore::default();
        let entry = IdempotencyEntry::new("moyasar:refund", "ord-316", 300);
        let request = ReconciliationRequest::new(
            entry,
            &reference,
            ReconciliationKind::Refund {
                expected_min_refunded_amount: 5_000,
            },
        );

        let res = reconcile_indeterminate_mutation(&provider, &store, &request).await;
        assert_eq!(
            res.unwrap(),
            ReconciliationOutcome::StillIndeterminate(payment)
        );
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn refund_zero_threshold_uses_refund_status_not_vacuous_amount() {
        let reference = sample_reference();
        let paid = sample_payment(PaymentStatus::Paid, Some(10_000), None);
        let store = RecordingStore::default();
        let entry = IdempotencyEntry::new("moyasar:refund", "ord-zero", 300);
        let request = ReconciliationRequest::new(
            entry,
            &reference,
            ReconciliationKind::Refund {
                expected_min_refunded_amount: 0,
            },
        );

        let provider_paid = MockProvider::returning(paid.clone());
        let released = reconcile_indeterminate_mutation(&provider_paid, &store, &request).await;
        assert_eq!(released.unwrap(), ReconciliationOutcome::Released(paid));
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 1);

        let refunded = sample_payment(PaymentStatus::Refunded, Some(10_000), None);
        let provider_refunded = MockProvider::returning(refunded.clone());
        let completed =
            reconcile_indeterminate_mutation(&provider_refunded, &store, &request).await;
        assert_eq!(
            completed.unwrap(),
            ReconciliationOutcome::Completed(refunded)
        );
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn capture_reconciliation_handles_completed_released_and_indeterminate() {
        let reference = sample_reference();
        let entry = IdempotencyEntry::new("paymob:capture", "ord-cap", 300);
        let request = ReconciliationRequest::new(
            entry,
            &reference,
            ReconciliationKind::Capture {
                expected_min_captured_amount: 10_000,
            },
        );

        let captured = sample_payment(PaymentStatus::Captured, Some(10_000), None);
        let store_complete = RecordingStore::default();
        let provider = MockProvider::returning(captured.clone());
        let res = reconcile_indeterminate_mutation(&provider, &store_complete, &request).await;
        assert_eq!(res.unwrap(), ReconciliationOutcome::Completed(captured));
        assert_eq!(store_complete.complete_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store_complete.release_calls.load(Ordering::SeqCst), 0);

        let authorized = sample_payment(PaymentStatus::Authorized, None, None);
        let store_release = RecordingStore::default();
        let provider = MockProvider::returning(authorized.clone());
        let res = reconcile_indeterminate_mutation(&provider, &store_release, &request).await;
        assert_eq!(res.unwrap(), ReconciliationOutcome::Released(authorized));
        assert_eq!(store_release.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store_release.release_calls.load(Ordering::SeqCst), 1);

        let processing = sample_payment(PaymentStatus::Processing, None, None);
        let store_retain = RecordingStore::default();
        let provider = MockProvider::returning(processing.clone());
        let res = reconcile_indeterminate_mutation(&provider, &store_retain, &request).await;
        assert_eq!(
            res.unwrap(),
            ReconciliationOutcome::StillIndeterminate(processing)
        );
        assert_eq!(store_retain.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store_retain.release_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn void_reconciliation_handles_completed_released_and_indeterminate() {
        let reference = sample_reference();
        let entry = IdempotencyEntry::new("paymob:void", "ord-void", 300);
        let request = ReconciliationRequest::new(entry, &reference, ReconciliationKind::Void);

        let voided = sample_payment(PaymentStatus::Voided, None, None);
        let store_complete = RecordingStore::default();
        let provider = MockProvider::returning(voided.clone());
        let res = reconcile_indeterminate_mutation(&provider, &store_complete, &request).await;
        assert_eq!(res.unwrap(), ReconciliationOutcome::Completed(voided));
        assert_eq!(store_complete.complete_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store_complete.release_calls.load(Ordering::SeqCst), 0);

        let authorized = sample_payment(PaymentStatus::Authorized, None, None);
        let store_release = RecordingStore::default();
        let provider = MockProvider::returning(authorized.clone());
        let res = reconcile_indeterminate_mutation(&provider, &store_release, &request).await;
        assert_eq!(res.unwrap(), ReconciliationOutcome::Released(authorized));
        assert_eq!(store_release.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store_release.release_calls.load(Ordering::SeqCst), 1);

        let requires_action = sample_payment(PaymentStatus::RequiresAction, None, None);
        let store_retain = RecordingStore::default();
        let provider = MockProvider::returning(requires_action.clone());
        let res = reconcile_indeterminate_mutation(&provider, &store_retain, &request).await;
        assert_eq!(
            res.unwrap(),
            ReconciliationOutcome::StillIndeterminate(requires_action)
        );
        assert_eq!(store_retain.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store_retain.release_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn reconciliation_redacts_embedded_client_secret_before_caching() {
        let reference = sample_reference();
        let mut payment = sample_payment(PaymentStatus::Refunded, Some(10_000), Some(10_000));
        payment.next_action = Some(NextAction::EmbeddedClientSecret {
            client_secret: "pi_secret_live_sensitive_token".to_owned(),
        });
        let provider = MockProvider::returning(payment);
        let store = RecordingStore::default();
        let entry = IdempotencyEntry::new("moyasar:refund", "ord-secret", 300);
        let request = ReconciliationRequest::new(
            entry,
            &reference,
            ReconciliationKind::Refund {
                expected_min_refunded_amount: 10_000,
            },
        );

        let res = reconcile_indeterminate_mutation(&provider, &store, &request).await;
        assert!(matches!(res.unwrap(), ReconciliationOutcome::Completed(_)));

        let cached_json = store
            .completed_payload
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .to_string();
        assert!(cached_json.contains("[REDACTED]"));
        assert!(!cached_json.contains("pi_secret_live_sensitive_token"));
    }

    #[tokio::test]
    async fn reconciliation_propagates_provider_error_without_touching_store() {
        let reference = sample_reference();
        let provider = MockProvider::failing(
            "moyasar",
            PaymentError::ProviderUnavailable {
                provider: ProviderId::new("moyasar"),
                http_status: Some(503),
                indeterminate: true,
                message: "upstream timeout".to_owned(),
            },
        );
        let store = RecordingStore::default();
        let entry = IdempotencyEntry::new("moyasar:refund", "ord-err", 300);
        let request = ReconciliationRequest::new(
            entry,
            &reference,
            ReconciliationKind::Refund {
                expected_min_refunded_amount: 1_000,
            },
        );

        let err = reconcile_indeterminate_mutation(&provider, &store, &request)
            .await
            .unwrap_err();
        assert_eq!(err.http_status(), Some(503));
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);
    }
}
