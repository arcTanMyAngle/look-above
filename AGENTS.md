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

1. Read only the `Now` section of [plans/CURRENT_STATUS.md](plans/CURRENT_STATUS.md), then only the active checklist item and top-level constraints in the milestone plan it names.
2. Work one coherent delivery slice at a time: one checklist item, or adjacent low-risk items sharing files and one acceptance check. Locate symbols before reading files over 400 lines.
3. Verify proportionally to risk using [docs/06_TOOL_USAGE_RULES.md](docs/06_TOOL_USAGE_RULES.md); the full workspace sequence is reserved for high-risk/cross-cutting Rust changes and gates.
4. Before ending: replace the ≤10-bullet `Now` section, add at most one short session-log line, and log only non-trivial decisions in `plans/DECISION_LOG.md`.
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

Default to direct work. Delegate only when a task is squarely in one lane and the cold
subagent avoids substantial main-session reads. Use at most one subagent per delivery slice
unless two lanes have independent files and acceptance checks. Nested subagents and generic
"double-check the work" agents are prohibited. Give the agent exact file paths, bounded doc
sections, and the acceptance check; do not forward the full parent transcript.

## Build & test commands

```sh
cargo build --workspace          # compile everything
cargo test --workspace           # all tests (offline; fixtures only, no live API calls)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo run --release -p look-above   # run the app (once crates/app exists)
```
