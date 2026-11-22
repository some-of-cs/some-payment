//! Thread-safe, immutable-post-build payment provider registry.
//!
//! [`ProviderRegistry`] stores registered [`PaymentProvider`] trait objects behind an
//! [`Arc`]-wrapped map so lookups, capability queries, and clones across worker threads
//! incur zero runtime lock contention.

use std::collections::HashMap;
use std::sync::Arc;

use some_payment_core::{PaymentCapabilities, PaymentError, PaymentProvider, ProviderId};

/// Immutable, thread-safe registry of [`PaymentProvider`] implementations.
///
/// Cloning a `ProviderRegistry` is cheap (`Arc::clone` of the underlying map) and all
/// read operations are lock-free after construction.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use std::sync::Arc;
///
/// use async_trait::async_trait;
/// use some_payment::registry::ProviderRegistry;
/// use some_payment_core::{
///     CreatePaymentRequest, NormalizedWebhookEvent, PaymentCapabilities,
///     PaymentError, PaymentProvider, PaymentResult, ProviderId,
///     ProviderReference, RawWebhookRequest,
/// };
///
/// #[derive(Debug)]
/// struct StubProvider {
///     id: ProviderId,
///     capabilities: PaymentCapabilities,
/// }
///
/// #[async_trait]
/// impl PaymentProvider for StubProvider {
///     fn id(&self) -> &ProviderId {
///         &self.id
///     }
///
///     fn capabilities(&self) -> PaymentCapabilities {
///         self.capabilities
///     }
///
///     async fn create_payment(
///         &self,
///         _req: &CreatePaymentRequest,
///     ) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.id, "create_payment"))
///     }
///
///     async fn get_payment(
///         &self,
///         _reference: &ProviderReference,
///     ) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.id, "get_payment"))
///     }
///
///     async fn parse_and_verify_webhook(
///         &self,
///         _raw: &RawWebhookRequest,
///     ) -> Result<NormalizedWebhookEvent, PaymentError> {
///         Err(PaymentError::unsupported(
///             &self.id,
///             "parse_and_verify_webhook",
///         ))
///     }
/// }
///
/// let id = ProviderId::new("stub");
/// let provider: Arc<dyn PaymentProvider> = Arc::new(StubProvider {
///     id: id.clone(),
///     capabilities: PaymentCapabilities::DIRECT_PAYMENT,
/// });
///
/// let mut providers = HashMap::new();
/// providers.insert(id.clone(), provider);
///
/// let registry = ProviderRegistry::new(providers, id.clone()).unwrap();
/// assert_eq!(registry.len(), 1);
/// assert!(!registry.is_empty());
/// assert_eq!(registry.default_id(), &id);
/// assert!(registry.require(&id).is_ok());
/// ```
#[derive(Clone, Debug)]
pub struct ProviderRegistry {
    providers: Arc<HashMap<ProviderId, Arc<dyn PaymentProvider>>>,
    default_id: ProviderId,
}

impl ProviderRegistry {
    /// Creates a new [`ProviderRegistry`] from a non-empty provider map and default ID.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] with
    /// [`ErrorKind::Configuration`](some_payment_core::ErrorKind::Configuration)
    /// when `providers` is empty or does not contain `default_id`.
    pub fn new(
        providers: HashMap<ProviderId, Arc<dyn PaymentProvider>>,
        default_id: ProviderId,
    ) -> Result<Self, PaymentError> {
        if providers.is_empty() {
            return Err(PaymentError::configuration(
                "provider registry must contain at least one registered provider",
            ));
        }
        if !providers.contains_key(&default_id) {
            return Err(PaymentError::configuration(format!(
                "default provider '{default_id}' is not registered in the provider registry"
            )));
        }
        Ok(Self {
            providers: Arc::new(providers),
            default_id,
        })
    }

    /// Looks up a registered provider by `id`, returning `None` if it is not present.
    #[must_use]
    pub fn get(&self, id: &ProviderId) -> Option<Arc<dyn PaymentProvider>> {
        self.providers.get(id).cloned()
    }

