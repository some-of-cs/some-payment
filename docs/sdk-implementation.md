# Full Implementation Documentation for `some-payment` and Multi-Provider Package Alignment

Implementing `some-payment` as a unified, extensible payment orchestration layer over `some-stripe`, `some-moyasar`, and `some-paymob` requires solving two engineering challenges up front: **inverting the package dependency graph** so future providers can be plugged in without circular dependencies or edits to core code, and **normalizing deep protocol and semantic divergences** across Stripe (`2026-11-18.endive`), Moyasar (November 2026 APIs), and Paymob (2026 Unified Intention and Accept APIs). Because `https://github.com/some-of-cs/some-payment` and its sibling provider repositories were not publicly accessible during documentation preparation, this guide cannot provide line-by-line Git patches against existing files; instead, it delivers the complete system architecture, canonical contract definitions, a prioritized pre-integration audit of changes required across `some-stripe`, `some-moyasar`, and `some-paymob`, and a step-by-step implementation and provider-extension manual.

Because engineering work under Some(CS) spans production **Rust** backends (Actix-web, Tonic gRPC, microkernel architectures) and **TypeScript** full-stack ecosystems (Next.js, MERN, Expo), and kebab-case package names (`some-*`) apply natively to both Cargo crates and npm packages, every domain contract, error rule, and package change in this specification is runtime-agnostic. Concrete packaging and trait/interface blueprints are provided for both Rust (Cargo workspace with explicit `dep:` feature flags) and TypeScript (ESM-only modules with optional `peerDependenciesMeta`).

All provider rules are anchored in official 2026 vendor documentation—covering Stripe's PaymentIntents, Checkout Sessions, Refunds, Currencies, Idempotency (v1 and v2), and Webhooks; Moyasar's Payments, Payment Intents, Invoices, and Webhooks APIs; and Paymob's Unified Intention, Void/Refund, Transaction Inquiry, and triple HMAC-SHA512 callback specifications—supplemented with clearly attributed operational findings from open-source multi-gateway SDKs and OpenAPI specs where vendor prose leaves gaps.

## System Architecture and Dependency Topology

A common mistake when building a multi-gateway orchestrator is having `some-payment` import `some-stripe`, `some-moyasar`, and `some-paymob` while expecting those provider packages to implement interfaces declared inside `some-payment`. That topology creates a **cyclic dependency** (`some-payment -> some-stripe -> some-payment`), which Cargo rejects at compile time and which breaks clean type-checking, versioning, and tree-shaking in TypeScript. Production payment orchestrators such as Juspay Hyperswitch (`hyperswitch_domain_models` + `hyperswitch_connectors` + `router`) and modular TypeScript payment cores such as `@paykernel/core` and `@ng-pay/core` break this cycle via **Dependency Inversion**: a minimal, HTTP-free contracts package (`some-payment-core`) sits at the base of the dependency graph, while all providers and the orchestrator sit above it.

Two architectural variants can connect `some-stripe`, `some-moyasar`, and `some-paymob` into this layered structure. The trade-off centers on where each provider's translation adapter lives and how tightly co-versioned the repositories are.

| Dimension | Option A: Shared `some-payment-core` Contract Package (Recommended) | Option B: Pure Low-Level SDKs + Internal Adapters in `some-payment` |
| :--- | :--- | :--- |
| **Dependency Direction** | `some-stripe`, `some-moyasar`, and `some-paymob` optionally depend on `some-payment-core`; `some-payment` depends on `some-payment-core` plus optional providers. | `some-stripe`, `some-moyasar`, and `some-paymob` have zero knowledge of `some-payment`; `some-payment` maintains internal `adapters/{stripe,moyasar,paymob}` modules. |
| **Standalone SDK Usability** | High—each `some-*` package remains standalone whenever its optional `some-payment-adapter` feature or subpath is left disabled. | Highest—`some-stripe`, `some-moyasar`, and `some-paymob` carry zero contract dependencies. |
| **Third-Party Extensibility** | First-class—future providers (`some-tap`, `some-tamara`, or external community packages) depend only on `some-payment-core` and register directly into `some-payment`. | Still requires `some-payment` to export a clean contract package (`some-payment-core`) so external plugins do not transitively pull built-in adapters. |
| **Adapter Maintenance** | Adapters live alongside vendor API models inside each `some-*` repo, preventing schema drift when a vendor updates payloads. | Breaking vendor API changes require synchronized two-repository releases (`some-paymob` followed by `some-payment`). |
| **Build & Bundle Footprint** | Minimal—consumers pay compile time and bundle weight only for the providers they activate. | Minimal—identical optional dependency gating inside `some-payment`. |

Option A is recommended when all four repositories are co-maintained under `github.com/some-of-cs`, because updates to Paymob's Intention API or Moyasar's webhook objects update the adapter in the same commit as the underlying SDK. Even if you prefer Option B to keep the three sibling packages completely unaware of `some-payment`, every functional change detailed in the Package Alignment Audit remains necessary so `some-payment`'s adapters never have to bypass your own SDKs with raw HTTP calls.

### Dependency Packaging Configuration

To ensure an application deploying only `some-stripe` and `some-paymob` never compiles or bundles `some-moyasar`, `some-payment` should declare built-in providers as **optional dependencies** behind explicit feature flags or subpath exports.

In **Rust**, declare optional dependencies with Cargo's explicit `dep:` syntax (supported since Rust 1.60 and recommended to prevent leaking implicit dependency-named features) paired with weak feature forwarding (`?/`). In CI, test each feature combination in isolation (`cargo check --no-default-features --features stripe`) rather than relying solely on `--workspace --all-features`, because Cargo feature unification across a workspace can hide missing `#[cfg(feature = "...")]` guards from minimal consumers.

```toml
# some-payment/Cargo.toml
[package]
name = "some-payment"
version = "0.1.0"
edition = "2024"

[features]
default = []
# Explicit dep: disables implicit features; ?/ forwards cleanly across Cargo 1.60+
stripe = ["dep:some-stripe", "some-stripe?/some-payment-adapter"]
moyasar = ["dep:some-moyasar", "some-moyasar?/some-payment-adapter"]
paymob = ["dep:some-paymob", "some-paymob?/some-payment-adapter"]
all-providers = ["stripe", "moyasar", "paymob"]

[dependencies]
some-payment-core = { version = "0.1", path = "../some-payment-core" }
some-stripe = { version = "0.1", optional = true }
some-moyasar = { version = "0.1", optional = true }
some-paymob = { version = "0.1", optional = true }
async-trait = "0.1"
```

In **TypeScript** (Node >= 18 or Bun >= 1.0), publish **ESM-only** (`"type": "module"`), declare sibling SDKs under `peerDependencies` marked optional via `peerDependenciesMeta` (supported since npm v7), and expose dedicated subpath exports so bundlers tree-shake unused providers cleanly:

