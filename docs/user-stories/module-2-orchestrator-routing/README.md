# Module `2`: Orchestrator, Registry, Hooks & Routing (`feat/module-2-orchestrator-routing`)

- **Module Branch**: `feat/module-2-orchestrator-routing`
- **Base Branch**: `main`
- **Field Profile**: `software`
- **User Stories**: `2` Stories (`US-2.1.0`, `US-2.2.0`) | **Total Parallel Units**: `8` Units
- **Merge Order Position to `main`**: 2nd — Strictly AFTER `module-1-core-contracts` merges to `main`, and BEFORE `module-3-idempotency-webhooks`

## 1. User Story & Implementation Unit Index

| Story ID | Title & Spec Link | $S_{\text{story}}$ Score | Parallel Units ($K$) | Owned Directory / Paths |
| :--- | :--- | :---: | :---: | :--- |
| `US-2.1.0` | [`us-2.1.0-registry-hooks-and-orchestrator-pipeline.md`](./us-2.1.0-registry-hooks-and-orchestrator-pipeline.md) | `2.55` | `4` | `src/{registry,hooks,orchestrator,builder}.rs` |
| `US-2.2.0` | [`us-2.2.0-routing-policies-and-composable-pipelines.md`](./us-2.2.0-routing-policies-and-composable-pipelines.md) | `2.20` | `4` | `src/routing/{policy,country_router,fallback,pipeline}.rs` |

---

## 2. Copy-Paste Module Execution Prompt

Copy and paste the prompt block below directly into your AI agent session to execute **Module `2`** end-to-end using the `some-divide-and-conquer` multi-agent workflow:

```text
Execute Module 2 (Orchestrator, Registry, Hooks & Routing) on https://github.com/some-of-cs/some-payment (base: main) using the some-divide-and-conquer plugin in full parallel mode.

### 1. Architecture, Scope & Feature/Deliverable Contract
- Module branch: `feat/module-2-orchestrator-routing` (base: `main`).
- Feature flags / Deliverable boundaries: Implements root `some-payment` crate with zero concrete provider dependencies (depends strictly on `some-payment-core`), exposing `PaymentOrchestrator` (`PaymentHub`), `ProviderRegistry`, `LifecycleHooks`, `country-router`, and `composable-pipelines` features.
- Specifications to implement 100% (every Acceptance Criterion `AC-*` and Test Case `TC-*`):
  1. `docs/user-stories/module-2-orchestrator-routing/us-2.1.0-registry-hooks-and-orchestrator-pipeline.md`
  2. `docs/user-stories/module-2-orchestrator-routing/us-2.2.0-routing-policies-and-composable-pipelines.md`

### 2. Agent Roster, Roles & Skills (27 Total Agents)
- **Shared Infrastructure Agents (3)**:
  - `git-workflow-master` (1): Exclusive owner of `.worktrees/`, branches, PR creation, and ordered cascading merges.
  - `github-projects-kanban` (1): Syncs `.agents/checkpoints/module-2-orchestrator-routing/checkpoint.json` and PR conversation reports.
  - `quota-sentinel` (1): Monitors rate limits and flushes state on HTTP 429 / 1% remaining quota.
- **Tier 1 — Module Level (2 Agents)**:
  - `module-leader` (1) on `feat/module-2-orchestrator-routing` (3-minute watchdog `*/3 * * * *`). NEVER spawns Unit Implementers directly and NEVER edits code directly.
  - Independent `code-review-council` + `code-reviewer-council-president` (1) for the Module PR.
- **Tier 2 — Story Level (6 Agents = 2 Story Leaders + 2 Integration Verifiers + 2 Story Review Councils)**:
  - 2 `story-leader` agents (`US-2.1.0`, `US-2.2.0`, 1-minute watchdog `* * * * *`). NEVER edits code directly.
  - 2 integration/API tester agents (`testing-api-tester`) for story-level verification.
  - 2 independent `code-review-council` subagents (1 per Story PR).
- **Tier 3 — Unit Level (16 Agents = 8 Unit Implementers + 8 Independent Unit Review Councils)**:
  - **US-2.1.0 (4 Units + 4 Reviewers)**:
    - Unit 1 (`feat/us-2.1.0/unit-1-registry` -> `src/registry.rs`): Thread-safe `ProviderRegistry` (`ProviderId -> Arc<dyn PaymentProvider>`) with capability queries & default provider | Agent: `backend-architect`
    - Unit 2 (`feat/us-2.1.0/unit-2-hooks` -> `src/hooks.rs`): Composable `before`/`after`/`on_error` `LifecycleHook` trait, `HookDecision::Abort`, non-reversible `after`-hook isolation | Agent: `backend-architect`
    - Unit 3 (`feat/us-2.1.0/unit-3-orchestrator` -> `src/orchestrator.rs`): `PaymentOrchestrator` five-step execution pipeline across all 6 core operations | Agent: `backend-architect`
    - Unit 4 (`feat/us-2.1.0/unit-4-builder` -> `src/builder.rs`): `PaymentOrchestratorBuilder` with startup validation & duplicate-provider rejection | Agent: `api-platform-engineer`
  - **US-2.2.0 (4 Units + 4 Reviewers)**:
    - Unit 1 (`feat/us-2.2.0/unit-1-policy` -> `src/routing/policy.rs`): `RoutingPolicy` trait, `RoutingContext`, `DefaultOnlyPolicy` | Agent: `backend-architect`
    - Unit 2 (`feat/us-2.2.0/unit-2-country-router` -> `src/routing/country_router.rs`): `RegionalCurrencyRouter` & `CountryRoutingPolicy` builder using static lookup tables | Agent: `backend-architect`
    - Unit 3 (`feat/us-2.2.0/unit-3-fallback` -> `src/routing/fallback.rs`): `FallbackPolicy` with safe pre-charge failover predicate & acyclic hop guard | Agent: `backend-architect`
    - Unit 4 (`feat/us-2.2.0/unit-4-pipeline` -> `src/routing/pipeline.rs`): `ComposablePipeline` builder pinning distinct providers per capability domain | Agent: `api-platform-engineer`
- **On-Demand Remediation Agent**: `patch-implementer` (spawned by any Story Leader or Module Leader whenever a Review Council, CodeScene, or CodeRabbit check flags any issue).
- **Skills Loaded**: `divide-and-conquer-orchestrator`, `preflight-git-gh`, `git-workflow-master`, `github-projects-kanban`, `quota-sentinel`, `security-audit`, `code-review-council`, `code-reviewer-council-president`, `cyclomatic-complexity`, `patch-implementer`.

### 3. Mandatory Anti-Regression Rules (Hard Invariants)
1. **Zero Shared-Index (`lib.rs` / `mod.rs` / `Cargo.toml`) Clobbering Across Stories & Units**: In Wave 1, `module-leader` MUST pre-scaffold all module index files (`lib.rs`, `mod.rs`, `Cargo.toml`) and empty stub files for all `8` units on `feat/module-2-orchestrator-routing` BEFORE branching the story branches. Story and Unit branches MUST NOT delete sibling declarations.
2. **Never Self-Review or Merge Before Independent Subagent Approval**: Every Unit PR, Story PR, Patch PR, and Module PR MUST receive a structured Implementation Report comment AND an independent `code-review-council` subagent review (`event="COMMENT"` when using a single GitHub account to avoid self-approval API errors). Loop until `Gate G-Review: Approved` with 0 issues.
3. **Plain UTF-8 Commits Only**: When committing files via GitHub API / MCP tools (`GITHUB_COMMIT_MULTIPLE_FILES`), ALWAYS pass plain-text UTF-8 strings with `encoding: "utf-8"` (never pre-encode Base64).
4. **Static Lookup Tables for Multi-Branch Mapping (`CC <= 6`, Nesting `<= 2`)**: Any enum or dispatcher with >5 variants mapping to/from strings MUST use a static `const KNOWN_*: &[(&str, Self)]` lookup table so cyclomatic complexity stays `CC <= 3`.
5. **Security, Bounds & Secret Hygiene (`G2` & `G-Law`)**: Enforce domain bounds during deserialization, require non-empty suffixes on prefixed IDs, redact all secrets/tokens/PII (`[REDACTED]`) in `Debug`/logs, zeroize intermediate credential buffers, and use checked integer math for monetary values (zero `f32`/`f64` floats).
6. **Complete Documentation, Runnable Doc-Tests & Compile-Fail Guards (`G-Docs` & `G1`)**: Every public symbol/interface MUST include doc comments, runnable doc-tests, and negative/compile-time guards where applicable.
7. **Memory-Safe Verification Execution**: Run `CARGO_BUILD_JOBS=1 cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `cargo test --doc` sequentially so linking never OOMs.

