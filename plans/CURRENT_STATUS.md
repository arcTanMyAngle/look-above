# Current Status

> The single source of truth for "where are we". Every session reads this first and updates
> it last. Keep the Now section ≤ 10 lines; move history to the log below.

## Now (updated 2026-07-15)

- **Phase:** M0 in progress — `core` done; the app opens a window and clears it (87 tests green).
- **Active milestone:** M0, items 0.1–0.6 done. Plan: [M0_REPO_AUDIT_AND_ARCHITECTURE.md](M0_REPO_AUDIT_AND_ARCHITECTURE.md)
- **Next action:** M0 item 0.7 (CI: GitHub Actions — fmt --check, clippy -D warnings,
  test --workspace on windows-latest + ubuntu-latest).
- **Acceptance §M0:** config (3 lines) and window met, verified live. Left: clean-clone build,
  CI badge, `cargo tree` direction — then the 0.8 gate.
- **Blockers:** none for M0. Before M1 item 1.3 the owner must create a free OpenSky account +
  API client ([NEXT_ACTIONS.md](NEXT_ACTIONS.md) #1); config fields exist and are empty until
  then — absence is a supported state, not an error.
- **Decisions pending:** none — ADRs 001–005 accepted (docs/02).
- **Watch at 0.7:** CI runs on ubuntu-latest with no GPU. Nothing in the suite opens a window
  today, so this should be fine; if a headless runner ever trips on wgpu, that is the cause.

## Gate record

| Milestone | Status | Evidence |
|---|---|---|
| M0 | not started | — |
| M1 | not started | — |
| M2 | not started | — |
| M3–M6 | not started (plan files written at preceding gates) | — |

## Session log (newest first)

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
