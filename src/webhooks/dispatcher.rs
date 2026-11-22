//! [`WebhookDispatcher`] verifying raw webhooks via provider lookup before
//! deduplication and enforcing secret-scrubbing invariants.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::BuildHasher;
use std::sync::Arc;

use serde_json::{Map, Value};
use some_payment_core::{
    NormalizedWebhookEvent, PaymentError, PaymentProvider, ProviderId, RawWebhookRequest,
    VerificationChallenge,
};

use super::dedup_guard::WebhookDeduplicationStore;

/// Default maximum accepted raw webhook body size (1 MiB).
///
/// Enforced prior to provider lookup, signature verification, and deduplication
/// to defend against oversized payload denial-of-service attacks (TM-5).
///
/// # Examples
///
/// ```
/// use some_payment::webhooks::dispatcher::DEFAULT_MAX_WEBHOOK_BYTES;
///
/// assert_eq!(DEFAULT_MAX_WEBHOOK_BYTES, 1_048_576);
/// ```
pub const DEFAULT_MAX_WEBHOOK_BYTES: usize = 1_048_576;

const REDACTED_SENTINEL: &str = "[REDACTED]";
const MAX_JSON_INSPECTION_DEPTH: usize = 32;

const FORBIDDEN_SECRET_KEYS: &[&str] = &[
    "secret_token",
    "api_key",
    "secret_key",
    "client_secret",
    "webhook_secret",
    "hmac_secret",
    "private_key",
];

/// Lookup abstraction resolving a [`ProviderId`] to its registered [`PaymentProvider`].
///
/// Concrete implementations are provided for [`BTreeMap`] and [`HashMap`] so callers
/// can plug in either an ordered registry or a hashed map without extra wrappers.
pub trait WebhookProviderLookup: Send + Sync {
    /// Returns the registered provider matching `id`, or `None` if unknown.
    fn get_provider(&self, id: &ProviderId) -> Option<Arc<dyn PaymentProvider>>;
}

impl WebhookProviderLookup for BTreeMap<ProviderId, Arc<dyn PaymentProvider>> {
    fn get_provider(&self, id: &ProviderId) -> Option<Arc<dyn PaymentProvider>> {
        self.get(id).map(Arc::clone)
    }
}

impl<S> WebhookProviderLookup for HashMap<ProviderId, Arc<dyn PaymentProvider>, S>
where
    S: BuildHasher + Send + Sync,
{
    fn get_provider(&self, id: &ProviderId) -> Option<Arc<dyn PaymentProvider>> {
        self.get(id).map(Arc::clone)
    }
}

/// Classification returned by [`WebhookDispatcher::handle_webhook`].
///
/// Sensitive fields inside [`VerificationChallenge`] remain redacted in [`fmt::Debug`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum WebhookOutcome {
    /// Verified, secret-scrubbed, newly recorded webhook event ready for fulfillment.
    Processed(NormalizedWebhookEvent),
    /// Verified webhook event whose `(provider_id, event_id)` was already processed.
    Duplicate {
        /// Provider that originated the duplicate delivery.
        provider_id: ProviderId,
        /// Provider-unique event identifier that collided in the dedup store.
        event_id: String,
    },
    /// Verified one-shot provider verification handshake (e.g., Moyasar) requiring
    /// an immediate `200 OK` response without touching the dedup or order ledger.
    VerificationHandshake(VerificationChallenge),
}

