# Module `1`: Core Contracts (`some-payment-core`) (`feat/module-1-core-contracts`)

- **Module Branch**: `feat/module-1-core-contracts`
- **Base Branch**: `main`
- **Field Profile**: `software`
- **User Stories**: `2` Stories (`US-1.1.0`, `US-1.2.0`) | **Total Parallel Units**: `9` Units
- **Merge Order Position to `main`**: 1st — Foundation crate with zero upstream prerequisites; required by Modules 2–5 and sibling Module 9 PRs

## 1. User Story & Implementation Unit Index

| Story ID | Title & Spec Link | $S_{\text{story}}$ Score | Parallel Units ($K$) | Owned Directory / Paths |
| :--- | :--- | :---: | :---: | :--- |
| `US-1.1.0` | [`us-1.1.0-domain-primitives-and-errors.md`](./us-1.1.0-domain-primitives-and-errors.md) | `2.40` | `4` | `crates/some-payment-core/src/{money,provider_id,reference,error}.rs` |
| `US-1.2.0` | [`us-1.2.0-provider-spi-and-idempotency-contracts.md`](./us-1.2.0-provider-spi-and-idempotency-contracts.md) | `2.80` | `5` | `crates/some-payment-core/src/{capabilities,requests,results,provider,idempotency}.rs` |

---

## 2. Copy-Paste Module Execution Prompt

Copy and paste the prompt block below directly into your AI agent session to execute **Module `1`** end-to-end using the `some-divide-and-conquer` multi-agent workflow:

```text
Execute Module 1 (Core Contracts (`some-payment-core`)) on https://github.com/some-of-cs/some-payment (base: main) using the some-divide-and-conquer plugin in full parallel mode.

### 1. Architecture, Scope & Feature/Deliverable Contract
- Module branch: `feat/module-1-core-contracts` (base: `main`).
- Feature flags / Deliverable boundaries: Initializes the root Cargo workspace and HTTP-free `crates/some-payment-core` crate (zero `reqwest`, zero concrete provider imports). `cargo test -p some-payment-core` MUST remain 100% green.
- Specifications to implement 100% (every Acceptance Criterion `AC-*` and Test Case `TC-*`):
  1. `docs/user-stories/module-1-core-contracts/us-1.1.0-domain-primitives-and-errors.md`
  2. `docs/user-stories/module-1-core-contracts/us-1.2.0-provider-spi-and-idempotency-contracts.md`

### 2. Agent Roster, Roles & Skills (29 Total Agents)
- **Shared Infrastructure Agents (3)**:
  - `git-workflow-master` (1): Exclusive owner of `.worktrees/`, branches, PR creation, and ordered cascading merges.
  - `github-projects-kanban` (1): Syncs `.agents/checkpoints/module-1-core-contracts/checkpoint.json` and PR conversation reports.
  - `quota-sentinel` (1): Monitors rate limits and flushes state on HTTP 429 / 1% remaining quota.
- **Tier 1 — Module Level (2 Agents)**:
  - `module-leader` (1) on `feat/module-1-core-contracts` (3-minute watchdog `*/3 * * * *`). NEVER spawns Unit Implementers directly and NEVER edits code directly.
  - Independent `code-review-council` + `code-reviewer-council-president` (1) for the Module PR.
- **Tier 2 — Story Level (6 Agents = 2 Story Leaders + 2 Integration Verifiers + 2 Story Review Councils)**:
  - 2 `story-leader` agents (`US-1.1.0`, `US-1.2.0`, 1-minute watchdog `* * * * *`). NEVER edits code directly.
  - 2 integration/API tester agents (`testing-api-tester`) for story-level verification.
  - 2 independent `code-review-council` subagents (1 per Story PR).
- **Tier 3 — Unit Level (18 Agents = 9 Unit Implementers + 9 Independent Unit Review Councils)**:
  - **US-1.1.0 (4 Units + 4 Reviewers)**:
    - Unit 1 (`feat/us-1.1.0/unit-1-money` -> `crates/some-payment-core/src/money.rs`): Integer minor-unit `Money`, `CurrencyCode`, 0/2/3-decimal table, checked arithmetic | Agent: `backend-architect`
    - Unit 2 (`feat/us-1.1.0/unit-2-provider-id` -> `crates/some-payment-core/src/provider_id.rs`): Normalized lowercase open `ProviderId` newtype with validation & serde | Agent: `backend-architect`
    - Unit 3 (`feat/us-1.1.0/unit-3-reference` -> `crates/some-payment-core/src/reference.rs`): `ProviderReference` correlating `payment_id`, `transaction_id`, `order_id`, `intent_id`, `checkout_session_id` | Agent: `backend-architect`
    - Unit 4 (`feat/us-1.1.0/unit-4-error` -> `crates/some-payment-core/src/error.rs`): Redacted `PaymentError` taxonomy (`thiserror`), `is_retryable()`, `http_status()`, `decline_code()` | Agent: `security-engineer`
  - **US-1.2.0 (5 Units + 5 Reviewers)**:
    - Unit 1 (`feat/us-1.2.0/unit-1-capabilities` -> `crates/some-payment-core/src/capabilities.rs`): `PaymentCapabilities` bitflags covering all 14 capability flags | Agent: `backend-architect`
    - Unit 2 (`feat/us-1.2.0/unit-2-requests` -> `crates/some-payment-core/src/requests.rs`): Canonical request DTOs (`CreatePaymentRequest`, `CheckoutSessionRequest`, `CaptureRequest`, `VoidRequest`, `RefundRequest`, `CaptureMode`, `SplitInstruction`) | Agent: `api-platform-engineer`
    - Unit 3 (`feat/us-1.2.0/unit-3-results` -> `crates/some-payment-core/src/results.rs`): `PaymentResult`, `RefundResult`, `PaymentStatus`, `NextAction`, `RawWebhookRequest`, `NormalizedWebhookEvent`, `VerificationChallenge` | Agent: `api-platform-engineer`
    - Unit 4 (`feat/us-1.2.0/unit-4-provider-trait` -> `crates/some-payment-core/src/provider.rs`): `#[async_trait] pub trait PaymentProvider: Send + Sync + Debug` with fail-closed default methods | Agent: `backend-architect`
    - Unit 5 (`feat/us-1.2.0/unit-5-idempotency-trait` -> `crates/some-payment-core/src/idempotency.rs`): `ReserveOutcome` and `#[async_trait] pub trait IdempotencyStore: Send + Sync` | Agent: `backend-architect`
