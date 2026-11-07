# some-payment

<div align="center">

> **Unified, multi-gateway Rust payment router and composable capability orchestrator.**  
> Run multiple payment gateways simultaneously • Dynamic country & currency routing • Mix-and-match KYC, checkout, and payout capabilities across [`some-stripe`](https://github.com/some-of-cs/some-stripe), [`some-paymob`](https://github.com/some-of-cs/some-paymob), [`some-moyasar`](https://github.com/some-of-cs/some-moyasar), and future providers

[![Crates.io](https://img.shields.io/crates/v/some-payment.svg)](https://crates.io/crates/some-payment)
[![Docs.rs](https://docs.rs/some-payment/badge.svg)](https://docs.rs/some-payment)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust: Edition 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://github.com/some-of-cs/some-payment)
[![Providers: Stripe | Paymob | Moyasar](https://img.shields.io/badge/providers-Stripe%20%7C%20Paymob%20%7C%20Moyasar-8a2be2.svg)](#first-party-provider-ecosystem)
[![Sponsor](https://img.shields.io/badge/Sponsor-%E2%99%A5-ff69b4.svg)](https://github.com/sponsors/some-of-cs)

[Why some-payment?](#why-some-payment) •
[Provider Ecosystem](#first-party-provider-ecosystem) •
[Architecture](#architecture-country-routing--composable-capabilities) •
[Quick Start](#quick-start) •
[Cross-Gateway Composition](#2-cross-gateway-capability-composition-stripe-kyc--paymob-payment--payouts) •
[Unified Webhook Router](#3-unified-multi-gateway-webhook-dispatcher) •
[Extensibility](#adding-future-payment-gateways) •
[Sponsors](#backers--corporate-sponsors)

</div>

---

## Why some-payment?

Global and MENA platforms rarely survive on a single payment processor. A marketplace operating across North America, Europe, Saudi Arabia, and Egypt needs:

1. **Simultaneous Multi-Gateway Execution**: Running [Stripe](https://docs.stripe.com/api), [Paymob](https://developers.paymob.com/), and [Moyasar](https://docs.moyasar.com/api/api-introduction) concurrently in the same binary without conflicting HTTP clients, incompatible currency representations, or tangled error types.
2. **Dynamic Country- & Currency-Based Gateway Switching**: Automatically routing a customer in Saudi Arabia (`SA`) to [`some-moyasar`](https://github.com/some-of-cs/some-moyasar) (for local Mada debit cards, STC Pay, and SADAD bills), a customer in Egypt (`EG`) to [`some-paymob`](https://github.com/some-of-cs/some-paymob) (for Meza, Vodafone Cash, Fawry Kiosk, and ValU BNPL), and a customer in the US/EU (`US`, `DE`, `GB`) to [`some-stripe`](https://github.com/some-of-cs/some-stripe)—with automatic health-aware fallback if a primary gateway experiences downtime.
3. **Cross-Gateway Capability Composition (Mix & Match Features)**: Regional gateways excel at local payment collection and local bank/wallet payouts, but often lack global identity verification (KYC) or global tax engines. `some-payment` decouples **capabilities** (`KycCapability`, `PaymentCapability`, `PayoutCapability`, `SubscriptionCapability`, `WebhookCapability`) from monolithic gateway structs. You can verify a user's passport and biometric selfie using **[`some-stripe`](https://github.com/some-of-cs/some-stripe) Identity KYC**, collect the checkout payment in `EGP` using **[`some-paymob`](https://github.com/some-of-cs/some-paymob)**, and disburse vendor earnings via **[`some-paymob`](https://github.com/some-of-cs/some-paymob) Payouts** (or **[`some-moyasar`](https://github.com/some-of-cs/some-moyasar) SARIE/IPS** in Saudi Arabia) within one unified pipeline.
4. **Future-Proof Plugin Architecture**: Trait-driven provider registration makes adding future gateways (Adyen, Checkout.com, Tap Payments, HyperPay, Tamara, Tabby, PayPal, Wise) a zero-breaking-change extension.

---

## First-Party Provider Ecosystem

`some-payment` is the umbrella orchestrator for our production-grade Rust payment SDKs:

| Provider Crate | Target Regions | Supported Capabilities in `some-payment` | Underlying Gateway Docs |
| :--- | :--- | :--- | :--- |
| **[`some-stripe`](https://github.com/some-of-cs/some-stripe)** | Global (`US`, `EU`, `GB`, `CA`, `AU`, 45+ countries) | **Payments** ([PaymentIntents](https://docs.stripe.com/api/payment_intents/confirm), Checkout), **KYC** (`Stripe Identity`), **Payouts** (`Connect`), **Tax** (`Stripe Tax`), **Billing**, **Webhooks** (`HMAC-SHA256`) | [Stripe API Reference](https://docs.stripe.com/api) |
| **[`some-paymob`](https://github.com/some-of-cs/some-paymob)** | Egypt & MENA (`EG`, `AE`, `OM`, `PK`, `SA`) | **Payments** ([Intention API](https://developers.paymob.com/paymob-docs/intention-apis/create-intention), Cards, Vodafone Cash, Fawry Kiosk, ValU BNPL), **Payouts** ([Paymob Send](https://developers.paymob.com/paymob-docs/beyond-payments/payouts)), **Subscriptions**, **Webhooks** ([HMAC-SHA512](https://developers.paymob.com/paymob-docs/developers/webhook-callbacks-and-hmac/hmac/hmac-transaction-callback)) | [Paymob Developer Portal](https://developers.paymob.com/) |
| **[`some-moyasar`](https://github.com/some-of-cs/some-moyasar)** | Saudi Arabia & GCC (`SA`, `KW`, `BH`, `QA`, `AE`) | **Payments** ([Payments](https://docs.moyasar.com/api/payments/01-create-payment) & [Payment Intents](https://docs.moyasar.com/guides/payment-intents/overview), Mada, STC Pay, SADAD, Apple Pay), **Payouts** ([SARIE & IPS Bulk Payouts](https://docs.moyasar.com/api/payouts/05-list-payouts)), **Settlements**, **Webhooks** ([Constant-Time Secret Verifier](https://docs.moyasar.com/api/other/webhooks/webhook-reference)) | [Moyasar Online Docs](https://docs.moyasar.com/api/api-introduction) |
| **Future Gateways** *(Planned)* | Global & Regional | `some-tap`, `some-hyperpay`, `some-checkout`, `some-adyen`, `some-tabby`, `some-tamara` via the open `GatewayProvider` trait | Extensible via `some-payment-core` |

---

## Architecture: Country Routing & Composable Capabilities

Instead of forcing every gateway into a lowest-common-denominator struct, `some-payment` separates **Routing Policies** from orthogonal **Capability Traits**:

```text
                                 ┌──────────────────────────────────────────┐
                                 │         some_payment::PaymentHub         │
                                 │  (Thread-Safe Multi-Gateway Orchestrator)│
                                 └────────────────────┬─────────────────────┘
                                                      │
          ┌───────────────────────────────────────────┼───────────────────────────────────────────┐
          ▼                                           ▼                                           ▼
┌───────────────────────────────┐     ┌───────────────────────────────┐     ┌───────────────────────────────┐
│   Country & Rule Router       │     │  Cross-Gateway Capability     │     │   Unified Webhook Ingress     │
│                               │     │  Composer (Mix & Match)       │     │                               │
│ • CountryCode::SA -> Moyasar  │     │ • KYC:      Stripe Identity   │     │ • /webhooks/stripe (SHA-256)  │
│ • CountryCode::EG -> Paymob   │     │ • Payment:  Paymob / Moyasar  │     │ • /webhooks/paymob (SHA-512)  │
│ • CountryCode::US -> Stripe   │     │ • Payout:   Paymob / Moyasar  │     │ • /webhooks/moyasar (Digest)  │
│ • Fallback:       -> Stripe   │     │ • Refunds:  Smart Void/Refund │     │ • Emits UnifiedPaymentEvent   │
└───────────────────────────────┘     └───────────────────────────────┘     └───────────────────────────────┘
```

### Orthogonal Capability Traits

| Capability Trait | Responsibility | Implementing Providers |
| :--- | :--- | :--- |
| `PaymentCapability` | Create two-step checkout intents (`create_intent`), charge saved tokens (`charge_token`), capture, void, or refund (`refund_or_void`) | [`some-stripe`](https://github.com/some-of-cs/some-stripe), [`some-paymob`](https://github.com/some-of-cs/some-paymob), [`some-moyasar`](https://github.com/some-of-cs/some-moyasar) |
| `KycCapability` | Create document/biometric verification sessions (`create_kyc_session`), inspect verification status (`get_kyc_report`) | [`some-stripe`](https://github.com/some-of-cs/some-stripe) (`Stripe Identity`), custom KYC providers |
| `PayoutCapability` | Single & bulk outbound disbursements to bank IBANs, cards, or mobile wallets (`disburse`, `disburse_bulk`, `payout_status`) | [`some-paymob`](https://github.com/some-of-cs/some-paymob) ([Paymob Send](https://developers.paymob.com/paymob-docs/beyond-payments/payouts)), [`some-moyasar`](https://github.com/some-of-cs/some-moyasar) ([SARIE/IPS](https://docs.moyasar.com/api/payouts/05-list-payouts)), [`some-stripe`](https://github.com/some-of-cs/some-stripe) (`Connect Payouts`) |
| `WebhookCapability` | Constant-time cryptographic verification and normalization into `UnifiedWebhookEvent` | [`some-stripe`](https://github.com/some-of-cs/some-stripe), [`some-paymob`](https://github.com/some-of-cs/some-paymob), [`some-moyasar`](https://github.com/some-of-cs/some-moyasar) |
| `SplitSettlementCapability` | Multi-party marketplace split payments and platform commission deduction | [`some-moyasar`](https://github.com/some-of-cs/some-moyasar) ([Splits](https://docs.moyasar.com/api/payments/01-create-payment)), [`some-paymob`](https://github.com/some-of-cs/some-paymob) ([Split Features](https://developers.paymob.com/paymob-docs/payments-and-features/core-features/split-features)), [`some-stripe`](https://github.com/some-of-cs/some-stripe) (`Connect`) |

---

## Installation & Feature Flags

Select only the gateways your service needs:

```toml
[dependencies]
some-payment = { version = "0.1.0", features = ["stripe", "paymob", "moyasar", "country-router", "composable-pipelines"] }
tokio = { version = "1", features = ["full"] }
```

| Feature Flag | Default | Description |
| :--- | :---: | :--- |
| `stripe` | **Yes** | Enables [`some-stripe`](https://github.com/some-of-cs/some-stripe) adapter (Checkout, PaymentIntents, Identity KYC, Connect Payouts, Webhooks) |
| `paymob` | **Yes** | Enables [`some-paymob`](https://github.com/some-of-cs/some-paymob) adapter ([Intention API](https://developers.paymob.com/paymob-docs/intention-apis/create-intention), Wallets, Kiosk, ValU, [Paymob Send Payouts](https://developers.paymob.com/paymob-docs/beyond-payments/payouts)) |
| `moyasar` | **Yes** | Enables [`some-moyasar`](https://github.com/some-of-cs/some-moyasar) adapter ([Payments](https://docs.moyasar.com/api/payments/01-create-payment), [Payment Intents](https://docs.moyasar.com/guides/payment-intents/overview), Mada, STC Pay, [SARIE/IPS Payouts](https://docs.moyasar.com/api/payouts/05-list-payouts)) |
| `country-router` | **Yes** | Dynamic ISO-3166-1 alpha-2 country, currency, and payment-method routing with automatic failover |
| `composable-pipelines` | **Yes** | Cross-gateway workflow builder (e.g., Stripe KYC + Paymob/Moyasar Payment + Payouts) |

---

## Quick Start

### 1. Country-Based Dynamic Gateway Switching (Stripe + Paymob + Moyasar Simultaneously)

Register all three gateways once at startup and let `PaymentHub` automatically route checkouts based on the customer's country (`SA` -> Moyasar, `EG` -> Paymob, default -> Stripe):

```rust,no_run
use some_payment::prelude::*;
use some_stripe::StripeClient;
use some_paymob::PaymobClient;
use some_moyasar::{MoyasarClient, SecretKey as MoyasarSecretKey};

#[tokio::main]
async fn main() -> Result<(), PaymentHubError> {
    // 1. Initialize individual gateway SDK clients
    let stripe = StripeClient::new("sk_live_STRIPE_KEY");
    let paymob = PaymobClient::new("egy_sk_live_PAYMOB_KEY")?;
    let moyasar = MoyasarClient::builder(MoyasarSecretKey::new("sk_live_MOYASAR_KEY")?).build()?;

    // 2. Build the unified PaymentHub with Country-Based Routing Rules
    let hub = PaymentHub::builder()
        .register_gateway(GatewayId::Stripe, stripe)
        .register_gateway(GatewayId::Paymob, paymob)
        .register_gateway(GatewayId::Moyasar, moyasar)
        .routing_policy(
            CountryRoutingPolicy::builder()
                // Route Saudi Arabia & GCC to Moyasar (Mada, STC Pay, Apple Pay, SADAD)
                .route_country(CountryCode::SA, GatewayId::Moyasar)
                // Route Egypt to Paymob (Meza, Vodafone Cash, Fawry, ValU)
                .route_country(CountryCode::EG, GatewayId::Paymob)
                // Route UAE to Paymob with automatic fallback to Stripe
                .route_country_with_fallback(CountryCode::AE, GatewayId::Paymob, vec![GatewayId::Stripe])
                // Default all other countries (US, EU, GB, etc.) to Stripe
                .default_gateway(GatewayId::Stripe)
                .build(),
        )
        .build()?;

    // 3. Dynamically routes to `some-moyasar` because buyer_country == CountryCode::SA
    let sa_checkout = hub
        .payments()
        .create_intent(UnifiedIntentRequest {
            buyer_country: CountryCode::SA,
            amount_minor: 15_000, // 150.00 SAR (in halalas)
            currency: CurrencyCode::SAR,
            order_reference: "ORD-SA-9901".into(),
            description: Some("Riyadh Express Delivery".into()),
            callback_url: "https://api.example.com/payments/callback".into(),
            customer_email: Some("customer@example.sa".into()),
            customer_phone: Some("+966501234567".into()),
        })
        .await?;

    println!(
        "Routed via {:?} -> intent_id={}, client_secret={:?}",
        sa_checkout.gateway, sa_checkout.intent_id, sa_checkout.client_secret
    );
    Ok(())
}
```

---

### 2. Cross-Gateway Capability Composition (Stripe KYC + Paymob Payment + Payouts)

Use `ComposablePipeline` to combine features from different gateways in a single strongly typed workflow—for example, enforcing **Stripe Identity KYC** before accepting a localized **Paymob** payment and disbursing vendor earnings via **Paymob Payouts** (or **Moyasar SARIE Payouts**):

```rust,no_run
use some_payment::prelude::*;

async fn run_cross_gateway_marketplace_flow(hub: &PaymentHub) -> Result<(), PaymentHubError> {
    // Compose capabilities from distinct gateways explicitly:
    let pipeline = hub
        .compose_pipeline()
        .kyc_provider(GatewayId::Stripe)       // Use Stripe Identity for global passport/selfie KYC
        .payment_provider(GatewayId::Paymob)   // Use Paymob Intention API for EGP checkout
        .payout_provider(GatewayId::Paymob)    // Use Paymob Send for instant Vodafone Cash disbursement
        .build()?;

    // Step 1: Verify Seller/Buyer Identity via Stripe KYC (`some-stripe`)
    let kyc_session = pipeline
        .kyc()
        .create_verification_session(KycSessionRequest {
            user_id: "usr_egypt_992".into(),
            document_types: vec![KycDocumentType::Passport, KycDocumentType::IdCard],
            require_live_selfie: true,
            return_url: "https://app.example.com/kyc/complete".into(),
        })
        .await?;

    // Step 2: Gate Paymob Checkout on KYC Verification Status
    if pipeline.kyc().is_verified(&kyc_session.session_id).await? {
        let payment_intent = pipeline
            .payment()
            .create_intent(UnifiedIntentRequest {
                buyer_country: CountryCode::EG,
                amount_minor: 500_000, // 5,000.00 EGP
                currency: CurrencyCode::EGP,
                order_reference: "ORD-EG-KYC-501".into(),
                description: Some("Verified Marketplace Escrow Order".into()),
                callback_url: "https://api.example.com/webhooks/paymob".into(),
                customer_email: Some("buyer@example.eg".into()),
                customer_phone: Some("+201012345678".into()),
            })
            .await?;

        println!("Paymob Checkout Ready (KYC Verified by Stripe): {}", payment_intent.checkout_url);
    }

    // Step 3: Disburse Seller Payout via Paymob Send (or switch dynamically to Moyasar SARIE for SA sellers)
    let payout = pipeline
        .payout()
        .disburse(UnifiedPayoutRequest {
            recipient_country: CountryCode::EG,
            amount_minor: 425_000, // 4,250.00 EGP (after commission)
            currency: CurrencyCode::EGP,
            reference_id: "PAYOUT-SELLER-501".into(),
            destination: UnifiedPayoutDestination::MobileWallet {
                provider: WalletProvider::VodafoneCash,
                mobile_number: "01012345678".into(),
            },
            purpose: PayoutPurposeCategory::MerchantSettlement,
        })
        .await?;

    println!("Disbursed via {:?}: payout_id={}, status={:?}", payout.gateway, payout.payout_id, payout.status);
    Ok(())
}
```

---

### 3. Unified Multi-Gateway Webhook Dispatcher

Normalize incoming webhooks from [Stripe](https://docs.stripe.com/api/events/types) (`Stripe-Signature` HMAC-SHA256), [Paymob](https://developers.paymob.com/paymob-docs/developers/webhook-callbacks-and-hmac/hmac/hmac-transaction-callback) (`hmac` query/body HMAC-SHA512), and [Moyasar](https://docs.moyasar.com/api/other/webhooks/webhook-reference) (`secret_token` constant-time digest) into a single `UnifiedWebhookEvent`:

```rust,no_run
use some_payment::prelude::*;
use http::HeaderMap;

fn handle_unified_webhook(
    hub: &PaymentHub,
    gateway: GatewayId,
    headers: &HeaderMap,
    query_string: Option<&str>,
    raw_body: &[u8],
) -> Result<(), PaymentHubError> {
    // Verifies signature/secret in constant time using the target gateway's cryptographic verifier
    let event = hub.webhooks().verify_and_normalize(gateway, headers, query_string, raw_body)?;

    match event.kind {
        UnifiedEventKind::PaymentSucceeded {
            transaction_id,
            order_reference,
            amount_minor,
            currency,
        } => {
            println!(
                "[{:?}] Payment succeeded: tx={}, order={:?}, amount={} {}",
                event.gateway, transaction_id, order_reference, amount_minor, currency
            );
        }
        UnifiedEventKind::PaymentFailed { transaction_id, reason } => {
            eprintln!("[{:?}] Payment failed: tx={}, reason={:?}", event.gateway, transaction_id, reason);
        }
        UnifiedEventKind::PayoutUpdated { payout_id, status } => {
            println!("[{:?}] Payout {} transitioned to {:?}", event.gateway, payout_id, status);
        }
        UnifiedEventKind::KycStatusChanged { session_id, verified } => {
            println!("[{:?}] KYC session {} verified={}", event.gateway, session_id, verified);
        }
        _ => {}
    }
    Ok(())
}
```

---

## Adding Future Payment Gateways

`some-payment` is designed with an open microkernel plugin boundary so third-party and upcoming first-party gateways (such as **Tap Payments**, **HyperPay**, **Checkout.com**, **Adyen**, **Tabby**, **Tamara**, or **PayPal**) can be registered without modifying core routing logic:

```rust,no_run
use async_trait::async_trait;
use some_payment::traits::{GatewayMetadata, PaymentCapability, PayoutCapability, WebhookCapability};

pub struct CustomGatewayAdapter;

#[async_trait]
impl PaymentCapability for CustomGatewayAdapter {
    async fn create_intent(&self, req: UnifiedIntentRequest) -> Result<UnifiedIntentResponse, GatewayError> {
        // Custom gateway protocol implementation
        todo!()
    }

    async fn refund_or_void(&self, tx_id: &str, amount: Option<u64>) -> Result<UnifiedReversalResponse, GatewayError> {
        todo!()
    }
}
```

---

## Backers & Corporate Sponsors

`some-payment` (along with [`some-stripe`](https://github.com/some-of-cs/some-stripe), [`some-paymob`](https://github.com/some-of-cs/some-paymob), and [`some-moyasar`](https://github.com/some-of-cs/some-moyasar)) is an independent, community-driven open-source project. If your business relies on this crate to process transactions or verify webhooks, corporate sponsorship directly finances continuous maintenance, zero-day security updates, and upcoming module expansion.

<div align="center">

[![Become a Sponsor](https://img.shields.io/badge/Become%20a%20Sponsor-%E2%99%A5-ea4aaa?style=for-the-badge)](https://github.com/sponsors/some-of-cs)

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
| *Your company logo here*<br>[Become a Visionary Sponsor](https://github.com/sponsors/some-of-cs) | *Your company logo here*<br>[Become a Gold Partner](https://github.com/sponsors/some-of-cs) | *Your company logo here*<br>[Become a Silver Backer](https://github.com/sponsors/some-of-cs) | [Join our Individual Backers](https://github.com/sponsors/some-of-cs) |

</div>

> [!TIP]
> **Need an Invoice or Procurement Documentation?**  
> We can issue formal VAT/sales-tax compliant invoices via Stripe Invoicing for corporate sponsors. Please contact [sponsors@someofcs.tech](mailto:sponsors@someofcs.tech).

---

## Contributing

We welcome community contributions! To ensure code quality and consistency:

1. Ensure all tests and doc-tests pass:
   ```bash
   cargo test
   ```
2. Ensure code compiles without warnings on the Rust 2024 edition:
   ```bash
   cargo clippy --all-targets -- -D warnings
   ```
3. Format all code according to standard `rustfmt`:
   ```bash
   cargo fmt --check
   ```

---

## License

Licensed under the Apache License, Version 2.0 ([LICENSE](LICENSE)).
