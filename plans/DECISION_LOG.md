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

## 2026-07-15 — Repo identity: `look-above`, not `look_above` (owner call)

- **The remote the owner supplied was `git@github.com:arcTanMyAngle/look_above.git` — an
  underscore, where every doc says hyphen.** Probed both spellings unauthenticated:
  `look_above` → HTTP 200 (exists), `look-above` → HTTP 404. So the repo that exists is *not*
  the one the code points at. This is not cosmetic — docs/09 makes
  `github.com/arcTanMyAngle/look-above` our identity in the User-Agent sent to every
  aviation source, i.e. the URL a source operator follows to find out who is polling them,
  and it currently 404s. The README CI badge has the same defect and would never render.
- **Decision (owner): rename the GitHub repo to `look-above`.** The alternative — keep the
  underscore and edit the identity in five places (USER_AGENT + its test, README badge,
  docs/09, the authorized-sources skill, this log) — was rejected: the hyphen already matches
  the crate names and the binary (`cargo run -p look-above`), so one rename fixes everything
  and changes no code. GitHub redirects the old URL, so nothing that already refers to
  `look_above` breaks. **The rename must land before the first push** — the remote is set to
  the hyphenated URL and will fail against the current name (NEXT_ACTIONS #1).
- **`origin` is now set** to `git@github.com:arcTanMyAngle/look-above.git`. The push itself is
  the owner's (their call): **this machine has no SSH key** — `~/.ssh` holds only
  `known_hosts`, and `git@github.com` returns `Permission denied (publickey)`. No key was
  generated; that was offered and declined in favour of the owner pushing from their own
  terminal.
- **The repo is public; inception recorded "private by default until owner says otherwise".**
  An unauthenticated `HEAD` returns 200. Flagged rather than acted on — it is the owner's
  call, and nothing sensitive is exposed (`config.toml` is gitignored, untracked, and absent
  from a fresh clone; verified at the 0.8 gate). Noting it so the record and reality agree.

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

## 2026-07-15 — M1 item 1.2 (host allowlist)

- **The allowlist is an enforced gate, not a checked const** — docs/10 §privacy specifies
  "a single const list of permitted hosts; test walks all adapter base URLs and asserts
  membership". Implemented as written it would assert over an *empty set* today (no adapters
  until 1.3) and, once they exist, would only ever see the base URLs an adapter remembered to
  declare — not a URL built at the call site. So `ingest::allowlist::HostPolicy` is checked in
  `HttpClient::get`, the choke point item 1.1 already established every adapter must pass
  through, against the parsed `Url` that would go on the wire. This extends a doc rather than
  following it; the const list and the membership test it asks for both exist.
- **Redirects are gated too.** `reqwest` follows up to 10 by default, so a gate that only
  checks the outbound URL is one `Location` header away from meaningless — an authorized host
  could hand us anywhere. A custom `redirect::Policy` applies the same check per hop. Because
  installing a custom policy *replaces* reqwest's default limit rather than adding to it,
  `MAX_REDIRECTS = 10` is restated explicitly, matching `Policy::limited`'s own `>` comparison
  (`previous()` counts the original request; `>=` silently costs a hop — caught by a test that
  asserts the request count on the mock, not by reading the docs).
- **`SourceError::Refused { reason }` added to `core`** — the second extension of docs/09's
  taxonomy after 1.1's `Request`. Every other variant reports what a source *did*; this one
  reports that we declined to ask. It needed to exist: `Network` is transient, so a refusal
  mapped there would retry an unauthorized host forever, and `Request` claims an HTTP exchange
  that never happened. Not transient, and not a reason to fail over — the next source would be
  asked the same wrong question. One variant covers both an unparseable URL and a rejected
  origin, since the caller's only branch is "permanent".
- **Exact host matching, never suffix.** `ends_with("opensky-network.org")` also welcomes
  `evil-opensky-network.org`. `auth.opensky-network.org` is listed in full instead. The test
  pins eight lookalikes that a `contains`/`ends_with`/`starts_with` allowlist would admit.
- **HTTPS is part of the gate**, not a property of the URL string: an `http://` typo on the
  token endpoint would put the OAuth2 client secret on the wire in cleartext.
- **Refusals log scheme + host only** — never path or query (privacy 7.1), same reasoning as
  1.1's `without_url()`: a source taking a token as a query param would otherwise leak it into
  every refusal.
- **Scope: runtime hosts only.** The skill also authorizes bulk static downloads (OurAirports,
  FAA registry, openflights, Natural Earth). They are deliberately *not* on the list: they are
  fetched by import tooling at setup time, not by `ingest`, and `raw.githubusercontent.com`
  serves anyone's repository — widening the live-polling gate to cover a build step it never
  uses weakens it for nothing. That tooling extends the list on purpose when it lands.
- **`#[cfg(test)]` escape hatch, not a cargo feature.** Tests point the *real* client at a
  loopback mock, so `HostPolicy` has an `AuthorizedOrLoopback` variant gated on `cfg(test)`.
  A `testing` feature was rejected: cargo feature unification could switch a privacy gate off
  in a shipped binary via an unrelated crate's dependency. `cfg(test)` cannot escape this
  crate's own test build. One test builds via `HttpClient::new` to prove loopback is refused
  in production.
- **The membership test scans source, not a registry.** It walks `src/**/*.rs`, skips comment
  lines (so citing a spec URL in a doc comment is not a failure — a rule that punishes
  documentation gets deleted), truncates at `#[cfg(test)]\nmod tests`, and asserts every URL
  literal's host is on the list. Today the crate has no request URL, so the walk is a tripwire
  that arms itself at 1.3; the extractor therefore has its own unit test, and the walk asserts
  it visited ≥ 1 file — a scan that silently found nothing would pass forever.
- **Verification:** the tripwire was exercised rather than assumed — a `flightradar24.com`
  const planted in `http.rs` failed the test with the file, host, and remedy named, then
  reverted. 126 tests (51 core, 39 ingest, 31 app, 5 render); fmt/clippy/test green.

## 2026-07-15 — M1 item 1.3 (OpenSky OAuth2 token fetch, cache, refresh)

- **`credentials.json` is read natively, as a third credential source.** OpenSky's account
  page hands out an API client as `{"clientId": …, "clientSecret": …}`; the plan assumed those
  values would be transcribed into `config.toml`. Owner chose to support the file as-issued.
  Precedence: `LOOK_ABOVE_OPENSKY_*` > `config.toml` > `credentials.json` > source disabled.
  The transcription step it removes is the one that drops a character, and the one where a
  secret gets pasted into a file that is not gitignored. **`credentials.json` added to
  `.gitignore`** — verified untracked and absent from history first, so nothing leaked.
