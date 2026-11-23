//! Composable cross-gateway pipelines pinning distinct providers per capability domain.
//!
//! [`ComposablePipeline`] lets applications mix and match registered [`PaymentProvider`]
//! adapters by capability domain (for example, direct payments via one gateway,
//! hosted checkout via a regional gateway, and refunds via another) while failing
//! closed at build time whenever a pinned provider is unregistered (`AC-4`, `TM-6`)
//! or lacks the required [`PaymentCapabilities`] (`TM-1`).

use std::sync::Arc;

use some_payment_core::{PaymentCapabilities, PaymentError, PaymentProvider, ProviderId};

use crate::registry::ProviderRegistry;

/// Internal lookup seam allowing build-time and runtime validation to run
/// against both [`ProviderRegistry`] and isolated unit-test registries.
trait ProviderLookup {
    fn lookup_provider(&self, id: &ProviderId) -> Result<Arc<dyn PaymentProvider>, PaymentError>;

    fn lookup_default_provider(&self) -> Arc<dyn PaymentProvider>;
}

impl ProviderLookup for ProviderRegistry {
    fn lookup_provider(&self, id: &ProviderId) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        self.require(id)
    }

    fn lookup_default_provider(&self) -> Arc<dyn PaymentProvider> {
        self.default_provider()
    }
}

fn verify_capability(
    provider: &dyn PaymentProvider,
    required: PaymentCapabilities,
    op: &str,
) -> Result<(), PaymentError> {
    if !provider.capabilities().contains(required) {
        return Err(PaymentError::unsupported(provider.id(), op));
    }
    Ok(())
}

fn validate_pin<R: ProviderLookup>(
    pin: Option<&ProviderId>,
    required: PaymentCapabilities,
    op: &str,
    registry: &R,
) -> Result<(), PaymentError> {
    let Some(provider_id) = pin else {
        return Ok(());
    };
    let provider = registry.lookup_provider(provider_id)?;
    verify_capability(provider.as_ref(), required, op)
}

/// Immutable cross-gateway pipeline pinning distinct providers per capability domain.
///
/// Unpinned domains fall back to [`ProviderRegistry::default_provider`], and every
/// resolved provider is verified to advertise the requested [`PaymentCapabilities`]
/// before dispatch (`TM-1`, `TM-6`).
///
/// # Examples
///
/// ```
/// use some_payment::routing::pipeline::ComposablePipeline;
/// use some_payment_core::ProviderId;
///
/// let builder = ComposablePipeline::builder()
///     .direct_payments(ProviderId::new("stripe"))
///     .hosted_checkout(ProviderId::new("paymob"))
///     .refunds(ProviderId::new("moyasar"));
///
/// let _ = builder;
/// ```
///
/// ```no_run
/// use some_payment::registry::ProviderRegistry;
/// use some_payment::routing::pipeline::ComposablePipeline;
/// use some_payment_core::{PaymentCapabilities, PaymentError, ProviderId};
///
/// fn configure(registry: &ProviderRegistry) -> Result<(), PaymentError> {
///     let pipeline = ComposablePipeline::builder()
///         .direct_payments(ProviderId::new("stripe"))
///         .hosted_checkout(ProviderId::new("paymob"))
///         .refunds(ProviderId::new("moyasar"))
///         .build(registry)?;
///
///     let checkout =
///         pipeline.resolve_for_capability(PaymentCapabilities::HOSTED_CHECKOUT, registry)?;
///     assert_eq!(checkout.id().as_str(), "paymob");
///     Ok(())
/// }
/// ```
///
/// Pinned provider fields are private and cannot be mutated after build:
///
/// ```compile_fail
/// use some_payment::registry::ProviderRegistry;
/// use some_payment::routing::pipeline::ComposablePipeline;
/// use some_payment_core::{PaymentError, ProviderId};
///
/// fn mutate_after_build(registry: &ProviderRegistry) -> Result<(), PaymentError> {
///     let mut pipeline = ComposablePipeline::builder()
///         .direct_payments(ProviderId::new("stripe"))
///         .build(registry)?;
///     pipeline.direct_payments = Some(ProviderId::new("paymob"));
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposablePipeline {
    direct_payments: Option<ProviderId>,
    hosted_checkout: Option<ProviderId>,
    captures: Option<ProviderId>,
    voids: Option<ProviderId>,
    refunds: Option<ProviderId>,
}

impl ComposablePipeline {
    /// Creates a new [`ComposablePipelineBuilder`] with no capability pins configured.
    #[must_use]
    pub fn builder() -> ComposablePipelineBuilder {
        ComposablePipelineBuilder::new()
    }

