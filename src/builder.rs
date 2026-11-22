//! Validated payment orchestrator builder (`US-2.1.0` / Unit 4).
//!
//! Provides [`PaymentOrchestratorBuilder`] for assembling a
//! [`PaymentOrchestrator`] with startup validation, duplicate-provider
//! rejection (`TM-1`), lifecycle hook registration, and optional route
//! resolution.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use some_payment_core::{PaymentError, PaymentProvider, ProviderId};

use crate::hooks::{HookChain, LifecycleHook};
use crate::orchestrator::{PaymentOrchestrator, RouteResolver};
use crate::registry::ProviderRegistry;

/// Validated builder for constructing a [`PaymentOrchestrator`].
///
/// Enforces startup invariants before any payment request is processed:
/// - Rejects duplicate [`ProviderId`] registrations (`TM-1`) so a second
///   registration can never silently overwrite an existing provider.
/// - Requires at least one registered provider.
/// - Auto-selects the default provider when exactly one provider is registered,
///   and requires an explicit [`Self::default_provider`] when multiple
///   providers are registered.
/// - Verifies that any configured default provider exists in the registry.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use async_trait::async_trait;
/// use some_payment::orchestrator::PaymentOrchestrator;
/// use some_payment::some_payment_core::{
///     CreatePaymentRequest, NormalizedWebhookEvent, PaymentCapabilities, PaymentError,
///     PaymentProvider, PaymentResult, ProviderId, ProviderReference, RawWebhookRequest,
/// };
///
/// #[derive(Debug)]
/// struct MinimalProvider(ProviderId);
///
/// #[async_trait]
/// impl PaymentProvider for MinimalProvider {
///     fn id(&self) -> &ProviderId {
///         &self.0
///     }
///
///     fn capabilities(&self) -> PaymentCapabilities {
///         PaymentCapabilities::empty()
///     }
///
///     async fn create_payment(
///         &self,
///         _req: &CreatePaymentRequest,
///     ) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "create_payment"))
///     }
///
///     async fn get_payment(
///         &self,
///         _reference: &ProviderReference,
///     ) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "get_payment"))
///     }
///
///     async fn parse_and_verify_webhook(
///         &self,
///         _raw: &RawWebhookRequest,
///     ) -> Result<NormalizedWebhookEvent, PaymentError> {
///         Err(PaymentError::unsupported(
///             &self.0,
///             "parse_and_verify_webhook",
///         ))
///     }
/// }
///
/// let provider: Arc<dyn PaymentProvider> =
///     Arc::new(MinimalProvider(ProviderId::new("primary")));
/// let orchestrator = PaymentOrchestrator::builder()
///     .register(provider)
///     .expect("first registration succeeds")
///     .build()
///     .expect("single provider auto-defaults");
///
/// drop(orchestrator);
/// ```
#[derive(Default)]
pub struct PaymentOrchestratorBuilder {
    providers: HashMap<ProviderId, Arc<dyn PaymentProvider>>,
    default_id: Option<ProviderId>,
    hooks: Vec<Arc<dyn LifecycleHook>>,
    route_resolver: Option<RouteResolver>,
}

impl fmt::Debug for PaymentOrchestratorBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut provider_ids: Vec<&str> = self.providers.keys().map(ProviderId::as_str).collect();
        provider_ids.sort_unstable();
        f.debug_struct("PaymentOrchestratorBuilder")
            .field("provider_ids", &provider_ids)
            .field("default_id", &self.default_id)
            .field("hook_count", &self.hooks.len())
            .field("has_route_resolver", &self.route_resolver.is_some())
            .finish()
    }
}

impl PaymentOrchestratorBuilder {
    /// Creates an empty [`PaymentOrchestratorBuilder`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a [`PaymentProvider`] under its normalized [`ProviderId`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ConfigurationError`] if a provider with the same
    /// [`ProviderId`] has already been registered (`TM-1`).
    pub fn register(mut self, provider: Arc<dyn PaymentProvider>) -> Result<Self, PaymentError> {
        let id = provider.id().clone();
        if self.providers.contains_key(&id) {
            return Err(PaymentError::configuration(format!(
                "duplicate payment provider registration for '{id}'"
            )));
        }
        self.providers.insert(id, provider);
        Ok(self)
    }

    /// Sets the default [`ProviderId`] used when requests do not specify an
    /// explicit provider override or routing policy match.
    #[must_use]
    pub fn default_provider(mut self, id: ProviderId) -> Self {
        self.default_id = Some(id);
        self
    }

