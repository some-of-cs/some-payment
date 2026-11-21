//! Distributed [`IdempotencyStore`] contract for non-idempotent gateway mutations.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::PaymentError;

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
/// use some_payment_core::ReserveOutcome;
///
/// let outcome = ReserveOutcome::Acquired;
/// assert_eq!(outcome, ReserveOutcome::Acquired);
/// ```
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Atomically attempts to acquire a lock for `(scope, key)` with a TTL in seconds.
    async fn reserve(
        &self,
        scope: &str,
        key: &str,
        ttl_secs: u64,
    ) -> Result<ReserveOutcome, PaymentError>;

    /// Marks `(scope, key)` as completed and caches the redacted serialized response.
    async fn complete(
        &self,
        scope: &str,
        key: &str,
        payload: &serde_json::Value,
        ttl_secs: u64,
    ) -> Result<(), PaymentError>;

    /// Releases a lock strictly after a definitive pre-flight or 4xx rejection.
    ///
    /// NEVER call after a 5xx error or network timeout.
    async fn release(&self, scope: &str, key: &str) -> Result<(), PaymentError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct MockStore;

    #[async_trait]
    impl IdempotencyStore for MockStore {
        async fn reserve(
            &self,
            _scope: &str,
            key: &str,
            _ttl_secs: u64,
        ) -> Result<ReserveOutcome, PaymentError> {
            match key {
                "done" => Ok(ReserveOutcome::Completed(serde_json::json!({"ok": true}))),
                "busy" => Ok(ReserveOutcome::InProgress),
                _ => Ok(ReserveOutcome::Acquired),
            }
        }

        async fn complete(
            &self,
            _scope: &str,
            _key: &str,
            _payload: &serde_json::Value,
            _ttl_secs: u64,
        ) -> Result<(), PaymentError> {
            Ok(())
        }

        async fn release(&self, _scope: &str, _key: &str) -> Result<(), PaymentError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_across_all_outcomes() {
        let store: Arc<dyn IdempotencyStore> = Arc::new(MockStore);
        assert_eq!(
            store.reserve("refund", "new", 60).await.unwrap(),
            ReserveOutcome::Acquired
        );
        assert_eq!(
            store.reserve("refund", "busy", 60).await.unwrap(),
            ReserveOutcome::InProgress
        );
        assert!(matches!(
            store.reserve("refund", "done", 60).await.unwrap(),
            ReserveOutcome::Completed(_)
        ));
    }
}
