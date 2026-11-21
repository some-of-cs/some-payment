# Module `4`: Optional `dep:` Feature-Gated Provider Wiring & Extension Kit (`feat/module-4-provider-wiring`)

- **Module Branch**: `feat/module-4-provider-wiring`
- **Base Branch**: `main`
- **Field Profile**: `software`
- **User Stories**: `2` Stories (`US-4.1.0`, `US-4.2.0`) | **Total Parallel Units**: `7` Units
- **Merge Order Position to `main`**: 4th — Strictly AFTER `module-3-idempotency-webhooks` AND after Module 9 merges in `some-stripe`, `some-moyasar`, and `some-paymob`; BEFORE `module-5-conformance-testing`

## 1. User Story & Implementation Unit Index

| Story ID | Title & Spec Link | $S_{\text{story}}$ Score | Parallel Units ($K$) | Owned Directory / Paths |
| :--- | :--- | :---: | :---: | :--- |
| `US-4.1.0` | [`us-4.1.0-feature-gated-provider-constructors.md`](./us-4.1.0-feature-gated-provider-constructors.md) | `2.10` | `4` | `src/providers/{stripe,moyasar,paymob}.rs, src/prelude.rs` |
| `US-4.2.0` | [`us-4.2.0-third-party-extension-kit-and-examples.md`](./us-4.2.0-third-party-extension-kit-and-examples.md) | `1.85` | `3` | `examples/{custom_provider,actix_webhooks,axum_webhooks}.rs` |

---

## 2. Copy-Paste Module Execution Prompt

Copy and paste the prompt block below directly into your AI agent session to execute **Module `4`** end-to-end using the `some-divide-and-conquer` multi-agent workflow:

```text
Execute Module 4 (Optional `dep:` Feature-Gated Provider Wiring & Extension Kit) on https://github.com/some-of-cs/some-payment (base: main) using the some-divide-and-conquer plugin in full parallel mode.

### 1. Architecture, Scope & Feature/Deliverable Contract
- Module branch: `feat/module-4-provider-wiring` (base: `main`).
- Feature flags / Deliverable boundaries: Wires optional dependencies using Cargo explicit `dep:` syntax + `?/` weak feature forwarding (`stripe = ["dep:some-stripe", "some-stripe?/some-payment-adapter"]`, `moyasar`, `paymob`, `all-providers`) with `default = []`. CI MUST verify each feature in isolation (`cargo check --no-default-features --features stripe`).
- Specifications to implement 100% (every Acceptance Criterion `AC-*` and Test Case `TC-*`):
  1. `docs/user-stories/module-4-provider-wiring/us-4.1.0-feature-gated-provider-constructors.md`
  2. `docs/user-stories/module-4-provider-wiring/us-4.2.0-third-party-extension-kit-and-examples.md`

### 2. Agent Roster, Roles & Skills (25 Total Agents)
- **Shared Infrastructure Agents (3)**:
  - `git-workflow-master` (1): Exclusive owner of `.worktrees/`, branches, PR creation, and ordered cascading merges.
  - `github-projects-kanban` (1): Syncs `.agents/checkpoints/module-4-provider-wiring/checkpoint.json` and PR conversation reports.
  - `quota-sentinel` (1): Monitors rate limits and flushes state on HTTP 429 / 1% remaining quota.
- **Tier 1 — Module Level (2 Agents)**:
  - `module-leader` (1) on `feat/module-4-provider-wiring` (3-minute watchdog `*/3 * * * *`). NEVER spawns Unit Implementers directly and NEVER edits code directly.
  - Independent `code-review-council` + `code-reviewer-council-president` (1) for the Module PR.
- **Tier 2 — Story Level (6 Agents = 2 Story Leaders + 2 Integration Verifiers + 2 Story Review Councils)**:
  - 2 `story-leader` agents (`US-4.1.0`, `US-4.2.0`, 1-minute watchdog `* * * * *`). NEVER edits code directly.
  - 2 integration/API tester agents (`testing-api-tester`) for story-level verification.
  - 2 independent `code-review-council` subagents (1 per Story PR).
- **Tier 3 — Unit Level (14 Agents = 7 Unit Implementers + 7 Independent Unit Review Councils)**:
  - **US-4.1.0 (4 Units + 4 Reviewers)**:
    - Unit 1 (`feat/us-4.1.0/unit-1-stripe-wiring` -> `src/providers/stripe.rs`): `#[cfg(feature = "stripe")]` convenience constructor `stripe_provider` & re-exports | Agent: `backend-architect`
    - Unit 2 (`feat/us-4.1.0/unit-2-moyasar-wiring` -> `src/providers/moyasar.rs`): `#[cfg(feature = "moyasar")]` convenience constructor `moyasar_provider` & re-exports | Agent: `backend-architect`
    - Unit 3 (`feat/us-4.1.0/unit-3-paymob-wiring` -> `src/providers/paymob.rs`): `#[cfg(feature = "paymob")]` convenience constructor `paymob_provider` & re-exports | Agent: `backend-architect`
    - Unit 4 (`feat/us-4.1.0/unit-4-prelude-ci` -> `src/prelude.rs`): Ergonomic `prelude` module + `.github/workflows/feature-matrix.yml` isolated feature checks | Agent: `devops-automator`
  - **US-4.2.0 (3 Units + 3 Reviewers)**:
    - Unit 1 (`feat/us-4.2.0/unit-1-custom-provider` -> `examples/custom_provider.rs`): Runnable third-party `TapMockProvider` registration & KWD routing example | Agent: `api-platform-engineer`
    - Unit 2 (`feat/us-4.2.0/unit-2-actix-webhooks` -> `examples/actix_webhooks.rs`): Actix-web `POST /webhooks/payments/{provider}` using `actix_web::web::Bytes` | Agent: `backend-architect`
    - Unit 3 (`feat/us-4.2.0/unit-3-axum-webhooks` -> `examples/axum_webhooks.rs`): Axum `POST /webhooks/payments/:provider` using `axum::body::Bytes` | Agent: `backend-architect`