/// Unified multi-gateway webhook dispatcher enforcing payload size caps,
/// verify-before-dedup ordering, secret-scrub assertions, and handshake routing.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use std::sync::Arc;
///
/// use async_trait::async_trait;
/// use some_payment::webhooks::dedup_guard::InMemoryDedupStore;
/// use some_payment::webhooks::dispatcher::{WebhookDispatcher, WebhookOutcome};
/// use some_payment_core::{
///     CreatePaymentRequest, HttpMethod, NormalizedWebhookEvent, PaymentCapabilities,
///     PaymentError, PaymentProvider, PaymentResult, PaymentStatus, ProviderId,
///     ProviderReference, RawWebhookRequest,
/// };
///
/// #[derive(Debug)]
/// struct StubProvider(ProviderId);
///
/// #[async_trait]
/// impl PaymentProvider for StubProvider {
///     fn id(&self) -> &ProviderId {
///         &self.0
///     }
///     fn capabilities(&self) -> PaymentCapabilities {
///         PaymentCapabilities::empty()
///     }
///     async fn create_payment(
///         &self,
///         _: &CreatePaymentRequest,
///     ) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "create_payment"))
///     }
///     async fn get_payment(
///         &self,
///         _: &ProviderReference,
///     ) -> Result<PaymentResult, PaymentError> {
///         Err(PaymentError::unsupported(&self.0, "get_payment"))
///     }
///     async fn parse_and_verify_webhook(
///         &self,
///         _: &RawWebhookRequest,
///     ) -> Result<NormalizedWebhookEvent, PaymentError> {
///         Ok(NormalizedWebhookEvent {
///             provider_id: self.0.clone(),
///             event_id: "evt_1".into(),
///             event_type: "payment_intent.succeeded".into(),
///             livemode: false,
///             requires_hydration: false,
///             reference: None,
///             status: Some(PaymentStatus::Paid),
///             money: None,
///             verification_challenge: None,
///             raw_redacted: serde_json::json!({"id": "evt_1"}),
///         })
///     }
/// }
///
/// let mut providers: BTreeMap<ProviderId, Arc<dyn PaymentProvider>> = BTreeMap::new();
/// providers.insert(
///     ProviderId::new("stripe"),
///     Arc::new(StubProvider(ProviderId::new("stripe"))),
/// );
/// let dispatcher = WebhookDispatcher::new(providers, InMemoryDedupStore::new())
///     .with_max_bytes(64 * 1024)
///     .unwrap();
/// let raw = RawWebhookRequest {
///     raw_body: b"{}".to_vec(),
///     headers: BTreeMap::new(),
///     query_params: BTreeMap::new(),
///     method: HttpMethod::Post,
/// };
/// let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
/// let first = rt.block_on(dispatcher.handle_webhook("stripe", &raw)).unwrap();
/// assert!(matches!(first, WebhookOutcome::Processed(_)));
/// let replay = rt.block_on(dispatcher.handle_webhook("stripe", &raw)).unwrap();
/// assert!(matches!(replay, WebhookOutcome::Duplicate { .. }));
/// ```
pub struct WebhookDispatcher<P, D> {
    providers: P,
    dedup: D,
    max_bytes: usize,
}

impl<P, D> fmt::Debug for WebhookDispatcher<P, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookDispatcher")
            .field("max_bytes", &self.max_bytes)
            .finish_non_exhaustive()
    }
}

impl<P, D> WebhookDispatcher<P, D> {
    /// Creates a new dispatcher using [`DEFAULT_MAX_WEBHOOK_BYTES`] (1 MiB).
    #[must_use]
    pub fn new(providers: P, dedup: D) -> Self {
        Self {
            providers,
            dedup,
            max_bytes: DEFAULT_MAX_WEBHOOK_BYTES,
        }
    }

    /// Overrides the maximum accepted raw webhook body size in bytes.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] when `max_bytes` is zero.
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Result<Self, PaymentError> {
        if max_bytes == 0 {
            return Err(PaymentError::validation(
                "max_bytes must be greater than zero",
            ));
        }
        self.max_bytes = max_bytes;
        Ok(self)
    }

