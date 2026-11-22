//! Composable lifecycle hooks and non-reversible `after`-hook isolation (US-2.1.0 / Unit 2).
//!
//! Lifecycle hooks let callers attach cross-cutting concerns (fraud checks, audit logging,
//! metrics, reconciliation alerts) around provider operations without coupling to any
//! concrete payment gateway crate.
//!
//! # Security & Isolation Invariants
//!
//! - **Immutable Monetary Amount (TM-2)**: [`HookContext::money`] exposes `&Money` only, so
//!   `before` hooks can inspect transaction amounts but can never mutate them.
//! - **Redacted Debug Output (TM-4)**: [`HookContext`] implements a custom [`fmt::Debug`]
//!   that redacts all attribute values as `"[REDACTED]"` so secrets and PII never leak into
//!   logs or diagnostics.
//! - **Non-Reversible `after` Isolation (AC-3, TM-3, TM-5)**: Once a provider succeeds,
//!   [`HookChain::run_after`] and [`HookChain::run_after_refund`] log a structured
//!   reconciliation alert via [`tracing::error!`] on any hook failure and never return `Err`.
//!
//! # Examples
//!
//! ```rust
//! use some_payment::hooks::{HookChain, HookContext, PaymentOperation};
//! use some_payment_core::{Money, ProviderId};
//!
//! let money = Money::new(5000, "USD").unwrap();
//! let mut ctx = HookContext::new(PaymentOperation::CreatePayment, ProviderId::new("stripe"))
//!     .with_money(money)
//!     .with_order_id("ord_123");
//! ctx.insert_attribute("customer_ip", "203.0.113.42");
//!
//! assert_eq!(ctx.operation().as_str(), "create_payment");
//! assert_eq!(ctx.money().unwrap().amount(), 5000);
//! assert!(!format!("{ctx:?}").contains("203.0.113.42"));
//! assert!(HookChain::default().is_empty());
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use some_payment_core::{Money, PaymentError, PaymentResult, ProviderId, RefundResult};

/// Canonical payment lifecycle operation executed through the orchestrator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaymentOperation {
    /// Direct payment creation.
    CreatePayment,
    /// Hosted checkout session creation.
    CreateCheckout,
    /// Fetching an existing payment's current state.
    GetPayment,
    /// Capturing an authorized payment.
    Capture,
    /// Voiding an authorized payment before settlement.
    VoidPayment,
    /// Issuing a full or partial refund.
    Refund,
}

const OPERATION_NAMES: &[(PaymentOperation, &str)] = &[
    (PaymentOperation::CreatePayment, "create_payment"),
    (PaymentOperation::CreateCheckout, "create_checkout"),
    (PaymentOperation::GetPayment, "get_payment"),
    (PaymentOperation::Capture, "capture"),
    (PaymentOperation::VoidPayment, "void_payment"),
    (PaymentOperation::Refund, "refund"),
];

impl PaymentOperation {
    /// Returns the canonical snake_case identifier for this operation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        OPERATION_NAMES
            .iter()
            .find(|(op, _)| *op == self)
            .map_or("unknown", |(_, name)| *name)
    }
}

/// Control-flow decision returned by [`LifecycleHook::before`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookDecision {
    /// Allow execution to continue to the next hook and then the provider.
    Proceed,
    /// Immediately abort the operation before any provider call is made.
    Abort(String),
}

/// Contextual metadata passed to [`LifecycleHook`] callbacks.
///
/// Monetary amounts and identifiers are read-only once constructed (TM-2), while hooks may
/// attach non-authoritative enrichment attributes via [`HookContext::insert_attribute`].
/// Its [`fmt::Debug`] implementation always redacts attribute values as `"[REDACTED]"`
/// to prevent secrets or PII from leaking into logs (TM-4).
#[derive(Clone)]
pub struct HookContext {
    operation: PaymentOperation,
    provider_id: ProviderId,
    money: Option<Money>,
    order_id: Option<String>,
    reference_id: Option<String>,
    attributes: BTreeMap<String, String>,
}

impl HookContext {
    /// Creates a new [`HookContext`] for the given operation and target provider.
    #[must_use]
    pub fn new(operation: PaymentOperation, provider_id: ProviderId) -> Self {
        Self {
            operation,
            provider_id,
            money: None,
            order_id: None,
            reference_id: None,
            attributes: BTreeMap::new(),
        }
    }