    /// Returns the provider pinned for [`PaymentCapabilities::DIRECT_PAYMENT`], if any.
    #[must_use]
    pub fn direct_payments_provider(&self) -> Option<&ProviderId> {
        self.direct_payments.as_ref()
    }

    /// Returns the provider pinned for [`PaymentCapabilities::HOSTED_CHECKOUT`], if any.
    #[must_use]
    pub fn hosted_checkout_provider(&self) -> Option<&ProviderId> {
        self.hosted_checkout.as_ref()
    }

    /// Returns the provider pinned for [`PaymentCapabilities::AUTH_CAPTURE`], if any.
    #[must_use]
    pub fn captures_provider(&self) -> Option<&ProviderId> {
        self.captures.as_ref()
    }

    /// Returns the provider pinned for [`PaymentCapabilities::VOID_AUTHORIZED`], if any.
    #[must_use]
    pub fn voids_provider(&self) -> Option<&ProviderId> {
        self.voids.as_ref()
    }

    /// Returns the provider pinned for [`PaymentCapabilities::REFUND`], if any.
    #[must_use]
    pub fn refunds_provider(&self) -> Option<&ProviderId> {
        self.refunds.as_ref()
    }

    /// Resolves the [`PaymentProvider`] responsible for the requested `required` capability.
    ///
    /// Maps `required` to its pinned capability domain when configured, or falls back to
    /// [`ProviderRegistry::default_provider`]. Both paths fail closed if the provider is
    /// missing from `registry` (`TM-6`) or does not claim `required` (`TM-1`).
    ///
    /// # Errors
    ///
    /// - Returns [`PaymentError::ConfigurationError`] if a pinned provider is not registered
    ///   in `registry`.
    /// - Returns [`PaymentError::Unsupported`] if the resolved provider does not advertise
    ///   all bits in `required`.
    pub fn resolve_for_capability(
        &self,
        required: PaymentCapabilities,
        registry: &ProviderRegistry,
    ) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        self.resolve_for_capability_with(required, registry)
    }

    fn pinned_for(&self, required: PaymentCapabilities) -> (Option<&ProviderId>, &'static str) {
        let domains: [(PaymentCapabilities, Option<&ProviderId>, &'static str); 5] = [
            (
                PaymentCapabilities::DIRECT_PAYMENT,
                self.direct_payments.as_ref(),
                "direct_payments",
            ),
            (
                PaymentCapabilities::HOSTED_CHECKOUT,
                self.hosted_checkout.as_ref(),
                "hosted_checkout",
            ),
            (
                PaymentCapabilities::AUTH_CAPTURE,
                self.captures.as_ref(),
                "captures",
            ),
            (
                PaymentCapabilities::VOID_AUTHORIZED,
                self.voids.as_ref(),
                "voids",
            ),
            (
                PaymentCapabilities::REFUND,
                self.refunds.as_ref(),
                "refunds",
            ),
        ];

        for (flag, pin, op) in domains {
            if required.contains(flag) {
                return (pin, op);
            }
        }
        (None, "capability_operation")
    }

    fn resolve_for_capability_with<R: ProviderLookup>(
        &self,
        required: PaymentCapabilities,
        registry: &R,
    ) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        let (pin, op) = self.pinned_for(required);
        let provider = match pin {
            Some(provider_id) => registry.lookup_provider(provider_id)?,
            None => registry.lookup_default_provider(),
        };
        verify_capability(provider.as_ref(), required, op)?;
        Ok(provider)
    }
}

/// Builder for [`ComposablePipeline`] with fail-closed build-time validation.
///
/// Every pinned [`ProviderId`] is validated in [`Self::build`] against a
/// [`ProviderRegistry`] to guarantee registration (`AC-4`, `TM-6`) and capability
/// compatibility (`TM-1`) before any payment request is executed.
///
/// # Examples
///
/// ```
/// use some_payment::routing::pipeline::ComposablePipelineBuilder;
/// use some_payment_core::ProviderId;
///
/// let builder = ComposablePipelineBuilder::new()
///     .direct_payments(ProviderId::new("stripe"))
///     .hosted_checkout(ProviderId::new("paymob"))
///     .captures(ProviderId::new("stripe"))
///     .voids(ProviderId::new("stripe"))
///     .refunds(ProviderId::new("moyasar"));
///
/// let _ = builder;
/// ```
#[derive(Clone, Debug, Default)]
pub struct ComposablePipelineBuilder {
    direct_payments: Option<ProviderId>,
    hosted_checkout: Option<ProviderId>,
    captures: Option<ProviderId>,
    voids: Option<ProviderId>,
    refunds: Option<ProviderId>,
}

