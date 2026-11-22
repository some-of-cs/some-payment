//! Safe pre-charge failover policy with acyclic hop-bounded traversal.
//!
//! [`FallbackPolicy`] governs when an orchestrator operation may fail over from
//! a primary provider to a configured backup provider. To prevent duplicate
//! charges (`TM-2`), failover is permitted strictly on pre-charge
//! [`PaymentError::ProviderUnavailable`] errors where `indeterminate == false`.
//!
//! Cyclic fallback chains (`A -> B -> A` or `A -> A`) are terminated via a
//! visited-set guard and capped at [`MAX_FALLBACK_HOPS`] (`TM-5`).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use some_payment_core::{PaymentError, PaymentProvider, ProviderId};

use crate::registry::ProviderRegistry;

/// Hard upper bound on the number of fallback hops executed after a primary
/// provider fails with a safe pre-charge error (`TM-5`).
pub const MAX_FALLBACK_HOPS: usize = 3;

/// Returns `true` strictly when `error` represents a pre-charge
/// [`PaymentError::ProviderUnavailable`] failure (`indeterminate == false`).
///
/// Ambiguous timeouts (`indeterminate == true`), customer declines
/// ([`PaymentError::CardDeclined`]), 3DS failures, authentication errors,
/// rate-limit responses, validation/configuration errors, and all other
/// variants return `false` to prevent double-charging (`TM-2`, `AC-3`).
///
/// # Examples
///
/// ```
/// use some_payment::routing::fallback::is_safe_failover;
/// use some_payment_core::{PaymentError, ProviderId};
///
/// let paymob = ProviderId::new("paymob");
/// let pre_charge = PaymentError::ProviderUnavailable {
///     provider: paymob.clone(),
///     http_status: Some(503),
///     indeterminate: false,
///     message: "service unavailable".into(),
/// };
/// assert!(is_safe_failover(&pre_charge));
///
/// let timeout = PaymentError::ProviderUnavailable {
///     provider: paymob,
///     http_status: None,
///     indeterminate: true,
///     message: "connection timed out".into(),
/// };
/// assert!(!is_safe_failover(&timeout));
/// ```
pub fn is_safe_failover(error: &PaymentError) -> bool {
    matches!(
        error,
        PaymentError::ProviderUnavailable {
            indeterminate: false,
            ..
        }
    )
}

enum AttemptOutcome<T> {
    Final(Result<T, PaymentError>),
    Retryable(PaymentError),
}

fn classify_attempt<T>(result: Result<T, PaymentError>) -> AttemptOutcome<T> {
    match result {
        Ok(value) => AttemptOutcome::Final(Ok(value)),
        Err(err) if is_safe_failover(&err) => AttemptOutcome::Retryable(err),
        Err(err) => AttemptOutcome::Final(Err(err)),
    }
}

trait ProviderLookup {
    fn require(&self, id: &ProviderId) -> Result<Arc<dyn PaymentProvider>, PaymentError>;
    fn get(&self, id: &ProviderId) -> Option<Arc<dyn PaymentProvider>>;
}

impl ProviderLookup for ProviderRegistry {
    fn require(&self, id: &ProviderId) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        ProviderRegistry::require(self, id)
    }

    fn get(&self, id: &ProviderId) -> Option<Arc<dyn PaymentProvider>> {
        ProviderRegistry::get(self, id)
    }
}

async fn execute_fallbacks<T, L, F, Fut>(
    candidates: Vec<ProviderId>,
    mut last_error: PaymentError,
    lookup: &L,
    mut op: F,
) -> Result<T, PaymentError>
where
    L: ProviderLookup,
    F: FnMut(Arc<dyn PaymentProvider>) -> Fut,
    Fut: std::future::Future<Output = Result<T, PaymentError>>,
{
    for candidate in candidates {
        let Some(provider) = lookup.get(&candidate) else {
            continue;
        };
        match classify_attempt(op(provider).await) {
            AttemptOutcome::Final(result) => return result,
            AttemptOutcome::Retryable(err) => last_error = err,
        }
    }
    Err(last_error)
}

