# Decision Log

Append-only. One dated entry per non-trivial decision; architecture-shaping decisions also
get an ADR in [../docs/02_ARCHITECTURE_DECISION_TEMPLATE.md](../docs/02_ARCHITECTURE_DECISION_TEMPLATE.md).
Format: `date — decision — rationale — (ADR-ref if any)`.

## 2026-07-14 — Project inception decisions (owner Q&A)

- **Language: Rust** over C++ — rayon/wgpu/cargo path is fastest to a safe multithreaded
  native app. (ADR-001)
- **CPU for data, GPU for pixels** — all simulation/geo-math/indexing CPU-parallel; wgpu only
  rasterizes a prepared instance buffer. This is the project's stated parallel-computing goal.
  (ADR-002)
- **wgpu + winit, WGSL** — modern portable graphics on Windows (DX12/Vulkan). (ADR-003)
- **SQLite via rusqlite (bundled)** — zero-admin persistence for enrichment + history. (ADR-004)
- **tokio for I/O only; rayon for compute; crossbeam channels between stages** — no async
  outside `ingest`. (ADR-005)
- **Dual view modes (global + regional) with LOD tiers and hysteresis** — owner chose "both
  modes" explicitly; spec in docs/01.
- **Free data only; OpenSky as primary** (free account, OAuth2, 4k credits/day) with
  airplanes.live / adsb.lol as no-key fallbacks — owner accepts a free signup, pays nothing.
  Allowlist is exhaustive; scraping FR24/FlightAware/ADSBx prohibited (docs/04 §1).
- **Privacy rules adopted as binding** (docs/04): LADD/PIA respected, no re-identification,
  no tail-watching features, history local + capped.
- **Docs-first workflow with milestone gates** — one checklist item per AI session, handoff
  via plans/CURRENT_STATUS.md; model-to-task mapping in docs/12.
- **GitHub: push to `arcTanMyAngle/look-above`** — private by default until owner says otherwise.

## 2026-07-15 — M0 item 0.1 (workspace skeleton)

- **Toolchain pinned to 1.96.0** in `rust-toolchain.toml` (exact stable version, not the
  `stable` channel) — reproducible builds across machines/CI; bumps are deliberate and logged.
  (ADR-001)
- **Clippy lint set** (root `Cargo.toml` `[workspace.lints]`, inherited by all crates via
  `[lints] workspace = true`): `clippy::all` + `clippy::pedantic` at warn (CI runs
  `-D warnings`, so effectively deny); `clippy::unwrap_used = warn` to enforce the
  "no unwrap outside tests" rule. Allowed pedantic exceptions: `module_name_repetitions`,
  `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`, `float_cmp` — noise
  outweighs value for this codebase. Also `unsafe_code = warn` and
  `missing_debug_implementations = warn` at the rustc level.
- **Crate/package naming:** packages `look-above-core/-ingest/-store/-render` in
  `crates/<short-name>/` directories; the binary package is `look-above` (crates/app).
- **Workspace resolver 3**, shared `version`/`edition`/`rust-version`/`license` via
  `[workspace.package]`. Dependency pins deferred to item 0.2 as planned.

## 2026-07-15 — M0 item 0.2 (dependency pins)

Versions pinned in root `[workspace.dependencies]`, inherited by crates via `dep.workspace = true`:

| Crate | Version | Features | Used by |
|---|---|---|---|
| serde | 1.0.228 | derive | core, ingest, app |
| serde_json | 1.0.150 | — | ingest |
| rayon | 1.12.0 | — | core |
| thiserror | 2.0.18 | — | core, ingest, store, render |
| tokio | 1.52.3 | rt-multi-thread, macros, time, sync | ingest, app |
| reqwest | 0.13.4 | json | ingest |
| crossbeam-channel | 0.5.16 | — | ingest, app |
| rusqlite | 0.40.1 | bundled | store |
| wgpu | `=30.0.0` | default | render, app |
| winit | `=0.30.13` | default | app |
| anyhow | 1.0.103 | — | app |
| toml | 1.1.3 | — | app |
| tracing | 0.1.44 | — | ingest, store, render, app |
| tracing-subscriber | 0.3.23 | env-filter | app |

