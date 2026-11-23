//! Five-step payment orchestrator execution pipeline (`US-2.1.0` / Unit 3).
//!
//! [`PaymentOrchestrator`] (aliased as [`PaymentHub`]) executes a uniform
//! five-step template method across all six core payment operations:
//!
//! 1. Validate request invariants and positive monetary amounts.
//! 2. Resolve the target [`PaymentProvider`] (explicit `provider_id` always
//!    takes precedence over any configured [`RouteResolver`] or default).
//! 3. Enforce fail-closed [`PaymentCapabilities`] guards prior to I/O.
//! 4. Run composable `before` hooks via [`HookChain`] (aborting with zero
//!    provider calls if rejected).
//! 5. Dispatch to the resolved provider and execute non-reversible `after` /
//!    `after_refund` or `on_error` hooks.
//!
//! This module depends strictly on [`some_payment_core`], [`crate::registry`],
//! [`crate::hooks`], and `std` — with zero references to concrete providers.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use some_payment_core::{
    CaptureMode, CaptureRequest, CheckoutSessionRequest, CreatePaymentRequest, CurrencyCode, Money,
    PaymentCapabilities, PaymentError, PaymentProvider, PaymentResult, ProviderId,
    ProviderReference, RefundRequest, RefundResult, VoidRequest,
};

use crate::hooks::{HookChain, HookContext, PaymentOperation};
use crate::registry::ProviderRegistry;

/// Ergonomic alias for [`PaymentOrchestrator`].
pub type PaymentHub = PaymentOrchestrator;

/// Pluggable routing callback invoked when a request omits an explicit [`ProviderId`].
///
/// Receives the immutable [`ProviderRegistry`], an optional explicit provider ID,
/// an optional ISO-4217 [`CurrencyCode`], an optional ISO-3166-1 alpha-2 buyer
/// country code, and the required [`PaymentCapabilities`] bitmask.
pub type RouteResolver = Arc<
    dyn Fn(
            &ProviderRegistry,
            Option<&ProviderId>,
            Option<&CurrencyCode>,
            Option<&str>,
            PaymentCapabilities,
        ) -> Result<Arc<dyn PaymentProvider>, PaymentError>
        + Send
        + Sync,
>;

/// Provider-agnostic orchestrator executing the uniform five-step payment pipeline.
///
/// Cloning a [`PaymentOrchestrator`] is inexpensive because its registry,
/// hook chain, and optional route resolver are all internally reference-counted.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use std::sync::Arc;
///
/// use async_trait::async_trait;
/// use some_payment::hooks::HookChain;
/// use some_payment::orchestrator::PaymentOrchestrator;
/// use some_payment::registry::ProviderRegistry;
/// use some_payment_core::{
///     CreatePaymentRequest, Money, NormalizedWebhookEvent, PaymentCapabilities,
///     PaymentError, PaymentProvider, PaymentResult, PaymentStatus, ProviderId,
///     ProviderReference, RawWebhookRequest,
/// };
///
/// #[derive(Debug)]
/// struct EchoProvider(ProviderId);
///
/// #[async_trait]
/// impl PaymentProvider for EchoProvider {
///     fn id(&self) -> &ProviderId {
///         &self.0
///     }
///
///     fn capabilities(&self) -> PaymentCapabilities {
///         PaymentCapabilities::DIRECT_PAYMENT
///     }
///
///     async fn create_payment(
///         &self,
///         req: &CreatePaymentRequest,
///     ) -> Result<PaymentResult, PaymentError> {
///         let reference = ProviderReference::builder(self.0.clone())
///             .payment_id("pay_doc_1")
///             .build()?;
///         Ok(PaymentResult {
///             reference,
///             status: PaymentStatus::Paid,
///             money: req.money.clone(),
///             captured_amount: Some(req.money.amount()),
///             refunded_amount: None,
///             next_action: None,
///             checkout_url: None,
///             livemode: false,
///         })
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
///         Err(PaymentError::unsupported(&self.0, "parse_and_verify_webhook"))
///     }
/// }
///
/// let id = ProviderId::new("echo");
/// let mut providers: HashMap<ProviderId, Arc<dyn PaymentProvider>> = HashMap::new();
/// providers.insert(id.clone(), Arc::new(EchoProvider(id.clone())));
///
/// let registry = ProviderRegistry::new(providers, id).unwrap();
/// let orchestrator = PaymentOrchestrator::new(registry, HookChain::default());
///
/// let req = CreatePaymentRequest::new(Money::new(10_000, "SAR").unwrap(), "ORD-DOC-1");
/// let rt = tokio::runtime::Builder::new_current_thread()
///     .build()
///     .unwrap();
/// let result = rt.block_on(orchestrator.create_payment(&req)).unwrap();
/// assert_eq!(result.status, PaymentStatus::Paid);
/// assert_eq!(result.reference.payment_id(), Some("pay_doc_1"));
/// ```
#[derive(Clone)]
pub struct PaymentOrchestrator {
    registry: ProviderRegistry,
    hooks: HookChain,
    route_resolver: Option<RouteResolver>,
}

