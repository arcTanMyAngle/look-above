# 07 — Milestone Plan (M0–M6)

Each milestone has a **gate**: exit criteria (measurable versions in
[11_ACCEPTANCE_CRITERIA.md](11_ACCEPTANCE_CRITERIA.md)) verified and recorded in
`plans/CURRENT_STATUS.md`, then a human review before the next milestone opens.
Detailed task checklists for M0–M2 live in [../plans/](../plans/); M3–M6 get their plan
files written as part of the preceding milestone's gate.

## M0 — Repo & Architecture Setup

Cargo workspace (`crates/core`, `ingest`, `store`, `render`, `app`), pinned dependencies,
CI stub (fmt + clippy + test on push), config loading, logging, ADRs confirmed.
**Gate:** `cargo test --workspace` green in CI; empty winit window opens and closes cleanly.
→ [../plans/M0_REPO_AUDIT_AND_ARCHITECTURE.md](../plans/M0_REPO_AUDIT_AND_ARCHITECTURE.md)

## M1 — Authorized Data Ingestion

OpenSky OAuth2 client + poller with credit budgeting; one no-key fallback source
(airplanes.live or adsb.lol); normalized `StateVector`; dedup/staleness handling;
fixture-recording script; `source_status` reporting.
**Gate:** app (headless) logs live aircraft counts for a bounding box for 10 continuous
minutes within rate budget, from either source, with tests on fixtures.
→ [../plans/M1_AUTHORIZED_DATA_INGESTION.md](../plans/M1_AUTHORIZED_DATA_INGESTION.md)

## M2 — High-Fidelity Renderer

winit + wgpu surface; Natural Earth base map (tessellated once); instanced aircraft glyphs
with heading; CPU interpolation/dead-reckoning worker feeding double-buffered render buffer;
regional camera (Web Mercator), pan/zoom.
**Gate:** live regional traffic renders at 60 fps with smooth (no-teleport) motion; visual
QA checklist §L2-core passes.
→ [../plans/M2_HIGH_FIDELITY_RENDERER.md](../plans/M2_HIGH_FIDELITY_RENDERER.md)

## M3 — Enrichment & Non-ADS-B Integration

OurAirports import (airports/runways → SQLite); METAR polling + flight-category badges;
adsbdb selection lookups behind the anonymity gate (privacy rule 2.2); selection info card
data path.
**Gate:** selecting an aircraft shows route/type/operator when available; airports render
with runway outlines; all enrichment failures degrade silently.

## M4 — Dual-Mode LOD & Interaction

Orthographic globe (L0) with density rendering; L0↔L1↔L2 LOD tiers with hysteresis; animated
globe↔mercator camera transition; label collision culling; selection UX; altitude color ramp
final.
**Gate:** continuous zoom world→runway with no popping/flicker; 8k+ aircraft global at 60 fps;
visual QA checklist passes in full.

## M5 — Persistence, History & Replay

Position history writes (batched, WAL); retention caps + pruning (privacy rule 5.1); trail
rendering from history; time-scrubber replay of the local recording window.
**Gate:** 24 h continuous run stays under memory/disk caps; replay honors anonymity
retroactively (rule 5.2).

## M6 — Polish & Packaging

Settings UI (egui overlay), light theme, day/night terminator, About/attribution screen,
first-run experience (no-credentials degraded mode messaging), Windows release packaging,
README quick-start verified end-to-end by a clean clone.
**Gate:** v1 success criteria in [00_PRODUCT_VISION.md](00_PRODUCT_VISION.md) all check out.

## Sequencing rules

- No milestone starts before the previous gate is recorded. Bugfixes to earlier milestones
  are always in scope.
- Scope creep goes to `plans/NEXT_ACTIONS.md` as post-v1 candidates, not into the current milestone.
- Dependency upgrades happen only at gates (ADR-003 consequence).
