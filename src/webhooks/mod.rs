//! Unified multi-gateway webhook ingestion pipeline: raw-byte payload caps,
//! verify-before-DB dispatching, Moyasar verification handshake routing, and
//! monotonic deduplication guards.

pub mod dedup_guard;
pub mod dispatcher;
pub mod handshake;
pub mod raw_request;