```json
{
  "name": "@some-of-cs/some-payment",
  "version": "0.1.0",
  "type": "module",
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "import": "./dist/index.js"
    },
    "./stripe": {
      "types": "./dist/providers/stripe.d.ts",
      "import": "./dist/providers/stripe.js"
    },
    "./moyasar": {
      "types": "./dist/providers/moyasar.d.ts",
      "import": "./dist/providers/moyasar.js"
    },
    "./paymob": {
      "types": "./dist/providers/paymob.d.ts",
      "import": "./dist/providers/paymob.js"
    }
  },
  "dependencies": {
    "@some-of-cs/some-payment-core": "^0.1.0"
  },
  "peerDependencies": {
    "@some-of-cs/some-stripe": "^0.1.0",
    "@some-of-cs/some-moyasar": "^0.1.0",
    "@some-of-cs/some-paymob": "^0.1.0"
  },
  "peerDependenciesMeta": {
    "@some-of-cs/some-stripe": { "optional": true },
    "@some-of-cs/some-moyasar": { "optional": true },
    "@some-of-cs/some-paymob": { "optional": true }
  }
}
```

### Rust Blueprint: Provider Trait and Core Contract

```rust
// some-payment-core/src/provider.rs
use async_trait::async_trait;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into().trim().to_ascii_lowercase())
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[async_trait]
pub trait PaymentProvider: Send + Sync + fmt::Debug {
    /// Unique lowercase provider slug (e.g., "stripe", "moyasar", "paymob", "tap").
    fn id(&self) -> &ProviderId;

    /// Capability bitmask derived from provider features and runtime config.
    fn capabilities(&self) -> PaymentCapabilities;

    /// Create a direct payment or an embedded payment intent.
    async fn create_payment(&self, req: &CreatePaymentRequest) -> Result<PaymentResult, PaymentError>;

    /// Fetch authoritative payment state directly from the provider API.
    async fn get_payment(&self, reference: &ProviderReference) -> Result<PaymentResult, PaymentError>;

    /// Create a provider-hosted checkout page, session, or invoice.
    async fn create_checkout(&self, _req: &CheckoutSessionRequest) -> Result<PaymentResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "create_checkout"))
    }

    /// Capture an authorized hold (full when amount is None; partial when Some).
    async fn capture(&self, _req: &CaptureRequest) -> Result<PaymentResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "capture"))
    }

    /// Release an authorization hold or void an unsettled payment.
    async fn void_payment(&self, _req: &VoidRequest) -> Result<PaymentResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "void_payment"))
    }

    /// Refund a settled payment (full when amount is None; partial when Some).
    async fn refund(&self, _req: &RefundRequest) -> Result<RefundResult, PaymentError> {
        Err(PaymentError::unsupported(self.id(), "refund"))
    }

    /// Verify authenticity over raw bytes/query params, scrub secrets, and normalize event.
    async fn parse_and_verify_webhook(
        &self,
        raw: &RawWebhookRequest,
    ) -> Result<NormalizedWebhookEvent, PaymentError>;
}
```

### TypeScript Blueprint: Provider Interface and Portable Runtime Contract

Mirror the identical SPI in TypeScript using `readonly` value types, `Uint8Array | string` raw webhook bodies (compatible with Node, Bun, Deno, and Web Fetch workers), and optional method declarations paired with explicit capability sets:

```typescript
// @some-of-cs/some-payment-core/src/provider.ts
export type BuiltinProviderId = 'stripe' | 'moyasar' | 'paymob';
export type ProviderId = BuiltinProviderId | (string & {});

export type PaymentCapability =
  | 'direct_payment'
  | 'embedded_intent'
  | 'hosted_checkout'
  | 'auth_capture'
  | 'partial_capture'
  | 'void_authorized'
  | 'void_settled_window'
  | 'refund'
  | 'partial_refund'
  | 'native_create_idempotency'
  | 'native_mutation_idempotency'
  | 'saved_cards'
  | 'splits'
  | 'webhook_verification_handshake';

export interface Money {
  /** Strict positive integer in the currency's smallest minor unit (never float). */
  readonly amount: number;
  /** Uppercase ISO-4217 three-letter currency code (e.g., "USD", "SAR", "EGP", "KWD"). */
  readonly currency: string;
}

export interface RawWebhookRequest {
  readonly rawBody: string | Uint8Array;
  readonly headers: Readonly<Record<string, string | undefined>>;
  readonly queryParams: Readonly<Record<string, string | undefined>>;
  readonly method: 'GET' | 'POST';
}

export interface PaymentProvider {
  readonly id: ProviderId;
  readonly capabilities: ReadonlySet<PaymentCapability>;

  createPayment(req: CreatePaymentRequest): Promise<PaymentResult>;
  getPayment(ref: ProviderReference): Promise<PaymentResult>;
  createCheckout?(req: CheckoutSessionRequest): Promise<PaymentResult>;
  capture?(req: CaptureRequest): Promise<PaymentResult>;
  voidPayment?(req: VoidRequest): Promise<PaymentResult>;
  refund?(req: RefundRequest): Promise<RefundResult>;
  verifyAndParseWebhook(raw: RawWebhookRequest): Promise<NormalizedWebhookEvent>;
}
```

## Required Changes Over `some-stripe`, `some-moyasar`, and `some-paymob`

Because the source code of `some-stripe`, `some-moyasar`, and `some-paymob` could not be inspected directly, treat the following subsections as a concrete **pre-integration audit and refactoring specification**. Every item tagged **P0 (Blocker)** must be verified as already present or implemented in the sibling package before `some-payment` routes live transactions through it.

### Shared Changes Required Across All Three Packages

1. **Eliminate Global Configuration Singletons (P0):** Construct isolated client instances (`StripeClient::new(cfg)` / `new StripeClient(cfg)`) rather than storing mutable global statics or reading environment variables implicitly inside request functions. Multi-tenant white-label SaaS applications require distinct tenant credentials to coexist safely inside a single process.
2. **Standardize Integer Minor-Unit Inputs (P0):** Audit every monetary parameter across `some-stripe`, `some-moyasar`, and `some-paymob` so low-level API methods accept integer minor units exclusively—removing any hidden `amount * 100` multiplication that would cause double-scaling when invoked from `some-payment`.
3. **Expose Injectable HTTP Transport and Explicit Timeouts (P1):** Accept an optional shared HTTP client (`reqwest::Client` in Rust or custom `fetch` in TypeScript) plus configurable request timeouts (e.g., 30 seconds default) to allow connection-pool reuse, edge runtime portability, and deterministic offline testing.
4. **Enforce Secret Redaction in Logs, Debug Output, and Errors (P0):** Implement custom `Debug` (Rust) or `toJSON()` / `util.inspect.custom` (TypeScript) redaction on configs, error types, and raw payloads so secret keys (`sk_...`, `whsec_...`, Paymob `api_key`/`hmac_secret`, Moyasar `secret_token`) and cardholder details never leak to logs or error trackers.
5. **Return Structured Typed Provider Errors (P0):** Replace stringified or opaque transport errors with typed error structs preserving HTTP status code, parsed vendor error type/code, message, `retry_after` hints (for HTTP 429), and redacted response metadata so `some-payment` can classify retryability accurately.
6. **Add Optional `some-payment-core` Adapter Support (P0 for Option A):** Add `some-payment-core` as an optional dependency behind `#[cfg(feature = "some-payment-adapter")]` (Rust) or a `./adapter` subpath export (TypeScript) implementing `PaymentProvider` without altering standalone usage.

