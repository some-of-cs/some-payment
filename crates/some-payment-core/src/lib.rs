//! HTTP-free domain primitives, capability bitmasks, [`PaymentProvider`] SPI,
//! and [`IdempotencyStore`] contracts for the `some-payment` ecosystem.
//!
//! This crate sits at the base of the dependency inversion graph so first-party
//! and third-party provider crates (`some-stripe`, `some-moyasar`, `some-paymob`,
//! `some-tap`, etc.) can implement [`PaymentProvider`] without depending on the
//! `some-payment` orchestrator crate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod capabilities;
pub mod error;
pub mod idempotency;
pub mod money;
pub mod provider;
pub mod provider_id;
pub mod reference;
pub mod requests;
pub mod results;

pub use capabilities::PaymentCapabilities;
pub use error::{ErrorKind, PaymentError};
pub use idempotency::{IdempotencyEntry, IdempotencyStore, ReserveOutcome};
pub use money::{CurrencyCode, MinorUnitScale, Money};
pub use provider::PaymentProvider;
pub use provider_id::ProviderId;
pub use reference::{ProviderReference, ProviderReferenceBuilder};
pub use requests::{
    CaptureMode, CaptureRequest, CheckoutSessionRequest, CreatePaymentRequest, RefundRequest,
    SplitInstruction, VoidRequest,
};
pub use results::{
    HttpMethod, NextAction, NormalizedWebhookEvent, PaymentResult, PaymentStatus,
    RawWebhookRequest, RefundResult, RefundStatus, VerificationChallenge,
};
