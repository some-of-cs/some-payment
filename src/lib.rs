//! Provider-agnostic payment orchestrator, thread-safe provider registry,
//! composable lifecycle hooks, and deterministic routing engine for the
//! `some-payment` ecosystem.
//!
//! This crate depends strictly on [`some_payment_core`] and contains zero
//! references to concrete payment gateway implementations.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub use some_payment_core;

pub mod builder;
pub mod hooks;
pub mod orchestrator;
pub mod registry;
pub mod routing;
