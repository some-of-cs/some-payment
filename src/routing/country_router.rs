//! Deterministic regional currency/country router and configurable country
//! routing policy (`US-2.2.0` / Unit 2).
//!
//! Provides two pure, zero-I/O [`RoutingPolicy`] implementations:
//!
//! - [`RegionalCurrencyRouter`]: routes MENA regional currencies and countries
//!   via static lookup tables (`SAR`/`SA` -> `moyasar`, `EGP`/`EG` -> `paymob`,
//!   international default -> `stripe`).
//! - [`CountryRoutingPolicy`]: builder-configured currency, country, and
//!   fallback rules with strict capability (`TM-1`) and registry (`TM-6`)
//!   enforcement.

use std::collections::BTreeMap;

use some_payment_core::{PaymentCapabilities, PaymentError, ProviderId};

use crate::registry::ProviderRegistry;
use crate::routing::policy::{RoutingContext, RoutingPolicy};

const REGIONAL_CURRENCY_ROUTES: &[(&str, &str)] = &[("SAR", "moyasar"), ("EGP", "paymob")];
const REGIONAL_COUNTRY_ROUTES: &[(&str, &str)] = &[("SA", "moyasar"), ("EG", "paymob")];
const INTERNATIONAL_DEFAULT_SLUG: &str = "stripe";

/// Deterministic MENA + international [`RoutingPolicy`] backed by static lookup
/// tables.
///
/// Selection precedence:
/// 1. `ctx.explicit_provider` (validated against [`ProviderRegistry`]; returns
///    [`PaymentError::ConfigurationError`] if unregistered).
/// 2. `ctx.currency` matched in the regional currency table (`SAR` ->
///    `moyasar`, `EGP` -> `paymob`) when registered and capable.
/// 3. Normalized `ctx.buyer_country` matched in the regional country table
///    (`SA` -> `moyasar`, `EG` -> `paymob`) when registered and capable.
/// 4. International default (`stripe`) when registered and capable, otherwise
///    [`ProviderRegistry::default_id`].
///
/// # Examples
///
/// ```
/// use some_payment::routing::country_router::RegionalCurrencyRouter;
///
/// let router = RegionalCurrencyRouter::default();
/// let copied = router;
/// assert_eq!(format!("{copied:?}"), "RegionalCurrencyRouter");
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct RegionalCurrencyRouter;

impl RoutingPolicy for RegionalCurrencyRouter {
    fn select(
        &self,
        ctx: &RoutingContext,
        registry: &ProviderRegistry,
    ) -> Result<ProviderId, PaymentError> {
        if let Some(explicit) = resolve_explicit(ctx, registry)? {
            return Ok(explicit);
        }

        let required = ctx.required_capabilities;
        let currency_candidate = ctx
            .currency
            .as_ref()
            .and_then(|code| lookup_static_route(REGIONAL_CURRENCY_ROUTES, code.as_str()))
            .filter(|id| is_capable(registry, id, required));

        let country_candidate = || {
            ctx.buyer_country
                .as_deref()
                .map(normalize_code)
                .and_then(|code| lookup_static_route(REGIONAL_COUNTRY_ROUTES, &code))
                .filter(|id| is_capable(registry, id, required))
        };

        let international_candidate = || {
            let id = ProviderId::new(INTERNATIONAL_DEFAULT_SLUG);
            is_capable(registry, &id, required).then_some(id)
        };

        Ok(currency_candidate
            .or_else(country_candidate)
            .or_else(international_candidate)
            .unwrap_or_else(|| registry.default_id().clone()))
    }
}

/// Configurable deterministic [`RoutingPolicy`] mapping ISO-4217 currencies and
/// ISO-3166-1 alpha-2 country codes to [`ProviderId`] targets.
///
/// Selection precedence:
/// 1. `ctx.explicit_provider` (validated against [`ProviderRegistry`]).
/// 2. Configured currency rule when registered and capable (`TM-1`, `TM-6`).
/// 3. Configured country rule when registered and capable (`TM-1`, `TM-6`).
/// 4. Configured fallback provider when registered and capable.
/// 5. [`ProviderRegistry::default_id`].
///
/// # Examples
///
/// ```
/// use some_payment::routing::country_router::CountryRoutingPolicy;
/// use some_payment_core::ProviderId;
///
/// let policy = CountryRoutingPolicy::builder()
///     .route_country(" sa ", ProviderId::new("moyasar"))
///     .route_currency("egp", ProviderId::new("paymob"))
///     .fallback(ProviderId::new("stripe"))
///     .build();
///
/// assert_eq!(
///     policy.country_route("SA"),
///     Some(&ProviderId::new("moyasar"))
/// );
/// assert_eq!(
///     policy.currency_route("EGP"),
///     Some(&ProviderId::new("paymob"))
/// );
/// assert_eq!(policy.fallback(), Some(&ProviderId::new("stripe")));
/// ```
#[derive(Clone, Debug)]
pub struct CountryRoutingPolicy {
    currency_routes: BTreeMap<String, ProviderId>,
    country_routes: BTreeMap<String, ProviderId>,
    fallback: Option<ProviderId>,
}