    /// Appends a [`LifecycleHook`] to the orchestrator's execution chain.
    #[must_use]
    pub fn with_hook(mut self, hook: Arc<dyn LifecycleHook>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Configures a custom [`RouteResolver`] for dynamic provider selection.
    #[must_use]
    pub fn with_route_resolver(mut self, resolver: RouteResolver) -> Self {
        self.route_resolver = Some(resolver);
        self
    }

    /// Validates builder configuration and constructs a [`PaymentOrchestrator`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ConfigurationError`] when:
    /// - No providers have been registered.
    /// - Multiple providers are registered without an explicit default provider.
    /// - The configured default provider is not present in the registry.
    pub fn build(self) -> Result<PaymentOrchestrator, PaymentError> {
        if self.providers.is_empty() {
            return Err(PaymentError::configuration(
                "at least one payment provider must be registered",
            ));
        }
        let default_id = resolve_default_id(&self.providers, self.default_id)?;
        let registry = ProviderRegistry::new(self.providers, default_id)?;
        let hooks = HookChain::new(self.hooks);
        Ok(PaymentOrchestrator::from_parts(registry, hooks, self.route_resolver))
    }
}

impl PaymentOrchestrator {
    /// Creates a new [`PaymentOrchestratorBuilder`] for configuring and
    /// validating a [`PaymentOrchestrator`] instance.
    #[must_use]
    pub fn builder() -> PaymentOrchestratorBuilder {
        PaymentOrchestratorBuilder::new()
    }
}

fn resolve_default_id(
    providers: &HashMap<ProviderId, Arc<dyn PaymentProvider>>,
    explicit_default: Option<ProviderId>,
) -> Result<ProviderId, PaymentError> {
    if let Some(id) = explicit_default {
        return validate_explicit_default(providers, id);
    }
    if providers.len() != 1 {
        return Err(PaymentError::configuration(
            "default_provider must be specified when multiple providers are registered",
        ));
    }
    providers.keys().next().cloned().ok_or_else(|| {
        PaymentError::configuration("at least one payment provider must be registered")
    })
}

fn validate_explicit_default(
    providers: &HashMap<ProviderId, Arc<dyn PaymentProvider>>,
    id: ProviderId,
) -> Result<ProviderId, PaymentError> {
    if providers.contains_key(&id) {
        return Ok(id);
    }
    Err(PaymentError::configuration(format!(
        "default provider '{id}' is not registered"
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, ErrorKind, NormalizedWebhookEvent, PaymentCapabilities, PaymentError,
        PaymentProvider, PaymentResult, ProviderId, ProviderReference, RawWebhookRequest,
    };

    use super::*;

    #[derive(Debug)]
    struct MockProvider(ProviderId);

    #[async_trait]
    impl PaymentProvider for MockProvider {
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

    #[derive(Debug)]
    struct NoOpHook;

    #[async_trait]
    impl LifecycleHook for NoOpHook {}

    fn mock_provider(slug: &str) -> Arc<dyn PaymentProvider> {
        Arc::new(MockProvider(ProviderId::new(slug)))
    }

    #[test]
    fn empty_registry_fails_with_configuration_error() {
        let err = PaymentOrchestrator::builder().build().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn duplicate_provider_registration_fails_with_configuration_error() {
        let builder = PaymentOrchestrator::builder()
            .register(mock_provider("alpha"))
            .expect("initial registration succeeds");
        let err = builder.register(mock_provider("ALPHA")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn invalid_default_provider_fails_with_configuration_error() {
        let err = PaymentOrchestrator::builder()
            .register(mock_provider("alpha"))
            .expect("registration succeeds")
            .default_provider(ProviderId::new("unknown"))
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn multi_provider_without_explicit_default_fails_with_configuration_error() {
        let err = PaymentOrchestrator::builder()
            .register(mock_provider("alpha"))
            .expect("first registration succeeds")
            .register(mock_provider("beta"))
            .expect("second registration succeeds")
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn single_provider_auto_defaults_and_builds_cleanly() {
        let orchestrator = PaymentOrchestrator::builder()
            .register(mock_provider("alpha"))
            .expect("registration succeeds")
            .build();
        assert!(orchestrator.is_ok());
    }

    #[test]
    fn multi_provider_with_valid_default_and_hook_builds_cleanly() {
        let hook: Arc<dyn LifecycleHook> = Arc::new(NoOpHook);
        let orchestrator = PaymentOrchestratorBuilder::new()
            .register(mock_provider("alpha"))
            .expect("first registration succeeds")
            .register(mock_provider("beta"))
            .expect("second registration succeeds")
            .default_provider(ProviderId::new("beta"))
            .with_hook(hook)
            .build();
        assert!(orchestrator.is_ok());
    }

    #[test]
    fn debug_output_formats_metadata_deterministically() {
        let hook: Arc<dyn LifecycleHook> = Arc::new(NoOpHook);
        let builder = PaymentOrchestrator::builder()
            .register(mock_provider("beta"))
            .expect("first registration succeeds")
            .register(mock_provider("alpha"))
            .expect("second registration succeeds")
            .default_provider(ProviderId::new("alpha"))
            .with_hook(hook);
        let rendered = format!("{builder:?}");
        assert!(rendered.contains("PaymentOrchestratorBuilder"));
        assert!(rendered.contains("provider_ids: ["alpha", "beta"]"));
        assert!(rendered.contains("hook_count: 1"));
        assert!(rendered.contains("has_route_resolver: false"));
    }
}
