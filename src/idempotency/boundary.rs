//! Generic async idempotent mutation execution boundary enforcing 4xx-only
//! lock release, 5xx/timeout lock retention, and secret-redacted caching.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use some_payment_core::{
    IdempotencyEntry, IdempotencyStore, PaymentCapabilities, PaymentError, ReserveOutcome,
};

/// Default lock and cached-response TTL in seconds (`5` minutes).
pub const DEFAULT_LOCK_TTL_SECS: u64 = 300;

const REDACTED_SENTINEL: &str = "[REDACTED]";
const MAX_REDACTION_DEPTH: usize = 32;

const SENSITIVE_KEYS: &[&str] = &[
    "client_secret",
    "secret_token",
    "api_key",
    "secret_key",
    "webhook_secret",
    "hmac_secret",
    "private_key",
];

/// Scoped execution parameters for [`IdempotencyBoundary::execute_idempotent_mutation`].
///
/// Implements a manual [`fmt::Debug`] that redacts `key` as `"[REDACTED]"` so
/// raw idempotency keys never leak into diagnostic logs.
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::boundary::MutationContext;
/// use some_payment_core::PaymentCapabilities;
///
/// let ctx = MutationContext::new(
///     "tenant-1:moyasar:refund",
///     "ref-001",
///     PaymentCapabilities::REFUND,
/// );
/// assert_eq!(ctx.scope, "tenant-1:moyasar:refund");
/// assert!(format!("{ctx:?}").contains("[REDACTED]"));
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MutationContext<'a> {
    /// Namespaced idempotency scope (e.g., `"tenant-1:moyasar:refund"`).
    pub scope: &'a str,
    /// Caller-supplied idempotency key.
    pub key: &'a str,
    /// Gateway capability flags governing native idempotency bypass.
    pub capabilities: PaymentCapabilities,
}

impl<'a> MutationContext<'a> {
    /// Creates a new [`MutationContext`].
    #[must_use]
    pub const fn new(scope: &'a str, key: &'a str, capabilities: PaymentCapabilities) -> Self {
        Self {
            scope,
            key,
            capabilities,
        }
    }
}

impl fmt::Debug for MutationContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutationContext")
            .field("scope", &self.scope)
            .field("key", &REDACTED_SENTINEL)
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

/// Provider-agnostic async execution boundary wrapping non-idempotent payment
/// mutations with a distributed [`IdempotencyStore`].
///
/// # Lock-Retention Safety Invariant
///
/// When a gateway does not claim
/// [`PaymentCapabilities::NATIVE_MUTATION_IDEMPOTENCY`], this boundary acquires
/// an atomic lock via [`IdempotencyStore::reserve`]:
///
/// - [`ReserveOutcome::Completed`] replays the cached response with zero
///   outbound provider calls.
/// - [`ReserveOutcome::InProgress`] returns
///   [`PaymentError::IdempotencyConflict`] with zero outbound provider calls.
/// - [`ReserveOutcome::Acquired`] executes the mutation and:
///   - recursively redacts sensitive keys (`client_secret`, `secret_token`,
///     `api_key`, `secret_key`, `webhook_secret`, `hmac_secret`, `private_key`)
///     to `"[REDACTED]"` in the cached JSON before calling
///     [`IdempotencyStore::complete`], while returning the live unredacted `T`
///     to the caller,
///   - calls [`IdempotencyStore::release`] **only** when
///     [`PaymentError::is_definitive_client_rejection`] returns `true` (local
///     validation or definitive 4xx rejection),
///   - **retains the lock** on 5xx, rate-limit, or timeout failures and emits a
///     structured warning with a non-secret key fingerprint for reconciliation.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use async_trait::async_trait;
/// use some_payment::idempotency::boundary::{
///     DEFAULT_LOCK_TTL_SECS, IdempotencyBoundary, MutationContext,
/// };
/// use some_payment_core::{
///     IdempotencyEntry, IdempotencyStore, PaymentCapabilities, PaymentError, ReserveOutcome,
/// };
///
/// struct AlwaysAcquireStore;
///
/// #[async_trait]
/// impl IdempotencyStore for AlwaysAcquireStore {
///     async fn reserve(
///         &self,
///         _entry: &IdempotencyEntry<'_>,
///     ) -> Result<ReserveOutcome, PaymentError> {
///         Ok(ReserveOutcome::Acquired)
///     }
///
///     async fn complete(
///         &self,
///         _entry: &IdempotencyEntry<'_>,
///         _payload: &serde_json::Value,
///     ) -> Result<(), PaymentError> {
///         Ok(())
///     }
///
///     async fn release(&self, _scope: &str, _key: &str) -> Result<(), PaymentError> {
///         Ok(())
///     }
/// }
///
/// let boundary = IdempotencyBoundary::new(Arc::new(AlwaysAcquireStore), DEFAULT_LOCK_TTL_SECS)?;
/// let ctx = MutationContext::new(
///     "tenant-1:moyasar:refund",
///     "ref-001",
///     PaymentCapabilities::REFUND,
/// );
/// let rt = tokio::runtime::Builder::new_current_thread().build()?;
/// let result: String = rt.block_on(boundary.execute_idempotent_mutation(&ctx, || async {
///     Ok("refunded".to_owned())
/// }))?;
/// assert_eq!(result, "refunded");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct IdempotencyBoundary {
    store: Arc<dyn IdempotencyStore>,
    ttl_secs: u64,
}