/// Deterministic, acyclic, hop-bounded pre-charge failover policy.
///
/// Maps primary [`ProviderId`]s to backup [`ProviderId`]s and executes
/// operations with safe pre-charge failover (`AC-3`, `TM-2`, `TM-4`, `TM-5`).
///
/// # Examples
///
/// ```
/// use some_payment::routing::fallback::{FallbackPolicy, MAX_FALLBACK_HOPS};
/// use some_payment_core::ProviderId;
///
/// let paymob = ProviderId::new("paymob");
/// let stripe = ProviderId::new("stripe");
///
/// let policy = FallbackPolicy::new()
///     .with_fallback(paymob.clone(), stripe.clone())
///     .with_max_hops(2)
///     .unwrap();
///
/// assert_eq!(policy.max_hops(), 2);
/// assert!(policy.max_hops() <= MAX_FALLBACK_HOPS);
/// assert_eq!(policy.next_candidates(&paymob), vec![stripe]);
/// ```
#[derive(Clone, Debug)]
pub struct FallbackPolicy {
    routes: BTreeMap<ProviderId, ProviderId>,
    max_hops: usize,
}

impl Default for FallbackPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl FallbackPolicy {
    /// Creates an empty [`FallbackPolicy`] with `max_hops` initialized to
    /// [`MAX_FALLBACK_HOPS`].
    pub fn new() -> Self {
        Self {
            routes: BTreeMap::new(),
            max_hops: MAX_FALLBACK_HOPS,
        }
    }

    /// Registers a directed fallback edge from `primary` to `fallback`.
    pub fn with_fallback(mut self, primary: ProviderId, fallback: ProviderId) -> Self {
        self.routes.insert(primary, fallback);
        self
    }

    /// Configures the maximum number of fallback hops allowed after a primary
    /// pre-charge failure.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ConfigurationError`] if `max_hops` is `0` or
    /// exceeds [`MAX_FALLBACK_HOPS`].
    pub fn with_max_hops(mut self, max_hops: usize) -> Result<Self, PaymentError> {
        if !(1..=MAX_FALLBACK_HOPS).contains(&max_hops) {
            return Err(PaymentError::configuration(
                "fallback max_hops must be between 1 and MAX_FALLBACK_HOPS",
            ));
        }
        self.max_hops = max_hops;
        Ok(self)
    }

    /// Returns the configured maximum number of fallback hops.
    pub fn max_hops(&self) -> usize {
        self.max_hops
    }

    /// Computes the ordered list of acyclic fallback candidates starting after
    /// `primary`, bounded to `<= self.max_hops` (`<= MAX_FALLBACK_HOPS`).
    ///
    /// Cycles (`A -> B -> A` or `A -> A`) terminate immediately via a visited
    /// set initialized with `primary` (`TM-5`).
    pub fn next_candidates(&self, primary: &ProviderId) -> Vec<ProviderId> {
        let hop_cap = self.max_hops.min(MAX_FALLBACK_HOPS);
        let mut visited = HashSet::with_capacity(hop_cap + 1);
        visited.insert(primary.clone());

        let mut candidates = Vec::with_capacity(hop_cap);
        let mut current = primary;

        while candidates.len() < hop_cap {
            let Some(next) = self.routes.get(current) else {
                break;
            };
            if !visited.insert(next.clone()) {
                break;
            }
            candidates.push(next.clone());
            current = next;
        }

        candidates
    }