impl fmt::Debug for PaymentOrchestrator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentOrchestrator")
            .field("registry", &self.registry)
            .field("hooks", &self.hooks)
            .field("has_route_resolver", &self.route_resolver.is_some())
            .finish()
    }
}

impl PaymentOrchestrator {
    /// Constructs a [`PaymentOrchestrator`] from its pre-validated components.
    pub(crate) fn from_parts(
        registry: ProviderRegistry,
        hooks: HookChain,
        route_resolver: Option<RouteResolver>,
    ) -> Self {
        Self {
            registry,
            hooks,
            route_resolver,
        }
    }

    /// Creates a new [`PaymentOrchestrator`] backed by `registry` and `hooks`
    /// with no custom route resolver (falling back to the registry default).
    #[must_use]
    pub fn new(registry: ProviderRegistry, hooks: HookChain) -> Self {
        Self::from_parts(registry, hooks, None)
    }

    /// Returns a reference to the immutable [`ProviderRegistry`].
    #[must_use]
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Returns a reference to the configured [`HookChain`].
    #[must_use]
    pub fn hooks(&self) -> &HookChain {
        &self.hooks
    }

    /// Creates a direct payment or embedded payment intent via the five-step pipeline.
    ///
    /// Requires [`PaymentCapabilities::DIRECT_PAYMENT`], plus
    /// [`PaymentCapabilities::AUTH_CAPTURE`] when `req.capture_mode` is
    /// [`CaptureMode::Manual`], and [`PaymentCapabilities::SPLITS`] when
    /// `req.splits` is non-empty.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] if request validation fails, the monetary
    /// amount is zero, provider resolution fails, the resolved provider lacks
    /// a required capability, a `before` hook aborts execution, or the provider
    /// returns an error.
    pub async fn create_payment(
        &self,
        req: &CreatePaymentRequest,
    ) -> Result<PaymentResult, PaymentError> {
        let (provider, ctx) = self.prepare_routed(req)?;
        self.execute_payment_op(ctx, || provider.create_payment(req))
            .await
    }

    /// Creates a provider-hosted checkout session via the five-step pipeline.
    ///
    /// Requires [`PaymentCapabilities::HOSTED_CHECKOUT`], plus
    /// [`PaymentCapabilities::AUTH_CAPTURE`] when `req.capture_mode` is
    /// [`CaptureMode::Manual`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] if request validation fails, the monetary
    /// amount is zero, provider resolution fails, the resolved provider lacks
    /// a required capability, a `before` hook aborts execution, or the provider
    /// returns an error.
    pub async fn create_checkout(
        &self,
        req: &CheckoutSessionRequest,
    ) -> Result<PaymentResult, PaymentError> {
        let (provider, ctx) = self.prepare_routed(req)?;
        self.execute_payment_op(ctx, || provider.create_checkout(req))
            .await
    }

    /// Fetches authoritative payment state for `reference` via the five-step pipeline.
    ///
    /// Resolves strictly via `registry.require(reference.provider_id())`.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] if the referenced provider is not registered,
    /// a `before` hook aborts execution, or the provider returns an error.
    pub async fn get_payment(
        &self,
        reference: &ProviderReference,
    ) -> Result<PaymentResult, PaymentError> {
        let provider = self.registry.require(reference.provider_id())?;
        let ctx = build_reference_context(
            PaymentOperation::GetPayment,
            provider.id().clone(),
            reference,
            None,
        );
        self.execute_payment_op(ctx, || provider.get_payment(reference))
            .await
    }

    /// Captures a previously authorized payment hold via the five-step pipeline.
    ///
    /// Requires [`PaymentCapabilities::AUTH_CAPTURE`], plus
    /// [`PaymentCapabilities::PARTIAL_CAPTURE`] when `req.amount` is `Some`.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] if validation fails, `req.amount` is zero,
    /// the referenced provider is not registered or lacks required capabilities,
    /// a `before` hook aborts execution, or the provider returns an error.
    pub async fn capture(&self, req: &CaptureRequest) -> Result<PaymentResult, PaymentError> {
        let (provider, ctx) = self.prepare_referenced(req)?;
        self.execute_payment_op(ctx, || provider.capture(req)).await
    }