### Required Changes in `some-stripe`

1. **Pin and Configure `Stripe-Version` (P1):** Send an explicit `Stripe-Version` header (current version as of November 2026: `2026-11-18.endive`) from client config instead of inheriting mutable Stripe Dashboard defaults, preventing unannounced breakage across Stripe's twice-yearly major API releases.
2. **Cover Both `PaymentIntents` and `CheckoutSessions` (P0):** Ensure `some-stripe` exposes `PaymentIntent` (`create` with `capture_method: automatic | manual`, `retrieve`, `capture` with optional `amount_to_capture`, `cancel`) alongside `CheckoutSession` (`create` returning hosted `url`, `retrieve`, `expire` for open sessions). Enforce Stripe's rule that a Checkout Session's underlying PaymentIntent can only be canceled directly when its status is `requires_capture`—otherwise call `expire` on the Checkout Session.
3. **Deserialize `last_payment_error` and Never Check `status == "failed"` (P0):** Stripe PaymentIntents transition through exactly seven statuses (`requires_payment_method`, `requires_confirmation`, `requires_action`, `processing`, `requires_capture`, `canceled`, `succeeded`); a declined attempt returns the intent to `requires_payment_method` with failure details in `last_payment_error`. Deserialize `last_payment_error` (`type`, `code`, `decline_code`, `advice_code`, `network_decline_code`, `message`) so `some-payment` can surface actionable decline errors.
4. **Support the Full Refund Resource Lifecycle (P0):** Treat Stripe refunds as first-class `re_...` resources supporting multiple partial refunds, all five statuses (`pending` + `pending_reason`, `requires_action` + `next_action`, `succeeded`, `failed` + `failure_reason` + `failure_balance_transaction`, `canceled`), and `POST /v1/refunds/:id/cancel` for `requires_action` refunds.
5. **Enforce Stripe Currency Normalization and Rounding Guardrails (P0):** Lowercase all ISO-4217 codes before sending requests. Skip x100 scaling on zero-decimal currencies (`JPY`, `KRW`, etc.), apply Stripe's mandatory two-decimal `00` encoding on `ISK` and `UGX`, and enforce Stripe's three-decimal trailing-0 multiple-of-10 requirement (`BHD`, `JOD`, `KWD`, `OMR`, `TND`, per archived Stripe documentation tracked by `python-getpaid-stripe`).
6. **Forward Caller-Supplied `Idempotency-Key` Safely (P0):** Accept an explicit `idempotency_key` (up to 255 characters) on every mutating `POST` request and transmit it in the `Idempotency-Key` header. If `some-stripe` includes built-in transport retries, reuse the exact same key across retries of a single logical attempt—never generate a new UUID per HTTP attempt.
7. **Harden Raw-Body Webhook Verification (P0):** Require exact unparsed bytes/string (rejecting pre-parsed objects), parse `Stripe-Signature` without assuming fixed entry order or count, verify against **every `v1` signature** (supporting Stripe's zero-downtime secret rotation window of up to 7 days in Dashboard), ignore `v0` test signatures and unsigned debug entries (`secret`, `destination`, `payload`), compare digests in constant time, enforce a configurable replay tolerance defaulting to **300 seconds**, and distinguish snapshot events (`data.object`) from thin events (which require API hydration).

### Required Changes in `some-moyasar`

1. **Replace `sandbox` Flags with Key-Prefix Detection and Scope Guards (P0):** Remove any boolean `sandbox` option because Moyasar routes both test and live traffic to `https://api.moyasar.com/v1` and determines mode strictly from the API key prefix (`sk_test_`/`pk_test_` vs. `sk_live_`/`pk_live_`). Authenticate with HTTP Basic (`username = key`, `password = ""`) and fail closed locally if a `pk_` publishable key is supplied to secret-key-only operations (`create_payment_intent`, `create_invoice`, `fetch`, `capture`, `void`, `refund`).
2. **Add Payment Intents API and Invoices API Coverage (P0):** Alongside `POST /v1/payments`, implement the **Payment Intents API** (`POST /v1/payment_intents`, `GET`, `Cancel`, and `POST /v1/payment_intents/:id/fulfill`) returning single-use `^pi_secret_` client secrets (1-hour expiry; consumed even on card decline, after which retries fail with `intent_not_fulfillable`), and implement the **Invoices API** (`POST /v1/invoices`, `GET`, `PUT`, `DELETE`) so `some-payment`'s `create_checkout()` works natively via Moyasar's hosted `invoice.url`.
3. **Default-Deny Raw Card PANs on the Server (P1):** Unless an explicit PCI-certified override (`allow_raw_cards: true`) is enabled, reject `source.type == "creditcard"` pre-flight with `ValidationError`, guiding callers to tokenized cards (`source.type == "token"`), digital wallets (`applepay`, `samsungpay`), `stcpay`, `sadadbill`, Payment Intents, or hosted Invoices.
4. **Inspect `payment.status` on Every HTTP 201 Created Response (P0):** Because Moyasar returns HTTP 201 for `initiated`, `failed`, and `paid` payments alike, always inspect `payment.status` and `source.type` before building `PaymentResult`. Map `failed` to `PaymentStatus::Failed` (never `Paid`), `initiated` with card/token to `NextAction::Redirect(source.transaction_url)`, `initiated` with `stcpay` to `NextAction::OtpChallenge(source.transaction_url)` (never browser-redirecting STC Pay), `sadadbill` to `NextAction::OfflineBill`, and `verified` to `PaymentStatus::SetupCompleted` rather than `Authorized`.
5. **Complete Capture, Void, Refund, and Pre-Flight Validation Rules (P0):** Validate UUID format pre-flight on `get`, `capture`, `void`, and `refund`; reject `manual: true` pre-flight for decrypted Apple Pay (DPAN) and STC Pay (as enforced in `@abshahin/payments-sdk`); send an empty request body for full capture (`POST /v1/payments/:id/capture`) and full refund (`POST /v1/payments/:id/refund`), or `{ "amount": <minor> }` for partial operations; ignore any canonical `refund.reason` cleanly since Moyasar's refund API has no reason field; implement void (`POST /v1/payments/:id/void`, empty body, accounting for the 14-day mada authorized window and ~2-hour paid/captured pre-settlement window); and validate Moyasar metadata constraints (<= 30 entries, key <= 40 chars, value <= 500 chars) plus splits sum equality.
6. **Forward `given_id` Idempotency and Declare Non-Idempotent Mutations (P0):** Map `CreatePaymentRequest.idempotency_key` (validated as UUID v4) to Moyasar's `given_id` parameter on `POST /v1/payments` and `POST /v1/payment_intents/:id/fulfill`. Set `NativeCreateIdempotency = true` and `NativeMutationIdempotency = false` in `capabilities()`, because Moyasar provides no native API idempotency for capture, void, or refund and therefore relies on `some-payment`'s shared `IdempotencyStore` in multi-worker production environments.
7. **Implement Constant-Time `secret_token` Verification, Secret Scrubbing, and Verification Handshake (P0):** Moyasar's official November 2026 Webhook Reference specifies **no HTTP signature headers**; instead, Moyasar embeds `secret_token` directly inside the JSON body (note that some third-party integration docs loosely refer to HMAC-SHA256, so adhere strictly to Moyasar's primary reference). Compare `payload.secret_token` in constant time, scrub `secret_token` from `raw_redacted` before returning, map `payload.live` to `event.livemode`, handle the one-shot unretried `webhook_verification` event (6-digit `data.verification_code`, 30-minute TTL, max 5 wrong attempts) with helper methods `verify_webhook(id, code)` and `resend_verification(id)`, and normalize all 10 live API events from `/v1/webhooks/available_events` (`payment_paid`, `payment_failed`, `payment_voided`, `payment_authorized`, `payment_captured`, `payment_refunded`, `payment_abandoned`, `payment_verified`, `card_auth_authenticated`, `card_auth_failed`) while defensively accepting `payment_faild` (which appears as a typo in Moyasar's Webhook Reference table).
8. **Correctly Classify `3ds_auth_error` (P1):** Map Moyasar's `3ds_auth_error` to `PaymentError::ThreeDsFailed` (or `CardDeclined`), never to `AuthenticationError`, which should be reserved for invalid merchant API keys (`authentication_error` / HTTP 401).

### Required Changes in `some-paymob`

Paymob requires the most substantial normalization when moving from a standalone wrapper into `some-payment`, especially if `some-paymob` was initially built against Paymob's legacy 3-step Accept API (`auth/tokens` -> `ecommerce/orders` -> `payment_keys` -> iframe).

1. **Implement the Unified Intention API (`POST /v1/intention/`) and Checkout Builders (P0):** Add first-class support for `POST /v1/intention/` authenticated via `Authorization: Token <secret_key>`. Return `id`, `intention_order_id`, and `client_secret`, and provide both a Unified Checkout redirect URL builder (`{regional_base_url}/unifiedcheckout/?publicKey={public_key}&clientSecret={client_secret}`, with an optional `checkout_base_url` override for configurations using `https://eg.checkout.paymob.com`) and raw `client_secret` output for embedded Paymob Pixel checkout.
2. **Add Regional Base-URL Routing (`PaymobRegion`) (P0):** Replace any hardcoded `https://accept.paymob.com` URL with a `PaymobRegion` configuration enum covering Egypt, Saudi Arabia, UAE, Oman, Pakistan, and custom hosts (see the reference table below). Remember that Paymob uses the same regional host for both sandbox and live environments, switching mode solely through keys and Integration IDs.
3. **Add a Concurrency-Safe Dual-Authentication Manager (P0):** Support `Authorization: Token <secret_key>` for Intention, Void, and Refund endpoints (per Paymob's 2026 official developer docs) alongside a cached `AuthTokenManager` that exchanges `api_key` at `POST /api/auth/tokens` for a 60-minute `auth_token` required by Transaction Inquiry, Card Token Inquiry, and Paymob's OpenAPI-documented Capture endpoint. Proactively refresh cached tokens before expiry (e.g., at 50 minutes), synchronize concurrent refreshes with a single-flight lock, and retry once on HTTP 401.
4. **Map Integration IDs and Enforce Auth/Capture Routing (P0):** Configure `some-paymob` with `default_integration_ids`, an optional `auth_integration_id` (selected automatically when `capture_mode == Manual`), and optional named aliases (`card`, `wallet`, `apple_pay`). Fail closed with `ConfigurationError` if `capture_mode == Manual` is requested without an `auth_integration_id`, and remind operators that Integration ID mode (Test/Live) and currency must match both the secret key and the request currency.
5. **Validate Conditional Intention Payload Constraints Pre-Flight (P1):** Enforce Paymob's strict schema checks before calling the API: when `items` is non-empty, every item requires `name` and `amount`; when `billing_data` is supplied, `phone_number` is mandatory. Always map `CreatePaymentRequest.order_id` into `special_reference` so it surfaces as `merchant_order_id` in callbacks and supports fallback inquiry.
6. **Correlate `transaction_id` and Support Full-Refund Amount Resolution (P0):** Because Paymob's Capture, Void, and Refund endpoints require the numeric `transaction_id` (`obj.id`) rather than the Intention ID, always populate `ProviderReference.transaction_id` from callbacks and inquiry responses. When a caller invokes `get_payment`, `capture`, `void_payment`, or `refund` with only `order_id` or `special_reference`, optionally resolve the latest numeric `transaction_id` via `POST /api/ecommerce/orders/transaction_inquiry`. Because Paymob's Refund endpoint requires `amount_cents` even for full refunds, resolve `amount_cents - refunded_amount_cents` via `GET /api/acceptance/transactions/{transaction_id}` whenever `RefundRequest.amount` is `None`.
7. **Synthesize Canonical Status and Parent/Child Transaction Flags (P0):** Build a deterministic status mapper over Paymob's flags (`pending`, `success`, `error_occured` [note Paymob's exact single-r spelling], `is_auth`, `is_capture`, `is_captured`, `captured_amount`, `is_void`, `is_voided`, `is_refund`, `is_refunded`, `refunded_amount_cents`). Evaluate in strict order: `pending == true` -> `Processing`; `success == false || error_occured == true` -> `Failed`; `is_voided == true` -> `Voided`; `is_refunded == true || refunded_amount_cents >= amount_cents` -> `Refunded`; `refunded_amount_cents > 0` -> `PartiallyRefunded`; `is_auth && !is_captured && captured_amount == 0` -> `Authorized`; `is_auth && captured_amount > 0 && captured_amount < amount_cents` -> `PartiallyCaptured`; `is_captured` -> `Captured`; otherwise -> `Paid`. Crucially, distinguish child action flags (`is_void`, `is_refund`, `is_capture`, which carry `parent_transaction`) from parent-state flags (`is_voided`, `is_refunded`, `is_captured`) so callbacks on child operations correlate cleanly to the original payment record.
8. **Implement All Three Paymob HMAC-SHA512 Verifiers (P0):** Because Paymob sends both transaction callbacks and saved-card `TOKEN` callbacks to the same `notification_url`, and uses a third format for subscriptions, `some-paymob` must dispatch and verify all three in constant time using the exact specifications tabulated below.