impl ComposablePipelineBuilder {
    /// Creates an empty [`ComposablePipelineBuilder`] with all domains unpinned.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins `provider_id` for direct payments ([`PaymentCapabilities::DIRECT_PAYMENT`]).
    #[must_use]
    pub fn direct_payments(mut self, provider_id: ProviderId) -> Self {
        self.direct_payments = Some(provider_id);
        self
    }

    /// Pins `provider_id` for hosted checkout ([`PaymentCapabilities::HOSTED_CHECKOUT`]).
    #[must_use]
    pub fn hosted_checkout(mut self, provider_id: ProviderId) -> Self {
        self.hosted_checkout = Some(provider_id);
        self
    }

    /// Pins `provider_id` for capture operations ([`PaymentCapabilities::AUTH_CAPTURE`]).
    #[must_use]
    pub fn captures(mut self, provider_id: ProviderId) -> Self {
        self.captures = Some(provider_id);
        self
    }

    /// Pins `provider_id` for void operations ([`PaymentCapabilities::VOID_AUTHORIZED`]).
    #[must_use]
    pub fn voids(mut self, provider_id: ProviderId) -> Self {
        self.voids = Some(provider_id);
        self
    }

    /// Pins `provider_id` for refund operations ([`PaymentCapabilities::REFUND`]).
    #[must_use]
    pub fn refunds(mut self, provider_id: ProviderId) -> Self {
        self.refunds = Some(provider_id);
        self
    }

    /// Validates all pinned providers against `registry` and constructs a [`ComposablePipeline`].
    ///
    /// Fails closed at build time if any pinned provider is not registered in `registry`
    /// (`AC-4`, `TM-6`) or does not advertise the required capability bit (`TM-1`).
    ///
    /// # Errors
    ///
    /// - Returns [`PaymentError::ConfigurationError`] if any pinned [`ProviderId`] is missing
    ///   from `registry`.
    /// - Returns [`PaymentError::Unsupported`] if any pinned provider lacks the required
    ///   [`PaymentCapabilities`] for its pinned domain.
    pub fn build(self, registry: &ProviderRegistry) -> Result<ComposablePipeline, PaymentError> {
        self.build_with(registry)
    }

    fn build_with<R: ProviderLookup>(
        self,
        registry: &R,
    ) -> Result<ComposablePipeline, PaymentError> {
        let checks = [
            (
                self.direct_payments.as_ref(),
                PaymentCapabilities::DIRECT_PAYMENT,
                "direct_payments",
            ),
            (
                self.hosted_checkout.as_ref(),
                PaymentCapabilities::HOSTED_CHECKOUT,
                "hosted_checkout",
            ),
            (
                self.captures.as_ref(),
                PaymentCapabilities::AUTH_CAPTURE,
                "captures",
            ),
            (
                self.voids.as_ref(),
                PaymentCapabilities::VOID_AUTHORIZED,
                "voids",
            ),
            (
                self.refunds.as_ref(),
                PaymentCapabilities::REFUND,
                "refunds",
            ),
        ];

        for (pin, required, op) in checks {
            validate_pin(pin, required, op, registry)?;
        }

        Ok(ComposablePipeline {
            direct_payments: self.direct_payments,
            hosted_checkout: self.hosted_checkout,
            captures: self.captures,
            voids: self.voids,
            refunds: self.refunds,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, ErrorKind, NormalizedWebhookEvent, PaymentCapabilities, PaymentError,
        PaymentProvider, PaymentResult, ProviderId, ProviderReference, RawWebhookRequest,
    };

    use super::{ComposablePipeline, ComposablePipelineBuilder, ProviderLookup};

    #[derive(Debug)]
    struct MockProvider {
        id: ProviderId,
        capabilities: PaymentCapabilities,
    }

    impl MockProvider {
        fn new(id: &str, capabilities: PaymentCapabilities) -> Arc<dyn PaymentProvider> {
            Arc::new(Self {
                id: ProviderId::new(id),
                capabilities,
            })
        }
    }

    #[async_trait]
    impl PaymentProvider for MockProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn capabilities(&self) -> PaymentCapabilities {
            self.capabilities
        }

        async fn create_payment(
            &self,
            _req: &CreatePaymentRequest,
        ) -> Result<PaymentResult, PaymentError> {
            Err(PaymentError::unsupported(&self.id, "create_payment"))
        }

        async fn get_payment(
            &self,
            _reference: &ProviderReference,
        ) -> Result<PaymentResult, PaymentError> {
            Err(PaymentError::unsupported(&self.id, "get_payment"))
        }

        async fn parse_and_verify_webhook(
            &self,
            _raw: &RawWebhookRequest,
        ) -> Result<NormalizedWebhookEvent, PaymentError> {
            Err(PaymentError::unsupported(
                &self.id,
                "parse_and_verify_webhook",
            ))
        }
    }

    struct TestRegistry {
        providers: HashMap<ProviderId, Arc<dyn PaymentProvider>>,
        default_id: ProviderId,
    }

    impl TestRegistry {
        fn new(providers: Vec<Arc<dyn PaymentProvider>>, default_id: &str) -> Self {
            let map = providers
                .into_iter()
                .map(|p| (p.id().clone(), p))
                .collect::<HashMap<_, _>>();
            Self {
                providers: map,
                default_id: ProviderId::new(default_id),
            }
        }
    }

    impl ProviderLookup for TestRegistry {
        fn lookup_provider(
            &self,
            id: &ProviderId,
        ) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
            self.providers.get(id).cloned().ok_or_else(|| {
                PaymentError::configuration(format!("provider `{id}` is not registered"))
            })
        }

        fn lookup_default_provider(&self) -> Arc<dyn PaymentProvider> {
            self.providers
                .get(&self.default_id)
                .cloned()
                .expect("default provider must be registered in TestRegistry")
        }
    }