    /// Voids an authorized hold or unsettled payment via the five-step pipeline.
    ///
    /// Requires [`PaymentCapabilities::VOID_AUTHORIZED`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] if validation fails, the referenced provider
    /// is not registered or lacks [`PaymentCapabilities::VOID_AUTHORIZED`],
    /// a `before` hook aborts execution, or the provider returns an error.
    pub async fn void_payment(&self, req: &VoidRequest) -> Result<PaymentResult, PaymentError> {
        let (provider, ctx) = self.prepare_referenced(req)?;
        self.execute_payment_op(ctx, || provider.void_payment(req))
            .await
    }

    /// Refunds a settled payment via the five-step pipeline.
    ///
    /// Requires [`PaymentCapabilities::REFUND`], plus
    /// [`PaymentCapabilities::PARTIAL_REFUND`] when `req.amount` is `Some`.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError`] if validation fails, `req.amount` is zero,
    /// the referenced provider is not registered or lacks required capabilities,
    /// a `before` hook aborts execution, or the provider returns an error.
    pub async fn refund(&self, req: &RefundRequest) -> Result<RefundResult, PaymentError> {
        let (provider, ctx) = self.prepare_referenced(req)?;
        let (ctx, result) = self.execute_op(ctx, || provider.refund(req)).await?;
        self.hooks.run_after_refund(&ctx, &result).await;
        Ok(result)
    }

    fn prepare_routed<R: RoutedRequest>(
        &self,
        req: &R,
    ) -> Result<(Arc<dyn PaymentProvider>, HookContext), PaymentError> {
        req.validate_request()?;
        let prep = req.to_routed_prep();
        assert_positive_amount(prep.money)?;
        let required = prep.resolution.required;
        let provider = self.resolve_for_request(prep.resolution)?;
        assert_capability(provider.as_ref(), required, prep.op.as_str())?;
        let ctx = HookContext::new(prep.op, provider.id().clone())
            .with_money(prep.money.clone())
            .with_order_id(prep.order_id.to_owned());
        Ok((provider, ctx))
    }

    fn prepare_referenced<R: ReferencedRequest>(
        &self,
        req: &R,
    ) -> Result<(Arc<dyn PaymentProvider>, HookContext), PaymentError> {
        req.validate_request()?;
        let prep = req.to_reference_prep();
        assert_optional_positive_amount(prep.amount)?;
        let provider = self.registry.require(prep.reference.provider_id())?;
        assert_capability(provider.as_ref(), prep.required, prep.op.as_str())?;
        let ctx = build_reference_context(
            prep.op,
            provider.id().clone(),
            prep.reference,
            prep.amount.cloned(),
        );
        Ok((provider, ctx))
    }

    fn resolve_for_request(
        &self,
        input: ResolutionInput<'_>,
    ) -> Result<Arc<dyn PaymentProvider>, PaymentError> {
        if let Some(id) = input.explicit {
            return self.registry.require(id);
        }
        if let Some(resolver) = &self.route_resolver {
            return resolver(
                &self.registry,
                None,
                input.currency,
                input.country,
                input.required,
            );
        }
        Ok(self.registry.default_provider())
    }

    async fn execute_op<T, F, Fut>(
        &self,
        mut ctx: HookContext,
        call: F,
    ) -> Result<(HookContext, T), PaymentError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, PaymentError>>,
    {
        self.hooks.run_before(&mut ctx).await?;
        match call().await {
            Ok(result) => Ok((ctx, result)),
            Err(err) => {
                self.hooks.run_on_error(&ctx, &err).await;
                Err(err)
            }
        }
    }

    async fn execute_payment_op<F, Fut>(
        &self,
        ctx: HookContext,
        call: F,
    ) -> Result<PaymentResult, PaymentError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<PaymentResult, PaymentError>>,
    {
        let (ctx, result) = self.execute_op(ctx, call).await?;
        self.hooks.run_after(&ctx, &result).await;
        Ok(result)
    }
}

struct ResolutionInput<'a> {
    explicit: Option<&'a ProviderId>,
    currency: Option<&'a CurrencyCode>,
    country: Option<&'a str>,
    required: PaymentCapabilities,
}

struct RoutedPrep<'a> {
    op: PaymentOperation,
    money: &'a Money,
    order_id: &'a str,
    resolution: ResolutionInput<'a>,
}

struct ReferencePrep<'a> {
    op: PaymentOperation,
    reference: &'a ProviderReference,
    amount: Option<&'a Money>,
    required: PaymentCapabilities,
}

trait RoutedRequest {
    fn validate_request(&self) -> Result<(), PaymentError>;
    fn to_routed_prep(&self) -> RoutedPrep<'_>;
}

trait ReferencedRequest {
    fn validate_request(&self) -> Result<(), PaymentError>;
    fn to_reference_prep(&self) -> ReferencePrep<'_>;
}

