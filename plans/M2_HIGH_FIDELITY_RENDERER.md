# M2 — High-Fidelity Renderer

**Goal:** live regional traffic drawn at 60 fps with smooth interpolated motion — the moment
the project becomes *watchable*. Exit criteria: [../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md) §M2.
Constraining docs: 01 (all budgets), 09 (`RenderFeed`), 10 (§4 smoke tests), 13 (§L2-core),
and the [high-fidelity-flight-visualization skill](../.claude/skills/high-fidelity-flight-visualization/SKILL.md).

## Checklist

- [x] 2.1 Device/queue/surface init: prefer DX12 on Windows, fall back per wgpu defaults;
      swapchain resize (already in place from 0.6); MSAA 4x render-target plumbing; F3 toggles
      a frame-stats mode that computes p50/p95 (not just mean/worst) and surfaces them (log at
      info while toggled, since no on-screen overlay exists yet — see 2.1b).
      *(Split 2026-07-18, owner-approved: the checklist's "frame-stats overlay ... toggled with
      F3" implies on-screen text, but no text/glyph pipeline exists until 2.5 (SDF atlas) /
      2.7 (glyph-atlas labels) — building one now for a debug overlay would be thrown away or
      duplicated once the real atlas lands. This item ships everything else now; drawing the
      numbers on screen is 2.1b.)*
      *(2026-07-18: implemented — `Renderer::request_backend` tries DX12-only first on
      `cfg(windows)` (skipped if `WGPU_BACKEND` is set, so the documented bisection path still
      wins), falling back to wgpu's default multi-backend selection on failure; verified live,
      `backend=dx12` on this machine. A 4x-multisampled color target (`Renderer::msaa_view`) is
      created alongside the swapchain and rebuilt in `reconfigure`, with an
      `adapter.get_texture_format_features` check gating a new `RenderError::UnsupportedMsaa`
      rather than letting an incapable adapter panic; `render`'s pass now targets it and
      resolves onto the swapchain view (`StoreOp::Discard` on the MSAA attachment itself, only
      the resolve needs to survive to present). `FrameStats` gained a per-window
      `Vec<Duration>` sample buffer and a nearest-rank `percentile` helper (integer arithmetic,
      sidesteps float-cast clippy lints) yielding `p50`/`p95` alongside the existing
      `mean`/`worst`; F3 (press-edge only, via `winit::keyboard`) toggles `App::stats_visible`,
      which widens the once-a-second log line from `debug` to `info` and adds
      `p50_ms`/`p95_ms`/`instances=0` (the last pinned until 2.5 gives the render loop
      something to count). Delegated to the renderer-agent, independently verified by this
      session: `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test
      --workspace` re-run fresh (**332 passed, 5 ignored, 0 failed** — the agent's own reported
      count was wrong, corrected here rather than trusted), diff read in full, and a live run
      driven over Win32 independent of the agent's own: `backend=dx12` confirmed, two live
      resizes (500×400 then 1000×700) with no panic and the MSAA target rebuilding cleanly
      each time, F3 toggled `stats_visible` with the log line switching format as designed,
      `WM_CLOSE` → "close requested" → "window closed", clean exit. DECISION_LOG 2.1.)*
- [ ] 2.1b Render the F3 frame-stats overlay (p50/p95, instance counts) on screen, reusing the
      glyph atlas built in 2.5/2.7 rather than a one-off text renderer. Depends on 2.7; do not
      start before it lands.