    fn multi_provider_registry() -> TestRegistry {
        TestRegistry::new(
            vec![
                MockProvider::new(
                    "stripe",
                    PaymentCapabilities::DIRECT_PAYMENT
                        | PaymentCapabilities::AUTH_CAPTURE
                        | PaymentCapabilities::VOID_AUTHORIZED,
                ),
                MockProvider::new("paymob", PaymentCapabilities::HOSTED_CHECKOUT),
                MockProvider::new("moyasar", PaymentCapabilities::REFUND),
            ],
            "stripe",
        )
    }

    #[test]
    fn tc_2_2_4_valid_multi_provider_pinning_succeeds() {
        let registry = multi_provider_registry();
        let pipeline = ComposablePipeline::builder()
            .direct_payments(ProviderId::new("stripe"))
            .hosted_checkout(ProviderId::new("paymob"))
            .captures(ProviderId::new("stripe"))
            .voids(ProviderId::new("stripe"))
            .refunds(ProviderId::new("moyasar"))
            .build_with(&registry)
            .expect("all pinned providers are registered and capable");

        assert_eq!(
            pipeline.direct_payments_provider(),
            Some(&ProviderId::new("stripe"))
        );
        assert_eq!(
            pipeline.hosted_checkout_provider(),
            Some(&ProviderId::new("paymob"))
        );
        assert_eq!(
            pipeline.captures_provider(),
            Some(&ProviderId::new("stripe"))
        );
        assert_eq!(pipeline.voids_provider(), Some(&ProviderId::new("stripe")));
        assert_eq!(
            pipeline.refunds_provider(),
            Some(&ProviderId::new("moyasar"))
        );
    }

    #[test]
    fn tc_2_2_4_ac_4_tm_6_unregistered_provider_fails_closed_at_build() {
        let registry = multi_provider_registry();
        let unknown = ProviderId::new("unknown_gateway");

        let builders = [
            ComposablePipeline::builder().direct_payments(unknown.clone()),
            ComposablePipeline::builder().hosted_checkout(unknown.clone()),
            ComposablePipeline::builder().captures(unknown.clone()),
            ComposablePipeline::builder().voids(unknown.clone()),
            ComposablePipeline::builder().refunds(unknown),
        ];

        for builder in builders {
            let err = builder
                .build_with(&registry)
                .expect_err("unregistered provider pin must fail closed at build time");
            assert_eq!(err.kind(), ErrorKind::Configuration);
        }
    }