impl fmt::Debug for IdempotencyBoundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdempotencyBoundary")
            .field("store", &"<IdempotencyStore>")
            .field("ttl_secs", &self.ttl_secs)
            .finish()
    }
}

impl IdempotencyBoundary {
    /// Creates a new [`IdempotencyBoundary`] with an explicit lock TTL in
    /// seconds.
    ///
    /// Returns [`PaymentError::ValidationError`] if `ttl_secs` is `0`.
    pub fn new(store: Arc<dyn IdempotencyStore>, ttl_secs: u64) -> Result<Self, PaymentError> {
        if ttl_secs == 0 {
            return Err(PaymentError::validation("idempotency lock TTL must be > 0"));
        }
        Ok(Self { store, ttl_secs })
    }

    /// Creates a new [`IdempotencyBoundary`] using [`DEFAULT_LOCK_TTL_SECS`].
    pub fn with_default_ttl(store: Arc<dyn IdempotencyStore>) -> Self {
        Self {
            store,
            ttl_secs: DEFAULT_LOCK_TTL_SECS,
        }
    }

    /// Returns the configured lock TTL in seconds.
    pub const fn ttl_secs(&self) -> u64 {
        self.ttl_secs
    }

    /// Returns a shared reference to the underlying [`IdempotencyStore`].
    pub fn store(&self) -> &Arc<dyn IdempotencyStore> {
        &self.store
    }

    /// Executes an async payment mutation within the distributed idempotency
    /// boundary.
    ///
    /// # Errors
    ///
    /// - [`PaymentError::ValidationError`] if `ctx.scope` or `ctx.key` is empty
    ///   or whitespace-only.
    /// - [`PaymentError::ConfigurationError`] if post-mutation serialization,
    ///   secret redaction depth bounds, or cached payload deserialization fails.
    /// - [`PaymentError::IdempotencyConflict`] if another worker currently
    ///   holds the lock (`ReserveOutcome::InProgress`).
    /// - Any [`PaymentError`] returned by the underlying [`IdempotencyStore`]
    ///   or `mutation` closure.
    pub async fn execute_idempotent_mutation<T, F, Fut>(
        &self,
        ctx: &MutationContext<'_>,
        mutation: F,
    ) -> Result<T, PaymentError>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, PaymentError>>,
    {
        validate_inputs(ctx.scope, ctx.key)?;
        if ctx
            .capabilities
            .contains(PaymentCapabilities::NATIVE_MUTATION_IDEMPOTENCY)
        {
            return mutation().await;
        }
        let entry = IdempotencyEntry::new(ctx.scope, ctx.key, self.ttl_secs);
        match self.store.reserve(&entry).await? {
            ReserveOutcome::Completed(cached) => deserialize_cached(cached),
            ReserveOutcome::InProgress => Err(PaymentError::IdempotencyConflict {
                scope: ctx.scope.to_owned(),
            }),
            ReserveOutcome::Acquired => self.handle_acquired(&entry, mutation).await,
        }
    }

    async fn handle_acquired<T, F, Fut>(
        &self,
        entry: &IdempotencyEntry<'_>,
        mutation: F,
    ) -> Result<T, PaymentError>
    where
        T: Serialize,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, PaymentError>>,
    {
        match mutation().await {
            Ok(result) => self.complete_acquired(entry, result).await,
            Err(err) => self.handle_mutation_error(entry, err).await,
        }
    }

