//! Background reconciliation helper checking [`PaymentProvider::get_payment`]
//! to resolve indeterminate idempotency locks.
//!
//! When a non-idempotent gateway mutation (`refund`, `capture`, or `void_payment`)
//! fails with an indeterminate 5xx response or network timeout, the idempotency
//! lock MUST remain held until authoritative state is fetched from the provider.
//! [`reconcile_indeterminate_mutation`] queries [`PaymentProvider::get_payment`]
//! and either completes the lock with a replay-compatible payload
//! ([`RefundResult`] for [`ReconciliationKind::Refund`], or redacted
//! [`PaymentResult`] for [`ReconciliationKind::Capture`] and
//! [`ReconciliationKind::Void`]), releases the lock when the mutation is
//! confirmed absent, or retains the lock when the payment is still in a
//! transitional state.

use std::fmt;

use some_payment_core::{
    IdempotencyEntry, IdempotencyStore, Money, NextAction, PaymentError, PaymentProvider,
    PaymentResult, PaymentStatus, ProviderReference, RefundResult, RefundStatus,
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
    ///
    /// # Single-In-Flight Assumption
    ///
    /// Cumulative `expected_min_refunded_amount` threshold matching assumes at
    /// most one refund mutation is in flight per payment at a time. Callers
    /// running concurrent partial refunds against the same payment should
    /// coordinate refund-level provider IDs externally.
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
/// Implements a manual [`fmt::Debug`] that redacts `entry.key` as
/// `"[REDACTED]"` so raw idempotency keys never leak into logs or traces.
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
/// assert!(format!("{request:?}").contains("[REDACTED]"));
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
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

impl fmt::Debug for ReconciliationRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReconciliationRequest")
            .field("scope", &self.entry.scope)
            .field("key", &"[REDACTED]")
            .field("ttl_secs", &self.entry.ttl_secs)
            .field("reference", &self.reference)
            .field("kind", &self.kind)
            .finish()
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

struct ReconciliationContext<'a> {
    provider: &'a dyn PaymentProvider,
    store: &'a dyn IdempotencyStore,
    request: &'a ReconciliationRequest<'a>,
}

/// Reconciles an indeterminate idempotency lock by fetching authoritative payment
/// state via [`PaymentProvider::get_payment`].
///
/// Depending on [`ReconciliationKind`] and the fetched [`PaymentResult`]:
///
/// - Calls [`IdempotencyStore::complete`] with a replay-compatible serialized
///   payload ([`RefundResult`] for [`ReconciliationKind::Refund`], or a
///   secret-redacted [`PaymentResult`] for [`ReconciliationKind::Capture`] and
///   [`ReconciliationKind::Void`]) and returns
///   [`ReconciliationOutcome::Completed`] when the mutation is verified
///   executed.
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
    let ctx = ReconciliationContext {
        provider,
        store,
        request,
    };
    apply_decision(&ctx, payment, decision).await
}

async fn apply_decision(
    ctx: &ReconciliationContext<'_>,
    payment: PaymentResult,
    decision: ReconciliationDecision,
) -> Result<ReconciliationOutcome, PaymentError> {
    let status = payment.status;
    let outcome = match decision {
        ReconciliationDecision::Complete => {
            let payload = serialize_reconciled_payload(ctx.request, &payment)?;
            ctx.store.complete(&ctx.request.entry, &payload).await?;
            ReconciliationOutcome::Completed(payment)
        }
        ReconciliationDecision::Release => {
            ctx.store
                .release(ctx.request.entry.scope, ctx.request.entry.key)
                .await?;
            ReconciliationOutcome::Released(payment)
        }
        ReconciliationDecision::RetainLock => ReconciliationOutcome::StillIndeterminate(payment),
    };
    log_decision(ctx, status, decision);
    Ok(outcome)
}