### 4. Step-by-Step Execution & Strict Merge-Order Constraint into `main`
- **Step 1 (Gate 0 & Wave 1 Scaffolding)**: Run `preflight-git-gh`, start `quota-sentinel`, create `feat/module-2-orchestrator-routing` from `main`, pre-scaffold index files + `8` empty unit stub files, and spawn all `2` `story-leader` subagents in parallel.
- **Step 2 (Wave 2 Parallel Unit Loop)**: Each `story-leader` spawns its `K` Unit Implementers in parallel. Each Unit Implementer implements its owned file + unit tests + doc-tests, verifies build/lint/format, opens its Unit PR, posts its Implementation Report comment, and loops with its dedicated Unit Review Council subagent until `APPROVED`.
- **Step 3 (Wave 3 Ordered Cascading Unit Merges & Story Review Loop)**: Merge Unit PRs `1 -> 2 -> ... -> K` one by one per story (rebasing/updating each next Unit PR onto the updated story branch). Open each Story PR into `feat/module-2-orchestrator-routing`, post the Story Implementation Report, run the independent Story Review Council subagent, and spawn `patch-implementer` if any issue is flagged until all `2` Story PRs are `APPROVED` and merged sequentially.
- **Step 4 (Wave 4 Module PR, Whole-Module Audit & Strict `main` Merge Order)**:
  - Open the Module `2` PR (`feat/module-2-orchestrator-routing` -> `main`), post the Module Implementation Report, and run the independent Module `2` Review Council subagent (+ `patch-implementer` loop if needed) until `GATE G-REVIEW APPROVED` with 0 issues.
  - **STRICT MERGE ORDER CONSTRAINT TO `main` (`Module 1 -> Module 2 -> Module 3 -> [some-stripe/moyasar/paymob M9] -> Module 4 -> Module 5`)**: Module `2` MUST be merged into `main` in position **2nd — Strictly AFTER `module-1-core-contracts` merges to `main`, and BEFORE `module-3-idempotency-webhooks`**. Before merging into `main`, sync/rebase `feat/module-2-orchestrator-routing` onto the latest `main`, re-run the full verification suite (`fmt`, `clippy`, `test`, `doc`), and then merge into `main`.
```
