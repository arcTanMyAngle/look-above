# CLAUDE.md — Look Above

Native Rust flight tracker: CPU-parallel data pipeline, thin wgpu GPU layer, dual
global/regional view modes, free authorized data sources only.

## Session start — read in this order, nothing more

1. Read only the **Now** section at the top of [plans/CURRENT_STATUS.md](plans/CURRENT_STATUS.md), stopping at the next `##` heading. Do not load its session history.
2. Read only the active delivery slice and top-level constraints in the milestone plan named there. A slice is one checklist item, or adjacent low-risk items that share files and one acceptance check. Completed-item annotations are history.
3. Read only the exact doc sections that slice cites. Do **not** read all of `docs/` or any large source file up front — see [docs/05_TOKEN_MANAGEMENT.md](docs/05_TOKEN_MANAGEMENT.md).

## Session end — always

- Replace the `plans/CURRENT_STATUS.md` Now section (≤ 10 bullets: done, next, blockers). Add at most one short session-log line; do not copy implementation narratives into it.
- Append to `plans/DECISION_LOG.md` only for a non-trivial choice (dated, with rationale). Test counts and file inventories are not decisions.
- Stop at milestone gates ([docs/07_MILESTONE_PLAN.md](docs/07_MILESTONE_PLAN.md)); don't start the next milestone unprompted.

## Hard rules (non-negotiable)

- **Data sources:** only those listed in [.claude/skills/authorized-aviation-sources/SKILL.md](.claude/skills/authorized-aviation-sources/SKILL.md).
  Never scrape FlightRadar24, FlightAware, or ADS-B Exchange web UIs. Never exceed documented rate limits.
- **Privacy:** follow [docs/04_PRIVACY_AND_SAFETY_RULES.md](docs/04_PRIVACY_AND_SAFETY_RULES.md).
  Blocked/anonymized aircraft (LADD, PIA) are never tracked, correlated, or displayed with identity.
- **Secrets:** API credentials live in exactly three places, in precedence order: `LOOK_ABOVE_*` env vars, gitignored `config.toml`, or the gitignored `credentials.json` OpenSky issues (read as-downloaded). Nowhere else — never in code, logs, fixtures, or commits. Credential-bearing types are `core::secret::SecretString` ([docs/04](docs/04_PRIVACY_AND_SAFETY_RULES.md) rule 7.1).
- **Never paste raw API responses into context.** Record trimmed fixtures to `tests/fixtures/` and reference them ([docs/06_TOOL_USAGE_RULES.md](docs/06_TOOL_USAGE_RULES.md)).

## Token Management & Workflow Efficiency

- **Bounded Reads:** For files over 400 lines, locate symbols/headings first and read only the relevant range. Review a subagent with its diff plus affected functions; never reread every changed file in full.
- **Bounded Output:** Cap command output and ask for summaries/counts. Never dump full recursive listings, build logs, status history, diffs, or generated data into context.
- **Batch File Edits:** Never issue multiple consecutive edit commands for the same file. Plan all necessary changes for a single file and apply them in one comprehensive edit block.
- **No "Compile-and-Fix" Loops:** Before running `cargo check`, `cargo clippy`, or `cargo test`, you must silently review your drafted code for ownership/borrowing errors (e.g., moving out of shared references) and common Clippy lints (e.g., suboptimal duration units). Write the code correctly the first time to avoid burning tokens on repetitive terminal feedback loops.
- **Dependency Discovery:** Do not use `cargo tree` to explore dependencies; it is too verbose and fails on un-targeted workspaces. To check existing dependencies, read the `Cargo.toml` files directly. To investigate a new crate, use `cargo add --dry-run <crate>` and read the output.
- **Strict Context Adherence:** Do not guess or hunt for missing milestones (e.g., blindly grepping the repo for a step number that isn't in the plan). If a requested task, milestone, or document is missing or ambiguous, stop immediately and ask for clarification.
- **Delegation Budget:** Default to no subagent. Use one only when its cold-start reads replace more main-session context than the handoff costs. Never ask a subagent to re-audit completed work, and never allow nested subagents. A second lane requires genuinely independent files and acceptance checks.
- **Delivery Slices:** Optimize for one usable, end-to-end outcome per session. Adjacent low-risk checklist items may ship together when they touch the same path and share verification; do not split merely to satisfy session bookkeeping.

## Rust conventions

- Stable toolchain, edition 2024. Cargo workspace: `crates/core` (types, geo-math), `crates/ingest`
  (sources, pollers), `crates/store` (SQLite), `crates/render` (wgpu), `crates/app` (binary).
- Concurrency: `tokio` for network I/O only; `rayon` for CPU-parallel compute (interpolation,
  projection batches, spatial indexing); `crossbeam-channel` between pipeline stages. Never
  block the render loop on I/O.
- Errors: `thiserror` in libraries, `anyhow` only in the `app` binary. No `unwrap()` outside tests.
- Logging: `tracing`. Serialization: `serde`. DB: `rusqlite` (bundled feature).
- Verification is proportional to risk; follow [docs/06_TOOL_USAGE_RULES.md](docs/06_TOOL_USAGE_RULES.md). Cross-crate, privacy, network, migration, concurrency, and renderer changes require the full workspace sequence. Documentation-only and isolated crate changes use the smaller checks defined there.
- CI still runs `cargo fmt --check`, workspace Clippy with all targets, and workspace tests on Windows and Linux.

## Agents & skills

- Agents in [.claude/agents/](.claude/agents/): use `data-source-agent` for API clients,
  `renderer-agent` for wgpu/shaders, `geo-math-agent` for projections/interpolation,
  `storage-agent` for schema/migrations, `testing-agent` for tests/fixtures,
  `architecture-agent` for ADRs/crate layout, `ux-agent` for interaction/visual QA.
  Roster and when-to-use details: [AGENTS.md](AGENTS.md).
- Skills in [.claude/skills/](.claude/skills/): consult **authorized-aviation-sources** before
  touching any API client, **high-fidelity-flight-visualization** before rendering work, and
  **token-managed-implementation** only for recovery, explicit budget audits, or when a session is ballooning; the normal startup loop is already above.

## Verification

A change isn't done until exercised at its risk tier. Run the app only when pixels or interaction
should change; use one focused manual pass after automated checks. If navigation automation is
unreliable, record the exact gap after one attempt instead of spending the session fighting it.
Full visual-checklist passes belong at visible feature completion and milestone gates.
Acceptance criteria per milestone: [docs/11_ACCEPTANCE_CRITERIA.md](docs/11_ACCEPTANCE_CRITERIA.md).
