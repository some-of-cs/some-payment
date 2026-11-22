//! Thread-safe [`InMemoryIdempotencyStore`] with TTL expiry and capacity bounds
//! strictly for development and automated testing.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use some_payment_core::{IdempotencyEntry, IdempotencyStore, PaymentError, ReserveOutcome};

/// Default upper bound on active entries stored in [`InMemoryIdempotencyStore`].
///
/// Enforces a fail-closed memory bound (`TM-5`) so unbounded caller keys cannot
/// exhaust process memory.
pub const DEFAULT_MAX_CAPACITY: usize = 10_000;

type StoreKey = (String, String);
type EntryMap = HashMap<StoreKey, StoredRecord>;

enum EntryState {
    InProgress,
    Completed(serde_json::Value),
}

impl EntryState {
    fn to_reserve_outcome(&self) -> ReserveOutcome {
        match self {
            Self::InProgress => ReserveOutcome::InProgress,
            Self::Completed(payload) => ReserveOutcome::Completed(payload.clone()),
        }
    }
}

struct StoredRecord {
    state: EntryState,
    expires_at: Instant,
}

/// Thread-safe, in-memory [`IdempotencyStore`] implementation with TTL eviction
/// and a fail-closed entry capacity bound.
///
/// # Warning
///
/// This store is intended **strictly for local development and automated
/// testing**. Entries live only inside a single process's heap and are lost on
/// restart or across horizontally scaled worker replicas. For production
/// deployments, use a shared durable backend such as Redis (`SET NX EX`) or
/// PostgreSQL (`INSERT ... ON CONFLICT`) so concurrent workers share a single
/// atomic lock space (mitigating `TM-5`).
///
/// # Security
///
/// - [`fmt::Debug`] is redacted and reports only the current entry count and
///   `max_capacity`, never cached payloads or raw keys (mitigating `TM-4`).
/// - Rejects new reservations once `max_capacity` is reached after evicting
///   expired entries (fail-closed DoS protection, mitigating `TM-5`).
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::memory_store::InMemoryIdempotencyStore;
/// use some_payment::some_payment_core::{
///     IdempotencyEntry, IdempotencyStore, PaymentError, ReserveOutcome,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let rt = tokio::runtime::Builder::new_current_thread().build()?;
/// rt.block_on(async {
///     let store = InMemoryIdempotencyStore::new();
///     let entry = IdempotencyEntry::new("tenant-1:moyasar:refund", "ord-1", 300);
///
///     assert_eq!(store.reserve(&entry).await?, ReserveOutcome::Acquired);
///     assert_eq!(store.reserve(&entry).await?, ReserveOutcome::InProgress);
///
///     let payload = serde_json::json!({"status": "succeeded"});
///     store.complete(&entry, &payload).await?;
///     assert_eq!(
///         store.reserve(&entry).await?,
///         ReserveOutcome::Completed(payload)
///     );
///     Ok::<(), PaymentError>(())
/// })?;
/// # Ok(())
/// # }
/// ```
pub struct InMemoryIdempotencyStore {
    entries: Mutex<EntryMap>,
    max_capacity: usize,
}

