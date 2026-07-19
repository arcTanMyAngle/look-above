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
- [x] 2.1b Render the F3 frame-stats overlay (p50/p95, instance counts) on screen, reusing the
      glyph atlas built in 2.5/2.7 rather than a one-off text renderer. Depends on 2.7; do not
      start before it lands.
      *(2026-07-19: implemented — new `render::stats_overlay` (pure: `StatsOverlay` plain-data
      input, `format_lines`, `pack_overlay_instances`), a small `StatsOverlayLayer` in
      `renderer.rs` built by **cloning** `LabelLayer`'s already-built text pipeline/atlas
      bind group/quad mesh/screen-params bind group (all cheap `Arc`-backed `wgpu` handles) —
      no second SDF atlas texture or pipeline. Fixed top-left HUD, 4 lines (`FPS n` /
      `P50 nMS  P95 nMS` / `WORST nMS` / `N n`), deliberately kept inside label_atlas's existing
      39-character set (ALL CAPS, whole numbers, no new glyphs) rather than growing the atlas for
      a debug overlay. `Renderer::render` gained a trailing `stats: Option<StatsOverlay>` param;
      `None` (F3 off) builds/uploads nothing. Drawn last, after the label pass — docs/01's draw
      order is now implemented end to end. `app::window` gained `last_stats_summary` (persists
      the once-a-second `FrameStats::record` result so the HUD doesn't blank between reports) and
      builds `StatsOverlay` from it each frame the same numbers the existing log line already
      uses. Delegated to the renderer-agent (GPU pipeline/atlas reuse, its stated remit), briefed
      with the exact character-set constraint and reuse-don't-duplicate call already made.
      Independently re-verified: every changed/new file read in full, fresh
      `fmt`/`clippy --all-targets -D warnings`/`test --workspace` matched the agent's own count
      exactly — **486 passed, 5 ignored, 0 failed** (+9 over 2.7b's 477, all in
      `render::stats_overlay`/`color`). **Live-verified independently**: launched the built
      binary directly, screenshotted with F3 off (no HUD) and on (HUD present), cropped/4x
      nearest-neighbor-upscaled the HUD region — legible cyan stroke-font text reading `FPS 47`,
      `P50 9MS  P95 17MS`, `WORST 60MS`, `N 9102` against a live whole-world OpenSky feed;
      aircraft glyphs/labels/trails all still rendered correctly alongside it. Clean `WM_CLOSE`;
      scratch `look_above.db` deleted after per 1.12/1.13's convention. DECISION_LOG 2.1b. M2
      checklist items remaining: **2.8** (selection), 2.9 (smoke test/CI), 2.10 (gate).)*
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
- [x] 2.3a Camera (regional): Web Mercator, pan (drag) + zoom (wheel, cursor-anchored) with
      inertia, replacing 2.2b's placeholder fit-to-window matrix with a real view-proj
      transform. Window mode only (headless has no camera).
      *(Split 2026-07-18, self-approved same-session, same shape as 2.1/2.1b and 2.2a/2.2b:
      the checklist bundles the camera itself with exposing its viewport to the poller, but
      those are two different lanes — pure `core`/`render`/input-handling math here, vs. a new
      `ingest::poller` retarget API and, for the first time, running the live network pipeline
      from window mode (today only `--headless` does) for 2.3b. Nothing in 2.3b can be
      meaningfully written or tested until 2.3a's camera exists to feed it, so the order is
      fixed, not arbitrary.)*
      *(2026-07-18: implemented — new `core::camera::Camera` (pure state/math, no wgpu/winit):
      pan (`pan_by_pixels`, immediate 1:1), drag lifecycle + EMA-sampled inertia velocity
      (`begin_drag`/`drag_to`/`end_drag`, exponential friction decay in `update`), cursor-
      anchored wheel zoom (`zoom_by_notches` sets a target + clip-space anchor, `update` eases
      `meters_per_pixel` toward it while re-centering to keep the anchored world point fixed).
      **Scope is deliberately the regional Web Mercator camera only**: `meters_per_pixel` is
      clamped at a zoom-out ceiling — `2 * WEB_MERCATOR_EXTENT_M / min(width_px, height_px)`,
      the same "contain, whole world visible, letterboxed" fit 2.2b's placeholder hardcoded —
      because there is no L0 globe/orthographic view yet; zooming out further would show empty
      space with nothing to fill it. That ceiling formula is also `Camera::new`'s starting
      framing, so construction reproduces the old placeholder's exact initial view (pinned by
      tests, and the actual reason there's no visual jump at startup). New
      `render::camera_view_proj(&Camera) -> [[f32;4];4]` (replacing the deleted
      `fit_to_window_matrix`) re-derives the scale from `meters_per_pixel`/viewport pixels and
      folds in the same `/ WEB_MERCATOR_EXTENT_M` pre-normalization `basemap.rs`'s static
      vertices already carry, so the matrix and the mesh agree on units without either
      duplicating the other's constant. `Renderer::set_view_proj` is now the buffer's only
      writer — the camera itself lives in `app` (it needs winit events; `render` stays
      winit-free per ADR-002/M0's dependency-direction check), so `Renderer::reconfigure` no
      longer touches the view-proj buffer at all on resize (the app's `Resized` handler calls
      `Camera::resize` + `set_view_proj` synchronously, before the next frame, so nothing stale
      is ever presented). `app::window`'s `App` gained a `Camera`, drag/wheel handling
      (`CursorMoved`/`MouseInput`/`MouseWheel`), and per-frame `dt_s` tracking so `Camera::update`
      runs once per presented frame before the matrix is rebuilt and handed to the renderer.
      Delegated in two lane-scoped pieces (geo-math-agent for `core::camera`, renderer-agent for
      the render/app wiring, the second briefed with the first's finished API rather than
      running in parallel, since it depends on exact method signatures) and independently
      re-verified both: `cargo fmt --check`/`clippy --workspace --all-targets -D
      warnings`/`test --workspace` re-run fresh after each (**369 passed, 5 ignored, 0 failed**
      final total, +20 from 2.2b's 349 — 14 new `core::camera` tests, 6 new `render` matrix
      tests), every changed/new file read in full, and a live run driven independently: a
      scripted Win32 drive (`SetCursorPos`/`mouse_event`, DPI-aware per 2.2b's own lesson)
      exercised a drag pan, inertia coasting to a stop, cursor-anchored zoom in then back out
      (round-tripped to the same view, confirming no drift), a resize, and a clean `WM_CLOSE`
      exit (code 0) — six screenshots confirmed the map follows the drag direction correctly on
      both axes, no seams/cracks/missing polygons at any point (docs/13's L2-core pan/zoom-
      inertia line), and the resize reflowed without distortion. DECISION_LOG 2.3a. Next:
      **2.3b**, viewport→bbox exposed to the poller.)*
- [x] 2.3b Viewport→bbox exposed to the poller: `ingest::poller` gains a way to retarget its
      `RegionQuery` while running; window mode (currently render-only, no ingest pipeline at
      all) starts the poller against the camera's current viewport bbox and retargets on
      camera settle, debounced 2 s. Depends on 2.3a.
      *(2026-07-18: implemented — new `core::camera::Camera::viewport_bbox() -> BBox`
      (clamped into the valid Mercator/lat-lon domain so an overflowing or off-world viewport
      still yields a constructible, non-inverted `BBox`); `ingest::poller::Poller::new`/
      `with_default_chain` now take a `tokio::sync::watch::Receiver<RegionQuery>` instead of a
      fixed `RegionQuery`, and `run()` races its cadence sleep against the channel so a retarget
      takes effect on the very next cycle, not after waiting out up to `MAX_INTERVAL`; window
      mode (`app::window`) now opens the same `store::Writer`/`HttpClient`/`OpenSkyAuth`/
      `Poller`/ledger-restore pipeline headless mode does (merge+log+persist extracted to shared
      `app::pipeline::record_cycle`), seeded from the camera's initial `viewport_bbox()` and
      retargeted once the camera has sat still for 2 s on a bbox that differs from whichever was
      last sent — including on a plain window resize with no pan/zoom, closing a gap this
      session's own re-verification found and fixed after the delegated implementation. Three
      lane-scoped pieces (this session for `core::camera`, data-source-agent for `ingest`,
      renderer-agent for `app`, sequential since each needed the previous one's finished
      signature); independently re-verified rather than trusted (full diffs read, fresh
      `fmt`/`clippy --all-targets -D warnings`/`test --workspace` — **375 passed, 5 ignored, 0
      failed**) and live-driven against the owner's real OpenSky credentials, confirming both
      the initial fetch and five real mid-run retargets with distinct bboxes. DECISION_LOG
      2.3b. Next: **2.4**, `core::sim`.)*
- [x] 2.4a `core::sim` engine: the pure interpolation/dead-reckoning worker and the
      `RenderFeed`/`AircraftInstance` shapes it produces — destination-point advance from last
      fix (speed/track/vert rate), ease-out correction blend (≤ 2 s) on a new fix with the
      no-backward-along-track invariant, teleport-snap exception (> 10 km), stale fade (60 s +
      5 s), altitude buckets, `advance_all` as a rayon parallel iterator. No I/O, no app/render
      wiring. Unit tests per docs/10 §1.
      *(Split 2026-07-18, self-approved same-session, same shape as 2.1/2.1b, 2.2a/2.2b,
      2.3a/2.3b: 2.4 bundles the pure `core` math with the double-buffer handoff and the
      app-loop wiring that runs it at render cadence, but those are two different lanes — pure
      geo/sim math in `core` here (nothing else can be written or tested against an engine that
      doesn't exist yet), vs. the double buffer + feeding it from the live `SessionTable` +
      swapping it for 2.4b. Nothing visible renders from the feed until 2.5's glyph pipeline
      regardless, so 2.4b's verification is a logged instance count, not a picture.)*
      *(2026-07-18: implemented — new `crates/core/src/sim.rs` (`Simulator`, `RenderFeed`,
      `AircraftInstance`, `AltitudeBucket`). Two entry points at two rates: `ingest(states,
      now_s)` on each poll cycle (a fix newer than the held one starts a correction blend;
      older-or-equal is ignored, so a re-sent `SessionTable` fix does not restart a blend), and
      `advance_all(now_s)` once per frame — a **rayon `par_iter_mut`** over the track table that
      dead-reckons, blends, fades, and projects to Web Mercator, returning the flat feed. The
      math is the high-fidelity-flight-visualization skill's, reusing `core::geo`
      (`destination_point`/`haversine`/`initial_bearing`/`web_mercator_forward`) rather than
      re-deriving: dead reckoning with Δt clamped `[0, DROP_AFTER_S]` (never rewinds on source-
      clock skew, never flings on a stale fix — both tested on the private `dead_reckon`
      directly since a *visible* aircraft never ages past ~65 s); ease-out (`1−(1−u)²`) geodesic-
      slerp blend over a 2 s window, heading blended shortest-arc; the **no-backward-along-track
      invariant** enforced by clamping any step whose along-track component (projected on the
      fix's track) is negative back to the previous position (a fix behind the shown position
      slows the aircraft to a stop, never reverses it); a **teleport exception** (> 10 km error)
      that fades out, snaps at the midpoint while invisible, and fades back in over 300 ms
      rather than sliding across the map; and the **stale fade** reusing `merge`'s
      `STALE_AFTER_S`(60)/`DROP_AFTER_S`(90) — alpha ramps to 0 over a new `FADE_DURATION_S`(5),
      the instance drops out of the feed at 65 s but the track lingers (invisible) until 90 s so
      a reacquisition inside that window blends rather than pops. `AltitudeBucket` wires the
      skill's six ramp stops (colors are M4); `AircraftInstance.category` is `Unknown` until
      enrichment (M3/2.5). All state is `f64` and `Copy` (no render-specific narrowing in
      `core`); `RenderFeed` carries `frame_ts` + address-sorted `aircraft` only — trails/labels
      (docs/09's full shape) are appended by 2.6/2.7, not defined empty ahead of need. **Done
      directly, not delegated** — the geo-math lane's inputs (`geo.rs`, `types.rs`, `merge.rs`,
      `contracts.rs`) were already fully read this session, so a cold subagent would only
      re-derive them. 20 new unit tests covering every docs/10 §1 line (advance-along-track,
      vertical-rate integration across a band boundary both signs, blend convergence within the
      window, the no-backward invariant, teleport, stale-fade timing + reacquisition, Δt clamp,
      missing-field holds, on-ground non-extrapolation, bucket boundaries, and the ease-out/
      heading/geodesic helpers). `cargo fmt --check`/`clippy --workspace --all-targets -D
      warnings`/`test --workspace` all green — **394 passed, 5 ignored, 0 failed** (+19 over
      2.3b's 375). No live run: pure library math with no runtime surface until 2.4b/2.5 wire a
      consumer. DECISION_LOG 2.4a. Next: **2.4b**, the double buffer + app-loop wiring.)*
- [x] 2.4b `core::sim` wiring: double-buffer the `RenderFeed` (producer on workers, consumer on
      the render thread, swapped at frame start per ADR-002); feed the simulator from the live
      `SessionTable` both pipelines maintain; run `advance_all` at render cadence in window mode
      and hand the buffer to the renderer. No glyphs yet (2.5) — verified by a logged instance
      count that tracks the live feed. Depends on 2.4a.
      *(2026-07-18: implemented — new `app::simulation` (a dedicated worker thread) and
      `app::double_buffer` (a latest-wins SPSC mailbox, `Producer`/`Consumer` over
      `Arc<Mutex<Option<RenderFeed>>>`). The merge/interpolate/persist side moved off the render
      thread entirely (ADR-002): the worker owns the `SessionTable`/`Writer`/batch-receiver,
      drains poll cycles through the shared `pipeline::record_cycle`, feeds the whole deduped
      table into `Simulator::ingest` (older-or-equal fixes are no-ops, so re-feeding is safe and
      only genuinely-refreshed aircraft start a blend), evicts at `DROP_AFTER_S` to keep the fed
      picture bounded and aligned with the sim's own drop horizon, runs `advance_all` at ~60 Hz,
      and publishes each feed. The render thread (`app::window`) now only swaps the latest feed
      at frame start and draws — nothing computes the feed there. The swapped feed's
      `aircraft.len()` replaces the pinned `instances=0` in the frame-stats log; plumbing the
      feed into `Renderer::render` waits for 2.5's glyph pipeline (a dead param on `render`
      otherwise). Clean shutdown signals + joins the worker before the store is torn down.
      **Live-verified** against the owner's real `credentials.json` (2× window-mode runs, Intel
      Arc/DX12): first whole-world OpenSky cycle `tracked=6468 stale=776` → next frame
      `instances=5692` (= `tracked − stale`, the sim's fade/stale gating — the count tracks the
      live feed exactly), steady ~180 fps / 5.5 ms mean (double buffer decouples render from
      production), and a clean `WM_CLOSE` join (`close requested → window closed` in 58 ms).
      `fmt`/`clippy --all-targets -D warnings`/`test --workspace` green — **402 passed, 5
      ignored, 0 failed** (+8: 4 `double_buffer`, 4 `simulation`). DECISION_LOG 2.4b. Next:
      **2.5**, aircraft glyphs.)*
- [x] 2.5 Aircraft glyphs: SDF atlas (6 categories per docs/01), instanced quad pipeline,
      per-instance rotation from heading, altitude-bucket tint attribute (final ramp colors
      may land in M4; buckets wired now).
      *(2026-07-19: implemented — the first item that actually draws a live aircraft. New
      `crates/render/src/glyph_atlas.rs`: docs/01 asks for an "SDF glyph atlas" but there is no
      image/font/asset-loading crate anywhere in this workspace and `render` must stay
      self-contained (no bundled artwork, no network — ADR-002), so the atlas is **generated
      procedurally at startup** rather than fetched or rasterized from an external asset — a
      genuine deviation from the literal word "atlas" implying pre-made art, recorded as a
      judgement call, not re-litigated. Six hand-authored 2D silhouettes (plain point lists,
      evocative not literal: jet swept/delta, turboprop/piston straight-winged with piston's
      wing set forward for a "high wing" read, glider the widest span/thinnest fuselage,
      helicopter a rotor disc unioned with a tail-boom stub via signed-distance `max`, unknown a
      plain dart) are rasterized via ray-casting point-in-polygon + point-to-segment distance
      into a 64×64 `R8Unorm` tile apiece, packed into one static `384×64` strip, uploaded once.
      New `crates/render/src/aircraft.rs`: the CPU-side instance packing (`InstanceRaw`) —
      Mercator-metres position divided by `WEB_MERCATOR_EXTENT_M` (the same pre-normalized plane
      `camera_view_proj`/`basemap::project_point` already operate on), heading in radians,
      category → atlas-tile index, altitude-bucket tint with the stale-fade `alpha` folded into
      `tint.a`. Glyph size is a **constant 20 on-screen pixels** (docs/01's L2 16–24px range),
      derived each frame from the camera's `meters_per_pixel` so it never grows/shrinks with
      zoom. New `crates/render/src/shaders/aircraft.wgsl`: instanced-quad vertex shader
      (clockwise-from-north rotation of the local quad corners — Mercator `+y` and clip-space
      `+y` both point "up/north" with no axis flip between them, so one rotation formula serves
      both geography and screen), fragment shader `smoothstep`s a `fwidth`-derived band around
      the SDF's `0.5` edge value for antialiasing (docs/01's "SDF-derived AA" quality-bar line),
      alpha-blended (unlike the opaque base-map pipelines — both the SDF edge and the stale-fade
      alpha need it). `color.rs` gained `altitude_bucket_tint`/`_table` — the skill's six flat
      hex stops run through the existing `linearize_for_format` helper; per the checklist's own
      parenthetical, these are flat placeholder colors, not the Oklab-interpolated ramp (M4).
      `Renderer::render` signature changed to `render(&mut self, feed: &RenderFeed,
      meters_per_pixel: f64)` (previously a dead parameterless call since 2.4b had nothing to
      draw); `Renderer::new` now builds one shared view-proj `BindGroupLayout` object passed to
      both `build_basemap_resources` and the new `build_aircraft_resources`, so one bind group
      serves every pass's `@group(0)`. `app::window`'s call site updated to pass
      `&self.current_feed`/`camera.meters_per_pixel()`; the existing `instances=` frame-stats
      field is unchanged. **LOD tiers are explicitly out of scope**: docs/01 specifies L0/L1/L2
      zoom tiers with cross-fade, but no M2 checklist item (2.1–2.10) actually implements tier
      switching, and 2.3a already scoped the camera to regional-only — every aircraft in the
      feed draws as one fixed-size L2-style glyph regardless of zoom, with no density-dot or
      small-glyph representation at any distance. This is a real gap the M2 gate (2.10) will
      hit at its L0/L1 visual-QA lines (docs/13 §L2-core's zoom-out-to-globe check) and needs
      its own future milestone item before that gate can honestly pass — flagged here rather
      than silently discovered at the gate. Delegated to the renderer-agent (interrupted mid-task
      by a session API/rate-limit error right after the design was settled and before any file
      was written; resumed the same agent from its transcript via `SendMessage` rather than
      restarting cold, per 2.2b's established precedent), independently re-verified by this
      session rather than trusted: every new/changed file read in full, `cargo fmt --check`/
      `clippy --workspace --all-targets -D warnings`/`test --workspace` re-run fresh — **420
      passed, 5 ignored, 0 failed** (+18 over 2.4b's 402, matching the agent's own count exactly)
      — and a live `cargo run -p look-above` driven independently against the owner's real
      `credentials.json` (Intel Arc/DX12): a whole-world OpenSky cycle (`tracked=13,307`, 4
      credits) rendered distinct, differently-rotated dart glyphs (category is always `Unknown`
      pre-M3 enrichment, as expected) tinted by altitude bucket (cyan/green/amber/violet visible
      across busy regions) over the dark desaturated map, aircraft clearly the brightest things
      on screen; clean `WM_CLOSE` exit (`close requested → window closed`, ~70 ms). Two stray
      extra window instances from this session's own screenshot-scripting attempts (not an app
      bug) were found still running afterward and closed the same way before the scratch
      `look_above.db` was deleted per 1.12/1.13's convention. DECISION_LOG 2.5. Next: **2.6**,
      trails.)*
- [x] 2.6a `core::sim` trail ring buffer: per-`Track` ring buffer of *displayed* positions
      (post-blend, post-clamp — same values the glyph shows), sampled at ≥ 1 Hz, retained 5 min;
      `RenderFeed` gains `trails: Vec<TrailVertex>` (flat centerline samples, altitude-bucket
      per vertex, grouped contiguously by aircraft in the same address-sorted order as
      `aircraft`). No ribbon geometry (offsetting/tapering into a mesh needs the camera's
      `meters_per_pixel`, which `core` doesn't have — that's 2.6b, on the render side, the same
      way 2.5's `glyph_scale_normalized` kept zoom-dependent sizing out of `core`). No I/O, no
      app/render wiring. Unit tests per docs/10 §1.
      *(Split 2026-07-19, self-approved same-session, same shape as every prior M2 item
      (2.1/2.1b, 2.2a/2.2b, 2.3a/2.3b, 2.4a/2.4b): the checklist bundles the pure ring-buffer/
      sampling math with the render-side ribbon tessellation and WGSL pipeline, but those are two
      lanes — nothing on the render side can honestly be written against a `RenderFeed.trails`
      shape that doesn't exist yet, and the ribbon-widening math is inherently render's problem
      (it needs live zoom, which `core::sim` never has). 2.6b is the render half.)*
      *(2026-07-19: implemented — `Track` gained a `VecDeque<TrailSample>` ring buffer
      (`TrailSample { pos, alt_m, alt_known, on_ground, t_s }`, private) and
      `last_trail_sample_s`; two new consts, `TRAIL_DURATION_S`(300) and
      `TRAIL_SAMPLE_INTERVAL_S`(1.0) — the skill's "last 5 min .. sampled at ≥ 1 Hz". Sampling
      happens inside `Track::advance`, throttled to at most one push per `TRAIL_SAMPLE_INTERVAL_S`
      and only when the instance is actually visible this frame (`alpha > 0`) — an aircraft that
      isn't shown has no "displayed position" to record, so a stale-faded gap simply leaves a
      hole in the trail rather than recording a phantom point; eviction (`front().t_s` older than
      `TRAIL_DURATION_S`) runs every call regardless. Recorded from `self.display` (the
      post-blend, post-no-backward-clamp position) rather than the raw fix, per the skill's
      "ring buffer of the last 5 min of *displayed* positions (so trails inherit smoothness)" —
      a teleport's fade-hidden midpoint snap is therefore never recorded either, since sampling
      is gated on `alpha > 0` and the teleport dip can (briefly) reach exactly the same invisible
      state stale-fade uses. New `TrailVertex { icao24, position: MercatorXy, altitude_bucket,
      age_s }` — `age_s` (0 at the aircraft, up to `TRAIL_DURATION_S` at the tail) is what 2.6b's
      render-side ribbon pass will derive width/alpha taper from, kept as a raw age rather than a
      pre-normalized `0..1` fraction so a partially-filled trail (an aircraft tracked < 5 min)
      still taper-maps correctly against the full 5 min scale, not its own shorter history.
      `altitude_bucket` is classified from *that sample's own* recorded altitude/on-ground state,
      not the track's current one — the skill's "colored by the altitude ramp" per vertex, so a
      climbing aircraft's trail shows its actual historical bands, not one repeated current-band
      color. `Simulator::advance_all` now collects `(AircraftInstance, Vec<TrailVertex>)` pairs
      over the `rayon` `par_iter_mut`, sorts the pairs by address (same ordering key as before),
      then splits into `aircraft`/`trails` — trails stay contiguous per aircraft in that same
      sorted order (2.6b's render-side grouping depends on this: a run of same-`icao24`
      `TrailVertex`es with no interleaving is what lets it build one ribbon per aircraft without
      needing an explicit run-length or index buffer in the feed itself). 7 new unit tests:
      sample-interval throttling (advances faster than 1 Hz don't duplicate samples, computing
      each probed time fresh from the base rather than accumulating with `+=`, so the assertion
      doesn't depend on floating-point drift), 5-minute eviction (a sample older than
      `TRAIL_DURATION_S` is dropped, the trail stays bounded), no sampling while invisible (a
      stale-faded gap leaves a real gap — reacquisition adds exactly one new sample, not a
      phantom one for the invisible interval), per-vertex altitude bucket reflecting a sample's
      own historical altitude (a climbing track's oldest trail vertex classifies into a lower
      band than its newest), trail contiguity/order matching the sorted aircraft list, and a
      track past `DROP_AFTER_S` carrying no trail into the feed (same visibility gating as the
      instance itself). Also required dropping `Track`'s `Copy` derive (kept `Clone`) — the new
      `VecDeque<TrailSample>` ring buffer field owns a heap allocation, and nothing in the module
      actually needed to duplicate a whole `Track` by value. `cargo fmt --check`/`clippy
      --workspace --all-targets -D warnings`/`test --workspace` all green — **427 passed, 5
      ignored, 0 failed** (+7 over 2.5's 420, all in `sim.rs`). No live run: pure library math,
      no runtime surface until 2.6b wires a consumer (2.4a's own precedent for the same reason).
      DECISION_LOG 2.6a. Next:
      **2.6b**, the render-side ribbon tessellation + WGSL trail pipeline.)*
- [x] 2.6b Trails render: `render` tessellates each frame's `RenderFeed.trails` into
      triangle-strip ribbons (CPU packing on the render thread, same pattern as 2.5's
      `pack_instance` — the per-vertex perpendicular offset needs the camera's current
      `meters_per_pixel` to keep the taper a constant screen-space width, which only the render
      side has), tapering width (3px → 0.5px) and alpha (0.8 → 0) from front (the aircraft) to
      tail, altitude-ramp colored per vertex from 2.6a's `altitude_bucket`. New trail WGSL
      pipeline, drawn in docs/01's order (map → map lines → **trails** → aircraft → labels → UI —
      before the aircraft glyphs, so a glyph never gets occluded by its own trail). Depends on
      2.6a.
      *(2026-07-19: implemented — new `crates/render/src/trail.rs` (pure, testable ribbon
      tessellation) + `shaders/trail.wgsl` (pass-through vertex/fragment) + a `TrailLayer` in
      `renderer.rs`, mirroring 2.5's `aircraft.rs`/`aircraft.wgsl`/`AircraftLayer` split.
      **CPU triangle list, not a `TriangleStrip` primitive or GPU-instanced segments**: each
      aircraft's contiguous `RenderFeed.trails` run (2.6a's grouping invariant) becomes one
      continuous ribbon — every centerline vertex offset ±half-width along the averaged
      perpendicular, joint vertices *shared* between adjacent segments so there is no gap and no
      double-blended overlap at joints (which would bead on an alpha-blended pass). Width
      `3 px → 0.5 px` and alpha `0.8 → 0` taper linearly over `[0, TRAIL_DURATION_S]` as a pure
      function of each vertex's `age_s`; the half-width is converted to normalized-plane units the
      same "pixels → world metres ÷ extent" way `aircraft::glyph_scale_normalized` is, from the
      camera's live `meters_per_pixel`. Coincident consecutive samples (a stationary/holding
      aircraft) are dropped so no zero-length segment yields a NaN normal; a run collapsing to
      `< 2` distinct points draws nothing. The trail pipeline reuses the shared view-proj
      `@group(0)` `BindGroupLayout` (2.5's) and is alpha-blended like the aircraft pass; the
      per-frame vertex buffer grows exactly like 2.5's instance buffer with a reused scratch
      (ADR-002). Drawn *before* the aircraft glyphs so a glyph is never occluded by its own trail;
      `Renderer::render`'s signature is unchanged (it already carried `feed`/`meters_per_pixel`).
      Done directly, not delegated (all touched files already read this session for 2.6a). 9 new
      unit tests in `render::trail`; `cargo fmt --check`/`clippy --workspace --all-targets -D
      warnings`/`test --workspace` all green — **436 passed, 5 ignored, 0 failed** (+9 over 2.6a's
      427). **Live-verified** against the owner's real `credentials.json` (Intel Arc/DX12,
      `Bgra8UnormSrgb`, 1920×1200): a scripted wheel-zoom over central Europe retargeted the poller
      to a ~187-aircraft region, and the zoomed-in frames showed each altitude-colored dart glyph
      trailing a tapered, altitude-ramp-colored ribbon (thinning/fading to the tail, glyph drawn on
      top), no wgpu validation errors/panics, clean `WM_CLOSE`. **Trails inherit 2.5's flagged LOD
      gap** (constant-3px trails blob at whole-world zoom) plus an unbounded per-frame
      tessellation cost there — both resolve with the same future LOD item, noted in DECISION_LOG
      2.6b. Next: **2.7**, labels.)*
- [x] 2.7a Label content: `core::sim` carries callsign/altitude/speed onto `AircraftInstance` so
      `render` has something to typeset. No glyph atlas, no placement, no collision — that's 2.7b.
      *(Split 2026-07-19, self-approved same-session, same shape as every prior M2 item: the
      checklist bundles the label *content* (callsign/FL/kt — plain per-fix data, no camera
      needed) with *placement and collision* (screen-space, needs the camera `core` deliberately
      doesn't have — 2.3a's own boundary, the same reason 2.6a/2.6b split trail sampling from
      ribbon widening). A second reason forced the split before any render code could be written:
      `AircraftInstance` didn't carry callsign, raw altitude, or ground speed at all — only the
      coarse `altitude_bucket` — so 2.7's render half has no data to typeset without this piece
      landing first.)*
      *(2026-07-19: implemented — `AircraftInstance` gained `callsign: Option<CallSign>`,
      `altitude_ft: Option<f64>`, `ground_speed_kt: Option<f64>`. `Track` gained a `callsign`
      field, sticky across fixes that omit it (a protocol framing gap, not a real loss of
      identity — identification messages arrive separately from position ones, so blanking the
      label on every other poll cycle would be wrong); replaced only when a fix actually carries
      one. `altitude_ft` is `self.display.alt_m` (the same blended value `AltitudeBucket::classify`
      already uses) converted via the existing `FT_PER_M`; `Some(0.0)` while on the ground rather
      than `None` — "0 ft" is real data, not unknown, so `core` doesn't gate it away (2.7b's
      formatting decides whether to actually show it). `ground_speed_kt` is `self.fix.speed_ms`
      (not blended — a label's text doesn't need the position blend's smoothing) through a new
      exact `KT_PER_MS = 3600.0 / 1852.0` constant (the definition of a knot, not a decimal
      approximation). **Dropped `AircraftInstance`'s `Copy` derive** (kept `Clone`) for the same
      reason `Track` dropped it at 2.6a: `callsign` owns a heap allocation; nothing in `core` or
      `render` ever needed to duplicate a whole instance by value, only pass it by reference or
      move it out of `advance_all`'s `rayon` collection — confirmed by grepping every call site
      before making the change, not assumed. **Documented deviation from docs/09**: that contract
      types a `labels: Vec<Label>` field directly on `RenderFeed`, "pre-collision-culled" and
      "built by the interpolation stage" — but collision culling and placement are inherently
      screen-space (need the camera), so they stay `render`'s problem entirely in 2.7b, the same
      way ribbon-widening stayed out of `core` at 2.6a/2.6b; `RenderFeed`'s doc comment now
      records this explicitly rather than silently diverging from the typed contract. Two existing
      `render::aircraft` test fixtures (direct `AircraftInstance { .. }` literals) updated for the
      three new fields. Done directly, not delegated — `sim.rs` was already fully read this
      session establishing the scope call above, so a cold subagent would only re-derive it (2.4a/
      2.6a's own precedent for the same reasoning). 5 new unit tests (content carried onto a first
      sighting; missing callsign/altitude/speed each leave their field `None`; a later fix's blank
      callsign does not clear a previously known one; a later fix's *new* callsign does replace
      it; altitude is still reported while on the ground). `cargo fmt --check`/`clippy --workspace
      --all-targets -D warnings`/`test --workspace` all green — **441 passed, 5 ignored, 0
      failed** (+5 over 2.6b's 436, all in `core`). No live run: pure library data plumbing, no
      renderable surface until 2.7b consumes the new fields (2.4a/2.6a's own precedent for the
      same reason). DECISION_LOG 2.7a. Next: **2.7b**, the render-side text glyph atlas +
      placement + collision culling + leader lines.)*
- [x] 2.7b Labels render: glyph-atlas text (a procedurally generated stroke-font SDF atlas, same
      technique as 2.5's `glyph_atlas.rs` — no font/asset crate exists in this workspace), content
      `CALLSIGN  FLnnn  nnnkt` from 2.7a's new `AircraftInstance` fields (omit unknowns; anonymous
      targets get no label — there is no selection yet to except them into "Unidentified", that
      wiring is 2.8's job), placement right of the glyph flipping left near the viewport edge, CPU
      collision culling with priority (docs/01: selected > speed > proximity to viewport center —
      "selected" has no real signal until 2.8 wires selection through, so treat it as always-false
      until then and flag the gap explicitly rather than silently faking it), leader-line when
      displaced, re-evaluated at ≤ 5 Hz with the skill's >10%-priority-beaten hysteresis so a label
      doesn't flicker. Depends on 2.7a.
      *(2026-07-19: implemented — new `render::label` (pure/testable content formatting, screen-
      space projection, priority, placement geometry, the collision sweep, GPU packing) and
      `render::label_atlas` (a procedurally generated **stroke-font** SDF atlas: exactly the 39
      characters the content format needs — `A`–`Z`, `0`–`9`, space, `k`/`t` — each 2–6 line
      segments over a 3×5 grid, rasterized via the same ray-casting-to-distance-field technique as
      2.5's `glyph_atlas.rs`, generalized to *unsigned* distance-to-nearest-stroke since a stroke
      isn't a closed polygon; `glyph_atlas::distance_to_segment`/`encode_distance` were widened to
      `pub(crate)`/parameterized-on-spread so both atlases share the primitive rather than
      duplicating it). New `label.wgsl` (two tiny pipelines sharing one screen-size uniform: text
      quads with the same SDF antialiasing as `aircraft.wgsl`, plus a leader-line `LineList`,
      mirroring `trail.wgsl`'s pass-through shape). New `LabelLayer` in `renderer.rs`, drawn last
      (after aircraft glyphs, docs/01's order) — the one pass that does *not* share the world
      view-proj bind group, since placement/collision are already screen-pixel space; it also owns
      the cross-frame hysteresis state (`held`/`last_eval_s`/`cached_placements`) the ≤5 Hz
      re-evaluation needs, re-projecting already-shown labels every frame in between ticks so
      motion stays smooth without reallocating text off the throttled path. `Renderer::render`'s
      signature changed from `(&mut self, feed, meters_per_pixel: f64)` to
      `(&mut self, feed, camera: &Camera)` — the label pass needs the camera's `center_m`/
      `width_px`/`height_px`, not just its zoom scalar; the one app call site
      (`crates/app/src/window.rs`) updated to pass `&camera`. Hysteresis is a priority *boost*
      (`× 1.1`) applied only to the currently-held candidate during the collision sort, which
      reduces the skill's ">10%" margin to a plain comparison. `selected` is hardcoded `false` in
      `label_priority` with an explicit doc comment (2.8 wires a real signal in later) rather than
      faking one. **Delegated to the renderer-agent** (glyph/SDF atlases and label drawing are
      squarely its remit, same as 2.5/2.6b); **interrupted mid-task by a session API/rate-limit
      error**, resumed via `SendMessage` from its own transcript rather than restarting cold — the
      same recovery path 2.5/2.2b used. **Independently re-verified, and this pass caught a real
      bug the agent's own headless-only verification couldn't have**: every changed/new file read
      in full, fresh `fmt`/`clippy --all-targets -D warnings`/`test --workspace` re-run (474
      passed, matching the agent's count exactly) — then a **live run against the owner's real
      `credentials.json`** (scripted zoom over Scandinavia/the Baltic, Win32 `mouse_event` wheel
      synthesis + `PrintWindow` capture) showed labels correctly attached to their aircraft and a
      genuine defect: `render::label::build_candidates` labeled *every* aircraft in the feed with
      no on-screen check, so aircraft outside the current viewport (the feed can span a wider
      region than the camera — e.g. right after a zoom, before the poller retargets) got labels
      whose position `placement_geometry`'s edge-clamp then pinned to the viewport border, drawing
      a dense stack of orphaned labels with no glyph anywhere near them (`aircraft.rs` has no such
      CPU-side viewport check of its own — an off-screen glyph simply never rasterizes in clip
      space — so this was new to the label pass, not an existing gap it inherited). **Fixed
      directly** (small, well-scoped, in a file already fully read this session — this session's
      own precedent for not delegating a sub-20-line fix): added `glyph_is_visible` (margin =
      the aircraft glyph's own half-width, so a glyph straddling the exact edge still gets
      labeled) and an early return in `build_candidates`; 3 new tests (an off-screen aircraft
      produces no candidate, an on-screen one does, the margin boundary itself). Re-verified
      green after the fix — **477 passed, 5 ignored, 0 failed** (+36 over 2.7a's 441). **Re-ran
      the live capture after rebuilding the binary with the fix**: the orphaned-label column was
      gone, every visible label sat beside its aircraft, and a cropped/upscaled inspection of a
      dense cluster confirmed the collision sweep itself works as specified — overlapping
      candidates culled entirely (fewer labels than glyphs in the cluster), no shrinking, no
      visible overlap anywhere in the captured frame. Flip-near-edge and leader-line behavior
      verified by the unit tests (`placement_flips_to_the_left_near_the_right_edge`,
      `no_leader_line_when_the_label_is_not_displaced`) rather than hunted for pixel-by-pixel live,
      per the same "unit tests + one confirming live pass" bar 2.6b's ribbon taper was held to.
      Clean `WM_CLOSE` exit both live runs; scratch `look_above.db` deleted after per 1.12/1.13's
      convention. DECISION_LOG 2.7b. Next: **2.1b** (the F3 stats overlay text, unblocked now that
      a text atlas exists) or **2.8** (selection) — both open, neither started.)*
- [x] 2.8a Selection state + hit-test: `AircraftInstance` gains a real `selected` signal (wiring
      the hardcoded-`false` gap 2.7b's `label_priority` left explicitly); cursor click (vs. drag)
      disambiguation in `app::window`; CPU hit-test against the current frame's projected glyph
      positions; the selected `icao24` threaded to the simulation worker so `core::sim` marks the
      right instance. No visuals — that's 2.8b (outline, info card).
      *(Split 2026-07-19, self-approved same-session, same shape as every prior M2 item: the
      checklist bundles *detecting* a selection (input handling, hit-test math, state threading —
      testable with no GPU surface at all) with *drawing its consequences* (a white-outline GPU
      pass, a new text-overlay pipeline) — two lanes, the same content/placement split every
      other 2.x item used. `render::label`'s `label_priority` already left an explicit seam for
      this: `selected` is hardcoded `false` there today with a doc comment pointing at 2.8.)*
      *(2026-07-19: implemented — `core::sim::AircraftInstance` gained `pub selected: bool`;
      `Simulator` gained a private `selected: Option<Icao24>` field and `set_selected(Option
      <Icao24>)`, applied per-track inside `advance_all`'s existing `par_iter_mut` pass (compared
      against each track's own address, no new allocation). New `render::selection` (pure,
      testable): `hit_test(aircraft, cursor_px, camera_center_m, meters_per_pixel, viewport_w,
      viewport_h) -> Option<Icao24>`, a **linear scan** (not the design notes' uniform grid — hit-
      testing runs once per click, not once per frame the way the label pass's collision sweep
      does, so a full scan over even a whole-world feed costs nothing worth optimizing ahead of a
      real cost; recorded as a deliberate deviation) reusing `label::world_to_screen_px` so both
      passes agree on glyph position, picking the nearest candidate within `AIRCRAFT_GLYPH_PX/2 +
      4px` or `None` (a click on open map deselects). `render::label::label_priority` now takes a
      real `selected: bool` parameter (`build_candidates` passes `instance.selected` through)
      instead of the hardcoded `false` 2.7b left with a doc comment pointing here. `app::window`
      gained click-vs-drag disambiguation (`CLICK_MAX_MOVEMENT_PX`=5px, `CLICK_MAX_DURATION`=
      300ms, tracked via new `press_pos`/`press_instant` fields alongside the existing drag
      fields) — on a qualifying release, `App::maybe_select` hit-tests the current feed and
      updates `selected_icao24`, mirrored to the simulation worker over a new `watch::Sender<
      Option<Icao24>>` (`select_tx`/`select_rx`, the same shape as 2.3b's `retarget_tx`); the
      worker re-applies `simulator.set_selected(*select_rx.borrow())` every ~60 Hz iteration
      (cheap `Copy`, simpler than edge-detecting the channel) before its own `advance_all`. Logs
      `selection changed selected=?` on every click (a real diagnostic, not just for this
      session's own verification, since 2.8b's visual feedback doesn't exist yet). Done directly,
      not delegated — every touched file (`sim.rs`, `label.rs`, `window.rs`, `simulation.rs`) was
      already read this session establishing the split, same precedent as every prior M2 item.
      14 new tests (4 `core::sim` selection, 6 `render::selection` hit-test, 2 `render::label`
      selected-priority integration, plus the 2 pre-existing `AircraftInstance`-literal fixtures
      in `aircraft.rs`/`label.rs` updated for the new field) — `cargo fmt --check`/`clippy
      --workspace --all-targets -D warnings`/`test --workspace` all green — **498 passed, 5
      ignored, 0 failed** (+12 over 2.1b's 486). **Live-verified** against the owner's real
      `credentials.json` (four separate window-mode runs, Intel Arc/DX12, whole-world OpenSky):
      confirmed end to end that a real left-click (scripted via Win32 `SetCursorPos`/
      `mouse_event`, window forced topmost to route input reliably regardless of focus) reaches
      `App::maybe_select`, runs the hit-test, and logs a result — `selection changed selected=
      None` on every attempted click at a live aircraft-cluster location, i.e. hit_test correctly
      found nothing within radius at the moment of each click, not a wiring failure (a live
      whole-world OpenSky feed churns heavily between poll cycles — several runs logged
      `dropped≈8700` of `tracked≈9800` between one 8–10 s cycle and the next, so the exact
      aircraft rendered in a screenshot is frequently gone or moved by the time a scripted click
      lands a few seconds later; `hit_test`'s own correctness, including the "a click within
      radius selects" case, is proven directly by its 6 unit tests instead). Also live-confirmed a
      **drag is correctly never read as a click**: a real press→10-step-move→release produced no
      `selection changed` log and did pan the camera (`ingest_poller`'s "retargeted mid-run" log
      showed the bbox actually change), the regression this session's own restructuring of the
      `MouseInput` handler most risked. Clean `WM_CLOSE` exit each run; scratch `look_above.db`
      deleted after per 1.12/1.13's convention. **Found and flagged, not fixed (out of scope for
      this item): a reproducible crash**, twice independently — after roughly 2–2.5 minutes of a
      live whole-world-zoom window-mode run, `wgpu` panics (`Device::create_buffer` validation
      error: the trail vertex buffer requests ~279 MiB, over this adapter's 256 MiB
      `max_buffer_size`). This is the already-flagged 2.5/2.6b LOD gap made concrete: whole-world
      zoom draws a full unbounded 5-minute trail for every one of ~9,800 aircraft with no LOD
      cross-fade to cull it, and that trail geometry keeps growing every frame until the buffer
      overflows — not a selection-path bug (2.8a touches none of `render::trail`/`renderer.rs`'s
      buffer sizing), but real evidence the flagged gap is a crash risk, not just a performance
      concern, and worth prioritizing before the M2 gate (2.10)'s live-run-over-a-busy-hub line.
      DECISION_LOG 2.8a.)*
- [ ] 2.8b Selection render: white outline on the selected aircraft's glyph (GPU), minimal info
      card (callsign/alt/speed/source — enrichment fields arrive in M3; anonymous →
      "Unidentified" already enforced here). Depends on 2.8a.
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