    /// Looks up a registered provider by `id`, returning a configuration error if missing.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] with
    /// [`ErrorKind::Configuration`](some_payment_core::ErrorKind::Configuration)
    /// when no provider with `id` is registered.
    pub fn require(&self, id: &ProviderId) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        self.get(id).ok_or_else(|| {
            PaymentError::configuration(format!(
                "payment provider '{id}' is not registered in the provider registry"
            ))
        })
    }

    /// Returns the configured default [`ProviderId`].
    #[must_use]
    pub fn default_id(&self) -> &ProviderId {
        &self.default_id
    }

    /// Returns the configured default [`PaymentProvider`].
    #[must_use]
    pub fn default_provider(&self) -> Arc<dyn PaymentProvider> {
        Arc::clone(
            self.providers
                .get(&self.default_id)
                .expect("default_id is validated during ProviderRegistry::new"),
        )
    }

    /// Resolves an explicit provider ID if supplied, or falls back to the default provider.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] with
    /// [`ErrorKind::Configuration`](some_payment_core::ErrorKind::Configuration)
    /// when `explicit` is `Some(id)` and `id` is not registered.
    pub fn resolve_explicit_or_default(
        &self,
        explicit: Option<&ProviderId>,
    ) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        match explicit {
            Some(id) => self.require(id),
            None => Ok(self.default_provider()),
        }
    }

    /// Returns `true` if a provider with the given `id` is registered.
    #[must_use]
    pub fn contains(&self, id: &ProviderId) -> bool {
        self.providers.contains_key(id)
    }

    /// Returns the number of registered providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Returns `true` if no providers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Returns all registered [`ProviderId`] values sorted lexicographically by `as_str()`.
    #[must_use]
    pub fn provider_ids(&self) -> Vec<ProviderId> {
        let mut ids: Vec<ProviderId> = self.providers.keys().cloned().collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids
    }

    /// Returns all registered providers supporting `required`, sorted by `id().as_str()`.
    #[must_use]
    pub fn providers_with_capability(
        &self,
        required: PaymentCapabilities,
    ) -> Vec<Arc<dyn PaymentProvider>> {
        let mut matching: Vec<Arc<dyn PaymentProvider>> = self
            .providers
            .values()
            .filter(|provider| provider.capabilities().contains(required))
            .cloned()
            .collect();
        matching.sort_by(|a, b| a.id().as_str().cmp(b.id().as_str()));
        matching
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::thread;

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, ErrorKind, NormalizedWebhookEvent, PaymentCapabilities, PaymentError,
        PaymentProvider, PaymentResult, ProviderId, ProviderReference, RawWebhookRequest,
    };

    use super::ProviderRegistry;

    #[derive(Debug)]
    struct MockProvider {
        id: ProviderId,
        capabilities: PaymentCapabilities,
    }

    impl MockProvider {
        fn new(slug: &str, capabilities: PaymentCapabilities) -> Arc<dyn PaymentProvider> {
            Arc::new(Self {
                id: ProviderId::new(slug),
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

    fn sample_registry() -> ProviderRegistry {
        let mut providers = HashMap::new();
        providers.insert(
            ProviderId::new("zeta"),
            MockProvider::new(
                "zeta",
                PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::REFUND,
            ),
        );
        providers.insert(
            ProviderId::new("alpha"),
            MockProvider::new(
                "alpha",
                PaymentCapabilities::HOSTED_CHECKOUT | PaymentCapabilities::REFUND,
            ),
        );
        providers.insert(
            ProviderId::new("beta"),
            MockProvider::new(
                "beta",
                PaymentCapabilities::DIRECT_PAYMENT
                    | PaymentCapabilities::HOSTED_CHECKOUT
                    | PaymentCapabilities::AUTH_CAPTURE,
            ),
        );
        ProviderRegistry::new(providers, ProviderId::new("beta")).unwrap()
    }

    #[test]
    fn rejects_empty_registry() {
        let err = ProviderRegistry::new(HashMap::new(), ProviderId::new("alpha")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn rejects_missing_default_provider() {
        let mut providers = HashMap::new();
        providers.insert(
            ProviderId::new("alpha"),
            MockProvider::new("alpha", PaymentCapabilities::DIRECT_PAYMENT),
        );
        let err = ProviderRegistry::new(providers, ProviderId::new("missing")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn basic_lookups_and_metadata_accessors() {
        let registry = sample_registry();
        let beta = ProviderId::new("beta");
        let unknown = ProviderId::new("unknown");

        assert_eq!(registry.len(), 3);
        assert!(!registry.is_empty());
        assert!(registry.contains(&beta));
        assert!(!registry.contains(&unknown));
        assert_eq!(registry.default_id(), &beta);
        assert_eq!(registry.default_provider().id(), &beta);
        assert_eq!(registry.get(&beta).unwrap().id(), &beta);
        assert!(registry.get(&unknown).is_none());
    }

    #[test]
    fn require_returns_hit_or_configuration_error() {
        let registry = sample_registry();
        let alpha = ProviderId::new("alpha");
        let missing = ProviderId::new("nonexistent");

        let found = registry.require(&alpha).unwrap();
        assert_eq!(found.id(), &alpha);

        let err = registry.require(&missing).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn resolve_explicit_or_default_behaviour() {
        let registry = sample_registry();
        let alpha = ProviderId::new("alpha");
        let missing = ProviderId::new("missing");

        let default_resolved = registry.resolve_explicit_or_default(None).unwrap();
        assert_eq!(default_resolved.id().as_str(), "beta");

        let explicit_resolved = registry.resolve_explicit_or_default(Some(&alpha)).unwrap();
        assert_eq!(explicit_resolved.id(), &alpha);

        let err = registry
            .resolve_explicit_or_default(Some(&missing))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Configuration);
    }

    #[test]
    fn provider_ids_are_sorted_lexicographically() {
        let registry = sample_registry();
        let ids: Vec<String> = registry
            .provider_ids()
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        assert_eq!(ids, vec!["alpha", "beta", "zeta"]);
    }

    #[test]
    fn providers_with_capability_filters_and_sorts_deterministically() {
        let registry = sample_registry();

        let direct_ids: Vec<String> = registry
            .providers_with_capability(PaymentCapabilities::DIRECT_PAYMENT)
            .into_iter()
            .map(|p| p.id().as_str().to_owned())
            .collect();
        assert_eq!(direct_ids, vec!["beta", "zeta"]);

        let refund_ids: Vec<String> = registry
            .providers_with_capability(PaymentCapabilities::REFUND)
            .into_iter()
            .map(|p| p.id().as_str().to_owned())
            .collect();
        assert_eq!(refund_ids, vec!["alpha", "zeta"]);

        let combined_ids: Vec<String> = registry
            .providers_with_capability(
                PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::HOSTED_CHECKOUT,
            )
            .into_iter()
            .map(|p| p.id().as_str().to_owned())
            .collect();
        assert_eq!(combined_ids, vec!["beta"]);

        let unmatched = registry.providers_with_capability(
            PaymentCapabilities::AUTH_CAPTURE | PaymentCapabilities::REFUND,
        );
        assert!(unmatched.is_empty());
    }

    #[test]
    fn concurrent_reads_across_threads() {
        let registry = sample_registry();
        let mut handles = Vec::new();

        for _ in 0..8 {
            let reg = registry.clone();
            handles.push(thread::spawn(move || {
                let alpha = ProviderId::new("alpha");
                let missing = ProviderId::new("missing");

                for _ in 0..50 {
                    assert_eq!(reg.len(), 3);
                    assert_eq!(reg.default_provider().id().as_str(), "beta");
                    assert_eq!(reg.require(&alpha).unwrap().id(), &alpha);
                    assert_eq!(
                        reg.require(&missing).unwrap_err().kind(),
                        ErrorKind::Configuration
                    );
                    assert_eq!(
                        reg.resolve_explicit_or_default(None).unwrap().id().as_str(),
                        "beta"
                    );
                    assert_eq!(reg.provider_ids().len(), 3);
                    assert_eq!(
                        reg.providers_with_capability(PaymentCapabilities::REFUND)
                            .len(),
                        2
                    );
                }
            }));
        }

        for handle in handles {
            handle.join().expect("worker thread panicked");
        }
    }
}
