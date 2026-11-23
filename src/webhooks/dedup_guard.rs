//! [`WebhookDeduplicationStore`] trait, [`InMemoryDedupStore`], monotonic
//! [`PaymentStatus`] transition guard, and [`Money`] equality verification.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use some_payment_core::{Money, PaymentError, PaymentStatus, ProviderId};

/// Default maximum number of active entries held by [`InMemoryDedupStore`].
pub const DEFAULT_DEDUP_CAPACITY: usize = 50_000;

/// Default time-to-live in seconds (24 hours) for deduplication entries.
pub const DEFAULT_DEDUP_TTL_SECS: u64 = 86_400;

const STATUS_RANKS: &[(PaymentStatus, u8)] = &[
    (PaymentStatus::Initiated, 0),
    (PaymentStatus::RequiresAction, 1),
    (PaymentStatus::Processing, 2),
    (PaymentStatus::Authorized, 3),
    (PaymentStatus::PartiallyCaptured, 4),
    (PaymentStatus::Captured, 5),
    (PaymentStatus::Paid, 6),
    (PaymentStatus::PartiallyRefunded, 7),
    (PaymentStatus::Refunded, 8),
    (PaymentStatus::Voided, 8),
    (PaymentStatus::Canceled, 8),
    (PaymentStatus::Failed, 8),
    (PaymentStatus::SetupCompleted, 8),
];

const TERMINAL_STATUSES: &[PaymentStatus] = &[
    PaymentStatus::Refunded,
    PaymentStatus::Voided,
    PaymentStatus::Canceled,
    PaymentStatus::Failed,
    PaymentStatus::SetupCompleted,
];

const NON_REFUND_TERMINAL_STATUSES: &[PaymentStatus] = &[
    PaymentStatus::Voided,
    PaymentStatus::Canceled,
    PaymentStatus::Failed,
    PaymentStatus::SetupCompleted,
];

type DedupKey = (ProviderId, String);

/// Pluggable store for deduplicating verified webhook events by
/// `(provider_id, event_id)`.
///
/// Production deployments should back this trait with Redis (`SET NX EX`) or a
/// PostgreSQL unique constraint on `(provider_id, event_id)`. For local
/// development and unit tests, use [`InMemoryDedupStore`].
///
/// # Examples
///
/// ```
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
/// use some_payment::webhooks::dedup_guard::{InMemoryDedupStore, WebhookDeduplicationStore};
/// use some_payment_core::ProviderId;
///
/// let store = InMemoryDedupStore::new();
/// let stripe = ProviderId::new("stripe");
/// assert!(store.record_if_new(&stripe, "evt_123").await.unwrap());
/// assert!(!store.record_if_new(&stripe, "evt_123").await.unwrap());
/// # });
/// ```
#[async_trait]
pub trait WebhookDeduplicationStore: Send + Sync {
    /// Atomically records `(provider_id, event_id)` if it has not been seen
    /// within the deduplication window.
    ///
    /// Returns `Ok(true)` on the first observation of `(provider_id, event_id)`
    /// and `Ok(false)` on duplicate deliveries.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] when `event_id` is empty or
    /// store limits are exceeded, or a backend storage error if persistence
    /// fails.
    async fn record_if_new(
        &self,
        provider_id: &ProviderId,
        event_id: &str,
    ) -> Result<bool, PaymentError>;
}

/// Thread-safe, bounded, TTL-expiring [`WebhookDeduplicationStore`] strictly
/// intended for local development and automated tests.
///
/// Implements a custom redacted [`fmt::Debug`] so recorded event identifiers
/// are never emitted into logs or panic traces.
///
/// # Examples
///
/// ```
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
/// use some_payment::webhooks::dedup_guard::{InMemoryDedupStore, WebhookDeduplicationStore};
/// use some_payment_core::ProviderId;
///
/// let store = InMemoryDedupStore::with_limits(100, 3_600).unwrap();
/// let moyasar = ProviderId::new("moyasar");
/// assert!(store.is_empty());
/// assert!(store.record_if_new(&moyasar, "evt_001").await.unwrap());
/// assert_eq!(store.len(), 1);
/// # });
/// ```
pub struct InMemoryDedupStore {
    entries: Mutex<HashMap<DedupKey, Instant>>,
    max_capacity: usize,
    ttl: Duration,
}

