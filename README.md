# some-payment

<div align="center">

> **Unified, multi-gateway Rust payment router and composable capability orchestrator.**  
> Shipped core SPI & domain primitives (`some-payment-core`) • Planned multi-gateway orchestrator & routing across [`some-stripe`](https://github.com/some-of-cs/some-stripe), [`some-paymob`](https://github.com/some-of-cs/some-paymob), [`some-moyasar`](https://github.com/some-of-cs/some-moyasar), and future providers

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust: Edition 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://github.com/some-of-cs/some-payment)
[![MSRV: 1.85](https://img.shields.io/badge/MSRV-1.85-blue.svg)](Cargo.toml)
[![Providers: Stripe | Paymob | Moyasar](https://img.shields.io/badge/providers-Stripe%20%7C%20Paymob%20%7C%20Moyasar-8a2be2.svg)](#first-party-provider-ecosystem)
[![Sponsor](https://img.shields.io/badge/Sponsor-%E2%99%A5-ff69b4.svg)](https://github.com/sponsors/maheraladdin)

[Workspace Status](#workspace-status) •
[Core Crate Quick Start](#quick-start-some-payment-core-shipped) •
[Provider Ecosystem](#first-party-provider-ecosystem) •
[Planned Architecture](#planned-architecture-modules-25) •
[Sponsors](#backers--corporate-sponsors) •
[Contributing](#contributing)

</div>

---

## Workspace Status

`some-payment` is developed incrementally across five modules tracked in [`docs/architecture/DECOMPOSITION_ROADMAP.md`](docs/architecture/DECOMPOSITION_ROADMAP.md) (see [`docs/`](docs/) for full specifications):

| Module | Crate / Scope | Status on `main` |
| :--- | :--- | :---: |
| **Module 1** | `crates/some-payment-core` — HTTP-free domain primitives (`Money`, `CurrencyCode`, `ProviderId`, `ProviderReference`), `PaymentCapabilities` bitmask, `PaymentProvider` SPI, `IdempotencyStore` contract, secret-redacted `PaymentError` | **Shipped** |
| **Module 2** | `some-payment` Orchestrator, Registry & Country/Currency Routing Policies | *Planned* |
| **Module 3** | Distributed Idempotency Middleware, Lifecycle Hooks & Reliability Guards | *Planned* |
| **Module 4** | First-Party Gateway Adapters (`some-stripe`, `some-moyasar`, `some-paymob`) | *Planned* |
| **Module 5** | Unified Webhook Dispatcher, Composable Pipelines & End-to-End Integration Suite | *Planned* |

For AI coding agents working in this repository, see [`AGENTS.md`](AGENTS.md) for a compact API cheat sheet and hard security invariants.

---

## Quick Start: `some-payment-core` (Shipped)

Add the workspace crate directly from git or your local workspace path:

```toml
[dependencies]
# External applications:
some-payment-core = { git = "https://github.com/some-of-cs/some-payment", package = "some-payment-core" }
# Or inside a checkout of this workspace:
# some-payment-core = { path = "crates/some-payment-core" }
async-trait = "0.1"
```

### 1. Validated Domain Primitives & Requests

`some-payment-core` enforces integer minor-unit `Money` (never floats), normalized `ProviderId` slugs, path-traversal-safe `ProviderReference` builders, and strict request validation:

```rust
use some_payment_core::{
    CaptureMode, CreatePaymentRequest, Money, PaymentCapabilities, PaymentError,
    ProviderId, ProviderReference,
};

fn build_validated_request() -> Result<CreatePaymentRequest, PaymentError> {
    let money = Money::new(15_000, "SAR")?; // 150.00 SAR in halalas
    let provider = ProviderId::try_new("moyasar")?;

    let req = CreatePaymentRequest::new(money, "ORD-SA-9901")
        .with_provider_id(provider.clone())
        .with_capture_mode(CaptureMode::Manual)
        .with_idempotency_key("550e8400-e29b-41d4-a716-446655440000");

    // Always validate request DTOs before dispatching to a provider.
    req.validate()?;

    let reference = ProviderReference::builder(provider)
        .payment_id("pay_01J9EXAMPLE")
        .order_id("ORD-SA-9901")
        .build()?;
    assert_eq!(reference.order_id(), Some("ORD-SA-9901"));

    let caps = PaymentCapabilities::DIRECT_PAYMENT | PaymentCapabilities::REFUND;
    assert!(caps.contains(PaymentCapabilities::REFUND));
    Ok(req)
}
```

### 2. Implementing the Open `PaymentProvider` SPI

First-party and third-party gateway crates implement `PaymentProvider` against `some-payment-core` without depending on the orchestrator. Optional lifecycle methods (`create_checkout`, `capture`, `void_payment`, `refund`) fail closed by default with `PaymentError::Unsupported`:

```rust
use async_trait::async_trait;
use some_payment_core::{
    CreatePaymentRequest, NormalizedWebhookEvent, PaymentCapabilities, PaymentError,
    PaymentProvider, PaymentResult, ProviderId, ProviderReference, RawWebhookRequest,
};

#[derive(Debug)]
pub struct CustomGateway {
    id: ProviderId,
}

#[async_trait]
impl PaymentProvider for CustomGateway {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capabilities(&self) -> PaymentCapabilities {
        PaymentCapabilities::empty()
    }

    async fn create_payment(
        &self,
        req: &CreatePaymentRequest,
    ) -> Result<PaymentResult, PaymentError> {
        req.validate()?;
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
        Err(PaymentError::unsupported(&self.id, "parse_and_verify_webhook"))
    }
}
```

---

## First-Party Provider Ecosystem

`some-payment` is designed as the umbrella orchestrator for our Rust payment SDKs:

| Provider Crate | Target Regions | Planned Capabilities | Underlying Gateway Docs |
| :--- | :--- | :--- | :--- |
| **[`some-stripe`](https://github.com/some-of-cs/some-stripe)** | Global (`US`, `EU`, `GB`, `CA`, `AU`, 45+ countries) | Payments ([PaymentIntents](https://docs.stripe.com/api/payment_intents/confirm), Checkout), KYC (`Stripe Identity`), Payouts (`Connect`), Webhooks (`HMAC-SHA256`) | [Stripe API Reference](https://docs.stripe.com/api) |
| **[`some-paymob`](https://github.com/some-of-cs/some-paymob)** | Egypt & MENA (`EG`, `AE`, `OM`, `PK`, `SA`) | Payments ([Intention API](https://developers.paymob.com/paymob-docs/intention-apis/create-intention), Cards, Vodafone Cash, Fawry Kiosk, ValU), Payouts ([Paymob Send](https://developers.paymob.com/paymob-docs/beyond-payments/payouts)), Webhooks ([HMAC-SHA512](https://developers.paymob.com/paymob-docs/developers/webhook-callbacks-and-hmac/hmac/hmac-transaction-callback)) | [Paymob Developer Portal](https://developers.paymob.com/) |
| **[`some-moyasar`](https://github.com/some-of-cs/some-moyasar)** | Saudi Arabia & GCC (`SA`, `KW`, `BH`, `QA`, `AE`) | Payments ([Payments](https://docs.moyasar.com/api/payments/01-create-payment) & [Payment Intents](https://docs.moyasar.com/guides/payment-intents/overview), Mada, STC Pay, SADAD, Apple Pay), Payouts ([SARIE & IPS](https://docs.moyasar.com/api/payouts/05-list-payouts)), Webhooks ([Constant-Time Secret Verifier](https://docs.moyasar.com/api/other/webhooks/webhook-reference)) | [Moyasar Online Docs](https://docs.moyasar.com/api/api-introduction) |
| **Third-Party Gateways** | Global & Regional | Any custom provider (`some-tap`, `some-hyperpay`, `some-checkout`, `some-adyen`, etc.) via `some_payment_core::PaymentProvider` | Extensible via `some-payment-core` |

---

## Planned Architecture (Modules 2–5)

> [!NOTE]
> The multi-provider orchestrator crate (`some-payment`), country/currency routing engine, distributed idempotency middleware, and unified webhook dispatcher described below are **Planned (Modules 2–5)** and are not yet published on `main`. Track detailed designs in [`docs/architecture/DECOMPOSITION_ROADMAP.md`](docs/architecture/DECOMPOSITION_ROADMAP.md).

```text
                                 ┌──────────────────────────────────────────┐
                                 │    some_payment (Planned: Modules 2–5)   │
                                 │  (Thread-Safe Multi-Gateway Orchestrator)│
                                 └────────────────────┬─────────────────────┘
                                                      │
          ┌───────────────────────────────────────────┼───────────────────────────────────────────┐
          ▼                                           ▼                                           ▼
┌───────────────────────────────┐     ┌───────────────────────────────┐     ┌───────────────────────────────┐
│   Country & Rule Router       │     │  Idempotency & Capabilities   │     │   Unified Webhook Ingress     │
│   (Planned — Module 2)        │     │  (Planned — Module 3)         │     │   (Planned — Modules 4–5)     │
│ • SA -> some-moyasar          │     │ • Fail-closed capability gate │     │ • /webhooks/stripe (SHA-256)  │
│ • EG -> some-paymob           │     │ • Distributed IdempotencyStore│     │ • /webhooks/paymob (SHA-512)  │
│ • Default -> some-stripe      │     │ • Lock-retention on 5xx/408   │     │ • /webhooks/moyasar (Digest)  │
└───────────────────────────────┘     └───────────────────────────────┘     └───────────────────────────────┘
```

---

## Backers & Corporate Sponsors

`some-payment` (along with [`some-stripe`](https://github.com/some-of-cs/some-stripe), [`some-paymob`](https://github.com/some-of-cs/some-paymob), and [`some-moyasar`](https://github.com/some-of-cs/some-moyasar)) is an independent, community-driven open-source project. If your business relies on this ecosystem to process transactions or verify webhooks, corporate sponsorship directly finances continuous maintenance, security updates, and upcoming module expansion.

<div align="center">

[![Become a Sponsor](https://img.shields.io/badge/Become%20a%20Sponsor-%E2%99%A5-ea4aaa?style=for-the-badge)](https://github.com/sponsors/maheraladdin)

</div>

### Corporate Sponsorship Tiers & Benefits

| Tier | Monthly | Placement & Benefits |
| :--- | :--- | :--- |
| **🏆 Visionary** | $1,000 / mo | Large logo on `README.md`, website, and Docs.rs header • Prioritized GitHub issues • Dedicated quarterly roadmap review |
| **🥇 Gold Partner** | $500 / mo | Prominent logo on `README.md` • Prioritized PR reviews • Direct issue escalation |
| **🥈 Silver Backer** | $150 / mo | Company logo, link, and mention in `README.md` and release changelogs |
| **☕ Individual** | $10 – $50 / mo | Listed as a contributor and backer on GitHub |

### Sponsors & Backers Wall

<div align="center">

| 🏆 Visionary Sponsors | 🥇 Gold Partners | 🥈 Silver Backers | ☕ Individual Backers |
| :---: | :---: | :---: | :---: |
| *Your company logo here*<br>[Become a Visionary Sponsor](https://github.com/sponsors/maheraladdin) | *Your company logo here*<br>[Become a Gold Partner](https://github.com/sponsors/maheraladdin) | *Your company logo here*<br>[Become a Silver Backer](https://github.com/sponsors/maheraladdin) | [Join our Individual Backers](https://github.com/sponsors/maheraladdin) |

</div>

> [!TIP]
> **Need an Invoice or Procurement Documentation?**  
> We can issue formal VAT/sales-tax compliant invoices via Stripe Invoicing for corporate sponsors. Please contact [sponsors@someofcs.tech](mailto:sponsors@someofcs.tech).

---

## Contributing

We welcome community contributions! Before opening a pull request, run the full workspace verification suite:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --doc --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

---

## License

Licensed under the Apache License, Version 2.0 ([LICENSE](LICENSE)).