    /// Executes `op` against `primary` from `registry`, failing over along
    /// [`Self::next_candidates`] strictly when [`is_safe_failover`] returns
    /// `true`.
    ///
    /// Returns on the first `Ok(value)` or the first non-safe-failover error
    /// (`TM-2`, `AC-3`). When all candidates fail with safe-failover errors,
    /// returns the last normalized [`PaymentError`] without exposing internal
    /// failover topology (`TM-4`).
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ConfigurationError`] if `primary` is not
    /// registered in `registry`, or the terminal [`PaymentError`] produced by
    /// `op`.
    pub async fn execute_with_failover<T, F, Fut>(
        &self,
        primary: &ProviderId,
        registry: &ProviderRegistry,
        op: F,
    ) -> Result<T, PaymentError>
    where
        F: FnMut(Arc<dyn PaymentProvider>) -> Fut,
        Fut: std::future::Future<Output = Result<T, PaymentError>>,
    {
        self.execute_with_resolver(primary, registry, op).await
    }

    async fn execute_with_resolver<T, L, F, Fut>(
        &self,
        primary: &ProviderId,
        lookup: &L,
        mut op: F,
    ) -> Result<T, PaymentError>
    where
        L: ProviderLookup,
        F: FnMut(Arc<dyn PaymentProvider>) -> Fut,
        Fut: std::future::Future<Output = Result<T, PaymentError>>,
    {
        let primary_provider = lookup.require(primary)?;
        let last_error = match classify_attempt(op(primary_provider).await) {
            AttemptOutcome::Final(result) => return result,
            AttemptOutcome::Retryable(err) => err,
        };

        let candidates = self.next_candidates(primary);
        execute_fallbacks(candidates, last_error, lookup, op).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use some_payment_core::{
        CreatePaymentRequest, ErrorKind, NormalizedWebhookEvent, PaymentCapabilities,
        PaymentResult, ProviderReference, RawWebhookRequest,
    };

    use super::*;

    #[derive(Debug)]
    struct StubProvider(ProviderId);

    #[async_trait]
    impl PaymentProvider for StubProvider {
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

    fn stub_map(slugs: &[&str]) -> HashMap<ProviderId, Arc<dyn PaymentProvider>> {
        slugs
            .iter()
            .map(|slug| {
                let id = ProviderId::new(*slug);
                let provider: Arc<dyn PaymentProvider> = Arc::new(StubProvider(id.clone()));
                (id, provider)
            })
            .collect()
    }

    impl ProviderLookup for HashMap<ProviderId, Arc<dyn PaymentProvider>> {
        fn require(&self, id: &ProviderId) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
            HashMap::get(self, id)
                .cloned()
                .ok_or_else(|| PaymentError::configuration("provider not registered"))
        }

        fn get(&self, id: &ProviderId) -> Option<Arc<dyn PaymentProvider>> {
            HashMap::get(self, id).cloned()
        }
    }

    fn pre_charge_unavailable(slug: &str) -> PaymentError {
        PaymentError::ProviderUnavailable {
            provider: ProviderId::new(slug),
            http_status: Some(503),
            indeterminate: false,
            message: "upstream 503 before charge".into(),
        }
    }

    fn ambiguous_timeout(slug: &str) -> PaymentError {
        PaymentError::ProviderUnavailable {
            provider: ProviderId::new(slug),
            http_status: None,
            indeterminate: true,
            message: "socket read timeout".into(),
        }
    }

    fn card_declined(slug: &str) -> PaymentError {
        PaymentError::CardDeclined {
            provider: ProviderId::new(slug),
            decline_code: Some("insufficient_funds".into()),
            advice_code: None,
            message: "declined".into(),
        }
    }

    #[test]
    fn is_safe_failover_allows_only_pre_charge_unavailable() {
        let p = ProviderId::new("paymob");
        assert!(is_safe_failover(&pre_charge_unavailable("paymob")));
        assert!(!is_safe_failover(&ambiguous_timeout("paymob")));
        assert!(!is_safe_failover(&card_declined("paymob")));

        let non_failover_errors = [
            PaymentError::ThreeDsFailed {
                provider: p.clone(),
                code: Some("3ds_failed".into()),
                message: "3ds failed".into(),
            },
            PaymentError::AuthenticationError {
                provider: p.clone(),
                message: "invalid api key".into(),
            },
            PaymentError::RateLimited {
                provider: p.clone(),
                retry_after_secs: Some(5),
            },
            PaymentError::validation("invalid amount"),
            PaymentError::configuration("missing provider"),
            PaymentError::unsupported(&p, "splits"),
            PaymentError::AbortedByHook {
                reason: "fraud block".into(),
            },
            PaymentError::WebhookVerificationFailed {
                provider: p.clone(),
                reason: "bad hmac".into(),
            },
            PaymentError::IdempotencyConflict {
                scope: "ord_1".into(),
            },
            PaymentError::ProviderError {
                provider: p,
                http_status: Some(500),
                vendor_code: Some("internal".into()),
                message: "post-send 500".into(),
            },
        ];

        for err in &non_failover_errors {
            assert!(!is_safe_failover(err), "unexpected failover for {err:?}");
        }
    }

    #[test]
    fn with_max_hops_enforces_valid_bounds() {
        assert_eq!(FallbackPolicy::default().max_hops(), MAX_FALLBACK_HOPS);

        let zero_err = FallbackPolicy::new().with_max_hops(0).unwrap_err();
        assert_eq!(zero_err.kind(), ErrorKind::Configuration);

        let over_err = FallbackPolicy::new()
            .with_max_hops(MAX_FALLBACK_HOPS + 1)
            .unwrap_err();
        assert_eq!(over_err.kind(), ErrorKind::Configuration);

        for hops in 1..=MAX_FALLBACK_HOPS {
            let policy = FallbackPolicy::new().with_max_hops(hops).unwrap();
            assert_eq!(policy.max_hops(), hops);
        }
    }

    #[test]
    fn next_candidates_breaks_cycles_and_caps_long_chains() {
        let a = ProviderId::new("alpha");
        let b = ProviderId::new("beta");
        let c = ProviderId::new("gamma");
        let d = ProviderId::new("delta");
        let e = ProviderId::new("epsilon");

        let self_cycle = FallbackPolicy::new().with_fallback(a.clone(), a.clone());
        assert!(self_cycle.next_candidates(&a).is_empty());

        let two_cycle = FallbackPolicy::new()
            .with_fallback(a.clone(), b.clone())
            .with_fallback(b.clone(), a.clone());
        assert_eq!(two_cycle.next_candidates(&a), vec![b.clone()]);

        let long_chain = FallbackPolicy::new()
            .with_fallback(a.clone(), b.clone())
            .with_fallback(b.clone(), c.clone())
            .with_fallback(c.clone(), d.clone())
            .with_fallback(d.clone(), e);
        assert_eq!(long_chain.next_candidates(&a), vec![b, c, d]);
    }

    async fn run_failover_scenario(primary_err: PaymentError, expect_fallback: bool) {
        let expected_kind = primary_err.kind();
        let paymob = ProviderId::new("paymob");
        let stripe = ProviderId::new("stripe");
        let map = stub_map(&["paymob", "stripe"]);
        let policy = FallbackPolicy::new().with_fallback(paymob.clone(), stripe);
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut pending_err = Some(primary_err);

        let result = policy
            .execute_with_resolver(&paymob, &map, |provider| {
                let calls = Arc::clone(&calls);
                let slug = provider.id().as_str().to_owned();
                let next_err = if slug == "paymob" {
                    pending_err.take()
                } else {
                    None
                };
                async move {
                    calls.lock().unwrap().push(slug.clone());
                    match next_err {
                        Some(err) => Err(err),
                        None => Ok(format!("charged:{slug}")),
                    }
                }
            })
            .await;

        if expect_fallback {
            assert_eq!(result.unwrap(), "charged:stripe");
            assert_eq!(*calls.lock().unwrap(), vec!["paymob", "stripe"]);
        } else {
            assert_eq!(result.unwrap_err().kind(), expected_kind);
            assert_eq!(*calls.lock().unwrap(), vec!["paymob"]);
        }
    }

    #[tokio::test]
    async fn pre_charge_unavailable_fails_over_to_fallback() {
        run_failover_scenario(pre_charge_unavailable("paymob"), true).await;
    }

    #[tokio::test]
    async fn card_declined_never_fails_over() {
        run_failover_scenario(card_declined("paymob"), false).await;
    }

    #[tokio::test]
    async fn ambiguous_timeout_never_fails_over() {
        run_failover_scenario(ambiguous_timeout("paymob"), false).await;
    }

    #[tokio::test]
    async fn cyclic_chain_terminates_cleanly_without_disclosing_topology() {
        let a = ProviderId::new("alpha");
        let b = ProviderId::new("beta");
        let map = stub_map(&["alpha", "beta"]);
        let policy = FallbackPolicy::new()
            .with_fallback(a.clone(), b.clone())
            .with_fallback(b, a.clone());
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));

        let err = policy
            .execute_with_resolver(&a, &map, |provider| {
                let calls = Arc::clone(&calls);
                async move {
                    let slug = provider.id().as_str().to_owned();
                    calls.lock().unwrap().push(slug.clone());
                    Err::<String, PaymentError>(pre_charge_unavailable(&slug))
                }
            })
            .await
            .unwrap_err();

        assert_eq!(*calls.lock().unwrap(), vec!["alpha", "beta"]);
        assert_eq!(err.kind(), ErrorKind::ProviderUnavailable);
        assert_eq!(err.provider().map(ProviderId::as_str), Some("beta"));
        assert!(!err.to_string().contains("alpha"));
        assert!(!format!("{err:?}").contains("alpha"));
    }

    #[tokio::test]
    async fn long_chain_execution_is_strictly_capped_at_max_hops() {
        let a = ProviderId::new("alpha");
        let b = ProviderId::new("beta");
        let c = ProviderId::new("gamma");
        let d = ProviderId::new("delta");
        let e = ProviderId::new("epsilon");
        let map = stub_map(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let policy = FallbackPolicy::new()
            .with_fallback(a.clone(), b.clone())
            .with_fallback(b, c.clone())
            .with_fallback(c, d.clone())
            .with_fallback(d, e);
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));

        let err = policy
            .execute_with_resolver(&a, &map, |provider| {
                let calls = Arc::clone(&calls);
                async move {
                    let slug = provider.id().as_str().to_owned();
                    calls.lock().unwrap().push(slug.clone());
                    Err::<String, PaymentError>(pre_charge_unavailable(&slug))
                }
            })
            .await
            .unwrap_err();

        assert_eq!(*calls.lock().unwrap(), vec!["alpha", "beta", "gamma", "delta"]);
        assert_eq!(err.provider().map(ProviderId::as_str), Some("delta"));
    }

    #[tokio::test]
    async fn unregistered_fallback_is_skipped_and_missing_primary_fails_closed() {
        let a = ProviderId::new("alpha");
        let b = ProviderId::new("beta");
        let c = ProviderId::new("gamma");
        let missing = ProviderId::new("missing");
        let map = stub_map(&["alpha", "gamma"]);
        let policy = FallbackPolicy::new()
            .with_fallback(a.clone(), b.clone())
            .with_fallback(b, c);

        let missing_err = policy
            .execute_with_resolver(&missing, &map, |_| async {
                Ok::<String, PaymentError>("never".into())
            })
            .await
            .unwrap_err();
        assert_eq!(missing_err.kind(), ErrorKind::Configuration);

        let outcome = policy
            .execute_with_resolver(&a, &map, |provider| async move {
                if provider.id().as_str() == "alpha" {
                    Err(pre_charge_unavailable("alpha"))
                } else {
                    Ok(provider.id().as_str().to_owned())
                }
            })
            .await
            .unwrap();
        assert_eq!(outcome, "gamma");
    }

    #[tokio::test]
    async fn non_safe_error_on_fallback_stops_chain_immediately() {
        let a = ProviderId::new("alpha");
        let b = ProviderId::new("beta");
        let c = ProviderId::new("gamma");
        let map = stub_map(&["alpha", "beta", "gamma"]);
        let policy = FallbackPolicy::new()
            .with_fallback(a.clone(), b.clone())
            .with_fallback(b, c);
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));

        let err = policy
            .execute_with_resolver(&a, &map, |provider| {
                let calls = Arc::clone(&calls);
                async move {
                    let slug = provider.id().as_str().to_owned();
                    calls.lock().unwrap().push(slug.clone());
                    if slug == "alpha" {
                        Err(pre_charge_unavailable("alpha"))
                    } else {
                        Err::<String, PaymentError>(card_declined(&slug))
                    }
                }
            })
            .await
            .unwrap_err();

        assert_eq!(*calls.lock().unwrap(), vec!["alpha", "beta"]);
        assert_eq!(err.kind(), ErrorKind::CardDeclined);
        assert_eq!(err.provider().map(ProviderId::as_str), Some("beta"));
    }
}
