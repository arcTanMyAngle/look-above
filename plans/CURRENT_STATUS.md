# Current Status

> The single source of truth for "where are we". Every session reads this first and updates
> it last. Keep the Now section ≤ 10 lines; move history to the log below.

## Now (updated 2026-07-15)

- **Phase:** **M1 open** (owner call, with the M0 gate at 6/7 — see below). Item 1.1 done;
  107 tests green. Plan: [M1_AUTHORIZED_DATA_INGESTION.md](M1_AUTHORIZED_DATA_INGESTION.md)
- **Next action:** **M1 item 1.2** — allowlist const + test (permitted hosts only, docs/10
  §privacy). Needs no account; 1.3 is the first item that does.
- **Blockers:** `origin` is now set, but **the owner must rename the repo `look_above` →
  `look-above` and then push, in that order** — the existing repo has an underscore; the
  hyphen is what the User-Agent and badge use, and it 404s. No SSH key on this machine, so the
  push is the owner's ([NEXT_ACTIONS.md](NEXT_ACTIONS.md) #1). Until then CI has never run —
  M0's one unmet line. **M1 item 1.3 needs the OpenSky account** (#2); 1.2 and the fallback
  adapters (1.5–1.6) proceed without it.
- **Watch at first CI run:** the Linux job is unproven (DECISION_LOG 0.7, "no apt step"), and
  M1 now runs ahead of it — a failure there will surface mid-M1.
- **No live API call has been made yet.** Every ingest test is a local mock; the first request
  to an allowlisted host is item 1.4.

## Gate record

| Milestone | Status | Evidence |
|---|---|---|
| M0 | **gate run 2026-07-15 — 6/7; owner opened M1 with the badge line outstanding** | per-line below |
| M1 | in progress — 1.1 done | — |
| M2 | not started | — |
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