impl InMemoryIdempotencyStore {
    /// Creates an empty store bounded by [`DEFAULT_MAX_CAPACITY`].
    ///
    /// # Examples
    ///
    /// ```
    /// use some_payment::idempotency::memory_store::{
    ///     DEFAULT_MAX_CAPACITY, InMemoryIdempotencyStore,
    /// };
    ///
    /// let store = InMemoryIdempotencyStore::new();
    /// assert!(store.is_empty());
    /// assert_eq!(store.max_capacity(), DEFAULT_MAX_CAPACITY);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_capacity: DEFAULT_MAX_CAPACITY,
        }
    }

    /// Creates an empty store with an explicit `max_capacity` ceiling.
    ///
    /// Returns [`PaymentError::ConfigurationError`] if `max_capacity` is `0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use some_payment::idempotency::memory_store::InMemoryIdempotencyStore;
    ///
    /// assert!(InMemoryIdempotencyStore::with_max_capacity(0).is_err());
    /// let store = InMemoryIdempotencyStore::with_max_capacity(64).unwrap();
    /// assert_eq!(store.max_capacity(), 64);
    /// ```
    pub fn with_max_capacity(max_capacity: usize) -> Result<Self, PaymentError> {
        if max_capacity == 0 {
            return Err(PaymentError::configuration(
                "in-memory idempotency store max_capacity must be greater than zero",
            ));
        }
        Ok(Self {
            entries: Mutex::new(HashMap::new()),
            max_capacity,
        })
    }

    /// Returns the number of entries currently held in the store.
    ///
    /// Expired entries remain counted until the next mutating operation evicts
    /// them. Returns `0` if the internal mutex has been poisoned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |guard| guard.len())
    }

    /// Returns `true` if the store currently holds zero entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the configured maximum entry capacity of this store.
    #[must_use]
    pub fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    fn lock_entries(&self) -> Result<MutexGuard<'_, EntryMap>, PaymentError> {
        self.entries
            .lock()
            .map_err(|_| PaymentError::configuration("in-memory idempotency store mutex poisoned"))
    }

    fn reserve_at(
        &self,
        entry: &IdempotencyEntry<'_>,
        now: Instant,
    ) -> Result<ReserveOutcome, PaymentError> {
        validate_entry(entry)?;
        let mut entries = self.lock_entries()?;
        evict_expired(&mut entries, now);

        let store_key = (entry.scope.to_owned(), entry.key.to_owned());
        if let Some(existing) = entries.get(&store_key) {
            return Ok(existing.state.to_reserve_outcome());
        }

        if entries.len() >= self.max_capacity {
            return Err(PaymentError::configuration(
                "in-memory idempotency store capacity reached",
            ));
        }

        let expires_at = compute_expires_at(now, entry.ttl_secs)?;
        entries.insert(
            store_key,
            StoredRecord {
                state: EntryState::InProgress,
                expires_at,
            },
        );
        Ok(ReserveOutcome::Acquired)
    }

    fn complete_at(
        &self,
        entry: &IdempotencyEntry<'_>,
        payload: &serde_json::Value,
        now: Instant,
    ) -> Result<(), PaymentError> {
        validate_entry(entry)?;
        let expires_at = compute_expires_at(now, entry.ttl_secs)?;
        let mut entries = self.lock_entries()?;
        evict_expired(&mut entries, now);

        let store_key = (entry.scope.to_owned(), entry.key.to_owned());
        if !entries.contains_key(&store_key) && entries.len() >= self.max_capacity {
            return Err(PaymentError::configuration(
                "in-memory idempotency store capacity reached",
            ));
        }

        entries.insert(
            store_key,
            StoredRecord {
                state: EntryState::Completed(payload.clone()),
                expires_at,
            },
        );
        Ok(())
    }
}

impl Default for InMemoryIdempotencyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemoryIdempotencyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryIdempotencyStore")
            .field("entries", &self.len())
            .field("max_capacity", &self.max_capacity)
            .finish()
    }
}

#[async_trait]
impl IdempotencyStore for InMemoryIdempotencyStore {
    async fn reserve(&self, entry: &IdempotencyEntry<'_>) -> Result<ReserveOutcome, PaymentError> {
        self.reserve_at(entry, Instant::now())
    }

    async fn complete(
        &self,
        entry: &IdempotencyEntry<'_>,
        payload: &serde_json::Value,
    ) -> Result<(), PaymentError> {
        self.complete_at(entry, payload, Instant::now())
    }

    async fn release(&self, scope: &str, key: &str) -> Result<(), PaymentError> {
        validate_scope_and_key(scope, key)?;
        let mut entries = self.lock_entries()?;
        entries.remove(&(scope.to_owned(), key.to_owned()));
        Ok(())
    }
}