- **On-Demand Remediation Agent**: `patch-implementer` (spawned by any Story Leader or Module Leader whenever a Review Council, CodeScene, or CodeRabbit check flags any issue).
- **Skills Loaded**: `divide-and-conquer-orchestrator`, `preflight-git-gh`, `git-workflow-master`, `github-projects-kanban`, `quota-sentinel`, `security-audit`, `code-review-council`, `code-reviewer-council-president`, `cyclomatic-complexity`, `patch-implementer`.

### 3. Mandatory Anti-Regression Rules (Hard Invariants)
1. **Zero Shared-Index (`lib.rs` / `mod.rs` / `Cargo.toml`) Clobbering Across Stories & Units**: In Wave 1, `module-leader` MUST pre-scaffold all module index files (`lib.rs`, `mod.rs`, `Cargo.toml`) and empty stub files for all `7` units on `feat/module-4-provider-wiring` BEFORE branching the story branches. Story and Unit branches MUST NOT delete sibling declarations.
2. **Never Self-Review or Merge Before Independent Subagent Approval**: Every Unit PR, Story PR, Patch PR, and Module PR MUST receive a structured Implementation Report comment AND an independent `code-review-council` subagent review (`event="COMMENT"` when using a single GitHub account to avoid self-approval API errors). Loop until `Gate G-Review: Approved` with 0 issues.
3. **Plain UTF-8 Commits Only**: When committing files via GitHub API / MCP tools (`GITHUB_COMMIT_MULTIPLE_FILES`), ALWAYS pass plain-text UTF-8 strings with `encoding: "utf-8"` (never pre-encode Base64).
4. **Static Lookup Tables for Multi-Branch Mapping (`CC <= 6`, Nesting `<= 2`)**: Any enum or dispatcher with >5 variants mapping to/from strings MUST use a static `const KNOWN_*: &[(&str, Self)]` lookup table so cyclomatic complexity stays `CC <= 3`.
5. **Security, Bounds & Secret Hygiene (`G2` & `G-Law`)**: Enforce domain bounds during deserialization, require non-empty suffixes on prefixed IDs, redact all secrets/tokens/PII (`[REDACTED]`) in `Debug`/logs, zeroize intermediate credential buffers, and use checked integer math for monetary values (zero `f32`/`f64` floats).
6. **Complete Documentation, Runnable Doc-Tests & Compile-Fail Guards (`G-Docs` & `G1`)**: Every public symbol/interface MUST include doc comments, runnable doc-tests, and negative/compile-time guards where applicable.
7. **Memory-Safe Verification Execution**: Run `CARGO_BUILD_JOBS=1 cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `cargo test --doc` sequentially so linking never OOMs.

### 4. Step-by-Step Execution & Strict Merge-Order Constraint into `main`
- **Step 1 (Gate 0 & Wave 1 Scaffolding)**: Run `preflight-git-gh`, start `quota-sentinel`, create `feat/module-4-provider-wiring` from `main`, pre-scaffold index files + `7` empty unit stub files, and spawn all `2` `story-leader` subagents in parallel.
- **Step 2 (Wave 2 Parallel Unit Loop)**: Each `story-leader` spawns its `K` Unit Implementers in parallel. Each Unit Implementer implements its owned file + unit tests + doc-tests, verifies build/lint/format, opens its Unit PR, posts its Implementation Report comment, and loops with its dedicated Unit Review Council subagent until `APPROVED`.
- **Step 3 (Wave 3 Ordered Cascading Unit Merges & Story Review Loop)**: Merge Unit PRs `1 -> 2 -> ... -> K` one by one per story (rebasing/updating each next Unit PR onto the updated story branch). Open each Story PR into `feat/module-4-provider-wiring`, post the Story Implementation Report, run the independent Story Review Council subagent, and spawn `patch-implementer` if any issue is flagged until all `2` Story PRs are `APPROVED` and merged sequentially.
- **Step 4 (Wave 4 Module PR, Whole-Module Audit & Strict `main` Merge Order)**:
  - Open the Module `4` PR (`feat/module-4-provider-wiring` -> `main`), post the Module Implementation Report, and run the independent Module `4` Review Council subagent (+ `patch-implementer` loop if needed) until `GATE G-REVIEW APPROVED` with 0 issues.
  - **STRICT MERGE ORDER CONSTRAINT TO `main` (`Module 1 -> Module 2 -> Module 3 -> [some-stripe/moyasar/paymob M9] -> Module 4 -> Module 5`)**: Module `4` MUST be merged into `main` in position **4th — Strictly AFTER `module-3-idempotency-webhooks` AND after Module 9 merges in `some-stripe`, `some-moyasar`, and `some-paymob`; BEFORE `module-5-conformance-testing`**. Before merging into `main`, sync/rebase `feat/module-4-provider-wiring` onto the latest `main`, re-run the full verification suite (`fmt`, `clippy`, `test`, `doc`), and then merge into `main`.
```