impl InMemoryDedupStore {
    /// Creates a new [`InMemoryDedupStore`] with [`DEFAULT_DEDUP_CAPACITY`]
    /// and [`DEFAULT_DEDUP_TTL_SECS`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_capacity: DEFAULT_DEDUP_CAPACITY,
            ttl: Duration::from_secs(DEFAULT_DEDUP_TTL_SECS),
        }
    }

    /// Creates a new [`InMemoryDedupStore`] with explicit capacity and TTL
    /// bounds.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ConfigurationError`] if `max_capacity` is `0` or
    /// `ttl_secs` is `0`.
    pub fn with_limits(max_capacity: usize, ttl_secs: u64) -> Result<Self, PaymentError> {
        if max_capacity == 0 {
            return Err(PaymentError::configuration(
                "webhook deduplication store max_capacity must be greater than zero",
            ));
        }
        if ttl_secs == 0 {
            return Err(PaymentError::configuration(
                "webhook deduplication store ttl_secs must be greater than zero",
            ));
        }
        Ok(Self {
            entries: Mutex::new(HashMap::new()),
            max_capacity,
            ttl: Duration::from_secs(ttl_secs),
        })
    }

    /// Returns the number of currently active (non-expired) deduplication
    /// entries.
    #[must_use]
    pub fn len(&self) -> usize {
        let mut guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        guard.retain(|_, expires_at| *expires_at > now);
        guard.len()
    }

    /// Returns `true` when there are no active (non-expired) deduplication
    /// entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock_entries(&self) -> Result<MutexGuard<'_, HashMap<DedupKey, Instant>>, PaymentError> {
        self.entries
            .lock()
            .map_err(|_| PaymentError::configuration("webhook deduplication store lock poisoned"))
    }
}

impl Default for InMemoryDedupStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemoryDedupStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryDedupStore")
            .field("max_capacity", &self.max_capacity)
            .field("ttl_secs", &self.ttl.as_secs())
            .field("entries", &"[REDACTED]")
            .finish()
    }
}

fn validate_event_id(event_id: &str) -> Result<&str, PaymentError> {
    let trimmed = event_id.trim();
    if trimmed.is_empty() {
        return Err(PaymentError::validation(
            "webhook event_id must not be empty",
        ));
    }
    Ok(trimmed)
}

fn compute_expiry(now: Instant, ttl: Duration) -> Result<Instant, PaymentError> {
    now.checked_add(ttl)
        .ok_or_else(|| PaymentError::configuration("webhook deduplication TTL overflow"))
}

#[async_trait]
impl WebhookDeduplicationStore for InMemoryDedupStore {
    async fn record_if_new(
        &self,
        provider_id: &ProviderId,
        event_id: &str,
    ) -> Result<bool, PaymentError> {
        let normalized_id = validate_event_id(event_id)?;
        let mut entries = self.lock_entries()?;
        let now = Instant::now();
        entries.retain(|_, expires_at| *expires_at > now);

        let key = (provider_id.clone(), normalized_id.to_owned());
        if entries.contains_key(&key) {
            return Ok(false);
        }
        if entries.len() >= self.max_capacity {
            return Err(PaymentError::validation(
                "webhook deduplication store capacity exceeded",
            ));
        }

        let expires_at = compute_expiry(now, self.ttl)?;
        entries.insert(key, expires_at);
        Ok(true)
    }
}

fn status_rank(status: PaymentStatus) -> u8 {
    STATUS_RANKS
        .iter()
        .find(|(candidate, _)| *candidate == status)
        .map_or(0, |(_, rank)| *rank)
}

