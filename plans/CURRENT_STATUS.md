# Current Status

> The single source of truth for "where are we". Every session reads this first and updates
> it last. Keep the Now section ≤ 10 lines; move history to the log below.

## Now (updated 2026-07-19)

- **Phase:** **M2 opened at the owner's direction**, M1 gate left at 6/7 (token-refresh line
  open, see below — same shape as M0→M1). Items 2.1, 2.2a, 2.2b, 2.3a, 2.3b, 2.4a, 2.4b, 2.5,
  2.6a, 2.6b done. Plan: [M2_HIGH_FIDELITY_RENDERER.md](M2_HIGH_FIDELITY_RENDERER.md)
- **Next action:** **M2 item 2.7** — labels: glyph-atlas text (callsign + FL + kt), CPU collision
  culling with priority (docs/01), leader-line when displaced. (2.1b — the F3 stats overlay —
  also depends on 2.7's glyph-atlas text and is still open; do 2.7 first.)
- **⚠ Flagged, not yet its own item: LOD tiers.** 2.5 draws every aircraft as one fixed-size
  L2-style glyph at any zoom, and 2.6b draws every aircraft's full trail as a constant-3px ribbon
  at any zoom (which piles into a colored blob at whole-world zoom, plus an unbounded per-frame
  tessellation cost there); no M2 checklist item (2.1–2.10) implements L0/L1 tier switching, and
  docs/13 §L2-core's zoom-out-to-globe check is part of the M2 gate (2.10) — a future milestone
  item for LOD cross-fade (drawing glyphs/trails only at L2) needs to exist before that gate can
  honestly pass. DECISION_LOG 2.5, 2.6b.
- **2.6b landed:** trails render. New `render::trail` (pure, testable ribbon tessellation) +
  `shaders/trail.wgsl` (pass-through) + a `TrailLayer` in `renderer.rs`, mirroring 2.5's
  `aircraft.rs`/`aircraft.wgsl`/`AircraftLayer` split. Each aircraft's contiguous
  `RenderFeed.trails` run (2.6a's grouping) becomes one **continuous CPU triangle-list ribbon** —
  centerline vertices offset ±half-width along the averaged perpendicular, joint vertices *shared*
  between adjacent segments so an alpha-blended pass gets no gap and no double-blended bead. Width
  `3px → 0.5px` and alpha `0.8 → 0` taper as a pure function of each vertex's `age_s`; half-width →
  normalized plane via the camera's live `meters_per_pixel` (same math as 2.5's glyph scale).
  Coincident samples (a stationary aircraft) are dropped so no zero-length segment NaNs; a run
  under 2 distinct points draws nothing. Alpha-blended pipeline reusing 2.5's shared view-proj
  `@group(0)`; per-frame vertex buffer grows like the instance buffer with a reused scratch
  (ADR-002). Drawn *before* the aircraft glyphs (docs/01 order) so a glyph is never occluded by its
  own trail; `Renderer::render`'s signature was already sufficient (unchanged). Done directly, not
  delegated. 9 new `render::trail` tests; fmt/clippy/test green — **436 passed, 5 ignored, 0
  failed** (+9 over 2.6a's 427). **Live-verified** against the owner's real `credentials.json`
  (Intel Arc/DX12, `Bgra8UnormSrgb`, 1920×1200): a scripted wheel-zoom over central Europe
  retargeted the poller to a ~187-aircraft region; the zoomed-in frames showed each altitude-
  colored dart glyph trailing a tapered, altitude-ramp-colored ribbon (thinning/fading toward the
  tail, glyph on top, never occluded), no wgpu validation errors/panics, clean `WM_CLOSE`. ~17
  credits (spent_today→36, far under the cap); scratch `look_above.db` deleted after. Trails
  inherit 2.5's LOD gap (see the flag above). DECISION_LOG 2.6b.
- **2.5 landed:** the aircraft glyph pipeline — the first item to actually draw a live aircraft.
  New `render::glyph_atlas` (a procedurally-generated SDF atlas, 6 categories, 64×64 tiles in one
  static `384×64` strip — no image/font/asset crate exists in this workspace, so the silhouettes
  are hand-authored polygons rasterized via ray-casting, not fetched/loaded art) and
  `render::aircraft` (CPU-side instance packing: Mercator metres → the same pre-normalized plane
  `camera_view_proj`/basemap already use, heading → rotation radians, altitude bucket → tint
  with stale-fade alpha folded in). New `aircraft.wgsl`: instanced quads, per-instance
  clockwise-from-north rotation, `smoothstep`-around-0.5 SDF antialiasing, alpha blended.
  `color.rs` gained flat placeholder altitude-bucket tints (real Oklab ramp is M4, per the
  checklist's own parenthetical). `Renderer::render` gained a real signature
  (`feed: &RenderFeed, meters_per_pixel: f64`) after 2.4b left it a dead parameterless call.
  Glyph size is a constant 20 on-screen px regardless of zoom (LOD tiers are explicitly out of
  scope — see the flagged item above). Delegated to the renderer-agent (interrupted mid-task by
  a session API/rate-limit error before any file was written; resumed via `SendMessage` from its
  transcript, same recovery path as 2.2b's connection-error interruption), independently
  re-verified by this session: every changed/new file read in full, `cargo fmt --check`/
  `clippy --workspace --all-targets -D warnings`/`test --workspace` re-run fresh — **420 passed,
  5 ignored, 0 failed** (+18 over 2.4b's 402), matching the agent's own count. **Live-verified**
  independently against the owner's real `credentials.json` (Intel Arc/DX12): a whole-world
  OpenSky cycle (`tracked=13,307`, 4 credits) rendered distinct, differently-rotated dart glyphs
  (category always `Unknown` pre-M3, as expected) tinted by altitude bucket over the dark map,
  aircraft clearly the brightest things on screen; clean `WM_CLOSE` exit (~70 ms). DECISION_LOG
  2.5.
- **2.4b landed:** the `core::sim` wiring. New `app::simulation` (a dedicated worker thread) +
  `app::double_buffer` (a latest-wins SPSC mailbox). Per ADR-002 the whole merge/interpolate/
  persist side moved *off* the render thread onto the worker: it owns the `SessionTable`/
  `Writer`/batch-receiver, drains poll cycles (shared `pipeline::record_cycle`), feeds the deduped
  table into `Simulator::ingest` (older-or-equal = no-op, so re-feeding is safe), evicts at
  `DROP_AFTER_S` to bound the fed picture, runs `advance_all` at ~60 Hz, and publishes each feed;
  the render thread now only swaps the latest feed at frame start and draws. The swapped feed's
  `aircraft.len()` replaces the pinned `instances=0` (plumbing `&RenderFeed` into
  `Renderer::render` waits for 2.5's glyphs — a dead param otherwise). Clean shutdown signals +
  joins the worker before the store is torn down. **Live-verified** against the owner's real
  `credentials.json` (2× window runs, Intel Arc/DX12): first whole-world OpenSky cycle
  `tracked=6468 stale=776` → next frame `instances=5692` (= `tracked − stale`, the count tracks
  the live feed exactly), steady ~180 fps / 5.5 ms mean (double buffer decouples render from
  production), clean `WM_CLOSE` join (`close requested → window closed` in 58 ms). fmt/clippy/test
  green — **402 passed, 5 ignored, 0 failed** (+8). DECISION_LOG 2.4b.
- **2.4a landed:** `core::sim` — the pure interpolation/dead-reckoning engine (`Simulator`,
  `RenderFeed`, `AircraftInstance`, `AltitudeBucket`). `ingest(states, now_s)` per poll cycle
  starts a correction blend on any newer fix (older/equal ignored, so a re-sent `SessionTable`
  fix doesn't restart a blend); `advance_all(now_s)` per frame is a rayon `par_iter_mut` that
  dead-reckons (Δt clamped `[0, 90 s]`), ease-out geodesic-slerp blends over 2 s with the
  no-backward-along-track invariant, teleport-snaps a > 10 km error over 300 ms, and stale-fades
  (60 s + 5 s, reusing `merge`'s `STALE_AFTER_S`/`DROP_AFTER_S`) — the track lingers invisibly to
  90 s so reacquisition blends rather than pops. All math reuses `core::geo`; all state `f64`/
  `Copy` (render narrows to `f32` at 2.5). Split 2.4 → 2.4a/2.4b first (same shape as every prior
  M2 item). Done directly (geo-math lane's inputs already read this session; a cold subagent
  would only re-derive). 20 new unit tests per docs/10 §1, no live run (no runtime consumer until
  2.4b/2.5). fmt/clippy/test green — **394 passed, 5 ignored, 0 failed** (+19). DECISION_LOG 2.4a.
- **2.3b landed:** viewport→bbox exposed to the poller, and window mode runs the live ingest
  pipeline for the first time. New `core::camera::Camera::viewport_bbox() -> BBox` (clamped so
  an off-world/overflowing viewport still yields a valid bbox — no antimeridian wrap yet, same
  scoping as 2.3a); `ingest::poller`'s constructors now take a `watch::Receiver<RegionQuery>`
  and `run()` races its cadence sleep against a retarget so a new region takes effect on the
  very next cycle, not after waiting out up to 60 s; `app::window` opens the same
  `Writer`/`HttpClient`/`Poller`/ledger-restore pipeline `--headless` does (merge/log/persist
  now shared via `app::pipeline::record_cycle`), seeded from the camera's initial viewport and
  retargeted once the camera settles 2 s on a changed bbox — including on a plain resize, a gap
  this session found in the delegated implementation and fixed directly. Three lane-scoped
  pieces (this session / data-source-agent / renderer-agent, sequential), independently
  re-verified (diffs read in full, fresh fmt/clippy/test — **375 passed, 5 ignored, 0 failed**)
  and live-driven against the owner's real OpenSky credentials: initial whole-world fetch (4
  credits) then five real mid-run retargets with distinct bboxes, source never failed over,
  clean `WM_CLOSE` exit. DECISION_LOG 2.3b.
- **2.3a landed:** the regional camera — new `core::camera::Camera` (pure pan/drag/cursor-
  anchored-zoom/inertia math, no wgpu/winit) plus `render::camera_view_proj` and `app::window`
  wiring (mouse drag/wheel → camera, `camera.update(dt)` once per frame) replace 2.2b's
  placeholder fit-to-window matrix with a real view-proj transform. Scoped to the regional Web
  Mercator camera only — zoom-out is clamped at the "whole world visible, letterboxed" fit
  (there is no L0 globe view yet). Delegated as two sequential, lane-scoped pieces (geo-math-
  agent for `core::camera`, renderer-agent for the render/app wiring against that finished
  API), independently re-verified both times: fmt/clippy/test green (**369 passed, 5 ignored**,
  +20 over 2.2b), every file read in full, and a scripted live Win32 drive (drag, inertia,
  cursor-anchored zoom in/out round-tripping to the same view, resize, clean `WM_CLOSE`) — no
  seams/cracks/distortion at any step. DECISION_LOG 2.3a.
- **2.2b landed:** the base map now actually draws — `crates/render/src/basemap.rs` tessellates
  2.2a's bundled `GeoJSON` (`lyon`: `FillTessellator`/`NonZero` for land, `StrokeTessellator`
  for coastlines) into static GPU buffers once at startup, reusing `core::geo::web_mercator_forward`
  for the projection rather than duplicating it in WGSL. New `basemap.wgsl` shader, a
  placeholder aspect-correcting view-proj uniform (2.3 replaces its contents with the real
  camera), and a land/coastline palette in `color.rs` (`#12161D`/`#2E3742`). Delegated to the
  renderer-agent (a connection error interrupted the first attempt mid-task; resumed the same
  agent from its transcript), independently re-verified: fmt/clippy/test all green (349
  passed, 5 ignored — matches the agent's count), full diff read, `lyon`/`bytemuck` moved into
  `[workspace.dependencies]` to fix a convention deviation, and a live run confirmed a correct
  symmetric world map across three window sizes — after a DPI-awareness bug in the
  verification screenshot script itself briefly looked like a renderer bug (see DECISION_LOG
  2.2b). Render crate: 5 → 12 tests. DECISION_LOG 2.2b.
- **2.2a landed:** the base map's real Natural Earth 1:50m land + coastline data, fetched once
  and bundled as `GeoJSON` in `crates/render/assets/basemap/` (1,421 polygon / 1,429 line
  features, ~2.5 MB combined) — `render` itself never touches the network. New workspace crate
  `crates/import` (`look-above-import`, depended on by nothing) holds the one-off fetch tool;
  docs/03's documented download host (`naturalearthdata.com`) 404s on its own direct links, so
  the tool fetches from Natural Earth's real CDN (`naciscdn.org`), verified live and recorded
  in docs/03. **Item was split 2.2/2.2a/2.2b**: bundling GeoJSON presumes the data exists
  first, and acquiring it (live download, shapefile→GeoJSON conversion, a new network-capable
  crate that must stay out of `render`'s dependency tree) is cleanly separable from the
  tessellation/pipeline work — same shape as 2.1/2.1b. 342 tests green (5 live `#[ignore]`d,
  +10 from 2.2a).
- **2.1 landed:** DX12-preferred backend (env-var bisection still wins), 4x MSAA render-target
  plumbing (adapter-capability-checked, not just assumed), F3-toggled frame-stats mode with
  real p50/p95. **Item was split 2.1/2.1b**: the on-screen text the checklist's "overlay"
  implies needs a glyph atlas that doesn't exist until 2.5/2.7, so 2.1 ships the toggle +
  richer log line now and 2.1b (drawing it on screen) is deferred, tracked explicitly in the
  M2 plan rather than left implicit.
- **M1 gate (2026-07-18, carried forward):** 6 of 7 acceptance §M1 lines pass; the OAuth2
  token auto-refresh line (needs an observed refresh across a > 30 min run) stays **open** —
  owner chose the checklist's literal 10-min scope. Not re-litigated at M2 open; carried the
  same way M0's CI-badge line was carried into M1. Full evidence: M1 plan 1.13,
  DECISION_LOG 1.13.
- **1.13 gate run (2026-07-18):** 10 min 20 s live `look-above --headless` against the
  owner's real `credentials.json`, 98 poll cycles, 0 panics, 0 429s/rate-limit hits, 196/3,200
  credits spent (6.1%, well under the 80% line), dedup and retry/backoff both observed live on
  real data. **6 of 7 acceptance §M1 lines pass**; the seventh (token auto-refresh "observed
  across a > 30 min run") stays **open** — the owner was asked and chose the checklist's
  literal 10-min scope over extending the run to cover it. Full per-line evidence: M1 plan
  1.13, DECISION_LOG 1.13. Same shape as M0's gate: recorded honestly short of a full pass,
  not silently marked done.
- **1.12 headless mode landed:** `app::headless` + `--headless` run the poller, merge, and
  store writer together as one process for the first time, closing 1.7's ledger-restore seam
  and 1.11's writer-wiring gap. `record_error` stays unwired (the poller's channel never
  carries a failure) — carried forward, not an oversight. DECISION_LOG 1.12.
- **Blockers:** the owner must rename the repo `look_above` → `look-above`, then push (no SSH
  key on this machine) — CI has never run; M0's one unmet gate line.
  [NEXT_ACTIONS.md](NEXT_ACTIONS.md) #1.
- **Credit spend to date: ~325+ of 4,000/day on 2026-07-18; a fresh day's ledger on 2026-07-19**
  (UTC-day reset, per `CreditLedger`'s own design) **spent roughly 20–30 more** today: the
  renderer-agent's own live-verification pass before this session's independent re-verification
  (ledger read back `credits_used_today=16` at the start of this session's own run), plus this
  session's run (one whole-world cycle, 4 credits, then failed over to the free `airplaneslive`
  fallback) and two stray extra window instances left running from this session's own
  screenshot-scripting attempts (found and closed, see DECISION_LOG 2.5) that would have polled
  independently for the few minutes they were up — not tallied exactly, but nowhere close to the
  3,200/80% cap either way. 2026-07-18's detail: the ~300+ below, plus 2.4b's two live
  window-mode verification runs that day spending ~24 more — 4 credits/cycle × ~6 whole-world
  OpenSky cycles, against the owner's real credentials. Further detail:
  203 carried from M1's 1.4/1.12/1.13 above, plus
  2.3b's two live-verification runs today: the renderer-agent's own window-mode drive spent at
  least 12 (its report showed 3 cycles before the snippet it quoted cut off, not necessarily its
  final total) and this session's independent re-verification spent 84 more — both against the
  owner's real credentials, all 2026-07-18. Each run's local ledger started fresh because the
  prior run's scratch `look_above.db` had already been deleted per 1.12/1.13's own cleanup
  convention — expected, not a sign the ledger-restore mechanism is broken (it did restore
  correctly within each single run; there was just nothing left to restore *from*). Still well
  under the 3,200 (80%) self-imposed cap either way; flagged here because two independently-
  fresh-started local ledgers on the same real day is exactly the scenario that *could* matter
  once combined runs get large — worth remembering if a future session sees an unexplained gap
  between the local ledger and OpenSky's own account usage.
- **⚠ Carried to M3:** `anonymous` catches only the no-callsign half of privacy rule 2.2 — a
  PIA hex broadcasting a callsign needs FAA range data not yet available. DECISION_LOG 1.4.

## Gate record

| Milestone | Status | Evidence |
|---|---|---|
| M0 | **gate run 2026-07-15 — 6/7; owner opened M1 with the badge line outstanding** | per-line below |
| M1 | **gate run 2026-07-18 — 6/7; token-refresh line open, owner-accepted** | M1 plan 1.13 |
| M2 | **opened 2026-07-18 (owner call, M1 gate left at 6/7)** | — |
| M3–M6 | not started (plan files written at preceding gates) | — |

### M0 acceptance §M0 — evidence (run 2026-07-15, Windows 11, rustc 1.96.0, Intel Arc / Vulkan)

| # | Line | Result | Evidence |
|---|---|---|---|
| 1 | `cargo build --workspace` on a clean clone | **pass** | fresh `git clone` to a scratch dir, cold build: **exit 0 in 66.2 s**. Not the warm tree — a clone is the only thing that can catch a needed-but-uncommitted file. |
| 2 | CI fmt + clippy + tests on push; badge green | **BLOCKED** | no git remote (`git remote -v` empty); `github.com/arcTanMyAngle/look-above` → **HTTP 404** (fetched). Workflow has never executed. Its three commands are green locally on Windows; the Linux job is unproven. |
| 3 | Five crates, direction core ← {ingest, store, render} ← app, no reverse deps | **pass** | full intra-workspace edge list from `cargo metadata`: `ingest`/`store`/`render` → `core`; `app` → all four; **nothing else**. Nothing depends on `app`; the three middle crates don't depend on each other. `core` externals: async-trait, rayon, serde, thiserror (no tokio/reqwest/rusqlite). `render`: no winit, no network, no DB. |
| 4 | `cargo run -p look-above` opens a window, resizes without panic, closes cleanly | **pass** | driven over Win32: window titled "Look Above" (hwnd confirmed), resized 800×600 / 1280×720 / 640×480 / 1024×768, minimized to 0×0 and restored, all alive; `WM_CLOSE` → "close requested" → "window closed"; **exit code 0**; zero panics on stdout/stderr. |
| 5 | Config from `config.toml` + env override; missing file → defaults, not error | **pass** | against the **binary**, not the tests. No file → "no configuration file; using defaults", `look_above.db`, 24 h, credentials "absent". With a file → `from_file.db`, 6 h. With `LOOK_ABOVE_*` → `from_env.db`, 3 h. Env > file > default, observed each time. |
| 6 | `config.toml` gitignored; repo contains `config.example.toml` | **pass** | `git check-ignore -v` hits for `config.toml`, `target/`, `qa/`, `*.db`; `config.toml` untracked and **absent from the clone**; `config.example.toml` tracked and present. |
| 7 | ADRs 001–005 accepted; DECISION_LOG updated | **pass** | docs/02: all five marked `Status: accepted`. DECISION_LOG has a dated entry per item 0.1–0.8. |

Suite at the gate: **87 tests** (51 core, 31 app, 5 render), `fmt`/`clippy --all-targets -D warnings`/`test` all green. No code changed at 0.8; working tree clean afterwards.

## Session log (newest first)

- **2026-07-19** — M2 item 2.6b: trails render — the render-side ribbon tessellation + WGSL
  trail pipeline that consumes 2.6a's `RenderFeed.trails`. New `crates/render/src/trail.rs` (pure,
  testable) tessellates each aircraft's contiguous trail run into one continuous triangle-list
  ribbon: every centerline vertex offset ±half-width along the averaged perpendicular of the local
  travel direction, joint vertices *shared* between adjacent segments (no gap, and — on this
  alpha-blended pass — no double-blended bead at joints). Width `3 px → 0.5 px` and alpha
  `0.8 → 0` taper as a pure function of each vertex's `age_s` over `[0, TRAIL_DURATION_S]`;
  half-width is converted to the normalized `[-1,1]` plane the same "pixels → world metres ÷
  extent" way 2.5's `glyph_scale_normalized` is, using the camera's live `meters_per_pixel`
  (which `core` doesn't have — exactly why 2.6a stopped at flat centerline samples). Coincident
  consecutive samples (a stationary/holding aircraft, which records repeated identical displayed
  positions) are dropped so no zero-length segment produces a NaN normal; a run collapsing to
  under 2 distinct points is a dot, not a ribbon, and draws nothing. New `shaders/trail.wgsl` is
  pass-through (every vertex arrives already offset and colored, so it only applies the shared
  `@group(0)` view-proj and passes the color through); the new `TrailLayer` in `renderer.rs`
  reuses 2.5's shared view-proj `BindGroupLayout`, is alpha-blended like the aircraft pass, and
  grows its per-frame vertex buffer with a reused scratch (ADR-002). Drawn *before* the aircraft
  glyphs (docs/01's map → lines → trails → aircraft → labels order) so a glyph is never occluded
  by its own trail; `Renderer::render`'s signature was already `(&RenderFeed, meters_per_pixel)`
  from 2.5, so no plumbing changed. Done directly, not delegated (every touched file was already
  read this session for 2.6a, per 2.4a/2.6a's precedent). 9 new unit tests, all in `render::trail`
  (both taper curves + clamps, half-width scaling, a straight run widening perpendicular to travel
  with per-vertex half-width, per-vertex bucket/alpha coloring, single-sample and coincident-
  sample runs drawing nothing, per-aircraft run independence, output-buffer reuse). `cargo fmt
  --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` all green — **436
  passed, 5 ignored, 0 failed** (+9 over 2.6a's 427). **Live-verified** against the owner's real
  `credentials.json` (Intel Arc/DX12, `Bgra8UnormSrgb`, 1920×1200): a scripted wheel-zoom anchored
  over central Europe retargeted the poller to a lat 47.7–49.7 / lon 5.6–10.5 bbox (~187 aircraft
  updated each cycle), and the zoomed-in frames showed each altitude-colored dart glyph trailing a
  tapered, altitude-ramp-colored ribbon — cyan/green/amber matching each aircraft's own band,
  thinning and fading toward the tail, glyph drawn on top and never occluded — no wgpu validation
  errors or panics, clean `WM_CLOSE`. ~17 credits (spent_today reached 36, far under the 3,200/80%
  cap); scratch `look_above.db` deleted after per 1.12/1.13's convention. A late capture that
  landed during the `WM_CLOSE` teardown briefly showed the view back at whole-world — a
  capture-timing artifact (the camera/view-proj path was untouched by 2.6b; the retarget log shows
  the camera held the Europe bbox all run), which incidentally illustrates the flagged LOD gap
  trails now inherit: constant-3px ribbons of hundreds of aircraft blob at whole-world zoom, plus
  an unbounded per-frame tessellation cost there — both resolve with the same future LOD item 2.5
  flagged. DECISION_LOG 2.6b. Next: **2.7**, labels.

- **2026-07-19** — M2 item 2.6a: the `core::sim` trail ring buffer. Split 2.6 into 2.6a/2.6b
  first (same shape as every prior M2 item): the checklist bundles the pure ring-buffer/sampling
  math with the render-side ribbon tessellation and WGSL pipeline, but the ribbon-widening math
  needs the camera's live `meters_per_pixel` to keep the taper a constant screen-space width, and
  `core` has no camera (2.3a deliberately kept it in `app`) — so that half is 2.6b's problem, the
  same way 2.5 kept the glyph's zoom-dependent on-screen sizing out of `core` entirely. Each
  `Track` gained a `VecDeque<TrailSample>` ring buffer (private), sampled inside `advance` at ≥
  1 Hz (throttled via a new `last_trail_sample_s`, since `advance` itself runs at render cadence)
  and evicted past a new `TRAIL_DURATION_S` (300 s) — the skill's "last 5 min .. sampled at
  ≥ 1 Hz". Sampling only happens while the instance is actually visible (`alpha > 0`): an
  aircraft not shown this frame has no "displayed position" to record, so a stale-fade gap in the
  trail is real, not filled in. Samples are recorded from `self.display` — the post-blend,
  post-no-backward-clamp position — per the skill's "ring buffer of the last 5 min of *displayed*
  positions (so trails inherit smoothness)". New `RenderFeed.trails: Vec<TrailVertex>`
  (`icao24`, projected `position`, `altitude_bucket` classified from *that sample's own*
  historical altitude/on-ground state — not the track's current one, so a climbing aircraft's
  trail shows its real historical bands — and `age_s`, the raw seconds since recording that
  2.6b will derive width/alpha taper from). `Simulator::advance_all` now collects
  `(AircraftInstance, Vec<TrailVertex>)` pairs over the `rayon` pass, sorts by address, then
  splits into `aircraft`/`trails` — trails stay contiguous per aircraft in that same sorted
  order, which is what lets 2.6b build one ribbon per aircraft without an explicit run-length or
  index in the feed. Dropped `Track`'s `Copy` derive (kept `Clone`): the new ring-buffer field
  owns a heap allocation, and nothing in the module actually copied a whole `Track` by value.
  Done directly, not delegated — this session had already read all of `sim.rs`, `geo.rs`, and
  `types.rs` while orienting on the M2 plan and the visualization skill, so a cold subagent would
  only re-derive them (2.4a's own precedent for the same call). 7 new unit tests (sample-interval
  throttling using freshly-computed probe times rather than accumulated ones, so the assertion
  doesn't ride on floating-point drift; 5-minute eviction; no sampling while invisible, with
  reacquisition adding exactly one new sample rather than a phantom one for the gap; per-vertex
  altitude bucket reflecting a sample's own historical altitude; trail contiguity/order matching
  the sorted aircraft list; a track past `DROP_AFTER_S` carrying no trail into the feed).
  `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` all green
  — **427 passed, 5 ignored, 0 failed** (+7 over 2.5's 420, all in `sim.rs`). No live run: pure
  library math with no runtime surface until 2.6b wires a consumer (2.4a's own precedent). Next:
  **2.6b**, the render-side ribbon tessellation + WGSL trail pipeline. DECISION_LOG 2.6a.

- **2026-07-19** — M2 item 2.5: the aircraft glyph pipeline — the first item to actually draw a
  live aircraft. New `render::glyph_atlas`: docs/01 asks for an "SDF glyph atlas" but no
  image/font/asset-loading crate exists anywhere in this workspace and `render` must stay
  self-contained (ADR-002), so the atlas is **procedurally generated at startup** rather than
  loaded — six hand-authored 2D silhouettes (evocative, not literal: jet swept/delta,
  turboprop/piston straight-winged, glider widest-span/thinnest-fuselage, helicopter a rotor
  disc unioned with a tail-boom stub, unknown a plain dart), each rasterized via ray-casting
  point-in-polygon + point-to-segment distance into a 64×64 `R8Unorm` tile, packed into one
  static `384×64` strip uploaded once — a genuine deviation from the doc's literal wording,
  recorded rather than silently substituted. New `render::aircraft`: CPU-side instance packing
  (Mercator metres ÷ `WEB_MERCATOR_EXTENT_M` — the same pre-normalized plane `camera_view_proj`/
  `basemap::project_point` already use — heading → rotation radians, category → atlas-tile
  index, altitude bucket → tint with the stale-fade `alpha` folded into `tint.a`), the
  clockwise-from-north rotation formula (mirrored exactly in the new `aircraft.wgsl`, since WGSL
  isn't unit-testable), and the constant-20px on-screen glyph scale derived each frame from the
  camera's `meters_per_pixel` (LOD tiers are explicitly out of scope — see below). `aircraft.wgsl`:
  instanced quads, per-instance rotation applied to the local corners, `smoothstep`-around-0.5
  SDF antialiasing, alpha blended (unlike the opaque base-map pipelines). `color.rs` gained
  `altitude_bucket_tint`/`_table` — the skill's six flat hex stops, not the Oklab-interpolated
  ramp (M4, per the checklist's own parenthetical). `Renderer::render` gained a real signature
  (`feed: &RenderFeed, meters_per_pixel: f64`) after 2.4b left it parameterless; `Renderer::new`
  now builds one shared view-proj `BindGroupLayout` handed to both the base-map and aircraft
  pipeline builders so one bind group serves every pass. **LOD tiers are explicitly out of
  scope**: no M2 checklist item (2.1–2.10) implements L0/L1 zoom-tier switching, and docs/13
  §L2-core's zoom-out-to-globe check is part of the M2 gate (2.10) — a future milestone item is
  needed before that gate can honestly pass; flagged now rather than discovered cold at the
  gate. Delegated to the renderer-agent (glyph/SDF atlases are named in its remit), with the
  atlas-generation and LOD-scope calls already made so the agent implemented rather than
  re-decided them; **interrupted mid-task by a session API/rate-limit error** right after the
  design was settled and before any file was written — resumed the same agent via `SendMessage`
  from its own transcript rather than restarting cold, the same recovery path 2.2b used for its
  connection error. **Independently re-verified rather than trusted**: every new/changed file
  read in full by this session, `cargo fmt --check`/`clippy --workspace --all-targets -D
  warnings`/`test --workspace` re-run fresh — **420 passed, 5 ignored, 0 failed** (+18 over
  2.4b's 402: 9 `aircraft.rs`, 5 `glyph_atlas.rs`, 4 new in `color.rs`), matching the agent's own
  count exactly. **Live-verified independently** (not just the agent's own screenshot): a fresh
  `cargo run -p look-above` against the owner's real `credentials.json` (Intel Arc/DX12,
  1920×1200) — a whole-world OpenSky cycle (`tracked=13,307`, 4 credits) rendered distinct,
  differently-rotated dart glyphs (category always `Unknown` pre-M3 enrichment, as expected)
  tinted by altitude bucket (cyan/green/amber/violet visible across busy regions) over the dark
  desaturated map, aircraft clearly the brightest things on screen; a scripted zoom-in attempt
  didn't visibly change the view (a cursor-focus scripting quirk in this session's own
  screenshot tooling, not chased further since the world-view screenshot already proved what 2.5
  needed) and a clean `WM_CLOSE` exit (`close requested → window closed`, ~70 ms). Two stray
  extra window instances turned up afterward from this session's own earlier failed
  screenshot-script launch attempts (not an app bug); closed the same way, then the scratch
  `look_above.db` was deleted per 1.12/1.13's convention. DECISION_LOG 2.5. Next: **2.6**,
  trails (in-memory ring buffer → triangle-strip ribbons).

- **2026-07-18** — M2 item 2.4b: the `core::sim` wiring (double buffer + simulation worker).
  Two new `app` modules: `double_buffer` (a latest-wins single-producer/single-consumer mailbox
  — `Producer::publish` overwrites any unconsumed feed, `Consumer::take_latest` returns `None`
  when nothing is new so the render thread keeps its held front buffer and never blanks) and
  `simulation` (a dedicated `std::thread` running `core::sim` at ~60 Hz). Per ADR-002 and the
  viz skill ("the render thread never computes any of the above"), the whole merge/interpolate/
  persist path moved *off* the render thread: 2.3b had it draining batches inside `draw` (fine
  while nothing consumed the merged table), and 2.4b is where that would start blocking frames,
  so the worker now owns the `SessionTable`/`Writer`/batch-receiver, drains poll cycles through
  the shared `pipeline::record_cycle`, feeds the whole deduped table into `Simulator::ingest`
  (2.4a's older-or-equal guard makes re-feeding the full picture every cycle a safe no-op except
  for genuinely-refreshed aircraft), evicts stale entries at `DROP_AFTER_S` (window-mode only —
  *not* folded into shared `record_cycle`, which would zero headless's documented stale count),
  runs `advance_all`, and publishes each feed. The render thread only swaps at frame start and
  draws. The swapped feed's `aircraft.len()` replaces the pinned `instances=0` in the frame-stats
  log; `&RenderFeed` is *not* yet plumbed into `Renderer::render` (that waits for 2.5's glyph
  pipeline — a dead param on `render` otherwise, the same way the `instances=0` reporting path
  was wired ahead of what it counts at 2.1). Clean shutdown signals an `AtomicBool` and joins the
  worker before the store is torn down (it owns the only window-mode `Writer` clone). Done
  directly, not delegated — the render/ingest/app wiring lanes were all already read this session.
  8 new unit tests (4 double-buffer semantics, 4 the sim wiring: instance count tracks the table,
  eviction removes dropped-out aircraft, re-sync doesn't restart a blend, the two-clock helper
  agrees). **Live-verified** against the owner's real `credentials.json` (2× window runs, Intel
  Arc/DX12, 1920×1200): initial whole-world region → first OpenSky cycle 4 credits,
  `tracked=6468 stale=776` → next frame `instances=5692` (exactly `tracked − stale`, the sim's
  fade/stale gating — the logged count tracks the live feed, not a stale or fabricated number),
  ~180 fps / 5.5 ms mean held steady throughout (the double buffer decouples the render thread
  from the sim thread), and a real `WM_CLOSE` (sent via the process `MainWindowHandle` —
  `FindWindow` by title returned 0 first, a machine-specific scripting quirk like 2.2b's DPI one,
  not an app fault) drove the clean `close requested → window closed` join in 58 ms. ~24 credits
  total across both runs, well under the cap; scratch `look_above.db` deleted after per 1.12/1.13.
  `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` all green
  — **402 passed, 5 ignored, 0 failed** (+8 over 2.4a's 394). DECISION_LOG 2.4b. Next: **2.5**,
  the aircraft glyph pipeline — the first item to actually draw the feed.

- **2026-07-18** — M2 item 2.4a: `core::sim`, the pure interpolation/dead-reckoning engine.
  Split 2.4 into 2.4a/2.4b first (same shape as every prior M2 item): the checklist bundles the
  pure `core` math with the double-buffer handoff and the app-loop wiring that runs it at render
  cadence, but those are two lanes — nothing could be written or tested against an engine that
  didn't exist yet, and nothing visible renders from the feed until 2.5's glyphs regardless. New
  `crates/core/src/sim.rs`: `Simulator` (one `Track` per aircraft), `RenderFeed`,
  `AircraftInstance`, `AltitudeBucket`. Two entry points at two rates — `ingest(states, now_s)`
  per poll cycle (a fix newer than the held one starts a correction blend; older-or-equal is
  ignored, so a re-sent `SessionTable` fix does not restart a blend) and `advance_all(now_s)`
  once per frame, a **rayon `par_iter_mut`** over the track table that dead-reckons, blends,
  fades, and projects to Web Mercator into the flat feed. The math is the
  high-fidelity-flight-visualization skill's, reusing `core::geo` rather than re-deriving:
  dead reckoning with Δt clamped `[0, DROP_AFTER_S]` (tested directly on the private
  `dead_reckon`, since a *visible* aircraft never ages past ~65 s so the clamp is unreachable
  through the fade-gated feed); an ease-out (`1−(1−u)²`) geodesic-slerp blend over a 2 s window
  with heading blended shortest-arc; the **no-backward-along-track invariant** (a step whose
  along-track component is negative clamps back to the previous position — a fix *behind* the
  shown position slows the aircraft to a stop, never reverses it); a **teleport exception**
  (> 10 km) that fades out, snaps at the midpoint while invisible, and fades back in over 300 ms;
  and the **stale fade** reusing `merge`'s `STALE_AFTER_S`(60)/`DROP_AFTER_S`(90) with a new
  `FADE_DURATION_S`(5) — the instance leaves the feed at 65 s but the track lingers (invisible)
  to 90 s so a reacquisition blends rather than pops. `AltitudeBucket` wires the skill's six ramp
  stops (colors are M4); `AircraftInstance.category` is `Unknown` until enrichment (M3/2.5). All
  state is `f64`/`Copy` — the renderer narrows to `f32` at 2.5, so `core` carries no render
  convention; `RenderFeed` is `frame_ts` + address-sorted `aircraft` only, trails/labels appended
  by 2.6/2.7. **Done directly, not delegated** — the geo-math lane's inputs (`geo.rs`,
  `types.rs`, `merge.rs`, `contracts.rs`) were already fully read this session, so a cold
  subagent would only re-derive them (per the token skill's "delegate only when it forces reads
  you'd otherwise skip"). 20 new unit tests, one per docs/10 §1 line plus the pure helpers;
  `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` all green
  — **394 passed, 5 ignored, 0 failed** (+19 over 2.3b's 375). No live run: pure library math
  with no runtime surface until 2.4b/2.5 wire a consumer (the verify skill's own "nothing to
  drive" exception). DECISION_LOG 2.4a. Next: **2.4b**, the double buffer + app-loop wiring.

- **2026-07-18** — M2 item 2.3a: the regional camera. Split 2.3 into 2.3a/2.3b first (same
  shape as 2.1/2.1b and 2.2a/2.2b): the checklist bundles the camera with exposing its viewport
  to the poller, but those are different lanes and 2.3b (a new `ingest::poller` retarget API,
  plus running the live pipeline from window mode for the first time) can't be honestly written
  until 2.3a's camera exists. New `core::camera::Camera` — pure pan/drag/cursor-anchored-zoom/
  inertia math, no wgpu or winit, living in `core` because it's ordinary `f64` arithmetic
  reusable by both `render` and `app` without a new cross-dependency. State is tracked as
  `meters_per_pixel` rather than a unitless zoom level, which turns cursor-anchored zoom into a
  few lines of "solve for the center that keeps one world point's screen position fixed" and the
  zoom-out ceiling into one formula (`2 * WEB_MERCATOR_EXTENT_M / min(width_px, height_px)`) that
  also happens to reproduce 2.2b's placeholder fit-to-window matrix exactly — **deliberately
  clamped there because there is no L0 globe view yet**; zooming out further would show empty
  space with nothing to render into it. "Inertia" was interpreted as pan-momentum-on-release
  (EMA-sampled drag velocity, exponential-friction decay) plus *eased*, not literal-momentum,
  zoom — docs/01's real requirement is smoothness, and no mainstream map keeps zooming after the
  wheel stops. New `render::camera_view_proj` (replacing the deleted `fit_to_window_matrix`)
  re-derives the scale from the camera and re-divides by `WEB_MERCATOR_EXTENT_M` to match
  `basemap.rs`'s pre-normalized static vertices, keeping that division in the one place that
  owns the mesh rather than teaching `core::camera` a render-specific convention.
  `Renderer::set_view_proj` is now the view-proj buffer's only writer (the camera lives in
  `app`, which needs winit events; `render` stays winit-free per ADR-002), and `reconfigure` no
  longer touches that buffer on resize — the app's `Resized` handler calls `Camera::resize` +
  `set_view_proj` synchronously before the next frame, so nothing stale is ever presented.
  **Delegated in two sequential, lane-scoped pieces**: geo-math-agent for `core::camera` first
  (nothing else could be honestly written against an API that didn't exist yet), then
  renderer-agent for the render/app wiring, briefed with the first agent's actual finished
  method signatures. One real ambiguity in this session's own brief was caught and correctly
  resolved by the first agent rather than silently picked one way: the prose said shrinking a
  window must not zoom past the new ceiling, backwards from the actual formula (shrinking
  *raises* the ceiling) — the agent implemented the formula as given (a safe, direction-agnostic
  `.min(max_mpp)` re-clamp) and flagged the prose error explicitly. Both pieces independently
  re-verified by this session, not trusted: `cargo fmt --check`/`clippy --workspace
  --all-targets -D warnings`/`test --workspace` re-run fresh after each (**369 passed, 5
  ignored, 0 failed** — +14 `core::camera` tests, +6 `render` matrix tests over 2.2b's 349),
  every changed/new file read in full. **Live-verified with a scripted Win32 drive**
  (`SetCursorPos`/`mouse_event`/`PostMessage`, DPI-aware per 2.2b's own recorded lesson): a drag
  pan followed the cursor correctly on both axes, inertia coasted a short distance further after
  release and decayed to a stop without reversing, eight wheel notches in then eight back out
  round-tripped to the same view (no drift), a resize reflowed without distortion, and a clean
  `WM_CLOSE` exit (code 0) — six screenshots showed no seams, cracks, or missing polygons at any
  step (docs/13's L2-core pan/zoom-inertia line). DECISION_LOG 2.3a. Next: **2.3b**, viewport→
  bbox exposed to the poller.

- **2026-07-18** — M2 item 2.2b: base map render. New `crates/render/src/basemap.rs`: embeds
  2.2a's bundled `land.geojson`/`coastline.geojson` via `include_str!` (render stays
  network/filesystem-free at runtime, ADR-002), parses with `serde_json` (no new `geojson`
  dependency needed for a shape this simple), projects each `[lon, lat]` through
  `core::geo::web_mercator_forward` — **reused from 0.4, not reimplemented in WGSL** — then
  normalizes by `WEB_MERCATOR_EXTENT_M` into a fixed `[-1,1]`-ish plane baked into the static
  buffers once and for all. Land polygons tessellate via `lyon::FillTessellator` with
  `FillRule::NonZero` (matches RFC 7946's outer-CCW/hole-CW winding, unlike `EvenOdd`'s default,
  which stops working the moment two holes in one feature overlap — proven with a synthetic
  square-with-hole test asserting no triangle centroid lands inside the hole); coastlines via
  `StrokeTessellator`, width `0.0015` in the same normalized space, picked by eye since there is
  no camera/zoom yet to make a screen-space width meaningful (2.3's problem to revisit). New
  `crates/render/src/shaders/basemap.wgsl`: one shared vertex stage reading a `view_proj`
  uniform, one fragment stage reading a per-pass `@group(1)` color uniform — **the view-proj
  matrix is a placeholder aspect-correcting fit-to-window transform** (no pan/zoom exists until
  2.3), rewritten in `Renderer::reconfigure` the same way `msaa_view` already is; 2.3 replaces
  only its *contents*, not the buffer/bind-group plumbing. New land (`#12161D`)/coastline
  (`#2E3742`) constants in `color.rs`, picked the same "ours to fix" way `#0A0E14`'s background
  was at M0 0.6, run through a shared `linearize_for_format` helper. Draw order per docs/01:
  background clear → land fill → coastline stroke, one render pass. **Delegated to the
  renderer-agent; a connection error interrupted the first attempt right after only the
  `Cargo.toml` dependency additions had landed** — resumed the same agent via `SendMessage`
  from its own transcript rather than restarting cold, and it completed the rest correctly.
  This session independently re-verified rather than trusting the report (established
  practice since 2.1): re-ran `cargo fmt --check`/`clippy --workspace --all-targets -D
  warnings`/`test --workspace` fresh — **349 passed, 5 ignored, 0 failed**, matching the
  agent's own reported count exactly this time — read every changed/new file in full, and
  found one real deviation to fix: `lyon`/`bytemuck` had been added as inline-versioned deps
  directly in `crates/render/Cargo.toml` rather than through `[workspace.dependencies]` like
  every other dependency in this repo; moved them, re-verified green. **Independently drove a
  live `cargo run -p look-above` rather than trusting the agent's own screenshots — and this
  caught a real pitfall, just in the verification tooling, not the renderer**: a `PrintWindow`
  capture sized off `GetWindowRect` from a DPI-unaware PowerShell process (1295×837 logical)
  against the renderer's actual 1920×1200 *physical* surface produced an apparently
  asymmetric, cropped map that briefly read as a possible bug in the new aspect-correction
  matrix. `SetThreadDpiAwarenessContext(-4)` in the capture script before any window-geometry
  call fixed it; re-captured at the true physical size and at two more resizes (1600×600 wide,
  500×1000 tall) — every one showed correct symmetric letterboxing, a fully recognizable dark,
  desaturated world map (all continents legible, Mercator's Greenland-exaggeration correctly
  present, confirming the real projection math is running, not a naive linear one), crisp
  coastlines, and a clean `WM_CLOSE` exit each time. Worth remembering alongside M0 0.8's own
  `FindWindow`/`MainWindowHandle` scripting breadcrumb — logged in DECISION_LOG 2.2b for the
  next session that scripts a visual check on this machine. Render crate: 5 → 12 tests; 349
  total. DECISION_LOG 2.2b. Next: **2.3**, the regional camera (real pan/zoom view-proj,
  replacing this item's placeholder).

- **2026-07-18** — M2 item 2.2a: base map data. Split 2.2 into 2.2a (this item — fetch and
  bundle) and 2.2b (tessellate and render), the same shape 2.1/2.1b was split, because
  "bundled as `GeoJSON`" presumes the data exists and acquiring it is its own scoped piece of
  work. **Found the documented download host dead before writing any code**: docs/03 pointed
  at `naturalearthdata.com/downloads/`, but that page's own direct file links 404 — checked
  with `curl -I`, not assumed. The same page links to Natural Earth's real CDN, `naciscdn.org`,
  confirmed live (`200`, ~450 KB per zip); docs/03 updated to record the real host. **New
  workspace crate `crates/import`** (`look-above-import`), depended on by nothing in the `app`
  graph — a network+zip+shapefile stack has no business near `render`'s Cargo.toml, which the
  M0 gate's "no network" check would otherwise start failing. One bin, `import-basemap`:
  downloads the two zips (host-gated exact-match/https-only, mirroring `ingest::allowlist`'s
  rigor), extracts `.shp` bytes in memory (no `.shx`/`.dbf` needed), parses with the pure-Rust
  `shapefile` crate (API confirmed by reading its vendored source, not guessed), and converts
  to `GeoJSON` — land polygons via an outer/inner ring-grouping heuristic (each outer ring
  starts a feature, holes attach to the one immediately before them, matching how real
  shapefile writers order rings), coastline via one `LineString` per shapefile part.
  Coordinates rounded to 1e-4° (~11 m) to keep the bundled text compact; no further geometric
  simplification, since 1:50m is already Natural Earth's own generalized tier. **Run live**:
  1,420 land shapefile records → 1,421 polygon features, 1,428 coastline records → 1,429 line
  features, written to `crates/render/assets/basemap/{land,coastline}.geojson` (~1.2 MB each).
  Verified structurally with a scratch Node script (feature/geometry-type counts, point totals,
  lon/lat extents sane at the expected ±180°/±90° bounds) — no coordinate was ever printed into
  this session (docs/06). 10 new offline unit tests (host gate, coordinate rounding, the
  two-disjoint-outer-rings case, hole attachment, ring-closure survival, polyline part
  splitting); 342 tests total (5 live `#[ignore]`d), fmt/clippy/test all green.
  `crates/render/assets/basemap/README.md` records provenance, format, and the regeneration
  command. DECISION_LOG 2.2a. Next: **2.2b**, `lyon` tessellation + line/fill WGSL pipelines.

- **2026-07-18** — M2 item 2.1: device/queue/surface init, MSAA 4x, F3 stats toggle. Split
  from the checklist's literal wording first (owner-approved via a quick check): "frame-stats
  overlay ... toggled with F3" reads as on-screen text, but no glyph/text pipeline exists
  until 2.5/2.7 — building throwaway text-rendering code now to show four numbers was rejected
  in favor of shipping everything else and tracking the on-screen part as 2.1b, explicit in
  the M2 plan rather than silently deferred. **DX12 preferred on Windows**
  (`Renderer::request_backend`): tries a DX12-only instance first, falls back to wgpu's normal
  multi-backend selection if that adapter request fails, and steps aside entirely if
  `WGPU_BACKEND` is set (the documented bisection path from M0 0.6 still wins). **MSAA 4x**:
  a multisampled color target is created alongside the swapchain, checked against the
  adapter's actual format features first (`RenderError::UnsupportedMsaa` rather than a panic
  on an incapable adapter), rebuilt on every `reconfigure`, resolved onto the swapchain view on
  submit. **F3** (press-edge only) toggles `App::stats_visible`, which widens the existing
  once-a-second frame-stats log from `debug` to `info` and adds real `p50`/`p95` (a new
  per-window sample buffer + nearest-rank `percentile` helper in `frame_stats.rs`, integer
  arithmetic to dodge float-cast clippy lints) plus `instances=0` (pinned until 2.5 gives the
  loop something to count). Delegated to the renderer-agent; this session independently
  re-verified rather than trusting its report, which turned out to matter — **the agent's own
  test count (282) was wrong**, corrected to the real, independently re-run figure: **332
  passed, 5 ignored, 0 failed** (+3 from 2.1). Diff read in full (exactly the 4 files scoped
  were touched), and a live run driven fresh over Win32 confirmed `backend=dx12`, two live
  resizes with the MSAA target rebuilding cleanly and no panic, F3 toggling the log format as
  designed, and a clean `WM_CLOSE` exit. fmt/clippy also re-run clean by this session, not
  assumed from the agent. DECISION_LOG 2.1. Next: **2.2**, the base map.

- **2026-07-18** — **M2 opened at the owner's direction**, told plainly ("continue with M2")
  with M1's gate still at 6/7 (the token-refresh line open per 1.13). No new information since
  1.13's ask — the owner already made that call there ("literal 10-min scope"); this is the
  same decision carried one step further into starting the next milestone, exactly the M0→M1
  precedent. Nothing about the open line changes; it stays carried, not silently dropped.
  Next: **2.1**, `render::gpu` init.

- **2026-07-18** — M1 item 1.13: the gate, run but not fully closed. Found a real conflict
  before running anything: this item's own checklist line says "10-min supervised live run",
  but acceptance §M1's first line needs the OAuth2 token auto-refresh "observed across a
  > 30 min run" — and 1.3's live test never actually watched a second token fetch happen, only
  the refresh-schedule math on one fetched token, so nothing prior covers that line. Asked the
  owner rather than guessing; **owner chose the literal 10-min scope**, accepting that line
  stays open. Ran `look-above --headless` live against the owner's real `credentials.json` for
  10 min 20 s (98 poll cycles, all `source=opensky`): **0 panics, 0 429s/rate-limit hits,
  196/3,200 credits spent (6.1%, well under the 80% cap)**, dedup visibly active in the
  new/updated/dropped counts across cycles, and three real transient network WARNs that
  self-healed via retry/backoff without ever reaching the 3-in-a-row failover threshold — full
  failover-and-recovery itself stays evidenced by 1.8's own dedicated live test rather than
  re-forced here. The aggressive ~5.8 s/cycle cadence (the floor) is the cadence controller
  working as designed: the ledger started fresh late in the UTC day, so `prorated_target`
  spread the whole 3,200-credit budget over a short remaining window — the **hard `can_afford`
  cap**, not the cadence, is what actually protects the 80% line, and didn't need to engage.
  `cargo fmt --check`/`clippy --all-targets -D warnings`/`test --workspace` all green, re-run
  fresh for this gate: **329 passed, 5 ignored, 0 failed** (corrected from 1.12's own entry,
  which stated "334" but whose six per-crate figures actually summed to 329 — noted rather
  than silently fixed, the log is append-only). **Result: 6 of 7 acceptance §M1 lines pass**;
  the token-refresh line is the one open item, carried forward the same way M0 carried its
  badge line. Scratch `look_above.db` and the raw run log were deleted after the numbers were
  recorded here, following 1.12's precedent. DECISION_LOG 1.13. Next: **human review** of the
  open line; M2 waits on that call.

- **2026-07-18** — M1 item 1.12: headless mode. New `app::headless` (`headless::run`) plus a
  `--headless` CLI switch in `main.rs` (`parse_args`/`parse_args_from`; any other argument is a
  hard error, matching `config`'s "a typo must not silently default" call). This is the first
  item that runs M1's pieces together as one live process rather than in isolation under a
  test: `Poller::with_default_chain` feeds a `crossbeam` channel; each `PollBatch` merges into
  a `core::merge::SessionTable`; `store::Writer::record_success` persists the cycle against
  `source_status` — the wiring 1.11 deliberately left open. **Closes 1.7's ledger-restore
  seam**: at startup, `Writer::source_status(OpenSky)`'s `credits_used_today` seeds the
  primary's ledger through `CreditLedger::restored`, so a restart mid-day resumes the day's
  spend rather than believing the budget is fresh — needed a new `Poller::restore_ledger`
  (`ledgers` is private, so nothing outside `ingest` could seed it before this; a no-op on an
  out-of-range index rather than a panic). **The fixed region**: with no camera to drive
  `RegionQuery` yet, headless mode polls a constant ~530×555 km bbox over the Alps
  (44.5–49.5°N, 4.5–11.5°E) — sized to match acceptance §M1's "~500×500 km bbox" credit-budget
  line and landing OpenSky's area pricing in its middle (2-credit) tier, the same airspace
  every adapter's own live test has flown since item 1.4. Per-cycle log carries
  `new`/`updated`/`dropped`/`stale`/`tracked`/`credits_spent`/`spent_today`/`source` — the
  checklist's "new/updated/stale, credits spent" plus what acceptance §M1's dedup and
  credit-budget lines need "observed in logs". **`record_error` is not wired**: the poller's
  channel only ever carries a successful `PollBatch` (1.8: failures are logged internally and
  never reach the channel), so there is no error here to hand `Writer::record_error` —
  extending the poller to surface failures over the channel is a real change, not this item's
  smallest-correct-change scope; carried forward. No graceful shutdown: the gate run (1.13) is
  operator-supervised and stopped with `Ctrl+C`, so a shutdown protocol would be scope the
  checklist does not ask for. 5 new tests (3 CLI parsing, 2 `restore_ledger` on the poller
  side); 334 total, fmt/clippy/test green. **Verified live** against the owner's real
  `credentials.json` (the actual OpenSky OAuth2 path, not just the keyless fallbacks): two
  short runs of the binary itself — 249 aircraft on the first cycle, then 231 updated / 1 new
  / 18 dropped on the second (dedup visibly correct), 2 credits/cycle, 6 of 3,200 spent; the
  second run's startup line read `credits_used_today=4`, confirming the restore round-tripped
  through a real restart. `source_status` writes confirmed by the *absence* of this module's
  own "could not record source_status" warning — what a failed write would have logged.
  Along the way: `Config::credentials()` had carried `#[allow(dead_code)]` and a stale "the
  poller reaches this in item 1.4" comment since 1.3; both are gone now that `headless::run`
  is the real caller. DECISION_LOG 1.12. Next: **1.13**, the M1 gate.

- **2026-07-18** — M1 item 1.11: `store` migrations + writer-thread skeleton. New
  `crates/store` code (the crate's first): `migrations::apply` (numbered, `include_str!`-embedded
  SQL, `PRAGMA user_version`-tracked, idempotent-by-version — each migration's DDL and version
  bump commit together in one `BEGIN IMMEDIATE … COMMIT`) plus migration 0001, which creates
  **only** `aircraft` and `source_status` — verbatim from docs/08, whose other tables
  (`positions`/`flights`/`airports`/`runways`/`airlines`/`metars`) are each tagged with a later
  milestone there and land as their own append-only migrations when needed, not ahead of time.
  `writer::Writer` is the single-writer-thread skeleton docs/08 calls for: a cheap-to-clone
  channel handle over one `Command` enum (`RecordSuccess`/`RecordError`/`SourceStatus`, each with
  its own one-shot reply channel) behind one `crossbeam` `Sender`, with a dedicated OS thread
  owning the sole `rusqlite::Connection` and draining commands until every `Writer` clone is
  dropped. `Writer::open` runs migrations synchronously before spawning the thread, so a broken
  database surfaces as an `Err` to the caller instead of silently killing an unwatched thread.
  **`core::contracts::Store` is deliberately not implemented yet**: its `insert_positions`/
  `airports_in_bbox`/`prune` each need a table (`positions`/`airports`) that doesn't exist until
  M3/M5 migrations land, so implementing the trait now would mean methods that can't work —
  `Writer`'s inherent API is scoped to exactly what 0001 backs, and wiring `Store` for real is a
  future item, noted so it isn't mistaken for an oversight. **Dependency direction verified from
  `Cargo.toml` directly** (not `cargo tree`, per CLAUDE.md): `store` depends on `core` only, so
  `record_success`/`record_error` take plain `SourceId`/`UnixSeconds`/`u32`/`String` — never
  `ingest::poller::PollBatch` — and `source_status` returns a `store`-local `SourceStatus`, never
  `ingest::budget::CreditLedger`. That readback's `credits_used_today` is exactly the `spent`
  argument `CreditLedger::restored(spent, now)` (item 1.7) takes; `restored` already discards a
  stale persisted day on its own, so `store` carries no UTC-day-rollover logic at all — the actual
  restore call is `ingest`/`app` wiring, still to come. **Each verb owns exactly its own
  columns**: `record_success` upserts only `last_success`/`credits_used_today`, `record_error`
  only `last_error`/`last_error_msg`, so a success after a prior error (or vice versa) never
  erases the other — proven both directions. `source` is the table's primary key, so a repeat
  write for one source overwrites rather than duplicating (tested). **Wiring an actual running
  `Writer` from the poller's channel inside `app` is out of scope here** — `app` doesn't consume
  `PollBatch` yet; that starts at 1.12. **The on-disk WAL smoke test is the one place WAL is
  actually checked**: `SQLite`'s `:memory:` can't use WAL at all, so `open_connection` requests it
  unconditionally without asserting it took in the in-memory tests; a dedicated on-disk test
  (temp file, `Drop`-guard cleanup that also removes `-wal`/`-shm`/`-journal` siblings) confirms
  `journal_mode` reads back `wal` for real. Work was delegated to the `storage-agent`; this
  session independently re-ran `cargo fmt --check`/`clippy --workspace --all-targets -D
  warnings`/`test --workspace` rather than taking the agent's word, and read every new file.
  16 new tests (4 on the migration runner + 1 trust-`user_version`-not-a-table-probe edge case, 6
  on the upsert semantics against a raw connection, 5 through the real channel/thread, 1 on-disk
  WAL smoke test). 329 tests total (43 app, 71 core, 180 ingest, 9 `record_fixture` bin, 5
  render, 16 store), 5 live ignored; fmt/clippy/test green. DECISION_LOG 1.11. Next: **1.12**,
  headless mode (`--headless` per-cycle counts — the M1 gate evidence tool, and the first item
  that actually wires a running poller loop to a live `Writer`).

- **2026-07-17** — M1 item 1.10: the fixture recorder. New `scripts/record_fixture.rs`, wired
  as a `[[bin]]` of `ingest` from the repo-root `scripts/` the docs name (out-of-package
  `path`, which Cargo accepts — probed first). It is the recorder docs/06 sanctions and the
  fixture READMEs have promised since 1.4: fetch from an authorized source → trim the record
  array to ≤ 20 → credential-scrub → write to `crates/ingest/tests/fixtures/<source>/`, printing
  only a count and path, **never the payload**. A bin of `ingest` (not a standalone crate)
  because a recording must go out exactly as a poll would — it reuses the allowlist-enforcing
  `HttpClient`, the OpenSky `OAuth2` client, `STATES_ENDPOINT`/the two `POINT_ENDPOINT`s, and
  `point::MAX_RADIUS_NM`. CLI speaks each source's native region shape (OpenSky bbox / readsb
  point+radius), which is what let it avoid a *third* copy of `point`'s covering-circle math —
  the recorded response *shape* is identical either way. OpenSky creds are env-only
  (`LOOK_ABOVE_OPENSKY_*`): reaching `app`'s `config.toml`/`credentials.json` loader would invert
  the crate direction. Scrub is a tripwire (denylist of account-shaped keys) that removes nothing
  from today's anonymous feeds but keeps the tool safe without reading the payload. **Not a
  drop-in re-record**: the crafted `*_nominal` fixtures pin exact values the parser tests assert,
  and `empty`/`nulls`/`malformed` stay hand-authored — the tool refreshes shape and resets after
  a documented source change. `Box<dyn Error>`, not `anyhow` (that stays in `app`). 9 offline
  unit tests (trim/scrub/naming/parse-order), and the **live path exercised** — `adsblol 47 8 73`
  fetched 16 real aircraft over Switzerland, wrote a valid trimmed `{ac, now, …}` file, printed
  only the count; checked structurally (never printing values) and deleted. 313 tests (the 9 in
  the new bin), fmt/clippy/test green. Root README's stale "51 tests / no API client" section and
  all three fixture READMEs updated. DECISION_LOG 1.10. Next: **1.11**, `store` migrations +
  writer thread.

- **2026-07-17** — M1 item 1.9: the cross-source merge. New `core::merge`: `SessionTable` (the
  session's deduplicated live picture — one `StateVector` per `Icao24`, the freshest seen) and
  `MergeStats { new, updated, dropped }`. 20 tests, 304 total (71 core, 180 ingest, 43 app, 5
  render); fmt/clippy/test green. **Dedup is strictly newest-`ts`-wins**: a record replaces the
  held one only when `incoming.ts > stored.ts`; anything not strictly newer — an out-of-order
  late arrival *or* an equal-`ts` duplicate from a second source — is dropped, the same
  time-of-applicability reasoning as 1.4's `time_position` (a slower feed must not drag an
  aircraft back to an older fix). **Sticky anonymity is a one-way latch honored independent of
  `ts`** (privacy 2.2): once any record marks a hex anonymous it stays so for the session with
  `callsign` pinned `None`, even against a *newer, identified* record — and the subtle call is
  that **the latch fires even on a record dropped as stale**, because an anonymity signal is a
  privacy fact, not a position. Insertion enforces the same invariant defensively rather than
  trusting an adapter. **Staleness is tracked here but faded in M2**: `age(now)` (signed, so a
  source clock ahead of us reads negative rather than underflowing), `stale_count(now, max_age)`,
  and `evict_stale(now, max_age)`, with named horizons `STALE_AFTER_S`=60 s and `DROP_AFTER_S`=90
  s pinned to the render skill's "begin fade" / "stop extrapolating" points — the visual fade
  stays render's job. `MergeStats` is exactly the per-batch tally 1.12's new/updated/stale
  readout consumes. Clock-free for merging (dedup/stickiness test in isolation); only staleness
  queries take a `now`. DECISION_LOG 1.9. Next: **1.10**, `scripts/record_fixture.rs`.

- **2026-07-17** — M1 item 1.8: the poller. New `ingest::poller`: `Poller` (the async poll
  loop), `PollBatch` (the `crossbeam` payload — source, states, and the cycle's own
  `credits_spent`/`spent_today` so 1.11/1.12 read cost off the channel, not the private
  ledger), and `WallClock`/`SystemWallClock` (the ledger's *calendar* clock, injected; the
  cadence sleeps + the 5-min probe use tokio's *monotonic* clock — the two-clock split `budget`
  already argued). 18 tests, 284 total. **Failover branches on `is_transient` three ways**
  (a pure, unit-tested `error_response`): transient (`RateLimited`/`Network`/`Server`) retries
  the *same* source with `http::backoff`, failing over only after **3 in a row** (one hiccup
  isn't a dead source); permanent-but-real (`Auth`/`Parse`/`Request`) fails over on the
  *first* (a disabled OpenSky returns `Auth` with no network call and drops straight to the
  keyless fallbacks); **`Refused` holds and idles — never a failover**, because it is *our*
  bug and the next source gets the same wrong question (error.rs already says so). Chain
  advance wraps; the **5-min recovery probe of the primary is the separate, faster path back**.
  **Budget veto = skip, not failover**: a cycle `can_afford` refuses is not fetched (proven by
  an `Arc`-shared scripted source whose `fetch` is asserted *never called*) and the poller
  idles at the ceiling until the UTC-day reset — a rationing primary is not a failed one
  (candidate M4+ improvement noted: serve from free fallbacks when the primary is budget-capped).
  The loop never panics on a wild clock and never crashes on a dead chain (idles + retries);
  only a dropped receiver stops `run`. **Verified live** (`#[ignore]`d, keyless, free): OpenSky
  disabled → failed over → real fallback batch, 0 credits. Next: **1.9**, `core::merge`.

- **2026-07-17** — M1 item 1.7: the credit ledger + cadence controller. New `ingest::budget`:
  `CreditLedger` (an in-memory per-UTC-day credit count that resets itself at the day
  boundary), the pure `poll_interval` / `can_afford` / `prorated_target` / `remaining_budget`
  functions, and `CreditLedger::decide` that bundles them into a `BudgetDecision`. 25 tests,
  267 total; fmt/clippy/test green. **The seam was the first call** (CURRENT_STATUS flagged it):
  the ledger is a small **owned struct, in memory now**, rehydrated from
  `source_status.credits_used_today` at 1.11 via `CreditLedger::restored` — no reach into
  `store`, which does not exist yet. **The number defended is 3,200 = 80% of OpenSky's
  4,000/day** (privacy rule 1.3's margin), never 4,000. **The cadence is even-spread of the
  *remaining* budget over the *remaining* seconds of the UTC day, clamped [5 s, 60 s] — and
  that *is* the pro-rating**: on the pro-rata line it gives the steady ~27 s/credit that just
  fills the day, under budget it shrinks toward the floor, over budget it widens toward the
  ceiling, which is exactly "interval widens as budget tightens". Rejected floor-by-default:
  the 5 s floor at cost 1 is ~5× the daily budget, so it must be the exception (banked budget
  late in the day), not the norm. **Two protections kept separate**: the soft cadence (bounded
  [5,60]) and the hard `can_afford` cap — the ceiling alone can't bound a 4-credit query, so
  the cap is what guarantees rule 1.3, and an exhausted budget idles at the ceiling until the
  midnight reset. **Wall-clock `UnixSeconds`, not the monotonic `Instant`** auth uses: the day
  boundary is a calendar fact, and a clock correction that shifts the day *should* reset the
  ledger. All pure functions — the poller (1.8) drives them. Next: **1.8**, the poller +
  failover chain.

- **2026-07-17** — M1 item 1.6: the adsb.lol adapter. New `ingest::adsb_lol`
  (`AdsbLolSource`), plus `ingest::point` (`PointSource`) — because the second readsb fallback
  showed the shared thing is bigger than 1.5 thought: not just the parser but the whole
  *request* path (bbox → covering circle, 250 nm clamp, pacing, send, bbox-trim), byte-for-byte
  identical between the two services. It moved into `point`, and 1.5's `airplanes_live` was
  refactored to delegate; each adapter is now only its host, `SourceId`, spacing, fixtures, and
  live test. **The design call worth knowing** (DECISION_LOG 1.6): 1.5 wrote the geometry as
  "the adapter's own problem", and that framing did not survive the second adapter — rule of
  two, and two copies of ~65 lines + their tests would fight the same ethos that made
  `readsb`/`normalize`/`pacer` shared. **adsb.lol's spacing mirrors airplanes.live's ≥ 2 s
  though no limit is documented**: privacy 1.3 is "never exceed documented limits", so with
  none published the safe reading is the gentle one, not licence to go faster. Four own fixtures
  + README with identities deliberately distinct from airplanes.live's, so a test can't pass off
  the wrong file. Geometry/URL/trim/global-`Refused` are proven once in `point::tests`; each
  adapter keeps only its own end-to-end/error-mapping/allowlist/live tests. **Verified live**:
  46 aircraft over Switzerland, all inside the bbox, `ts` within the hour, SI ranges — the same
  three beliefs (ms `now`, feet/knots, field names) pinned against adsb.lol *independently*, 0
  credits, `#[ignore]`d. 242 tests (56 core, 138 ingest, 43 app, 5 render); fmt/clippy/test
  green. docs/09's adsb.lol entry gained the shared-`point`/spacing/live-verified detail. Next:
  **1.7**, the credit ledger + cadence controller — which first needs the `store`-vs-now seam
  decided, since `source_status` lands at 1.11.

- **2026-07-17** — M1 item 1.5: the airplanes.live adapter. Four new modules:
  `ingest::readsb` (the shared `{ac: [...]}` parser, parameterized by `SourceId` so 1.6
  drops in), `ingest::airplanes_live` (`AirplanesLiveSource`), `ingest::pacer` (≥ 2 s
  spacing), `ingest::normalize` (`coordinate`/`narrow` lifted out of `opensky::states`).
  37 new tests, 233 total; fmt/clippy/test green. **The headline risk was units, and it is
  the first adapter where that is true**: readsb sends feet/knots/ft-per-min where OpenSky
  sent SI, and a missed conversion produces plausible-looking numbers in the wrong unit —
  so conversion happens at the parse boundary through named constants, and the live test
  asserts ranges an unconverted value cannot pass. **Verified live, keyless, free**: 48
  aircraft over Switzerland (a 73 nm circle around 47°N 8°E), every one inside the
  requested bbox, every `ts` within the hour — which pins the other belief at risk, that
  the API's `now` is epoch *milliseconds* (raw readsb uses seconds; the parser normalizes
  by magnitude). Judgement calls, all in DECISION_LOG: **ts = `now − seen_pos`** (1.4's
  time-of-applicability reasoning); **`~`-hex TIS-B synthetics are skipped**, never minted
  an identity (0.3's `Icao24` strictness paying off); **bbox → covering circle** (midpoint
  center, farthest of the four corners — the sphere makes them unequal — ceil'd, clamped to
  the documented 250 nm with a warn) and **results filtered back to the bbox** so every
  source answers the same question for 1.9's merge; **a global query is `Refused`** rather
  than approximated (M4's problem); **`cost()` = 0** — what this source meters is rate,
  paid in time by the pacer, which lives in the *adapter* because the limit is the
  source's, not a scheduling choice. Pacing is proven under tokio's paused clock
  (`test-util`, dev-only); deliberately not re-proven over wiremock, where the
  auto-advancing clock can fire the 10 s timeout mid-reply. docs/09 and the skill gained
  the units/`seen_pos`/`~`-hex detail — the contract summary had field names but not
  units, and units are the trap. Next: **1.6**, the adsb.lol adapter over the same parser.

- **2026-07-15** — M1 item 1.4: the OpenSky `/states/all` adapter. `ingest::opensky::states` —
  `OpenSkySource` (implements `LiveSource`), positional-array parsing, `credit_cost`. 35 new
  tests, 196 total; fmt/clippy/test green. **The project made its first live *data* request,
  and it is the headline**: every fixture here is hand-written to OpenSky's documented shape,
  so the mocks prove only that we parse what we *believe* they send — and the belief is the
  risky part, because **OpenSky sends lon before lat**, backwards from every other source and
  invisible to the compiler. An `#[ignore]`d live test fetched **72 real aircraft over
  Switzerland and asserted every one falls inside the requested bbox** (swapped, they would be
  near 8°N 47°E — Somalia — and every one would have failed). 20 on the ground, **1 credit of
  4,000** spent, `#[ignore]`d so CI never repeats it. It also asserts *someone* has a callsign
  and *someone* a velocity: reading the wrong indices would otherwise call every optional field
  absent and pass. Field indices are named constants for the same reason. Parsing is per-field
  tolerant, per-record fallible — `states` elements stay `Value` so one non-array record cannot
  fail the batch (docs/10 §2), and losing *every* record logs a **warn**, since that is exactly
  what a shape change looks like and an empty sky does not explain itself. Four judgement calls
  worth knowing, all in DECISION_LOG: **`time_position`, not `last_contact`** (the newer one
  dates a stale fix to now, and M2's dead reckoning would then advance an aircraft from a place
  it had already left); **credit tiers round to the dearer band** (under-pricing overruns the
  allowance rule 1.3 caps, over-pricing only widens the poll interval); **a disabled source
  returns `Auth`** rather than silently dropping to OpenSky's 400-credit anonymous tier, which
  would turn a missing credential into a tenth of the budget with no clue why; and **a global
  query sends no bbox params**, since the endpoint's default *is* the world. **Both of 1.3's
  carry-overs are closed**: `retry_after` now reads a list — standard header, then
  `X-Rate-Limit-Retry-After-Seconds` — taking the first *usable* hint so a bad standard header
  cannot shadow a good vendor one; and `reqwest`'s `query` feature is on. **One gap found and
  carried to M3**: `anonymous` catches only the no-callsign half of privacy 2.2 — a PIA hex
  broadcasting a callsign needs FAA range data we do not have, and the enrichment gate is where
  it binds. Next: **1.5**, the airplanes.live adapter.

- **2026-07-15** — M1 item 1.3: OpenSky OAuth2. `ingest::opensky::auth` — `OpenSkyAuth`
  (token fetch, cache, refresh at 80% TTL, `Ok(None)` when disabled), `Credentials`, an
  injected `Clock`. 35 new tests, 161 total; fmt/clippy/test green. **The project made its
  first live API call**, and it is the headline: every other test here is a mock, which proves
  only that we parse what we *believe* OpenSky sends. An `#[ignore]`d live test proves the
  belief — the real endpoint **accepted the owner's credentials, TTL 1798 s, refresh scheduled
  at 1438 s = 79.98%**, confirming the documented ~30 min and validating the whole schedule
  against reality rather than against my own fixture. It costs no credits (the ledger meters
  `/states/*`, not the token endpoint) and stays `#[ignore]`d so CI never runs it. **The owner
  supplied `credentials.json`** rather than transcribing into `config.toml`; it is gitignored
  (checked untracked and absent from history *before* anything else — nothing leaked) and read
  as-issued, at a new precedence rung below `config.toml`. That file is **all-or-nothing**: if
  either half is configured elsewhere it is ignored entirely, because the two values are issued
  as a pair and mixing halves builds a credential that authenticates as nobody — a 401 that
  neither file explains. **`SecretString` moved to `core::secret`**: `ingest` must hold
  credentials and cannot depend on `app`, and the alternative was privacy rule 7.1 implemented
  twice. **`HttpClient::post_form` is new** — 1.1/1.2 gated `get` only, and the grant is a POST
  carrying the secret, so a bare client for it would have routed the credential straight around
  the allowlist. The 80% refresh is a **retry window, not just a deadline**: a failed refresh
  reuses the still-valid token with a warning, since refreshing early and then hard-failing buys
  nothing over refreshing at 100%. 1.2's tripwire **armed exactly as predicted** and was
  exercised, not assumed: a `flightradar24.com` host planted in `TOKEN_ENDPOINT` failed the scan
  with file, host and remedy named, then reverted. Two things handed to 1.4, both in
  DECISION_LOG: OpenSky's 429 carries **`X-Rate-Limit-Retry-After-Seconds`**, not the standard
  header 1.1 reads, so the backoff floor misses their hint; and reqwest 0.13 keeps **`query`
  behind a feature** (as it did `form`, added here) that the bbox params will need.
  Next: **1.4**, the `/states/all` adapter.

- **2026-07-15** — M1 item 1.2: the host allowlist. `ingest::allowlist` — `AUTHORIZED_HOSTS`
  (the skill's six runtime hosts), `is_authorized_host`, and `HostPolicy`. 19 new tests, 126
  total; fmt/clippy/test green. The item's real decision was that docs/10's spec for it —
  "a const list; test walks all adapter base URLs and asserts membership" — is weaker than it
  reads: there are no adapters until 1.3, so it would **pass over an empty set today**, and it
  could only ever see base URLs an adapter *declared*, not a URL built at a call site. So the
  list is enforced, not merely checked: `HttpClient::get` (1.1's choke point, which every
  adapter must pass through) checks the parsed `Url`, and **so does every redirect hop** —
  reqwest follows 10 by default, so a gate on the outbound URL alone is one `Location` header
  away from irrelevant. Matching is exact, never suffix (`ends_with("opensky-network.org")`
  welcomes `evil-opensky-network.org`; eight such lookalikes are pinned), and **https is part
  of the gate** — an `http://` typo on the token endpoint would send the OAuth2 secret in
  cleartext. **`SourceError::Refused` is new in `core`**, the second extension of docs/09's
  taxonomy after 1.1: `Network` is transient, so a refusal mapped there would retry an
  unauthorized host forever. Static-download hosts (OurAirports, FAA, Natural Earth) are
  deliberately *off* the list — import tooling, not this crate, and `raw.githubusercontent.com`
  serves anyone's repo. The test escape hatch is `#[cfg(test)]`, **not** a cargo feature, since
  feature unification could switch a privacy gate off in a shipped binary. Verified the way 0.8
  did: a `flightradar24.com` const planted in `http.rs` **failed** the scan test with file, host
  and remedy named, then reverted — a tripwire nobody has seen trip is a decoration. It is a
  tripwire, though: the crate has no request URL yet, so it arms itself at 1.3 (hence the
  extractor's own unit test and an assert that the walk visited ≥ 1 file). Two calls to revisit
  if they chafe: a blocked redirect surfaces as the 3xx status mapped to `Refused` (reqwest's
  policy API offers only follow/stop), and `Retry-After`-style HTTP-date parsing stays out.
  Next: **1.3**, which needs the OpenSky account — or 1.5–1.6 without it.

- **2026-07-15** — Repo identity settled. The owner supplied
  `git@github.com:arcTanMyAngle/look_above.git` — an **underscore**, where every doc says
  hyphen. Probed both: `look_above` exists (HTTP 200), `look-above` 404s. That gap is the
  User-Agent we send every aviation source (docs/09), so the URL a source operator would
  follow to identify us points at nothing. Owner chose to **rename the repo to `look-above`**
  over editing the identity in five files — the hyphen already matches the crate names, so a
  rename fixes it with zero code change. `origin` set to the hyphenated URL; **the rename must
  land before the first push**. The push is the owner's: no SSH key exists here
  (`Permission denied (publickey)`), and generating one was declined. Also flagged: the repo
  is **public** while inception recorded "private by default" — nothing sensitive is exposed,
  but the record and reality disagree (NEXT_ACTIONS #1).

- **2026-07-15** — **M1 opened at the owner's direction** with the M0 gate still at 6/7 (the
  badge line needs a push that hasn't happened; nothing about the blocker changed). Then M1
  item 1.1: `ingest::http` — the shared reqwest client (User-Agent + 10 s timeout per docs/09),
  `send_json`, the `SourceError` mapping, and `ingest::http::backoff` (pure `retry_delay`,
  base 5 s → cap 5 min). 20 new tests, 107 total; fmt/clippy/test green. Three calls worth
  knowing about, all in DECISION_LOG. **`SourceError::Request { status }` is new in `core`** —
  docs/09's taxonomy had no non-retryable home for a 400/404, so every existing variant either
  retried a permanent failure forever or swallowed it silently; this extends a doc rather than
  following one. **`Retry-After` is treated as a floor**, `max(header, backoff)`, and honored
  in full even past the 5-min cap — the header means "not before", so waiting longer honors it
  while honoring it *exactly* would drop escalation on repeated 429s. **Equal jitter, not full
  jitter**, because full jitter can retry milliseconds after a 429. Testing followed 0.8's
  habit of checking the artifact, not a proxy: wiremock (docs/10 §2 already required it, pulled
  in early) proves the User-Agent and the timeout on the wire rather than asserting constants
  against themselves. The privacy test caught its own flake before CI could — dropping a
  `MockServer` for a connection failure let a parallel test bind the freed port and answer 404;
  it targets `127.0.0.1:1` now. New deps: `fastrand` (jitter; `rand`'s defaults drag in a
  CSPRNG to smear a retry), `wiremock` (dev). Next: **1.2**, the host allowlist.

- **2026-07-15** — M0 item 0.8: the gate. Ran acceptance §M0 — **6 of 7 lines met**, per-line
  evidence in the table above; no code changed. The gate is recorded as *run*, not passed: the
  badge line needs a remote that doesn't exist (the repo 404s, verified rather than assumed),
  and a gate that certifies its own unverifiable line is worth nothing. Everything else was
  checked against the real artifact rather than a proxy — a fresh clone for the cold build
  (the warm tree cannot prove that line), the running binary for config precedence (the 31 app
  tests already assert the rules; the question was whether the shipped binary obeys them), and
  the live window over Win32 for resize/close (exit 0). Dependency direction came from
  `cargo metadata` edges instead of eyeballing `cargo tree`, which is precisely where a reverse
  edge would hide: the whole graph is seven lines and has none. Two scripting breadcrumbs for
  M2's visual QA, logged in DECISION_LOG: `FindWindow` returns 0 against this app from a
  non-interactive host though the window is real and correctly titled (use `Get-Process` →
  `MainWindowHandle`; this produced one false "no window" scare), and `cargo run` makes the app
  a child, so exit codes must come from a foreground `$LASTEXITCODE`. Next: **human review**;
  M1 does not start until the gate closes.

- **2026-07-15** — M0 item 0.7: CI. `.github/workflows/ci.yml` — one job per OS
  (windows-latest + ubuntu-latest, `fail-fast: false`), each running fmt → clippy → test, plus
  the README badge. The item's real decision was that CI must run *exactly* what CLAUDE.md
  tells a human to run: the two had drifted (0.6 verified with `--all-targets`, the doc didn't
  say so), and CI stricter than the documented check means green locally, red in CI, for
  someone who followed the docs — so `--all-targets` went into both, verified green first.
  Toolchain comes from `rust-toolchain.toml` via a bare `rustup toolchain install` rather than
  a setup action, so the pinned version lives in exactly one place (confirmed against local
  rustup 1.29.0). No apt step on Linux: winit defaults to `wayland-dlopen`, x11/xkbcommon load
  via `dlopen`, and x11-dl's build.rs treats a missing pkg-config entry as `None` — read, not
  assumed — so nothing links a system windowing lib at build time. That also settles the
  "watch at 0.7" note: no test opens a window or requests an adapter, so the GPU-less runner is
  a non-issue. `Swatinem/rust-cache@v2` is the one third-party action (bare `actions/cache` on
  `target/` is the problem it exists to solve); pinned by tag, not SHA — noted in DECISION_LOG
  as a choice. No Rust code changed; 87 tests, fmt/clippy/test green. **The workflow has never
  executed** — there is no remote (see Blockers). Next: 0.8, the gate.

- **2026-07-15** — M0 item 0.6: the window. `render::Renderer` (instance/surface/device +
  background clear) and `app::window` (winit `ApplicationHandler`) and `app::frame_stats`.
  The crate seam is a wgpu trait, not a winit type: `Renderer::new` takes
  `Arc<W: DisplayAndWindowHandle>`, so `render` never depends on winit and the surface can be
  `'static`. `render` stays sync per ADR-005, which is what `pollster` (new dep) buys — wgpu's
  setup calls are async but resolve without yielding on native. Background `#0A0E14` is
  linearized before use: `wgpu::Color` is linear and the surface is `Bgra8UnormSrgb`, so
  passing encoded values through would land near `#3A4351` — a washed-out grey that still
  looks "dark" in a screenshot and quietly breaks the contrast the altitude ramp assumes.
  Transient surface states (`Timeout`/`Occluded`/`Outdated`) are `Skipped`, not errors, and a
  0×0 (minimized) window is never configured — otherwise minimizing kills the app. Four wgpu
  30 API changes vs. every tutorial online (`CurrentSurfaceTexture` enum, `Queue::present`,
  no `InstanceDescriptor::default`, `multiview_mask`) were found by reading the vendored
  source; ADR-003 predicted exactly this churn. 87 tests; fmt/clippy/test green. The window
  has no unit test (needs a real GPU + event loop), so it was exercised by driving the live
  window over Win32: opened titled "Look Above" on Intel Arc/Vulkan, four resizes, minimize
  (0×0) + restore, `WM_CLOSE` → exit 0, stderr empty; a `PrintWindow` capture reads exactly
  `#0A0E14`, confirming the linearization rather than assuming it. Frame stats log at `debug`
  (a line/second would bury the startup lines at the default filter). Next: 0.7.

- **2026-07-15** — M0 item 0.5: `app::config` + `app::logging` — `config.toml` → serde struct,
  `LOOK_ABOVE_*` overrides, tracing init, `config.example.toml`. Precedence env > file >
  default. The item's real decision was the one the plan didn't answer: acceptance §M0 excuses
  a *missing* file, not a broken one, so absence → defaults but a present-but-unparseable file
  (or an unknown key, or retention past the 7-day cap) is a hard error — silent defaults hide
  a typo, and the app then looks fine while running unauthenticated or keeping the wrong
  history. Credentials are a redacted-`Debug` `SecretString` (rule 7.1) and the startup line
  logs only `configured|absent`. Env injected via an `EnvSource` trait because `set_var` is
  `unsafe` in edition 2024. No new deps (`toml` was pinned in 0.2 for exactly this; a small
  `TempDir` avoids `tempfile`). `.gitignore` already covered all four paths — verified, not
  recreated. 26 app tests, 77 workspace; fmt/clippy/test green. Binary exercised beyond the
  tests: no file → defaults, env beats file, broken file → exit 1 with line/column.
  Self-audit caught the environment path violating the very rule the file path enforced:
  `std::env::var(..).ok()` flattens "unset" and "set to non-Unicode" into one `None`, so a
  corrupt secret read as absent. `EnvSource::var` now returns `Result<Option<String>>`;
  verified by spawning the binary with an unpaired surrogate. Next: 0.6.

- **2026-07-15** — M0 item 0.4: `core::geo` — haversine, initial bearing, destination-point
  (the dead-reckoning step), Web Mercator fwd/inv in `EPSG:3857` metres, `LatLon`/`MercatorXy`
  types, lon/bearing normalization. Two radii kept distinct (mean 6371008.8 for great-circle,
  WGS84 6378137.0 for Mercator per its definition). Goldens are analytic arcs + published
  `EPSG:3857` constants, not recalled figures — a remembered LAX→JFK value was wrong and the
  code was right; test now pins the published 2,145 nm. Deferred: orthographic globe (M2, L0
  camera), proptest (deterministic sweep covers docs/10's 1e-9° round-trip), rayon batch
  helpers (M2, with the bench). 28 geo tests, 51 in `core`; fmt/clippy/test green.
  Also rewrote README for a human reader: explains ICAO24/ADS-B/TIS-B/ADS-R/dead reckoning,
  and states plainly that the project needs no receiver hardware. Next: 0.5.

- **2026-07-15** — M0 item 0.3: `core` types + contracts — `core::types` (StateVector, Icao24,
  CallSign, BBox, SourceId, UnixSeconds), `core::error` (SourceError/StoreError, backend-agnostic),
  `core::contracts` (LiveSource, Store, RegionQuery, AircraftMeta, Airport, AirportSize).
  Added `async-trait` 0.1.89 (proc-macro only; needed for a dyn-compatible `LiveSource`).
  Icao24 stores bytes (case-safe Eq/Hash) and rejects readsb `~`-prefixed non-ICAO addresses;
  BBox validates bounds and refuses antimeridian spans. Deferred: `RenderFeed` (M2 shapes),
  serde derives (no consumer yet). 23 unit tests; fmt/clippy/test green. Next: 0.4.

- **2026-07-15** — M0 item 0.2: workspace dependency pins (table + rationale in DECISION_LOG).
  Full `major.minor.patch` + committed Cargo.lock for reproducibility; `=` pins only on
  wgpu 30.0.0 / winit 0.30.13 per ADR-003. winit held at stable 0.30.13 (0.31 is beta).
  Verified wgpu+winit share raw-window-handle 0.6.2, tree is rustls-only (no OpenSSL), and
  SQLite is bundled. Deps wired into owning crates; build/fmt/clippy/test green. Next: 0.3.

- **2026-07-15** — M0 item 0.1: cargo workspace (resolver 3) + five crates
  (core/ingest/store/render/app), workspace lints (clippy all+pedantic, unwrap_used),
  rust-toolchain.toml pinned to 1.96.0, edition 2024 via workspace.package.
  fmt/clippy/test all green. Next: 0.2 (pin deps).

- **2026-07-14** — Repository scaffolded: README/CLAUDE/AGENTS, master prompt, docs 00–13,
  plans (M0–M2, status/decision/risk/next-actions), 7 agents, 3 skills. Initial commit.
  Next: M0 item 0.1.
