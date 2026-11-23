# AGENTS.md — `some-payment` Workspace Guide

Token-efficient reference for AI coding agents working in `some-of-cs/some-payment` (Rust Edition 2024, MSRV `1.85`, resolver `"3"`).

## 1. Workspace Status

| Path / Module | Status | Notes |
| :--- | :--- | :--- |
| `crates/some-payment-core` (Module 1) | **Shipped on `main`** | HTTP-free domain primitives, `PaymentCapabilities`, `PaymentProvider` SPI, `IdempotencyStore`, secret-redacted `PaymentError`. |
| Modules 2–5 (`some-payment` orchestrator, middleware, adapters, webhook router) | **Planned / In Progress** | See `docs/architecture/DECOMPOSITION_ROADMAP.md` and `docs/user-stories/`. Do not import `some_payment::*` unless that crate exists in `Cargo.toml`. |

## 2. Fast API Cheat Sheet (`use some_payment_core::*;`)

| Type | Construction / Key Methods | Notes |
| :--- | :--- | :--- |
| `ProviderId` | `try_new(&str)`, `new(static)`, `FromStr`, `TryFrom<&str \| String>`, `as_str()` | Normalized lowercase `[a-z0-9][a-z0-9_-]{0,63}`; `new` panics on invalid slug. |
| `CurrencyCode` | `try_new(&str)`, `FromStr`, `TryFrom<&str>`, `as_str()`, `to_lowercase()`, `scale() -> MinorUnitScale` | Uppercase 3-letter ASCII ISO-4217; `Zero` (incl. `ISK`, `JPY`, `KRW`), `Two`, `Three` (`KWD`, `BHD`, `JOD`, `OMR`, `TND`, `IQD`, `LYD`). |
| `Money` | `new(u64, &str)`, `from_currency(u64, CurrencyCode)`, `amount() -> u64`, `currency()`, `checked_add`, `checked_sub` | Strictly positive minor units (`amount > 0`); no floats. |
| `ProviderReference` | `ProviderReference::builder(ProviderId).payment_id(..).transaction_id(..).order_id(..).intent_id(..).checkout_session_id(..).build()` | `Hash + Eq`; requires >=1 ID; raw len `<= 255` before trim; graphic ASCII, no `..`, `/`, `\`, `?`, `#`, `%`. |
| `PaymentCapabilities` | `bitflags` (`u32`, 14 flags, default `empty()`) | Fail-closed: `DIRECT_PAYMENT`, `EMBEDDED_INTENT`, `HOSTED_CHECKOUT`, `AUTH_CAPTURE`, `PARTIAL_CAPTURE`, `VOID_AUTHORIZED`, `VOID_SETTLED_WINDOW`, `REFUND`, `PARTIAL_REFUND`, `NATIVE_CREATE_IDEMPOTENCY`, `NATIVE_MUTATION_IDEMPOTENCY`, `SAVED_CARDS`, `SPLITS`, `WEBHOOK_VERIFICATION_HANDSHAKE`. |
| Request DTOs | `CreatePaymentRequest::new(money, order_id)`, `CheckoutSessionRequest::new(money, order_id, success_url)`, `CaptureRequest::new(ref)`, `VoidRequest::new(ref)`, `RefundRequest::new(ref)` + fluent `.with_*()` | **Always call `.validate()?`** (especially after `serde::Deserialize`). |
| Result / Webhook DTOs | `PaymentResult`, `RefundResult`, `PaymentStatus::from_slug`, `RefundStatus`, `NextAction`, `RawWebhookRequest::new(method, body, headers, query)`, `VerificationChallenge`, `NormalizedWebhookEvent`, `HttpMethod` (`GET`/`POST`) | `RawWebhookRequest::new` lowercases header keys; `header()` fails closed (`None`) on case-duplicate keys. |
| `IdempotencyStore` | `async fn reserve(&IdempotencyEntry) -> Result<ReserveOutcome, PaymentError>`, `complete`, `release` | `Send + Sync`; `ReserveOutcome::Completed` redacts payload in `Debug`. |
| `PaymentError` / `ErrorKind` | `validation`, `configuration`, `unsupported`; `kind()`, `provider()`, `http_status()`, `decline_code()`, `vendor_code()`, `retry_after_secs()`, `is_indeterminate()`, `is_retryable()`, `is_definitive_client_rejection()` | 12 `ErrorKind` variants; `Display`/`Debug` NEVER print `message`/`reason`; buffers zeroized on drop. |