- **The file is all-or-nothing, unlike the env/file path.** If either half of the credential
  was named anywhere else, `credentials.json` is not consulted *at all* rather than filling
  the gap. The two values are issued as a pair: completing a `config.toml` `client_id` with a
  `clientSecret` from an unrelated download builds a pair that authenticates as nobody, and
  the resulting 401 is invisible from either file. (Env-completes-file stays supported for
  `config.toml`, where the halves are typed by hand and splitting them is documented.)
- **Unknown fields tolerated in `credentials.json`, denied in `config.toml`.** The asymmetry
  is deliberate: `config.toml` is written by a human, so an unknown key is a typo that would
  read as "absent" (0.5's rule). `credentials.json` is generated by OpenSky, so an unknown key
  is OpenSky adding a field — refusing to start over it means breaking on their release note.
  A *malformed* file is still a hard error, same as a broken `config.toml`: the file exists,
  so the operator meant to authenticate, and dropping silently to the no-key fallbacks hides it.
- **`SecretString` moved `app::config` → `core::secret`.** `ingest` must hold credentials (it
  sends them) and cannot depend on `app`. The alternative was a second copy of the redaction,
  and privacy rule 7.1 implemented twice is enforced once. Re-exported from `app::config`.
- **Privacy rule 7.1 amended (owner-approved), and its two restatements with it.** Supporting
  `credentials.json` made the old wording — "credentials live in gitignored `config.toml` or
  environment variables" — false as written, and a hard rule the code contradicts is a rule
  that gets ignored wholesale. Amended in **all three** places that stated it, since fixing
  only one relocates the contradiction: `docs/04` **7.1** (the authority the code cites by
  name), `CLAUDE.md` Hard rules, and `docs/06`'s never-commit list (which now names
  `credentials.json` explicitly rather than relying on "credentials" to cover it). **New
  `docs/04` rule 7.1a** records the mechanism, which nothing had written down: credential
  material is `SecretString` (redacted `Debug`, deliberately no `Display`), `expose` is the
  single audited route to a value, and URL-bearing errors are stripped via `without_url`
  because a token can ride in a query string. Rule *numbers* are unchanged, so the many
  `privacy rule 7.1` citations in code still point where they claim to.
- **Refresh at 80% is a retry window, not just a deadline.** On a refresh failure the cached
  token is reused if it is still inside its life, with a warning. Refreshing early and then
  hard-failing on error would buy exactly nothing over refreshing at 100% — the 20% slack *is*
  the feature, so a token-endpoint blip costs a log line rather than a poll cycle. Past actual
  expiry the error surfaces.