    /// Attaches an immutable transaction [`Money`] amount to the context.
    #[must_use]
    pub fn with_money(mut self, money: Money) -> Self {
        self.money = Some(money);
        self
    }

    /// Attaches a merchant order identifier to the context.
    #[must_use]
    pub fn with_order_id(mut self, order_id: impl Into<String>) -> Self {
        self.order_id = Some(order_id.into());
        self
    }

    /// Attaches a provider or correlation reference identifier to the context.
    #[must_use]
    pub fn with_reference_id(mut self, reference_id: impl Into<String>) -> Self {
        self.reference_id = Some(reference_id.into());
        self
    }

    /// Returns the lifecycle operation being executed.
    #[must_use]
    pub fn operation(&self) -> PaymentOperation {
        self.operation
    }

    /// Returns the target provider identifier.
    #[must_use]
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    /// Returns a read-only reference to the transaction [`Money`], if present.
    ///
    /// Returning `&Money` prevents `before` hooks from mutating the transaction amount (TM-2).
    #[must_use]
    pub fn money(&self) -> Option<&Money> {
        self.money.as_ref()
    }

    /// Returns the merchant order identifier, if present.
    #[must_use]
    pub fn order_id(&self) -> Option<&str> {
        self.order_id.as_deref()
    }

    /// Returns the correlation reference identifier, if present.
    #[must_use]
    pub fn reference_id(&self) -> Option<&str> {
        self.reference_id.as_deref()
    }

    /// Returns the read-only map of enrichment attributes attached by hooks.
    #[must_use]
    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    /// Inserts or updates an enrichment attribute on the context.
    pub fn insert_attribute(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.attributes.insert(key.into(), value.into());
    }
}

impl fmt::Debug for HookContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_attributes: BTreeMap<&str, &str> = self
            .attributes
            .keys()
            .map(|k| (k.as_str(), "[REDACTED]"))
            .collect();
        f.debug_struct("HookContext")
            .field("operation", &self.operation)
            .field("provider_id", &self.provider_id)
            .field("money", &self.money)
            .field("order_id", &self.order_id)
            .field("reference_id", &self.reference_id)
            .field("attributes", &redacted_attributes)
            .finish()
    }
}

/// Composable asynchronous lifecycle hook invoked around provider operations.
///
/// All callback methods provide safe no-op defaults so implementations only need to
/// override the lifecycle phases they care about.
#[async_trait]
pub trait LifecycleHook: Send + Sync + fmt::Debug {
    /// Returns a stable, human-readable name identifying this hook in structured logs.
    fn name(&self) -> &str;

    /// Runs before the provider is invoked.
    ///
    /// Returning [`HookDecision::Abort`] immediately short-circuits subsequent hooks and
    /// prevents the provider call from executing.
    async fn before(&self, _ctx: &mut HookContext) -> HookDecision {
        HookDecision::Proceed
    }

    /// Runs after a payment operation succeeds at the provider.
    ///
    /// Returning `Err` triggers a structured reconciliation alert via [`tracing::error!`]
    /// without reversing or masking the provider's [`PaymentResult`].
    async fn after(&self, _ctx: &HookContext, _result: &PaymentResult) -> Result<(), PaymentError> {
        Ok(())
    }

    /// Runs after a refund operation succeeds at the provider.
    ///
    /// Returning `Err` triggers a structured reconciliation alert via [`tracing::error!`]
    /// without reversing or masking the provider's [`RefundResult`].
    async fn after_refund(
        &self,
        _ctx: &HookContext,
        _result: &RefundResult,
    ) -> Result<(), PaymentError> {
        Ok(())
    }

    /// Runs when a provider operation fails with a [`PaymentError`].
    async fn on_error(&self, _ctx: &HookContext, _error: &PaymentError) {}
}

/// Ordered, cheaply cloneable chain of [`LifecycleHook`] implementations.
///
/// # Examples
///
/// ```rust
/// use std::sync::Arc;
///
/// use async_trait::async_trait;
/// use some_payment::hooks::{
///     HookChain, HookContext, HookDecision, LifecycleHook, PaymentOperation,
/// };
/// use some_payment_core::ProviderId;
///
/// #[derive(Debug)]
/// struct TagHook;
///
/// #[async_trait]
/// impl LifecycleHook for TagHook {
///     fn name(&self) -> &str {
///         "tag_hook"
///     }
///
///     async fn before(&self, ctx: &mut HookContext) -> HookDecision {
///         ctx.insert_attribute("tagged", "true");
///         HookDecision::Proceed
///     }
/// }
///
/// #[tokio::main(flavor = "current_thread")]
/// async fn main() {
///     let chain = HookChain::new(vec![Arc::new(TagHook)]);
///     let mut ctx = HookContext::new(PaymentOperation::GetPayment, ProviderId::new("stripe"));
///     chain.run_before(&mut ctx).await.unwrap();
///     assert_eq!(ctx.attributes().get("tagged").map(String::as_str), Some("true"));
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct HookChain {
    hooks: Arc<Vec<Arc<dyn LifecycleHook>>>,
}