impl CountryRoutingPolicy {
    /// Creates a new [`CountryRoutingPolicyBuilder`] with empty routing tables.
    #[must_use]
    pub fn builder() -> CountryRoutingPolicyBuilder {
        CountryRoutingPolicyBuilder::default()
    }

    /// Returns the configured provider for an ISO-3166-1 alpha-2 country code,
    /// if any.
    #[must_use]
    pub fn country_route(&self, country_iso2: impl AsRef<str>) -> Option<&ProviderId> {
        let key = normalize_code(country_iso2);
        self.country_routes.get(&key)
    }

    /// Returns the configured provider for an ISO-4217 currency code, if any.
    #[must_use]
    pub fn currency_route(&self, currency_iso3: impl AsRef<str>) -> Option<&ProviderId> {
        let key = normalize_code(currency_iso3);
        self.currency_routes.get(&key)
    }

    /// Returns the configured fallback [`ProviderId`], if any.
    #[must_use]
    pub fn fallback(&self) -> Option<&ProviderId> {
        self.fallback.as_ref()
    }
}

impl RoutingPolicy for CountryRoutingPolicy {
    fn select(
        &self,
        ctx: &RoutingContext,
        registry: &ProviderRegistry,
    ) -> Result<ProviderId, PaymentError> {
        if let Some(explicit) = resolve_explicit(ctx, registry)? {
            return Ok(explicit);
        }

        let required = ctx.required_capabilities;
        let currency_candidate = ctx
            .currency
            .as_ref()
            .and_then(|code| self.currency_routes.get(code.as_str()))
            .filter(|id| is_capable(registry, id, required))
            .cloned();

        let country_candidate = || {
            ctx.buyer_country
                .as_deref()
                .map(normalize_code)
                .and_then(|code| self.country_routes.get(&code))
                .filter(|id| is_capable(registry, id, required))
                .cloned()
        };

        let fallback_candidate = || {
            self.fallback
                .as_ref()
                .filter(|id| is_capable(registry, id, required))
                .cloned()
        };

        Ok(currency_candidate
            .or_else(country_candidate)
            .or_else(fallback_candidate)
            .unwrap_or_else(|| registry.default_id().clone()))
    }
}

/// Fluent builder for constructing a [`CountryRoutingPolicy`].
#[derive(Clone, Debug, Default)]
pub struct CountryRoutingPolicyBuilder {
    currency_routes: BTreeMap<String, ProviderId>,
    country_routes: BTreeMap<String, ProviderId>,
    fallback: Option<ProviderId>,
}

impl CountryRoutingPolicyBuilder {
    /// Registers a country routing rule, normalizing `country_iso2` to trimmed
    /// ASCII uppercase.
    #[must_use]
    pub fn route_country(mut self, country_iso2: impl AsRef<str>, provider_id: ProviderId) -> Self {
        let normalized = normalize_code(country_iso2);
        if !normalized.is_empty() {
            self.country_routes.insert(normalized, provider_id);
        }
        self
    }

    /// Registers a currency routing rule, normalizing `currency_iso3` to
    /// trimmed ASCII uppercase.
    #[must_use]
    pub fn route_currency(
        mut self,
        currency_iso3: impl AsRef<str>,
        provider_id: ProviderId,
    ) -> Self {
        let normalized = normalize_code(currency_iso3);
        if !normalized.is_empty() {
            self.currency_routes.insert(normalized, provider_id);
        }
        self
    }

    /// Configures the fallback [`ProviderId`] used when neither currency nor
    /// country rules produce a registered, capable provider.
    #[must_use]
    pub fn fallback(mut self, provider_id: ProviderId) -> Self {
        self.fallback = Some(provider_id);
        self
    }

    /// Consumes this builder and returns the immutable [`CountryRoutingPolicy`].
    #[must_use]
    pub fn build(self) -> CountryRoutingPolicy {
        CountryRoutingPolicy {
            currency_routes: self.currency_routes,
            country_routes: self.country_routes,
            fallback: self.fallback,
        }
    }
}

fn normalize_code(code: impl AsRef<str>) -> String {
    code.as_ref().trim().to_ascii_uppercase()
}

