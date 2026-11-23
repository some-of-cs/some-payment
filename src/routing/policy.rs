//! Deterministic routing policy trait, routing context, default-only policy,
//! and provider resolution helper (`US-2.2.0` / Unit 1).
//!
//! Routing policies are pure and deterministic with zero network I/O. Every
//! resolved provider is verified against [`ProviderRegistry`] (`TM-6`) and
//! checked for required [`PaymentCapabilities`] (`TM-1`), while explicit
//! per-call `provider_id` overrides always take absolute precedence (`AC-2`).

use std::fmt;
use std::sync::Arc;

use some_payment_core::{
    CurrencyCode, PaymentCapabilities, PaymentError, PaymentProvider, ProviderId,
};

use crate::builder::PaymentOrchestratorBuilder;
use crate::orchestrator::RouteResolver;
use crate::registry::ProviderRegistry;

/// Immutable routing inputs supplied to a [`RoutingPolicy`] and
/// [`resolve_provider`].
///
/// # Examples
///
/// ```
/// use some_payment::routing::policy::RoutingContext;
/// use some_payment_core::{CurrencyCode, PaymentCapabilities, ProviderId};
///
/// let ctx = RoutingContext::new()
///     .with_explicit_provider(ProviderId::new("stripe"))
///     .with_currency(CurrencyCode::try_new("SAR").unwrap())
///     .with_buyer_country("SA")
///     .with_required_capabilities(PaymentCapabilities::DIRECT_PAYMENT);
///
/// assert_eq!(
///     ctx.explicit_provider.as_ref().map(ProviderId::as_str),
///     Some("stripe"),
/// );
/// assert_eq!(ctx.currency.as_ref().map(CurrencyCode::as_str), Some("SAR"));
/// assert_eq!(ctx.buyer_country.as_deref(), Some("SA"));
/// assert!(
///     ctx.required_capabilities
///         .contains(PaymentCapabilities::DIRECT_PAYMENT)
/// );
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoutingContext {
    /// Explicit per-call provider override; always takes absolute precedence
    /// over any [`RoutingPolicy`] decision (`AC-2`).
    pub explicit_provider: Option<ProviderId>,
    /// Optional ISO-4217 currency code for the payment operation.
    pub currency: Option<CurrencyCode>,
    /// Optional ISO-3166-1 alpha-2 buyer country hint (e.g., `"SA"`, `"EG"`, `"US"`).
    pub buyer_country: Option<String>,
    /// Capability bitmask required by the requested operation (`TM-1`).
    pub required_capabilities: PaymentCapabilities,
}

impl RoutingContext {
    /// Creates an empty [`RoutingContext`] with no explicit override, currency,
    /// country hint, or required capabilities.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an explicit per-call [`ProviderId`] override.
    pub fn with_explicit_provider(mut self, id: ProviderId) -> Self {
        self.explicit_provider = Some(id);
        self
    }

    /// Sets the ISO-4217 [`CurrencyCode`] hint for routing evaluation.
    pub fn with_currency(mut self, currency: CurrencyCode) -> Self {
        self.currency = Some(currency);
        self
    }

    /// Sets the buyer country hint (e.g., `"SA"`, `"EG"`, `"US"`).
    pub fn with_buyer_country(mut self, country: impl Into<String>) -> Self {
        self.buyer_country = Some(country.into());
        self
    }

    /// Sets the required [`PaymentCapabilities`] bitmask enforced on the
    /// resolved provider.
    pub fn with_required_capabilities(mut self, capabilities: PaymentCapabilities) -> Self {
        self.required_capabilities = capabilities;
        self
    }
}

/// Pure, deterministic provider selection policy.
///
/// Implementations must perform zero network I/O and rely strictly on the
/// supplied [`RoutingContext`] and [`ProviderRegistry`].
pub trait RoutingPolicy: Send + Sync + fmt::Debug {
    /// Selects the target [`ProviderId`] for the given [`RoutingContext`] and
    /// [`ProviderRegistry`].
    fn select(
        &self,
        ctx: &RoutingContext,
        registry: &ProviderRegistry,
    ) -> Result<ProviderId, PaymentError>;
}

/// Minimal [`RoutingPolicy`] that honors a registered explicit override and
/// otherwise always routes to [`ProviderRegistry::default_id`].
///
/// # Examples
///
/// ```
/// use some_payment::routing::policy::DefaultOnlyPolicy;
///
/// let policy = DefaultOnlyPolicy;
/// let copied = policy;
/// assert_eq!(format!("{copied:?}"), "DefaultOnlyPolicy");
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultOnlyPolicy;