- **"Exact versions" read as: full `major.minor.patch` + committed `Cargo.lock`, not `=` on
  every dep** — the lockfile is what actually makes builds reproducible. Blanket `=` pins are
  actively harmful: any transitive crate needing a semver-compatible patch bump (e.g. serde
  1.0.229) would fail to resolve or duplicate the crate in the tree. `=` is therefore reserved
  for `wgpu`/`winit`, the one pair ADR-003 flags for churn and restricts to milestone-boundary
  upgrades. (ADR-003)
- **winit pinned to 0.30.13 (latest stable), not 0.31.0-beta.2** — 0.31 is the max published
  version but is a prerelease; a foundational dep does not ride a beta. Revisit at a milestone
  boundary once 0.31 is stable.
- **wgpu 30.0.0 + winit 0.30.13 verified compatible**: both resolve to a single
  `raw-window-handle` 0.6.2, which is the interface surface creation goes through — this is the
  classic version-mismatch failure, so it was checked now rather than discovered at item 0.6.
- **reqwest: default features + `json` (no `rustls-tls` flag needed)** — reqwest 0.13 changed
  `default-tls` to mean rustls, so the default is already the rustls stack. Verified no
  `openssl-sys`/`native-tls` anywhere in the tree, so Windows builds need no system OpenSSL.
- **rusqlite `bundled`** — verified `libsqlite3-sys` builds with feature `bundled`, so SQLite is
  compiled in with no system dependency. (ADR-004)
- **`toml` 1.1.3 added beyond the 0.2 checklist** — item 0.5 needs `config.toml` parsing and
  a config format dep belongs with the other pins rather than appearing ad hoc later.
- **tokio features `time` + `sync` added** beyond the checklist's (rt-multi-thread, macros) —
  pollers need interval timers and the token-refresh cache needs a shared lock. (ADR-005)
- **Deps wired into crates now, ahead of their code** (unused until 0.3–0.6) — pinning is only
  meaningful if the graph is proven to resolve and build; a version table nobody compiled is a
  guess. Dependency direction from the plan is respected: `core` takes only serde/rayon/thiserror,
  `render` takes no network/DB deps, winit lives in `app` (item 0.6 owns the window).
- **Verification:** `cargo build --workspace`, `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all green
  on Windows / rustc 1.96.0.

## 2026-07-15 — M0 item 0.3 (core types + contracts)

Shapes taken verbatim from docs/09 where specified; the decisions below are the gaps docs/09
left open. Module layout: `core::types` (vocabulary), `core::error` (taxonomies),
`core::contracts` (traits), all re-exported at the crate root.

- **`async-trait` 0.1.89 added to `[workspace.dependencies]` and to `core`** — beyond the 0.2
  table, because docs/09 specifies `#[async_trait]` on `LiveSource` and 0.2 didn't pin it.
  Native async-fn-in-trait (stable since 1.75) was rejected: it is not dyn-compatible, and the
  poller needs `dyn LiveSource` to hold a failover list of sources. The crate is proc-macro
  only (`proc-macro2`/`quote`/`syn`) — verified via `cargo tree` that it pulls no runtime, so
  `core` keeps its "no I/O deps" rule and ADR-005 (no async outside `ingest`) still holds:
  `core` declares the async seam, `ingest` alone runs it.
- **Error taxonomies are backend-agnostic** — `SourceError`/`StoreError` carry `String`
  messages, not `reqwest::Error`/`rusqlite::Error` sources, since `core` cannot depend on
  either. Implementing crates map their library errors in. `SourceError::is_transient()`
  encodes the docs/09 branch rule (retry `RateLimited`/`Network`/`Server`; never `Auth`, whose
  retry only burns budget, or `Parse`, whose bytes won't change).
- **`StoreError` variants invented** (docs/09 named the type but not its shape):
  `Backend`/`Migration{version}`/`Corrupt`. Minimal set the docs/08 startup path needs.
- **`Icao24` stores `[u8; 3]`, not text** — feeds disagree on hex casing for the same aircraft,
  so bytes make `Eq`/`Hash` case-safe for free; `Display` emits the canonical lower-case hex of
  the `aircraft.icao24` key (docs/08).
- **`Icao24::from_hex` rejects readsb's `~`-prefixed addresses** (TIS-B/ADS-R synthetic
  targets) by being strict about 6 hex digits. Forces each M1 adapter to handle them
  deliberately rather than silently minting an aircraft identity for a non-aircraft.
