# Current Status

> Startup handoff only. Read `Now`, then stop at the next `##` heading. Detailed history and
> rationale live in `plans/DECISION_LOG.md` and Git. Keep Now at no more than 10 bullets and
> retain only the 10 newest one-line session entries.

## Now (updated 2026-07-22)

- **M4 items 4.1–4.3 done:** 4.1 — `LodTier` enum + `next_tier` hysteresis state machine in
  `crates/core/src/lod.rs`, plus `Camera::viewport_span_km()`. 4.2 — `orthographic_forward`/
  `orthographic_inverse` in `crates/core/src/geo.rs` (new `UnitDiskXy` type) and a new
  `GlobeCamera` in `crates/core/src/globe_camera.rs` (rotate + cursor-anchored zoom, mirroring
  `Camera`'s scoping). 4.3 — `app::window::App` drives both cameras + an eased `mode_blend` every
  frame; `render` gained a real spherical basemap (`basemap::tessellate_globe`,
  `globe_basemap.wgsl`, per-fragment horizon clipping) and an additively-blended L0 density-dot
  layer (`density.rs`/`density.wgsl`). Full workspace check/test/clippy/fmt clean (677 tests).
  Live-verified against real OpenSky traffic in a windowed run — see DECISION_LOG for a bug found
  and fixed during that pass (flat Mercator map wasn't fading out under the globe) and a gap
  knowingly carried to 4.4 (ungated aircraft glyphs float disconnected from the globe while
  exploring L0 — owner-accepted, 4.4's own gating work resolves it).
- **M4 plan** ([plans/M4_DUAL_MODE_LOD_AND_INTERACTION.md](M4_DUAL_MODE_LOD_AND_INTERACTION.md)):
  7 items — 4.1–4.3 done above, 4.4 tier-gated rendering/cross-fade (expected to resolve both the
  carried trails-buffer blocker *and* 4.3's carried disconnected-glyphs gap as a consequence —
  verify, don't assume, for both), 4.5 Oklab altitude ramp, 4.6 emergency squawk plumbing + styling
  (closes 3.6's unscoped finding below), 4.7 gate.
- **Next action:** start 4.4 in a fresh session. Clean checkpoint — recommend `/clear` before
  opening it.
- **M3 gate closed 2026-07-21 (item 3.6):** docs/11 §M3's 5 acceptance lines all evidenced (2
  carry an already-recorded open half — L1/L2 tier switching → M4, now this plan; click-triggered
  live verification, pre-existing scripted-navigation gap). Kill-switch test live-verified (hosts
  file blocked adsbdb/aviationweather.gov; OpenSky kept flowing, METAR poller warned and retried
  normally, no panic). Full workspace fmt/clippy/`test --workspace` clean (629 passed, 8 ignored)
  before the gate. Emergency-squawk styling found unimplemented/unscoped — now 4.6 above, not
  reopened as a loose finding.
- **3.1–3.5 remain uncommitted** in the working tree — untouched this session; committing is the
  owner's call.
- **Carried 3.2 gap:** runway outlines still visually unconfirmed at close zoom.
- **Carried renderer blocker:** whole-world trails can exceed wgpu's 256 MiB buffer limit and
  panic; M4 4.4 is expected to resolve this via tier-gating, verified there rather than assumed.
- **Carried visual gap:** dense regional labels are algorithmically non-overlapping but remain
  visually cluttered after the 2.10 size increase.

## Gate record

| Milestone | State | Evidence |
|---|---|---|
| M0 | Gate run 2026-07-15 — 6/7; CI badge awaits first remote workflow run | DECISION_LOG M0 gate |
| M1 | Gate run 2026-07-18 — 6/7; token-refresh line owner-accepted open | M1 plan 1.13 |
| M2 | Gate run 2026-07-20 — 3/6 clean; 3 lines carried | M2 plan 2.10 |
| M3 | Gate run 2026-07-21 — 5/5 acceptance lines evidenced; 2 carry open halves (L1/L2 tier-switching → M4; click-triggered live verification, both pre-existing) | M3 plan 3.6, DECISION_LOG 2026-07-21 |
| M4 | In progress — 3/7 items done (4.1–4.3) | M4 plan, DECISION_LOG 2026-07-21/2026-07-22 |
| M5–M6 | Not started | Plans not yet written |

## Session log (newest first; keep 10)

- 2026-07-22 — M4 4.3 globe transition + L0 density layer implemented and live-verified against
  real traffic; fixed a flat-map-not-fading-out bug found during that pass; disconnected-glyphs
  gap knowingly carried to 4.4.
- 2026-07-21 — Wrote plans/M4_DUAL_MODE_LOD_AND_INTERACTION.md (7 items) after confirming
  approach with the owner; no implementation started.
- 2026-07-21 — M3 3.6 gate: acceptance lines recorded, docs/13 QA pass, live hosts-file
  kill-switch test passed; found emergency-squawk styling unimplemented and unscoped.
- 2026-07-21 — M3 3.5 selection info card enrichment (type/operator/route) implemented.
- 2026-07-21 — M3 3.4 adsbdb selection lookups implemented; `flights` table pulled forward from
  M5 to back route caching (owner decision).
- 2026-07-21 — M3 3.3 METAR polling + flight-category badges implemented and live-verified;
  fixed an idle-poller startup delay caught during that verification.
- 2026-07-20 — Token/throughput audit: bounded context and agents, delivery slices,
  risk-tiered checks, one-attempt visual QA, and a deterministic-navigation follow-up.
- 2026-07-20 — M3 3.2 airport/runway rendering implemented; visual confirmation gap recorded.
- 2026-07-20 — M3 3.1 OurAirports/runways import implemented.
- 2026-07-20 — M3 opened; plan written and M2 carry-overs retained.