#### Paymob Regional Hosts, Auth Split, and Triple HMAC-SHA512 Reference

| Paymob Subsystem | Exact Specification Required in `some-paymob` |
| :--- | :--- |
| **Regional Base URLs** | Egypt: `https://accept.paymob.com` (widget/checkout host override: `https://eg.checkout.paymob.com`); KSA: `https://ksa.paymob.com`; UAE: `https://uae.paymob.com`; Oman: `https://oman.paymob.com`; Pakistan: `https://pakistan.paymob.com` (per Paymob OpenAPI spec on APIs.io). |
| **Auth Scheme Split** | `Authorization: Token <secret_key>` on `POST /v1/intention/`, `POST /api/acceptance/void_refund/void`, and `POST /api/acceptance/void_refund/refund`. Cached 60-min `auth_token` (from `POST /api/auth/tokens`) via `Bearer` header on `GET /api/acceptance/transactions/{id}` and `POST /api/acceptance/capture` (per Paymob OpenAPI spec on APIs.io), and in JSON body on `POST /api/ecommerce/orders/transaction_inquiry` and `POST /api/acceptance/order_card_tokens`. |
| **HMAC 1: Transaction Callback (POST JSON & GET Query)** | Concatenate values of 20 lexicographically sorted keys with lowercase booleans (`true`/`false`): `amount_cents`, `created_at`, `currency`, `error_occured`, `has_parent_transaction`, `obj.id` (POST) / `id` (GET), `integration_id`, `is_3d_secure`, `is_auth`, `is_capture`, `is_refunded`, `is_standalone_payment`, `is_voided`, `order.id` (POST) / `order_id` (GET), `owner`, `pending`, `source_data.pan`, `source_data.sub_type`, `source_data.type`, `success`. Hash with `HMAC-SHA512(hmac_secret)` hex and compare against query param `?hmac=`. |
| **HMAC 2: Card Token Callback (`type == "TOKEN"`)** | Concatenate values of 8 lexicographically sorted keys from `obj`: `card_subtype`, `created_at`, `email`, `id`, `masked_pan`, `merchant_id`, `order_id`, `token`. Hash with `HMAC-SHA512(hmac_secret)` hex and compare against query param `?hmac=`. |
| **HMAC 3: Subscription Callback** | Concatenate `"{trigger_type}for{subscription_data.id}"` (e.g., `"suspendedfor1264"`). Hash with `HMAC-SHA512(hmac_secret)` hex and compare against the `hmac` field **inside the JSON request body**. |