- **`BBox` is validated + private-field** (`new` checks ±90/±180, ordering, and NaN) and
  **does not model antimeridian wrap** — `lon_min <= lon_max` always holds; a ±180°-spanning
  box must be split by the caller. Global is `RegionQuery { bbox: None }`, never a whole-world
  box, because sources bill global and regional queries differently (docs/09).
- **`SourceId` is a closed enum with `as_str`/`FromStr`** round-tripping the docs/08 spellings
  (`opensky`/`airplaneslive`/`adsblol`) — a new source must add a variant, which forces the
  allowlist test (docs/10) and budget logic to be updated rather than a string slipping through.
- **`AirportSize` ordered `Heliport < Small < Medium < Large`** so `airports_in_bbox`'s
  `min_size` filters as `size >= min_size` (L1 = large+medium, docs/08). Mapping the remaining
  OurAirports types (`seaplane_base`, `balloonport`, `closed`) is deferred to the M3 importer.
- **No serde derives yet** — deferred until a consumer needs them. M1 adapters deserialize
  their own source-shaped DTOs and convert into `StateVector`; `StateVector` itself is never
  parsed from a feed, so a derive now would be a guess at a wire format we don't have.
- **`RenderFeed` (docs/09) deliberately not defined** — item 0.3 doesn't list it, and its
  fields (projected positions, LOD tier, label rects) depend on M2 render decisions.
- **Verification:** fmt/clippy(`-D warnings`, all-targets)/test green on Windows / rustc
  1.96.0; 23 new unit tests in `core`.

## 2026-07-15 — M0 item 0.4 (`core::geo`)

- **Two Earth radii, deliberately not unified** — great-circle math uses the IUGG mean radius
  `6_371_008.8` m; Web Mercator uses the WGS84 semi-major axis `6_378_137.0` m because
  `EPSG:3857` is *defined* on it. Collapsing them to one constant would silently shift every
  projected position by ~0.1%. Both are named consts with the reason on them.
- **Spherical, not ellipsoidal** (no `geographiclib`/Vincenty) — ~0.5% worst-case error against
  WGS84, far below the feeds' own position error, and cheap enough to dead-reckon every
  tracked aircraft per frame. Revisit only if a measurement feature (not a display feature)
  ever needs it.
- **Projection output is `EPSG:3857` metres**, not normalized [0,1] tile space — it is the
  standard definition, so it can be checked against published constants
  (`20037508.342789244`), and the camera can scale metres to clip space in M2 without `core`
  needing to know about viewports.
- **`LatLon` / `MercatorXy` structs rather than `(f64, f64)` tuples** — lat/lon transposition
  is the classic silent bug in geo code: it yields a plausible position elsewhere on Earth
  rather than an error. `LatLon` is unvalidated (feeds are its source; validation belongs at
  the M1 parse boundary, not the hot path), unlike `BBox`, which is camera/config input and
  validates in `new`.
- **Mercator forward implemented as `R·artanh(sin φ)`, not `R·ln(tan(π/4 + φ/2))`** — the two
  are the same function (inverse Gudermannian), but the tan form blows up approaching the
  latitude limit. A test pins the equivalence so an edit to either form must keep them agreeing.
- **Forward projection clamps latitude to ±85.051128779806590° instead of erroring** — the
  projection is undefined only at the poles, and a camera panned to the top of the map should
  show the map's edge, not fail.
- **Golden values are analytic arcs, not recalled table values** — quarter-equator, pole-to-pole,
  antipodal, one meridian degree, plus the published `EPSG:3857` constants. Rationale: a
  "golden" number recalled from memory is not golden. This was not theoretical — the first
  draft asserted LAX→JFK ≈ 3,983 km from memory and failed against the implementation's
  3,974.2 km. The implementation was right (every analytic test passed); the remembered figure
  was the *flight* distance, not the great circle. The test now asserts 2,145 nm, the unit the
  Great Circle Mapper publishes, and is documented as a cross-check rather than the proof.
- **No `proptest` dep; deterministic sweep instead** — docs/10 §1 asks for
  `inverse(forward(p)) ≈ p` within 1e-9°, which a fixed lat/lon grid (>1,000 points, corners
  and limits included) covers reproducibly without a new dev-dependency or a random seed in CI.
  Revisit when `core::sim` lands in M2, where randomized properties earn their keep.
- **Orthographic globe projection deferred to M2** — docs/10 §1 lists it under geo math, but
  plan item 0.4 does not, and it is the L0 camera's projection (docs/01). It lands with the
  camera that needs it.