- **`HttpClient::post_form` added — the choke point had to cover POST.** 1.1/1.2 gated `get`
  only; the OAuth2 grant is a POST with a body, and a bare `reqwest::Client` for it would have
  routed the client secret around the allowlist. Credential goes in the body, never the query
  string (a URL reaches proxy logs and reqwest's error `Display`; a body reaches neither).
  `HttpClient::build` widened to `pub(crate)` so sibling tests reach the loopback policy.
- **`reqwest` feature `form` added.** 0.13 moved `RequestBuilder::form` behind a feature that
  0.12 had always on. Noted for 1.4: **`query` is behind a feature too** and `/states/all`'s
  bbox params will need it.
- **Token response is validated, not trusted:** `token_type` must be bearer (else we would
  send a non-bearer token in an `Authorization: Bearer` header and get a 401 that looks
  exactly like bad credentials), `access_token` non-blank, `expires_in` non-zero, and **TTL
  clamped to 24 h** — `Instant + Duration` *panics* on overflow and `expires_in` is a number
  off the network.
- **Clock injected (`Clock` trait, `Instant` not `SystemTime`).** The 80% schedule is testable
  by advancing a fake clock instead of sleeping 24 minutes; `Instant` because a token's life is
  a duration and a user correcting their wall clock must not expire it early or late.
- **`tokio::sync::Mutex` held across the fetch** — ten concurrent callers on a cold cache cost
  one token request, not ten. A `std` guard cannot be held across an await; the thundering-herd
  case is pinned by a 10-task test.
- **⚠ Found, deferred to 1.4: OpenSky's 429 does not use `Retry-After`.** Their docs specify
  **`X-Rate-Limit-Retry-After-Seconds`**, which 1.1's `retry_after()` does not read — so the
  backoff floor silently misses OpenSky's own hint and falls back to the exponential schedule.
  Not wrong today (the token endpoint is not credit-metered), but `/states/all` is, and 1.4
  must handle it.
- **Verification — the first live API call in the project.** Every other test here is a mock,
  which proves only that we parse what we *believe* OpenSky sends. An `#[ignore]`d
  `live_opensky_issues_a_usable_bearer_token` proves the belief against the real endpoint, and
  was run once with the owner's credentials: **accepted; TTL 1798 s; refresh scheduled at
  1438 s = 79.98%**. That confirms the documented ~30 min and validates the whole schedule on
  real data. It costs no credits (the ledger meters `/states/*`, not the token endpoint) and
  is `#[ignore]`d so CI — which has neither credentials nor a network policy for this — never
  runs it. The allowlist tripwire from 1.2 **armed as predicted and was exercised**: a
  `flightradar24.com` host planted in `TOKEN_ENDPOINT` failed the scan test with file, host and
  remedy named, then reverted. 161 tests (56 core, 57 ingest, 43 app, 5 render); fmt/clippy/test
  green.

## 2026-07-15 — M1 item 1.4 (OpenSky `/states/all` adapter)

- **`ingest::opensky::states`: `OpenSkySource` (implements `LiveSource`), positional-array
  parsing, and `credit_cost`.** Split from `auth` because the two fail differently: a token
  endpoint blip is survivable inside the refresh slack, a bad bbox is not.
- **Field indices are named constants (`mod field`), not literals.** OpenSky sends **lon
  before lat** — backwards from every other source here and from every map UI. A swap
  compiles, parses, and puts aircraft in the wrong hemisphere. Nothing in the type system can
  catch it, so it is pinned by a test asserting real geography (Frankfurt is at 50°N 8.6°E,
  not 8.6°N 50°E) and, live, by asserting every returned aircraft falls inside the requested
  bbox.
- **Parsing is per-field tolerant, per-record fallible.** `states` elements are `Value`, not
  `Vec<Value>` — typing them as arrays would make one non-array record fail the whole batch,
  where docs/10 §2 requires skipping it. Four facts are required (`icao24`, `time_position`,
  lon, lat) and a record missing any is dropped, because the pipeline treats those as given.
  A *wrongly typed* optional field reads as absent (a string altitude is no altitude), which
  keeps the aircraft on screen. Skips are counted: routine ones log at `debug`, but losing
  **every** record logs a `warn` — that is what a changed field order looks like, and an empty
  sky does not explain itself.
- **`time_position` (index 3), not `last_contact` (index 4).** They differ when OpenSky has
  heard the aircraft recently but not its position. Using the newer one would date a stale fix
  to now, and M2's dead reckoning would then advance it from a place it had already left —
  drawing a confidently wrong aircraft. No `time_position` means no position to report.
- **Coordinates are range-checked on arrival.** `BBox` validates its own corners, but these
  come off the wire: Web Mercator of latitude 91 is not an error, it is a plausible-looking
  point in the wrong place, and NaN would propagate into the vertex buffer.
- **`states: null` is an empty sky, not a parse error.** OpenSky sends `null`, not `[]`, for a
  quiet region — this would otherwise fail on every quiet bbox.
- **`on_ground` absent → airborne.** Documented non-null, so absence means drift; airborne is
  the assumption that loses least (a glyph, versus the whole aircraft).
- **A global query sends no bbox parameters.** The endpoint's default *is* the world; a ±180°
  box is a different, 4-credit question with a worse answer. This is why `RegionQuery` keeps
  global distinct from a whole-world box (docs/09).
- **`credit_cost` is a free `pub fn`, not only a trait method** — 1.7's ledger needs the price
  without holding a source, and the alternative has it construct an adapter to ask a question
  about arithmetic. **Tier boundaries round to the dearer band**: OpenSky documents "0–25",
  "25–100", "100–400", leaving each edge in two tiers. Guessing cheap is the direction that
  hurts — the ledger would believe it holds credits it has already spent, and overrunning a
  documented allowance is what privacy rule 1.3 forbids. Guessing dear costs a slightly wider
  poll interval.
- **A disabled source returns `Auth`, and deliberately does not fall back to anonymous
  access.** OpenSky also serves unauthenticated callers at 400 credits/day; silently dropping
  to that would turn a missing credential into a tenth of the budget with no clue why. `Auth`
  is not transient, so the poller fails over to the keyless fallbacks (1.5–1.6) instead.
- **Closed from 1.3: OpenSky's 429 header.** `http::retry_after` now reads a list —
  `Retry-After` first, then **`X-Rate-Limit-Retry-After-Seconds`** — taking the first
  *usable* hint, so an unparseable standard header cannot shadow a good vendor one. Naming a
  vendor header in the shared client leaks one source into a place that serves all of them;
  the alternative was threading a per-adapter header list through `send_json`, which is a lot
  of machinery for a header no other authorized source sends and none can be harmed by us
  looking for. Revisit if a second source needs its own spelling.
- **Closed from 1.3: `reqwest`'s `query` feature enabled**, as predicted, for the bbox params.
  `async-trait` added to `ingest` (implementing the dyn-compatible `LiveSource`);
  `OpenSkyAuth::build` widened to `pub(crate)`, the precedent `HttpClient::build` set.
- **⚠ Known gap, binds in M3: the anonymity flag catches only half of privacy rule 2.2.**
  `anonymous` is set when a record carries no callsign — the "position with no identity" case.
  A **PIA hex that does broadcast a callsign is not detected**: that needs the FAA's assigned
  address ranges, which we do not have. Rule 2.1 notes our feeds already honor the programs,
  and the enrichment gate (M3) is where this binds and where the range data must land.
- **Fixtures are hand-written to the documented shape**, not recorded: `scripts/record_fixture.rs`
  (item 1.10) does not exist yet, CLAUDE.md forbids pasting raw responses into context, and the
  awkward cases (a non-array mid-`states`, every field null) arrive live when they arrive.
  Provenance and the re-record-at-1.10 note are in `tests/fixtures/opensky/README.md`. No
  credential material (privacy 7.2).
- **Verification — the project's first live *data* request, and the reason the mocks can be
  trusted.** Hand-written fixtures prove only that the parser matches our belief; the belief is
  the risky part, and field order in a positional array is invisible to the compiler. An
  `#[ignore]`d `live_opensky_states_match_the_documented_shape` was run against the real
  endpoint: **72 aircraft over Switzerland, every one inside the requested bbox, 20 on the
  ground, 1 credit spent**. Containment is the assertion that matters — swapped coordinates
  would put these near (8°N, 47°E), in Somalia, and every one would have failed. It also
  asserts *someone* has a callsign and *someone* a velocity, since reading the wrong indices
  would otherwise report every optional field absent and pass. `#[ignore]`d so CI never spends
  a credit. 196 tests (56 core, 92 ingest, 43 app, 5 render); fmt/clippy/test green.

## 2026-07-17 — M1 item 1.5 (airplanes.live adapter, shared readsb parser)

- **`ingest::readsb` is the shared parser; `ingest::airplanes_live` is the adapter.** docs/09
  mandates the split (adsb.lol speaks the same readsb shape at 1.6): the field mapping is one
  implementation parameterized by `SourceId`, while endpoint, spacing, fixtures, and the live
  test stay per-adapter, because the two services drift independently. `coordinate`/`narrow`
  were lifted from `opensky::states` into a `pub(crate)` `ingest::normalize` so the two
  parsers share them instead of growing copies.
- **Units convert at the parse boundary — readsb is aviation units, `StateVector` is SI.**
  `alt_baro` in feet, `gs` in knots, `baro_rate` in ft/min (OpenSky sent SI already). A missed
  conversion compiles and produces plausible-looking numbers in the wrong unit, so the factors
  are named constants (`METRES_PER_FOOT` = 0.3048 exactly, knot = 1852/3600 m/s) and both the
  fixture tests and the live test assert values that an unconverted number would fail.
- **A position is dated `now − seen_pos`, never receipt time** — the same call as 1.4's
  `time_position`: `seen_pos` is the position's age, and dating a stale fix to now would have
  M2's dead reckoning advance the aircraft from a place it had left. A record without
  `seen_pos` (or `hex`, `lat`, `lon`) is dropped. **`now` is normalized by magnitude**
  (> 10¹¹ → milliseconds): the APIs send ms where readsb's own `aircraft.json` sends seconds,
  and a wrong scale dates every position to 1970 or the year ~56,000 — the live test asserts
  `ts` lands within the current hour. A response without a usable `now` yields zero records
  (the loud all-skipped `warn`), not a parse error and not a receipt-time batch.
- **`alt_baro: "ground"` → `on_ground = true`, altitude `None`** — a surface flag, not an
  altitude of zero. Any other non-numeric `alt_baro` reads as absent-and-airborne (the
  assumption that loses least, as in 1.4).
- **`~`-prefixed hexes (TIS-B/ADS-R synthetics) are skipped, counted, and logged at `debug`.**
  `Icao24::from_hex` already rejects them (0.3 built that in for exactly this): a synthetic
  target must not be tracked under a minted identity. The all-records-lost `warn` tripwire is
  reused from 1.4.
- **bbox → covering circle: midpoint center, radius = farthest corner, ceil'd, clamped to the
  documented 250 nm with a `warn`.** The endpoint takes a point and radius, the contract is a
  bbox. All four corners are measured (the lat/lon midpoint is not equidistant from them on a
  sphere — the pair farther from the pole is farther in metres); ceil so the circle
  circumscribes rather than clips; floor 1 nm so a degenerate box still queries. Clamping an
  oversized box (M1 allows up to ~1,000 km across → ~382 nm) trades partial coverage for a
  working failover, loudly; the acceptance bbox (~500 × 500 km → ~191 nm) fits whole.
- **Results are filtered back to the requested bbox.** The circle sees past the corners, and
  every source must answer the same question or 1.9's merge compares different regions.
- **A global query returns `Refused` without sending anything.** A point/radius endpoint
  cannot answer "the world", and a max-radius circle around an arbitrary point would be a
  confidently wrong answer. Global polling is M4's problem; `Refused` is not transient, so the
  poller moves on.
- **`cost()` is 0** (the contract's "0 when unmetered") — what airplanes.live meters is
  *rate*, which is paid in time by the pacer, not in credits by the ledger.
- **≥ 2 s spacing lives in the adapter (`ingest::pacer::Pacer`), not the poller.** The
  documented limit (1 req/s; the skill directs ≥ 2 s) is the source's, not a scheduling
  choice, so the adapter enforces it whatever the caller does: a tokio-mutexed timestamp,
  lock held across the sleep so concurrent callers queue spaced rather than waking together.
  Paced *after* the allowlist could refuse — a request that never leaves spends no interval.
  Tested under `start_paused` (tokio `test-util`, dev-only — no injected `Clock` needed where
  1.3 needed one); deliberately **not** re-proven over wiremock, where the auto-advancing
  paused clock can fire the 10 s timeout while a real socket reply is in flight. The adapter
  asserts its wiring (`interval == 2 s`) instead.
- **Fixtures hand-written to the documented shape** (1.10's recorder still absent), per-case
  README with provenance and units notes in `tests/fixtures/airplaneslive/`. docs/09 §airplanes.live
  and the skill's response line gained the units/`seen_pos`/`~`-hex detail — the contract
  summary listed field names but not units, and units are the trap.
- **Verification — live, keyless, free.** `live_airplanes_live_point_matches_the_documented_shape`
  ran once against the real `/v2/point`: **48 aircraft over Switzerland (73 nm circle around
  47°N 8°E), every one inside the bbox, every `ts` within the hour (so `now` is confirmed
  ms), every altitude/speed in SI ranges (so the conversions ran), 1 anonymous, 4 on the
  ground, 0 credits.** `#[ignore]`d; run once after changes, never in CI. 233 tests (56 core,
  129 ingest, 43 app, 5 render); fmt/clippy/test green.

## 2026-07-17 — M1 item 1.6 (adsb.lol adapter; shared point-query in `ingest::point`)

- **The second readsb fallback shares the *request*, not just the parser.** 1.5 shared the
  field mapping (`ingest::readsb`) but wrote the bbox→circle geometry as "the adapter's own
  geometry problem". adsb.lol proved that framing wrong: the whole request path — global →
  `Refused`, covering circle, 250 nm clamp + partial-coverage warn, four-decimal URL, pacing
  after the allowlist, `send_json`, bbox-trim — is byte-identical between the two services
  (same `/v2/point/{lat}/{lon}/{radius}` shape, same readsb reply). Rule of two: it moved to
  `ingest::point::PointSource`, and `airplanes_live` was refactored to delegate. Two copies of
  ~65 lines + their geometry tests would have contradicted the same ethos that made
  `readsb`/`normalize`/`pacer` shared. What each adapter still owns is exactly what differs:
  **host, `SourceId`, spacing, fixtures, live test** — docs/09's "separate adapter per source"
  is preserved by the thin wrappers, not by copied logic.
- **adsb.lol's spacing mirrors airplanes.live's ≥ 2 s, though no limit is documented.** The
  skill gives airplanes.live a number (1 req/s) but only "be gentle" for adsb.lol. Privacy
  rule 1.3 is "never exceed documented limits"; with none documented, the safe reading is the
  gentle one, not a licence to go faster. Inheriting the neighbour's conservative interval
  costs nothing (the source is a last-resort fallback) and cannot under-honour an unknown cap.
- **Fixtures are adsb.lol's own, with deliberately distinct identities.** Four hand-written
  files + README in `tests/fixtures/adsblol/` (1.10's recorder still absent). Hexes are Swiss
  `4b….` / US `a2b3c4`, unlike airplanes.live's `3c6444`/`a1b2c3`, so a test can never pass by
  reading the wrong source's fixture. Parser null/empty tolerance is proven source-agnostically
  in `readsb::tests`; each adapter re-checks empty/nulls/malformed through its *own* fetch to
  confirm the wrapper (not just the parser) handles them and stamps the right id.
- **Test placement.** Pure covering-circle geometry (midpoint, farthest-corner ceil, clamp,
  degenerate floor), the on-the-wire URL shape, bbox-trim, and global-`Refused` are proven
  once in `point::tests` (a representative `SourceId`); each adapter keeps only what is its
  own — fixtures end-to-end, error mapping surviving the wrapper, endpoint-authorized, the
  real-client refuses an unauthorized host, spacing wiring, and the live check.
- **Verification — live, keyless, free.** `live_adsb_lol_point_matches_the_documented_shape`
  ran once against the real `/v2/point`: **46 aircraft over Switzerland (73 nm circle around
  47°N 8°E), every one inside the bbox, every `ts` within the hour (so `now` is confirmed ms
  for adsb.lol too), every altitude/speed in SI ranges (so the conversions ran), 0 anonymous,
  4 on the ground, 0 credits.** `#[ignore]`d; run once after changes, never in CI. 242 tests
  (56 core, 138 ingest, 43 app, 5 render), 4 live tests ignored; fmt/clippy/test green.

## 2026-07-17 — M1 item 1.7 (`ingest::budget`: credit ledger + cadence controller)

- **The `store`-vs-now seam, decided first (as CURRENT_STATUS asked).** The daily ledger is a
  small **owned struct held in memory** for M1, not a handle into `store` — `source_status`
  does not exist until item 1.11. The commitment is "in-memory now, persisted then":
  `CreditLedger::restored(spent, now)` is the single seam 1.11 rehydrates through, and the
  poller (1.8) owns the ledger meanwhile. Building it as a reach into a not-yet-existent table
  would have coupled 1.7 to 1.11 for no gain; a pure owned counter is testable today and
  trivially serialisable later.
- **The number defended is 3,200, not 4,000.** Privacy rule 1.3 is "stay under 80% of any
  documented limit with margin", so `DAILY_BUDGET = 0.8 × 4,000` is the cap the whole module
  enforces; the real 4,000 is never the target. `const` cannot do the `f64` multiply, so the
  value is written out and a test pins it to `(4000 · 0.8) as u32`.
- **Cadence = even-spread of the remaining budget over the remaining UTC day, and that *is*
  the pro-rating.** `poll_interval = seconds_until_midnight ÷ (remaining_budget ÷ cost)`,
  clamped to [5 s, 60 s]. On the pro-rata line (spent = budget × fraction-of-day) this reduces
  to a constant `86400 × cost / 3200 ≈ 27 s`/credit — the steady state that just fills the day.
  Spend *slower* than pro-rata → more budget into less day → interval shrinks toward the 5 s
  floor (we have savings, poll faster). Spend *faster* → interval grows toward the 60 s ceiling
  (ahead of budget, slow down). So "poll interval widens as the budget tightens" and "pro-rated
  spend targets" are one calculation, not two — `prorated_target` is exposed only as an
  at-a-glance health number (1.12), never read by the cadence. Rejected the alternative of
  "poll at the floor while under a pro-rata threshold, else widen": at cost 1 the 5 s floor is
  17,280 credits/day, ~5× the budget, so a floor-by-default cadence would blow the allowance in
  hours — the floor must be the *exception* (banked budget late in the day), not the norm.
- **Two protections, deliberately separate.** The cadence is soft and bounded to [5 s, 60 s];
  the hard stop is `can_afford` (`spent + cost ≤ 3,200`), which the poller must honour by not
  running a refused cycle. The ceiling alone cannot bound spend — a 4-credit query every 60 s
  is 5,760 credits/day — so the cap, not the interval, is what guarantees rule 1.3. When the
  budget is exhausted the cadence returns the ceiling (idle slowly, pick back up at the
  midnight reset) and `can_afford` is what actually stops the fetch.
- **Wall-clock `UnixSeconds`, not the monotonic `Instant`** the token refresh (1.3) uses. A
  daily budget resets on a *calendar* boundary, and a duration cannot roll over at midnight; a
  user correcting their wall clock across the day boundary *should* reset the ledger, which is
  behaviour to want, not a bug to guard. `div_euclid`/`rem_euclid` on the UTC-day index keep
  the arithmetic total even pre-epoch (nothing polls in 1969, but the functions stay total).
- **`cost == 0` (the unmetered fallbacks) is always affordable and polls at the floor.** The
  credit budget governs credits; a source that spends none is bounded by its own `pacer`
  (1.5/1.6), not by this ledger — so budget imposes nothing on it. `record` uses
  `saturating_add` so a runaway count pins at `u32::MAX` rather than wrapping to a small number
  that would read as budget restored.
- **Verification.** 25 unit tests: day-boundary arithmetic (incl. pre-epoch and rollover), the
  pro-rata steady state, floor/ceiling clamping under a swept `(spent, cost, time-of-day)`
  grid, the hard-cap boundary, the ledger's daily reset and restore, and `decide` agreeing with
  the free functions it composes. Pure functions, no network, no clock injection needed (`now`
  is a parameter). 267 tests (56 core, 163 ingest, 43 app, 5 render), 4 live tests ignored;
  fmt/clippy/test green. Next: **1.8**, the poller that drives this cadence and the failover
  chain.

## 2026-07-17 — M1 item 1.8 (`ingest::poller`: the poll loop + failover chain)

- **The three-way failover branch on `is_transient`.** A fetch error means one of three things
  to the active source, and `error_response` (a pure, unit-tested function) encodes exactly
  which: **transient** (`RateLimited`/`Network`/`Server`) → retry the *same* source with
  `http::backoff`, failing over only after `TRANSIENT_FAILOVER_THRESHOLD` = 3 consecutive
  failures (one timeout is not a dead source); **permanent-but-a-real-answer**
  (`Auth`/`Parse`/`Request`) → fail over on the *first*, because the identical request cannot
  succeed on a re-fetch; **our own refusal** (`Refused`) → **hold and idle**, never fail over.
  That last one is the subtle call, and it follows `error.rs`'s own note: a `Refused` is an
  unauthorized host or a global query to a point source — the *next* source would be asked the
  same wrong question, so failing over would just launder a bug into a silent degradation. The
  disabled-OpenSky case falls straight out of the permanent branch: `fetch` returns `Auth`
  without a network call, so a missing credential drops us to the keyless fallbacks on cycle one.
- **Budget veto is a *skip*, not a failover.** When `can_afford` refuses a cycle (the metered
  primary would cross the 3,200/day cap), the poller does not fetch and idles at the ceiling
  until the UTC-day reset — it does **not** fail over to a free fallback. A primary that is
  rationing its budget is not a *failed* source, and the fallbacks exist for failures; dropping
  to them on budget would poll a redundant source while the allowance simply rests. This is the
  spec-faithful reading of item 1.8 ("skips … any cycle `can_afford` refuses") and 1.7's "an
  exhausted budget idles at the ceiling until the midnight reset". *Noted as a candidate M4+
  improvement*: once global/multi-region polling lands, serving from the free fallbacks while
  the primary is budget-capped may be worth the extra source — deferred, not forgotten.
- **Recovery is a separate, faster path than the failover rotation.** Failover advances through
  the chain *wrapping* (`(active+1) % len`) so every source stays in rotation when things are
  bad; but a *working* fallback never errs, so nothing in the error path would ever pull us back
  to the primary. `PRIMARY_PROBE_INTERVAL` = 5 min is the fix: while failed over, the loop
  re-probes index 0 and switches back the instant it answers. The probe goes through the same
  budgeted `run_cycle`, so it respects the ledger and costs nothing when the primary is disabled
  (no network on `Auth`).
- **Two clocks, for the two reasons `budget` already separated them.** The ledger reads an
  injected wall-clock `WallClock` (`UnixSeconds`) because the day boundary is a *calendar* fact;
  the cadence sleeps and the 5-min probe timer use tokio's *monotonic* clock (`tokio::time`)
  because "wait 27 s" and "5 min since the last probe" are elapsed-time facts. Only the wall
  clock is injected — the monotonic side is virtual under `start_paused`, so it needs no seam.
- **`PollBatch` carries its own spend.** `credits_spent` (this cycle) and `spent_today` (running
  total) ride with the batch so the store writer (1.11) and the headless readout (1.12) read the
  cost off the channel rather than reaching back into the poller's private ledger. An *empty*
  `states` is delivered like any other — a quiet region is a real answer, and a consumer needs
  to see that the cycle happened.
- **The `Poller` never panics on a bad world.** A wild system clock reads as 0 / `i64::MAX`
  rather than overflowing; a fully dead chain idles and retries forever (the plan's "the app
  idles and retries; it never crashes"); only a dropped channel receiver stops `run`. No
  `unwrap` outside tests; `Poller`'s `Debug` is manual (`Box<dyn LiveSource>` is not `Debug`).
- **Verification.** 18 tests: the pure failover policy (transient-below/at-threshold, permanent
  fails over first, `Refused` holds), the probe gate, a successful metered cycle (spend
  recorded, batch emitted, stays primary), spend accumulation, the unmetered path (0 credits),
  the budget veto (an `Arc`-shared scripted source proves `fetch` is *never called* and the
  ceiling interval is returned), disabled-primary immediate failover, transient failover only
  after the streak, refusal-holds, chain wraps, recovery-to-primary and stay-on-fallback, and
  the dropped-receiver shutdown signal — all via an in-memory scripted `LiveSource`, no network
  and no injected monotonic clock needed. Plus a live `#[ignore]`d test that drives the real
  default chain with OpenSky disabled and asserts a real keyless-fallback batch, 0 credits. 284
  tests (56 core, 180 ingest, 43 app, 5 render), 5 live tests ignored; fmt/clippy/test green.
  Next: **1.9**, `core::merge` (dedup, out-of-order drop, sticky anonymity).

## 2026-07-17 — M1 item 1.9 (`core::merge`: dedup, out-of-order drop, sticky anonymity, staleness)

- **`SessionTable` in `core`, not `ingest`.** The merge is the *pipeline's* source of truth
  (docs/09), keyed on `Icao24` with one `StateVector` per aircraft, and it depends on nothing
  but the core vocabulary — so it lives in `core::merge` (the crate the plan reserved for it),
  clock-free and I/O-free. `ingest` produces `PollBatch`es; `core::merge` consumes their
  `states`. The store (1.11) and headless readout (1.12) will drive it.
- **Dedup is strictly newest-`ts`-wins; equal `ts` is a drop.** A record replaces the held one
  only when `incoming.ts > stored.ts`. Anything not strictly newer — an out-of-order late
  arrival *or* an equal-`ts` duplicate from a second source — is dropped, because there is no
  newer information in it. This is the same time-of-applicability reasoning as item 1.4's
  `time_position` choice: a slower feed must never drag an aircraft back to an older fix, or M2's
  dead reckoning would advance it from a place it had already left.
- **Sticky anonymity is a one-way latch, honored independent of `ts` (privacy rule 2.2).** Once
  *any* record for a hex is `anonymous`, the tracked target stays anonymous for the session and
  its `callsign` is pinned to `None` — even a *newer, identified* record does not un-anonymize
  it (`stored.anonymous || incoming.anonymous`, and clear the callsign whenever the result is
  true). The subtle call: the latch fires **even for a record we drop as stale**. An anonymity
  signal is a privacy fact, not a position; a stale out-of-order record that reveals a hex is
  anonymous still latches the flag though its position is discarded. Insertion enforces the same
  invariant defensively (an anonymous first sighting is stripped of any callsign an adapter left
  on) rather than trusting upstream. This is the code side of docs/04 §2.2 and §5.2 (anonymity
  survives into replay).
- **Staleness is tracked here but *faded* in M2.** Entries carry their `ts`, so `age(now)`,
  `stale_count(now, max_age)`, and `evict_stale(now, max_age)` are the data-layer view of
  staleness. The horizons are named constants pinned to the render skill: `STALE_AFTER_S` = 60 s
  (the skill's "begin fade" point — a track *reported* stale but still tracked) and
  `DROP_AFTER_S` = 90 s (the skill's "stop extrapolating" point — past which holding the entry
  only serves a frozen ghost, so it is forgotten). The methods take the horizon as a parameter
  (fully testable), and the constants are the standard values 1.12 will pass. The *visual* fade
  (alpha ramp, frozen extrapolation) stays the render layer's job — merge only decides fresh /
  stale / forgotten. `age` is signed (`now − ts`), so a source clock ahead of this machine reads
  negative rather than underflowing; callers wanting an unsigned age clamp at zero.
- **`MergeStats { new, updated, dropped }` is the per-batch tally** the headless readout (1.12)
  needs — "new/updated/stale" counts come from `merge` (new/updated/dropped) plus
  `stale_count`. `total()` equals the batch length, so every record is accounted for.
- **Verification.** 20 tests: newest-`ts`-wins across sources, out-of-order drop, equal-`ts`
  duplicate drop, distinct aircraft tracked separately, in-batch reconciliation; the three
  anonymity cases (first anonymous sighting strips a callsign, a later identified record does not
  un-anonymize, a stale out-of-order anonymous record still latches while its position is
  dropped) plus the negative case (an ordinary target is never touched by the latch); age,
  `stale_count`, and `evict_stale` against explicit horizons, the `STALE ≤ DROP` invariant as a
  `const` assertion, and the stats-total accounting. 304 tests (71 core, 180 ingest, 43 app, 5
  render), 5 live ignored; fmt/clippy/test green. Next: **1.10**, `scripts/record_fixture.rs`.

## 2026-07-17 — M1 item 1.10 (`scripts/record_fixture.rs`: the fixture recorder)

- **The tool the hand-written fixtures have stood in for since 1.4.** docs/06 sanctions exactly
  two live fetches during development — running the app, and this recorder — and every fixture
  README promised "re-record once item 1.10 lands". It fetches from an authorized source, trims
  the record array to ≤ 20, credential-scrubs (privacy 7.2), and writes to
  `crates/ingest/tests/fixtures/<source>/<name>.json`, **printing only a count and a path,
  never the payload** (docs/06 network rule).
- **A bin of the `ingest` crate, sourced from repo-root `scripts/`.** The docs name
  `scripts/record_fixture.rs`, so the file lives there and is wired as `[[bin]]` with
  `path = "../../scripts/record_fixture.rs"` (Cargo accepts the out-of-package path cleanly —
  probed before building on it). It is a bin of `ingest`, not a standalone crate, because a
  recording must go out *exactly as a poll would*: it reuses the allowlist-enforcing
  `HttpClient`, the OpenSky `OAuth2` client, `STATES_ENDPOINT`, the two `POINT_ENDPOINT`s, and
  `point::MAX_RADIUS_NM` rather than reconstructing any of them. It is never built unless asked
  for by name, so it costs nothing on a normal `cargo build -p look-above-ingest`.
- **Region parameters are each source's own native shape, not a bbox everywhere.** OpenSky takes
  its `lamin/lomin/lamax/lomax` bbox; the readsb feeds take `/point/{lat}/{lon}/{radius_nm}`
  directly. This is what let the recorder avoid a *third* copy of `point`'s covering-circle
  geometry — the recorded *response shape* is identical however the region was specified, and
  the recorder is a tool, not a production request path, so the honest move was to speak each
  endpoint's own language rather than duplicate 30 lines of sphere math the rule-of-two ethos
  already consolidated (item 1.6).
- **Credentials: env-only, and that is forced by layering, not laziness.** OpenSky recording
  reads `LOOK_ABOVE_OPENSKY_CLIENT_ID` / `_SECRET` — the highest-precedence rung of privacy 7.1.
  It cannot read `config.toml`/`credentials.json` because that loader lives in `app`, and
  `ingest` depending on `app` would invert the crate direction. A manual tool run by the account
  owner can set two env vars.
- **Trim before scrub; the scrub is a tripwire, not a cleaner.** Trimming first keeps the scrub
  off discarded records. The scrub recursively drops a denylist of credential/account-shaped
  keys (case-insensitively) — and on today's authorized responses it removes *nothing*, because
  the readsb feeds are anonymous and `/states/all` is public aircraft data. It exists so the
  tool stays safe the day a source echoes an account field, precisely because docs/06 forbids
  reading the payload to check by eye.
- **Not a drop-in re-record.** The crafted `*_nominal.json` fixtures pin *exact* values the
  parser tests assert (e.g. 36,000 ft → 10,972.8 m, the lon-before-lat Frankfurt record), which
  live data will not reproduce, and the `empty`/`nulls`/`malformed` cases capture shapes that
  arrive only when they arrive. So the recorder refreshes a fixture's *shape* and resets one
  after a documented source change; it is not a routine overwrite. The three fixture READMEs and
  the root README now say so and carry the command.
- **Errors are `Box<dyn Error>`, not `anyhow`.** CLAUDE.md reserves `anyhow` for the `app`
  binary, and adding it to `ingest` for a script would pull a dep into the wrong crate; the std
  boxed error takes `?` from `SourceError`/`io`/`serde_json` and `.into()` from a `&str`/`String`
  usage message with no new dependency.
- **Verification.** 9 offline unit tests (trim to the ceiling / leave a short list / a null array
  is zero records; scrub at every depth, case-insensitively, leaving public fields untouched, and
  a no-op on an ordinary body; source→dir/key mapping; an unsafe fixture name refused before any
  write; output-name index tracking region arity; bbox parse order). Then the **live path itself
  was exercised**, since a recording tool is only proven by recording: `adsblol 47 8 73` fetched
  16 real aircraft over Switzerland, wrote a valid trimmed `{ac, now, …}` file, printed only the
  count — checked structurally (never by printing values) and deleted. 313 tests total (the 9 in
  the new bin target), fmt/clippy/test green. Next: **1.11**, `store` migrations + writer thread.

## 2026-07-18 — M1 item 1.11 (`store`: migrations + writer-thread skeleton)

- **`crates/store`'s first real code.** `migrations::apply` — numbered SQL, `include_str!`-embedded
  (so the compiled binary is self-contained; no `migrations/` directory ships alongside it),
  progress tracked in `SQLite`'s own `PRAGMA user_version`. Each migration's DDL and its version
  bump commit together inside one `BEGIN IMMEDIATE … COMMIT`, so a crash mid-migration can never
  leave `user_version` ahead of the schema it claims, and `BEGIN IMMEDIATE` claims the write lock
  up front rather than on the first statement, so a concurrent reader can never observe a
  half-applied migration. A migration whose version is `<=` the connection's current
  `user_version` is skipped, which is what makes re-running `apply` against an already-migrated
  database a no-op rather than a "table already exists" error (docs/10 §3's "idempotent-by-version"
  requirement) — and it trusts `user_version`, not a live `sqlite_master` probe, so a connection
  that already *claims* the latest version has nothing re-run even if (hypothetically) its tables
  were missing.
- **Migration 0001 creates only `aircraft` and `source_status` — verbatim from docs/08, comments
  included.** docs/08 tags every other table in its eventual schema (`positions`, `flights`,
  `airports`, `runways`, `airlines`, `metars`) with its own later milestone (M3/M5), and migrations
  are append-only ("never edit a shipped migration"), so creating them now would mean a table with
  nothing to populate until a future item anyway. The doc and the migration file must never drift —
  a schema change updates both in the same commit, same as any other doc-is-contract rule here.
- **`core::contracts::Store` is deliberately not implemented yet.** Its four methods
  (`insert_positions`, `upsert_aircraft_meta`, `airports_in_bbox`, `prune`) each need a table
  (`positions`, `airports`) migration 0001 doesn't create — implementing the trait now would mean
  methods that can't work against the schema that exists. Instead `writer::Writer` is a concrete,
  non-trait handle scoped to exactly what 0001 backs: recording a poll cycle's outcome against
  `source_status`, and reading it back. Wiring `Store` for real is a future item once
  `positions`/`airports` land — recorded here so it isn't mistaken for an oversight.
- **The writer-thread skeleton is one `Command` enum behind one channel, not a channel per
  operation.** `Writer` is a cheap-to-clone handle (`Sender<Command>`); a dedicated OS thread owns
  the one `rusqlite::Connection` and drains the channel until every clone is dropped. Each command
  carries its own one-shot `bounded(1)` reply channel, which is what keeps every public `Writer`
  method synchronous (docs/09: "Sync API; called from the writer thread only" — the *callers* are
  sync, the thread is the one place `SQLite` is touched) while still letting the command set grow
  later (`positions`/`airports` commands, once those tables exist) without changing `Writer`'s
  public shape. `Writer::open` runs migrations synchronously on the caller's thread *before*
  spawning the writer thread, so a broken/corrupt database is reported to the caller as an `Err`
  rather than silently killing a detached thread nobody is watching.
- **Dependency direction verified, not assumed**: `crates/store/Cargo.toml` depends on
  `look-above-core` only (plus `crossbeam-channel`/`rusqlite`/`thiserror`/`tracing`, none of them
  workspace crates) — checked by reading the manifest directly per CLAUDE.md's "don't use `cargo
  tree`" rule, not inferred. That is what forces `Writer`'s API shape: `record_success`/
  `record_error` take plain `SourceId`/`UnixSeconds`/`u32`/`String`, never
  `ingest::poller::PollBatch`, and `source_status` returns a `store`-local `SourceStatus`, never
  `ingest::budget::CreditLedger`. The actual `CreditLedger::restored(spent, now)` call (1.7) happens
  in `ingest`/`app` wiring, a later item — `store` only stores and returns the raw counter it's
  given. `restored` already tolerates a stale persisted value (it compares day index against `now`
  and treats an earlier day as zero), so `store` carries no notion of UTC-day rollover at all.
- **Each verb owns exactly its own columns.** `record_success` upserts only
  `last_success`/`credits_used_today`; `record_error` upserts only
  `last_error`/`last_error_msg`. A success after a prior error doesn't erase the error record (or
  vice versa) — each write only touches the columns that verb is responsible for, proven by
  round-trip tests in both orders. `source` is `source_status`'s primary key, so a repeat write for
  the same source overwrites the row rather than duplicating it (also tested).
- **App/poller wiring is explicitly out of scope here.** `crates/app` doesn't consume `PollBatch`
  yet, so there is no running loop to feed a live `Writer` from; that lands at 1.12 (headless mode)
  or later. This item's deliverable is the `store`-crate capability alone, exercised by its own
  tests.
- **The on-disk WAL smoke test is the one place WAL is actually checked**: `SQLite`'s `:memory:`
  connections cannot use WAL (there is no shared file to write one against), so `open_connection`
  requests `journal_mode = WAL` unconditionally without asserting it took — the in-memory tests
  never could prove it. A dedicated on-disk test (temp file, cleaned up via a `Drop` guard that
  also removes the `-wal`/`-shm`/`-journal` side files even on a failed assertion) opens a real
  connection and reads `journal_mode` back, confirming it is genuinely `wal`.
- **Verification.** 16 new tests: 4 on the migration runner (fresh DB starts at version 0; apply
  reaches the latest version; apply creates exactly the two tables 0001 owns and no others;
  re-applying is a no-op) plus one edge case proving `apply` trusts `user_version` over a live
  table probe; 6 directly against a migrated connection for the upsert semantics (unrecorded source
  reads `None`; success round-trips; error round-trips without touching `credits_used_today`; a
  later success doesn't erase an earlier error; a second success overwrites rather than duplicating;
  independent sources get independent rows); 5 through the real `Writer` channel/thread (open +
  immediately usable, success end-to-end, error end-to-end, cloned handles share one thread and
  database); 1 on-disk WAL smoke test. 329 tests total (43 app, 71 core, 180 ingest, 9
  `record_fixture` bin, 5 render, 16 store), 5 live ignored; fmt/clippy/test green — independently
  re-run, not just taken on the implementing agent's word. Next: **1.12**, headless mode (the
  `--headless` per-cycle counts readout — the M1 gate evidence tool).

## 2026-07-18 — M1 item 1.12 (headless mode)

- **The region had no owner yet, so this item had to pick one.** `RegionQuery` has existed
  since M0's contracts, but nothing before this fed it a real bbox outside a test — the
  poller's own doc says the camera drives it "in M2/M4", and no config key for it exists.
  Headless mode needed *some* fixed region to poll, so it is a `const` in `app::headless`
  rather than new config surface: acceptance §M1 already names a size ("10-min live run over a
  ~500×500 km bbox stays ≤ 80% of pro-rated daily budget"), so the constant was sized to match
  it (44.5–49.5°N, 4.5–11.5°E; ≈530×555 km, 35 deg² of `OpenSky` bbox area — the middle,
  2-credit pricing tier, not the cheapest or dearest) rather than reusing the smaller
  Switzerland box every adapter's unit/live tests fly against. Adding a config key for a value
  nothing yet varies (M1 has exactly one region, ever) would be surface with no second caller —
  the camera work in M2/M4 is the right point to make it configurable, not now.
- **`Poller` needed a new public method to make the ledger-restore seam reachable.** Item 1.7
  named the seam and item 1.11 built the persistence half, but `Poller::ledgers` is a private
  field — nothing outside `crates/ingest` could have seeded it even with a `CreditLedger` in
  hand. `restore_ledger(&mut self, index: usize, ledger: CreditLedger)` is the minimal opening:
  it overwrites one slot and is a no-op out of range rather than panicking, since a hand-built
  chain (via `Poller::new`, used only in tests) isn't asserted against a valid index the way
  `with_default_chain` is. Only the primary (`OpenSky`, index `PRIMARY`) is ever metered, so
  only it is ever restored — the fallbacks' ledgers start and stay at zero, harmlessly.
- **`record_error` is not wired, and can't be without a further poller change.** The
  `PollBatch` channel (1.8) only ever carries a *successful* cycle — a fetch error is handled
  entirely inside `handle_error` (backoff/failover/hold) and only ever reaches `tracing`, never
  the channel. So a consumer here has no error value to hand `Writer::record_error`; wiring it
  would mean teaching the poller to emit failures too, a real behavioral addition outside
  "logs per-cycle counts", the checklist line this item is scoped to. Recorded here rather than
  silently doing half the job and calling it done — a future item's problem, not an oversight
  discovered later.
- **No graceful shutdown.** The gate run this unblocks (1.13) is a *supervised* 10-minute
  session — an operator watches it and stops it. Building a shutdown protocol (signal handler,
  channel teardown, drain-in-flight) for a debug tool that is never run unattended would be
  scope invented ahead of a need; the OS's default `Ctrl+C`/`SIGINT` behavior already ends the
  process correctly (the writer thread and the poller task simply stop existing).
- **A CLI parser was written by hand, not via a dependency.** One flag (`--headless`) doesn't
  justify `clap` or any argument-parsing crate; `parse_args_from` is nine lines. It rejects an
  unrecognized argument rather than ignoring it — the same call `app::config` already makes for
  an unknown TOML key ("a typo must not silently default"), so a mistyped flag is loud instead
  of quietly running the window.
- **Errors cross the `store`/`ingest` → `anyhow` boundary for free.** `StoreError` and
  `SourceError` are both `thiserror`-derived (`std::error::Error + Send + Sync + 'static`), so
  `anyhow::Context`/`?` accept them without a manual `map_err` — confirmed by the code
  compiling with none written; recorded because it's easy to reach for `map_err` out of habit
  when it isn't needed here.
- **Found while wiring, not part of the plan:** `app::config::OpenSkyConfig::credentials()` had
  carried `#[allow(dead_code)]` and a comment claiming "the poller reaches this in item 1.4"
  since item 1.3 — 1.4 never called it, and nothing did until this item. Removed the attribute
  and the now-wrong comment along with landing the real caller, rather than leaving a stale
  note next to code that finally does what it always claimed to.
- **Verification.** 5 new tests: 3 on `main::parse_args_from` (no arguments → window mode;
  `--headless` → headless mode; an unknown flag is a hard error naming itself), 2 on
  `Poller::restore_ledger` (a restored ledger is what the next cycle is judged against, not a
  fresh one; an out-of-range index is a harmless no-op). 334 tests total (46 app, 71 core, 182
  ingest, 9 `record_fixture` bin, 5 render, 16 store), 5 live ignored; fmt/clippy/test green.
  **Verified live, twice, against the owner's real `credentials.json`** (the actual OpenSky
  OAuth2 path, not the keyless fallbacks — the first time this project's own binary, not a
  test, has authenticated live): run 1 — 249 aircraft on the first cycle (`new=249`), then
  `new=1, updated=231, dropped=18` on the second (dedup visibly correct across cycles), 2
  credits/cycle, spend `2 → 4`; run 2 (a fresh process) logged `restored the OpenSky credit
  ledger from source_status credits_used_today=4` at startup and then `spent_today=6` after
  its first cycle — proving the restore round-tripped through a real process restart, not just
  the unit test. Total live spend this session: 6 of 3,200 credits (7 lifetime with 1.4's).
  `source_status` writes were confirmed by the *absence* of this module's own "could not
  record source_status" warning, which a failed write would have logged; the scratch
  `look_above.db` created by the live runs was deleted afterward (gitignored; not evidence
  worth keeping past the session). Next: **1.13**, the M1 gate — a 10-min supervised live run
  per acceptance §M1, numbers recorded, human review.