    async fn complete_acquired<T: Serialize>(
        &self,
        entry: &IdempotencyEntry<'_>,
        result: T,
    ) -> Result<T, PaymentError> {
        let mut payload = serialize_payload(&result)?;
        scrub_sensitive_json(&mut payload, 0)?;
        self.store.complete(entry, &payload).await?;
        Ok(result)
    }

    async fn handle_mutation_error<T>(
        &self,
        entry: &IdempotencyEntry<'_>,
        err: PaymentError,
    ) -> Result<T, PaymentError> {
        let key_fingerprint = fingerprint_key(entry.scope, entry.key);
        if err.is_definitive_client_rejection() {
            self.release_after_client_rejection(entry, &key_fingerprint)
                .await;
            return Err(err);
        }
        tracing::warn!(
            scope = entry.scope,
            key_fingerprint = %key_fingerprint,
            error_kind = ?err.kind(),
            http_status = ?err.http_status(),
            "retaining idempotency lock after non-definitive failure; reconciliation required"
        );
        Err(err)
    }

    async fn release_after_client_rejection(
        &self,
        entry: &IdempotencyEntry<'_>,
        key_fingerprint: &str,
    ) {
        if let Err(release_err) = self.store.release(entry.scope, entry.key).await {
            tracing::warn!(
                scope = entry.scope,
                key_fingerprint = %key_fingerprint,
                error_kind = ?release_err.kind(),
                "failed to release idempotency lock after definitive client rejection"
            );
        }
    }
}

fn validate_inputs(scope: &str, key: &str) -> Result<(), PaymentError> {
    if scope.trim().is_empty() {
        return Err(PaymentError::validation(
            "idempotency scope must not be empty",
        ));
    }
    if key.trim().is_empty() {
        return Err(PaymentError::validation(
            "idempotency key must not be empty",
        ));
    }
    Ok(())
}

fn serialize_payload<T: Serialize>(value: &T) -> Result<Value, PaymentError> {
    let err = |_| PaymentError::configuration("failed to serialize idempotent mutation result");
    serde_json::to_value(value).map_err(err)
}

fn deserialize_cached<T: DeserializeOwned>(value: Value) -> Result<T, PaymentError> {
    let err = |_| PaymentError::configuration("failed to deserialize cached idempotency payload");
    serde_json::from_value(value).map_err(err)
}

fn scrub_sensitive_json(value: &mut Value, depth: usize) -> Result<(), PaymentError> {
    if depth > MAX_REDACTION_DEPTH {
        return Err(PaymentError::configuration(
            "idempotency payload exceeds maximum JSON nesting depth",
        ));
    }
    match value {
        Value::Object(map) => scrub_json_object(map, depth),
        Value::Array(items) => scrub_json_array(items, depth),
        _ => Ok(()),
    }
}

fn scrub_json_object(map: &mut Map<String, Value>, depth: usize) -> Result<(), PaymentError> {
    for (key, val) in map.iter_mut() {
        scrub_object_entry(key, val, depth)?;
    }
    Ok(())
}

fn scrub_object_entry(key: &str, val: &mut Value, depth: usize) -> Result<(), PaymentError> {
    if is_sensitive_key(key) && !val.is_null() {
        *val = Value::String(REDACTED_SENTINEL.to_owned());
        return Ok(());
    }
    scrub_sensitive_json(val, depth + 1)
}

fn scrub_json_array(items: &mut [Value], depth: usize) -> Result<(), PaymentError> {
    for item in items {
        scrub_sensitive_json(item, depth + 1)?;
    }
    Ok(())
}

fn is_sensitive_key(key: &str) -> bool {
    SENSITIVE_KEYS
        .iter()
        .any(|sensitive| key.eq_ignore_ascii_case(sensitive))
}