impl HookChain {
    /// Creates a new [`HookChain`] executing hooks in the provided order.
    #[must_use]
    pub fn new(hooks: Vec<Arc<dyn LifecycleHook>>) -> Self {
        Self {
            hooks: Arc::new(hooks),
        }
    }

    /// Returns the number of registered hooks in this chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Returns `true` if no hooks are registered in this chain.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Executes every hook's [`LifecycleHook::before`] callback sequentially.
    ///
    /// On the first [`HookDecision::Abort`], execution immediately short-circuits and
    /// returns `Err(PaymentError::AbortedByHook { reason })` without invoking remaining
    /// hooks (AC-2).
    pub async fn run_before(&self, ctx: &mut HookContext) -> Result<(), PaymentError> {
        for hook in self.hooks.iter() {
            if let HookDecision::Abort(reason) = hook.before(ctx).await {
                return Err(PaymentError::AbortedByHook { reason });
            }
        }
        Ok(())
    }

    /// Executes every hook's [`LifecycleHook::after`] callback sequentially.
    ///
    /// Failures are isolated: any returned `Err` is logged as a structured reconciliation
    /// alert via [`tracing::error!`] and execution continues to the remaining hooks without
    /// propagating an error (AC-3, TM-3, TM-5).
    pub async fn run_after(&self, ctx: &HookContext, result: &PaymentResult) {
        for hook in self.hooks.iter() {
            if let Err(err) = hook.after(ctx, result).await {
                tracing::error!(
                    hook = hook.name(),
                    provider = %ctx.provider_id(),
                    operation = ctx.operation().as_str(),
                    error = %err,
                    "reconciliation alert: after hook failed after provider succeeded"
                );
            }
        }
    }

    /// Executes every hook's [`LifecycleHook::after_refund`] callback sequentially.
    ///
    /// Failures are isolated: any returned `Err` is logged as a structured reconciliation
    /// alert via [`tracing::error!`] and execution continues to the remaining hooks without
    /// propagating an error (AC-3, TM-3, TM-5).
    pub async fn run_after_refund(&self, ctx: &HookContext, result: &RefundResult) {
        for hook in self.hooks.iter() {
            if let Err(err) = hook.after_refund(ctx, result).await {
                tracing::error!(
                    hook = hook.name(),
                    provider = %ctx.provider_id(),
                    operation = ctx.operation().as_str(),
                    error = %err,
                    "reconciliation alert: after_refund hook failed after provider succeeded"
                );
            }
        }
    }

