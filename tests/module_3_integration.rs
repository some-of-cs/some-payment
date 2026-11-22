//! End-to-end Module 3 integration tests wiring [`WebhookDispatcher`],
//! [`InMemoryDedupStore`], [`ValidatedRawWebhook`], and
//! [`route_verification_handshake`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::json;
use some_payment::webhooks::{
    HandshakeRouteOutcome, InMemoryDedupStore, ValidatedRawWebhook, VerificationHandshakeHandler,
    WebhookDispatcher, WebhookOutcome, route_verification_handshake,
};
use some_payment_core::{
    CreatePaymentRequest, HttpMethod, NormalizedWebhookEvent, PaymentCapabilities, PaymentError,
    PaymentProvider, PaymentResult, PaymentStatus, ProviderId, ProviderReference,
    RawWebhookRequest, VerificationChallenge,
};

#[derive(Debug)]
struct StubWebhookProvider {
    id: ProviderId,
    event: NormalizedWebhookEvent,
}

#[async_trait]
impl PaymentProvider for StubWebhookProvider {
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
        Ok(self.event.clone())
    }
}

struct CountingHandshakeHandler(AtomicUsize);

#[async_trait]
impl VerificationHandshakeHandler for CountingHandshakeHandler {
    async fn handle_verification(
        &self,
        _provider_id: &ProviderId,
        _challenge: &VerificationChallenge,
    ) -> Result<(), PaymentError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn validated_post_request(body: &[u8]) -> ValidatedRawWebhook {
    ValidatedRawWebhook::try_new(RawWebhookRequest {
        raw_body: body.to_vec(),
        headers: BTreeMap::new(),
        query_params: BTreeMap::new(),
        method: HttpMethod::Post,
    })
    .unwrap()
}

fn build_event(
    provider: &str,
    event_id: &str,
    challenge: Option<VerificationChallenge>,
) -> NormalizedWebhookEvent {
    NormalizedWebhookEvent {
        provider_id: ProviderId::new(provider),
        event_id: event_id.into(),
        event_type: "payment_paid".into(),
        livemode: false,
        requires_hydration: false,
        reference: None,
        status: Some(PaymentStatus::Paid),
        money: None,
        verification_challenge: challenge,
        raw_redacted: json!({"secret_token": "[REDACTED]"}),
    }
}

#[tokio::test]
async fn dispatcher_wires_directly_with_in_memory_dedup_store_and_validated_raw_webhook() {
    let event = build_event("stripe", "evt_int_001", None);
    let mut providers: BTreeMap<ProviderId, Arc<dyn PaymentProvider>> = BTreeMap::new();
    providers.insert(
        ProviderId::new("stripe"),
        Arc::new(StubWebhookProvider {
            id: ProviderId::new("stripe"),
            event: event.clone(),
        }),
    );

    let dispatcher = WebhookDispatcher::new(providers, InMemoryDedupStore::new());
    let validated = validated_post_request(br#"{"id":"evt_int_001"}"#);

    let first = dispatcher
        .handle_webhook("stripe", validated.as_raw())
        .await
        .unwrap();
    let replay = dispatcher
        .handle_webhook("stripe", validated.as_raw())
        .await
        .unwrap();

    assert_eq!(first, WebhookOutcome::Processed(event));
    assert_eq!(
        replay,
        WebhookOutcome::Duplicate {
            provider_id: ProviderId::new("stripe"),
            event_id: "evt_int_001".into(),
        }
    );
}

#[tokio::test]
async fn dispatcher_and_handshake_router_handle_moyasar_verification_end_to_end() {
    let challenge = VerificationChallenge {
        webhook_id: "wh_moyasar_int".into(),
        verification_code: "482910".into(),
    };
    let event = build_event("moyasar", "evt_verify_int", Some(challenge.clone()));
    let mut providers: BTreeMap<ProviderId, Arc<dyn PaymentProvider>> = BTreeMap::new();
    providers.insert(
        ProviderId::new("moyasar"),
        Arc::new(StubWebhookProvider {
            id: ProviderId::new("moyasar"),
            event: event.clone(),
        }),
    );

    let dispatcher = WebhookDispatcher::new(providers, InMemoryDedupStore::new());
    let validated = validated_post_request(br#"{"type":"webhook_verification"}"#);

    let outcome = dispatcher
        .handle_webhook("moyasar", validated.as_raw())
        .await
        .unwrap();
    assert_eq!(outcome, WebhookOutcome::VerificationHandshake(challenge));

    let handler = CountingHandshakeHandler(AtomicUsize::new(0));
    let routed = route_verification_handshake(&event, &handler)
        .await
        .unwrap();
    assert_eq!(
        routed,
        HandshakeRouteOutcome::Handled {
            provider_id: ProviderId::new("moyasar"),
            webhook_id: "wh_moyasar_int".into(),
        }
    );
    assert_eq!(handler.0.load(Ordering::SeqCst), 1);
}