fn lookup_static_route(table: &[(&str, &str)], key: &str) -> Option<ProviderId> {
    table
        .iter()
        .find(|(code, _)| *code == key)
        .map(|(_, slug)| ProviderId::new(*slug))
}

fn is_capable(registry: &ProviderRegistry, id: &ProviderId, required: PaymentCapabilities) -> bool {
    registry
        .get(id)
        .is_some_and(|provider| provider.capabilities().contains(required))
}

fn resolve_explicit(
    ctx: &RoutingContext,
    registry: &ProviderRegistry,
) -> Result<Option<ProviderId>, PaymentError> {
    let Some(id) = ctx.explicit_provider.as_ref() else {
        return Ok(None);
    };
    if !registry.contains(id) {
        return Err(PaymentError::configuration(format!(
            "explicit provider '{id}' is not registered"
        )));
    }
    Ok(Some(id.clone()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, CurrencyCode, ErrorKind, NormalizedWebhookEvent, PaymentCapabilities,
        PaymentError, PaymentProvider, PaymentResult, ProviderId, ProviderReference,
        RawWebhookRequest,
    };

    use super::*;

    #[derive(Debug)]
    struct StubProvider {
        id: ProviderId,
        capabilities: PaymentCapabilities,
    }

    impl StubProvider {
        fn new(slug: &str, capabilities: PaymentCapabilities) -> Arc<dyn PaymentProvider> {
            Arc::new(Self {
                id: ProviderId::new(slug),
                capabilities,
            })
        }
    }

    #[async_trait]
    impl PaymentProvider for StubProvider {
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

    fn build_registry(providers: Vec<Arc<dyn PaymentProvider>>, default: &str) -> ProviderRegistry {
        let mut map = HashMap::new();
        for provider in providers {
            map.insert(provider.id().clone(), provider);
        }
        ProviderRegistry::new(map, ProviderId::new(default)).expect("valid test registry")
    }

    fn standard_registry() -> ProviderRegistry {
        let core_caps = PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::HOSTED_CHECKOUT;
        build_registry(
            vec![
                StubProvider::new("stripe", core_caps | PaymentCapabilities::SPLITS),
                StubProvider::new("moyasar", core_caps),
                StubProvider::new("paymob", core_caps),
            ],
            "stripe",
        )
    }

    fn routing_ctx(
        explicit: Option<&str>,
        currency: Option<&str>,
        country: Option<&str>,
        required_capabilities: PaymentCapabilities,
    ) -> RoutingContext {
        RoutingContext {
            explicit_provider: explicit.map(ProviderId::new),
            currency: currency.map(|c| CurrencyCode::try_new(c).expect("valid currency")),
            buyer_country: country.map(str::to_owned),
            required_capabilities,
        }
    }

    #[test]
    fn regional_router_routes_sar_egp_and_usd() {
        let registry = standard_registry();
        let router = RegionalCurrencyRouter;

        let sar = routing_ctx(None, Some("SAR"), None, PaymentCapabilities::DIRECT_PAYMENT);
        let egp = routing_ctx(None, Some("EGP"), None, PaymentCapabilities::DIRECT_PAYMENT);
        let usd = routing_ctx(None, Some("USD"), None, PaymentCapabilities::DIRECT_PAYMENT);

        assert_eq!(
            router.select(&sar, &registry).unwrap(),
            ProviderId::new("moyasar")
        );
        assert_eq!(
            router.select(&egp, &registry).unwrap(),
            ProviderId::new("paymob")
        );
        assert_eq!(
            router.select(&usd, &registry).unwrap(),
            ProviderId::new("stripe")
        );
    }

    #[test]
    fn regional_router_routes_by_normalized_country_when_currency_missing() {
        let registry = standard_registry();
        let router = RegionalCurrencyRouter;

        let sa = routing_ctx(None, None, Some(" sa "), PaymentCapabilities::empty());
        let eg = routing_ctx(None, None, Some("eg"), PaymentCapabilities::empty());
        let us = routing_ctx(None, None, Some("US"), PaymentCapabilities::empty());

        assert_eq!(
            router.select(&sa, &registry).unwrap(),
            ProviderId::new("moyasar")
        );
        assert_eq!(
            router.select(&eg, &registry).unwrap(),
            ProviderId::new("paymob")
        );
        assert_eq!(
            router.select(&us, &registry).unwrap(),
            ProviderId::new("stripe")
        );
    }

    #[test]
    fn regional_router_explicit_override_takes_precedence() {
        let registry = standard_registry();
        let router = RegionalCurrencyRouter;

        let ctx = routing_ctx(
            Some("stripe"),
            Some("SAR"),
            Some("SA"),
            PaymentCapabilities::DIRECT_PAYMENT,
        );
        assert_eq!(
            router.select(&ctx, &registry).unwrap(),
            ProviderId::new("stripe")
        );
    }

    #[test]
    fn regional_router_rejects_unregistered_explicit_provider() {
        let registry = standard_registry();
        let router = RegionalCurrencyRouter;

        let ctx = routing_ctx(
            Some("unknown"),
            Some("SAR"),
            Some("SA"),
            PaymentCapabilities::empty(),
        );
        let err = router.select(&ctx, &registry).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn regional_router_capability_mismatch_falls_back_safely() {
        let registry = standard_registry();
        let router = RegionalCurrencyRouter;

        // moyasar does not declare SPLITS; stripe does -> falls back to stripe (TM-1)
        let ctx = routing_ctx(
            None,
            Some("SAR"),
            Some("SA"),
            PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::SPLITS,
        );
        assert_eq!(
            router.select(&ctx, &registry).unwrap(),
            ProviderId::new("stripe")
        );
    }

    #[test]
    fn regional_router_falls_back_to_registry_default_when_international_incapable() {
        let registry = build_registry(
            vec![
                StubProvider::new("moyasar", PaymentCapabilities::DIRECT_PAYMENT),
                StubProvider::new("paymob", PaymentCapabilities::DIRECT_PAYMENT),
            ],
            "paymob",
        );
        let router = RegionalCurrencyRouter;

        // Neither SAR (moyasar) nor international default (stripe, absent) has AUTH_CAPTURE
        let ctx = routing_ctx(
            None,
            Some("SAR"),
            Some("SA"),
            PaymentCapabilities::AUTH_CAPTURE,
        );
        assert_eq!(
            router.select(&ctx, &registry).unwrap(),
            ProviderId::new("paymob")
        );
    }

    #[test]
    fn country_policy_builder_normalizes_and_applies_precedence() {
        let registry = standard_registry();
        let policy = CountryRoutingPolicy::builder()
            .route_currency(" sar ", ProviderId::new("moyasar"))
            .route_country(" eg ", ProviderId::new("paymob"))
            .fallback(ProviderId::new("stripe"))
            .build();

        // Explicit override wins over currency and country
        let explicit_ctx = routing_ctx(
            Some("stripe"),
            Some("SAR"),
            Some("EG"),
            PaymentCapabilities::DIRECT_PAYMENT,
        );
        assert_eq!(
            policy.select(&explicit_ctx, &registry).unwrap(),
            ProviderId::new("stripe")
        );

        // Currency rule wins over country rule
        let currency_ctx = routing_ctx(
            None,
            Some("SAR"),
            Some("EG"),
            PaymentCapabilities::DIRECT_PAYMENT,
        );
        assert_eq!(
            policy.select(&currency_ctx, &registry).unwrap(),
            ProviderId::new("moyasar")
        );

        // Country rule applies when currency has no rule
        let country_ctx = routing_ctx(
            None,
            Some("USD"),
            Some("eg"),
            PaymentCapabilities::DIRECT_PAYMENT,
        );
        assert_eq!(
            policy.select(&country_ctx, &registry).unwrap(),
            ProviderId::new("paymob")
        );

        // Fallback applies when neither currency nor country matches
        let fallback_ctx = routing_ctx(
            None,
            Some("EUR"),
            Some("DE"),
            PaymentCapabilities::DIRECT_PAYMENT,
        );
        assert_eq!(
            policy.select(&fallback_ctx, &registry).unwrap(),
            ProviderId::new("stripe")
        );
    }

    #[test]
    fn country_policy_skips_unregistered_or_incapable_rules() {
        let registry = standard_registry();
        let policy = CountryRoutingPolicy::builder()
            .route_currency("AED", ProviderId::new("unregistered-gw"))
            .route_country("AE", ProviderId::new("paymob"))
            .fallback(ProviderId::new("stripe"))
            .build();

        // Unregistered currency rule (TM-6) skipped -> falls through to registered AE country rule
        let unregistered_ctx = routing_ctx(
            None,
            Some("AED"),
            Some("AE"),
            PaymentCapabilities::DIRECT_PAYMENT,
        );
        assert_eq!(
            policy.select(&unregistered_ctx, &registry).unwrap(),
            ProviderId::new("paymob")
        );

        // Country rule lacks SPLITS (TM-1) -> falls through to capable fallback (stripe)
        let incapable_ctx = routing_ctx(
            None,
            Some("AED"),
            Some("AE"),
            PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::SPLITS,
        );
        assert_eq!(
            policy.select(&incapable_ctx, &registry).unwrap(),
            ProviderId::new("stripe")
        );
    }
}