    /// Executes every hook's [`LifecycleHook::on_error`] callback sequentially.
    pub async fn run_on_error(&self, ctx: &HookContext, error: &PaymentError) {
        for hook in self.hooks.iter() {
            hook.on_error(ctx, error).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use some_payment_core::{
        ErrorKind, Money, PaymentError, PaymentResult, PaymentStatus, ProviderId,
        ProviderReference, RefundResult, RefundStatus,
    };

    use super::{HookChain, HookContext, HookDecision, LifecycleHook, PaymentOperation};

    #[derive(Debug)]
    struct RecordingHook {
        hook_name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
        abort_reason: Option<&'static str>,
        fail_after: bool,
        fail_after_refund: bool,
    }

    impl RecordingHook {
        fn new(hook_name: &'static str, events: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                hook_name,
                events,
                abort_reason: None,
                fail_after: false,
                fail_after_refund: false,
            }
        }

        fn aborting(mut self, reason: &'static str) -> Self {
            self.abort_reason = Some(reason);
            self
        }

        fn failing_after(mut self) -> Self {
            self.fail_after = true;
            self
        }

        fn failing_after_refund(mut self) -> Self {
            self.fail_after_refund = true;
            self
        }

        fn record(&self, event: String) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[async_trait]
    impl LifecycleHook for RecordingHook {
        fn name(&self) -> &str {
            self.hook_name
        }

        async fn before(&self, ctx: &mut HookContext) -> HookDecision {
            self.record(format!("before:{}", self.hook_name));
            ctx.insert_attribute(format!("seen_{}", self.hook_name), "yes");
            match self.abort_reason {
                Some(reason) => HookDecision::Abort(reason.to_owned()),
                None => HookDecision::Proceed,
            }
        }

        async fn after(
            &self,
            _ctx: &HookContext,
            _result: &PaymentResult,
        ) -> Result<(), PaymentError> {
            self.record(format!("after:{}", self.hook_name));
            if self.fail_after {
                return Err(PaymentError::configuration("downstream ledger failed"));
            }
            Ok(())
        }

        async fn after_refund(
            &self,
            _ctx: &HookContext,
            _result: &RefundResult,
        ) -> Result<(), PaymentError> {
            self.record(format!("after_refund:{}", self.hook_name));
            if self.fail_after_refund {
                return Err(PaymentError::configuration("refund ledger failed"));
            }
            Ok(())
        }

        async fn on_error(&self, _ctx: &HookContext, error: &PaymentError) {
            self.record(format!("on_error:{}:{:?}", self.hook_name, error.kind()));
        }
    }

    fn sample_payment_result() -> PaymentResult {
        let provider_id = ProviderId::new("stripe");
        let reference = ProviderReference::builder(provider_id)
            .payment_id("pi_12345")
            .build()
            .unwrap();
        PaymentResult {
            reference,
            status: PaymentStatus::Paid,
            money: Money::new(2500, "USD").unwrap(),
            captured_amount: Some(2500),
            refunded_amount: None,
            next_action: None,
            checkout_url: None,
            livemode: false,
        }
    }

    fn sample_refund_result() -> RefundResult {
        let provider_id = ProviderId::new("stripe");
        let payment_reference = ProviderReference::builder(provider_id)
            .payment_id("pi_12345")
            .build()
            .unwrap();
        RefundResult {
            refund_id: "re_98765".to_owned(),
            payment_reference,
            status: RefundStatus::Succeeded,
            money: Money::new(1000, "USD").unwrap(),
            reason_detail: None,
        }
    }

    #[test]
    fn operation_as_str_covers_all_variants() {
        assert_eq!(PaymentOperation::CreatePayment.as_str(), "create_payment");
        assert_eq!(PaymentOperation::CreateCheckout.as_str(), "create_checkout");
        assert_eq!(PaymentOperation::GetPayment.as_str(), "get_payment");
        assert_eq!(PaymentOperation::Capture.as_str(), "capture");
        assert_eq!(PaymentOperation::VoidPayment.as_str(), "void_payment");
        assert_eq!(PaymentOperation::Refund.as_str(), "refund");
    }

    #[test]
    fn hook_context_builders_getters_and_redacted_debug() {
        let money = Money::new(9900, "USD").unwrap();
        let mut ctx = HookContext::new(PaymentOperation::CreatePayment, ProviderId::new("stripe"))
            .with_money(money)
            .with_order_id("ord_42")
            .with_reference_id("ref_99");
        ctx.insert_attribute("api_token", "sk_live_super_secret_12345");
        ctx.insert_attribute("customer_email", "alice@example.com");

        assert_eq!(ctx.operation(), PaymentOperation::CreatePayment);
        assert_eq!(ctx.provider_id().as_str(), "stripe");
        assert_eq!(ctx.money().map(Money::amount), Some(9900));
        assert_eq!(ctx.order_id(), Some("ord_42"));
        assert_eq!(ctx.reference_id(), Some("ref_99"));
        assert_eq!(
            ctx.attributes().get("api_token").map(String::as_str),
            Some("sk_live_super_secret_12345")
        );

        let debug_output = format!("{ctx:?}");
        assert!(debug_output.contains("api_token"));
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("sk_live_super_secret_12345"));
        assert!(!debug_output.contains("alice@example.com"));
    }

    #[test]
    fn hook_chain_default_len_and_is_empty() {
        let empty = HookChain::default();
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());

        let events = Arc::new(Mutex::new(Vec::new()));
        let chain = HookChain::new(vec![Arc::new(RecordingHook::new("h1", events))]);
        assert_eq!(chain.len(), 1);
        assert!(!chain.is_empty());
    }