- **No rayon batch/projection helpers yet** — docs/10 §5 budgets a 10k-point projection batch
  at < 0.5 ms, but a parallel batch API with no caller is a guess at the call shape. Add it in
  M2 alongside the pipeline stage, with the criterion bench.
- **Verification:** fmt/clippy(`-D warnings`, all-targets)/test green on Windows / rustc
  1.96.0; 28 new geo tests (51 in `core` total).

## 2026-07-15 — M0 item 0.5 (config + tracing)

- **Precedence: environment > file > default.** `LOOK_ABOVE_*` beats `config.toml` beats the
  built-in default. Rationale: the environment is the more specific, more immediate context
  (a shell, a CI job, a secrets injector) while the file is the machine's persistent choice;
  the narrower scope should win. Privacy rule 7.1 also names environment variables as a home
  for credentials, which *requires* env to work with no file present and to beat a stale file.
- **A missing `config.toml` yields defaults; a present-but-broken one is a hard error.**
  Acceptance §M0 excuses *absence* only, and the two cases carry different information.
  Absence is unambiguous ("I have no config, use defaults"). A parse failure is evidence of
  intent — the operator meant to configure something and mistyped it. Silently defaulting
  there hides the typo and the app then *looks* fine while running unauthenticated on a
  fallback source, or keeping the wrong amount of history. Only `ErrorKind::NotFound` takes
  the defaults path; every other read failure (permissions, a directory in the way) errors.
  Verified live: a broken file exits 1 with the toml line/column.
- **Unknown keys are rejected** (`deny_unknown_fields`). The same argument one step down: a
  typo'd *key* (`clientid`) is indistinguishable from an absent one, which is exactly how a
  credential goes silently missing. Costs forward-compatibility (an old binary rejects a
  newer file) — acceptable pre-v1, revisit if config ever ships ahead of binaries.
- **Retention above the 7-day cap is rejected, not clamped.** Privacy rule 5.1 says history is
  configurable downward only. Clamping would silently give someone 168 h when they asked for
  720; a warning at load time is also unreliable, since config is read *before* the subscriber
  exists and the warning would be dropped. Erroring needs no logger and cannot be missed.
  `retention_hours = 0` is legal — keeping nothing is the private extreme, not a mistake.
- **Half an `OpenSky` credential is an error**, blank is not. Blank/whitespace credentials
  normalize to `None` ("run on the no-key fallbacks"), so `config.example.toml` copied
  verbatim behaves exactly like having no file — a property the test suite asserts. But
  id-without-secret cannot authenticate and reads as a typo, so it fails loudly. The split
  "id in the file, secret in the environment" is supported and tested.
- **Credentials are `SecretString` with a redacted `Debug`** — privacy rule 7.1 says never in
  logs, and `#[derive(Debug)]` on a config struct is precisely how a secret reaches one. The
  startup line logs `opensky_credentials = configured|absent`, never a value. Regression-tested.
- **No config crate (`figment`/`config`/`clap`).** `toml` was pinned in item 0.2 for this; the
  whole loader is ~5 env keys over a serde struct. No new dependency was added, including for
  tests: a 20-line self-cleaning `TempDir` avoids `tempfile`.
- **Environment is injected via an `EnvSource` trait, not read globally.** `std::env::set_var`
  is `unsafe` in edition 2024 (and the workspace warns on `unsafe_code`), and the environment
  is process-global state that parallel tests race on. Tests pass a `BTreeMap`; `main` passes
  `SystemEnv`. This is why "env override wins" is testable at all.
- **`RUST_LOG` is deliberately not consulted** — `LOOK_ABOVE_LOG_FILTER` is the one variable,
  keeping a single precedence chain. Two variables with their own ordering is a second thing
  to reason about when the logs come out empty.
- **Verification:** fmt/clippy(`-D warnings`, all-targets)/test green; 24 new tests in `app`
  (75 workspace-wide). Beyond the tests, the binary was exercised: no file → defaults + clean
  run; file → values read; env on top → env wins; broken file → exit 1 with line/column;
  over-cap → refused by name; typo'd key → refused. `git check-ignore` confirms `config.toml`
  is ignored (`.gitignore:2`) and `git status` never saw the real one used during testing.

## 2026-07-15 — M0 item 0.5 follow-up (self-audit correction)

