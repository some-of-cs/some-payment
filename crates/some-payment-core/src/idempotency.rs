//! Distributed [`IdempotencyStore`] contract for non-idempotent gateway mutations.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::PaymentError;

/// Scoped idempotency lock descriptor passed to [`IdempotencyStore`] operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdempotencyEntry<'a> {
    /// Namespaced scope (e.g., `"tenant-1:moyasar:refund"`).
    pub scope: &'a str,
    /// Caller-supplied idempotency key.
    pub key: &'a str,
    /// Lock or cached-response TTL in seconds.
    pub ttl_secs: u64,
}

impl<'a> IdempotencyEntry<'a> {
    /// Creates a new [`IdempotencyEntry`].
    pub const fn new(scope: &'a str, key: &'a str, ttl_secs: u64) -> Self {
        Self {
            scope,
            key,
            ttl_secs,
        }
    }
}

/// Atomic lock reservation outcome returned by [`IdempotencyStore::reserve`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReserveOutcome {
    /// Caller acquired the atomic lock and may invoke the provider mutation.
    Acquired,
    /// Earlier execution finished; return the cached serialized response.
    Completed(serde_json::Value),
    /// Another worker holds the active lock; fail with [`PaymentError::IdempotencyConflict`].
    InProgress,
}

/// Pluggable distributed idempotency store (backed by Redis `SET NX EX` or
/// PostgreSQL `INSERT ... ON CONFLICT` in production).
///
/// # Lock-Retention Safety Invariant
///
/// Callers MUST invoke [`IdempotencyStore::release`] **only** on definitive
/// pre-flight or 4xx client rejections — **never** on 5xx errors or network
/// timeouts, where a non-idempotent gateway (such as Moyasar or Paymob) may
/// have already executed the capture, void, or refund.
///
/// # Examples
///
/// ```
/// use some_payment_core::{IdempotencyEntry, ReserveOutcome};
///
/// let entry = IdempotencyEntry::new("moyasar:refund", "key-1", 300);
/// assert_eq!(entry.ttl_secs, 300);
/// assert_eq!(ReserveOutcome::Acquired, ReserveOutcome::Acquired);
/// ```
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Atomically attempts to acquire a lock for `entry`.
    async fn reserve(&self, entry: &IdempotencyEntry<'_>) -> Result<ReserveOutcome, PaymentError>;

    /// Marks `entry` as completed and caches the redacted serialized response.
    async fn complete(
        &self,
        entry: &IdempotencyEntry<'_>,
        payload: &serde_json::Value,
    ) -> Result<(), PaymentError>;

    /// Releases a lock strictly after a definitive pre-flight or 4xx rejection.
    ///
    /// NEVER call after a 5xx error or network timeout.
    async fn release(&self, scope: &str, key: &str) -> Result<(), PaymentError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct MockStore {
        completed_calls: AtomicUsize,
        released_calls: AtomicUsize,
    }

    #[async_trait]
    impl IdempotencyStore for MockStore {
        async fn reserve(
            &self,
            entry: &IdempotencyEntry<'_>,
        ) -> Result<ReserveOutcome, PaymentError> {
            match entry.key {
                "done" => Ok(ReserveOutcome::Completed(serde_json::json!({"ok": true}))),
                "busy" => Ok(ReserveOutcome::InProgress),
                _ => Ok(ReserveOutcome::Acquired),
            }
        }

        async fn complete(
            &self,
            _entry: &IdempotencyEntry<'_>,
            _payload: &serde_json::Value,
        ) -> Result<(), PaymentError> {
            self.completed_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn release(&self, _scope: &str, _key: &str) -> Result<(), PaymentError> {
            self.released_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_across_all_outcomes_and_mutations() {
        let mock = Arc::new(MockStore::default());
        let store: Arc<dyn IdempotencyStore> = mock.clone();
        let new_entry = IdempotencyEntry::new("refund", "new", 60);
        let busy_entry = IdempotencyEntry::new("refund", "busy", 60);
        let done_entry = IdempotencyEntry::new("refund", "done", 60);

        assert_eq!(
            store.reserve(&new_entry).await.unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(
            store.reserve(&busy_entry).await.unwrap(),
            ReserveOutcome::InProgress
        );
        assert!(matches!(
            store.reserve(&done_entry).await.unwrap(),
            ReserveOutcome::Completed(_)
        ));

        store
            .complete(&new_entry, &serde_json::json!({"status": "succeeded"}))
            .await
            .unwrap();
        store.release("refund", "new").await.unwrap();

        assert_eq!(mock.completed_calls.load(Ordering::SeqCst), 1);
        assert_eq!(mock.released_calls.load(Ordering::SeqCst), 1);
    }
}
