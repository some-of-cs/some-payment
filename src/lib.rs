//! Provider-agnostic distributed idempotency boundary and unified webhook
//! pipeline for the `some-payment` ecosystem.
//!
//! This crate depends strictly on [`some_payment_core`] and contains zero
//! references to concrete payment gateway implementations.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub use some_payment_core;

pub mod idempotency;
pub mod webhooks;
