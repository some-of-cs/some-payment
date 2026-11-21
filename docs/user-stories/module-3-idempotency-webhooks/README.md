# Module `3`: Distributed Idempotency Boundary & Unified Webhook Pipeline (`feat/module-3-idempotency-webhooks`)

- **Module Branch**: `feat/module-3-idempotency-webhooks`
- **Base Branch**: `main`
- **Field Profile**: `software`
- **User Stories**: `2` Stories (`US-3.1.0`, `US-3.2.0`) | **Total Parallel Units**: `8` Units
- **Merge Order Position to `main`**: 3rd — Strictly AFTER `module-2-orchestrator-routing` merges to `main`, and BEFORE `module-4-provider-wiring`

## 1. User Story & Implementation Unit Index

| Story ID | Title & Spec Link | $S_{\text{story}}$ Score | Parallel Units ($K$) | Owned Directory / Paths |
| :--- | :--- | :---: | :---: | :--- |
| `US-3.1.0` | [`us-3.1.0-distributed-idempotency-boundary.md`](./us-3.1.0-distributed-idempotency-boundary.md) | `2.25` | `4` | `src/idempotency/{scope_keys,boundary,memory_store,reconciliation}.rs` |
| `US-3.2.0` | [`us-3.2.0-unified-webhook-pipeline.md`](./us-3.2.0-unified-webhook-pipeline.md) | `2.55` | `4` | `src/webhooks/{raw_request,dispatcher,handshake,dedup_guard}.rs` |

---

## 2. Copy-Paste Module Execution Prompt

Copy and paste the prompt block below directly into your AI agent session to execute **Module `3`** end-to-end using the `some-divide-and-conquer` multi-agent workflow:

```text
Execute Module 3 (Distributed Idempotency Boundary & Unified Webhook Pipeline) on https://github.com/some-of-cs/some-payment (base: main) using the some-divide-and-conquer plugin in full parallel mode.

### 1. Architecture, Scope & Feature/Deliverable Contract
- Module branch: `feat/module-3-idempotency-webhooks` (base: `main`).
- Feature flags / Deliverable boundaries: Adds `src/idempotency/` (mutation lock boundary, `InMemoryIdempotencyStore` for dev/tests, indeterminate-timeout reconciliation) and `src/webhooks/` (raw-byte dispatch, Moyasar verification handshake routing, monotonic deduplication guard) with zero concrete provider dependencies.
- Specifications to implement 100% (every Acceptance Criterion `AC-*` and Test Case `TC-*`):
  1. `docs/user-stories/module-3-idempotency-webhooks/us-3.1.0-distributed-idempotency-boundary.md`
  2. `docs/user-stories/module-3-idempotency-webhooks/us-3.2.0-unified-webhook-pipeline.md`

### 2. Agent Roster, Roles & Skills (27 Total Agents)
- **Shared Infrastructure Agents (3)**:
  - `git-workflow-master` (1): Exclusive owner of `.worktrees/`, branches, PR creation, and ordered cascading merges.
  - `github-projects-kanban` (1): Syncs `.agents/checkpoints/module-3-idempotency-webhooks/checkpoint.json` and PR conversation reports.
  - `quota-sentinel` (1): Monitors rate limits and flushes state on HTTP 429 / 1% remaining quota.
- **Tier 1 — Module Level (2 Agents)**:
  - `module-leader` (1) on `feat/module-3-idempotency-webhooks` (3-minute watchdog `*/3 * * * *`). NEVER spawns Unit Implementers directly and NEVER edits code directly.
  - Independent `code-review-council` + `code-reviewer-council-president` (1) for the Module PR.
- **Tier 2 — Story Level (6 Agents = 2 Story Leaders + 2 Integration Verifiers + 2 Story Review Councils)**:
  - 2 `story-leader` agents (`US-3.1.0`, `US-3.2.0`, 1-minute watchdog `* * * * *`). NEVER edits code directly.
  - 2 integration/API tester agents (`testing-api-tester`) for story-level verification.
  - 2 independent `code-review-council` subagents (1 per Story PR).
- **Tier 3 — Unit Level (16 Agents = 8 Unit Implementers + 8 Independent Unit Review Councils)**:
  - **US-3.1.0 (4 Units + 4 Reviewers)**:
    - Unit 1 (`feat/us-3.1.0/unit-1-scope-keys` -> `src/idempotency/scope_keys.rs`): Namespaced scope-key builder `(provider_id, operation)` & UUID v4 key validator | Agent: `backend-architect`
    - Unit 2 (`feat/us-3.1.0/unit-2-boundary` -> `src/idempotency/boundary.rs`): Generic async `execute_idempotent_mutation` enforcing 4xx-only release & 5xx lock retention | Agent: `backend-architect`
    - Unit 3 (`feat/us-3.1.0/unit-3-memory-store` -> `src/idempotency/memory_store.rs`): Thread-safe `InMemoryIdempotencyStore` with TTL expiry strictly for dev/tests | Agent: `backend-architect`
    - Unit 4 (`feat/us-3.1.0/unit-4-reconciliation` -> `src/idempotency/reconciliation.rs`): Background reconciliation helper checking `get_payment()` to resolve indeterminate locks | Agent: `backend-architect`
  - **US-3.2.0 (4 Units + 4 Reviewers)**:
    - Unit 1 (`feat/us-3.2.0/unit-1-raw-request` -> `src/webhooks/raw_request.rs`): Raw webhook byte-cap validation, case-insensitive header helpers & query parsing | Agent: `security-engineer`
    - Unit 2 (`feat/us-3.2.0/unit-2-dispatcher` -> `src/webhooks/dispatcher.rs`): `WebhookDispatcher` verifying via registry before dedup & enforcing secret-scrub assertion | Agent: `backend-architect`
    - Unit 3 (`feat/us-3.2.0/unit-3-handshake` -> `src/webhooks/handshake.rs`): `VerificationHandshakeHandler` trait & routing for Moyasar `webhook_verification` events | Agent: `api-platform-engineer`
    - Unit 4 (`feat/us-3.2.0/unit-4-dedup-guard` -> `src/webhooks/dedup_guard.rs`): `WebhookDeduplicationStore` trait, `InMemoryDedupStore`, monotonic transition & `Money` match guard | Agent: `backend-architect`
- **On-Demand Remediation Agent**: `patch-implementer` (spawned by any Story Leader or Module Leader whenever a Review Council, CodeScene, or CodeRabbit check flags any issue).
- **Skills Loaded**: `divide-and-conquer-orchestrator`, `preflight-git-gh`, `git-workflow-master`, `github-projects-kanban`, `quota-sentinel`, `security-audit`, `code-review-council`, `code-reviewer-council-president`, `cyclomatic-complexity`, `patch-implementer`.

### 3. Mandatory Anti-Regression Rules (Hard Invariants)
1. **Zero Shared-Index (`lib.rs` / `mod.rs` / `Cargo.toml`) Clobbering Across Stories & Units**: In Wave 1, `module-leader` MUST pre-scaffold all module index files (`lib.rs`, `mod.rs`, `Cargo.toml`) and empty stub files for all `8` units on `feat/module-3-idempotency-webhooks` BEFORE branching the story branches. Story and Unit branches MUST NOT delete sibling declarations.
2. **Never Self-Review or Merge Before Independent Subagent Approval**: Every Unit PR, Story PR, Patch PR, and Module PR MUST receive a structured Implementation Report comment AND an independent `code-review-council` subagent review (`event="COMMENT"` when using a single GitHub account to avoid self-approval API errors). Loop until `Gate G-Review: Approved` with 0 issues.
3. **Plain UTF-8 Commits Only**: When committing files via GitHub API / MCP tools (`GITHUB_COMMIT_MULTIPLE_FILES`), ALWAYS pass plain-text UTF-8 strings with `encoding: "utf-8"` (never pre-encode Base64).
4. **Static Lookup Tables for Multi-Branch Mapping (`CC <= 6`, Nesting `<= 2`)**: Any enum or dispatcher with >5 variants mapping to/from strings MUST use a static `const KNOWN_*: &[(&str, Self)]` lookup table so cyclomatic complexity stays `CC <= 3`.
5. **Security, Bounds & Secret Hygiene (`G2` & `G-Law`)**: Enforce domain bounds during deserialization, require non-empty suffixes on prefixed IDs, redact all secrets/tokens/PII (`[REDACTED]`) in `Debug`/logs, zeroize intermediate credential buffers, and use checked integer math for monetary values (zero `f32`/`f64` floats).
6. **Complete Documentation, Runnable Doc-Tests & Compile-Fail Guards (`G-Docs` & `G1`)**: Every public symbol/interface MUST include doc comments, runnable doc-tests, and negative/compile-time guards where applicable.
7. **Memory-Safe Verification Execution**: Run `CARGO_BUILD_JOBS=1 cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `cargo test --doc` sequentially so linking never OOMs.

