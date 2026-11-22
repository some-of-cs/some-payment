//! Deterministic routing policies, regional country/currency routing, safe
//! pre-charge failover, and composable cross-gateway pipelines.

#[cfg(feature = "country-router")]
pub mod country_router;
pub mod fallback;
#[cfg(feature = "composable-pipelines")]
pub mod pipeline;
pub mod policy;
