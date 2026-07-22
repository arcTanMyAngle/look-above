# M4 — Dual-Mode LOD & Interaction

**Goal:** the three-tier zoom experience becomes real: an orthographic L0 globe with density
rendering, hysteresis-gated L0↔L1↔L2 tier switching, an animated globe↔Mercator camera
transition, tier-gated trails/labels, the perceptual altitude ramp, and emergency squawk styling.
Exit criteria: [../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md) §M4.
Constraining docs: 01 (§Projection, §LOD budgets), the
[high-fidelity-flight-visualization skill](../.claude/skills/high-fidelity-flight-visualization/SKILL.md)
(LOD tier table, glyph/trail/label tier rules, Oklab altitude ramp — read before any renderer or
`core::camera` work), 04 (rule 6.1 — squawk display stays passive-only, no correlation/export),
13 (§L1/L0 + transitions, §Selection & overlays — both required at this gate).

## Known cross-milestone inheritance (read before 4.1)

M2's gate (2.10) and M3's gate (3.6) both explicitly deferred true LOD tier switching to this
milestone: the renderer currently draws everything at one fixed L2-style tier, and
`crates/core/src/camera.rs` is documented as "the *regional* Web Mercator camera only... there is
no global/orthographic view yet." M3's `Store::airports_in_bbox(bbox, min_size)` query-side
filtering already exists and is ready to be driven by whatever tier signal 4.1 produces — no
store-side work is expected here.

Two more open items feed into this milestone rather than being new discoveries:
- The M3 gate (3.6) found "emergency squawk styling" unimplemented and unscoped anywhere in
  `core::contracts`/`ingest`/`render`, flagged for the owner to schedule. Docs/13 §Selection &
  overlays lists it as "required at M3/M4," so it's scoped into this plan as 4.6 rather than
  left open again.
- `crates/render/src/color.rs`'s own comment (lines 84–88) already flags that the *perceptual*
  Oklab-interpolated altitude ramp — as opposed to the six flat per-bucket tints M2 shipped —
  "lands in M4." Scoped as 4.5.
- The carried renderer blocker (whole-world trails exceed wgpu's 256 MiB buffer, reproduced live
  during M3) is expected to be resolved as a *consequence* of 4.4's tier-gating (trails are L2-only
  per the skill, so an L0/L1 whole-world view never asks the trail buffer to hold planet-scale
  data) — 4.4 must verify this rather than assume it, and reopen the blocker explicitly if gating
  alone doesn't fix it (e.g. a fast zoom-out while a large L2 trail buffer is still live).

## Checklist

- [x] 4.1 LOD tier state with hysteresis (`core`): `LodTier` enum (`Global`/`Continental`/
      `Regional`) and `next_tier(previous, viewport_span_km)` state machine in new
      `crates/core/src/lod.rs`; `Camera::viewport_span_km()` added alongside `viewport_bbox` as
      the km-span source (a direct `width_px * meters_per_pixel` reading, not bbox-derived, so it
      stays representative when panned past the world edge or framing a pole). Asymmetric
      thresholds (3,300/3,000 km, 330/300 km) as the hysteresis band, each tier only exiting
      toward the threshold it can be re-entered from — one call resolves a fast multi-tier zoom
      directly, no stepping required. Acceptance met: unit tests dither ±5% around all four
      thresholds from each adjacent starting tier and settle without flipping; genuine-crossing
      tests both directions; workspace-isolated `cargo check`/`test`/`clippy --all-targets`/`fmt`
      on `look-above-core` all clean (31 tests passing, no new lints).