fn fingerprint_key(scope: &str, key: &str) -> String {
    let mut hasher = DefaultHasher::new();
    scope.hash(&mut hasher);
    0xff_u8.hash(&mut hasher);
    key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use serde::Deserialize;
    use some_payment_core::{
        ErrorKind, Money, NextAction, PaymentResult, PaymentStatus, ProviderId, ProviderReference,
    };

    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct MockReceipt {
        id: String,
        amount: i64,
    }

    #[derive(Debug)]
    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("intentional serialization fault"))
        }
    }

    impl<'de> Deserialize<'de> for FailingSerialize {
        fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
            Ok(Self)
        }
    }

    struct MockStore {
        outcome: ReserveOutcome,
        fail_release: bool,
        reserve_calls: AtomicUsize,
        complete_calls: AtomicUsize,
        release_calls: AtomicUsize,
        last_completed: Mutex<Option<Value>>,
    }

    impl MockStore {
        fn new(outcome: ReserveOutcome) -> Self {
            Self {
                outcome,
                fail_release: false,
                reserve_calls: AtomicUsize::new(0),
                complete_calls: AtomicUsize::new(0),
                release_calls: AtomicUsize::new(0),
                last_completed: Mutex::new(None),
            }
        }

        fn with_failing_release(outcome: ReserveOutcome) -> Self {
            Self {
                fail_release: true,
                ..Self::new(outcome)
            }
        }

        fn counts(&self) -> (usize, usize, usize) {
            (
                self.reserve_calls.load(Ordering::SeqCst),
                self.complete_calls.load(Ordering::SeqCst),
                self.release_calls.load(Ordering::SeqCst),
            )
        }
    }

    #[async_trait]
    impl IdempotencyStore for MockStore {
        async fn reserve(
            &self,
            _entry: &IdempotencyEntry<'_>,
        ) -> Result<ReserveOutcome, PaymentError> {
            self.reserve_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.outcome.clone())
        }

        async fn complete(
            &self,
            _entry: &IdempotencyEntry<'_>,
            payload: &Value,
        ) -> Result<(), PaymentError> {
            self.complete_calls.fetch_add(1, Ordering::SeqCst);
            *self.last_completed.lock().unwrap() = Some(payload.clone());
            Ok(())
        }

        async fn release(&self, _scope: &str, _key: &str) -> Result<(), PaymentError> {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_release {
                return Err(PaymentError::ProviderUnavailable {
                    provider: ProviderId::new("store"),
                    http_status: Some(503),
                    indeterminate: true,
                    message: "redis unavailable".to_owned(),
                });
            }
            Ok(())
        }
    }

    #[test]
    fn constructor_rejects_zero_ttl_and_default_uses_constant() {
        let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
        let err = IdempotencyBoundary::new(store.clone(), 0).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);

        let boundary = IdempotencyBoundary::with_default_ttl(store);
        assert_eq!(boundary.ttl_secs(), DEFAULT_LOCK_TTL_SECS);
        let debug_repr = format!("{boundary:?}");
        assert!(
            debug_repr.contains("IdempotencyBoundary") && debug_repr.contains("<IdempotencyStore>")
        );
    }

    #[test]
    fn mutation_context_debug_redacts_raw_key() {
        let ctx = MutationContext::new(
            "tenant-1:moyasar:refund",
            "secret-idemp-key-999",
            PaymentCapabilities::REFUND,
        );
        let dbg = format!("{ctx:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("secret-idemp-key-999"));
    }

    #[tokio::test]
    async fn rejects_empty_or_whitespace_scope_and_key() {
        let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let provider_calls = AtomicUsize::new(0);

        for (scope, key) in [("", "k1"), ("   ", "k1"), ("s1", ""), ("s1", "  \t ")] {
            let ctx = MutationContext::new(scope, key, PaymentCapabilities::REFUND);
            let err = boundary
                .execute_idempotent_mutation(&ctx, || async {
                    provider_calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, PaymentError>(MockReceipt {
                        id: "r1".to_owned(),
                        amount: 100,
                    })
                })
                .await
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation);
        }

        assert_eq!(
            (provider_calls.load(Ordering::SeqCst), store.counts()),
            (0, (0, 0, 0))
        );
    }

    #[tokio::test]
    async fn native_mutation_idempotency_bypasses_store() {
        let store = Arc::new(MockStore::new(ReserveOutcome::InProgress));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let provider_calls = AtomicUsize::new(0);
        let expected = MockReceipt {
            id: "rf_native".to_owned(),
            amount: 2500,
        };
        let ctx = MutationContext::new(
            "tenant-1:stripe:refund",
            "key-native",
            PaymentCapabilities::REFUND | PaymentCapabilities::NATIVE_MUTATION_IDEMPOTENCY,
        );

        let result = boundary
            .execute_idempotent_mutation(&ctx, || async {
                provider_calls.fetch_add(1, Ordering::SeqCst);
                Ok(expected.clone())
            })
            .await
            .unwrap();

        assert_eq!(result, expected);
        assert_eq!(
            (provider_calls.load(Ordering::SeqCst), store.counts()),
            (1, (0, 0, 0))
        );
    }

    #[tokio::test]
    async fn completed_cache_replays_with_zero_provider_calls() {
        let cached = MockReceipt {
            id: "rf_cached".to_owned(),
            amount: 5000,
        };
        let store = Arc::new(MockStore::new(ReserveOutcome::Completed(
            serde_json::to_value(&cached).unwrap(),
        )));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let provider_calls = AtomicUsize::new(0);
        let ctx = MutationContext::new(
            "tenant-1:moyasar:refund",
            "refund:ord-1",
            PaymentCapabilities::REFUND,
        );

        let result: MockReceipt = boundary
            .execute_idempotent_mutation(&ctx, || async {
                provider_calls.fetch_add(1, Ordering::SeqCst);
                Ok(MockReceipt {
                    id: "should_not_run".to_owned(),
                    amount: 0,
                })
            })
            .await
            .unwrap();

        assert_eq!(result, cached);
        assert_eq!(
            (provider_calls.load(Ordering::SeqCst), store.counts()),
            (0, (1, 0, 0))
        );
    }

    #[tokio::test]
    async fn in_progress_returns_idempotency_conflict_with_zero_provider_calls() {
        let store = Arc::new(MockStore::new(ReserveOutcome::InProgress));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let provider_calls = AtomicUsize::new(0);
        let ctx = MutationContext::new(
            "tenant-1:moyasar:refund",
            "refund:ord-1",
            PaymentCapabilities::REFUND,
        );

        let err = boundary
            .execute_idempotent_mutation(&ctx, || async {
                provider_calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, PaymentError>(MockReceipt {
                    id: "never".to_owned(),
                    amount: 0,
                })
            })
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            PaymentError::IdempotencyConflict { ref scope } if scope == "tenant-1:moyasar:refund"
        ));
        assert_eq!(
            (provider_calls.load(Ordering::SeqCst), store.counts()),
            (0, (1, 0, 0))
        );
    }

    #[tokio::test]
    async fn acquired_ok_completes_store_and_returns_result() {
        let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::new(store.clone(), 120).unwrap();
        let expected = MockReceipt {
            id: "rf_ok".to_owned(),
            amount: 7500,
        };
        let ctx = MutationContext::new(
            "tenant-1:paymob:capture",
            "cap:ord-2",
            PaymentCapabilities::AUTH_CAPTURE,
        );

        let result = boundary
            .execute_idempotent_mutation(&ctx, || async { Ok(expected.clone()) })
            .await
            .unwrap();

        assert_eq!(result, expected);
        assert_eq!(store.counts(), (1, 1, 0));
        let saved = store.last_completed.lock().unwrap().clone().unwrap();
        assert_eq!(saved, serde_json::to_value(&expected).unwrap());
    }

    #[tokio::test]
    async fn tm4_embedded_client_secret_redacted_in_store_but_returned_live() {
        let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let reference = ProviderReference::builder(ProviderId::new("moyasar"))
            .payment_id("pay_live_1")
            .build()
            .unwrap();
        let live_payment = PaymentResult {
            reference,
            status: PaymentStatus::RequiresAction,
            money: Money::new(5_000, "SAR").unwrap(),
            captured_amount: None,
            refunded_amount: None,
            next_action: Some(NextAction::EmbeddedClientSecret {
                client_secret: "pi_secret_live_CANARY_777".to_owned(),
            }),
            checkout_url: None,
            livemode: false,
        };
        let ctx = MutationContext::new(
            "tenant-1:moyasar:create",
            "ord-secret-1",
            PaymentCapabilities::empty(),
        );

        let returned = boundary
            .execute_idempotent_mutation(&ctx, || async { Ok(live_payment.clone()) })
            .await
            .unwrap();

        assert_eq!(returned, live_payment);
        let cached_str = store
            .last_completed
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .to_string();
        assert!(
            cached_str.contains("[REDACTED]") && !cached_str.contains("pi_secret_live_CANARY_777")
        );
    }

    #[tokio::test]
    async fn serialization_failure_returns_configuration_error_not_validation() {
        let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let ctx = MutationContext::new(
            "tenant-1:moyasar:refund",
            "ref:ser-err",
            PaymentCapabilities::REFUND,
        );

        let err = boundary
            .execute_idempotent_mutation::<FailingSerialize, _, _>(&ctx, || async {
                Ok(FailingSerialize)
            })
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ErrorKind::Configuration);
        assert!(!err.is_definitive_client_rejection());
        assert_eq!(store.counts(), (1, 0, 0));
    }

    #[tokio::test]
    async fn acquired_definitive_4xx_and_validation_errors_release_lock() {
        let cases = [
            PaymentError::validation("capture amount exceeds authorized amount"),
            PaymentError::ProviderError {
                provider: ProviderId::new("paymob"),
                http_status: Some(400),
                vendor_code: Some("invalid_capture".to_owned()),
                message: "bad capture request".to_owned(),
            },
        ];

        for expected_err in cases {
            let expected_kind = expected_err.kind();
            let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
            let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
            let ctx = MutationContext::new(
                "tenant-1:paymob:capture",
                "cap:ord-3",
                PaymentCapabilities::AUTH_CAPTURE,
            );

            let err = boundary
                .execute_idempotent_mutation::<MockReceipt, _, _>(&ctx, || async {
                    Err(expected_err)
                })
                .await
                .unwrap_err();

            assert_eq!((err.kind(), store.counts()), (expected_kind, (1, 0, 1)));
        }
    }

    #[tokio::test]
    async fn acquired_release_failure_still_returns_original_client_error() {
        let store = Arc::new(MockStore::with_failing_release(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let ctx = MutationContext::new(
            "tenant-1:paymob:capture",
            "cap:ord-4",
            PaymentCapabilities::AUTH_CAPTURE,
        );

        let err = boundary
            .execute_idempotent_mutation::<MockReceipt, _, _>(&ctx, || async {
                Err(PaymentError::validation("invalid amount"))
            })
            .await
            .unwrap_err();

        assert_eq!(
            (err.kind(), store.counts()),
            (ErrorKind::Validation, (1, 0, 1))
        );
    }

    #[tokio::test]
    async fn acquired_5xx_and_timeout_retain_lock_without_release() {
        let cases = [
            PaymentError::ProviderUnavailable {
                provider: ProviderId::new("moyasar"),
                http_status: Some(504),
                indeterminate: true,
                message: "gateway timeout".to_owned(),
            },
            PaymentError::ProviderError {
                provider: ProviderId::new("moyasar"),
                http_status: Some(500),
                vendor_code: None,
                message: "internal server error".to_owned(),
            },
            PaymentError::RateLimited {
                provider: ProviderId::new("moyasar"),
                retry_after_secs: Some(30),
            },
        ];

        for expected_err in cases {
            let expected_kind = expected_err.kind();
            let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
            let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
            let ctx = MutationContext::new(
                "tenant-1:moyasar:refund",
                "ref:ord-5",
                PaymentCapabilities::REFUND,
            );

            let err = boundary
                .execute_idempotent_mutation::<MockReceipt, _, _>(&ctx, || async {
                    Err(expected_err)
                })
                .await
                .unwrap_err();

            assert_eq!((err.kind(), store.counts()), (expected_kind, (1, 0, 0)));
        }
    }

    #[test]
    fn fingerprint_key_is_deterministic_and_non_secret() {
        let fp1 = fingerprint_key("tenant-1:moyasar:refund", "secret-key-123");
        let fp2 = fingerprint_key("tenant-1:moyasar:refund", "secret-key-123");

        assert_eq!(fp1, fp2);
        assert!(fp1.len() == 16 && fp1.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!fp1.contains("secret-key-123"));
    }

    #[test]
    fn fingerprint_key_separates_scope_and_key_boundaries() {
        let fp_base = fingerprint_key("tenant-1:moyasar:refund", "secret-key-123");
        let fp_other = fingerprint_key("tenant-1:moyasar:refund", "secret-key-124");
        let fp_ab_c = fingerprint_key("scope-ab", "c");
        let fp_a_bc = fingerprint_key("scope-a", "bc");

        assert_ne!(fp_base, fp_other);
        assert_ne!(fp_ab_c, fp_a_bc);
    }
}