    #[test]
    fn tc_2_2_4_tm_1_missing_capability_fails_closed_at_build() {
        let registry = multi_provider_registry();

        let cases = [
            (
                ComposablePipeline::builder().direct_payments(ProviderId::new("paymob")),
                "paymob",
                "direct_payments",
            ),
            (
                ComposablePipeline::builder().hosted_checkout(ProviderId::new("stripe")),
                "stripe",
                "hosted_checkout",
            ),
            (
                ComposablePipeline::builder().captures(ProviderId::new("paymob")),
                "paymob",
                "captures",
            ),
            (
                ComposablePipeline::builder().voids(ProviderId::new("moyasar")),
                "moyasar",
                "voids",
            ),
            (
                ComposablePipeline::builder().refunds(ProviderId::new("stripe")),
                "stripe",
                "refunds",
            ),
        ];

        for (builder, expected_provider, expected_op) in cases {
            let err = builder
                .build_with(&registry)
                .expect_err("provider lacking required capability must fail closed at build time");
            assert_eq!(err.kind(), ErrorKind::Unsupported);
            assert!(matches!(
                err,
                PaymentError::Unsupported {
                    ref provider,
                    ref operation,
                } if provider.as_str() == expected_provider && operation == expected_op
            ));
        }
    }

    #[test]
    fn resolve_for_capability_returns_pinned_provider_per_domain() {
        let registry = multi_provider_registry();
        let pipeline = ComposablePipeline::builder()
            .direct_payments(ProviderId::new("stripe"))
            .hosted_checkout(ProviderId::new("paymob"))
            .captures(ProviderId::new("stripe"))
            .voids(ProviderId::new("stripe"))
            .refunds(ProviderId::new("moyasar"))
            .build_with(&registry)
            .expect("valid pipeline");

        let checks = [
            (PaymentCapabilities::DIRECT_PAYMENT, "stripe"),
            (PaymentCapabilities::HOSTED_CHECKOUT, "paymob"),
            (PaymentCapabilities::AUTH_CAPTURE, "stripe"),
            (PaymentCapabilities::VOID_AUTHORIZED, "stripe"),
            (PaymentCapabilities::REFUND, "moyasar"),
        ];

        for (capability, expected_id) in checks {
            let resolved = pipeline
                .resolve_for_capability_with(capability, &registry)
                .expect("pinned domain should resolve");
            assert_eq!(resolved.id().as_str(), expected_id);
        }
    }

    #[test]
    fn resolve_for_capability_falls_back_to_default_and_verifies_capability() {
        let registry = multi_provider_registry();
        let pipeline = ComposablePipelineBuilder::new()
            .build_with(&registry)
            .expect("empty pipeline is valid");

        let direct = pipeline
            .resolve_for_capability_with(PaymentCapabilities::DIRECT_PAYMENT, &registry)
            .expect("default provider supports DIRECT_PAYMENT");
        assert_eq!(direct.id().as_str(), "stripe");

        let err = pipeline
            .resolve_for_capability_with(PaymentCapabilities::HOSTED_CHECKOUT, &registry)
            .expect_err("default provider lacks HOSTED_CHECKOUT and must fail closed");
        assert_eq!(err.kind(), ErrorKind::Unsupported);
    }

    #[test]
    fn resolve_for_capability_enforces_tm_6_and_tm_1_at_runtime() {
        let build_registry = multi_provider_registry();
        let pipeline = ComposablePipeline::builder()
            .hosted_checkout(ProviderId::new("paymob"))
            .refunds(ProviderId::new("moyasar"))
            .build_with(&build_registry)
            .expect("valid pipeline");

        // TM-6: swapped runtime registry missing the pinned provider fails closed.
        let narrowed_registry = TestRegistry::new(
            vec![MockProvider::new(
                "stripe",
                PaymentCapabilities::DIRECT_PAYMENT,
            )],
            "stripe",
        );
        let missing_err = pipeline
            .resolve_for_capability_with(PaymentCapabilities::HOSTED_CHECKOUT, &narrowed_registry)
            .expect_err("missing pinned provider in runtime registry must fail closed");
        assert_eq!(missing_err.kind(), ErrorKind::Configuration);

        // TM-1: compound capability mask fails closed when provider lacks PARTIAL_REFUND.
        let compound_err = pipeline
            .resolve_for_capability_with(
                PaymentCapabilities::REFUND | PaymentCapabilities::PARTIAL_REFUND,
                &build_registry,
            )
            .expect_err("compound capability check must fail closed when any bit is missing");
        assert_eq!(compound_err.kind(), ErrorKind::Unsupported);
    }

    #[test]
    fn pipeline_derives_clone_debug_and_eq() {
        let registry = multi_provider_registry();
        let first = ComposablePipeline::builder()
            .direct_payments(ProviderId::new("stripe"))
            .build_with(&registry)
            .expect("valid pipeline");
        let second = first.clone();
        let builder_debug = format!("{:?}", ComposablePipelineBuilder::default());

        assert_eq!(first, second);
        assert!(format!("{first:?}").contains("ComposablePipeline"));
        assert!(builder_debug.contains("ComposablePipelineBuilder"));
    }
}