- **`EnvSource::var` returns `Result<Option<String>>`, not `Option<String>`.** The first cut
  of this item read the environment with `std::env::var(key).ok()`, which flattens
  `VarError::NotPresent` and `VarError::NotUnicode` into the same `None`. A client secret that
  was *set but not valid Unicode* therefore read as "no credentials", and the app ran on the
  no-key fallbacks without saying why — exactly the present-but-broken-reads-as-absent failure
  the entry above calls unacceptable. The file path honored that principle and the environment
  path silently did not; the inconsistency was in the code while the rationale was in this log
  claiming otherwise. Reachable, not theoretical: the Windows environment is UTF-16 and can
  hold unpaired surrogates. `Ok(None)` now means unset and an `Err` means present-but-unusable.
  Verified by spawning the binary with `OsString::from_wide(&[0xD800])` as the secret: it exits
  1 naming the variable, where before it logged `opensky_credentials=absent` and exited 0. The
  message never echoes the value (rule 7.1) — an error that printed a bad secret to the
  terminal would be its own leak.
- **Env var names are asserted to appear in `config.example.toml`.** That file is the only
  place the `LOOK_ABOVE_*` names are published, so renaming a const without touching it would
  leave the documentation silently wrong — the same class of quiet drift.
- **Verification:** fmt/clippy(`-D warnings`, all-targets)/test green; 26 app tests, 77
  workspace-wide. The `SystemEnv` `NotUnicode` branch itself is covered by the manual spawn
  above rather than a unit test: forcing it in-process needs a non-Unicode variable, and
  `set_var` is `unsafe` in edition 2024. A `#[cfg(windows)]` spawn test could pin it if this
  path ever grows; noted rather than built, since CI (item 0.7) runs Linux too.

## 2026-07-15 — M0 item 0.6 (window + wgpu surface)

- **`render` owns the GPU, `app` owns the window; the seam is a wgpu trait, not winit.**
  `Renderer::new` takes `Arc<W> where W: wgpu::DisplayAndWindowHandle`, so `render` has no
  windowing dependency and the plan's crate description ("wgpu pipelines … no network, no DB")
  stays literally true. The `Arc` is what makes the surface `'static`: it borrows the window
  for as long as it draws to it. `app` keeps the event loop, per ADR-005.
- **`render` stays sync; `pollster` 1.0.1 added to make that possible.** wgpu's
  `request_adapter`/`request_device` are `async`, and ADR-005 says "no async in core/render
  crates at all". The alternatives were to make `Renderer::new` async (violates the ADR and
  drags a runtime into a crate that needs none) or to hand the futures to `app`'s tokio
  runtime (leaks GPU setup into the async half of the app for no gain). On native these two
  futures resolve without ever yielding, so blocking on them costs nothing. `pollster` is a
  ~100-line executor with no dependencies. New dep — recorded here per the 0.2 pin policy.
- **Background is `#0A0E14`, authored in sRGB and linearized before use.** docs/01 fixes the
  intent ("dark, desaturated, aircraft are the brightest things on screen") but not a shade,
  so the value is ours. The non-obvious part is the conversion: `wgpu::Color` is *linear*, the
  surface here is `Bgra8UnormSrgb`, and handing encoded values straight over gets them
  brightened a second time by the hardware — `#0A0E14` would land near `#3A4351`, a washed-out
  grey that would have read as "some dark colour, near enough" and quietly broken the
  contrast the altitude ramp is designed against. `color::clear_color` linearizes only when
  `format.is_srgb()`, so a non-sRGB surface still gets what was authored. Verified by
  capturing the live window with `PrintWindow`: pixels read exactly `#0A0E14`.
- **`PowerPreference::HighPerformance`.** Picks the discrete GPU where there is one and falls
  back to integrated where there is not, so it costs nothing on the integrated-only machines
  docs/01's frame budget assumes. Revisit at M2 if it turns out to matter for battery.
- **Transient surface states are not errors.** `Timeout`/`Occluded` (and `Outdated`, after a
  reconfigure) return `FrameOutcome::Skipped`; only `Lost`/`Validation` are `RenderError`.
  A minimized window on Windows reports a 0×0 size, which is invalid to configure, so
  `resize` ignores zero and `render` skips the frames until it comes back — otherwise
  minimizing the window would kill the app. `Suboptimal` draws the frame and reconfigures
  *after* presenting, because `Surface::configure` panics while a surface texture is alive.