### `PaymentProvider` SPI (`#[async_trait] pub trait PaymentProvider: Send + Sync + Debug`)

- **Required**: `id(&self)`, `capabilities(&self)`, `create_payment(&self, &CreatePaymentRequest)`, `get_payment(&self, &ProviderReference)`, `parse_and_verify_webhook(&self, &RawWebhookRequest)`.
- **Optional (default fail-closed -> `PaymentError::Unsupported`)**: `create_checkout`, `capture`, `void_payment`, `refund`.

## 3. Hard Invariants & Footguns

1. **Money**: Integer minor units only (`u64 > 0`); zero fails `Money::new`, `from_currency`, `checked_sub`, and `Deserialize`.
2. **Always validate requests**: `Deserialize` does not run cross-field checks — call `req.validate()?` before any provider I/O.
3. **URLs (`F-02`)**: Max `2048` bytes, valid port `> 0`, strict `[A-Za-z0-9-]` host labels, **`https://` required** for all non-loopback hosts (`http://` allowed ONLY for `localhost`, `127.0.0.1`, `[::1]`).
4. **IDs & text (`F-01`, `F-03`, `F-08`)**: `idempotency_key` must be 1..=255 graphic ASCII with no leading/trailing whitespace; `order_id`, `recipient_id` (`<= 255`), `description` (`<= 1000`), `reason` (`<= 500`) reject ASCII control chars (`\r`, `\n`, `\0`); `ProviderReference` rejects `..`, `/`, `\`, `?`, `#`, `%` and raw length `> 255`.
5. **Idempotency lock retention (`F-06`)**: Call `IdempotencyStore::release` **ONLY** when `err.is_definitive_client_rejection()` is `true`. NEVER release on `5xx`, timeouts, `408`, `409`, `425`, `429`, `499`, `ProviderUnavailable`, `CardDeclined`, or `ThreeDsFailed`.
6. **Zero secret leakage (`F-04`)**: Never log raw `PaymentError.message`/`.reason`, `NextAction::EmbeddedClientSecret`, `VerificationChallenge`, `RawWebhookRequest` body/headers/query, `ReserveOutcome::Completed`, or URL query/fragment strings; sensitive fields implement `Zeroize`/`ZeroizeOnDrop`.
7. **Crate lints & complexity**: `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]` (document `# Errors` on all `Result`-returning public fns), keep every function CC `<= 6` and nesting depth `<= 2`.

## 4. Minimal Snippets

### Request + Reference + Validation
```rust
use some_payment_core::{CaptureMode, CreatePaymentRequest, Money, ProviderId, ProviderReference};

let money = Money::new(15_000, "SAR")?;
let req = CreatePaymentRequest::new(money, "ORD-1")
    .with_provider_id(ProviderId::try_new("moyasar")?)
    .with_capture_mode(CaptureMode::Manual)
    .with_idempotency_key("idem-ord-1");
req.validate()?;
let r = ProviderReference::builder(ProviderId::new("moyasar")).payment_id("pay_1").build()?;
```

### Custom `PaymentProvider` Skeleton
```rust
use async_trait::async_trait;
use some_payment_core::*;

#[derive(Debug)]
struct MyProvider(ProviderId);

#[async_trait]
impl PaymentProvider for MyProvider {
    fn id(&self) -> &ProviderId { &self.0 }
    fn capabilities(&self) -> PaymentCapabilities { PaymentCapabilities::DIRECT_PAYMENT }
    async fn create_payment(&self, req: &CreatePaymentRequest) -> Result<PaymentResult, PaymentError> {
        req.validate()?;
        Err(PaymentError::unsupported(&self.0, "create_payment"))
    }
    async fn get_payment(&self, _r: &ProviderReference) -> Result<PaymentResult, PaymentError> {
        Err(PaymentError::unsupported(&self.0, "get_payment"))
    }
    async fn parse_and_verify_webhook(&self, _raw: &RawWebhookRequest) -> Result<NormalizedWebhookEvent, PaymentError> {
        Err(PaymentError::unsupported(&self.0, "parse_and_verify_webhook"))
    }
}
```

## 5. Verification Commands

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```