### Summary Package Migration Matrix

| Package | Priority | Summary of Required Changes | Breaking Risk for Existing Callers |
| :--- | :--- | :--- | :--- |
| **All Three** | **P0** | Remove global singletons; enforce integer minor-unit amounts; redact secrets in logs/errors; typed error hierarchy; optional `some-payment-core` adapter feature/subpath. | Medium (config constructors and integer amount types if previously float/global). |
| **`some-stripe`** | **P0** | Complete `PaymentIntent`, `CheckoutSession`, and `Refund` coverage; `last_payment_error` parsing; `Idempotency-Key` passthrough; raw-body multi-`v1` HMAC-SHA256 verifier (ignoring `v0`, 300s tolerance, thin-event check); ISK/UGX/3-decimal rules. | Low–Medium (mostly additive; webhook verification must accept raw bytes). |
| **`some-stripe`** | **P1** | Explicit `Stripe-Version` pinning (`2026-11-18.endive`) and injectable HTTP transport. | Low (additive config options). |
| **`some-moyasar`** | **P0** | Key-prefix mode detection + `pk_`/`sk_` scope guards; Payment Intents and Invoices APIs; status inspection on HTTP 201; full capture/void/refund + UUID checks; STC Pay OTP vs. 3DS distinction; `given_id` forwarding; constant-time `secret_token` verifier + scrubbing + `webhook_verification` handshake + `payment_faild` tolerance. | Medium (deprecating `sandbox` flag and changing raw-201 assumptions). |
| **`some-moyasar`** | **P1** | Default-deny raw server-side PANs; map `3ds_auth_error` to `ThreeDsFailed`; metadata and splits pre-flight checks. | Low (opt-in flag preserves PCI-certified callers). |
| **`some-paymob`** | **P0** | Unified `/v1/intention/` + Unified Checkout/Pixel; `PaymobRegion` router; dual-auth manager (`Token <sk>` + cached 60-min `auth_token`); `auth_integration_id` routing; `transaction_id` correlation + full-refund lookup; boolean-flag status synthesizer; all 3 HMAC-SHA512 verifiers. | High if currently legacy-v2-only (retain legacy endpoints deprecated while making Intention primary). |

## Implementing `some-payment` Step by Step

Once `some-payment-core` is published and the three provider packages satisfy the audit above, `some-payment` stays lean and provider-agnostic, focusing on orchestration, routing, lifecycle hooks, idempotency, and webhook dispatch.

### Repository Module Layout

| Module (Rust / TypeScript) | Responsibility | Coupling Rule |
| :--- | :--- | :--- |
| `src/lib.rs` / `src/index.ts` | Public entry point exporting `PaymentOrchestrator`, `RoutingPolicy`, `LifecycleHooks`, `IdempotencyStore`, and re-exporting `some-payment-core`. | Zero references to concrete providers. |
| `src/orchestrator.rs` / `src/client.ts` | Executes the unified five-step pipeline (`create_payment`, `create_checkout`, `get_payment`, `capture`, `void_payment`, `refund`). | Dispatches strictly via `PaymentProvider` trait/interface. |
| `src/registry.rs` / `src/registry.ts` | Thread-safe `ProviderId -> Arc<dyn PaymentProvider>` map supporting startup registration, default provider lookup, and runtime capability queries. | Open string keys; no closed provider enum. |
| `src/routing.rs` / `src/routing.ts` | Deterministic routing policies (inspired by `@paykernel/routing` and Hyperswitch)—e.g., SAR/mada -> `moyasar`, EGP/wallets -> `paymob`, international USD/EUR -> `stripe`, with explicit per-call override always taking priority. | Pure selection function over request context + registry capabilities. |
| `src/hooks.rs` / `src/hooks/` | Composable `before`, `after`, and `on_error` hooks for telemetry, audit logging, fraud checks, and ledger persistence. | Runs uniformly around every registered provider. |
| `src/idempotency.rs` / `src/idempotency.ts` | `IdempotencyStore` contract (`reserve`, `complete`, `release`) plus an `InMemoryIdempotencyStore` strictly for local development and tests. | Shared Redis/SQL implementation injected by host app in production. |
| `src/webhooks.rs` / `src/webhooks.ts` | Provider-scoped signature verification, secret redaction enforcement, Moyasar handshake routing, and optional deduplication hook. | Operates exclusively on `RawWebhookRequest`. |
| `src/providers/` | Optional convenience constructors (`stripe_provider`, `moyasar_provider`, `paymob_provider`). | Compiled/exported strictly per active Cargo feature or npm subpath. |

### Orchestrator Execution Pipeline and Lifecycle Hook Semantics

Every public payment operation on `PaymentOrchestrator` executes a uniform five-step template method:

1. **Resolve Target Provider:** Select the caller's explicit `provider_id` if provided; otherwise evaluate `RoutingPolicy::select(&request, &registry)`, falling back to `default_provider`. Return `PaymentError::ConfigurationError` if no matching provider is registered.
2. **Enforce Pre-Flight Capability and Money Guards:** Verify `provider.capabilities()` claims every required capability (for example, `AuthCapture` when `capture_mode == Manual`, `PartialCapture` when `capture.amount` is set, or `Splits` when splits are attached) and assert `money.amount > 0` integer bounds. Fail closed before touching the network.
3. **Run `before` Lifecycle Hooks:** Execute registered `before` hooks sequentially. Hooks may inspect or enrich metadata or abort execution by returning `HookDecision::Abort(reason)`, which halts the call with `PaymentError::AbortedByHook` before any gateway request is dispatched.
4. **Invoke Provider Within Idempotency Boundary:** Forward native idempotency keys where supported (Stripe header, Moyasar `given_id`), and wrap non-idempotent mutations (Moyasar and Paymob capture, void, and refund) inside the shared `IdempotencyStore` reservation boundary.
5. **Run `after` or `on_error` Hooks (Non-Reversible Boundary):** If the gateway call failed, run `on_error` hooks and propagate the normalized `PaymentError`. If the gateway call succeeded, run `after` hooks—**never convert a failing `after` hook into a thrown payment error or abort**. As emphasized in production multi-gateway SDKs, after-hooks cannot undo settled money; throwing after a real charge succeeds invites upstream retries and double-charges, so `some-payment` must log the failure, emit a reconciliation alert, and still return the successful `PaymentResult`.

