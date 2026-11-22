//! Distributed [`some_payment_core::IdempotencyStore`] execution boundary,
//! thread-safe in-memory store for development/tests, and indeterminate-timeout
//! reconciliation helpers.

pub mod boundary;
pub mod memory_store;
pub mod reconciliation;
pub mod scope_keys;

pub use boundary::{DEFAULT_LOCK_TTL_SECS, IdempotencyBoundary};
pub use memory_store::{DEFAULT_MAX_CAPACITY, InMemoryIdempotencyStore};
pub use reconciliation::{
    ReconciliationKind, ReconciliationOutcome, ReconciliationRequest,
    reconcile_indeterminate_mutation,
};
pub use scope_keys::{
    IdempotencyScope, MAX_IDEMPOTENCY_KEY_LEN, MAX_SCOPE_SEGMENT_LEN, validate_idempotency_key,
    validate_uuid_v4,
};
