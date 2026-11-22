//! Open [`PaymentProvider`] Service Provider Interface (SPI) trait.

use std::fmt;

use async_trait::async_trait;

use crate::capabilities::PaymentCapabilities;
use crate::error::PaymentError;
use crate::provider_id::ProviderId;
use crate::reference::ProviderReference;
use crate::requests::{
    CaptureRequest, CheckoutSessionRequest, CreatePaymentRequest, RefundRequest, VoidRequest,
};
use crate::results::{NormalizedWebhookEvent, PaymentResult, RawWebhookRequest, RefundResult};

/// Unified async trait implemented by every first-party (`some-stripe`,
/// `some-moyasar`, `some-paymob`) and third-party (`some-tap`, etc.) payment
/// gateway adapter.
///
/// Optional lifecycle methods (`create_checkout`, `capture`, `void_payment`,
/// `refund`) fail closed by default with [`PaymentError::Unsupported`].
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use async_trait::async_trait;
/// use some_payment_core::{
///     CreatePaymentRequest, NormalizedWebhookEvent, PaymentCapabilities,
///     PaymentError, PaymentProvider, PaymentResult, ProviderId,
///     ProviderReference, RawWebhookRequest,
/// };
///
/// #[derive(Debug)]
/// struct MinimalProvider(ProviderId);
///
/// #[async_trait]
/// impl PaymentProvider for MinimalProvider {
///     fn id(&self) -> &ProviderId { &self.0 }
///     fn capabilities(&self) -> PaymentCapabilities { PaymentCapabilities::empty() }
///     async fn create_payment(&self, _: &CreatePaymentRequest) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "create_payment"))
///     }
///     async fn get_payment(&self, _: &ProviderReference) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "get_payment"))
///     }
///     async fn parse_and_verify_webhook(&self, _: &RawWebhookRequest) -> Result<NormalizedWebhookEvent, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "parse_and_verify_webhook"))
///     }
/// }
///
/// let provider: Arc<dyn PaymentProvider> = Arc::new(MinimalProvider(ProviderId::new("custom")));
/// assert_eq!(provider.id().as_str(), "custom");
/// ```
#[async_trait]
pub trait PaymentProvider: Send + Sync + fmt::Debug {
    /// Unique lowercase provider slug (e.g., `"stripe"`, `"moyasar"`, `"paymob"`, `"tap"`).
    fn id(&self) -> &ProviderId;

    /// Capability bitmask derived from provider features and runtime config.
    fn capabilities(&self) -> PaymentCapabilities;

    /// Create a direct payment or an embedded payment intent.
    async fn create_payment(
        &self,
        req: &CreatePaymentRequest,
    ) -> Result<PaymentResult, PaymentError>;

    /// Fetch authoritative payment state directly from the provider API.
    async fn get_payment(
        &self,
        reference: &ProviderReference,
    ) -> Result<PaymentResult, PaymentError>;

    /// Create a provider-hosted checkout page, session, or invoice.
    async fn create_checkout(
        &self,
        _req: &CheckoutSessionRequest,
    ) -> Result<PaymentResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "create_checkout"))
    }

    /// Capture an authorized hold (full when `amount` is `None`; partial when `Some`).
    async fn capture(&self, _req: &CaptureRequest) -> Result<PaymentResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "capture"))
    }

    /// Release an authorization hold or void an unsettled payment.
    async fn void_payment(&self, _req: &VoidRequest) -> Result<PaymentResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "void_payment"))
    }

    /// Refund a settled payment (full when `amount` is `None`; partial when `Some`).
    async fn refund(&self, _req: &RefundRequest) -> Result<RefundResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "refund"))
    }

    /// Verify authenticity over raw bytes/query params, scrub secrets, and normalize event.
    async fn parse_and_verify_webhook(
        &self,
        raw: &RawWebhookRequest,
    ) -> Result<NormalizedWebhookEvent, PaymentError>;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::error::ErrorKind;
    use crate::money::Money;
    use crate::requests::CaptureMode;

    #[derive(Debug)]
    struct DummyProvider(ProviderId);

    #[async_trait]
    impl PaymentProvider for DummyProvider {
        fn id(&self) -> &ProviderId {
            &self.0
        }

        fn capabilities(&self) -> PaymentCapabilities {
            PaymentCapabilities::empty()
        }

        async fn create_payment(
            &self,
            _req: &CreatePaymentRequest,
        ) -> Result<PaymentResult, PaymentError> {
            Err(PaymentError::unsupported(&self.0, "create_payment"))
        }

        async fn get_payment(
            &self,
            _reference: &ProviderReference,
        ) -> Result<PaymentResult, PaymentError> {
            Err(PaymentError::unsupported(&self.0, "get_payment"))
        }

        async fn parse_and_verify_webhook(
            &self,
            _raw: &RawWebhookRequest,
        ) -> Result<NormalizedWebhookEvent, PaymentError> {
            Err(PaymentError::unsupported(
                &self.0,
                "parse_and_verify_webhook",
            ))
        }
    }

    fn assert_unsupported(err: PaymentError, expected_provider: &str, expected_op: &str) {
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        assert!(matches!(
            err,
            PaymentError::Unsupported {
                ref provider,
                ref operation,
            } if provider.as_str() == expected_provider && operation == expected_op
        ));
    }

    #[tokio::test]
    async fn default_optional_methods_fail_closed_as_unsupported() {
        let provider: Arc<dyn PaymentProvider> = Arc::new(DummyProvider(ProviderId::new("tap")));
        let reference = ProviderReference::builder(ProviderId::new("tap"))
            .payment_id("pay_1")
            .build()
            .unwrap();

        let checkout_err = provider
            .create_checkout(&CheckoutSessionRequest {
                provider_id: None,
                money: Money::new(100, "KWD").unwrap(),
                order_id: "ORD-1".into(),
                success_url: "https://example.com/ok".into(),
                cancel_url: None,
                description: None,
                customer_email: None,
                customer_phone: None,
                capture_mode: CaptureMode::Automatic,
                idempotency_key: None,
                metadata: BTreeMap::new(),
            })
            .await
            .unwrap_err();
        assert_unsupported(checkout_err, "tap", "create_checkout");

        let cap_err = provider
            .capture(&CaptureRequest {
                reference: reference.clone(),
                amount: None,
                idempotency_key: None,
            })
            .await
            .unwrap_err();
        assert_unsupported(cap_err, "tap", "capture");

        let void_err = provider
            .void_payment(&VoidRequest {
                reference: reference.clone(),
                reason: None,
                idempotency_key: None,
            })
            .await
            .unwrap_err();
        assert_unsupported(void_err, "tap", "void_payment");

        let ref_err = provider
            .refund(&RefundRequest {
                reference,
                amount: Some(Money::new(100, "KWD").unwrap()),
                reason: None,
                idempotency_key: None,
            })
            .await
            .unwrap_err();
        assert_unsupported(ref_err, "tap", "refund");
    }
}