- **Frame stats log at `debug`, not `info`.** A line every second at the default filter
  (`look_above=info,warn`) would bury the startup lines it sits next to. Seen with
  `LOOK_ABOVE_LOG_FILTER=look_above=debug`. `FrameStats::record` takes `Instant` as an
  argument rather than reading the clock, so the reporting logic is unit-tested without
  sleeping. It reports mean *and* worst: the mean alone hides exactly the stutter M2's
  p95 budget (docs/11 §M2) cares about. This is the stub the item asks for — M2 replaces it
  with the on-screen overlay.
- **wgpu 30 API notes (for the next person who reads a tutorial written against 0.19):**
  `get_current_texture` returns a `CurrentSurfaceTexture` enum, not `Result<_, SurfaceError>`;
  presenting is `Queue::present(frame)`; `InstanceDescriptor` has no `Default` and needs
  `new_without_display_handle_from_env()` (the `_from_env` form keeps `WGPU_BACKEND` working
  for bisecting a backend bug); `RenderPassDescriptor` gained `multiview_mask`. All four were
  found by reading the vendored source, not by recall — ADR-003 predicted this churn.
- **Verification:** fmt/clippy(`-D warnings`, all-targets)/test green; 87 tests (5 new in
  `render`, 5 in `app::frame_stats`). The window itself has no unit test — it needs a real
  GPU and a real event loop — so acceptance §M0's window line was exercised by driving the
  live window over Win32 from PowerShell: opened titled "Look Above" on Intel Arc / Vulkan
  (`Bgra8UnormSrgb`), survived four resizes and a minimize (0×0) / restore, and exited 0 on
  `WM_CLOSE` with an empty stderr. Scripts are in the session scratchpad, not committed:
  they are throwaway harnesses, and the headless smoke test that belongs in the repo is
  M2's (docs/10). Frame pacing is uncapped (~1700–2300 fps on a 1280×800 clear), which is
  expected under `ControlFlow::Poll` with no vsync-bound content yet; the 60 fps budget is
  an M2 measurement against real traffic, not this.

## 2026-07-15 — M0 item 0.7: CI (GitHub Actions)

- **One matrix job, not a fmt/clippy/test fan-out.** `.github/workflows/ci.yml` runs the three
  commands in sequence on `windows-latest` and `ubuntu-latest` (`fail-fast: false` — Windows is
  the primary target and a Linux failure must not mask a Windows one). Splitting them into
  parallel jobs would triple the compile cost for a workspace this size to save a minute of
  wall clock; revisit if CI ever gets slow enough to notice.
- **CI runs exactly what CLAUDE.md tells a human to run.** The two had drifted: CLAUDE.md said
  `cargo clippy --workspace -- -D warnings`, but item 0.6 actually verified with
  `--all-targets` (which lints test code too). Rather than let CI be stricter than the
  documented check — the failure mode being green locally, red in CI, for someone who followed
  the docs — `--all-targets` went into CLAUDE.md and the workflow together. Verified green
  before the doc changed.
- **Toolchain comes from `rust-toolchain.toml`, not a setup action.** The step is a bare
  `rustup toolchain install`, which since rustup 1.28 reads the file with no arguments and
  installs the pinned channel plus its `components` (rustfmt, clippy). A `dtolnay/rust-toolchain`
  step would name a version in a second place and let CI silently test a toolchain the repo
  isn't pinned to. Confirmed against local rustup 1.29.0: no-arg install resolves 1.96.0 and
  says it is "overridden by rust-toolchain.toml".
- **No apt step on Linux, deliberately.** Every winit/wgpu tutorial's CI installs
  `libx11-dev`/`libwayland-dev`, and it is dead weight here: winit's default features include
  `wayland-dlopen`, `xkbcommon-dl` and `x11-dl` load through `dlopen`, and x11-dl's build script
  treats a missing pkg-config entry as `None` rather than failing (read the build.rs, didn't
  assume). Nothing links a system windowing library at build time. The runtime question is moot
  because no test opens a window or requests an adapter — `Renderer::new` is the only GPU entry
  point and nothing calls it under `cargo test` (this is the "watch at 0.7" CURRENT_STATUS
  flagged; it resolves to a non-issue). If a Linux job ever fails on a missing `.so`, this is
  the paragraph that was wrong.