    /// Returns the configured maximum raw webhook payload size in bytes.
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

impl<P, D> WebhookDispatcher<P, D>
where
    P: WebhookProviderLookup,
    D: WebhookDeduplicationStore,
{
    /// Verifies, scrubs, routes, and deduplicates an incoming raw webhook request.
    ///
    /// Execution order is strictly enforced:
    /// 1. Payload byte-cap check (`raw.raw_body.len() <= max_bytes`).
    /// 2. Provider slug validation and provider registry lookup.
    /// 3. Cryptographic/token verification via [`PaymentProvider::parse_and_verify_webhook`].
    /// 4. Recursive secret-scrub assertion over `event.raw_redacted`.
    /// 5. Early return for [`WebhookOutcome::VerificationHandshake`] without dedup access.
    /// 6. Non-empty `event_id` validation and atomic deduplication via [`WebhookDeduplicationStore`].
    ///
    /// # Errors
    ///
    /// Returns:
    /// - [`PaymentError::ValidationError`] for oversized payloads, invalid provider slugs,
    ///   or empty `event_id` values.
    /// - [`PaymentError::ConfigurationError`] when no provider is registered for `provider_slug`.
    /// - [`PaymentError::WebhookVerificationFailed`] when provider verification fails,
    ///   unscrubbed secret fields appear in `raw_redacted`, or JSON nesting exceeds the depth cap.
    /// - Any error propagated by [`WebhookDeduplicationStore::record_if_new`].
    pub async fn handle_webhook(
        &self,
        provider_slug: &str,
        raw: &RawWebhookRequest,
    ) -> Result<WebhookOutcome, PaymentError> {
        let (provider_id, provider) = self.resolve_for_request(provider_slug, raw)?;
        let event = verify_with_provider(provider.as_ref(), &provider_id, raw).await?;
        assert_raw_redacted_scrubbed(&provider_id, &event.raw_redacted)?;

        if let Some(challenge) = event.verification_challenge {
            return Ok(WebhookOutcome::VerificationHandshake(challenge));
        }

        self.record_and_classify(event).await
    }

    fn resolve_for_request(
        &self,
        provider_slug: &str,
        raw: &RawWebhookRequest,
    ) -> Result<(ProviderId, Arc<dyn PaymentProvider>), PaymentError> {
        self.check_payload_size(raw)?;
        let provider_id = ProviderId::try_new(provider_slug)?;
        let provider = self.resolve_provider(&provider_id)?;
        Ok((provider_id, provider))
    }

    fn check_payload_size(&self, raw: &RawWebhookRequest) -> Result<(), PaymentError> {
        if raw.raw_body.len() > self.max_bytes {
            return Err(PaymentError::validation(
                "webhook payload exceeds maximum allowed size",
            ));
        }
        Ok(())
    }

    fn resolve_provider(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        self.providers.get_provider(provider_id).ok_or_else(|| {
            PaymentError::configuration(format!("unknown webhook provider: {provider_id}"))
        })
    }

    async fn record_and_classify(
        &self,
        event: NormalizedWebhookEvent,
    ) -> Result<WebhookOutcome, PaymentError> {
        if event.event_id.trim().is_empty() {
            return Err(PaymentError::validation(
                "webhook event_id must not be empty",
            ));
        }

        let is_new = self
            .dedup
            .record_if_new(&event.provider_id, &event.event_id)
            .await?;
        if !is_new {
            return Ok(WebhookOutcome::Duplicate {
                provider_id: event.provider_id,
                event_id: event.event_id,
            });
        }

        Ok(WebhookOutcome::Processed(event))
    }
}

async fn verify_with_provider(
    provider: &dyn PaymentProvider,
    provider_id: &ProviderId,
    raw: &RawWebhookRequest,
) -> Result<NormalizedWebhookEvent, PaymentError> {
    match provider.parse_and_verify_webhook(raw).await {
        Ok(event) => Ok(event),
        Err(err) => {
            tracing::warn!(
                provider = %provider_id,
                error_kind = ?err.kind(),
                "webhook verification failed; rejecting before deduplication"
            );
            Err(err)
        }
    }
}

fn assert_raw_redacted_scrubbed(
    provider_id: &ProviderId,
    value: &Value,
) -> Result<(), PaymentError> {
    inspect_json_value(provider_id, value, 0)
}

fn inspect_json_value(
    provider_id: &ProviderId,
    value: &Value,
    depth: usize,
) -> Result<(), PaymentError> {
    if depth > MAX_JSON_INSPECTION_DEPTH {
        return Err(PaymentError::WebhookVerificationFailed {
            provider: provider_id.clone(),
            reason: "raw_redacted exceeds maximum JSON nesting depth".into(),
        });
    }
    match value {
        Value::Object(map) => inspect_json_object(provider_id, map, depth),
        Value::Array(items) => inspect_json_array(provider_id, items, depth),
        _ => Ok(()),
    }
}

fn inspect_json_object(
    provider_id: &ProviderId,
    map: &Map<String, Value>,
    depth: usize,
) -> Result<(), PaymentError> {
    for (key, val) in map {
        check_object_entry(provider_id, key, val, depth)?;
    }
    Ok(())
}

fn check_object_entry(
    provider_id: &ProviderId,
    key: &str,
    val: &Value,
    depth: usize,
) -> Result<(), PaymentError> {
    if is_forbidden_secret_key(key) && !is_scrubbed_secret_value(val) {
        tracing::warn!(
            provider = %provider_id,
            "unscrubbed secret detected in webhook raw_redacted; rejecting event"
        );
        return Err(PaymentError::WebhookVerificationFailed {
            provider: provider_id.clone(),
            reason: "unscrubbed secret detected in raw_redacted".into(),
        });
    }
    inspect_json_value(provider_id, val, depth + 1)
}

fn inspect_json_array(
    provider_id: &ProviderId,
    items: &[Value],
    depth: usize,
) -> Result<(), PaymentError> {
    for item in items {
        inspect_json_value(provider_id, item, depth + 1)?;
    }
    Ok(())
}

fn is_forbidden_secret_key(key: &str) -> bool {
    FORBIDDEN_SECRET_KEYS
        .iter()
        .any(|forbidden| key.eq_ignore_ascii_case(forbidden))
}

fn is_scrubbed_secret_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text == REDACTED_SENTINEL,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{Map, Value, json};
    use some_payment_core::{
        CreatePaymentRequest, ErrorKind, HttpMethod, NormalizedWebhookEvent, PaymentCapabilities,
        PaymentError, PaymentProvider, PaymentResult, PaymentStatus, ProviderId, ProviderReference,
        RawWebhookRequest, VerificationChallenge,
    };

