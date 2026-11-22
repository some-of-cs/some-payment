//! Unified multi-gateway webhook ingestion pipeline: raw-byte payload caps,
//! verify-before-DB dispatching, Moyasar verification handshake routing, and
//! monotonic deduplication guards.

pub mod dedup_guard;
pub mod dispatcher;
pub mod handshake;
pub mod raw_request;

pub use dedup_guard::{
    DEFAULT_DEDUP_CAPACITY, DEFAULT_DEDUP_TTL_SECS, InMemoryDedupStore, WebhookDeduplicationStore,
    validate_monotonic_transition, verify_order_money,
};
pub use dispatcher::{
    DEFAULT_MAX_WEBHOOK_BYTES, WebhookDispatcher, WebhookOutcome, WebhookProviderLookup,
};
pub use handshake::{
    HandshakeRouteOutcome, MAX_WEBHOOK_ID_LEN, VERIFICATION_CODE_LEN, VerificationHandshakeHandler,
    route_verification_handshake, validate_verification_challenge,
};
pub use raw_request::{
    MAX_HEADER_COUNT, MAX_QUERY_PARAM_COUNT, MAX_WEBHOOK_BYTES, ValidatedRawWebhook,
    parse_query_string,
};