impl RoutedRequest for CreatePaymentRequest {
    fn validate_request(&self) -> Result<(), PaymentError> {
        self.validate()
    }

    fn to_routed_prep(&self) -> RoutedPrep<'_> {
        RoutedPrep {
            op: PaymentOperation::CreatePayment,
            money: &self.money,
            order_id: &self.order_id,
            resolution: ResolutionInput {
                explicit: self.provider_id.as_ref(),
                currency: Some(self.money.currency()),
                country: self.buyer_country.as_deref(),
                required: required_create_payment_caps(self),
            },
        }
    }
}

impl RoutedRequest for CheckoutSessionRequest {
    fn validate_request(&self) -> Result<(), PaymentError> {
        self.validate()
    }

    fn to_routed_prep(&self) -> RoutedPrep<'_> {
        RoutedPrep {
            op: PaymentOperation::CreateCheckout,
            money: &self.money,
            order_id: &self.order_id,
            resolution: ResolutionInput {
                explicit: self.provider_id.as_ref(),
                currency: Some(self.money.currency()),
                country: None,
                required: required_checkout_caps(self),
            },
        }
    }
}

impl ReferencedRequest for CaptureRequest {
    fn validate_request(&self) -> Result<(), PaymentError> {
        self.validate()
    }

    fn to_reference_prep(&self) -> ReferencePrep<'_> {
        ReferencePrep {
            op: PaymentOperation::Capture,
            reference: &self.reference,
            amount: self.amount.as_ref(),
            required: required_capture_caps(self),
        }
    }
}

impl ReferencedRequest for VoidRequest {
    fn validate_request(&self) -> Result<(), PaymentError> {
        self.validate()
    }

    fn to_reference_prep(&self) -> ReferencePrep<'_> {
        ReferencePrep {
            op: PaymentOperation::VoidPayment,
            reference: &self.reference,
            amount: None,
            required: PaymentCapabilities::VOID_AUTHORIZED,
        }
    }
}

impl ReferencedRequest for RefundRequest {
    fn validate_request(&self) -> Result<(), PaymentError> {
        self.validate()
    }

    fn to_reference_prep(&self) -> ReferencePrep<'_> {
        ReferencePrep {
            op: PaymentOperation::Refund,
            reference: &self.reference,
            amount: self.amount.as_ref(),
            required: required_refund_caps(self),
        }
    }
}

fn assert_positive_amount(money: &Money) -> Result<(), PaymentError> {
    if money.amount() == 0 {
        return Err(PaymentError::validation(
            "monetary amount must be strictly greater than zero",
        ));
    }
    Ok(())
}

fn assert_optional_positive_amount(money: Option<&Money>) -> Result<(), PaymentError> {
    match money {
        Some(value) => assert_positive_amount(value),
        None => Ok(()),
    }
}

fn assert_capability(
    provider: &dyn PaymentProvider,
    required: PaymentCapabilities,
    operation: &str,
) -> Result<(), PaymentError> {
    if !provider.capabilities().contains(required) {
        return Err(PaymentError::unsupported(provider.id(), operation));
    }
    Ok(())
}

fn required_create_payment_caps(req: &CreatePaymentRequest) -> PaymentCapabilities {
    let mut caps = PaymentCapabilities::DIRECT_PAYMENT;
    if req.capture_mode == CaptureMode::Manual {
        caps |= PaymentCapabilities::AUTH_CAPTURE;
    }
    if !req.splits.is_empty() {
        caps |= PaymentCapabilities::SPLITS;
    }
    caps
}

fn required_checkout_caps(req: &CheckoutSessionRequest) -> PaymentCapabilities {
    let mut caps = PaymentCapabilities::HOSTED_CHECKOUT;
    if req.capture_mode == CaptureMode::Manual {
        caps |= PaymentCapabilities::AUTH_CAPTURE;
    }
    caps
}

fn required_capture_caps(req: &CaptureRequest) -> PaymentCapabilities {
    let mut caps = PaymentCapabilities::AUTH_CAPTURE;
    if req.amount.is_some() {
        caps |= PaymentCapabilities::PARTIAL_CAPTURE;
    }
    caps
}

fn required_refund_caps(req: &RefundRequest) -> PaymentCapabilities {
    let mut caps = PaymentCapabilities::REFUND;
    if req.amount.is_some() {
        caps |= PaymentCapabilities::PARTIAL_REFUND;
    }
    caps
}

fn primary_reference_id(reference: &ProviderReference) -> Option<&str> {
    [
        reference.payment_id(),
        reference.transaction_id(),
        reference.intent_id(),
        reference.checkout_session_id(),
        reference.order_id(),
    ]
    .into_iter()
    .flatten()
    .next()
}