    use super::*;

    #[allow(clippy::large_enum_variant)]
    enum MockOutcome {
        Verified(NormalizedWebhookEvent),
        VerifyFailed,
    }

    struct MockProvider {
        id: ProviderId,
        verify_calls: Arc<AtomicUsize>,
        outcome: MockOutcome,
    }

    impl fmt::Debug for MockProvider {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("MockProvider")
                .field("id", &self.id)
                .finish_non_exhaustive()
        }
    }

    #[async_trait]
    impl PaymentProvider for MockProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn capabilities(&self) -> PaymentCapabilities {
            PaymentCapabilities::empty()
        }

        async fn create_payment(
            &self,
            _: &CreatePaymentRequest,
        ) -> Result<PaymentResult, PaymentError> {
            Err(PaymentError::unsupported(&self.id, "create_payment"))
        }

        async fn get_payment(&self, _: &ProviderReference) -> Result<PaymentResult, PaymentError> {
            Err(PaymentError::unsupported(&self.id, "get_payment"))
        }

        async fn parse_and_verify_webhook(
            &self,
            _: &RawWebhookRequest,
        ) -> Result<NormalizedWebhookEvent, PaymentError> {
            self.verify_calls.fetch_add(1, Ordering::SeqCst);
            match &self.outcome {
                MockOutcome::Verified(event) => Ok(event.clone()),
                MockOutcome::VerifyFailed => Err(PaymentError::WebhookVerificationFailed {
                    provider: self.id.clone(),
                    reason: "invalid signature".into(),
                }),
            }
        }
    }

    #[derive(Default)]
    struct CountingDedup {
        calls: Arc<AtomicUsize>,
        seen: Mutex<HashSet<(ProviderId, String)>>,
    }

    impl CountingDedup {
        fn new(calls: Arc<AtomicUsize>) -> Self {
            Self {
                calls,
                seen: Mutex::new(HashSet::new()),
            }
        }
    }

    #[async_trait]
    impl WebhookDeduplicationStore for CountingDedup {
        async fn record_if_new(
            &self,
            provider_id: &ProviderId,
            event_id: &str,
        ) -> Result<bool, PaymentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut guard = self.seen.lock().unwrap();
            Ok(guard.insert((provider_id.clone(), event_id.to_owned())))
        }
    }

    fn raw_request_with_body(body: Vec<u8>) -> RawWebhookRequest {
        RawWebhookRequest {
            raw_body: body,
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            method: HttpMethod::Post,
        }
    }

    fn sample_event(provider: &str, event_id: &str, raw_redacted: Value) -> NormalizedWebhookEvent {
        NormalizedWebhookEvent {
            provider_id: ProviderId::new(provider),
            event_id: event_id.into(),
            event_type: "payment_intent.succeeded".into(),
            livemode: false,
            requires_hydration: false,
            reference: None,
            status: Some(PaymentStatus::Paid),
            money: None,
            verification_challenge: None,
            raw_redacted,
        }
    }

    type TestDispatcher =
        WebhookDispatcher<BTreeMap<ProviderId, Arc<dyn PaymentProvider>>, CountingDedup>;

    fn build_dispatcher(
        slug: &str,
        outcome: MockOutcome,
    ) -> (TestDispatcher, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let verify_calls = Arc::new(AtomicUsize::new(0));
        let dedup_calls = Arc::new(AtomicUsize::new(0));
        let mut providers: BTreeMap<ProviderId, Arc<dyn PaymentProvider>> = BTreeMap::new();
        providers.insert(
            ProviderId::new(slug),
            Arc::new(MockProvider {
                id: ProviderId::new(slug),
                verify_calls: Arc::clone(&verify_calls),
                outcome,
            }),
        );
        let dedup = CountingDedup::new(Arc::clone(&dedup_calls));
        (
            WebhookDispatcher::new(providers, dedup),
            verify_calls,
            dedup_calls,
        )
    }

    #[tokio::test]
    async fn tm5_oversized_payload_rejected_before_provider_and_dedup() {
        let event = sample_event("stripe", "evt_ok", json!({"id": "evt_ok"}));
        let (dispatcher, verify_calls, dedup_calls) =
            build_dispatcher("stripe", MockOutcome::Verified(event));
        let dispatcher = dispatcher.with_max_bytes(16).unwrap();
        assert_eq!(dispatcher.max_bytes(), 16);

        let oversized = raw_request_with_body(vec![b'x'; 17]);
        let err = dispatcher
            .handle_webhook("stripe", &oversized)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
        assert_eq!(verify_calls.load(Ordering::SeqCst), 0);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 0);

        let exact = raw_request_with_body(vec![b'x'; 16]);
        let ok = dispatcher.handle_webhook("stripe", &exact).await.unwrap();
        assert!(matches!(ok, WebhookOutcome::Processed(_)));
        assert_eq!(verify_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn with_max_bytes_rejects_zero() {
        let providers: BTreeMap<ProviderId, Arc<dyn PaymentProvider>> = BTreeMap::new();
        let err = WebhookDispatcher::new(providers, CountingDedup::default())
            .with_max_bytes(0)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
    }

    #[tokio::test]
    async fn invalid_slug_and_unknown_provider_rejected_and_hashmap_lookup_works() {
        let verify_calls = Arc::new(AtomicUsize::new(0));
        let dedup_calls = Arc::new(AtomicUsize::new(0));
        let event = sample_event("paymob", "evt_pm_1", json!({"id": "evt_pm_1"}));
        let mut providers: HashMap<ProviderId, Arc<dyn PaymentProvider>> = HashMap::new();
        providers.insert(
            ProviderId::new("paymob"),
            Arc::new(MockProvider {
                id: ProviderId::new("paymob"),
                verify_calls: Arc::clone(&verify_calls),
                outcome: MockOutcome::Verified(event),
            }),
        );
        let dedup = CountingDedup::new(Arc::clone(&dedup_calls));
        let dispatcher = WebhookDispatcher::new(providers, dedup);
        let raw = raw_request_with_body(b"{}".to_vec());

        let invalid_err = dispatcher
            .handle_webhook("../evil", &raw)
            .await
            .unwrap_err();
        assert_eq!(invalid_err.kind(), ErrorKind::Validation);

        let unknown_err = dispatcher.handle_webhook("stripe", &raw).await.unwrap_err();
        assert_eq!(unknown_err.kind(), ErrorKind::Configuration);
        assert_eq!(verify_calls.load(Ordering::SeqCst), 0);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 0);

        let outcome = dispatcher.handle_webhook("  PayMob ", &raw).await.unwrap();
        assert!(matches!(outcome, WebhookOutcome::Processed(_)));
        assert_eq!(verify_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ac1_tm1_failed_verification_skips_dedup_store() {
        let (dispatcher, verify_calls, dedup_calls) =
            build_dispatcher("stripe", MockOutcome::VerifyFailed);
        let raw = raw_request_with_body(b"{}".to_vec());

        let err = dispatcher.handle_webhook("stripe", &raw).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WebhookVerificationFailed);
        assert_eq!(verify_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn ac2_tc322_dedup_first_delivery_processed_and_replay_duplicate() {
        let expected_event = sample_event("stripe", "evt_123", json!({"id": "evt_123"}));
        let (dispatcher, verify_calls, dedup_calls) =
            build_dispatcher("stripe", MockOutcome::Verified(expected_event.clone()));
        let raw = raw_request_with_body(b"{}".to_vec());

        let first = dispatcher.handle_webhook("stripe", &raw).await.unwrap();
        assert_eq!(first, WebhookOutcome::Processed(expected_event));

        let replay = dispatcher.handle_webhook("stripe", &raw).await.unwrap();
        assert_eq!(
            replay,
            WebhookOutcome::Duplicate {
                provider_id: ProviderId::new("stripe"),
                event_id: "evt_123".into(),
            }
        );
        assert_eq!(verify_calls.load(Ordering::SeqCst), 2);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn ac3_verification_handshake_routes_without_dedup() {
        let mut event = sample_event(
            "moyasar",
            "evt_verify_1",
            json!({"secret_token": "[REDACTED]", "type": "webhook_verification"}),
        );
        event.verification_challenge = Some(VerificationChallenge {
            webhook_id: "wh_moy_1".into(),
            verification_code: "654321".into(),
        });
        let (dispatcher, verify_calls, dedup_calls) =
            build_dispatcher("moyasar", MockOutcome::Verified(event));
        let raw = raw_request_with_body(b"{}".to_vec());

        let outcome = dispatcher.handle_webhook("moyasar", &raw).await.unwrap();
        assert_eq!(
            outcome,
            WebhookOutcome::VerificationHandshake(VerificationChallenge {
                webhook_id: "wh_moy_1".into(),
                verification_code: "654321".into(),
            })
        );
        assert_eq!(verify_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 0);

        let dbg = format!("{outcome:?}");
        assert!(!dbg.contains("654321"));
        assert!(dbg.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn tm4_tc322_unscrubbed_secrets_rejected_recursively_before_dedup() {
        let raw = raw_request_with_body(b"{}".to_vec());
        for forbidden_key in FORBIDDEN_SECRET_KEYS {
            let mut secret_obj = Map::new();
            secret_obj.insert(
                forbidden_key.to_ascii_uppercase(),
                Value::String("CANARY_SECRET_LEAK".into()),
            );
            let payload = json!({ "data": [{ "nested": Value::Object(secret_obj) }] });
            let event = sample_event("moyasar", "evt_leak", payload);
            let (dispatcher, verify_calls, dedup_calls) =
                build_dispatcher("moyasar", MockOutcome::Verified(event));

            let err = dispatcher
                .handle_webhook("moyasar", &raw)
                .await
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::WebhookVerificationFailed);
            assert_eq!(verify_calls.load(Ordering::SeqCst), 1);
            assert_eq!(dedup_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn tm4_redacted_and_null_secret_keys_are_accepted() {
        let payload = json!({
            "secret_token": "[REDACTED]",
            "nested": { "API_KEY": "[REDACTED]", "private_key": null }
        });
        let event = sample_event("moyasar", "evt_clean", payload);
        let (dispatcher, _, dedup_calls) =
            build_dispatcher("moyasar", MockOutcome::Verified(event));
        let raw = raw_request_with_body(b"{}".to_vec());

        let outcome = dispatcher.handle_webhook("moyasar", &raw).await.unwrap();
        assert!(matches!(outcome, WebhookOutcome::Processed(_)));
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tm4_excessive_json_nesting_depth_rejected_safely() {
        let mut deep = json!("leaf");
        for _ in 0..=MAX_JSON_INSPECTION_DEPTH {
            deep = json!({ "a": deep });
        }
        let event = sample_event("stripe", "evt_deep", deep);
        let (dispatcher, _, dedup_calls) = build_dispatcher("stripe", MockOutcome::Verified(event));
        let raw = raw_request_with_body(b"{}".to_vec());

        let err = dispatcher.handle_webhook("stripe", &raw).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WebhookVerificationFailed);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn empty_event_id_rejected_before_dedup() {
        let event = sample_event("stripe", "   ", json!({}));
        let (dispatcher, _, dedup_calls) = build_dispatcher("stripe", MockOutcome::Verified(event));
        let raw = raw_request_with_body(b"{}".to_vec());

        let err = dispatcher.handle_webhook("stripe", &raw).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
        assert_eq!(dedup_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn dispatcher_debug_omits_provider_and_dedup_internals() {
        let providers: BTreeMap<ProviderId, Arc<dyn PaymentProvider>> = BTreeMap::new();
        let dispatcher = WebhookDispatcher::new(providers, CountingDedup::default());
        let dbg = format!("{dispatcher:?}");
        assert!(dbg.contains("WebhookDispatcher"));
        assert!(dbg.contains("1048576"));
    }
}