- **On-Demand Remediation Agent**: `patch-implementer` (spawned by any Story Leader or Module Leader whenever a Review Council, CodeScene, or CodeRabbit check flags any issue).
- **Skills Loaded**: `divide-and-conquer-orchestrator`, `preflight-git-gh`, `git-workflow-master`, `github-projects-kanban`, `quota-sentinel`, `security-audit`, `code-review-council`, `code-reviewer-council-president`, `cyclomatic-complexity`, `patch-implementer`.

### 3. Mandatory Anti-Regression Rules (Hard Invariants)
1. **Zero Shared-Index (`lib.rs` / `mod.rs` / `Cargo.toml`) Clobbering Across Stories & Units**: In Wave 1, `module-leader` MUST pre-scaffold all module index files (`lib.rs`, `mod.rs`, `Cargo.toml`) and empty stub files for all `9` units on `feat/module-1-core-contracts` BEFORE branching the story branches. Story and Unit branches MUST NOT delete sibling declarations.
2. **Never Self-Review or Merge Before Independent Subagent Approval**: Every Unit PR, Story PR, Patch PR, and Module PR MUST receive a structured Implementation Report comment AND an independent `code-review-council` subagent review (`event="COMMENT"` when using a single GitHub account to avoid self-approval API errors). Loop until `Gate G-Review: Approved` with 0 issues.
3. **Plain UTF-8 Commits Only**: When committing files via GitHub API / MCP tools (`GITHUB_COMMIT_MULTIPLE_FILES`), ALWAYS pass plain-text UTF-8 strings with `encoding: "utf-8"` (never pre-encode Base64).
4. **Static Lookup Tables for Multi-Branch Mapping (`CC <= 6`, Nesting `<= 2`)**: Any enum or dispatcher with >5 variants mapping to/from strings MUST use a static `const KNOWN_*: &[(&str, Self)]` lookup table so cyclomatic complexity stays `CC <= 3`.
5. **Security, Bounds & Secret Hygiene (`G2` & `G-Law`)**: Enforce domain bounds during deserialization, require non-empty suffixes on prefixed IDs, redact all secrets/tokens/PII (`[REDACTED]`) in `Debug`/logs, zeroize intermediate credential buffers, and use checked integer math for monetary values (zero `f32`/`f64` floats).
6. **Complete Documentation, Runnable Doc-Tests & Compile-Fail Guards (`G-Docs` & `G1`)**: Every public symbol/interface MUST include doc comments, runnable doc-tests, and negative/compile-time guards where applicable.
7. **Memory-Safe Verification Execution**: Run `CARGO_BUILD_JOBS=1 cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `cargo test --doc` sequentially so linking never OOMs.

### 4. Step-by-Step Execution & Strict Merge-Order Constraint into `main`
- **Step 1 (Gate 0 & Wave 1 Scaffolding)**: Run `preflight-git-gh`, start `quota-sentinel`, create `feat/module-1-core-contracts` from `main`, pre-scaffold index files + `9` empty unit stub files, and spawn all `2` `story-leader` subagents in parallel.
- **Step 2 (Wave 2 Parallel Unit Loop)**: Each `story-leader` spawns its `K` Unit Implementers in parallel. Each Unit Implementer implements its owned file + unit tests + doc-tests, verifies build/lint/format, opens its Unit PR, posts its Implementation Report comment, and loops with its dedicated Unit Review Council subagent until `APPROVED`.
- **Step 3 (Wave 3 Ordered Cascading Unit Merges & Story Review Loop)**: Merge Unit PRs `1 -> 2 -> ... -> K` one by one per story (rebasing/updating each next Unit PR onto the updated story branch). Open each Story PR into `feat/module-1-core-contracts`, post the Story Implementation Report, run the independent Story Review Council subagent, and spawn `patch-implementer` if any issue is flagged until all `2` Story PRs are `APPROVED` and merged sequentially.
- **Step 4 (Wave 4 Module PR, Whole-Module Audit & Strict `main` Merge Order)**:
  - Open the Module `1` PR (`feat/module-1-core-contracts` -> `main`), post the Module Implementation Report, and run the independent Module `1` Review Council subagent (+ `patch-implementer` loop if needed) until `GATE G-REVIEW APPROVED` with 0 issues.
  - **STRICT MERGE ORDER CONSTRAINT TO `main` (`Module 1 -> Module 2 -> Module 3 -> [some-stripe/moyasar/paymob M9] -> Module 4 -> Module 5`)**: Module `1` MUST be merged into `main` in position **1st — Foundation crate with zero upstream prerequisites; required by Modules 2–5 and sibling Module 9 PRs**. Before merging into `main`, sync/rebase `feat/module-1-core-contracts` onto the latest `main`, re-run the full verification suite (`fmt`, `clippy`, `test`, `doc`), and then merge into `main`.
```