    #[tokio::test]
    async fn before_hooks_execute_sequentially_and_enrich_context() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let chain = HookChain::new(vec![
            Arc::new(RecordingHook::new("first", Arc::clone(&events))),
            Arc::new(RecordingHook::new("second", Arc::clone(&events))),
        ]);
        let mut ctx = HookContext::new(PaymentOperation::CreatePayment, ProviderId::new("stripe"))
            .with_money(Money::new(1500, "USD").unwrap());

        chain.run_before(&mut ctx).await.unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            vec!["before:first".to_owned(), "before:second".to_owned()]
        );
        assert_eq!(
            ctx.attributes().get("seen_first").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            ctx.attributes().get("seen_second").map(String::as_str),
            Some("yes")
        );
        assert_eq!(ctx.money().map(Money::amount), Some(1500));
    }

    #[tokio::test]
    async fn before_abort_short_circuits_subsequent_hooks() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let chain = HookChain::new(vec![
            Arc::new(RecordingHook::new("allow", Arc::clone(&events))),
            Arc::new(RecordingHook::new("fraud", Arc::clone(&events)).aborting("high_risk_score")),
            Arc::new(RecordingHook::new("never_reached", Arc::clone(&events))),
        ]);
        let mut ctx = HookContext::new(PaymentOperation::CreateCheckout, ProviderId::new("paymob"));

        let err = chain.run_before(&mut ctx).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AbortedByHook);
        match err {
            PaymentError::AbortedByHook { reason } => assert_eq!(reason, "high_risk_score"),
            other => panic!("expected AbortedByHook, got {other:?}"),
        }
        assert_eq!(
            *events.lock().unwrap(),
            vec!["before:allow".to_owned(), "before:fraud".to_owned()]
        );
    }

    #[tokio::test]
    async fn after_hook_failure_is_isolated_and_continues_chain() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let chain = HookChain::new(vec![
            Arc::new(RecordingHook::new("broken_ledger", Arc::clone(&events)).failing_after()),
            Arc::new(RecordingHook::new("metrics", Arc::clone(&events))),
        ]);
        let ctx = HookContext::new(PaymentOperation::Capture, ProviderId::new("stripe"));
        let result = sample_payment_result();

        chain.run_after(&ctx, &result).await;

        assert_eq!(
            *events.lock().unwrap(),
            vec!["after:broken_ledger".to_owned(), "after:metrics".to_owned()]
        );
    }

    #[tokio::test]
    async fn after_refund_hook_failure_is_isolated_and_continues_chain() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let chain = HookChain::new(vec![
            Arc::new(
                RecordingHook::new("broken_refund_audit", Arc::clone(&events))
                    .failing_after_refund(),
            ),
            Arc::new(RecordingHook::new("notify", Arc::clone(&events))),
        ]);
        let ctx = HookContext::new(PaymentOperation::Refund, ProviderId::new("stripe"));
        let result = sample_refund_result();

        chain.run_after_refund(&ctx, &result).await;

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "after_refund:broken_refund_audit".to_owned(),
                "after_refund:notify".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn on_error_invokes_all_hooks_in_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let chain = HookChain::new(vec![
            Arc::new(RecordingHook::new("alerting", Arc::clone(&events))),
            Arc::new(RecordingHook::new("metrics", Arc::clone(&events))),
        ]);
        let ctx = HookContext::new(PaymentOperation::VoidPayment, ProviderId::new("moyasar"));
        let error = PaymentError::configuration("missing api key");

        chain.run_on_error(&ctx, &error).await;

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "on_error:alerting:Configuration".to_owned(),
                "on_error:metrics:Configuration".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn default_lifecycle_hook_methods_are_noops() {
        #[derive(Debug)]
        struct NoopHook;

        #[async_trait]
        impl LifecycleHook for NoopHook {
            fn name(&self) -> &str {
                "noop"
            }
        }

        let hook = NoopHook;
        assert_eq!(hook.name(), "noop");

        let mut ctx = HookContext::new(PaymentOperation::GetPayment, ProviderId::new("stripe"));
        assert_eq!(hook.before(&mut ctx).await, HookDecision::Proceed);

        let payment_result = sample_payment_result();
        assert!(hook.after(&ctx, &payment_result).await.is_ok());

        let refund_result = sample_refund_result();
        assert!(hook.after_refund(&ctx, &refund_result).await.is_ok());

        let error = PaymentError::configuration("noop error");
        hook.on_error(&ctx, &error).await;
    }
}