fn validate_scope_and_key(scope: &str, key: &str) -> Result<(), PaymentError> {
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

fn validate_entry(entry: &IdempotencyEntry<'_>) -> Result<(), PaymentError> {
    validate_scope_and_key(entry.scope, entry.key)?;
    if entry.ttl_secs == 0 {
        return Err(PaymentError::validation(
            "idempotency ttl_secs must be greater than zero",
        ));
    }
    Ok(())
}

fn compute_expires_at(now: Instant, ttl_secs: u64) -> Result<Instant, PaymentError> {
    now.checked_add(Duration::from_secs(ttl_secs))
        .ok_or_else(|| PaymentError::validation("idempotency ttl_secs overflows instant"))
}

fn evict_expired(entries: &mut EntryMap, now: Instant) {
    entries.retain(|_, record| now < record.expires_at);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use some_payment_core::{
        ErrorKind, IdempotencyEntry, IdempotencyStore, PaymentError, ReserveOutcome,
    };

    use super::{DEFAULT_MAX_CAPACITY, InMemoryIdempotencyStore};

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tc_3_1_3_fifty_concurrent_tasks_acquire_exactly_once() {
        let store = Arc::new(InMemoryIdempotencyStore::new());
        let mut handles = Vec::with_capacity(50);

        for _ in 0..50 {
            let store = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                let entry = IdempotencyEntry::new("tenant-1:moyasar:refund", "ord-tc-3-1-3", 300);
                store.reserve(&entry).await
            }));
        }

        let mut acquired = 0usize;
        let mut in_progress = 0usize;

        for handle in handles {
            let outcome = handle
                .await
                .expect("task panicked")
                .expect("reserve failed");
            match outcome {
                ReserveOutcome::Acquired => acquired += 1,
                ReserveOutcome::InProgress => in_progress += 1,
                ReserveOutcome::Completed(_) => panic!("unexpected completed outcome"),
            }
        }

        assert_eq!(acquired, 1);
        assert_eq!(in_progress, 49);
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn complete_and_replay_returns_cached_payload_and_isolates_scopes() {
        let store = InMemoryIdempotencyStore::new();
        let refund_entry = IdempotencyEntry::new("tenant-1:moyasar:refund", "ord-1", 300);
        let capture_entry = IdempotencyEntry::new("tenant-1:moyasar:capture", "ord-1", 300);
        let payload = serde_json::json!({"refund_id": "rf-1", "status": "succeeded"});

        assert_eq!(
            store.reserve(&refund_entry).await.unwrap(),
            ReserveOutcome::Acquired
        );
        store.complete(&refund_entry, &payload).await.unwrap();
        assert_eq!(
            store.reserve(&refund_entry).await.unwrap(),
            ReserveOutcome::Completed(payload)
        );
        assert_eq!(
            store.reserve(&capture_entry).await.unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(store.len(), 2);
    }

    #[tokio::test]
    async fn release_removes_lock_and_allows_reacquire() {
        let store = InMemoryIdempotencyStore::new();
        let entry = IdempotencyEntry::new("tenant-1:paymob:capture", "ord-2", 300);

        assert_eq!(
            store.reserve(&entry).await.unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(
            store.reserve(&entry).await.unwrap(),
            ReserveOutcome::InProgress
        );

        store.release(entry.scope, entry.key).await.unwrap();
        assert!(store.is_empty());

        assert_eq!(
            store.reserve(&entry).await.unwrap(),
            ReserveOutcome::Acquired
        );
    }

    #[test]
    fn ttl_expiration_evicts_in_progress_and_completed_entries() {
        let store = InMemoryIdempotencyStore::new();
        let base = Instant::now();
        let entry = IdempotencyEntry::new("tenant-1:moyasar:refund", "ord-ttl", 60);
        let payload = serde_json::json!({"status": "refunded"});

        assert_eq!(
            store.reserve_at(&entry, base).unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(
            store
                .reserve_at(&entry, base + Duration::from_secs(59))
                .unwrap(),
            ReserveOutcome::InProgress
        );

        let expired_at = base + Duration::from_secs(60);
        assert_eq!(
            store.reserve_at(&entry, expired_at).unwrap(),
            ReserveOutcome::Acquired
        );

        let completed_entry = IdempotencyEntry::new("tenant-1:moyasar:refund", "ord-ttl", 30);
        store
            .complete_at(&completed_entry, &payload, expired_at)
            .unwrap();
        assert_eq!(
            store
                .reserve_at(&completed_entry, expired_at + Duration::from_secs(29))
                .unwrap(),
            ReserveOutcome::Completed(payload)
        );
        assert_eq!(
            store
                .reserve_at(&completed_entry, expired_at + Duration::from_secs(30))
                .unwrap(),
            ReserveOutcome::Acquired
        );
    }

    #[test]
    fn max_capacity_enforces_fail_closed_dos_bound() {
        let zero_err = InMemoryIdempotencyStore::with_max_capacity(0).unwrap_err();
        assert_eq!(zero_err.kind(), ErrorKind::Configuration);
        assert_eq!(
            InMemoryIdempotencyStore::default().max_capacity(),
            DEFAULT_MAX_CAPACITY
        );

        let store = InMemoryIdempotencyStore::with_max_capacity(2).unwrap();
        assert_eq!(store.max_capacity(), 2);

        let base = Instant::now();
        let k1 = IdempotencyEntry::new("scope", "k1", 10);
        let k2 = IdempotencyEntry::new("scope", "k2", 10);
        let k3 = IdempotencyEntry::new("scope", "k3", 10);
        let payload = serde_json::json!({"ok": true});

        assert_eq!(
            store.reserve_at(&k1, base).unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(
            store.reserve_at(&k2, base).unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(store.len(), 2);

        let mid = base + Duration::from_secs(5);
        let cap_err = store.reserve_at(&k3, mid).unwrap_err();
        assert_eq!(cap_err.kind(), ErrorKind::Configuration);
        let PaymentError::ConfigurationError { message } = cap_err else {
            panic!("expected configuration error");
        };
        assert!(message.contains("capacity reached"));

        store.complete_at(&k1, &payload, mid).unwrap();
        assert_eq!(
            store.reserve_at(&k1, mid).unwrap(),
            ReserveOutcome::Completed(payload)
        );

        let cap_complete_err = store
            .complete_at(&k3, &serde_json::json!({}), mid)
            .unwrap_err();
        assert_eq!(cap_complete_err.kind(), ErrorKind::Configuration);

        let after_expiry = base + Duration::from_secs(16);
        assert_eq!(
            store.reserve_at(&k3, after_expiry).unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn debug_output_redacts_keys_and_cached_payloads() {
        let store = InMemoryIdempotencyStore::new();
        let entry = IdempotencyEntry::new("tenant-secret-scope", "raw-secret-key-42", 300);
        let payload = serde_json::json!({"secret_token": "tok_live_9999"});

        store.reserve(&entry).await.unwrap();
        store.complete(&entry, &payload).await.unwrap();

        let debug = format!("{store:?}");
        assert!(debug.contains("InMemoryIdempotencyStore"));
        assert!(debug.contains("entries: 1"));
        assert!(debug.contains("max_capacity: 10000"));
        assert!(!debug.contains("tenant-secret-scope"));
        assert!(!debug.contains("raw-secret-key-42"));
        assert!(!debug.contains("tok_live_9999"));
        assert!(!debug.contains("secret_token"));
    }

    #[tokio::test]
    async fn rejects_invalid_entries_and_ttl_overflow() {
        let store = InMemoryIdempotencyStore::new();
        let invalid_entries = [
            IdempotencyEntry::new("", "k1", 60),
            IdempotencyEntry::new("   ", "k1", 60),
            IdempotencyEntry::new("scope", "", 60),
            IdempotencyEntry::new("scope", "   ", 60),
            IdempotencyEntry::new("scope", "k1", 0),
            IdempotencyEntry::new("scope", "k1", u64::MAX),
        ];

        for entry in invalid_entries {
            let err = store.reserve(&entry).await.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation);
        }

        assert_eq!(
            store.release("", "k1").await.unwrap_err().kind(),
            ErrorKind::Validation
        );
        assert_eq!(
            store.release("scope", " ").await.unwrap_err().kind(),
            ErrorKind::Validation
        );
    }

    #[tokio::test]
    async fn poisoned_mutex_returns_configuration_error_without_panicking() {
        let store = Arc::new(InMemoryIdempotencyStore::new());
        let poison_target = Arc::clone(&store);
        let _ = std::thread::spawn(move || {
            let _guard = poison_target.entries.lock().unwrap();
            panic!("intentional mutex poison");
        })
        .join();

        let entry = IdempotencyEntry::new("scope", "key", 60);
        let payload = serde_json::json!({"ok": true});

        assert_eq!(
            store.reserve(&entry).await.unwrap_err().kind(),
            ErrorKind::Configuration
        );
        assert_eq!(
            store.complete(&entry, &payload).await.unwrap_err().kind(),
            ErrorKind::Configuration
        );
        assert_eq!(
            store.release("scope", "key").await.unwrap_err().kind(),
            ErrorKind::Configuration
        );
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
        assert!(format!("{store:?}").contains("InMemoryIdempotencyStore"));
    }
}