### Production Idempotency Architecture

Because native idempotency guarantees diverge sharply across the three gateways, `some-payment` separates attempt-level creation idempotency from distributed mutation idempotency. For payment creation, host applications should mint one UUID v4 idempotency key (compatible with both Stripe's <= 255-character `Idempotency-Key` header and Moyasar's UUID `given_id`) when the customer confirms a specific purchase attempt, persist it before calling `some-payment`, and reuse it across 5xx and timeout retries. Whenever a customer retries after a declined card or changes the amount, mint a **new** attempt record and a new UUID: reusing a declined `given_id` causes Moyasar to return HTTP 400 `"Payment is already created"` or creates terminal-state webhook conflicts. Keep retention horizons in mind as well: Stripe API v1 retains keys for at least **24 hours** before pruning (API v2 retains keys for **30 days**), so old keys cannot prevent duplicates indefinitely.

For post-payment mutations (`capture`, `void_payment`, `refund`), neither Moyasar nor Paymob provides native API idempotency. In-memory maps fail across container restarts, serverless functions, and multi-worker deployments, so `some-payment` should expose a pluggable `IdempotencyStore` backed by Redis (`SET NX EX`) or PostgreSQL (`INSERT ... ON CONFLICT`) in production:

```rust
// some-payment-core/src/idempotency.rs
#[derive(Debug, Clone)]
pub enum ReserveOutcome {
    /// Caller acquired the atomic lock and may invoke the provider mutation.
    Acquired,
    /// Earlier execution finished; return the cached serialized response.
    Completed(serde_json::Value),
    /// Another worker holds the active lock; fail with IdempotencyConflict.
    InProgress,
}

#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    async fn reserve(&self, scope: &str, key: &str, ttl_secs: u64) -> Result<ReserveOutcome, PaymentError>;
    async fn complete(&self, scope: &str, key: &str, payload: &serde_json::Value, ttl_secs: u64) -> Result<(), PaymentError>;
    /// Invoke ONLY on definitive pre-flight or 4xx client rejections — NEVER on 5xx or timeouts.
    async fn release(&self, scope: &str, key: &str) -> Result<(), PaymentError>;
}
```

A crucial operational safeguard governs `release()`: never release a mutation lock after a network timeout or HTTP 500 on a non-idempotent Moyasar or Paymob refund, because the provider may have executed the refund before the connection dropped. Keep the key locked in an indeterminate state until background reconciliation queries `get_payment()` to verify whether `refunded_amount` changed.

### Unified Webhook Endpoint Pipeline

Expose a single parameterized route in your host backend—such as `POST /webhooks/payments/:provider` (plus `GET /webhooks/payments/paymob/return` solely for customer-facing redirect UX)—and enforce a five-step ingestion pipeline:

1. **Read Exact Unparsed Request Bytes Before Middleware:** In Actix-web (Rust), extract `actix_web::web::Bytes` rather than `web::Json`; in Next.js App Router, call `await req.text()`; in Express, mount `express.raw({ type: 'application/json' })` ahead of `app.use(express.json())`. Exempt webhook endpoints from CSRF validation and ensure HTTPS uses TLS 1.2 or 1.3.
2. **Verify Authenticity Before Database Reads or Deduplication:** Pass `RawWebhookRequest` to `orchestrator.handle_webhook(&provider_id, &raw).await`. Stripe validates HMAC-SHA256 + 300-second replay window; Moyasar checks `secret_token` in constant time and scrubs it; Paymob checks the matching 20-key, 8-key, or subscription HMAC-SHA512. Immediately return HTTP 400 or 401 on `WebhookVerificationFailed` and emit a structured security alert.
3. **Process Provider Verification Handshakes:** When `event.verification_challenge` is populated (Moyasar's unretried `webhook_verification` event), automatically invoke `some-moyasar`'s `verify_webhook(webhook_id, verification_code)` endpoint and respond `200 OK`.
4. **Deduplicate Durably on `(provider, event.event_id)`:** Gateways retry deliveries aggressively—Stripe up to 3 days in live mode, Moyasar up to 5 retries across 1m, 10m, 30m, 1h, and 2h intervals—and do not guarantee delivery order. Record `(provider, event_id)` in a unique-indexed `processed_webhook_events` table inside the same database transaction as your order state update; if already recorded, return `200 OK` without re-triggering fulfillment.
5. **Acknowledge Fast (2xx) and Validate Monotonic Transitions and Totals:** Return `200 OK` promptly and dispatch heavy side effects (emails, PDF invoicing, external fulfillment) onto an async queue. Before marking an order paid, verify that `event.money` matches the persisted order's amount and currency and guard against out-of-order regressions (such as a late `Authorized` or `Processing` event arriving after `Paid` or `Refunded`).

### Testing and Conformance Verification Strategy

Ship a reusable `ProviderConformanceSuite` inside `some-payment-core` (or a `some-payment-testing` companion crate/package) that accepts any `PaymentProvider` wired to offline mock HTTP transports (`stripe-mock` Docker container on port 12111, `wiremock` in Rust, or custom `fetch` stubs in TypeScript) and verifies every behavioral invariant before release.

| Conformance Test Category | Core Contract Assertions | Specific Provider Edge Cases Verified |
| :--- | :--- | :--- |
| **Money & Currency Vectors** | Exact minor-unit wire serialization and case normalization across 0-, 2-, and 3-decimal currencies. | USD/SAR/EGP (x100), JPY (0-dec), KWD (3-dec + Stripe trailing-0 rule), Stripe ISK/UGX x100 special cases. |
| **Decline & Ambiguous Status Handling** | Non-paid creation responses never trigger `Paid`; decline codes surface cleanly. | Moyasar HTTP 201 + `status: "failed"`; Stripe `requires_payment_method` + `last_payment_error`; Paymob `error_occured: true`. |
| **Webhook Security Fixtures** | Constant-time rejection of tampered payloads, stale timestamps, and forged signatures; complete secret redaction. | Stripe re-serialized JSON rejection, stale `t > 300s`, `v0`-only downgrade attack, dual-`v1` rotation; Moyasar `secret_token` scrubbing + `payment_faild`; Paymob 20-key POST/GET, 8-key TOKEN, and body-hmac subscription fixtures. |
| **Fail-Closed Capability Matrix** | Unclaimed operations and invalid combinations fail locally with zero outbound HTTP calls. | Moyasar `manual: true` on STC Pay/DPAN and SADAD refund; Paymob `Manual` without `auth_integration_id`; Stripe cancel on non-`requires_capture` Checkout Session. |
| **Idempotency & Lock Retention** | Duplicate keys replay cached results without duplicate provider calls; uncertain timeouts do not unlock non-idempotent mutations. | Stripe `Idempotency-Key` reuse across retries; Moyasar `given_id` collision; shared `IdempotencyStore` protecting Moyasar/Paymob refunds. |

## Extending `some-payment` With Future Providers

Because `some-payment` routes over the open `PaymentProvider` contract in `some-payment-core` using open `ProviderId` identifiers rather than a closed enum, adding a fourth or tenth provider—such as **Tap Payments**, **Tamara**, **Tabby**, **MyFatoorah**, **Fawry**, **PayTabs**, or **PayPal**—requires **zero code modifications to `some-payment` or `some-payment-core`**.

1. **Create an Independent Provider Package:** Create `some-<provider>` depending only on `some-payment-core` (never on `some-payment`), with instance-scoped configuration, injectable HTTP transport, and redacted logging/debug formatting.
2. **Declare Exact `PaymentCapabilities` (Fail-Closed by Default):** Audit the new gateway's API and claim only verified capabilities—for example, a hosted-only gateway like MyFatoorah V3 claims `HostedCheckout`, `Refund`, and `PartialRefund` while leaving `AuthCapture`, `VoidAuthorized`, and `Splits` unclaimed so `some-payment` rejects unsupported calls automatically.
3. **Isolate Wire Schemas Behind Pure Adapters:** Translate between canonical `some-payment-core` types and vendor JSON inside private adapter modules (`TryFrom` implementations in Rust; pure mapper modules in TypeScript), separating customer declines (`CardDeclined`, `ThreeDsFailed`) from merchant/infrastructure errors (`AuthenticationError`, `ProviderUnavailable`).
4. **Implement Webhook Verification and Run `ProviderConformanceSuite`:** Implement constant-time cryptographic verification over `RawWebhookRequest` (or, for gateways lacking signed webhooks like Hesabe, verify transaction facts asynchronously via the gateway's inquiry API before emitting a normalized event), and pass the shared conformance test suite in CI.
5. **Register the Provider at Application Startup:** Wire the new package directly into `PaymentOrchestrator` and optionally add currency or method rules to your `RoutingPolicy`:

```rust
// Host application bootstrap (Rust) — zero edits to `some-payment`
let orchestrator = PaymentOrchestrator::builder()
    .with_idempotency_store(Arc::new(RedisIdempotencyStore::new(redis_pool)))
    .register(Arc::new(some_stripe::StripeProvider::new(stripe_cfg)?))
    .register(Arc::new(some_moyasar::MoyasarProvider::new(moyasar_cfg)?))
    .register(Arc::new(some_paymob::PaymobProvider::new(paymob_cfg)?))
    // Plug in any future first-party or third-party provider seamlessly:
    .register(Arc::new(some_tap::TapProvider::new(tap_cfg)?))
    .with_routing_policy(Arc::new(RegionalCurrencyRouter::default()))
    .with_default_provider(ProviderId::new("stripe"))
    .build()?;
```

```typescript
// Host application bootstrap (TypeScript) — zero edits to `@some-of-cs/some-payment`
import { createPaymentOrchestrator } from '@some-of-cs/some-payment';
import { stripeProvider } from '@some-of-cs/some-payment/stripe';
import { moyasarProvider } from '@some-of-cs/some-payment/moyasar';
import { paymobProvider } from '@some-of-cs/some-payment/paymob';
import { tapProvider } from '@some-of-cs/some-tap'; // future external plugin

export const payments = createPaymentOrchestrator({
  defaultProvider: 'stripe',
  idempotencyStore: redisIdempotencyStore,
  providers: [
    stripeProvider(stripeConfig),
    moyasarProvider(moyasarConfig),
    paymobProvider(paymobConfig),
    tapProvider(tapConfig),
  ],
  routingPolicy: ({ money }) => {
    if (money.currency === 'SAR') return 'moyasar';
    if (money.currency === 'EGP') return 'paymob';
    if (money.currency === 'KWD') return 'tap';
    return 'stripe';
  },
});
```

### New Provider Readiness Acceptance Checklist

Before promoting any new `some-<provider>` package into production under `some-payment`, confirm all ten engineering gates:

1. Zero runtime dependency on `some-payment` (depends strictly on `some-payment-core`).
2. Zero global configuration singletons; supports concurrent multi-tenant instances in one process.
3. Strict integer minor-unit `Money` serialization with automated tests covering 0-, 2-, and 3-decimal currencies.
4. Explicit `PaymentCapabilities` declaration with fail-closed tests for every unclaimed operation.
5. Complete `ProviderReference` correlation across create, webhook, inquiry, capture, void, and refund flows.
6. Explicit `NextAction` discrimination separating browser redirects, embedded client secrets, API OTP prompts, and offline bills.
7. Deterministic `PaymentStatus` mapping verified against live success, decline, partial-capture, and partial-refund fixtures.
8. Raw-body constant-time webhook verification (or authenticated inquiry verification) with all secrets scrubbed from `raw_redacted`.
9. Accurate `NativeCreateIdempotency` / `NativeMutationIdempotency` declarations wired cleanly into `some-payment`'s `IdempotencyStore`.
10. Full `ProviderConformanceSuite` passing in CI with verified secret redaction across logs and error outputs.

## Works Cited

1. Create Intention - Paymob Documentation, https://developers.paymob.com/paymob-docs/intention-apis/create-intention
2. Versioning | Stripe API Reference, https://docs.stripe.com/api/versioning
3. The Story Behind Rust & How Code Really Works - YouTube, https://www.youtube.com/watch?v=KvniB0B7X-8
4. Receive Stripe events in your webhook endpoint, https://docs.stripe.com/webhooks
5. Supported currencies - Stripe Documentation, https://docs.stripe.com/currencies
6. API v2 overview - Stripe Documentation, https://docs.stripe.com/api-v2-overview
7. The PaymentIntent object | Stripe API Reference, https://docs.stripe.com/api/payment_intents/object
8. The Refund object | Stripe API Reference, https://docs.stripe.com/api/refunds/object
9. Payment Operations | Moyasar Online Docs, https://docs.moyasar.com/guides/payment-operations
10. Webhook Reference | Moyasar Online Docs, https://docs.moyasar.com/api/other/webhooks/webhook-reference
11. Creating invoices | Moyasar Online Docs, https://docs.moyasar.com/guides/invoices/creating-invoices
12. Subscription Callback and HMAC Calculation, https://developers.paymob.com/paymob-docs/developers/subscription/hmac-calculation-for-subscription-callback
13. Void - Paymob Documentation, https://developers.paymob.com/paymob-docs/developers/manage-payment-apis/void
14. HMAC Transaction Callback - Paymob Developer Portal, https://developers.paymob.com/paymob-docs/developers/webhook-callbacks-and-hmac/hmac/hmac-transaction-callback
15. stripe-amounts-currencies.md - django-getpaid - GitHub, https://github.com/django-getpaid/python-getpaid-stripe/blob/main/docs/research/stripe-amounts-currencies.md
16. payments-sdk/docs/moyasar.md at master - GitHub, https://github.com/aashahin/payments-sdk/blob/master/docs/moyasar.md
17. hyperswitch architecture diagram (juspay/hyperswitch) - GitDiagram, https://gitdiagram.com/juspay/hyperswitch
18. @paykernel/gateway-myfatoorah - NPM, https://www.npmjs.com/package/@paykernel/gateway-myfatoorah
19. @paykernel/core | Yarn, https://chore-update--yarnpkg.netlify.app/pt-BR/package/@paykernel/core
20. @ng-pay/core - npm, https://npmjs.com/package/@ng-pay/core
21. Features - The Cargo Book - Rust Documentation, https://doc.rust-lang.org/cargo/reference/features.html
22. Unification, Optional Dependencies, and Workspace Builds - PracHub, https://prachub.com/resources/cargo-feature-interview-questions-unification-optional-dependencies-and-workspace-builds
23. #3491 remove-implicit-feature - Rust RFCs, https://rust.dossie.rs/3491
24. Cargo.toml: how to conditionally enable a dependency feature?, https://stackoverflow.com/questions/77057406/cargo-toml-how-to-conditionally-enable-a-dependency-feature
25. GitHub - aashahin/payments-sdk: Unified, framework-agnostic, https://github.com/aashahin/payments-sdk
26. How do I set a peer dependency optional? - Stack Overflow, https://stackoverflow.com/questions/62047806/how-do-i-set-a-peer-dependency-optional
27. package.json - npm Docs, https://docs.npmjs.com/cli/configuring-npm/package-json/
28. iabduul7/laravel-moyasar - Packagist.org, https://packagist.org/packages/iabduul7/laravel-moyasar
29. @paykernel/gateway-hesabe | npm.io, https://npm.io/package/@paykernel/gateway-hesabe
30. Payment API for developers: orders, tax, refunds - Flint Pay, https://withflintpay.com/solutions/payment-api
31. Expire a Checkout Session | Stripe API Reference, https://docs.stripe.com/api/checkout/sessions/expire
32. Cancel a PaymentIntent | Stripe API Reference, https://docs.stripe.com/api/payment_intents/cancel
33. Stripe Payment Failed? Fix Declines & Retries | RapidDev, https://www.rapidevelopers.com/stripe-guide/how-to-fix-stripe-payment-failed-issue
34. Stripe PaymentIntents: statuses, SCA and SetupIntents, https://payments-playbook.com/guides/stripe-payment-intents
35. Errors | Stripe API Reference, https://docs.stripe.com/api/errors
36. Stripe Refund Object Explained | Acodei, https://www.acodei.com/glossary/stripe-refund
37. Refund and cancel payments - Stripe Documentation, https://docs.stripe.com/refunds
38. Stripe Refunds API — OpenAPI & Docs | APIs.io, https://apis.io/apis/stripe/stripe-refunds-api/
39. Ecommerce revenue and attribution tracking | Plausible docs, https://plausible.io/docs/ecommerce-revenue-tracking
40. Idempotent requests | Stripe API Reference, https://stripe.com/docs/api/idempotent_requests?dlvrit=1368847&lang=python
41. Stripe AI Phone Payments: Idempotency, Webhooks - Evalgent, https://www.evalgent.com/blog/stripe-ai-phone-payments
42. Stripe Webhook Signature Checker | Refgrow, https://refgrow.com/tools/stripe-webhook-signature-checker
43. Manage webhook endpoints - Stripe Documentation, https://docs.stripe.com/events/manage-webhook-endpoints
44. Stripe Webhook Signature Verification (2026): Code & Tests, https://www.hooklistener.com/learn/stripe-webhook-security-guide
45. Stripe Webhooks Specs | Examples| How to integrate, https://webhooks.fyi/webhook-directory/stripe
46. Moyasar Invoices Api - API Evangelist APIs, https://apis.apievangelist.com/store/moyasar-invoices-api/
47. Moyasar - Payrails Docs, https://docs.payrails.com/docs/integrations/payment-providers/moyasar
48. Payment Intents API | Moyasar Online Docs, https://docs.moyasar.com/category/payment-intents-api
49. Fulfill Payment Intent | Moyasar Online Docs, https://docs.moyasar.com/api/payment_intents/02-fulfill-payment-intent
50. Create Invoice | Moyasar Online Docs, https://docs.moyasar.com/api/invoices/01-create-invoice
51. Invoices API | Moyasar Online Docs, https://docs.moyasar.com/category/invoices-api
52. Idempotency | Moyasar Online Docs, https://docs.moyasar.com/api/idempotency
53. Payment Status Reference | Moyasar Online Docs, https://docs.moyasar.com/api/payments/payment-status-reference
54. Moyasar Connection Setup - Flexprice, https://docs.flexprice.io/integrations/moyasar/connection-setup
55. Verify Webhook | Moyasar Online Docs, https://docs.moyasar.com/api/other/webhooks/verify-webhook
56. Errors | Moyasar Online Docs, https://docs.moyasar.com/api/errors
57. Paymob Accept Legacy (v2) Accounts Payment Keys API, https://apis.io/collections/paymob/postman-paymob-payment-keys-api/
58. Paymob Orders Api - API Evangelist APIs, https://apis.apievangelist.com/store/paymob-orders-api/
59. Affordability Widget - Paymob Documentation, https://developers.paymob.com/paymob-docs/developers/checkout-experiences/affordability-widget
60. Paymob, Kashier & Fawry Integration | Egypt Payment Guide, https://kemetova.com.eg/paymob-kashier-fawry-integration-egypt/
61. Paymob Developer Portal: Integration Onboarding Planner, https://wizard.paymob.com/
62. Paymob-AI-Integration-Skill/universal-prompt.md at main ... - GitHub, https://github.com/PaymobAccept/Paymob-AI-Integration-Skill/blob/main/universal-prompt.md
63. Paymob Subscription Module API.postman_collection.json - GitHub, https://github.com/PaymobAccept/API-Postman-Collections/blob/main/Paymob%20Subscription%20Module%20API.postman_collection.json
64. Overview - Paymob Documentation, https://developers.paymob.com/paymob-docs/developers/checkout-experiences/overview
65. Paymob Documentation, https://developers.paymob.com/paymob-docs
66. Refund - Paymob Documentation, https://developers.paymob.com/paymob-docs/developers/manage-payment-apis
67. By Order ID or Reference - Paymob Developer Portal, https://developers.paymob.com/paymob-docs/developers/transaction-inquiry-apis/transaction-inquiry/by-order-id-or-reference
68. Card Token Inquiry - Paymob Developer Portal, https://developers.paymob.com/paymob-docs/developers/transaction-inquiry-apis/card-token-inquiry
69. Capture - Paymob Documentation, https://developers.paymob.com/paymob-docs/payments-and-features/managing-payments/capture
70. Transaction callbacks - Paymob Documentation, https://developers.paymob.com/paymob-docs/developers/transaction-callbacks
71. Moyasar: card can't be saved after a declined attempt #3208 - GitHub, https://github.com/flexprice/flexprice/issues/3208
72. stripity-stripe/README.md at main - GitHub, https://github.com/beam-community/stripity-stripe/blob/main/README.md