- **`Swatinem/rust-cache@v2` is the only third-party action.** Without it each job rebuilds
  wgpu + winit + bundled SQLite from scratch (minutes, every push, twice). `actions/cache` alone
  is not a substitute — caching `target/` naively grows unbounded and restores stale artifacts,
  which is the problem that action exists to solve. Pinned to the major tag, not a SHA; that is
  a looser posture than this repo takes with cargo deps, and if it starts to matter the fix is a
  SHA pin, recorded here so the inconsistency is a choice and not an oversight.
- **Badge added to README pointing at `arcTanMyAngle/look-above`.** Not a guess: docs/09 and the
  authorized-sources skill already fix that URL as the project's identity in the outgoing
  User-Agent. **It will 404 until the owner creates the remote and pushes — there is no git
  remote today** (NEXT_ACTIONS #1). Acceptance §M0's "CI runs on push; badge green" is therefore
  the one M0 line the 0.8 gate cannot check locally; the workflow is verified as far as it can
  be offline (YAML parses, the three commands are green on Windows, the toolchain step resolves).
- **Verification:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` all green locally — 87 tests (51 core, 31 app, 5 render), unchanged
  by this item, which adds no Rust code.

## 2026-07-15 — M1 opened with the M0 gate at 6/7 (owner call)

- **M0 closes with the badge line outstanding.** The owner directed "continue to M1" while the
  0.8 gate stands at six of seven acceptance lines, the seventh being "CI runs on push; badge
  green" — still unmeetable, still for the same reason (no git remote; NEXT_ACTIONS #1). This
  is recorded as a decision rather than a silent transition because CLAUDE.md says not to start
  a milestone at an open gate unprompted, and this was prompted. Nothing about the blocker
  changed; the risk carried forward is that the Linux CI job has never executed, so the first
  push may surface a failure attributable to M0 work while M1 is already underway.

## 2026-07-15 — M1 item 1.1: the shared HTTP client

- **`SourceError::Request { status }` added — docs/09's taxonomy was incomplete.** The listed
  variants are `{Auth, RateLimited, Network, Parse, Server}`, and a plain 400/404/410 fits none
  of them: `Auth` is a lie, `Server` means 5xx *and* is transient, and `Parse` is documented as
  non-fatal "log and skip". Every option therefore either retries a permanent failure forever
  or swallows it silently — a 404 from a moved endpoint would be invisible. The new variant is
  non-transient, so the poller fails over instead of burning budget on our own bug. This
  extends a doc rather than following it, which is why it is here.
- **`Retry-After` is a floor, not an appointment: `max(header, jittered_backoff)`.** The header
  means "not before", so waiting longer always honors it and waiting less never does. Honoring
  it *exactly* would pin us to the server's suggestion and drop the escalation on repeated
  429s — a source answering `Retry-After: 1` would have us back once a second indefinitely.
  A `Retry-After` beyond the 5-min cap is honored **in full**: the cap governs our own
  guesswork, not an explicit instruction from the source (CLAUDE.md: never exceed documented
  rate limits).
- **Equal jitter (`[d/2, d]`), not the more usual full jitter (`[0, d]`).** Full jitter can
  schedule a retry milliseconds after a 429 — the one response that means *stop asking*. Half
  the delay stays fixed, which puts a floor under every retry and still spreads them out.
- **`Retry-After` is parsed as delta-seconds only.** RFC 9110 also permits an HTTP-date; that
  would cost a date-parsing dependency to serve a form none of the allowlisted sources send.
  An unparseable header is not an error — it degrades to `None`, i.e. the exponential
  schedule, so the failure mode is "we wait longer", never "we wait less".
- **`fastrand` 2.4.1 for jitter, not `rand`.** `rand` is the ecosystem default, but its default
  features pull in chacha20 — a CSPRNG, to smear a retry by a few seconds. `fastrand` is one
  crate with no dependencies. The randomness here is not security-relevant; if anything in this
  project ever needs a CSPRNG, that is the moment to add `rand`, not now.
- **Error messages strip the URL (`reqwest::Error::without_url`).** `reqwest`'s `Display`
  includes the failing URL, and privacy rule 7.1 bars credentials from logs — a source taking a
  token as a query parameter would put one in every error string. The poller already knows the
  `SourceId` it called, so the URL adds nothing. Asserted by a test that requests
  `?access_token=super-secret` and greps the message.
- **`wiremock` 0.6.5 as a dev-dependency.** Not a new choice — docs/10 §2 already mandates it
  for adapter tests. Pulled in at 1.1 rather than 1.4 so the User-Agent and the timeout are
  verified *on the wire* at the moment they are introduced; a constant asserted against itself
  proves nothing about what reqwest actually sends.
- **The 10 s timeout is asserted two ways** — as a constant, and by a mock that hangs for 30 s
  against a 200 ms client (mechanism: `Client::timeout` is wired and maps to `Network`).
  Asserting the mechanism *at* 10 s would mean a ten-second test. Every other mock test uses
  the real 10 s client: a tight deadline against loopback buys nothing but CI flakes.
- **A test caught its own flake before CI could.** The privacy test originally dropped a
  `MockServer` to get a connection failure; with tests running in parallel another server bound
  the freed port and answered 404. It now targets `127.0.0.1:1` — refused instantly, no DNS,
  and nothing a sibling test can bind underneath it.
- **Verification:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace` all green — **107 tests** (51 core, 31 app, 20 ingest, 5 render),
  ingest suite 0.22 s. No network was contacted: every test is a local mock, and no
  allowlisted host has been called yet (that starts at 1.4).

## 2026-07-15 — M0 item 0.8: the gate

- **M0 does not close: six of seven acceptance lines are met, the seventh cannot be checked.**
  "CI runs fmt + clippy + tests on push; badge green" needs a remote, and `git remote -v` is
  still empty — `github.com/arcTanMyAngle/look-above` returns HTTP 404 (fetched, not assumed).
  The workflow has therefore never executed. The decision here is to record the gate as **run
  with one line blocked** rather than pass it: a gate that certifies its own unverifiable line
  is worth nothing, and "the YAML looks right" is not the claim acceptance §M0 asks for. M0
  closes when the owner pushes and the badge goes green (NEXT_ACTIONS #1) — nothing else is
  outstanding.
- **The clean-clone line was checked in an actual fresh clone.** `git clone` to a scratch dir,
  then `cargo build --workspace` from cold: exit 0 in 66.2s. The warm working tree could not
  have proven this line no matter how green it looked — it cannot catch an uncommitted file the
  build needs, and that is the entire failure mode the line exists to catch. It also
  incidentally confirmed the two config-adjacent lines from the outside: the clone contains
  `config.example.toml` and no `config.toml`, and the binary built there ran on defaults.
- **Dependency direction verified from `cargo metadata` edges, not by reading `cargo tree`.**
  Acceptance says "no reverse deps in `cargo tree`"; the intent is the property, and scanning a
  deep tree by eye is exactly where a reverse edge would survive. Enumerating intra-workspace
  edges yields the whole graph in seven lines: `ingest`/`store`/`render` → `core`, `app` → all
  four, nothing else — no crate depends on `app`, and the three middle crates do not depend on
  each other. `core`'s only externals are async-trait, rayon, serde, thiserror (no tokio,
  reqwest or rusqlite), and `render` pulls no winit, no network, no DB, which is the 0.6 crate
  seam holding.
- **Config and the window were checked against the running binary, not the unit tests.** The
  31 app tests already assert the precedence rules, so re-reading them would prove nothing new;
  the gate's question is whether the shipped binary behaves that way. Missing file → defaults
  (`look_above.db`, 24h, credentials "absent"); a `config.toml` → `from_file.db`/6h; with
  `LOOK_ABOVE_*` set → `from_env.db`/3h. Env beats file beats default, observed in the startup
  log each time. The window was driven over Win32: opened titled "Look Above", four resizes,
  minimize (0×0) and restore, `WM_CLOSE` → "close requested" → "window closed", `cargo run -p
  look-above` exit code 0, no panic on stdout or stderr.
- **Note for future window-driving sessions (M2 visual QA):** `FindWindow(NULL, "Look Above")`
  returns 0 against this app from a non-interactive PowerShell host even though the window
  exists and is correctly titled — `EnumWindows` and `Process.MainWindowHandle` both find it
  (hwnd confirmed, title exact). Discover the handle via `Get-Process -Name look-above` and
  `MainWindowHandle`. This is a quirk of the scripting host, not a defect in the app; it cost a
  wrong "no window appeared" result once already. Also: `cargo run` makes the app a *child*
  process, so an exit code must come from `$LASTEXITCODE` on a foreground `cargo run` —
  `Start-Process -PassThru` reports `ExitCode` empty here.
- **Verification:** all three commands green on Windows; 87 tests (51 core, 31 app, 5 render).
  No code changed at this item. Working tree clean afterwards — the runs left no `config.toml`
  or `*.db` behind in the repo.