- [x] 4.2 Orthographic globe camera (`core`): `orthographic_forward`/`orthographic_inverse` added
      to `crates/core/src/geo.rs` (new `UnitDiskXy` unit-disk position type alongside
      `MercatorXy`), plus a new `GlobeCamera` in `crates/core/src/globe_camera.rs` mirroring
      `Camera`'s "no wgpu, no matrices, no winit" scoping: `rotate_by_pixels` (immediate, pixel-
      to-radian-linearized drag, sign convention matching `pan_by_pixels`) and `zoom_by_notches` +
      `update` (eased `radius_px`, cursor-anchored via a per-frame first-order correction toward
      the anchor's disk position — documented as a linear approximation, exact only near screen
      center; off-globe cursor clicks fall back to center-anchored scaling). No renderer wiring
      yet. Acceptance met: combinatorial-grid property tests (7 latitudes × 9 longitudes,
      including both poles and the antimeridian, for both `center` and `point`) confirm every
      visible-hemisphere projection lands inside the unit disk, every far-hemisphere point is
      excluded, and no NaN/Infinity is produced anywhere in the grid for forward or inverse;
      workspace-isolated `cargo check`/`test`/`clippy --all-targets`/`fmt` on `look-above-core`
      all clean (168 tests passing, no new lints).
- [x] 4.3 Globe↔Mercator transition + L0 density layer (`render`, `app`): `app::window::App` gained
      a `GlobeCamera` alongside the Mercator `Camera`, an `LodTier` recomputed every frame from
      `Camera::viewport_span_km()`, and an eased `mode_blend` (owner confirmed full-spherical-basemap
      scope via AskUserQuestion, not a placeholder disk) — `ease_mode_blend` mirrors `Camera`/
      `GlobeCamera`'s own exponential-ease-toward-target shape, converging within docs/13's 500 ms
      ceiling, retargetable (interruptible) for free. Both cameras receive every drag/wheel/resize
      input unconditionally (deliberate — see the field doc on `App::globe_camera`). `render` gained
      a real spherical basemap (`basemap::tessellate_globe`, `GlobeBasemapLayer`, per-vertex
      orthographic projection in `globe_basemap.wgsl` with a per-fragment `cos_c` discard for clean
      horizon clipping) and an additively-blended L0 density-dot layer (`density.rs`,
      `density.wgsl`) fed by the same `RenderFeed.aircraft` the glyph layer reads, culled to the
      near hemisphere via `core::geo::orthographic_forward`. Aircraft glyphs/trails/labels are
      deliberately *not* gated here — that's 4.4's job. Acceptance met via live-app visual pass
      (real OpenSky traffic, ~7,000 aircraft): globe renders as a correctly clipped hemisphere,
      density dots plausibly track real traffic contrast (Europe/Middle East/India denser than open
      ocean), flat Mercator map fades out under the globe with no leftover corners. One real bug
      found and fixed during that pass (see DECISION_LOG) and one gap knowingly carried to 4.4 (see
      Now section) — both owner-confirmed, not silently resolved.
- [ ] 4.4 Tier-gated rendering + cross-fade (`render`): gate `TrailLayer`/`LabelLayer` to
      `Regional` only and glyph vs. density-dot drawing to the correct tier per the skill's table;
      cross-fade opacity over 250 ms at tier boundaries so nothing pops. Verify (not assume) that
      this resolves the carried whole-world-trails-vs-256-MiB-buffer blocker from M2/M3 by
      exercising a fast global-to-runway-and-back zoom during manual verification; if a residual
      panic path remains (e.g. mid-transition), record it as a reopened blocker rather than
      silently re-carrying it. Acceptance: docs/13 §L1/L0 lines 1, 4, 5 (continuous zoom
      cross-fade; L0 density honesty; 8,000+ aircraft global at p95 < 16.6 ms via frame-stats
      overlay). Gating/cross-fade code implemented (`renderer.rs`, `aircraft.wgsl`,
      `trail.rs`/`label.rs` alpha multipliers), workspace-clean; buffer blocker verified resolved
      (reached ~12,500 tracked at Global, no panic, twice). Still open after two live passes: (a)
      pop-free cross-fade — owner reports popping still occurring at both tier boundaries; no
      gating/tint logic bug found on inspection, leading suspicion is `TIER_BLEND_EASE_TAU_S`'s
      front-loaded exponential ease reading as a snap rather than a fade (owner declined an
      experimental fix this session — see `DECISION_LOG` 2026-07-22); (b) the 8,000+-aircraft p95
      line — attempted this session but the run's OpenSky call failed over to `airplaneslive`
      (250 nm radius cap, only 3–5 aircraft), so no valid reading exists yet.
