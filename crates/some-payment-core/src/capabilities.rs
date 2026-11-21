//! Fail-closed [`PaymentCapabilities`] bitmask for gateway feature discovery.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Bitmask declaring the exact capabilities supported by a [`crate::PaymentProvider`].
    ///
    /// Capabilities are fail-closed by default (`PaymentCapabilities::empty()`):
    /// `some-payment` rejects any operation whose required capability bit is not
    /// explicitly claimed before making outbound network requests.
    ///
    /// # Examples
    ///
    /// ```
    /// use some_payment_core::PaymentCapabilities;
    ///
    /// let hosted_only = PaymentCapabilities::HOSTED_CHECKOUT
    ///     | PaymentCapabilities::REFUND
    ///     | PaymentCapabilities::PARTIAL_REFUND;
    /// assert!(hosted_only.contains(PaymentCapabilities::HOSTED_CHECKOUT));
    /// assert!(!hosted_only.contains(PaymentCapabilities::AUTH_CAPTURE));
    /// ```
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct PaymentCapabilities: u32 {
        /// Direct server-side payment creation (e.g., tokenized card or wallet).
        const DIRECT_PAYMENT = 1 << 0;
        /// Embedded payment intent returning a client secret (Stripe PI, Moyasar PI, Paymob Pixel).
        const EMBEDDED_INTENT = 1 << 1;
        /// Provider-hosted checkout session, unified checkout page, or invoice URL.
        const HOSTED_CHECKOUT = 1 << 2;
        /// Two-step authorize-then-capture (`CaptureMode::Manual`).
        const AUTH_CAPTURE = 1 << 3;
        /// Partial capture of an authorized hold.
        const PARTIAL_CAPTURE = 1 << 4;
        /// Voiding / releasing an uncaptured authorization hold.
        const VOID_AUTHORIZED = 1 << 5;
        /// Voiding an unsettled paid/captured transaction within a gateway window.
        const VOID_SETTLED_WINDOW = 1 << 6;
        /// Full refund of a settled payment.
        const REFUND = 1 << 7;
        /// Partial refund (and multiple partial refunds) of a settled payment.
        const PARTIAL_REFUND = 1 << 8;
        /// Native provider idempotency on payment creation (Stripe header, Moyasar `given_id`).
        const NATIVE_CREATE_IDEMPOTENCY = 1 << 9;
        /// Native provider idempotency on post-payment mutations (Stripe only; false for Moyasar/Paymob).
        const NATIVE_MUTATION_IDEMPOTENCY = 1 << 10;
        /// Saved card tokenization and recurring charges.
        const SAVED_CARDS = 1 << 11;
        /// Multi-party marketplace split settlements.
        const SPLITS = 1 << 12;
        /// Provider requires a webhook verification handshake (e.g., Moyasar `webhook_verification`).
        const WEBHOOK_VERIFICATION_HANDSHAKE = 1 << 13;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_empty_and_bitmask_operations_work() {
        let empty = PaymentCapabilities::default();
        assert!(empty.is_empty());

        let moyasar = PaymentCapabilities::DIRECT_PAYMENT
            | PaymentCapabilities::EMBEDDED_INTENT
            | PaymentCapabilities::HOSTED_CHECKOUT
            | PaymentCapabilities::AUTH_CAPTURE
            | PaymentCapabilities::REFUND
            | PaymentCapabilities::PARTIAL_REFUND
            | PaymentCapabilities::NATIVE_CREATE_IDEMPOTENCY
            | PaymentCapabilities::WEBHOOK_VERIFICATION_HANDSHAKE;

        assert!(moyasar.contains(PaymentCapabilities::NATIVE_CREATE_IDEMPOTENCY));
        assert!(!moyasar.contains(PaymentCapabilities::NATIVE_MUTATION_IDEMPOTENCY));
    }
}