fn is_blocked_transition(current: PaymentStatus, incoming: PaymentStatus) -> bool {
    let current_rank = status_rank(current);
    let incoming_rank = status_rank(incoming);
    incoming_rank <= current_rank
        || (current_rank >= 5 && NON_REFUND_TERMINAL_STATUSES.contains(&incoming))
}

/// Enforces monotonic [`PaymentStatus`] lifecycle progression for incoming
/// webhook events.
///
/// Same-state idempotent re-deliveries (`current == incoming`) are always
/// accepted. Regressions to earlier lifecycle states (such as `Authorized`
/// arriving after `Paid` or `Paid` arriving after `Refunded`) and transitions
/// out of terminal states (`Refunded`, `Voided`, `Canceled`, `Failed`,
/// `SetupCompleted`) are rejected.
///
/// # Errors
///
/// Returns [`PaymentError::ValidationError`] when the transition regresses or
/// violates terminal-state exclusivity.
///
/// # Examples
///
/// ```
/// use some_payment::webhooks::dedup_guard::validate_monotonic_transition;
/// use some_payment_core::PaymentStatus;
///
/// assert!(validate_monotonic_transition(PaymentStatus::Authorized, PaymentStatus::Paid).is_ok());
/// assert!(validate_monotonic_transition(PaymentStatus::Paid, PaymentStatus::Paid).is_ok());
/// assert!(validate_monotonic_transition(PaymentStatus::Paid, PaymentStatus::Authorized).is_err());
/// ```
pub fn validate_monotonic_transition(
    current: PaymentStatus,
    incoming: PaymentStatus,
) -> Result<(), PaymentError> {
    if current == incoming {
        return Ok(());
    }
    if TERMINAL_STATUSES.contains(&current) || is_blocked_transition(current, incoming) {
        return Err(PaymentError::validation(
            "invalid non-monotonic payment status transition",
        ));
    }
    Ok(())
}