- [x] 4.4a Camera-sync follow-up (`app`): `window.rs` feeds every wheel/drag input to both
      `Camera` (Mercator) and `GlobeCamera` unconditionally (4.3's deliberate design), but nothing
      keeps their *centers* aligned, so navigating via the globe doesn't reliably land the Mercator
      camera — the one that actually decides `LodTier` and renders aircraft/trail/label — on the
      same place. Confirmed live during 4.4 verification: a retargeted-bbox log showed ~1,000+ km
      span at a point the owner believed was a runway-level Regional zoom. Fix: re-center the
      Mercator camera to the globe's sub-observer point as tier leaves `Global` (`Camera::
      set_center_latlon`, `crates/core/src/camera.rs`; called once from `App::draw`,
      `crates/app/src/window.rs`). Not a 4.4 regression — pre-existing since 4.3, newly visible now
      that tiers render differently. Acceptance met: live-pass log shows a globe-driven zoom onto
      Sardinia/southern Italy landing a ~91 km × 132 km bbox (well under the 300 km Regional
      threshold) at that exact real location, matching screenshots of full Regional-tier rendering
      there — see `DECISION_LOG` 2026-07-22.
- [x] 4.5 Perceptual altitude ramp (`render`): `color::AltitudeRamp` replaces the six flat
      per-bucket tints with Oklab interpolation between the same six stops, anchored at each
      bucket's midpoint altitude and continuous by the aircraft's/trail-sample's actual altitude
      (`AircraftInstance`/new `TrailVertex::altitude_ft`+`on_ground` fields) rather than snapping
      at a discrete bucket. Built once per surface format (mirrors the old tint-table shape);
      per-instance/per-vertex cost is a lerp plus one inverse-only Oklab conversion. A real,
      previously-unverified gap surfaced while writing the deuteranopia test: the M2/M3-authored
      stops' Oklab lightness peaked at `To10000Ft` and declined afterward, so it could never have
      passed docs/13's monotonic-lightness line — owner chose (asked directly, not assumed) to
      nudge each stop's lightness by the minimal-L2-distance monotonic correction (isotonic
      regression, hue/chroma unchanged), not a free re-authoring; see `DECISION_LOG` 2026-07-22.
      Acceptance met: docs/13 accessibility line now covered by an automated deuteranopia-
      simulation test; former `altitude_bucket_tint*` tests replaced with continuity/anchor/clamp/
      range coverage for the new ramp. Full workspace fmt/clippy/`test --workspace` clean (684
      passed across all crates, 8 ignored in `ingest`).
- [ ] 4.6 Emergency squawk plumbing + styling (`ingest`, `core`, `render`): surface the squawk
      code already present in the raw ADS-B feed JSON (confirmed unused past ingest — see
      `crates/ingest/src/opensky/states.rs` and the adsblol/airplaneslive fixtures) through
      `ingest::normalize` into `core::contracts::StateVector`, and a pulsing red ring (1 Hz) in
      `render::aircraft`/`aircraft.wgsl` for 7500/7600/7700 per the skill's glyph spec. Passive
      display only — no notification, alert, export, or any new correlation path (privacy rule
      6.1); anonymous (LADD/PIA) aircraft keep rule 2.2's existing no-identity treatment
      regardless of squawk. Acceptance: docs/13 §Selection & overlays squawk line; a unit test
      confirming no new outbound call or persisted record is triggered by a squawk value alone.
- [ ] 4.7 Gate: full docs/13 visual QA checklist pass (not just the L2-core subset — §L1/L0 +
      transitions and §Selection & overlays in full), frame-stats evidence for the 8k+ global
      p95 < 16.6 ms line, docs/11 §M4's five acceptance lines all evidenced, human review.