- [x] 2.2a Base map data: fetch Natural Earth 1:50m land + coastlines and bundle as GeoJSON in
      `crates/render/assets/basemap/` (no runtime fetch — `render` stays network-free, ADR-002).
      *(Split 2026-07-18, self-approved same-session: the checklist's "bundled as GeoJSON"
      needs the data to actually exist first, and acquiring it — a live download, format
      conversion, a new crate to hold tooling that must never touch `render`'s Cargo.toml — is
      its own scoped piece of work, cleanly separable from the tessellation/pipeline half.
      Same shape as 2.1/2.1b's split.)*
      *(2026-07-18: implemented — new workspace crate `crates/import` (`look-above-import`),
      not depended on by anything (`app` never sees it; it exists only to be run by hand), one
      bin: `import-basemap`. **The documented download host is dead**: docs/03 pointed at
      `naturalearthdata.com/downloads/`, but that page's own direct file links 404 — checked
      live, not assumed. The actual files are served from Natural Earth's own CDN,
      `naciscdn.org` (linked from the same downloads page), confirmed with a live `200` on
      both zips (~450 KB each); docs/03 updated to record this. `ALLOWED_STATIC_HOSTS` gates
      the fetch exact-match/https-only, mirroring `ingest::allowlist`'s rigor even though
      nothing here ships in the app. **Shapefile, not GDAL**: the `shapefile` crate (pure
      Rust, no system GDAL dependency) parses `.shp` bytes read straight out of the downloaded
      zip via the `zip` crate — no `.shx`/`.dbf` needed, since this tool reads every shape once
      sequentially and wants no attribute columns. API confirmed by reading the vendored crate
      source directly (CLAUDE.md's dependency-discovery rule), not guessed. **The grouping
      heuristic**: a shapefile `Polygon` record can hold several disjoint outer rings (a
      continent plus its islands in one record), which GeoJSON's `Polygon` type cannot
      represent — each outer ring starts a new output feature, and inner (hole) rings attach to
      the outer ring immediately preceding them, the same ordering convention every common
      shapefile writer (including Natural Earth's own) actually produces. Coastline parts each
      become their own `LineString` feature. Coordinates rounded to 1e-4° (~11 m) to keep the
      bundled text compact — 1:50m is already Natural Earth's own generalization, so no further
      simplification pass was added. **Verified live**: 1,420 land shapefile records → 1,421
      polygon features (one record held two disjoint outer rings), 1,428 coastline records →
      1,429 line features; both files structurally checked (feature/geometry-type counts, point
      totals, lon/lat extents sane at ±180°/±90°) without ever printing a coordinate into this
      session (docs/06). ~1.2 MB each, ~2.5 MB combined — well inside the render-asset memory
      budget. 10 new offline unit tests (host gate, coordinate rounding, the outer/inner
      grouping heuristic including the two-disjoint-outer-rings case, polyline part splitting);
      `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace`
      green. `crates/render/assets/basemap/README.md` documents provenance, format, and the
      regeneration command. DECISION_LOG 2.2a. Next: **2.2b**, tessellation + pipelines.)*
- [x] 2.2b Base map render: tessellate the bundled GeoJSON from 2.2a once at startup (`lyon`)
      into static vertex buffers; line + fill pipelines; desaturated dark palette per docs/01.
      *(2026-07-18: implemented — new `crates/render/src/basemap.rs` embeds both `GeoJSON`
      files via `include_str!`, parses with `serde_json`, projects `[lon,lat]` → Web Mercator
      via `core::geo::web_mercator_forward` (reused, not reimplemented), normalizes to a
      `[-1,1]`-ish plane, and tessellates land polygons (`lyon::FillTessellator`,
      `FillRule::NonZero` — matches RFC 7946's outer-CCW/hole-CW winding) and coastlines
      (`StrokeTessellator`) into one static vertex/index buffer per layer, uploaded once in
      `Renderer::new` and never rebuilt. New `crates/render/src/shaders/basemap.wgsl`: one
      shared vertex stage (a `view_proj` uniform — a placeholder aspect-correcting
      fit-to-window matrix for now, no camera until 2.3), one fragment stage reading a
      per-pass `@group(1)` color uniform sourced from new `color.rs` constants (`#12161D`
      land, `#2E3742` coastline, picked the same "ours to fix" way the `#0A0E14` background
      was). Draw order: background clear → land fill → coastline stroke, all in one render
      pass per docs/01. Delegated to the renderer-agent (a mid-session connection error
      interrupted the first attempt; resumed the same agent from its transcript rather than
      restarting cold), independently re-verified by this session: `cargo fmt --check`/
      `clippy --workspace --all-targets -D warnings`/`test --workspace` re-run fresh
      (**349 passed, 5 ignored, 0 failed**, matching the agent's own count), every changed/new
      file read in full, `lyon`/`bytemuck` moved from ad-hoc inline deps into
      `[workspace.dependencies]` to match repo convention (the one deviation found), and a
      live `cargo run -p look-above` driven independently rather than trusting the agent's own
      screenshots — which surfaced a real DPI-awareness pitfall in the verification tooling
      itself (see DECISION_LOG 2.2b) before confirming a correct, symmetric, aspect-preserving
      world map across three window sizes and a clean `WM_CLOSE` exit. DECISION_LOG 2.2b.
      Next: **2.3**, the regional camera.)*
- [ ] 2.3 Camera (regional): Web Mercator, pan (drag) + zoom (wheel, cursor-anchored) with
      inertia; viewport→bbox exposed to the poller (M1 poller re-targets on camera settle,
      debounced 2 s).
- [ ] 2.4 `core::sim`: interpolation/dead-reckoning worker — rayon over the live aircraft
      table at render cadence; destination-point advance from last fix (speed/track/vert
      rate); ease-out correction blend (≤ 2 s) on new fix; stale fade (60 s + 5 s); writes
      `RenderFeed` into the double buffer. Unit tests per docs/10 §1.
- [ ] 2.5 Aircraft glyphs: SDF atlas (6 categories per docs/01), instanced quad pipeline,
      per-instance rotation from heading, altitude-bucket tint attribute (final ramp colors
      may land in M4; buckets wired now).
- [ ] 2.6 Trails (in-memory, last 5 min): per-aircraft ring buffer → triangle-strip ribbons,
      taper width/alpha, altitude-ramp coloring.
- [ ] 2.7 Labels: glyph-atlas text (callsign + FL + kt), CPU collision culling with priority
      (docs/01), leader-line when displaced.
- [ ] 2.8 Selection: cursor hit-test against glyph quads (CPU, spatial index), white outline,
      minimal info card (callsign/alt/speed/source — enrichment fields arrive in M3;
      anonymous → "Unidentified" already enforced here).
- [ ] 2.9 Renderer smoke test (headless, per docs/10 §4) wired into CI (skip-if-no-adapter).
- [ ] 2.10 Gate: live run over a busy hub; visual QA §L2-core; frame-stats evidence; human review.

## Design notes

- Render thread does: swap buffer, upload instances, record command buffer, present. Nothing
  else. All layout/culling/interpolation happens on workers (ADR-002).
- Instance buffer uploads use a persistently mapped staging ring; budget in docs/01 makes
  this comfortably small — don't over-engineer.
- Spatial index (for hit-testing and later label density): start with a simple uniform grid
  over screen space rebuilt per frame; r-tree only if profiling demands it (record either
  way in DECISION_LOG).
- The interpolation math is specified in the visualization skill — implement to that spec so
  tests and code cite the same formulas.