fn build_reference_context(
    operation: PaymentOperation,
    provider_id: ProviderId,
    reference: &ProviderReference,
    money: Option<Money>,
) -> HookContext {
    let mut ctx = HookContext::new(operation, provider_id);
    if let Some(amount) = money {
        ctx = ctx.with_money(amount);
    }
    if let Some(ref_id) = primary_reference_id(reference) {
        ctx = ctx.with_reference_id(ref_id.to_owned());
    }
    ctx
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use some_payment_core::{
        CaptureMode, CaptureRequest, CheckoutSessionRequest, CreatePaymentRequest, ErrorKind,
        Money, NormalizedWebhookEvent, PaymentCapabilities, PaymentError, PaymentProvider,
        PaymentResult, PaymentStatus, ProviderId, ProviderReference, RawWebhookRequest,
        RefundRequest, RefundResult, RefundStatus, SplitInstruction, VoidRequest,
    };

    use super::{PaymentHub, PaymentOrchestrator, RouteResolver};
    use crate::hooks::{HookChain, HookContext, HookDecision, LifecycleHook};
    use crate::registry::ProviderRegistry;

    #[derive(Debug)]
    struct MockProvider {
        id: ProviderId,
        capabilities: PaymentCapabilities,
        calls: AtomicUsize,
        fail_provider: bool,
    }

    impl MockProvider {
        fn new(slug: &str, capabilities: PaymentCapabilities) -> Self {
            Self {
                id: ProviderId::new(slug),
                capabilities,
                calls: AtomicUsize::new(0),
                fail_provider: false,
            }
        }

        fn with_failure(mut self, fail_provider: bool) -> Self {
            self.fail_provider = fail_provider;
            self
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn record_call(&self) -> Result<(), PaymentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_provider {
                return Err(self.provider_failure());
            }
            Ok(())
        }

        fn sample_payment_result(&self, status: PaymentStatus, money: Money) -> PaymentResult {
            let reference = ProviderReference::builder(self.id.clone())
                .payment_id("pay_mock_1")
                .build()
                .unwrap();
            PaymentResult {
                reference,
                status,
                money,
                captured_amount: None,
                refunded_amount: None,
                next_action: None,
                checkout_url: None,
                livemode: false,
            }
        }

        fn provider_failure(&self) -> PaymentError {
            PaymentError::ProviderUnavailable {
                provider: self.id.clone(),
                http_status: Some(503),
                indeterminate: false,
                message: "upstream timeout".to_owned(),
            }
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
            req: &CreatePaymentRequest,
        ) -> Result<PaymentResult, PaymentError> {
            self.record_call()?;
            Ok(self.sample_payment_result(PaymentStatus::Paid, req.money.clone()))
        }

        async fn create_checkout(
            &self,
            req: &CheckoutSessionRequest,
        ) -> Result<PaymentResult, PaymentError> {
            self.record_call()?;
            Ok(self.sample_payment_result(PaymentStatus::RequiresAction, req.money.clone()))
        }

        async fn get_payment(
            &self,
            reference: &ProviderReference,
        ) -> Result<PaymentResult, PaymentError> {
            self.record_call()?;
            let money = Money::new(5_000, "SAR").unwrap();
            let mut result = self.sample_payment_result(PaymentStatus::Paid, money);
            result.reference = reference.clone();
            Ok(result)
        }

        async fn capture(&self, req: &CaptureRequest) -> Result<PaymentResult, PaymentError> {
            self.record_call()?;
            let money = req
                .amount
                .clone()
                .unwrap_or_else(|| Money::new(5_000, "SAR").unwrap());
            Ok(self.sample_payment_result(PaymentStatus::Captured, money))
        }

        async fn void_payment(&self, _req: &VoidRequest) -> Result<PaymentResult, PaymentError> {
            self.record_call()?;
            let money = Money::new(5_000, "SAR").unwrap();
            Ok(self.sample_payment_result(PaymentStatus::Voided, money))
        }

        async fn refund(&self, req: &RefundRequest) -> Result<RefundResult, PaymentError> {
            self.record_call()?;
            let money = req
                .amount
                .clone()
                .unwrap_or_else(|| Money::new(5_000, "SAR").unwrap());
            Ok(RefundResult {
                refund_id: "re_mock_1".to_owned(),
                payment_reference: req.reference.clone(),
                status: RefundStatus::Succeeded,
                money,
                reason_detail: None,
            })
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

    #[derive(Debug, Default)]
    struct TestHook {
        before: AtomicUsize,
        after: AtomicUsize,
        after_refund: AtomicUsize,
        on_error: AtomicUsize,
        fail_after: bool,
    }

    impl TestHook {
        fn recording() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn failing_after() -> Arc<Self> {
            Arc::new(Self {
                fail_after: true,
                ..Self::default()
            })
        }

        fn finish_after(&self, counter: &AtomicUsize) -> Result<(), PaymentError> {
            counter.fetch_add(1, Ordering::SeqCst);
            if self.fail_after {
                return Err(PaymentError::configuration("ledger sink offline"));
            }
            Ok(())
        }

        fn counts(&self) -> (usize, usize, usize, usize) {
            (
                self.before.load(Ordering::SeqCst),
                self.after.load(Ordering::SeqCst),
                self.after_refund.load(Ordering::SeqCst),
                self.on_error.load(Ordering::SeqCst),
            )
        }
    }

    #[async_trait]
    impl LifecycleHook for TestHook {
        fn name(&self) -> &str {
            if self.fail_after {
                "failing-after-hook"
            } else {
                "recording-hook"
            }
        }

        async fn before(&self, _ctx: &mut HookContext) -> HookDecision {
            self.before.fetch_add(1, Ordering::SeqCst);
            HookDecision::Proceed
        }

        async fn after(
            &self,
            _ctx: &HookContext,
            _result: &PaymentResult,
        ) -> Result<(), PaymentError> {
            self.finish_after(&self.after)
        }

        async fn after_refund(
            &self,
            _ctx: &HookContext,
            _result: &RefundResult,
        ) -> Result<(), PaymentError> {
            self.finish_after(&self.after_refund)
        }

        async fn on_error(&self, _ctx: &HookContext, _error: &PaymentError) {
            self.on_error.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Debug)]
    struct AbortingHook;

    #[async_trait]
    impl LifecycleHook for AbortingHook {
        fn name(&self) -> &str {
            "aborting-hook"
        }

        async fn before(&self, _ctx: &mut HookContext) -> HookDecision {
            HookDecision::Abort("fraud_score_high".to_owned())
        }
    }

    fn single_provider_registry(provider: Arc<MockProvider>) -> ProviderRegistry {
        let id = provider.id().clone();
        let mut map: HashMap<ProviderId, Arc<dyn PaymentProvider>> = HashMap::new();
        map.insert(id.clone(), provider);
        ProviderRegistry::new(map, id).unwrap()
    }

    fn sample_money() -> Money {
        Money::new(10_000, "SAR").unwrap()
    }

    fn sample_create_req() -> CreatePaymentRequest {
        CreatePaymentRequest::new(sample_money(), "ORD-213-1")
    }

    fn sample_checkout_req() -> CheckoutSessionRequest {
        CheckoutSessionRequest {
            provider_id: None,
            money: sample_money(),
            order_id: "ORD-213-CHK".to_owned(),
            success_url: "https://merchant.example/success".to_owned(),
            cancel_url: Some("https://merchant.example/cancel".to_owned()),
            description: None,
            customer_email: None,
            customer_phone: None,
            capture_mode: CaptureMode::Automatic,
            idempotency_key: None,
            metadata: BTreeMap::new(),
        }
    }

    fn sample_ref(slug: &str) -> ProviderReference {
        ProviderReference::builder(ProviderId::new(slug))
            .payment_id("pay_ref_1")
            .build()
            .unwrap()
    }

    fn sample_capture_req(slug: &str, amount: Option<Money>) -> CaptureRequest {
        CaptureRequest {
            reference: sample_ref(slug),
            amount,
            idempotency_key: None,
        }
    }

    fn sample_void_req(slug: &str) -> VoidRequest {
        VoidRequest {
            reference: sample_ref(slug),
            reason: Some("customer_canceled".to_owned()),
            idempotency_key: None,
        }
    }

    fn sample_refund_req(slug: &str, amount: Option<Money>) -> RefundRequest {
        RefundRequest {
            reference: sample_ref(slug),
            amount,
            reason: Some("duplicate".to_owned()),
            idempotency_key: None,
        }
    }

    fn outcome<T>(res: Result<T, PaymentError>) -> Result<(), ErrorKind> {
        res.map(|_| ()).map_err(|e| e.kind())
    }

    async fn run_all_six_ops(
        orch: &PaymentOrchestrator,
        slug: &str,
        explicit_provider: Option<&str>,
    ) -> [Result<(), ErrorKind>; 6] {
        let mut create_req = sample_create_req();
        let mut checkout_req = sample_checkout_req();
        if let Some(id) = explicit_provider {
            create_req = create_req.with_provider_id(ProviderId::new(id));
            checkout_req.provider_id = Some(ProviderId::new(id));
        }
        [
            outcome(orch.create_payment(&create_req).await),
            outcome(orch.create_checkout(&checkout_req).await),
            outcome(orch.get_payment(&sample_ref(slug)).await),
            outcome(orch.capture(&sample_capture_req(slug, None)).await),
            outcome(orch.void_payment(&sample_void_req(slug)).await),
            outcome(orch.refund(&sample_refund_req(slug, None)).await),
        ]
    }

    #[tokio::test]
    async fn tc_2_1_3_happy_path_all_six_operations() {
        let provider = Arc::new(MockProvider::new("alpha", PaymentCapabilities::all()));
        let hook = TestHook::recording();
        let hooks = HookChain::new(vec![hook.clone()]);
        let orch = PaymentOrchestrator::new(single_provider_registry(provider.clone()), hooks);

        let created = orch.create_payment(&sample_create_req()).await.unwrap();
        let checkout = orch.create_checkout(&sample_checkout_req()).await.unwrap();
        let fetched = orch.get_payment(&sample_ref("alpha")).await.unwrap();
        let captured = orch
            .capture(&sample_capture_req("alpha", Some(sample_money())))
            .await
            .unwrap();
        let voided = orch.void_payment(&sample_void_req("alpha")).await.unwrap();
        let refunded = orch
            .refund(&sample_refund_req("alpha", Some(sample_money())))
            .await
            .unwrap();

        assert_eq!(
            (
                created.status,
                checkout.status,
                fetched.status,
                captured.status,
                voided.status,
                refunded.status,
            ),
            (
                PaymentStatus::Paid,
                PaymentStatus::RequiresAction,
                PaymentStatus::Paid,
                PaymentStatus::Captured,
                PaymentStatus::Voided,
                RefundStatus::Succeeded,
            )
        );
        assert_eq!((provider.call_count(), hook.counts()), (6, (6, 5, 1, 0)));
    }

    #[tokio::test]
    async fn ac_1_capability_guards_fail_closed_with_zero_provider_calls() {
        let provider = Arc::new(MockProvider::new("limited", PaymentCapabilities::empty()));
        let orch = PaymentOrchestrator::new(
            single_provider_registry(provider.clone()),
            HookChain::default(),
        );

        let results = [
            outcome(orch.create_payment(&sample_create_req()).await),
            outcome(orch.create_checkout(&sample_checkout_req()).await),
            outcome(orch.capture(&sample_capture_req("limited", None)).await),
            outcome(orch.void_payment(&sample_void_req("limited")).await),
            outcome(orch.refund(&sample_refund_req("limited", None)).await),
        ];
        assert_eq!(results, [Err(ErrorKind::Unsupported); 5]);
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn ac_1_extended_capabilities_for_manual_splits_and_partials() {
        let base_caps = PaymentCapabilities::DIRECT_PAYMENT
            | PaymentCapabilities::HOSTED_CHECKOUT
            | PaymentCapabilities::AUTH_CAPTURE
            | PaymentCapabilities::REFUND;
        let provider = Arc::new(MockProvider::new("base", base_caps));
        let orch = PaymentOrchestrator::new(
            single_provider_registry(provider.clone()),
            HookChain::default(),
        );

        let mut split_req = sample_create_req();
        split_req.splits.push(SplitInstruction {
            recipient_id: "acct_1".to_owned(),
            amount: 2_000,
            fee_source: false,
        });
        let cap_req = sample_capture_req("base", Some(sample_money()));
        let ref_req = sample_refund_req("base", Some(sample_money()));
        let base_results = [
            outcome(orch.create_payment(&split_req).await),
            outcome(orch.capture(&cap_req).await),
            outcome(orch.refund(&ref_req).await),
        ];
        assert_eq!(base_results, [Err(ErrorKind::Unsupported); 3]);

        let no_auth_caps =
            PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::HOSTED_CHECKOUT;
        let no_auth = Arc::new(MockProvider::new("no-auth", no_auth_caps));
        let no_auth_orch = PaymentOrchestrator::new(
            single_provider_registry(no_auth.clone()),
            HookChain::default(),
        );
        let manual_pay = sample_create_req().with_capture_mode(CaptureMode::Manual);
        let mut manual_checkout = sample_checkout_req();
        manual_checkout.capture_mode = CaptureMode::Manual;
        let manual_results = [
            outcome(no_auth_orch.create_payment(&manual_pay).await),
            outcome(no_auth_orch.create_checkout(&manual_checkout).await),
        ];
        assert_eq!(manual_results, [Err(ErrorKind::Unsupported); 2]);

        assert_eq!((provider.call_count(), no_auth.call_count()), (0, 0));
    }

    #[tokio::test]
    async fn ac_2_before_hook_abort_prevents_provider_calls_across_all_six_ops() {
        let provider = Arc::new(MockProvider::new("alpha", PaymentCapabilities::all()));
        let hooks = HookChain::new(vec![Arc::new(AbortingHook)]);
        let orch = PaymentOrchestrator::new(single_provider_registry(provider.clone()), hooks);

        let results = run_all_six_ops(&orch, "alpha", None).await;
        assert_eq!(results, [Err(ErrorKind::AbortedByHook); 6]);
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn ac_3_failing_after_hook_preserves_ok_results() {
        let provider = Arc::new(MockProvider::new("alpha", PaymentCapabilities::all()));
        let failing_hook = TestHook::failing_after();
        let hooks = HookChain::new(vec![failing_hook.clone()]);
        let orch = PaymentOrchestrator::new(single_provider_registry(provider.clone()), hooks);

        assert_eq!(run_all_six_ops(&orch, "alpha", None).await, [Ok(()); 6]);
        assert_eq!(
            (provider.call_count(), failing_hook.counts()),
            (6, (6, 5, 1, 0))
        );
    }

    #[tokio::test]
    async fn on_error_hook_fires_on_provider_failure_across_all_six_ops() {
        let mock = MockProvider::new("alpha", PaymentCapabilities::all()).with_failure(true);
        let provider = Arc::new(mock);
        let hook = TestHook::recording();
        let hooks = HookChain::new(vec![hook.clone()]);
        let orch = PaymentOrchestrator::new(single_provider_registry(provider.clone()), hooks);

        let results = run_all_six_ops(&orch, "alpha", None).await;
        assert_eq!(results, [Err(ErrorKind::ProviderUnavailable); 6]);
        assert_eq!((provider.call_count(), hook.counts()), (6, (6, 0, 0, 6)));
    }

    #[tokio::test]
    async fn ac_4_unknown_provider_id_returns_configuration_error() {
        let provider = Arc::new(MockProvider::new("alpha", PaymentCapabilities::all()));
        let orch = PaymentOrchestrator::new(
            single_provider_registry(provider.clone()),
            HookChain::default(),
        );

        let results = run_all_six_ops(&orch, "unknown", Some("unknown")).await;
        assert_eq!(results, [Err(ErrorKind::Configuration); 6]);
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn explicit_provider_id_takes_precedence_over_route_resolver() {
        let primary = Arc::new(MockProvider::new("primary", PaymentCapabilities::all()));
        let secondary = Arc::new(MockProvider::new("secondary", PaymentCapabilities::all()));
        let mut map: HashMap<ProviderId, Arc<dyn PaymentProvider>> = HashMap::new();
        map.insert(primary.id().clone(), primary.clone());
        map.insert(secondary.id().clone(), secondary.clone());
        let registry = ProviderRegistry::new(map, primary.id().clone()).unwrap();

        let resolver: RouteResolver = Arc::new(|reg, _explicit, _currency, _country, _caps| {
            reg.require(&ProviderId::new("secondary"))
        });
        let orch = PaymentOrchestrator::from_parts(registry, HookChain::default(), Some(resolver));

        let routed = orch.create_payment(&sample_create_req()).await.unwrap();
        assert_eq!(
            (
                routed.reference.provider_id().as_str(),
                secondary.call_count(),
                primary.call_count(),
            ),
            ("secondary", 1, 0)
        );

        let explicit_req = sample_create_req().with_provider_id(ProviderId::new("primary"));
        let overridden = orch.create_payment(&explicit_req).await.unwrap();
        assert_eq!(
            (
                overridden.reference.provider_id().as_str(),
                primary.call_count(),
                secondary.call_count(),
            ),
            ("primary", 1, 1)
        );
    }

    #[tokio::test]
    async fn validation_error_short_circuits_before_provider_and_hooks() {
        let provider = Arc::new(MockProvider::new("alpha", PaymentCapabilities::all()));
        let hook = TestHook::recording();
        let hooks = HookChain::new(vec![hook.clone()]);
        let orch: PaymentHub =
            PaymentOrchestrator::new(single_provider_registry(provider.clone()), hooks);

        let bad_create = CreatePaymentRequest::new(sample_money(), "   ");
        let mut bad_capture = sample_capture_req("alpha", None);
        bad_capture.idempotency_key = Some("   ".to_owned());

        let results = [
            outcome(orch.create_payment(&bad_create).await),
            outcome(orch.capture(&bad_capture).await),
        ];
        assert_eq!(results, [Err(ErrorKind::Validation); 2]);
        assert_eq!(
            (
                provider.call_count(),
                hook.before.load(Ordering::SeqCst),
                format!("{orch:?}").contains("PaymentOrchestrator"),
                orch.registry().default_provider().id().as_str(),
                format!("{:?}", orch.hooks()).contains("HookChain"),
            ),
            (0, 0, true, "alpha", true)
        );
    }
}