impl RoutingPolicy for DefaultOnlyPolicy {
    fn select(
        &self,
        ctx: &RoutingContext,
        registry: &ProviderRegistry,
    ) -> Result<ProviderId, PaymentError> {
        let Some(id) = ctx.explicit_provider.as_ref() else {
            return Ok(registry.default_id().clone());
        };
        if !registry.contains(id) {
            return Err(PaymentError::configuration(
                "explicit routing provider is not registered",
            ));
        }
        Ok(id.clone())
    }
}

/// Resolves a concrete [`PaymentProvider`] using `policy`, `ctx`, and
/// `registry` while enforcing all routing and security invariants:
///
/// - `AC-2`: When [`RoutingContext::explicit_provider`] is `Some(id)`, it
///   always takes absolute precedence over `policy.select(...)`.
/// - `TM-6`: The selected [`ProviderId`] is verified against `registry` via
///   [`ProviderRegistry::require`].
/// - `TM-1`: When [`RoutingContext::required_capabilities`] is non-empty, the
///   resolved provider must claim all required capability bits or resolution
///   fails closed with [`PaymentError::Unsupported`].
pub fn resolve_provider(
    policy: &dyn RoutingPolicy,
    ctx: &RoutingContext,
    registry: &ProviderRegistry,
) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
    let selected = match ctx.explicit_provider.as_ref() {
        Some(id) => id.clone(),
        None => policy.select(ctx, registry)?,
    };

    let provider = registry.require(&selected)?;

    if !ctx.required_capabilities.is_empty()
        && !provider.capabilities().contains(ctx.required_capabilities)
    {
        return Err(PaymentError::unsupported(
            provider.id(),
            "routing_capability_check",
        ));
    }

    Ok(provider)
}

impl PaymentOrchestratorBuilder {
    /// Installs a deterministic [`RoutingPolicy`] as the orchestrator's route
    /// resolver.
    ///
    /// Wraps `policy` into a [`RouteResolver`] delegating to
    /// [`resolve_provider`], guaranteeing explicit-override precedence (`AC-2`),
    /// registry membership validation (`TM-6`), and capability checks (`TM-1`).
    pub fn with_routing_policy(self, policy: Arc<dyn RoutingPolicy>) -> Self {
        let resolver: RouteResolver = Arc::new(
            move |registry: &ProviderRegistry,
                  explicit_provider: Option<&ProviderId>,
                  currency: Option<&CurrencyCode>,
                  buyer_country: Option<&str>,
                  required_capabilities: PaymentCapabilities| {
                let ctx = RoutingContext {
                    explicit_provider: explicit_provider.cloned(),
                    currency: currency.cloned(),
                    buyer_country: buyer_country.map(str::to_owned),
                    required_capabilities,
                };
                resolve_provider(policy.as_ref(), &ctx, registry)
            },
        );
        self.with_route_resolver(resolver)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, ErrorKind, NormalizedWebhookEvent, PaymentResult, ProviderReference,
        RawWebhookRequest,
    };

    use super::*;

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

    #[derive(Debug)]
    struct FixedPolicy {
        target: ProviderId,
    }

    impl RoutingPolicy for FixedPolicy {
        fn select(
            &self,
            _ctx: &RoutingContext,
            _registry: &ProviderRegistry,
        ) -> Result<ProviderId, PaymentError> {
            Ok(self.target.clone())
        }
    }

    fn test_registry() -> ProviderRegistry {
        let stripe = MockProvider::new(
            "stripe",
            PaymentCapabilities::DIRECT_PAYMENT
                | PaymentCapabilities::HOSTED_CHECKOUT
                | PaymentCapabilities::SPLITS,
        );
        let moyasar = MockProvider::new("moyasar", PaymentCapabilities::DIRECT_PAYMENT);
        let empty = MockProvider::new("minimal", PaymentCapabilities::empty());
        let mut providers: HashMap<ProviderId, Arc<dyn PaymentProvider>> = HashMap::new();
        for provider in [stripe, moyasar, empty] {
            let id = provider.id().clone();
            providers.insert(id, provider);
        }
        ProviderRegistry::new(providers, ProviderId::new("stripe")).expect("valid test registry")
    }

    #[test]
    fn tc_2_2_1_ac_2_explicit_override_takes_absolute_precedence() {
        let registry = test_registry();
        let always_moyasar = FixedPolicy {
            target: ProviderId::new("moyasar"),
        };
        let ctx = RoutingContext::new()
            .with_currency(CurrencyCode::try_new("SAR").unwrap())
            .with_buyer_country("SA")
            .with_explicit_provider(ProviderId::new("stripe"));

        let resolved = resolve_provider(&always_moyasar, &ctx, &registry)
            .expect("explicit override must succeed");
        assert_eq!(resolved.id().as_str(), "stripe");
    }

