# CLAUDE.md — Look Above

Native Rust flight tracker: CPU-parallel data pipeline, thin wgpu GPU layer, dual
global/regional view modes, free authorized data sources only.

## Session start — read in this order, nothing more

1. [plans/CURRENT_STATUS.md](plans/CURRENT_STATUS.md) — where the project is and the single next action.
2. The **active milestone plan** named there (e.g. [plans/M0_REPO_AUDIT_AND_ARCHITECTURE.md](plans/M0_REPO_AUDIT_AND_ARCHITECTURE.md)).
3. Only the docs that milestone's plan explicitly references. Do **not** read all of `docs/` up front — see [docs/05_TOKEN_MANAGEMENT.md](docs/05_TOKEN_MANAGEMENT.md).

## Session end — always

- Update `plans/CURRENT_STATUS.md` (what was done, what's next, any blockers).
- Append to `plans/DECISION_LOG.md` for any non-trivial choice (dated, with rationale).
- Stop at milestone gates ([docs/07_MILESTONE_PLAN.md](docs/07_MILESTONE_PLAN.md)); don't start the next milestone unprompted.

## Hard rules (non-negotiable)

- **Data sources:** only those listed in [.claude/skills/authorized-aviation-sources/SKILL.md](.claude/skills/authorized-aviation-sources/SKILL.md).
  Never scrape FlightRadar24, FlightAware, or ADS-B Exchange web UIs. Never exceed documented rate limits.
- **Privacy:** follow [docs/04_PRIVACY_AND_SAFETY_RULES.md](docs/04_PRIVACY_AND_SAFETY_RULES.md).
  Blocked/anonymized aircraft (LADD, PIA) are never tracked, correlated, or displayed with identity.
- **Secrets:** API credentials live in exactly three places, in precedence order: `LOOK_ABOVE_*` env vars, gitignored `config.toml`, or the gitignored `credentials.json` OpenSky issues (read as-downloaded). Nowhere else — never in code, logs, fixtures, or commits. Credential-bearing types are `core::secret::SecretString` ([docs/04](docs/04_PRIVACY_AND_SAFETY_RULES.md) rule 7.1).
- **Never paste raw API responses into context.** Record trimmed fixtures to `tests/fixtures/` and reference them ([docs/06_TOOL_USAGE_RULES.md](docs/06_TOOL_USAGE_RULES.md)).

## Token Management & Workflow Efficiency

- **Batch File Edits:** Never issue multiple consecutive edit commands for the same file. Plan all necessary changes for a single file and apply them in one comprehensive edit block.
- **No "Compile-and-Fix" Loops:** Before running `cargo check`, `cargo clippy`, or `cargo test`, you must silently review your drafted code for ownership/borrowing errors (e.g., moving out of shared references) and common Clippy lints (e.g., suboptimal duration units). Write the code correctly the first time to avoid burning tokens on repetitive terminal feedback loops.
- **Dependency Discovery:** Do not use `cargo tree` to explore dependencies; it is too verbose and fails on un-targeted workspaces. To check existing dependencies, read the `Cargo.toml` files directly. To investigate a new crate, use `cargo add --dry-run <crate>` and read the output.
- **Strict Context Adherence:** Do not guess or hunt for missing milestones (e.g., blindly grepping the repo for a step number that isn't in the plan). If a requested task, milestone, or document is missing or ambiguous, stop immediately and ask for clarification.

## Rust conventions

- Stable toolchain, edition 2024. Cargo workspace: `crates/core` (types, geo-math), `crates/ingest`
  (sources, pollers), `crates/store` (SQLite), `crates/render` (wgpu), `crates/app` (binary).
- Concurrency: `tokio` for network I/O only; `rayon` for CPU-parallel compute (interpolation,
  projection batches, spatial indexing); `crossbeam-channel` between pipeline stages. Never
  block the render loop on I/O.
- Errors: `thiserror` in libraries, `anyhow` only in the `app` binary. No `unwrap()` outside tests.
- Logging: `tracing`. Serialization: `serde`. DB: `rusqlite` (bundled feature).
- Before claiming done: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.
  CI runs exactly these three, on Windows and Linux ([.github/workflows/ci.yml](.github/workflows/ci.yml)) — if they pass here they pass there.

## Agents & skills

- Agents in [.claude/agents/](.claude/agents/): use `data-source-agent` for API clients,
  `renderer-agent` for wgpu/shaders, `geo-math-agent` for projections/interpolation,
  `storage-agent` for schema/migrations, `testing-agent` for tests/fixtures,
  `architecture-agent` for ADRs/crate layout, `ux-agent` for interaction/visual QA.
  Roster and when-to-use details: [AGENTS.md](AGENTS.md).
- Skills in [.claude/skills/](.claude/skills/): consult **authorized-aviation-sources** before
  touching any API client, **high-fidelity-flight-visualization** before rendering work, and
  **token-managed-implementation** for the per-session workflow.

## Verification

A change isn't done until exercised: run the affected crate's tests, and for renderer work run
the app (`cargo run -p look-above`) and check against [docs/13_VISUAL_QA_CHECKLIST.md](docs/13_VISUAL_QA_CHECKLIST.md).
Acceptance criteria per milestone: [docs/11_ACCEPTANCE_CRITERIA.md](docs/11_ACCEPTANCE_CRITERIA.md).