### 4. Step-by-Step Execution & Strict Merge-Order Constraint into `main`
- **Step 1 (Gate 0 & Wave 1 Scaffolding)**: Run `preflight-git-gh`, start `quota-sentinel`, create `feat/module-3-idempotency-webhooks` from `main`, pre-scaffold index files + `8` empty unit stub files, and spawn all `2` `story-leader` subagents in parallel.
- **Step 2 (Wave 2 Parallel Unit Loop)**: Each `story-leader` spawns its `K` Unit Implementers in parallel. Each Unit Implementer implements its owned file + unit tests + doc-tests, verifies build/lint/format, opens its Unit PR, posts its Implementation Report comment, and loops with its dedicated Unit Review Council subagent until `APPROVED`.
- **Step 3 (Wave 3 Ordered Cascading Unit Merges & Story Review Loop)**: Merge Unit PRs `1 -> 2 -> ... -> K` one by one per story (rebasing/updating each next Unit PR onto the updated story branch). Open each Story PR into `feat/module-3-idempotency-webhooks`, post the Story Implementation Report, run the independent Story Review Council subagent, and spawn `patch-implementer` if any issue is flagged until all `2` Story PRs are `APPROVED` and merged sequentially.
- **Step 4 (Wave 4 Module PR, Whole-Module Audit & Strict `main` Merge Order)**:
  - Open the Module `3` PR (`feat/module-3-idempotency-webhooks` -> `main`), post the Module Implementation Report, and run the independent Module `3` Review Council subagent (+ `patch-implementer` loop if needed) until `GATE G-REVIEW APPROVED` with 0 issues.
  - **STRICT MERGE ORDER CONSTRAINT TO `main` (`Module 1 -> Module 2 -> Module 3 -> [some-stripe/moyasar/paymob M9] -> Module 4 -> Module 5`)**: Module `3` MUST be merged into `main` in position **3rd — Strictly AFTER `module-2-orchestrator-routing` merges to `main`, and BEFORE `module-4-provider-wiring`**. Before merging into `main`, sync/rebase `feat/module-3-idempotency-webhooks` onto the latest `main`, re-run the full verification suite (`fmt`, `clippy`, `test`, `doc`), and then merge into `main`.
```