/// Verifies exact integer minor-unit amount and ISO-4217 currency equality
/// between an expected order total and a webhook-reported [`Money`] value.
///
/// Performs strictly integer comparisons with zero floating-point arithmetic.
///
/// # Errors
///
/// Returns [`PaymentError::ValidationError`] if either the ISO-4217 currency
/// code or the exact minor-unit amount differs.
///
/// # Examples
///
/// ```
/// use some_payment::webhooks::dedup_guard::verify_order_money;
/// use some_payment_core::Money;
///
/// let expected = Money::new(15_000, "SAR").unwrap();
/// let actual = Money::new(15_000, "sar").unwrap();
/// let underpaid = Money::new(1_000, "SAR").unwrap();
///
/// assert!(verify_order_money(&expected, &actual).is_ok());
/// assert!(verify_order_money(&expected, &underpaid).is_err());
/// ```
pub fn verify_order_money(expected: &Money, actual: &Money) -> Result<(), PaymentError> {
    if expected.currency() != actual.currency() {
        return Err(PaymentError::validation(
            "webhook currency does not match expected order currency",
        ));
    }
    if expected.amount() != actual.amount() {
        return Err(PaymentError::validation(
            "webhook minor-unit amount does not match expected order amount",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use some_payment_core::{ErrorKind, Money, PaymentStatus, ProviderId};

    use super::{
        DEFAULT_DEDUP_CAPACITY, DEFAULT_DEDUP_TTL_SECS, InMemoryDedupStore,
        WebhookDeduplicationStore, validate_monotonic_transition, verify_order_money,
    };

    #[tokio::test]
    async fn ac2_dedup_records_first_and_rejects_duplicate() {
        let store = InMemoryDedupStore::default();
        let stripe = ProviderId::new("stripe");

        assert!(store.is_empty());
        assert!(store.record_if_new(&stripe, "evt_123").await.unwrap());
        assert!(!store.record_if_new(&stripe, "evt_123").await.unwrap());
        assert!(!store.record_if_new(&stripe, "  evt_123  ").await.unwrap());
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
    }

    #[tokio::test]
    async fn ac2_tm6_dedup_namespaces_by_provider_id() {
        let store = InMemoryDedupStore::new();
        let stripe = ProviderId::new("stripe");
        let moyasar = ProviderId::new("moyasar");

        assert!(store.record_if_new(&stripe, "evt_shared").await.unwrap());
        assert!(store.record_if_new(&moyasar, "evt_shared").await.unwrap());
        assert!(!store.record_if_new(&stripe, "evt_shared").await.unwrap());
        assert_eq!(store.len(), 2);
    }

    #[tokio::test]
    async fn dedup_rejects_empty_event_id() {
        let store = InMemoryDedupStore::new();
        let stripe = ProviderId::new("stripe");

        for invalid in ["", "   ", "\t\n"] {
            let err = store.record_if_new(&stripe, invalid).await.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation);
        }
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn dedup_enforces_capacity_limit() {
        let bounded = InMemoryDedupStore::with_limits(2, 60).unwrap();
        let stripe = ProviderId::new("stripe");

        assert!(bounded.record_if_new(&stripe, "evt_1").await.unwrap());
        assert!(bounded.record_if_new(&stripe, "evt_2").await.unwrap());
        let err = bounded.record_if_new(&stripe, "evt_3").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
    }

    #[tokio::test]
    async fn dedup_evicts_expired_entries() {
        let store = InMemoryDedupStore::with_limits(1, 60).unwrap();
        let stripe = ProviderId::new("stripe");
        let expired = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        store
            .entries
            .lock()
            .unwrap()
            .insert((stripe.clone(), "evt_a".to_owned()), expired);

        assert!(store.record_if_new(&stripe, "evt_a").await.unwrap());
    }

    #[test]
    fn with_limits_rejects_zero_capacity_and_zero_ttl() {
        let zero_cap = InMemoryDedupStore::with_limits(0, 60)
            .expect_err("zero max_capacity must be rejected");
        assert_eq!(zero_cap.kind(), ErrorKind::Configuration);

        let zero_ttl =
            InMemoryDedupStore::with_limits(10, 0).expect_err("zero ttl_secs must be rejected");
        assert_eq!(zero_ttl.kind(), ErrorKind::Configuration);
    }

    #[tokio::test]
    async fn dedup_debug_redacts_recorded_event_ids() {
        let store = InMemoryDedupStore::new();
        let stripe = ProviderId::new("stripe");
        assert!(
            store
                .record_if_new(&stripe, "evt_sensitive_999")
                .await
                .unwrap()
        );

        let dbg = format!("{store:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(dbg.contains(&DEFAULT_DEDUP_CAPACITY.to_string()));
        assert!(dbg.contains(&DEFAULT_DEDUP_TTL_SECS.to_string()));
        assert!(!dbg.contains("evt_sensitive_999"));
    }

    #[test]
    fn tc324_ac4_monotonic_allows_same_state_redelivery() {
        let all_statuses = [
            PaymentStatus::Initiated,
            PaymentStatus::RequiresAction,
            PaymentStatus::Processing,
            PaymentStatus::Authorized,
            PaymentStatus::PartiallyCaptured,
            PaymentStatus::Captured,
            PaymentStatus::Paid,
            PaymentStatus::PartiallyRefunded,
            PaymentStatus::Refunded,
            PaymentStatus::Voided,
            PaymentStatus::Canceled,
            PaymentStatus::Failed,
            PaymentStatus::SetupCompleted,
        ];

        for status in all_statuses {
            assert!(validate_monotonic_transition(status, status).is_ok());
        }
    }

    #[test]
    fn tc324_ac4_monotonic_allows_valid_forward_transitions() {
        let valid_pairs = [
            (PaymentStatus::Initiated, PaymentStatus::RequiresAction),
            (PaymentStatus::RequiresAction, PaymentStatus::Processing),
            (PaymentStatus::Processing, PaymentStatus::Authorized),
            (PaymentStatus::Authorized, PaymentStatus::PartiallyCaptured),
            (PaymentStatus::Authorized, PaymentStatus::Captured),
            (PaymentStatus::Authorized, PaymentStatus::Paid),
            (PaymentStatus::Authorized, PaymentStatus::Voided),
            (PaymentStatus::Authorized, PaymentStatus::Canceled),
            (PaymentStatus::Authorized, PaymentStatus::Failed),
            (PaymentStatus::PartiallyCaptured, PaymentStatus::Captured),
            (PaymentStatus::Captured, PaymentStatus::Paid),
            (PaymentStatus::Captured, PaymentStatus::PartiallyRefunded),
            (PaymentStatus::Captured, PaymentStatus::Refunded),
            (PaymentStatus::Paid, PaymentStatus::PartiallyRefunded),
            (PaymentStatus::Paid, PaymentStatus::Refunded),
            (PaymentStatus::PartiallyRefunded, PaymentStatus::Refunded),
            (PaymentStatus::Initiated, PaymentStatus::SetupCompleted),
        ];

        for (current, incoming) in valid_pairs {
            assert!(validate_monotonic_transition(current, incoming).is_ok());
        }
    }

    #[test]
    fn tc324_ac4_tm6_monotonic_rejects_regressions_and_terminal_conflicts() {
        let invalid_pairs = [
            (PaymentStatus::Paid, PaymentStatus::Authorized),
            (PaymentStatus::Refunded, PaymentStatus::Paid),
            (PaymentStatus::Voided, PaymentStatus::Initiated),
            (PaymentStatus::Canceled, PaymentStatus::Initiated),
            (PaymentStatus::Failed, PaymentStatus::Initiated),
            (PaymentStatus::Captured, PaymentStatus::PartiallyCaptured),
            (PaymentStatus::Refunded, PaymentStatus::PartiallyRefunded),
            (PaymentStatus::Refunded, PaymentStatus::Voided),
            (PaymentStatus::Voided, PaymentStatus::Canceled),
            (PaymentStatus::Canceled, PaymentStatus::Failed),
            (PaymentStatus::Failed, PaymentStatus::Paid),
            (PaymentStatus::SetupCompleted, PaymentStatus::Paid),
            (PaymentStatus::Paid, PaymentStatus::Failed),
            (PaymentStatus::Paid, PaymentStatus::Voided),
            (PaymentStatus::Paid, PaymentStatus::Canceled),
            (PaymentStatus::Captured, PaymentStatus::Voided),
            (PaymentStatus::PartiallyRefunded, PaymentStatus::Failed),
        ];

        for (current, incoming) in invalid_pairs {
            let err = validate_monotonic_transition(current, incoming).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation);
        }
    }

    #[test]
    fn tc324_ac4_tm2_verify_order_money_checks_amount_and_currency() {
        let expected = Money::new(15_000, "SAR").unwrap();
        let exact = Money::new(15_000, "sar").unwrap();
        let underpaid = Money::new(1_000, "SAR").unwrap();
        let wrong_currency = Money::new(15_000, "USD").unwrap();

        assert!(verify_order_money(&expected, &exact).is_ok());

        let amount_err = verify_order_money(&expected, &underpaid).unwrap_err();
        assert_eq!(amount_err.kind(), ErrorKind::Validation);

        let currency_err = verify_order_money(&expected, &wrong_currency).unwrap_err();
        assert_eq!(currency_err.kind(), ErrorKind::Validation);

        let jpy_expected = Money::new(500, "JPY").unwrap();
        let jpy_actual = Money::new(500, "JPY").unwrap();
        assert!(verify_order_money(&jpy_expected, &jpy_actual).is_ok());

        let kwd_expected = Money::new(1_500, "KWD").unwrap();
        let kwd_off = Money::new(1_499, "KWD").unwrap();
        let kwd_err = verify_order_money(&kwd_expected, &kwd_off).unwrap_err();
        assert_eq!(kwd_err.kind(), ErrorKind::Validation);
    }
}
