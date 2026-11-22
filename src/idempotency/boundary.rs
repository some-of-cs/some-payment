//! Generic async idempotent mutation execution boundary enforcing 4xx-only
//! lock release and 5xx/timeout lock retention.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use some_payment_core::{
    IdempotencyEntry, IdempotencyStore, PaymentCapabilities, PaymentError, ReserveOutcome,
};

/// Default lock and cached-response TTL in seconds (`5` minutes).
pub const DEFAULT_LOCK_TTL_SECS: u64 = 300;

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
///   - calls [`IdempotencyStore::complete`] on `Ok`,
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
/// use some_payment::idempotency::boundary::{DEFAULT_LOCK_TTL_SECS, IdempotencyBoundary};
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
/// let rt = tokio::runtime::Builder::new_current_thread().build()?;
/// let result: String = rt.block_on(boundary.execute_idempotent_mutation(
///     "tenant-1:moyasar:refund",
///     "ref-001",
///     PaymentCapabilities::REFUND,
///     || async { Ok("refunded".to_owned()) },
/// ))?;
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
    /// - [`PaymentError::ValidationError`] if `scope` or `key` is empty or
    ///   whitespace-only, or if payload serialization/deserialization fails.
    /// - [`PaymentError::IdempotencyConflict`] if another worker currently
    ///   holds the lock (`ReserveOutcome::InProgress`).
    /// - Any [`PaymentError`] returned by the underlying [`IdempotencyStore`]
    ///   or `mutation` closure.
    pub async fn execute_idempotent_mutation<T, F, Fut>(
        &self,
        scope: &str,
        key: &str,
        capabilities: PaymentCapabilities,
        mutation: F,
    ) -> Result<T, PaymentError>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, PaymentError>>,
    {
        validate_inputs(scope, key)?;
        if capabilities.contains(PaymentCapabilities::NATIVE_MUTATION_IDEMPOTENCY) {
            return mutation().await;
        }
        let entry = IdempotencyEntry::new(scope, key, self.ttl_secs);
        match self.store.reserve(&entry).await? {
            ReserveOutcome::Completed(cached) => deserialize_cached(cached),
            ReserveOutcome::InProgress => Err(PaymentError::IdempotencyConflict {
                scope: scope.to_owned(),
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
        let payload = serialize_payload(&result)?;
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
            self.release_after_client_rejection(entry, &key_fingerprint).await;
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
        return Err(PaymentError::validation("idempotency scope must not be empty"));
    }
    if key.trim().is_empty() {
        return Err(PaymentError::validation("idempotency key must not be empty"));
    }
    Ok(())
}

fn serialize_payload<T: Serialize>(value: &T) -> Result<serde_json::Value, PaymentError> {
    let err = |_| PaymentError::validation("failed to serialize idempotency payload");
    serde_json::to_value(value).map_err(err)
}

fn deserialize_cached<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, PaymentError> {
    let err = |_| PaymentError::validation("failed to deserialize cached idempotency payload");
    serde_json::from_value(value).map_err(err)
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
    use some_payment_core::{ErrorKind, ProviderId};

    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct MockReceipt {
        id: String,
        amount: i64,
    }

    struct MockStore {
        outcome: ReserveOutcome,
        fail_release: bool,
        reserve_calls: AtomicUsize,
        complete_calls: AtomicUsize,
        release_calls: AtomicUsize,
        last_completed: Mutex<Option<serde_json::Value>>,
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
            payload: &serde_json::Value,
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
        assert!(debug_repr.contains("IdempotencyBoundary"));
        assert!(debug_repr.contains("<IdempotencyStore>"));
    }

    #[tokio::test]
    async fn rejects_empty_or_whitespace_scope_and_key() {
        let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let provider_calls = AtomicUsize::new(0);

        for (scope, key) in [("", "k1"), ("   ", "k1"), ("s1", ""), ("s1", "  	 ")] {
            let err = boundary
                .execute_idempotent_mutation(scope, key, PaymentCapabilities::REFUND, || async {
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

        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.reserve_calls.load(Ordering::SeqCst), 0);
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

        let result = boundary
            .execute_idempotent_mutation(
                "tenant-1:stripe:refund",
                "key-native",
                PaymentCapabilities::REFUND | PaymentCapabilities::NATIVE_MUTATION_IDEMPOTENCY,
                || async {
                    provider_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(expected.clone())
                },
            )
            .await
            .unwrap();

        assert_eq!(result, expected);
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.reserve_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);
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

        let result: MockReceipt = boundary
            .execute_idempotent_mutation(
                "tenant-1:moyasar:refund",
                "refund:ord-1",
                PaymentCapabilities::REFUND,
                || async {
                    provider_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(MockReceipt {
                        id: "should_not_run".to_owned(),
                        amount: 0,
                    })
                },
            )
            .await
            .unwrap();

        assert_eq!(result, cached);
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn in_progress_returns_idempotency_conflict_with_zero_provider_calls() {
        let store = Arc::new(MockStore::new(ReserveOutcome::InProgress));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());
        let provider_calls = AtomicUsize::new(0);

        let err = boundary
            .execute_idempotent_mutation(
                "tenant-1:moyasar:refund",
                "refund:ord-1",
                PaymentCapabilities::REFUND,
                || async {
                    provider_calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, PaymentError>(MockReceipt {
                        id: "never".to_owned(),
                        amount: 0,
                    })
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            PaymentError::IdempotencyConflict { ref scope } if scope == "tenant-1:moyasar:refund"
        ));
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn acquired_ok_completes_store_and_returns_result() {
        let store = Arc::new(MockStore::new(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::new(store.clone(), 120).unwrap();
        let expected = MockReceipt {
            id: "rf_ok".to_owned(),
            amount: 7500,
        };

        let result = boundary
            .execute_idempotent_mutation(
                "tenant-1:paymob:capture",
                "cap:ord-2",
                PaymentCapabilities::AUTH_CAPTURE,
                || async { Ok(expected.clone()) },
            )
            .await
            .unwrap();

        assert_eq!(result, expected);
        assert_eq!(store.reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.complete_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);
        let saved = store.last_completed.lock().unwrap().clone().unwrap();
        assert_eq!(saved, serde_json::to_value(&expected).unwrap());
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

            let err = boundary
                .execute_idempotent_mutation::<MockReceipt, _, _>(
                    "tenant-1:paymob:capture",
                    "cap:ord-3",
                    PaymentCapabilities::AUTH_CAPTURE,
                    || async { Err(expected_err) },
                )
                .await
                .unwrap_err();

            assert_eq!(err.kind(), expected_kind);
            assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
            assert_eq!(store.release_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn acquired_release_failure_still_returns_original_client_error() {
        let store = Arc::new(MockStore::with_failing_release(ReserveOutcome::Acquired));
        let boundary = IdempotencyBoundary::with_default_ttl(store.clone());

        let err = boundary
            .execute_idempotent_mutation::<MockReceipt, _, _>(
                "tenant-1:paymob:capture",
                "cap:ord-4",
                PaymentCapabilities::AUTH_CAPTURE,
                || async { Err(PaymentError::validation("invalid amount")) },
            )
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ErrorKind::Validation);
        assert_eq!(store.release_calls.load(Ordering::SeqCst), 1);
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

            let err = boundary
                .execute_idempotent_mutation::<MockReceipt, _, _>(
                    "tenant-1:moyasar:refund",
                    "ref:ord-5",
                    PaymentCapabilities::REFUND,
                    || async { Err(expected_err) },
                )
                .await
                .unwrap_err();

            assert_eq!(err.kind(), expected_kind);
            assert_eq!(store.complete_calls.load(Ordering::SeqCst), 0);
            assert_eq!(store.release_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn fingerprint_key_is_deterministic_and_does_not_leak_raw_key() {
        let fp1 = fingerprint_key("tenant-1:moyasar:refund", "secret-key-123");
        let fp2 = fingerprint_key("tenant-1:moyasar:refund", "secret-key-123");
        let fp3 = fingerprint_key("tenant-1:moyasar:refund", "secret-key-124");
        let fp_ab_c = fingerprint_key("scope-ab", "c");
        let fp_a_bc = fingerprint_key("scope-a", "bc");

        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 16);
        assert!(fp1.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!fp1.contains("secret-key-123"));
        assert_ne!(fp1, fp3);
        assert_ne!(fp_ab_c, fp_a_bc);
    }
}