    #[test]
    fn tm_6_unregistered_provider_from_custom_policy_fails_with_configuration_error() {
        let registry = test_registry();
        let rogue_policy = FixedPolicy {
            target: ProviderId::new("unregistered_gateway"),
        };

        let err = resolve_provider(&rogue_policy, &RoutingContext::new(), &registry)
            .expect_err("unregistered provider must fail closed");
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn tm_6_default_only_policy_rejects_unregistered_explicit_provider() {
        let registry = test_registry();
        let ctx = RoutingContext::new().with_explicit_provider(ProviderId::new("unknown_provider"));

        let err = DefaultOnlyPolicy
            .select(&ctx, &registry)
            .expect_err("unregistered explicit override must fail");
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn tm_1_missing_capability_fails_with_unsupported_error() {
        let registry = test_registry();
        let ctx = RoutingContext::new()
            .with_explicit_provider(ProviderId::new("moyasar"))
            .with_required_capabilities(PaymentCapabilities::SPLITS);

        let err = resolve_provider(&DefaultOnlyPolicy, &ctx, &registry)
            .expect_err("missing capability must fail closed");
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        assert_eq!(err.provider().map(ProviderId::as_str), Some("moyasar"));
    }

    #[test]
    fn tm_1_satisfied_or_empty_capabilities_succeed() {
        let registry = test_registry();

        let with_splits = RoutingContext::new()
            .with_explicit_provider(ProviderId::new("stripe"))
            .with_required_capabilities(PaymentCapabilities::SPLITS);
        let provider = resolve_provider(&DefaultOnlyPolicy, &with_splits, &registry)
            .expect("provider with required capability must succeed");
        assert_eq!(provider.id().as_str(), "stripe");

        let empty_caps = RoutingContext::new()
            .with_explicit_provider(ProviderId::new("minimal"))
            .with_required_capabilities(PaymentCapabilities::empty());
        let minimal = resolve_provider(&DefaultOnlyPolicy, &empty_caps, &registry)
            .expect("empty required capabilities must skip capability guard");
        assert_eq!(minimal.id().as_str(), "minimal");
    }

    #[test]
    fn default_only_policy_selects_registry_default_when_no_override() {
        let registry = test_registry();
        let ctx = RoutingContext::new()
            .with_currency(CurrencyCode::try_new("USD").unwrap())
            .with_buyer_country("US");

        let selected = DefaultOnlyPolicy
            .select(&ctx, &registry)
            .expect("default selection must succeed");
        assert_eq!(selected.as_str(), "stripe");

        let resolved = resolve_provider(&DefaultOnlyPolicy, &ctx, &registry)
            .expect("resolve_provider with DefaultOnlyPolicy must succeed");
        assert_eq!(resolved.id().as_str(), "stripe");
    }

    #[test]
    fn routing_context_builders_populate_all_fields() {
        let empty = RoutingContext::new();
        assert_eq!(empty, RoutingContext::default());

        let ctx = RoutingContext::new()
            .with_explicit_provider(ProviderId::new("moyasar"))
            .with_currency(CurrencyCode::try_new("SAR").unwrap())
            .with_buyer_country("SA")
            .with_required_capabilities(PaymentCapabilities::DIRECT_PAYMENT);

        assert_eq!(
            ctx.explicit_provider.as_ref().map(ProviderId::as_str),
            Some("moyasar"),
        );
        assert_eq!(ctx.currency.as_ref().map(CurrencyCode::as_str), Some("SAR"));
        assert_eq!(ctx.buyer_country.as_deref(), Some("SA"));
        assert_eq!(
            ctx.required_capabilities,
            PaymentCapabilities::DIRECT_PAYMENT
        );
    }

    #[test]
    fn builder_with_routing_policy_installs_route_resolver_and_builds() {
        let stripe = MockProvider::new(
            "stripe",
            PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::HOSTED_CHECKOUT,
        );
        let moyasar = MockProvider::new("moyasar", PaymentCapabilities::DIRECT_PAYMENT);
        let policy: Arc<dyn RoutingPolicy> = Arc::new(FixedPolicy {
            target: ProviderId::new("moyasar"),
        });

        let builder = PaymentOrchestratorBuilder::new()
            .register(stripe)
            .expect("stripe registration must succeed")
            .register(moyasar)
            .expect("moyasar registration must succeed")
            .default_provider(ProviderId::new("stripe"))
            .with_routing_policy(policy);

        assert!(format!("{builder:?}").contains("has_route_resolver: true"));
        assert!(builder.build().is_ok());
    }
}