fn log_decision(
    ctx: &ReconciliationContext<'_>,
    status: PaymentStatus,
    decision: ReconciliationDecision,
) {
    let provider = ctx.provider.id().as_str();
    let scope = ctx.request.entry.scope;
    match decision {
        ReconciliationDecision::Complete => tracing::info!(
            provider,
            scope,
            ?status,
            "reconciliation verified mutation execution; idempotency lock completed"
        ),
        ReconciliationDecision::Release => tracing::info!(
            provider,
            scope,
            ?status,
            "reconciliation confirmed mutation did not execute; idempotency lock released"
        ),
        ReconciliationDecision::RetainLock => tracing::warn!(
            provider,
            scope,
            ?status,
            "reconciliation observed transitional payment state; retaining idempotency lock"
        ),
    }
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
        PaymentStatus::Captured
            | PaymentStatus::PartiallyCaptured
            | PaymentStatus::Paid
            | PaymentStatus::PartiallyRefunded
            | PaymentStatus::Refunded
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

fn reconciled_refund_money(
    payment: &PaymentResult,
    expected_min_refunded_amount: u64,
) -> Result<Money, PaymentError> {
    let amount = if expected_min_refunded_amount > 0 {
        expected_min_refunded_amount
    } else {
        payment
            .refunded_amount
            .filter(|&amt| amt > 0)
            .unwrap_or(payment.money.amount())
    };
    Money::new(amount, payment.money.currency())
}

fn build_reconciled_refund(
    payment: &PaymentResult,
    expected_min_refunded_amount: u64,
) -> Result<RefundResult, PaymentError> {
    let payment_id = payment.reference.payment_id().unwrap_or("unknown");
    Ok(RefundResult {
        refund_id: format!("reconciled:{payment_id}"),
        payment_reference: payment.reference.clone(),
        status: RefundStatus::Succeeded,
        money: reconciled_refund_money(payment, expected_min_refunded_amount)?,
        reason_detail: None,
    })
}

fn serialize_reconciled_payload(
    request: &ReconciliationRequest<'_>,
    payment: &PaymentResult,
) -> Result<serde_json::Value, PaymentError> {
    match request.kind {
        ReconciliationKind::Refund {
            expected_min_refunded_amount,
        } => {
            let refund = build_reconciled_refund(payment, expected_min_refunded_amount)?;
            serde_json::to_value(&refund).map_err(|_| {
                PaymentError::configuration("failed to serialize reconciled RefundResult payload")
            })
        }
        ReconciliationKind::Capture { .. } | ReconciliationKind::Void => {
            let sanitized = redact_for_cache(payment);
            serde_json::to_value(&sanitized).map_err(|_| {
                PaymentError::configuration("failed to serialize reconciled PaymentResult payload")
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, Money, NormalizedWebhookEvent, PaymentCapabilities, ProviderId,
        RawWebhookRequest, ReserveOutcome,
    };

    use super::*;
    use crate::idempotency::boundary::{IdempotencyBoundary, MutationContext};
    use crate::idempotency::memory_store::InMemoryIdempotencyStore;

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

    impl RecordingStore {
        fn counts(&self) -> (usize, usize) {
            (
                self.complete_calls.load(Ordering::SeqCst),
                self.release_calls.load(Ordering::SeqCst),
            )
        }
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

    async fn run_case(
        scope: &str,
        key: &str,
        kind: ReconciliationKind,
        payment: PaymentResult,
    ) -> (ReconciliationOutcome, RecordingStore) {
        let reference = payment.reference.clone();
        let provider = MockProvider::returning(payment);
        let store = RecordingStore::default();
        let entry = IdempotencyEntry::new(scope, key, 300);
        let request = ReconciliationRequest::new(entry, &reference, kind);
        let outcome = reconcile_indeterminate_mutation(&provider, &store, &request)
            .await
            .unwrap();
        (outcome, store)
    }

    #[test]
    fn reconciliation_request_debug_redacts_idempotency_key() {
        let reference = sample_reference();
        let entry = IdempotencyEntry::new("moyasar:refund", "secret-idemp-key-42", 300);
        let request = ReconciliationRequest::new(entry, &reference, ReconciliationKind::Void);
        let dbg = format!("{request:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("secret-idemp-key-42"));
    }

    #[tokio::test]
    async fn tc_3_1_4_refund_completed_caches_refund_result() {
        let kind = ReconciliationKind::Refund {
            expected_min_refunded_amount: 5_000,
        };
        let completed_pay =
            sample_payment(PaymentStatus::PartiallyRefunded, Some(10_000), Some(5_000));
        let (res, store) = run_case("moyasar:refund", "ord-314", kind, completed_pay.clone()).await;
        assert_eq!(
            (res, store.counts()),
            (ReconciliationOutcome::Completed(completed_pay), (1, 0))
        );
        let cached = store.completed_payload.lock().unwrap().clone().unwrap();
        let decoded: RefundResult = serde_json::from_value(cached).unwrap();
        assert_eq!(
            (decoded.status, decoded.money.amount()),
            (RefundStatus::Succeeded, 5_000)
        );
    }

    #[tokio::test]
    async fn tc_3_1_4_refund_released_and_processing_states() {
        let kind = ReconciliationKind::Refund {
            expected_min_refunded_amount: 5_000,
        };
        let paid = sample_payment(PaymentStatus::Paid, Some(10_000), Some(1_000));
        let (rel, rel_store) = run_case("moyasar:refund", "ord-315", kind, paid.clone()).await;
        assert_eq!(
            (rel, rel_store.counts()),
            (ReconciliationOutcome::Released(paid), (0, 1))
        );
        assert_eq!(
            rel_store.released_target.lock().unwrap().as_ref(),
            Some(&("moyasar:refund".to_owned(), "ord-315".to_owned()))
        );
        let proc = sample_payment(PaymentStatus::Processing, None, None);
        let (hold, hold_store) = run_case("moyasar:refund", "ord-316", kind, proc.clone()).await;
        assert_eq!(
            (hold, hold_store.counts()),
            (ReconciliationOutcome::StillIndeterminate(proc), (0, 0))
        );
    }

    #[tokio::test]
    async fn refund_reconciliation_round_trips_through_idempotency_boundary() {
        let store = Arc::new(InMemoryIdempotencyStore::new());
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let reference = sample_reference();
        let ctx = MutationContext::new("moyasar:refund", "ord-rt-1", PaymentCapabilities::REFUND);

        let err = boundary
            .execute_idempotent_mutation::<RefundResult, _, _>(&ctx, || async {
                Err(PaymentError::ProviderUnavailable {
                    provider: ProviderId::new("moyasar"),
                    http_status: Some(504),
                    indeterminate: true,
                    message: "timeout".to_owned(),
                })
            })
            .await
            .unwrap_err();
        assert_eq!(err.http_status(), Some(504));

        let payment = sample_payment(PaymentStatus::PartiallyRefunded, Some(10_000), Some(2_500));
        let provider = MockProvider::returning(payment);
        let request = ReconciliationRequest::new(
            IdempotencyEntry::new("moyasar:refund", "ord-rt-1", 300),
            &reference,
            ReconciliationKind::Refund {
                expected_min_refunded_amount: 2_500,
            },
        );
        let outcome = reconcile_indeterminate_mutation(&provider, store.as_ref(), &request)
            .await
            .unwrap();
        assert!(matches!(outcome, ReconciliationOutcome::Completed(_)));

        let replayed: RefundResult = boundary
            .execute_idempotent_mutation(&ctx, || async {
                panic!("provider must not be invoked on reconciled replay")
            })
            .await
            .unwrap();
        assert_eq!(
            (
                replayed.status,
                replayed.money.amount(),
                replayed.payment_reference
            ),
            (RefundStatus::Succeeded, 2_500, reference)
        );
    }

    #[tokio::test]
    async fn refund_zero_threshold_uses_refund_status_not_vacuous_amount() {
        let kind = ReconciliationKind::Refund {
            expected_min_refunded_amount: 0,
        };
        let paid = sample_payment(PaymentStatus::Paid, Some(10_000), None);
        let (released, rel_store) = run_case("moyasar:refund", "ord-z1", kind, paid.clone()).await;
        assert_eq!(
            (released, rel_store.counts()),
            (ReconciliationOutcome::Released(paid), (0, 1))
        );
        let refunded = sample_payment(PaymentStatus::Refunded, Some(10_000), None);
        let (done, done_store) = run_case("moyasar:refund", "ord-z2", kind, refunded.clone()).await;
        assert_eq!(
            (done, done_store.counts()),
            (ReconciliationOutcome::Completed(refunded), (1, 0))
        );
    }

    #[tokio::test]
    async fn capture_reconciliation_handles_completed_post_refund_and_released() {
        let kind = ReconciliationKind::Capture {
            expected_min_captured_amount: 10_000,
        };
        let captured = sample_payment(PaymentStatus::Captured, Some(10_000), None);
        let (res, store) = run_case("paymob:capture", "ord-cap", kind, captured.clone()).await;
        assert_eq!(
            (res, store.counts()),
            (ReconciliationOutcome::Completed(captured), (1, 0))
        );
        let later_refunded =
            sample_payment(PaymentStatus::PartiallyRefunded, Some(10_000), Some(3_000));
        let (ref_res, ref_store) = run_case(
            "paymob:capture",
            "ord-cap-ref",
            kind,
            later_refunded.clone(),
        )
        .await;
        assert_eq!(
            (ref_res, ref_store.counts()),
            (ReconciliationOutcome::Completed(later_refunded), (1, 0))
        );
        let authorized = sample_payment(PaymentStatus::Authorized, None, None);
        let (rel, rel_store) =
            run_case("paymob:capture", "ord-cap-auth", kind, authorized.clone()).await;
        assert_eq!(
            (rel, rel_store.counts()),
            (ReconciliationOutcome::Released(authorized), (0, 1))
        );
    }

    #[tokio::test]
    async fn capture_later_fully_refunded_and_processing_states() {
        let kind = ReconciliationKind::Capture {
            expected_min_captured_amount: 10_000,
        };
        let refunded = sample_payment(PaymentStatus::Refunded, Some(10_000), Some(10_000));
        let (done, done_store) =
            run_case("paymob:capture", "ord-cap-full", kind, refunded.clone()).await;
        assert_eq!(
            (done, done_store.counts()),
            (ReconciliationOutcome::Completed(refunded), (1, 0))
        );
        let processing = sample_payment(PaymentStatus::Processing, None, None);
        let (hold, hold_store) =
            run_case("paymob:capture", "ord-cap-proc", kind, processing.clone()).await;
        assert_eq!(
            (hold, hold_store.counts()),
            (
                ReconciliationOutcome::StillIndeterminate(processing),
                (0, 0)
            )
        );
    }

    #[tokio::test]
    async fn void_reconciliation_handles_completed_released_and_indeterminate() {
        let voided = sample_payment(PaymentStatus::Voided, None, None);
        let (done, done_store) = run_case(
            "paymob:void",
            "ord-v1",
            ReconciliationKind::Void,
            voided.clone(),
        )
        .await;
        assert_eq!(
            (done, done_store.counts()),
            (ReconciliationOutcome::Completed(voided), (1, 0))
        );
        let authorized = sample_payment(PaymentStatus::Authorized, None, None);
        let (rel, rel_store) = run_case(
            "paymob:void",
            "ord-v2",
            ReconciliationKind::Void,
            authorized.clone(),
        )
        .await;
        assert_eq!(
            (rel, rel_store.counts()),
            (ReconciliationOutcome::Released(authorized), (0, 1))
        );
        let req_action = sample_payment(PaymentStatus::RequiresAction, None, None);
        let (hold, hold_store) = run_case(
            "paymob:void",
            "ord-v3",
            ReconciliationKind::Void,
            req_action.clone(),
        )
        .await;
        assert_eq!(
            (hold, hold_store.counts()),
            (
                ReconciliationOutcome::StillIndeterminate(req_action),
                (0, 0)
            )
        );
    }

    #[tokio::test]
    async fn reconciliation_redacts_embedded_client_secret_before_caching() {
        let mut payment = sample_payment(PaymentStatus::Captured, Some(10_000), None);
        payment.next_action = Some(NextAction::EmbeddedClientSecret {
            client_secret: "pi_secret_live_sensitive_token".to_owned(),
        });
        let kind = ReconciliationKind::Capture {
            expected_min_captured_amount: 10_000,
        };
        let (res, store) = run_case("moyasar:capture", "ord-secret", kind, payment).await;
        assert!(matches!(res, ReconciliationOutcome::Completed(_)));

        let cached_json = store
            .completed_payload
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .to_string();
        assert!(
            cached_json.contains("[REDACTED]")
                && !cached_json.contains("pi_secret_live_sensitive_token")
        );
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
        assert_eq!((err.http_status(), store.counts()), (Some(503), (0, 0)));
    }
}
