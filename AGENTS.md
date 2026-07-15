# AGENTS.md — Look Above

Guidance for any AI coding agent working in this repository. This file mirrors
[CLAUDE.md](CLAUDE.md); if the two ever disagree, CLAUDE.md wins and this file should be fixed.

## Project

Native Rust flight tracker. CPU worker threads (rayon) handle ingestion, interpolation,
geo-math, and spatial indexing; a thin wgpu layer draws pixels. Two view modes: global
overview and detailed regional view, connected by level-of-detail transitions. Data comes
exclusively from free, authorized sources (OpenSky Network + no-key community aggregators +
NOAA weather + open airport datasets).

## Workflow contract

1. Read [plans/CURRENT_STATUS.md](plans/CURRENT_STATUS.md) first, then the active milestone plan it names.
2. Work one checklist item at a time; keep diffs scoped to it.
3. Verify with `cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace`.
4. Before ending: update `plans/CURRENT_STATUS.md`, log decisions in `plans/DECISION_LOG.md`.
5. Stop at milestone gates; a human reviews before the next milestone starts.

## Hard rules

- **Authorized sources only** — the allowlist is [.claude/skills/authorized-aviation-sources/SKILL.md](.claude/skills/authorized-aviation-sources/SKILL.md).
  No scraping of FlightRadar24 / FlightAware / ADS-B Exchange web properties; no rate-limit evasion.
- **Privacy** — [docs/04_PRIVACY_AND_SAFETY_RULES.md](docs/04_PRIVACY_AND_SAFETY_RULES.md) is binding:
  LADD/PIA-blocked aircraft are never identified, correlated, or displayed with identity.
- **Secrets** — credentials only in gitignored `config.toml` or env vars; never in code or commits.
- **Context discipline** — never dump raw API JSON into a session; use trimmed fixtures
  ([docs/05_TOKEN_MANAGEMENT.md](docs/05_TOKEN_MANAGEMENT.md), [docs/06_TOOL_USAGE_RULES.md](docs/06_TOOL_USAGE_RULES.md)).

## Specialized subagent roster (`.claude/agents/`)

| Agent | Use for | Not for |
|---|---|---|
| [architecture-agent](.claude/agents/architecture-agent.md) | ADRs, crate layout, dependency choices, module boundaries | Writing feature code |
| [data-source-agent](.claude/agents/data-source-agent.md) | API clients, pollers, rate budgeting, auth flows, normalization | Rendering, storage schema |
| [renderer-agent](.claude/agents/renderer-agent.md) | wgpu pipelines, WGSL shaders, instancing, LOD, camera | Data ingestion |
| [geo-math-agent](.claude/agents/geo-math-agent.md) | Projections, haversine/bearing, interpolation, dead reckoning, spatial indexing | UI/UX decisions |
| [storage-agent](.claude/agents/storage-agent.md) | SQLite schema, migrations, retention/pruning, query performance | Network code |
| [testing-agent](.claude/agents/testing-agent.md) | Unit/integration tests, HTTP fixtures, benchmarks | Product decisions |
| [ux-agent](.claude/agents/ux-agent.md) | Interaction design, labels, color ramps, accessibility, visual QA | Shader internals |

Delegate to a subagent when a task is squarely in one lane; work directly for small
cross-cutting edits. Give each subagent the specific file paths and the docs it needs —
they start with no context.

## Build & test commands

```sh
cargo build --workspace          # compile everything
cargo test --workspace           # all tests (offline; fixtures only, no live API calls)
cargo clippy --workspace -- -D warnings
cargo fmt --check
cargo run --release -p look-above   # run the app (once crates/app exists)
```
