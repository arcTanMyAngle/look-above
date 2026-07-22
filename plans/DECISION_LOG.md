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

## 2026-07-18 — M1 item 1.13: the gate (run, not fully closed)

- **A real conflict surfaced before the run started, and was put to the owner rather than
  guessed at.** This item's own checklist line (M1 plan) scopes the run to 10 minutes;
  acceptance §M1's first line requires the OAuth2 token auto-refresh be "observed across a
  > 30 min run" — and checking 1.3's live test (`live_opensky_issues_a_usable_bearer_token`)
  confirmed it only ever fetched *one* token and asserted the refresh-schedule *arithmetic*
  against its real TTL; it never stayed connected long enough to watch an actual second fetch
  happen. So no prior work covers that acceptance line, and a literal 10-minute run cannot
  either. Asked the owner directly (CLAUDE.md: stop and ask rather than guess at a
  plan/acceptance-doc conflict); **the owner chose the checklist's literal 10-min scope**,
  accepting that the token-refresh line stays open. This is the same shape as M0's gate: a
  gate can be *run* and recorded honestly short of a full pass.
- **Result: 6 of 7 acceptance §M1 lines met.** Full per-line evidence lives in the M1 plan's
  1.13 entry; the open line is the token-refresh one above, carried forward exactly as M0
  carried its badge line.
- **The run's aggressive cadence (~5.8 s/cycle, the floor) is explained, not a bug.** The
  ledger started fresh (no `source_status` row existed — the prior session's scratch DB had
  been deleted) at 21:35 UTC, so `prorated_target` spread a full 3,200-credit budget over the
  ~2.4 h left in the UTC day and landed near the 5 s floor. This is the cadence controller
  working as designed (1.7): the **hard `can_afford` cap**, not the cadence, is what actually
  bounds the 80% line, and 196/3,200 credits spent (6.1%) over the run shows it never needed
  to engage. Worth flagging for whoever reads this later: unchanged, this cadence would hit
  the 3,200 cap roughly 2.8 h into a day started this way, then legitimately idle — expected,
  not a failure mode.
- **Corrected count for the record: the tests total is 329, not "334" as 1.12's own entry
  computed.** 1.12 stated "334 tests total (46 app, 71 core, 182 ingest, 9 record_fixture bin,
  5 render, 16 store)" — those six figures sum to 329; `cargo test --workspace`, re-run for
  this gate, independently confirms **329 passed, 5 ignored, 0 failed**. Recorded here rather
  than silently editing 1.12's entry (append-only log) or repeating the arithmetic slip.
- **The three live "transient source failure" WARNs mid-run were treated as evidence, not
  noise.** Real network hiccups (streak 1 → recovered, streak 1 → streak 2 → recovered) never
  reached `TRANSIENT_FAILOVER_THRESHOLD` (3), so retry/backoff was observed live end-to-end
  without a full failover — the failover-and-recovery path itself stays evidenced by 1.8's own
  dedicated live test (OpenSky forcibly disabled → real fallback batch), combined here rather
  than re-proven, since forcing a failover in this run would have meant deliberately disabling
  the credentials mid-gate, which the checklist doesn't ask for.
- **Scratch artifacts deleted after recording, following 1.12's precedent.** `look_above.db`
  (gitignored) and the raw `qa/gate_1.13/run.log` (gitignored) are not evidence worth keeping
  past the session — the numbers they proved are in this entry and the M1 plan instead.
- **Verification:** `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace` all green (329 passed, 5 ignored, 0 failed) — run fresh for this
  gate, not assumed from 1.12. No code changed by this item. Next: **human review** of the
  open token-refresh line; M2 does not start until the owner closes or explicitly carries it,
  per CLAUDE.md's milestone-gate rule.

## 2026-07-18 — M2 opened with the M1 gate at 6/7 (owner call)

- **The owner directed "continue with M2"** — the human review 1.13 asked for, and the same
  shape as M0→M1: a milestone opens with its predecessor's gate short one line rather than
  blocked on it indefinitely. No new information arrived; the owner had already made the
  substantive call at 1.13 (accepting the literal 10-min scope over extending the run), so
  this is that decision carried one step further into starting M2, not a fresh trade-off.
- **What stays open, unchanged:** the OAuth2 token auto-refresh line (acceptance §M1, "observed
  across a > 30 min run") — 1.3's live test proved the refresh-schedule arithmetic on one
  fetched token but never watched a live second fetch happen. It is carried into M2 the same
  way M0's CI-badge line was carried into M1: named here, not silently dropped, revisit if a
  future live run happens to run long enough to observe it incidentally.
- **No code changed.** Plan-only session: CURRENT_STATUS Now/gate-table/log updated, this
  entry added. Next: **M2 item 2.1**, `render::gpu` device/queue/surface init.

## 2026-07-18 — M2 item 2.1 (device/queue/surface init, MSAA 4x, F3 stats toggle)

- **Item split into 2.1/2.1b before implementation, checked with the owner rather than
  guessed.** The checklist's own wording ("frame-stats overlay ... toggled with F3") reads as
  on-screen text, but nothing in the codebase can draw text yet — the SDF glyph atlas (2.5)
  and glyph-atlas labels (2.7) are later items in this same milestone. Writing an ad-hoc
  bitmap/quad text renderer just to show four numbers now would be thrown away or duplicated
  once the real atlas exists — exactly the kind of premature-abstraction/duplicate-work
  CLAUDE.md warns against. Owner chose the split: 2.1 ships device init, MSAA plumbing, and
  the F3 toggle with a richer *log* line; 2.1b (on-screen rendering of those numbers, reusing
  2.5/2.7's atlas) is a new, explicit checklist line rather than an implicit gap.
- **DX12 preference is two separate instance/surface/adapter builds, not one instance with a
  backend hint.** Read from the real wgpu-30.0.0/wgpu-types-30.0.0 source (not a tutorial —
  the M0 0.6 decision log entry already burned a session on stale-API tutorials): wgpu 30's
  `RequestAdapterOptions` carries no `backends` field at all. Which backend(s) an adapter can
  come from is fixed entirely by the `Backends` set the owning `Instance` was constructed
  with. So "prefer DX12, fall back to default" has to attempt a DX12-only `Instance` first and
  build a second, differently-configured `Instance` (with its own `Surface`, since a surface
  must come from the instance that produces its compatible adapter) if that fails — there is
  no single-instance way to express a preference-with-fallback.
- **`WGPU_BACKEND` still wins over the DX12 preference.** `Backends::from_env().is_some()` is
  checked first; if the operator pinned a backend (the documented way to bisect a backend bug,
  per M0 0.6), the DX12-preference branch is skipped entirely rather than racing it. The rest
  of `new_without_display_handle_from_env()`'s env handling (`WGPU_DEBUG` etc.) still applies
  to the DX12-only attempt — only the backend set itself is overridden.
- **MSAA support is checked against the adapter before the texture is created, not assumed.**
  `adapter.get_texture_format_features(config.format)` is checked for both
  `MULTISAMPLE_X4` and `MULTISAMPLE_RESOLVE` (the pass resolves into the swapchain view, so
  resolve support is load-bearing too, not just the sample count itself). A new
  `RenderError::UnsupportedMsaa { adapter, format }` surfaces a genuinely incapable adapter
  (a software/CI renderer) as a startup error instead of a `create_texture` panic the first
  time a frame is drawn — docs/01 requires 4x MSAA unconditionally, so this is the "fail
  loudly at the boundary" version of that requirement, matching 0.6's `UnsupportedSurface`
  precedent for the same class of problem.
- **The MSAA target's own contents use `StoreOp::Discard`, only the resolve survives.**
  Nothing ever reads the multisampled texture back — only the single-sampled resolve target
  (the swapchain view) needs to survive to present — so storing the MSAA attachment itself
  would be pure wasted bandwidth on every frame, every pass, from here through the rest of M2.
- **Percentiles use integer nearest-rank arithmetic, not `f64` fractions.** The workspace's
  `clippy::pedantic` lint set (`cast_precision_loss`/`cast_sign_loss`/`cast_possible_truncation`
  at `-D warnings`) flagged a first float-based cut; since a report window holds at most a few
  hundred samples (one second of frames per docs/01's 60fps target), there is no precision
  being traded away by staying in integers, so the fight with the lints wasn't worth having.
- **`instances` is logged as a hardcoded `0`, not omitted.** The field exists in the log line
  now (what 2.1 is actually asked to wire — "the reporting path") even though nothing produces
  a real count until 2.5's aircraft glyphs exist; a comment at the call site says so, so it
  reads as deliberately pinned rather than a forgotten wire-up when 2.5 lands and someone goes
  looking for where to plug in the real number.
- **Delegated to the renderer-agent; its own reported test count was wrong and is corrected
  here rather than trusted.** The agent's summary claimed "282 tests passed, 0 failed, 5
  ignored" after its changes; this session re-ran `cargo test --workspace` independently and
  got **332 passed, 5 ignored, 0 failed** (329 before this item, +3 new percentile tests in
  `frame_stats.rs` — arithmetically consistent with what was actually added, unlike the
  agent's number). `git diff --stat` confirmed only the four files scoped in the delegation
  prompt were touched. This is exactly the "trust but verify" a delegated diff gets — the
  agent's *implementation* held up under independent review; its self-reported *verification
  number* did not, and would have silently corrupted the test-count trend in this log if
  copied through unchecked.
- **Verification, run independently by this session (not taken from the agent):**
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test
  --workspace` (332 passed, 5 ignored, 0 failed) all green. Live run driven fresh over Win32:
  `backend=dx12` confirmed in the startup log against the owner's real Intel Arc GPU, two live
  resizes (500×400, then 1000×700) with the MSAA target rebuilding cleanly each time and zero
  panics/validation errors in stderr, F3 toggling `stats_visible` and the log line switching
  from `debug` (`mean_ms`/`worst_ms`) to `info` (adding `p50_ms`/`p95_ms`/`instances=0`) on
  press and back on a second press, `WM_CLOSE` → "close requested" → "window closed", clean
  exit. No `look_above.db` or other stray artifact left behind (the windowed app doesn't poll
  yet — that's 2.3's camera→poller wiring — so reading `credentials.json` at startup logged
  "configured" but made no network call and spent no credits). Scratch stdout/stderr log files
  from the run deleted after review, following 1.12/1.13's precedent. Next: **2.2**, the base
  map (Natural Earth land/coastlines).

## 2026-07-18 — M2 item 2.2a (base map data: fetch + bundle Natural Earth as GeoJSON)

- **Split 2.2 into 2.2a/2.2b, same shape as 2.1/2.1b.** The checklist's "bundled as GeoJSON"
  presumes the data already exists; acquiring it is a live download plus a format conversion,
  genuinely separable from the tessellation/pipeline half and requiring tooling that must
  never touch `render`'s Cargo.toml (see next point). Self-approved mid-session — no owner
  ambiguity here, just a scope split the same way 2.1 was split, recorded rather than left
  implicit per the token-managed-implementation skill.
- **New workspace crate, `crates/import` (`look-above-import`), depended on by nothing.**
  `render`'s Cargo.toml is one of the M0 gate's checked invariants ("no winit, no network, no
  DB" — verified from `cargo metadata` edges, not `cargo tree`, per CLAUDE.md); adding
  `reqwest`/`zip`/`shapefile` there to fetch a one-time asset would break that claim even if
  gated behind a bin-only target, since Cargo has no per-target dependency isolation within one
  package. A separate crate that nothing in the `app` dependency graph reaches keeps the
  invariant intact and matches what M1 1.2's decision log already anticipated: "[static-download
  hosts] are fetched by import tooling at setup time, not by `ingest` ... that tooling extends
  the list on purpose when it lands." One bin today (`import-basemap`); a natural home for a
  future OurAirports/FAA importer if one lands, but not built out ahead of that need.
- **The documented download host is dead; the real one is `naciscdn.org`.** docs/03 named
  `https://www.naturalearthdata.com/downloads/`, but that page's own direct file links
  (`.../download/50m/physical/ne_50m_land.zip`) return `404` — checked live with `curl -I`,
  not assumed. The same downloads page links to Natural Earth's actual CDN, `naciscdn.org`;
  both files confirmed there with a live `200` and a plausible size (~450 KB each, `AmazonS3`/
  `CloudFront` headers). docs/03 updated to record the real host rather than silently working
  around a stale doc. `ALLOWED_STATIC_HOSTS` in the import tool gates on it exact-match,
  https-only — the same shape as `ingest::allowlist`, even though this tool never ships.
- **`shapefile` crate over GDAL/`ogr2ogr`.** Natural Earth ships shapefiles, not GeoJSON
  directly. `ogr2ogr` would need a system GDAL install (a new, undocumented host dependency for
  anyone running this tool); the pure-Rust `shapefile` crate parses `.shp` bytes with zero
  system dependencies, matching the project's "bundled SQLite, no system dependency" bias
  (ADR-004's reasoning, extended here). The exact API (`ShapeReader::new`/`read_as`,
  `PolygonRing::Outer`/`Inner`, `Polyline::parts`, `Point { x, y }`) was confirmed by reading
  the vendored crate source under `~/.cargo/registry/src/`, not guessed from a tutorial or a
  possibly-stale doc page — the same discipline M0 0.6 established for wgpu. Only `.shp` bytes
  are read; `.shx`/`.dbf` are skipped entirely since this tool reads every shape once,
  sequentially, and wants no attribute columns — one less thing that could fail to parse.
- **The outer/inner ring grouping is a documented heuristic, not a format guarantee.** A
  shapefile `Polygon` record can hold several disjoint outer rings (e.g. a continent plus its
  islands packed into one record); GeoJSON's `Polygon` type allows exactly one shell, so each
  outer ring starts a new output feature and any inner (hole) rings attach to whichever outer
  ring immediately precedes them. The shapefile spec technically says ring order is
  insignificant, but every common shapefile writer — including Natural Earth's own toolchain —
  emits a shell immediately followed by its holes, so the heuristic matches reality; a
  point-in-polygon nesting analysis would be more format-correct but is unneeded complexity for
  data that, verified live, contains no holes needing it (land is 1,420 records → 1,421
  features — one extra feature from a single two-outer-ring record — with `total_point_count`
  matching exactly). Coastline parts need no such grouping: each shapefile part becomes its own
  `LineString` feature independently.
- **Coordinates rounded to 1e-4° (~11 m), no further geometric simplification.** 1:50m is
  already Natural Earth's own generalization tier (as opposed to 1:10m); the checklist's
  "simplified" reads as "use the simplified tier they publish," not "simplify further" — adding
  a Douglas-Peucker pass would be complexity the checklist didn't ask for and this data (1.2 MB
  each, ~60k points each, well under the 256 MB render-asset budget) doesn't need to justify.
  Revisit only if 2.2b's tessellation or the ≤ 2 s startup budget shows otherwise.
- **Verified live, structurally, never printing a coordinate into this session** (docs/06):
  1,420 land shapefile records → 1,421 polygon features, 1,428 coastline records → 1,429 line
  features; a scratch Node script (no `jq`/`python3` available here) checked feature counts,
  geometry-type histograms, total point counts, and lon/lat extents (exactly `±180°` and
  `[-89.9989°, 83.5996°]`/`[-85.1922°, 83.5996°]` — sane, no swapped axes, no garbage) without
  ever loading a raw coordinate into the transcript. 10 new offline unit tests (host gate,
  coordinate rounding, the two-outer-rings-in-one-record split, hole attachment, ring closure
  survival, polyline part splitting) — all synthetic, no network in `cargo test` per docs/10.
  `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` green.
  `crates/render/assets/basemap/README.md` records provenance, format, sizes, and the
  regeneration command. Next: **2.2b**, `lyon` tessellation + line/fill WGSL pipelines
  consuming this bundle.

## 2026-07-18 — M2 item 2.2b (base map render: tessellation + fill/line pipelines)

- **Reused `core::geo::web_mercator_forward` instead of a shader-side projection.**
  Tessellation runs once, on the CPU, at startup — so lon/lat → Web Mercator metres uses the
  same tested function `core::sim`/future camera code will (0.4's goldens already pin it),
  rather than duplicating the formula in WGSL, where a transcription slip would be invisible
  until the map looked subtly wrong. The result is further divided by
  `WEB_MERCATOR_EXTENT_M` to land in a normalized `[-1, 1]`-ish plane baked into the static
  vertex buffers; this normalization is fixed forever (it is a coordinate-system choice, not a
  camera), unlike the view-proj uniform below.
- **No camera exists yet (2.3), so `Renderer` carries a placeholder aspect-correcting
  fit-to-window matrix** in the same uniform-buffer/bind-group seam `msaa_view` already uses —
  rewritten in `reconfigure` on every resize, never rebuilt from scratch. 2.3 replaces what
  gets written into that buffer (a real pan/zoom transform); the buffer, bind group, and pipeline
  layout do not change. Keeping this seam explicit means 2.3 has nothing to restructure in
  `basemap.rs` or the pipelines, only in what feeds the one matrix.
- **`FillRule::NonZero`, not `EvenOdd` (lyon's default).** RFC 7946 and `import-basemap`'s own
  writer (2.2a) both use outer-CCW/hole-CW winding; `NonZero` is the fill rule that rule
  actually implies, and unlike `EvenOdd` it keeps working if two holes in one feature ever
  overlap. Verified, not assumed: a synthetic square-with-hole test asserts no output
  triangle's centroid falls inside the hole.
- **Coastline stroke width (`0.0015`, in the same normalized unit space) is a judgement call,
  not a physical one** — there is no camera/zoom yet to make "screen-space constant width"
  meaningful, so a fixed world-space width was picked by eye against the placeholder
  fit-to-window view and documented as revisit-worthy once 2.3 introduces zoom (a constant
  world-space width will look wrong at high zoom; screen-space width is the eventual answer).
- **Land/coastline palette (`#12161D` land, `#2E3742` coastline) is ours to pick**, the same
  position `clear_color`'s `#0A0E14` background was in at M0 0.6 — docs/01/docs/13 fix the
  *character* ("desaturated", "aircraft brightest") but not exact shades. Land sits barely
  above the background so the coastline stroke does the real land/ocean separation, not a
  strong fill contrast; both colors run through the same sRGB-linearize-if-needed path
  `clear_color` established, refactored into a shared `linearize_for_format` helper.
  `color.rs` gained a test asserting the brightness ordering background < land < coastline and
  that the palette stays dark throughout (docs/01's "aircraft are the brightest things on
  screen" only holds if the map itself never gets close).
- **One shared WGSL fragment entry point reading a per-pass `@group(1)` color uniform**, rather
  than two entry points with colors baked into the shader source — keeps `color.rs` the single
  source of truth for the palette; two `wgpu::RenderPipeline` objects are still built (one per
  layer) so either can diverge in blend/primitive state later without disturbing the other.
  Both are `TriangleList`: `lyon`'s stroke tessellator already emits triangles for the
  coastline, not a `LineList` primitive, so "line pipeline" means a pipeline for the stroked
  line geometry, not a `PrimitiveTopology::LineList` draw.
- **New dependency `lyon` (default features only — no `debugger`/`serialization`/`extra`) and
  `bytemuck` (`derive`, for the vertex/uniform buffer casts), both pinned in
  `[workspace.dependencies]`** rather than inline in `crates/render/Cargo.toml`, matching how
  every other dependency in this workspace is declared (root Cargo.toml's own header: pins
  live in one place, crates reference `.workspace = true`) — an inconsistency introduced mid-session
  by the delegated implementation and corrected before this item was called done.
  `serde_json` (already a workspace dependency) parses the bundled `GeoJSON` directly; a
  dedicated `geojson` crate was considered and rejected as an unneeded second JSON-modeling
  dependency for a shape this simple.
- **Delegated to the renderer-agent** (mid-session connection error interrupted the first
  attempt after only the `Cargo.toml` dependency additions had landed; resumed from the same
  agent transcript rather than restarting cold, per the SendMessage-resume pattern). This
  session independently re-verified rather than trusting the report: re-ran
  `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` fresh
  (**349 passed, 5 ignored, 0 failed** — matches the agent's own count exactly this time,
  unlike 2.1's), read every changed/new file in full, and independently drove a live
  `cargo run -p look-above` rather than accepting the agent's own screenshots. **That live
  check caught a real tooling pitfall worth recording**: `GetWindowRect` from a DPI-unaware
  PowerShell process reports logical pixels, not the physical pixels the renderer actually
  configures its surface at (winit is DPI-aware) — sizing a `PrintWindow` capture bitmap off
  the logical rect (1295×837) against a 1920×1200 physical surface produced a capture that
  looked like an asymmetric, cropped map, which briefly read as a possible aspect-correction
  bug in the new matrix math. Calling `SetThreadDpiAwarenessContext(-4)` (per-monitor-v2) in
  the capture script before any `GetWindowRect`/`MoveWindow`/`PrintWindow` call fixed it;
  re-captured at the true physical size and every resize (1920×1200, 1600×600 wide,
  500×1000 tall) showed correct symmetric letterboxing, no stretching, all continents
  recognizable, coastlines crisp, clean `WM_CLOSE` exit. Worth remembering alongside M0 0.8's
  own `FindWindow`/`MainWindowHandle` breadcrumb for any future scripted visual QA on this
  machine. Render crate: 5 → 12 tests. Next: **2.3**, the regional camera (pan/zoom, the real
  view-proj matrix this item's placeholder hands off to).

## 2026-07-18 — M2 item 2.3a (regional camera: pan/drag, cursor-anchored zoom, inertia)

- **Split 2.3 into 2.3a/2.3b before writing any code**, same shape as 2.1/2.1b and 2.2a/2.2b:
  the checklist's one line bundles the camera itself with exposing its viewport to the poller,
  but those are different lanes doing genuinely different things — pure `core`/`render`/input
  math here, versus a new `ingest::poller` retarget API and running the live network pipeline
  from window mode for the first time (only `--headless` does today) in 2.3b. 2.3b cannot be
  meaningfully written or tested without 2.3a's `Camera` to feed it, so the order is fixed.
- **Camera state lives in `core::camera`, not `render` or `app`.** It is pure arithmetic on
  `f64`s (pan/zoom/inertia math), a natural fit for `core`'s "types, geo-math" charter and
  reusable by both `render` (to build the GPU matrix) and `app` (to drive it from winit events)
  without a new cross-dependency. It imports nothing but `core::geo`'s existing `MercatorXy`/
  `WEB_MERCATOR_EXTENT_M` — no wgpu, no winit, no bytemuck.
- **Meters-per-pixel, not a unitless "zoom level"**, is the state variable. It makes the two
  hard parts fall out of ordinary arithmetic instead of needing lookup tables: cursor-anchored
  zoom is "solve for the new center that keeps one world point's screen position fixed as scale
  changes" (a few lines of algebra), and the zoom-out ceiling is "the world's fixed metre extent
  divided by the smaller pixel dimension" — the same formula that reproduces 2.2b's placeholder
  fit-to-window matrix exactly, so the camera's initial framing needed no separate case.
- **The zoom-out ceiling doubles as the initial framing, and neither is arbitrary: there is no
  L0 globe/orthographic view yet.** `max_meters_per_pixel = 2 * WEB_MERCATOR_EXTENT_M /
  min(width_px, height_px)` is the "whole projected world visible, letterboxed" fit; zooming out
  past it would show empty space with nothing to render into it, since only the regional Web
  Mercator camera exists (L0/L1 tier switching is 2.5+, not this item). Revisit this clamp when
  the globe view lands — it will need to hand off to an orthographic camera around this same
  threshold rather than just raising the ceiling.
- **"Inertia" was interpreted as pan-momentum-on-release plus eased (not momentum) zoom**, not
  literal continued zooming after the wheel stops. No mainstream map product keeps zooming after
  you stop scrolling; docs/01's actual requirement is smoothness ("no visible teleporting...
  during pan/zoom"), which an eased approach to a wheel-set target delivers without the odd feel
  of a zoom that outlives the input. Pan genuinely coasts and decays (drag velocity is sampled
  via an EMA during the drag, then integrated with exponential friction after release) because
  that is standard, expected map-drag feel and the checklist explicitly names it for pan.
- **`render::camera_view_proj` re-derives the scale from the camera's `meters_per_pixel` and
  divides by `WEB_MERCATOR_EXTENT_M` again**, rather than having `core::camera` produce
  pre-divided "plane units" itself. `basemap.rs`'s static vertices are already baked in that
  divided form (2.2b), so the alternative would mean either `core::camera` importing a
  render-flavored normalization convention it has no other reason to know about, or `render`
  trusting a second crate to have silently pre-divided its state correctly. Keeping the division
  in one place (`render`, right next to the vertices it must agree with) means there is exactly
  one spot that can get the constant wrong, and it is the spot that already owns the mesh.
- **`Renderer::reconfigure` stopped writing the view-proj buffer on resize.** It used to
  rewrite the placeholder matrix on every resize because it was the only thing that could — the
  camera now lives in `app`, which calls `Camera::resize` then `Renderer::set_view_proj`
  synchronously in its own `Resized` handler, before winit ever delivers the next
  `RedrawRequested`. A stale buffer is therefore never presented; a neutral fallback would have
  been unreachable dead code guarding against a sequencing winit doesn't allow.
- **Delegated in two lane-scoped, sequential pieces**: geo-math-agent for `core::camera` (pure
  math, run first since nothing else can be honestly written against an API that doesn't exist
  yet), then renderer-agent for the render/app wiring, briefed with the first agent's *actual*
  finished method signatures rather than the original spec, since a mismatch would not compile.
  Both independently re-verified rather than trusted: `cargo fmt --check`/
  `clippy --workspace --all-targets -D warnings`/`test --workspace` re-run fresh after each
  (**369 passed, 5 ignored, 0 failed** final — +14 `core::camera` tests, +6 `render` matrix
  tests over 2.2b's 349), every changed/new file read in full. One real ambiguity surfaced and
  resolved correctly by the first agent despite an imprecise brief: the task's prose said
  "shrinking a window must not leave the camera zoomed out past the new ceiling," which is
  backwards from the actual formula (shrinking *raises* the ceiling; growing lowers it) — the
  agent implemented the formula as specified (an unconditional `.min(max_mpp)` re-clamp, correct
  regardless of which direction the ceiling moves) and flagged the prose error explicitly rather
  than silently picking one, which is exactly the right call when code and English disagree.
- **Live-verified with a scripted Win32 drive** (`SetCursorPos`/`mouse_event`/`PostMessage`,
  DPI-aware per 2.2b's own recorded lesson — `SetThreadDpiAwarenessContext(-4)` before any
  window-geometry call): a drag pan followed the cursor correctly on both axes (verified against
  the derived sign convention, not just "something moved"), inertia coasted a short distance
  further after release and decayed to a stop without reversing, eight wheel notches in then
  eight back out round-tripped to the same view (no drift), a resize reflowed without distortion
  or a crash, and `WM_CLOSE` exited cleanly (code 0). Six screenshots confirmed no seams, cracks,
  or missing polygons at any step — docs/13's L2-core pan/zoom-inertia line. Next: **2.3b**,
  viewport→bbox exposed to the poller (retarget API + wiring window mode to the live pipeline
  for the first time).

## 2026-07-18 — M2 item 2.3b (viewport→bbox exposed to the poller; window mode runs the live pipeline)

- **Three pieces, in a fixed dependency order**: `core::camera::Camera::viewport_bbox() -> BBox`
  (the math), `ingest::poller`'s mid-run retarget API (the mechanism), then `app::window`'s
  wiring (the caller of both) — each genuinely needed the previous one's *finished* signature,
  same shape as 2.3a's two-piece split. `viewport_bbox` was small and self-contained enough
  (~20 lines, in a file already fully read this session) to write directly rather than delegate,
  per the token-managed-implementation skill's own threshold; the other two were delegated,
  lane-scoped and sequential (data-source-agent for `ingest`, renderer-agent for `app`, the
  second briefed with the first's real signatures).
- **`viewport_bbox` must clamp, not just project**: nothing in `Camera` constrains `center_m` to
  the projected world (no antimeridian wrap yet, per `BBox`'s own doc), and whichever pixel
  dimension is *not* the letterbox-constraining one already overflows past
  `±WEB_MERCATOR_EXTENT_M` at the default "whole world" framing — a landscape window overflows
  in x, a portrait one in y. Both corners of the viewport are clamped into the valid Mercator/
  lat-lon domain before `BBox::new`, which is provably always then constructible (clamping both
  endpoints of an already-ordered interval to the same range preserves the ordering) — proven by
  a dedicated test that pans a camera `1e9` px past the world's edge and asserts no panic and a
  non-inverted result, plus a test on the default (near-whole-world) framing and one confirming
  the bbox shrinks correctly as the camera zooms in.
- **The poller's retarget is a `tokio::sync::watch` channel, not a queue**: a retarget is "the
  latest desired region," and `watch::Sender::send` needs no `.await`, so it can be called
  directly from the winit thread. `run()`'s loop now races `sleep(tick.interval)` against
  `retarget.changed()` so a new region takes effect on the very next cycle instead of waiting
  out up to `MAX_INTERVAL` (a real 60 s at `OpenSky`'s slowest, which would make a camera pan
  feel broken for a milestone whose whole goal is "watchable"). **The footgun this required
  defending against explicitly**: once every paired `Sender` is dropped, `changed()` resolves
  `Err` *immediately, and forever after* — a `select!` that keeps racing it would busy-spin with
  zero delay between cycles, hammering the active source. `run()` tracks a `retarget_live` bool:
  the first `Err` still waits out one interval and disarms the channel from the `select!`
  permanently; both headless mode (which never retargets) and window mode keep their `Sender`
  alive for the process's life so this path is a defensive backstop, not the expected route.
  Proven under `#[tokio::test(start_paused = true)]` (the pacer's own established pattern):
  one test confirms a retarget sent mid-run changes the *very next* cycle's query with zero
  virtual time elapsed (proving the `select!` actually won the race, not that the test waited
  it out), another confirms a fully-dropped `Sender` degrades to exactly-paced cycles rather
  than a spin.
- **Window mode restores and persists the credit ledger exactly like headless mode does — not a
  stripped-down version.** The ledger is a real-world daily-quota safety cap (privacy rule 1.3),
  not a per-process bookkeeping nicety: without reading `source_status.credits_used_today` at
  startup and writing it back after every cycle, running headless and window mode on the same
  day (or window mode across two sessions in one day) would each track spend independently from
  zero, risking the *actual* OpenSky quota even with each process's own ledger looking fine. The
  merge/log/persist step itself was extracted out of `headless::record_cycle` into a new shared
  `app::pipeline::record_cycle`, so window mode doesn't duplicate it — `headless.rs` now calls
  the same function.
- **A real cross-crate compile break, caught by the second delegated agent, not by me**: the
  renderer-agent doing the `app::window` wiring found that `headless.rs` still called
  `Poller::with_default_chain` with a bare `RegionQuery` — the already-landed `ingest::poller`
  signature change (a `watch::Receiver<RegionQuery>`) meant the `app` crate did not compile at
  all until that call site was fixed too, even though `headless.rs` itself was explicitly listed
  as out-of-scope for the *first* agent (whose job was `ingest` only). Fixed in the same session
  by building an immediately-dropped `watch::channel` there — headless never retargets, and per
  the poller's own documented behavior a closed channel just falls back to the fixed cadence,
  which is headless's fixed-region behavior anyway. Worth remembering: splitting a signature
  change and its call sites across two delegated, sequentially-briefed agents leaves a real
  window where the crate doesn't compile in between — this time the second agent's own build
  caught it before it reached this session's independent verification, but the brief for the
  first agent should probably have said "and confirm `cargo build --workspace` still succeeds"
  rather than scoping verification to the `ingest` crate alone.
- **A real gap found in this session's own independent re-verification, not by either agent**:
  the brief specified detecting "camera changed" by comparing `(center_m, meters_per_pixel)`
  immediately before/after `camera.update(dt_s)` inside `draw()` — which the renderer-agent
  implemented exactly as specified and then correctly flagged, rather than silently patching,
  that a `WindowEvent::Resized` with no accompanying pan/zoom would never be observed by that
  comparison (a resize is fully applied by its own event handler *before* the next `draw()`
  runs, so `draw()`'s before/after snapshot never sees a delta even though `viewport_bbox()`
  genuinely changed with the new aspect ratio). Since aircraft don't render yet (M2 2.4/2.5),
  this had zero visible effect today but would matter the moment they do. Fixed directly in this
  session (small, ~6 lines, in a file already fully read): the `Resized` handler now also arms
  `last_camera_change_instant` itself, letting the existing settle-and-send path in `draw()`
  pick it up on its own. **Live-verified by accident, and more convincingly than a scripted
  resize would have been**: a real multi-minute gap in this session (an unrelated tool outage)
  left the running app alone for over an hour, during which something external resized its
  window several times with no pan/zoom input at all — the log shows five separate "retargeted
  mid-run" lines with five genuinely *different* bboxes (proving each was a real size change,
  not a no-op resize storm re-arming the same value), confirming the fix end-to-end rather than
  in isolation.
- **Live-verified end to end against the owner's real credentials** (`cargo run -p look-above`,
  `LOOK_ABOVE_LOG_FILTER=look_above=info,look_above_ingest=info,warn` to see both crates' info
  lines): window mode's first cycle fetched the whole-world default viewport (6,229 aircraft, 4
  credits — `OpenSky` bills a bbox query by area tier regardless of how large the bbox actually
  is, not the separate ~400-credit global-query tier, so this is an expected one-time cost, not
  a budget concern at 4 of 3,200/day), settled into the same budget-driven ~60 s cadence headless
  mode already produces, and — across the accidental long run above — correctly retargeted five
  times with shrinking/shifting bboxes as the window's real size changed, each followed by a
  poll cycle against the *new* region. Source stayed `opensky` the entire run (never failed
  over). Closed with `WM_CLOSE` → "close requested" → "window closed", no panic, exit clean.
  Scratch `look_above.db` deleted afterward, following 1.12/1.13's precedent.
- **Independently re-verified rather than trusted, both delegated pieces**: full diffs of
  `crates/ingest/src/poller.rs`, `crates/app/src/window.rs`, `crates/app/src/headless.rs`,
  `crates/app/src/main.rs`, and the new `crates/app/src/pipeline.rs` read in full (not
  skimmed); `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/
  `test --workspace` re-run fresh after the resize fix — **375 passed, 5 ignored, 0 failed**
  (+6 over 2.3a's 369: 4 new `core::camera::viewport_bbox` tests, 2 new `ingest::poller`
  retarget tests). Next: **2.4**, `core::sim` — the interpolation/dead-reckoning worker that
  finally gives the `SessionTable` both pipelines now maintain somewhere to render from.

## 2026-07-18 — M2 item 2.4a (`core::sim`: the pure interpolation/dead-reckoning engine)

- **Split 2.4 into 2.4a/2.4b before writing any code**, the same shape as 2.1/2.1b, 2.2a/2.2b,
  and 2.3a/2.3b. 2.4 as written bundles three things: the pure `core` interpolation math, the
  double-buffer handoff between the worker producer and the render-thread consumer (ADR-002),
  and the app-loop wiring that runs `advance_all` at render cadence and feeds it from the live
  `SessionTable`. The first is a self-contained geo-math lane that nothing else can be written
  or tested against until it exists; the latter two are app/render plumbing that depend on its
  finished API. Nothing visible renders from the feed until 2.5's glyph pipeline regardless, so
  2.4b's verification is a logged instance count, not a picture — no honesty cost to deferring
  it. 2.4a is this item.
- **`core::sim` is stateful (one `Track` per aircraft), not a bag of pure functions**, because
  the correction blend needs memory: on a new fix it eases from *where the aircraft is currently
  shown* to the new fix, so the last displayed position must persist frame-to-frame. The pure
  helpers (`dead_reckon`, `ease_out`, `geodesic_lerp`, `blend_heading_deg`, `alpha_from_age`,
  `AltitudeBucket::classify`) are factored out and unit-tested in isolation; `Track::advance`
  composes them.
- **Two entry points at two rates, not one.** `ingest(states, now_s)` is called per poll cycle
  (5–60 s): a record whose `ts` is strictly newer than the held fix installs it and starts a
  blend, older-or-equal is ignored. That last rule matters — the caller (2.4b) will feed the
  whole `SessionTable` snapshot each cycle, which re-sends the same fix until a newer one
  arrives, and a naïve "any ingest starts a blend" would restart the ease-out every frame and
  freeze the aircraft in place. `advance_all(now_s)` is called per frame and does all the
  motion. The split mirrors how fixes and frames actually arrive.
- **`advance_all` is a rayon `par_iter_mut` over the track table** (ADR-002 / the skill's
  performance recipe: "advance all aircraft in a rayon parallel iterator … results written into
  the render buffer"). Each track advances independently — no shared mutable state — so the
  parallelism is embarrassing and needs no synchronization. The feed is then sorted by ICAO24
  address for determinism (reproducible tests and a stable draw order); a *real* draw-priority
  order (altitude, then selection) is 2.5/2.8's concern and noted as such.
- **All sim state is `f64` and `Copy`; the renderer narrows to `f32` at 2.5.** Keeping the
  narrowing out of `core` means `core` carries no render-specific numeric convention (the same
  reasoning 2.3a used to keep the `/ WEB_MERCATOR_EXTENT_M` normalization in `render`), and it
  also sidesteps the `cast_possible_truncation` pedantic-clippy lints that an f64→f32 in `core`
  would otherwise trip. The one unavoidable cast — `i64` seconds-since-epoch → `f64` — is a
  single `const fn` with a scoped `#[allow(clippy::cast_precision_loss)]` and a comment noting
  epoch seconds (~1.7×10⁹) and the tens-of-seconds horizons are all exact in f64's 2⁵³ integer
  range. Same discipline `frame_stats::percentile` used (integer arithmetic to dodge float-cast
  lints), applied the other way.
- **The dead-reckoning Δt clamp `[0, DROP_AFTER_S]` is defensive and, for a *visible* aircraft,
  unreachable — so it is tested on the private `dead_reckon` directly, not through the feed.** A
  track fully fades out at `STALE_AFTER_S + FADE_DURATION_S` = 65 s, so no drawn aircraft ever
  has an age near the 90 s clamp; the clamp only guards a wild clock or a source-clock skew
  (negative Δt must hold position, not rewind the aircraft). Testing it through `advance_all`
  would be impossible (the aircraft is gone from the feed by then), which is itself the tell that
  it belongs in a direct unit test.
- **The no-backward-along-track invariant is enforced by clamping, not velocity-blending.** The
  skill says a new fix behind the shown position must "slow the shown aircraft … instead of
  reversing." Implemented as: project each frame's candidate displacement onto the fix's track
  bearing; if the along-track component is negative, keep the previous position. This is "slow to
  a full stop until the (still-advancing) target catches up," a faithful and directly testable
  reading of the rule (the test asserts the along-track coordinate is monotonic non-decreasing
  across frames). A smoother speed-blend is a candidate refinement, noted, not needed for
  correctness.
- **Teleport (> 10 km error) snaps at the fade midpoint, not at the window start.** The glyph
  alpha dips symmetrically (1 → 0 → 1) over 300 ms and the position jumps only once alpha has
  reached 0, so the eye never sees either a slide across the map or a pop — the quality bar's
  "no visible teleporting." Below the threshold it is an ordinary ease-out slide.
- **Stale-fade constants are reused from `core::merge`, not redefined.** `STALE_AFTER_S`(60) and
  `DROP_AFTER_S`(90) already exist there (item 1.9) precisely as the render layer's "begin fade"
  and "stop extrapolating" points; `sim` imports them and adds only `FADE_DURATION_S`(5). An
  instance leaves the feed at 65 s (alpha 0) but its `Track` is retained until 90 s, so a
  reacquisition inside that window blends from the last shown position rather than popping back
  in as a fresh sighting — and it keeps `sim`'s own drop horizon aligned with the `SessionTable`
  the app feeds it from, avoiding a re-create-then-fade flicker at the seam.
- **`RenderFeed` is introduced incrementally (aircraft first).** docs/09's full shape carries
  `trails` and `labels` too, but those types' shapes belong to 2.6/2.7; defining them empty now
  would either invent premature types or leave dead `Vec`s. `RenderFeed` is `frame_ts` +
  address-sorted `aircraft` for now, with a doc note that the other two fields are appended by
  their own items — the same append-only, land-it-when-needed approach the `store` migrations and
  the `SourceError`/`AircraftCategory` taxonomies took. This is a seam type (docs/09), so it is
  logged here; the change is purely additive to a not-yet-implemented contract.
- **`AircraftInstance.category` is `AircraftCategory::Unknown` for now**, because `StateVector`
  carries no category — it arrives from adsbdb/registry enrichment in M3. The field is present so
  the instance shape is complete for the 2.5 glyph pipeline; wiring a real category is M3/2.5.
- **Done directly, not delegated to the geo-math-agent, despite the lane matching.** CLAUDE.md
  names geo-math-agent for projection/interpolation, and the token skill says delegate a
  one-lane subtask *when it would force reading files otherwise not needed*. Here the opposite
  held: `geo.rs`, `types.rs`, `merge.rs`, and `contracts.rs` were all read in full this session
  while scoping 2.4, so a cold subagent would only re-derive context already in hand and add a
  verification round-trip. Implementing it directly was the cheaper, tighter path for a
  formula-heavy module where the skill's math had to be matched exactly.
- **Verified by the unit suite, no live run.** 20 new tests, at least one per docs/10 §1 line:
  advance-along-track at ground speed, vertical-rate integration across a band boundary in both
  signs, blend convergence within the window (and no jump at u=0), the no-backward invariant, the
  teleport snap + alpha dip, stale-fade timing + reacquisition + drop, the Δt clamp and no-rewind,
  missing-speed/missing-track holds, on-ground non-extrapolation, altitude-bucket boundaries, and
  the ease-out/heading/geodesic helpers. `cargo fmt --check`/`clippy --workspace --all-targets -D
  warnings`/`test --workspace` all green — **394 passed, 5 ignored, 0 failed** (+19 over 2.3b's
  375, all in `core::sim`). No app behavior changed and nothing renders the feed yet, so there is
  no runtime surface to drive (the verify skill's explicit "nothing to observe" exception); the
  feed becomes live and visually checkable at 2.4b/2.5. Next: **2.4b**, the double buffer +
  app-loop wiring.

## 2026-07-18 — M2 item 2.4b (`core::sim` wiring: double buffer + simulation worker thread)

- **The producer is a dedicated worker thread, not the render loop calling `advance_all`.**
  ADR-002 and the high-fidelity-flight-visualization skill are explicit: "results written into
  the inactive render buffer, swapped atomically at frame start; the render thread never computes
  any of the above." `advance_all` *is* interpolation/projection, so it cannot run on the render
  thread even though its internals are rayon-parallel — orchestrating it there would still block
  the frame on production. New `app::simulation` owns a `std::thread` that drains poll batches,
  runs `core::sim` at ~60 Hz (a `FRAME_BUDGET` of 16,667 µs, sleeping the remainder so a quiet
  sky does not spin a core), and publishes each feed. The render thread only swaps and draws.
  This also moved the `SessionTable`/`Writer`/batch-receiver *off* the render thread — 2.3b had
  them draining inside `draw`, which was acceptable only because nothing consumed the merged
  table yet; 2.4b is where that would have started blocking frames, so it moves now.
- **The double buffer is a latest-wins SPSC mailbox, not a queue** (`app::double_buffer`). A
  feed the consumer never reads has no value once a newer one exists, so `Producer::publish`
  overwrites any unconsumed feed rather than buffering it, and `Consumer::take_latest` returns
  `Option<T>` — `None` means "nothing new since last frame", and the render thread keeps showing
  the front buffer it already holds (`App::current_feed`) so the picture never blanks between
  publishes. Those two held buffers (the consumer's current one + the one in the slot) are the
  two of the double buffer. Implemented over `Arc<Mutex<Option<T>>>`; the render-thread lock is
  one uncontended `take()` per frame (microseconds), well inside ADR-002's frame budget, so a
  lock-free crate (triple-buffer/arc-swap) would be a dependency earned by nothing. A poisoned
  lock is recovered (`PoisonError::into_inner`), not unwrapped — the slot holds only plain data,
  so at worst the other side sees a stale value, never a torn one, and that beats taking the
  render thread down with it (also keeps the no-`unwrap` rule).
- **The simulator is fed the whole `SessionTable` each poll cycle, not just the new batch.**
  `Simulator::ingest` ignores a fix not newer than the one it holds (2.4a's older-or-equal
  guard), so re-installing the full deduped picture every cycle is safe: only the aircraft that
  cycle actually refreshed start a correction blend, and a re-sent identical fix is a no-op.
  Feeding the merged table (not the raw batch) is what carries the dedup + sticky-anonymity the
  merge already applied. Re-sync only fires on a cycle that delivered a batch; between cycles the
  worker just `advance_all`s (dead reckoning).
- **Window mode evicts the table at `DROP_AFTER_S` before each ingest; headless does not.**
  Left unbounded, the table would keep frozen entries for aircraft that left the region forever,
  and re-feeding one the simulator had already dropped (past 90 s) would re-create a track that
  is faded-out and immediately dropped again — churn. Evicting at the simulator's own drop
  horizon keeps the fed picture bounded and the two horizons aligned. This lives in the sim
  worker, deliberately *not* in the shared `pipeline::record_cycle`: headless's per-cycle
  `stale`/`tracked` readout (items 1.12/1.13) is documented evidence, and folding eviction into
  the shared path would zero its stale count and change that readout's meaning.
- **`RenderFeed` is *handed to the render thread*, which logs its instance count — the buffer is
  not yet plumbed into `Renderer::render`.** The item says "hand the buffer to the renderer";
  the render thread (the thread that owns and drives the `Renderer`) receives the swapped feed
  and logs `instances = current_feed.aircraft.len()`. Passing `&RenderFeed` into
  `Renderer::render` was deferred to 2.5, when the renderer has a glyph pipeline to upload it
  into — adding the parameter now would be a dead API on the `render` crate, the same way the
  `instances=0` reporting path was wired ahead of the thing it counts (2.1). The logged count is
  2.4b's verification and it is exact: live, the first whole-world OpenSky cycle showed
  `tracked=6468 stale=776` and the very next frame-stats line read `instances=5692` — i.e.
  `tracked − stale`, the sim's own fade/stale gating, confirming the feed reaching the render
  thread tracks the live table rather than a stale or fabricated number.
- **Clean shutdown joins the worker before the store is torn down.** The worker owns the only
  window-mode `Writer` clone, so `App::exiting` signals an `AtomicBool` and joins the thread —
  flushing the last cycle's DB writes — before dropping the renderer/window. Signal-then-join,
  because the loop checks the flag once per iteration; live, `close requested → window closed`
  took 58 ms (well under one poll cycle). No graceful protocol beyond that: winit delivers the
  close and the join happens synchronously in the exit callback.
- **Verified live against the owner's real `credentials.json`** (2× short window-mode runs,
  2026-07-18, Intel Arc / DX12, 1920×1200). Initial whole-world region → first OpenSky cycle 4
  credits, `instances` stepped 0 → 5692 and thereafter tracked each cycle's updates/drops
  (~5650–5710); render held a steady ~180 fps / 5.5 ms mean throughout, confirming the double
  buffer decouples the render thread from production (the sim runs on its own thread, the frame
  loop never blocks on it). Second run drove a real `WM_CLOSE` (via the process
  `MainWindowHandle` — `FindWindow` by title returned 0 in the first attempt, a
  verification-tooling quirk on this machine like 2.2b's DPI one, not an app fault) and observed
  the clean `close requested → window closed` join. ~24 credits total across both runs
  (4/cycle × ~6 cycles), well under the 3,200 cap. `cargo fmt --check`/`clippy --workspace
  --all-targets -D warnings`/`test --workspace` all green — **402 passed, 5 ignored, 0 failed**
  (+8 over 2.4a's 394: 4 `double_buffer`, 4 `simulation`). Scratch `look_above.db` deleted after,
  per 1.12/1.13's convention. Next: **2.5**, the aircraft glyph pipeline (SDF atlas, instanced
  quads) — the first item to actually draw the feed 2.4b now delivers.

## 2026-07-19 — M2 item 2.5 (aircraft glyphs)

- **The SDF atlas is generated procedurally at startup, not loaded from a bundled/fetched
  asset.** docs/01 says "SDF glyph atlas", which reads as pre-made art, but no image/font/SVG
  crate exists anywhere in this workspace and `render` must stay self-contained (no network, no
  filesystem assets beyond the basemap GeoJSON already bundled — ADR-002). Reaching for a new
  crate (an SVG rasterizer, a font-SDF baker) to draw six simple category silhouettes would be
  the premature-abstraction/dependency-weight the token-management rules warn against for a v1
  "distinguishable at a glance" bar. Instead `crates/render/src/glyph_atlas.rs` hand-authors six
  simple polygon silhouettes (plain `(f32, f32)` point lists) and rasterizes each into a 64×64
  `R8Unorm` tile via ray-casting point-in-polygon + point-to-segment distance (standard SDF
  convention: `0.5` at the edge), packed into one static `384×64` strip uploaded once. A genuine
  deviation from the doc's literal wording, made and recorded rather than silently substituted.
- **Six silhouettes, evocative not literal**: jet swept/delta, turboprop and piston/light both
  straight-winged (piston's wing set further forward and narrower for a "high wing" read),
  glider the widest span with the thinnest fuselage, helicopter a circular rotor disc unioned
  with a small tail-boom rectangle (signed-distance union via `max`, the mirror of the usual
  negative-inside SDF union's `min`), unknown a plain 4-point dart. "Distinguishable, not
  pretty" is the explicit v1 bar (docs/01/skill) — these are not meant to be revisited as an
  embarrassment once real artwork exists, only once a real asset pipeline is worth building.
- **Every aircraft draws as one fixed-size L2-style glyph; LOD tiers are out of scope for this
  item.** docs/01 specifies three zoom tiers (L0 density dots > 3,000 km, L1 small glyphs
  300–3,000 km, L2 full glyphs < 300 km) with hysteresis and cross-fade, but no M2 checklist
  item (2.1 through 2.10) actually implements tier switching — 2.3a already scoped the camera
  itself to regional-only, no L0 globe view. `aircraft::AIRCRAFT_GLYPH_PX` (20 px, docs/01's
  16–24 px L2 range) is converted to a world-space scale from the camera's `meters_per_pixel`
  every frame, so the glyph stays a constant screen size at any zoom — but there is no
  density-dot or small-glyph representation at any distance. **This is a real gap, not a
  deferred nice-to-have**: docs/13 §L2-core's zoom-out-to-globe check is part of the M2 gate
  (2.10), and nothing in 2.1–2.9 will produce the L0/L1 behavior it tests. A future milestone
  item (LOD tier switching + cross-fade) needs to exist before 2.10 can honestly run that line —
  flagged here at 2.5 rather than discovered cold at the gate.
- **Rotation is "clockwise from geographic north" and "clockwise on screen" via one formula,
  because no axis flip sits between them.** Web Mercator's `+y` (north) and clip space's `+y`
  (up) point the same way (`camera_view_proj` never flips an axis), so
  `aircraft::rotate_clockwise_from_north` — mirrored exactly in `aircraft.wgsl`'s vertex shader,
  since WGSL isn't unit-testable from `cargo test` — serves both without a sign correction. Pinned
  by four cardinal-point tests (0°/90°/180°/270° → north/east/south/west).
- **Instance packing (Mercator metres → normalized plane, `f64` → `f32`, altitude bucket → tint,
  stale-fade alpha folded into `tint.a`) happens on the render thread inside `Renderer::render`,
  not in `core::sim`.** `core::sim`'s own module doc already said this narrowing was deferred to
  2.5; per ADR-002 the render thread's job is "swap buffer, upload instances, ... nothing else",
  and packing an already-computed feed into GPU-ready bytes is upload prep, not simulation —
  `core` stays render-convention-free (`f64`/`Copy` throughout, as 2.4a left it).
- **`Renderer::render` gained a signature** (`feed: &RenderFeed, meters_per_pixel: f64`) after
  2.4b deliberately left it parameterless (nothing to draw yet). `Renderer::new` now builds one
  shared view-proj `BindGroupLayout` object handed to both the base-map and aircraft pipeline
  builders, so the one `basemap_view_proj_bind_group` can be passed into every pass's draw call
  — wgpu rejects a bind group against a pipeline built from a merely structurally-identical but
  distinct layout object, so this had to be one shared object, not two equivalent ones.
- **Altitude-bucket tints are flat placeholder colors, not the skill's Oklab-interpolated ramp**
  — `color::altitude_bucket_tint` wires the skill's six hex stops directly (through the existing
  `linearize_for_format` helper), per the checklist's own parenthetical that the perceptual ramp
  lands at M4. Buckets are wired now so the attribute exists and is visibly distinguishable
  (verified live: cyan/green/amber/violet visible across busy regions), not because this is the
  final palette.
- **Delegated to the renderer-agent** (glyph/SDF atlases and instanced pipelines are named
  exactly in its remit) with the atlas-generation and LOD-out-of-scope calls above already made,
  so the agent implemented rather than re-decided them. **Interrupted mid-task by a session
  API/rate-limit error** right after the design was settled and before any file had been
  written; resumed the same agent via `SendMessage` from its own transcript rather than
  restarting cold — the same recovery path 2.2b used for its own mid-session connection error.
- **Independently re-verified rather than trusted**: every new/changed file
  (`glyph_atlas.rs`, `aircraft.rs`, `aircraft.wgsl`, `color.rs`, `renderer.rs`, `lib.rs`,
  `app::window.rs`) read in full by this session, `cargo fmt --check`/`clippy --workspace
  --all-targets -D warnings`/`test --workspace` re-run fresh — **420 passed, 5 ignored, 0
  failed** (+18 over 2.4b's 402: 9 `aircraft.rs`, 5 `glyph_atlas.rs`, 4 new in `color.rs`),
  matching the agent's own reported count exactly. **Live-verified independently**, not just via
  the agent's own screenshot: `cargo run -p look-above` against the owner's real
  `credentials.json` (Intel Arc/DX12, 1920×1200) — a whole-world OpenSky cycle
  (`tracked=13,307`, 4 credits, ledger already at 16/3,200 before this run) rendered distinct,
  differently-rotated dart glyphs (category is always `Unknown` pre-M3 enrichment, as expected)
  tinted by altitude bucket over the dark desaturated map, aircraft clearly the brightest things
  on screen; a scripted zoom-in attempt did not visibly change the view (a cursor-focus
  scripting quirk in this session's own screenshot tooling, not exercised further — the
  world-view screenshot already showed everything 2.5 needed to prove) and was not chased
  further. Clean `WM_CLOSE` exit (`close requested → window closed`, ~70 ms). Two extra stray
  window instances turned up afterward from this session's own earlier failed screenshot-script
  launch attempts (not an app bug); closed the same way before the scratch `look_above.db` was
  deleted per 1.12/1.13's convention. Next: **2.6**, trails.

## 2026-07-19 — M2 item 2.6a (`core::sim` trail ring buffer)

- **Split 2.6 into 2.6a/2.6b before writing anything**, same shape as every prior M2 item. The
  checklist bundles the pure ring-buffer/sampling math with the render-side ribbon tessellation
  and WGSL pipeline, but the ribbon-widening (perpendicular offset, screen-constant taper) needs
  the camera's live `meters_per_pixel`, and `core` has no camera by design (2.3a deliberately
  kept it in `app`, ADR-002's dependency direction) — so that half can only honestly be written
  render-side, the same way 2.5 kept the glyph's zoom-dependent on-screen sizing
  (`aircraft::glyph_scale_normalized`) out of `core` entirely.
- **Trail vertices are flat centerline samples, not pre-widened ribbon geometry.** `TrailVertex`
  carries a projected `position`, a per-sample `altitude_bucket`, and a raw `age_s` — no width or
  screen-space offset. Deferring the perpendicular-offset math to 2.6b (which will pack it on the
  render thread, the same pattern 2.5's `pack_instance` already established for per-frame CPU
  work that isn't heavy simulation) keeps `core` free of any render-specific convention, matching
  `AircraftInstance`'s own `f64`/`Copy`-only shape.
- **Trail samples are recorded from `Track::display` (the post-blend, post-no-backward-clamp
  position), not the raw fix** — the skill's explicit "ring buffer of the last 5 min of
  *displayed* positions (so trails inherit smoothness)". A teleport's fade-hidden midpoint snap
  is therefore never recorded either, since sampling is gated on the instance being visible
  (`alpha > 0`) and the teleport dip can (briefly) reach the same invisible state the stale fade
  uses.
- **Sampling only happens while the instance is visible this frame.** An aircraft not shown has
  no "displayed position" to record; recording anyway during an invisible stale-fade gap would
  fabricate a trail point for a moment nothing was actually drawn. This means a reacquisition
  after a fade leaves a real gap in the trail rather than a phantom straight segment across the
  gap — accepted as correct per the skill's wording, not chased further (2.6b's ribbon build
  will simply start a new run rather than bridging it).
- **Altitude bucket is classified per-sample from that sample's own recorded altitude/on-ground
  state, not the track's current one.** The skill says trails are "colored by the altitude
  ramp" per vertex; storing only the track's live bucket at each `advance_all` call would make a
  climbing aircraft's whole trail repaint to its current color every frame instead of showing
  its actual historical bands. `TrailSample` therefore keeps `alt_m`/`alt_known`/`on_ground`
  alongside the position so `AltitudeBucket::classify` can be re-run per sample at emission time.
- **`Simulator::advance_all` collects `(AircraftInstance, Vec<TrailVertex>)` pairs, sorts by
  address, then splits** — rather than collecting `aircraft` and `trails` as two independent
  parallel passes. This is what guarantees `RenderFeed.trails` stays contiguous per aircraft in
  the same order as `RenderFeed.aircraft` without an explicit run-length or index field: 2.6b's
  render-side ribbon build can assume "a run of identical `icao24` is exactly one aircraft's
  trail" and never needs to search for boundaries.
- **Dropped `Track`'s `Copy` derive, kept `Clone`.** The new `VecDeque<TrailSample>` ring-buffer
  field owns a heap allocation, so `Copy` no longer applies; nothing in the module ever
  duplicated a whole `Track` by value (only mutated it in place through the tracks `HashMap`), so
  this cost nothing.
- **Done directly, not delegated** — this session had already read all of `sim.rs`, `geo.rs`,
  and `types.rs` in full while orienting on the M2 plan and the visualization skill before
  writing any code, so handing the lane to a cold subagent would only force it to re-read files
  already in this session's context (2.4a's own precedent for the same call, per the
  token-managed-implementation skill's delegation rule).
- **7 new unit tests, all in `sim.rs`**: sample-interval throttling (computing each probed time
  fresh from the same base rather than accumulating with `+=`, so the assertion doesn't depend on
  floating-point drift staying under the 1 Hz threshold); 5-minute eviction; no sampling while
  invisible (reacquisition adds exactly one new sample, never a phantom one for the gap);
  per-vertex altitude bucket reflecting a sample's own historical altitude; trail contiguity/
  order matching the sorted aircraft list; a track past `DROP_AFTER_S` carrying no trail into the
  feed. `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` all
  green — **427 passed, 5 ignored, 0 failed** (+7 over 2.5's 420). No live run: pure library math
  with no runtime surface until 2.6b wires a consumer, same as 2.4a. Next: **2.6b**, the
  render-side ribbon tessellation + WGSL trail pipeline.

## 2026-07-19 — M2 item 2.6b (trails render: ribbon tessellation + WGSL pipeline)

- **CPU triangle-list tessellation, not GPU-instanced segments or a `TriangleStrip` primitive.**
  The checklist says "triangle-strip ribbons"; the design note says "CPU packing on the render
  thread, same pattern as 2.5's `pack_instance`". Both point at building ribbon geometry on the
  CPU. Chose a **triangle list** (`trail::tessellate_trails` → a per-frame, dynamically-grown
  vertex buffer, one non-indexed `draw`) over the two alternatives: (a) a real `TriangleStrip`
  topology would need one draw call per aircraft, or a primitive-restart index between aircraft —
  more moving parts for no gain at this vertex volume; (b) GPU-instanced line segments (one
  instance per segment, quad built in the shader) is compact but double-blends at every joint,
  which is visible on an alpha-blended pass. The triangle list is the simplest faithful reading
  and is fully unit-testable (no WGSL geometry logic to leave unexercised).
- **Continuous ribbon with shared joint vertices, not independent per-segment quads.** Each
  centerline vertex gets one left/right offset point (offset by ±half-width along the averaged
  perpendicular), and adjacent segments *share* those points. That is what avoids both a gap and —
  the reason it matters for an alpha-blended pass — an overlapping (double-blended) sliver at each
  joint that would read as a bright bead at every 1 Hz sample. Cost: a miterless join pinches the
  width slightly at sharp turns, negligible because the no-backward-along-track invariant in
  `core::sim` already keeps trails near-straight at 1 Hz.
- **Taper is a pure function of each vertex's `age_s`, computed on the render side.** Width
  `3 px → 0.5 px` (`taper_width_px`) and alpha `0.8 → 0` (`taper_alpha`) linearly over
  `[0, TRAIL_DURATION_S]`, both clamped; the half-width is converted to normalized-plane units the
  same "pixels → world metres ÷ extent" way `aircraft::glyph_scale_normalized` does, using the
  camera's live `meters_per_pixel` (2.6a deliberately left `age_s` raw, not pre-normalized, so a
  trail shorter than 5 min still taper-maps against the full scale rather than its own history).
- **Coincident consecutive samples are dropped before building.** A stationary aircraft (on the
  ground, or holding) records repeated identical displayed positions; a zero-length segment has no
  travel direction (NaN normal). Dropping the newer of any pair closer than `MIN_SEGMENT_LEN_SQ`
  (~2 cm on the ground, far below the metres a moving aircraft covers per 1 Hz sample) makes every
  surviving segment well-defined; a run that collapses to `< 2` distinct points is a dot, not a
  ribbon, and draws nothing.
- **New `trail.rs` (pure, testable) + `trail.wgsl` (pass-through) + a `TrailLayer` in
  `renderer.rs`**, mirroring 2.5's split of `aircraft.rs` (CPU math) / `aircraft.wgsl` /
  `AircraftLayer`. The shader carries no geometry: every vertex arrives already offset and already
  colored, so `trail.wgsl` only applies the shared `@group(0)` view-proj matrix and passes the
  color through. The pipeline reuses the *same* `view_proj` `BindGroupLayout` object the base-map
  and aircraft passes were built from (the one 2.5 introduced), so one bind group still serves
  every pass. Alpha-blended like the aircraft pass (the taper alpha needs it), unlike the opaque
  base-map passes. The instance/vertex buffer grows exactly like 2.5's (×2-or-exact, min
  `MIN_TRAIL_VERTEX_CAPACITY`), with a reused `vertex_scratch` so a warmed-up frame never
  allocates (ADR-002).
- **Draw order (docs/01): trails go *before* the aircraft glyphs** (map base → map lines → trails
  → aircraft → labels → UI), so a glyph is never occluded by its own trail. `Renderer::render`'s
  signature is unchanged from 2.5 (`feed: &RenderFeed, meters_per_pixel: f64`) — it already
  carried everything 2.6b needs; the trail pass just consumes `feed.trails` and the same
  `meters_per_pixel`.
- **Delegation: done directly, not delegated.** This session had already read `sim.rs`,
  `aircraft.rs`, `renderer.rs`, `aircraft.wgsl`, and `color.rs` in full while implementing 2.6a
  and orienting on 2.6b, so a cold renderer-agent would only re-derive files already in context
  (2.4a/2.6a's precedent).
- **9 new unit tests, all in `trail.rs`**: the two taper curves (head/tail values + clamp past the
  tail so width never goes negative and alpha never goes below 0); half-width positivity and
  linear scaling with both pixels and zoom; a straight run widening into a ribbon offset purely
  perpendicular to travel, each vertex by its own age's half-width (head wider than tail); head
  vertices more opaque and colored by *their own* bucket while tail vertices carry the older
  sample's bucket/alpha (per-vertex coloring, not one repeated color); a single-sample run and a
  stationary coincident-sample run both producing no geometry; each aircraft's run tessellated
  independently (the run boundary respected, no phantom segment stitching one tail to the next
  head); and the output buffer being cleared-and-reused rather than appended. `cargo fmt --check`/
  `clippy --workspace --all-targets -D warnings`/`test --workspace` all green — **436 passed, 5
  ignored, 0 failed** (+9 over 2.6a's 427, all in `render::trail`).
- **Live-verified** against the owner's real `credentials.json` (Intel Arc / DX12,
  `Bgra8UnormSrgb`, 1920×1200): scripted a wheel-zoom anchored over central Europe, which
  retargeted the poller to a lat 47.7–49.7 / lon 5.6–10.5 bbox (~187 aircraft, updated each
  cycle). The zoomed-in frames showed each altitude-colored dart glyph trailing a **tapered,
  altitude-ramp-colored ribbon** behind it — cyan/green/amber matching each aircraft's own band,
  thinning and fading toward the tail, with the glyph drawn on top of (never occluded by) its own
  trail. No wgpu validation errors or panics anywhere in the run; clean `WM_CLOSE`
  (`close requested → window closed`). ~17 credits across the run (spent_today reached 36, far
  under the 3,200/80% cap); scratch `look_above.db` deleted after per 1.12/1.13's convention.
  (A late capture that landed during `WM_CLOSE` teardown showed the view briefly back at
  whole-world — a capture-timing artifact, not a trail issue: the camera/view-proj path was
  untouched by 2.6b, and the retarget log shows the camera held the Europe bbox the whole run.)
- **Trails inherit 2.5's flagged LOD gap, plus a per-frame tessellation-cost concern.** At
  whole-world zoom the constant-3px-width trails of hundreds of aircraft pile into a colored blob
  (the same "no L0/L1 tier" gap 2.5 flagged for glyphs — docs/13 §L2-core / the M2 2.10 gate), and
  the CPU re-tessellates *every* visible trail *every* frame, unbounded by zoom — cheap at a
  regional viewport (the poller's bbox keeps the feed small there) but a real cost at a
  whole-world viewport with accumulated trails. Both resolve with the same future LOD item that
  2.5 already flagged (draw trails only at L2); noted here so the trail cost is on record for that
  item rather than discovered at the gate. Next: **2.7**, labels.

## 2026-07-19 — M2 item 2.7a (label content on `AircraftInstance`)

- **Split 2.7 → 2.7a/2.7b before writing anything**, same shape as every prior M2 item. Two
  independent reasons, not one: (1) label *content* (callsign/FL/kt) is plain per-fix data with no
  camera dependency, while *placement and collision culling* are inherently screen-space and need
  the camera `core` deliberately doesn't have (2.3a's boundary — the same reason 2.6a/2.6b split
  trail sampling from ribbon widening); (2) `AircraftInstance` didn't carry callsign, raw altitude,
  or ground speed at all before this item — only the coarse `altitude_bucket` — so nothing on the
  render side could be honestly written yet regardless.
- **Deviated from docs/09's literal `RenderFeed.labels: Vec<Label>` field, deliberately.** That
  contract types labels as "pre-collision-culled" and "built by the interpolation stage" — i.e.
  in `core`. But collision culling and placement need pixel-space viewport geometry, which only
  `render`/`app` have; folding that into `core::sim` would mean teaching `core` about the camera,
  which ADR-002/2.3a already ruled out. Chose instead: `core` carries label *content* as new
  fields directly on `AircraftInstance` (no new `Label`/`RenderFeed.labels` type), `render` (2.7b)
  owns everything screen-space. Recorded here rather than silently diverging from the typed
  contract in docs/09 — same category of call as 2.5's atlas-generation deviation and 2.6a/2.6b's
  own split.
- **`callsign` is sticky across fixes that omit it.** Identification messages and position reports
  arrive on separate cadences in the real feeds (`docs/09`'s adapters already tolerate nulls
  per-field); if a later fix's blank callsign cleared a previously known one, the label would
  flicker to "no callsign" and back on every other poll cycle. A fix's callsign only *replaces*
  the held one when it actually carries one — verified by a dedicated test distinguishing "blank
  doesn't clear" from "a real new value does replace."
- **`altitude_ft` is `Some(0.0)` on the ground, not `None`.** "0 ft while on the ground" is real
  data, not an unknown field — gating it away in `core` would be a formatting decision (2.7b's
  job: should a taxiing aircraft's label show `FL000`? probably not), not a data-availability one.
  `core` only reports `None` when the fix genuinely never carried an altitude.
- **`ground_speed_kt` uses the raw fix's speed, not a blended value.** Position/heading/altitude
  all blend over the 2 s correction window because a *visible jump* would look wrong; a label's
  *text* updating immediately when a new, more current speed arrives is correct, not a bug — text
  has no "motion" to smooth.
- **Dropped `AircraftInstance`'s `Copy` derive** (kept `Clone`), same reasoning and same
  precedent as `Track` at 2.6a: `callsign: Option<CallSign>` owns a heap allocation. Grepped every
  call site first (`aircraft.rs`, `renderer.rs`) rather than assuming — both already took
  `&AircraftInstance` or consumed owned values out of a `Vec`, so the blast radius was exactly the
  two test fixtures that constructed the struct literal directly (both updated).
- **Priority's "selected" component is explicitly deferred to 2.8**, not implemented as a
  placeholder. There's no selection state anywhere yet (2.8 hasn't landed); 2.7b's collision
  priority will treat it as always-false and this gap is flagged in the 2.7b checklist line
  itself, not discovered cold at that item's own implementation time.
- **Done directly, not delegated.** `sim.rs` was already read in full this session to make the
  split call above; a cold subagent would only re-derive it (2.4a/2.6a's own precedent for the
  same reasoning).
- 5 new unit tests in `core::sim` (content carried onto a first sighting with the exact
  `KT_PER_MS`/`FT_PER_M` conversions pinned; missing callsign/altitude/speed each leave their
  field `None`; a later blank-callsign fix does not clear a previously known one; a later fix with
  an actual new callsign does replace it; altitude is still reported while on the ground).
  `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` all green
  — **441 passed, 5 ignored, 0 failed** (+5 over 2.6b's 436, all in `core`). No live run: pure
  data plumbing with no renderable surface until 2.7b consumes the new fields (2.4a/2.6a's own
  precedent for the same reason). Next: **2.7b**, the render-side text glyph atlas + placement +
  collision culling + leader lines.

## 2026-07-19 — M2 item 2.7b (labels render)

- **Stroke font, not a filled-silhouette atlas.** 2.5's `glyph_atlas.rs` rasterizes closed
  polygons (signed inside/outside distance); text strokes aren't closed shapes, so `label_atlas.rs`
  rasterizes *unsigned* distance-to-nearest-line-segment instead, reusing `glyph_atlas`'s
  `distance_to_segment` (widened to `pub(crate)`) and generalizing `encode_distance` into a
  `pub(crate) encode_signed_distance(distance, spread)` both atlases call with their own spread —
  shared primitive, not a duplicated one.
- **Character set kept to exactly the 39 characters the label format needs** (`A`–`Z`, `0`–`9`,
  space, `k`/`t`) rather than a general ASCII font — this is a technical/UI font for one fixed
  content string, not a reusable asset; digits use the familiar seven-segment layout, letters a
  compact stick font over a 3×5 grid. Evocative, not typographic, the same bar 2.5's aircraft
  silhouettes were held to.
- **`Renderer::render` takes `&Camera`, not a lone `meters_per_pixel: f64`.** The aircraft/trail
  passes only ever needed the zoom scalar; the label pass additionally needs `center_m`/
  `width_px`/`height_px` to project world positions into screen-pixel space for placement and
  collision. Passing the whole camera (rather than growing the parameter list scalar by scalar)
  keeps the signature stable as any future screen-space pass gets added. One call site
  (`app::window`) updated.
- **Hysteresis as a priority boost, not a stored margin.** The skill's "a label keeps its slot
  until beaten by >10%" is implemented by ranking a currently-held candidate at
  `priority × 1.1` during the collision sort — a challenger only outranks it once its own raw
  priority exceeds that boosted value, which is exactly the >10% margin, without needing a second
  comparison path.
- **Priority folded into one scalar** (`selected` weight `»` speed weight `»` proximity term),
  not a lexicographic tuple — makes the hysteresis "beaten by >10%" comparison a single number
  rather than a tuple ordering rule. Weights are sized so each tier dominates the next at any
  viewport size docs/01 supports; `selected` is hardcoded `false` with a doc comment (no
  placeholder signal) since 2.8 hasn't landed.
- **Re-evaluation (≤5 Hz) and per-frame refresh are two different code paths, on purpose.** The
  candidate rebuild + collision sweep is genuinely throttled (`LabelLayer::last_eval_s`), but a
  *shown* label still needs to visually track its moving aircraft every frame in between — so
  `LabelLayer` calls `label::placement_geometry` alone (no text allocation, no sweep) on the
  off-ticks. Keeps ADR-002's no-per-frame-allocation rule on the common path while still meeting
  the "doesn't flicker" requirement on the throttled one.
- **Delegated to the renderer-agent** (glyph/SDF atlases and label drawing are its stated remit,
  same call as 2.5/2.6b); interrupted mid-task by a session API/rate-limit error, resumed via
  `SendMessage` from its own transcript rather than restarting cold — the same recovery path
  2.5/2.2b used, no work re-derived or lost.
- **Independent re-verification found a real bug, not just a rubber stamp.** Every changed/new
  file read in full; fresh `fmt`/`clippy --all-targets -D warnings`/`test --workspace` matched the
  agent's own reported 474 exactly. A **live run against the owner's real `credentials.json`**
  (scripted zoom over Scandinavia/the Baltic via Win32 `mouse_event` wheel synthesis + `PrintWindow`
  capture, DPI-aware per 2.2b's own recorded lesson) showed a dense stack of labels along the
  window's left edge with no aircraft glyph anywhere near most of them.
- **Root cause and fix: `build_candidates` had no on-screen check.** It built a label candidate
  for every aircraft in the feed regardless of whether its glyph was actually visible — the feed
  can span a wider region than the current viewport (e.g. right after a camera zoom, before the
  poller has retargeted per 2.3b) — and `placement_geometry`'s viewport-edge clamp then pinned
  each off-screen candidate's label to the border. `aircraft.rs` needs no equivalent check because
  an off-screen glyph simply never rasterizes in wgpu's clip space; the label pass, having no such
  natural clipping of its own, needed one added explicitly. Fixed with `glyph_is_visible` (margin
  = the aircraft glyph's own on-screen half-width, so a glyph straddling the exact edge — still
  partly drawn — still gets labeled) gating `build_candidates`. Done directly (small, well-scoped,
  in a file already fully read this session — this session's own bar for not delegating a
  sub-20-line fix, same as the token skill's rule). 3 new tests (off-screen aircraft → no
  candidate; on-screen → a candidate; the margin boundary itself).
- **Re-verified again after the fix, live, not just re-tested.** Rebuilt the binary (the first
  live capture had run against a stale pre-fix build — a process-hygiene lesson for any future
  session scripting a live check against a directly-launched `.exe`: `cargo test`/`clippy` do not
  refresh `target/debug/<bin>.exe`, only `cargo build -p <bin>` does), re-ran the same scripted
  zoom: the orphaned-label column was gone, and a cropped/upscaled inspection of a dense airport
  cluster confirmed the collision sweep itself works as specified (fewer labels than glyphs —
  overlapping losers culled entirely, never shrunk — no visible overlap in the captured frame).
  Flip-near-edge and leader-line behavior relied on unit tests rather than a live pixel hunt
  (`placement_flips_to_the_left_near_the_right_edge`,
  `no_leader_line_when_the_label_is_not_displaced`), the same "unit tests + one confirming live
  pass" bar 2.6b's ribbon taper was held to.
- `cargo fmt --check`/`clippy --workspace --all-targets -D warnings`/`test --workspace` all green
  — **477 passed, 5 ignored, 0 failed** (+36 over 2.7a's 441: 33 from the agent's implementation,
  +3 from this session's fix). Clean `WM_CLOSE` on both live runs; scratch `look_above.db` deleted
  after each per 1.12/1.13's convention. Credit spend from the two live runs not precisely
  tallied — the verification script launched the binary directly rather than through a
  log-capturing harness, so the per-cycle credit lines weren't recorded; flagged in
  CURRENT_STATUS rather than assumed zero. Next: **2.1b** (F3 stats overlay text, unblocked now
  that a text atlas exists) or **2.8** (selection) — owner's call, neither started.

## 2026-07-19 — M2 item 2.1b (F3 frame-stats overlay: on-screen HUD text)

- **Owner picked 2.1b over 2.8** to close out (both were unblocked at 2.7b's landing); the F3
  debug HUD is docs/01's last unimplemented draw-order step ("map base → map lines → trails →
  aircraft glyphs → labels → **UI overlay**"), closing the gap 2.1's own split first opened.
- **Reuse, not a second text renderer.** `wgpu::RenderPipeline`/`Buffer`/`BindGroup` are cheap
  `Clone` (`Arc`-backed) handles, so the new `StatsOverlayLayer` in `renderer.rs` is built by
  cloning `LabelLayer`'s already-built text pipeline, atlas bind group, shared text-quad mesh, and
  screen-params bind group straight out of it — verified directly in the diff that no second SDF
  atlas texture or pipeline is created anywhere. Only the overlay's own instance buffer/capacity/
  scratch `Vec` and text color are new GPU state, the same "layer owns the buffer, a pure module
  owns the content" split every other pass (`aircraft.rs`/`trail.rs`/`label.rs`) already uses.
- **Character-set scoping call, made before delegating, not re-derived by the agent**:
  `label_atlas::CHARSET` (39 characters: `A`-`Z`, `0`-`9`, space, `k`, `t`, from 2.7b) is
  deliberately *not* grown for this task — extending a shared stroke-font atlas for a debug-only
  overlay was judged out of proportion to the item. `render::stats_overlay::format_lines` stays
  entirely ALL CAPS with whole numbers (`FPS 47`, `P50 9MS  P95 17MS`, `WORST 60MS`, `N 9102`),
  no `.`/`=`/`_`/lowercase — a precision loss (whole ms, not `12.34`) accepted for a corner HUD,
  not the `tracing::info!` log line the F3 toggle already writes (which keeps its own 2-decimal
  format unchanged). A dedicated unit test iterates every character of every generated line
  through `label_atlas::char_index` and asserts it resolves, rather than relying on
  `pack_overlay_instances`'s own defensive skip-unsupported-characters fallback to hide a mistake
  silently.
- **`Renderer::render` gained a fourth parameter** (`stats: Option<StatsOverlay>`, trailing after
  `camera`) — the third signature change this milestone (2.4b added the feed, 2.7b added the
  camera). `StatsOverlay` is plain `f64` data (`fps`/`p50_ms`/`p95_ms`/`worst_ms`) rather than
  `app::frame_stats::FrameSummary` itself, since `render` must not depend on `app` (the
  workspace's one-way dependency direction, checked at M0). `None` (F3 off) builds/uploads
  nothing for the pass at all, not even an empty buffer write — toggling the HUD off costs
  nothing per frame beyond the existing `instance_count == 0` early-return every other pass
  already has.
- **`app::window`'s `FrameStats::record` only fires once a second**, but the HUD needs to draw
  every frame — added `App::last_stats_summary: Option<FrameSummary>`, persisted unconditionally
  (regardless of `stats_visible`) whenever a report lands, so toggling F3 on shows current numbers
  immediately rather than waiting up to a second for the first report. The HUD's numbers therefore
  lag the true instantaneous frame time by at most one reporting interval — an accepted,
  documented tradeoff, not an oversight (the same interval the existing log line already reports
  at).
- Delegated to the renderer-agent (GPU pipeline/atlas wiring is its stated remit), briefed with
  every design call already made (character-set constraint, reuse-don't-duplicate, the exact new
  types/params/fields and where they live) so the agent implemented rather than re-decided them —
  same shape as 2.5/2.6b/2.7b's delegations. **Independently re-verified rather than trusted**:
  every new/changed file read in full (`stats_overlay.rs`, and the diffs to `renderer.rs`,
  `color.rs`, `lib.rs`, `window.rs`, `frame_stats.rs`), `cargo fmt --check`/
  `clippy --workspace --all-targets -D warnings`/`test --workspace` re-run fresh — **486 passed,
  5 ignored, 0 failed** (+9 over 2.7b's 477: 7 in `stats_overlay`, 2 in `color`), matching the
  agent's own reported count exactly.
- **Live-verified independently, not just the agent's own screenshots**: built and launched the
  binary directly (`target/debug/look-above.exe`, real `credentials.json`, whole-world OpenSky
  feed), screenshotted with F3 off (confirmed no HUD present) and on (confirmed a cyan HUD block
  at the top-left), then cropped and 4x nearest-neighbor-upscaled the HUD region to read it
  precisely: `FPS 47` / `P50 9MS  P95 17MS` / `WORST 60MS` / `N 9102`, matching the designed
  format exactly, with aircraft glyphs/labels/trails still rendering correctly around it. Clean
  `WM_CLOSE` (Win32 `PostMessage`, exit code 0); scratch `look_above.db` deleted after per
  1.12/1.13's convention.
- M2 checklist now has three items left: **2.8** (selection — cursor hit-test, white outline,
  info card), 2.9 (headless renderer smoke test wired into CI), 2.10 (the M2 gate itself).

## 2026-07-19 — M2 item 2.8a (selection state + hit-test)

- **Split 2.8 → 2.8a/2.8b**, self-approved same-session, same shape as every prior M2 item:
  *detecting* a selection (input handling, hit-test math, state threading — fully testable with
  no GPU surface) is one lane; *drawing its consequences* (a white-outline GPU pass, a new
  text-overlay pipeline) is another. 2.7b's `label_priority` had already left an explicit seam
  for this (`selected` hardcoded `false`, doc comment pointing at "2.8").
- **`selected` lives on `AircraftInstance`, computed by `core::sim`, not `render`** — matches
  docs/01's own framing ("...category, selection state) produced by the CPU interpolation
  stage"). `Simulator` holds `selected: Option<Icao24>`; `set_selected` is the only mutator;
  `advance_all`'s existing `par_iter_mut` compares each track's own address against it — no new
  per-frame allocation or pass over the table.
- **Hit-testing is a plain linear scan, not the design notes' uniform grid** — deliberate
  deviation. The design notes propose "a simple uniform grid over screen space rebuilt per
  frame ... for hit-testing and later label density", but hit-testing runs once per click, not
  once per frame the way the label pass's collision sweep does; even a full scan over a
  whole-world feed (order 10,000 aircraft) is cheap next to a discrete click event. Building the
  grid now, with no label-density work yet to justify a per-frame structure, would be exactly
  the premature abstraction CLAUDE.md warns against. Revisit if profiling ever says otherwise.
- **Selection threads app → simulation worker over a new `watch` channel**, the identical shape
  2.3b already established for camera → poller retargeting (`select_tx`/`select_rx` mirroring
  `retarget_tx`/`retarget_rx`). The worker re-applies `set_selected` from the channel's current
  value every ~60 Hz iteration rather than edge-detecting a change — simpler, and free next to
  the iteration's existing cost (`Option<Icao24>` is `Copy`).
- **Click vs. drag is a movement + duration threshold** (`CLICK_MAX_MOVEMENT_PX` = 5px,
  `CLICK_MAX_DURATION` = 300ms), not a separate gesture-recognizer abstraction — the smallest
  correct disambiguation for a codebase with exactly one other pointer gesture (pan-drag).
- **Live verification produced a real click pipeline confirmation but no live "hit"**: four
  window-mode runs against the owner's real `credentials.json` all showed a scripted click
  reaching `App::maybe_select`, running the hit-test, and logging `selection changed
  selected=?` — proving the whole chain (click → hit_test → `selected_icao24` → `select_tx` →
  worker → `Simulator::set_selected`) executes without panicking or hanging. Every attempted
  click (including ones aimed via a screenshot at a visible aircraft cluster, and one found by
  programmatically scanning a screenshot for the most saturated pixel, to remove human
  pixel-estimation error) logged `selected=None`. Root-caused, not shrugged off: a live
  whole-world OpenSky feed churns roughly 80–90% of its tracked set between one 8–10 s poll
  cycle and the next (`dropped≈8,700` of `tracked≈9,800` was typical this session), so the
  specific aircraft visible in a screenshot is frequently gone or moved before a scripted click
  a few seconds later can land on it — an artifact of scripted-input round-trip latency against
  a fast-churning live feed, not evidence against `hit_test`'s correctness, which rests on its
  own 6 unit tests instead (exact hit, radius-boundary hit/miss, empty feed, nearest-of-two-
  overlapping, anonymous-still-selectable). Recorded honestly rather than claiming a live hit
  that wasn't actually observed.
- **Did live-confirm the one real regression risk**: this item restructured `window.rs`'s
  `MouseInput` handler (previously one `if let Some(camera) = ... { match state { ... } }`
  block, now two independent `match state` blocks — one for the camera, one for click
  detection). A scripted real drag (press → 10-step move → release) produced no `selection
  changed` log and did pan the camera (the poller's own "retargeted mid-run" log showed the bbox
  change) — drag-to-pan is intact.
- **Found and flagged, deliberately not fixed here: a reproducible `wgpu` crash**, independent
  of this item's own diff. Twice across the four live runs, roughly 2–2.5 minutes into a
  whole-world-zoom window-mode session, `Device::create_buffer` panics: the trail vertex buffer
  requests ~279 MiB against this adapter's 256 MiB `max_buffer_size`. This is the LOD gap
  already flagged at 2.5/2.6b (no L0/L1 cross-fade; every aircraft draws a full unbounded
  5-minute trail at any zoom) turning out to be a stability bug, not just a performance cost,
  once ~9,800 simultaneously-tracked aircraft each grow a trail at once. 2.8a's own changes
  touch none of `render::trail`/`renderer.rs`'s buffer sizing — confirmed pre-existing, not
  introduced here. Flagged prominently in `plans/CURRENT_STATUS.md`'s Now section as
  higher-priority than continuing straight to 2.8b, since a crash blocks the M2 gate's (2.10)
  live-run-over-a-busy-hub line outright if that hub view is left at whole-world zoom long
  enough to hit it.
- Scripting notes (own tooling, not app bugs, recorded so a future session doesn't re-debug
  them): `FindWindow` by exact title can return `0` from this environment (2.4b hit the same);
  `SetForegroundWindow` from a process that doesn't own the target window is unreliable here
  (Windows' foreground-lock protection) — forcing the target topmost via `SetWindowPos(...,
  HWND_TOPMOST, ...)` routes synthetic mouse input reliably regardless of focus, and was used
  instead. `GetClientRect`/`PrintWindow` both return logical (DPI-scaled) coordinates unless the
  calling process itself declares `SetProcessDPIAware()` first — the same pitfall 2.2b's own
  screenshot tooling hit, re-encountered and re-fixed here rather than assumed already known by
  a fresh script.

## 2026-07-19 — M2 item 2.8b (selection render: white outline + info card)

- **Outline is a second, scaled-up instance packed into the same aircraft instance buffer, not a
  second shader pass or an SDF-threshold shrink.** `AircraftLayer`'s pass has no depth test
  (alpha-blended, painter's-algorithm draw order — same as every other M2 pass), so
  `aircraft::pack_instances` prepends one solid-white, `SELECTION_OUTLINE_SCALE_MUL`-scaled copy
  of the selected aircraft's own instance (same position/heading/category, so the silhouette
  matches exactly) before the ordinary per-aircraft instances, all in one draw call. A new
  per-instance `InstanceRaw::scale_mul: f32` (`1.0` for ordinary glyphs) multiplies
  `aircraft.wgsl`'s existing per-frame `glyph_params.x` scale for just that one instance.
  Considered and rejected: shrinking the SDF's `0.5`-edge threshold inward by
  `SELECTION_OUTLINE_WIDTH_PX` worth of local-glyph-space units, which would give a true
  uniform-width ring — but `glyph_atlas::SPREAD` is deliberately tuned tight (so adjacent atlas
  tiles never bleed under bilinear filtering) and has no distance gradient left that far inside a
  silhouette to threshold against; widening `SPREAD` to support it was out of scope for this item.
  The scaled-copy approach is not a true uniform-width offset (a dart's nose scales out further
  than its waist) but reads clearly as a highlight ring in practice — confirmed live (see below).
- **`SELECTION_OUTLINE_SCALE_MUL` is derived at compile time from `AIRCRAFT_GLYPH_PX` and
  `SELECTION_OUTLINE_WIDTH_PX` (2 px, the skill's own number)**, not a bare literal — Rust's
  const float arithmetic (stable since well before this workspace's edition-2024 toolchain floor)
  makes `((AIRCRAFT_GLYPH_PX + 2.0 * SELECTION_OUTLINE_WIDTH_PX) / AIRCRAFT_GLYPH_PX) as f32` a
  `const`, so the two numbers can never silently drift apart if the glyph size ever changes.
- **`core::sim` gained `AircraftInstance::source: SourceId`** (and `Track::source`, updated from
  every fix, deliberately *not* sticky like `callsign` — a mid-track source failover is real,
  current information the info card should reflect, not paper over) — needed because the M2
  checklist's own 2.8b wording specifies the card's content as "callsign/alt/speed/source", and
  no field carrying which feed a fix came from existed on the render-facing type before this item;
  `StateVector::source` already existed (`positions.source`, docs/08) but stopped at `core::merge`.
- **Info card content formatting lives in a new `render::info_card` module**, reusing
  `label::format_flight_level`/`format_speed_kt` (widened from private to `pub(crate)` for
  exactly this reuse) rather than duplicating the `FLnnn`/`nnnkt` formatting a second time, and
  reusing `stats_overlay::pack_overlay_instances` directly for GPU packing (already generic over
  an arbitrary line list/origin/color — the card's own shape, not a new packer). `InfoCardLayer`
  in `renderer.rs` is *cloned* from `LabelLayer`'s pipeline/atlas/mesh/screen-params bind group,
  the identical "one SDF text atlas, one text pipeline in the whole crate" reuse `StatsOverlayLayer`
  established at 2.1b — only the instance buffer/scratch/color are new state. Fixed origin
  `(10, 80)`, below the F3 HUD's own 4-line block, so the two never overlap regardless of whether
  F3 is toggled.
- **Privacy rule 2.2's anonymous exception is wired in `label::format_label_text` itself, not
  duplicated in `info_card`**: an anonymous target is still never labeled *unless selected*, in
  which case the label (and, separately, the card) show `"UNIDENTIFIED"` plus altitude only (rule
  2.2's literal "position/altitude only") — never callsign (there isn't a real one) or speed, and
  the check does not even read `instance.callsign`, so an upstream glitch that somehow attached
  one still cannot leak it. This was explicitly flagged as "2.8's job" in both 2.7a's and 2.7b's
  own doc comments/decision-log entries; closed here.
- **Raw position (lat/lon) text for an anonymous card is deferred, not silently dropped**: docs/13's
  fuller acceptance line asks for "position data" on an anonymous card, but no numeric
  lat/lon-to-text formatting exists anywhere in `render` yet, and adding it would mean widening
  `label_atlas::CHARSET` (a decimal point, a minus sign) for a feature this item's own checklist
  wording ("callsign/alt/speed/source") does not name. Flagged for the M2 gate (2.10) to verify
  against docs/13 directly rather than assumed satisfied here.
- **Enrichment fields (type/operator/route) are M3's job**, per the checklist's own parenthetical
  — `info_card` documents this in its module doc comment rather than leaving the gap implicit.
- **Live-verified two ways, not one, after the first attempt was ambiguous.** A window-mode run
  against the owner's real `credentials.json` (whole-world OpenSky) produced, for the first time
  in this project's history, an actual live click landing on a *real* tracked aircraft
  (`selected=Some(Icao24([13, 16, 120]))` — every one of 2.8a's own four attempts had missed): the
  info card appeared with real callsign/altitude/speed/`SRC OPENSKY` content matching the clicked
  target, and a before/after pair confirmed no card at all before the click (`info_card: None`'s
  own path). The outline, however, was not visually distinguishable in that screenshot — the
  click happened to land in a very dense whole-world cluster (dozens of overlapping glyphs), and a
  programmatic scan for near-pure-white pixels near the click found none, either because the thin
  (2 px) ring was occluded by other, unrelated aircraft glyphs drawn after it in the same instance
  buffer (address-sorted order, not selection-aware), or simply lost in the cluster's own visual
  noise. Rather than guess, built a second, isolated check: a throwaway (uncommitted, deleted
  after use) `winit`/`Renderer` harness driving a synthetic two-aircraft `RenderFeed` — one
  selected, one not, positioned with no overlap — through the real `Renderer::render`. That
  screenshot showed a crisp white outline ring around exactly the selected glyph and none around
  the other, plus a card reading `SMOKE01` / `FL350` / `450kt` / `SRC OPENSKY` matching the
  synthetic data exactly. Recorded honestly as two-part evidence (real click + real data on one
  axis, isolated visual proof of the outline on the other) rather than either overclaiming the
  live run caught the outline or dismissing the outline as unverified.
- **Scripting note**: `FindWindow(null, title)` returned `0` even though the window definitely
  existed (confirmed via `EnumWindows` finding it immediately) — a P/Invoke marshaling issue with
  passing a literal `$null` string argument from PowerShell, not a real absence. Switched to an
  `EnumWindows`-based title search (matching by exact `GetWindowText`) for both this item's
  verification scripts; a `$using:` closure variable also failed outside a remoting context
  inside the `EnumWindows` callback and was replaced with a script-scoped variable. Neither is an
  app bug; recorded so a future session doesn't re-diagnose either from scratch.
- Credit spend: two real window-mode runs against the owner's real `credentials.json` (one
  whole-world poll cycle each, 4 credits/cycle per 2.4b/2.5's own figures — 8 total this item),
  plus the isolated synthetic harness (no network, 0 credits). Scratch `look_above.db` deleted
  after each per 1.12/1.13's convention.

## 2026-07-19 — M2 item 2.9: headless renderer smoke test, wired into CI

- **`Renderer`'s `surface`/`config` fields became a private `Target` enum** (`Windowed` — the
  only variant outside test builds — or a `#[cfg(test)]`-only `Offscreen`) rather than adding a
  second, parallel renderer type: every `build_*_resources` free function already took only
  `&device`/`&queue`/a format/a size, never the surface, so the windowed and headless paths
  share pipeline construction unchanged, and the only real duplication risk was the *draw pass
  sequence* — solved by extracting `Renderer::record_draw_passes` (shared by `render` and the
  new `render_headless`) so the two can never drift apart on docs/01's draw order. `resize`/
  `reconfigure`/`render`/`format` all match on `Target` rather than `let`-`else`: in a non-test
  build `Target` has exactly one variant, and clippy correctly flags a `let`-`else` there as an
  always-taken (dead) `else` branch — `match` stays exhaustive and lint-clean regardless of how
  many variants exist.
- **`Renderer::new_headless(width, height)`** (`#[cfg(test)]`, crate-private) requests a
  *fallback* adapter (`force_fallback_adapter: true`, `compatible_surface: None`) — docs/10 §4's
  own "headless wgpu (fallback adapter)" wording — never opening a window, and renders into a
  plain `RENDER_ATTACHMENT | COPY_SRC` texture (`Rgba8Unorm`, fixed — there is no surface to pick
  a format from) instead of a swapchain. `render_headless` returns `FrameOutcome` directly, not
  `Result`, since an offscreen texture has none of a swapchain's lost/outdated/occluded states to
  report. `read_offscreen_pixels` maps the copied-out readback buffer and blocks on
  `device.poll(PollType::wait_indefinitely())` (test-only code, no event loop to drive the map
  callback otherwise), stripping `copy_texture_to_buffer`'s required row padding back out.
- **The smoke test** (`renderer::tests::renderer_smoke_test_headless_1000_aircraft`) builds a
  fixed-seed, deterministic 1,000-aircraft `RenderFeed` (splitmix64 PRNG hand-rolled inline —
  this workspace has no `rand` dependency, and a reproducible spread is all a smoke fixture
  needs, not real randomness; per CLAUDE.md, not worth a new dependency for this) spread across
  80% of the Web Mercator extent with varied altitude buckets/categories/sources/headings and a
  short synthetic trail per aircraft (so the trail pass isn't trivially empty), renders it
  headless against an empty-feed baseline, and asserts the non-background pixel delta the 1,000
  aircraft add lands in `(20_000, 250_000)` of 480,000 total pixels (800×600) — the band is from
  an actual measured run on this machine's DX12 WARP fallback adapter (`aircraft_non_background =
  86,817`), kept loose (~4x margin below, ~3x above) to absorb a different fallback adapter's own
  AA/rounding behavior while still catching "renders nothing" (≈0) or "renders garbage
  everywhere" (≈the whole non-baseline frame) outside the band. `Err(RenderError::NoAdapter(_))`
  is treated as skip (`eprintln!` + early `return`), per docs/10's own "skipped, not failed"
  wording — deliberately not `#[ignore]`, since the spec wants the adapter attempted every run
  and only conditionally skipped, not permanently excluded.
- **`.github/workflows/ci.yml`'s comment updated, no apt step added.** The prior comment claimed
  "no test opens a window or a GPU adapter" to justify the runner needing no display stack; that
  became false the moment this test exists, so the comment was corrected to explain the new test
  attempts a headless fallback adapter but is written to skip itself (not fail) when none is
  found — the simpler, correct reading of "skip-if-no-adapter" is that CI is allowed to just
  skip, not that CI must be given a software Vulkan ICD (e.g. Mesa lavapipe) so the test can run
  for real. Concrete finding: `ubuntu-latest` has no Vulkan ICD registered without an apt step
  this job deliberately doesn't take, so it is expected to skip; `windows-latest` may or may not
  expose DX12 WARP depending on the runner image, so it either runs for real or skips too —
  either outcome is a passing CI run under this contract.
- Delegated to the renderer-agent (the `Target`/offscreen split is core wgpu pipeline plumbing,
  its stated remit). Independently re-verified rather than trusted: both changed files
  (`renderer.rs`, `ci.yml` — nothing else touched) read in full, fresh `cargo fmt --check`/
  `clippy --workspace --all-targets -D warnings`/`test --workspace` re-run — **515 passed, 5
  ignored, 0 failed** (render crate 112 → 113, +1 over 2.8b's 514), matching the agent's own
  reported count. The new test was independently confirmed to actually *run*, not skip, on this
  machine (DX12 WARP fallback adapter found — `AdapterInfo { name: "Microsoft Basic Render
  Driver", device_type: Cpu, backend: Dx12 }`), so the whole headless pipeline (adapter → device
  → offscreen texture → all six passes → copy → map → readback) was exercised for real, not just
  compiled. No live network run needed (a pure offscreen/synthetic-data test); no credit spend.
  The flagged trail-buffer crash (2.8a) and item 2.10 (the M2 gate) were both left untouched, as
  scoped. M2 checklist now has only **2.10** (the gate) left.

## 2026-07-20 — M2 item 2.10 (the M2 gate)

- **Gate ran; recorded as a partial pass, not silently marked done**: 3 of 6 acceptance §M2 lines
  pass cleanly (60fps/p95 at regional scale with a pan-sample caveat; no-teleport glide; stale
  fade-and-drop), 3 are open (heading spot-check — the info card doesn't expose a heading value to
  check against, a gap 2.8b itself flagged; antimeridian — unimplemented, not just untested;
  interpolation benchmark — `sim_advance_all` exceeds its 2ms/10k-aircraft budget). Same shape as
  M0's 6/7 and M1's 6/7: a gate that doesn't fully pass is still recorded, not blocked on or
  hidden. Full per-line evidence: CURRENT_STATUS.md's gate table and session log, M2 plan 2.10.
- **Finished two pieces of work found already uncommitted at this session's start**, from an
  interrupted prior attempt at this same item: a `criterion` gate-benchmark harness
  (`crates/core/benches/interpolation.rs`) covering docs/10 §5's first two budgets (`sim::
  advance_all` for 10k aircraft; a `web_mercator_forward` batch of 10k points, benched standalone
  under the same `rayon` parallelism `advance_all` already applies it with, rather than adding a
  `project_batch` production API with no real caller — the harness's own module doc names this as
  avoiding the exact trap M0's DECISION_LOG flagged: "a parallel batch API with no caller is a
  guess at the call shape"); and a label legibility fix (`label::LABEL_CHAR_WIDTH_PX`/
  `LABEL_CHAR_HEIGHT_PX` `7×12`→`16×28`, `renderer::InfoCardLayer`'s origin `80`→`145` to clear the
  now-taller F3 HUD). Both read in full, both correct and tested (fmt/clippy/test all green, 515
  passed/5 ignored/0 failed — unchanged from 2.9, since the label change is a pure constant tweak
  needing no new test), both committed here rather than redone from scratch.
- **`sim_advance_all` measured over its budget on real, capable hardware, not weak hardware**:
  3.2–4.3ms against a 2ms/10k-aircraft budget, measured on an Intel Core Ultra 7 155H (16 cores,
  22 threads — more than the budget's own "8 cores" reference point). A repeat run measured
  *worse* (3.28ms → 4.26ms; the projection-batch number also worsened in step, 94µs → 123µs),
  which points at thermal/background-load variance on a laptop under sustained load rather than a
  stable regression — not chased further with more reruns (each one costs time/battery and the
  trend was uniformly worse, not converging). Recorded as a genuine open budget line, not
  explained away; diagnosing/fixing `core::sim::advance_all`'s performance is new work belonging
  to whoever picks up this open line, not something to fix mid-gate-check.
- **New finding, not fixed here**: the label-size legibility fix (above) makes dense regional
  clusters (150+ aircraft on screen — including the gate's own busy-hub test region) read as a
  visually cluttered, overlapping mass in screenshots. 2.7b's collision sweep has its own
  unit-tested no-two-rects-overlap invariant which this session did not re-derive or disprove, so
  this is recorded as a legibility concern at high density, not a confirmed invariant violation —
  flagged for a follow-up item since diagnosing the collision sweep at this density is new work,
  not gate-checking.
- **Reconfirmed the flagged trail-buffer crash twice more, live** — both times while the
  live-verification camera was mistargeted to whole-world/hemisphere-scale views (a ~400 MiB
  trail vertex buffer against the adapter's 256 MiB cap, same failure shape as 2.8a's original
  find). The gate's own properly-scoped regional view (Italy/Adriatic, `count=219`) never
  crashed once actually reached — consistent with this being a whole-world/huge-region density
  problem, not an L2-core one, so 2.10 proceeded on the regional scope rather than waiting on a
  fix for a condition the gate's own setup (docs/13: "busy region", not whole world) doesn't
  require.
- **Most of this session's own time went to a DPI-awareness bug in the verification scripting
  itself, not the app** — worth recording in detail since it will bite any future session's own
  live-verification scripting the same way. `SetProcessDPIAware()` is per-process; this
  environment's PowerShell tool starts a fresh process on every invocation with no shared state
  across calls. The very first navigation script called it and got true 1920×1200 client
  coordinates; every subsequent separate pan/zoom script silently did not, computing
  `GetClientRect`/cursor math against a DPI-virtualized 1280×800 rect instead (confirmed by
  checking saved screenshot PNGs' actual pixel dimensions directly — they were 1280×800, not
  1920×1200, despite `GetClientSize` correctly reporting 1920×1200 in the one script that had
  called `SetProcessDPIAware`). The ~1.5x coordinate error compounded across repeated
  cursor-anchored zoom steps threw targeting off wildly and unpredictably (mistargeted views over
  Northern Canada, a Grand-Canyon-scale accidental over-zoom, mid-Pacific/Hawaii, and one
  degenerate zero-height bbox pinned at the Mercator pole clamp) before the root cause was found.
  Fixed by moving the `SetProcessDPIAware()` call into the shared helper script itself, called
  unconditionally on every dot-source rather than assumed to carry over — the same category of
  scripting pitfall as 2.2b's DPI screenshot bug and 2.8b's `FindWindow`/`EnumWindows` P/Invoke
  marshaling miss, not an app fault, but costlier this time since it wasn't recognized until well
  into the session.
- **A planned 90-second network-kill test (docs/13's dead-reckoning/reacquisition line) was not
  performed.** Adding even a process-scoped Windows Firewall outbound-block rule (targeting only
  `look-above.exe`'s own path, intended to be removed immediately after) was denied by this
  environment's own auto-mode action classifier as a shared-system change needing explicit
  authorization. Not attempted around (e.g. disabling a whole network adapter, which would have
  been a *worse* blast radius, not a workaround) — left for the owner to run by hand if wanted,
  same as the M1 token-refresh line was left for the owner's own literal-scope call rather than
  forced through.
- Live run: released binary (`cargo build --release -p look-above`), real `credentials.json`,
  four relaunches while working through the DPI bug and mistargeted navigation above; final
  regional busy hub Italy/Adriatic (`lat 41.6–46.3, lon 12.25–22.77`, `count=219`). Clean
  `WM_CLOSE` each time (`close requested → window closed` in logs); scratch `look_above.db`
  deleted after. `spent_today` ended around 286 of 3,200 (~9%), nowhere near the 80% cap — higher
  than a typical single-item session's spend because of the repeated mistargeted retargets, still
  far under the ceiling. Evidence screenshots and full run logs kept under `qa/2026-07-20/`
  (gitignored, per docs/13's own evidence-storage convention).
- **Per CLAUDE.md, M3 is not opened here.** The owner decides whether to open it with the 3 open
  M2 lines carried forward (M1's own precedent for a gate that didn't fully pass) or work them
  first.

## 2026-07-20 — M3 opened; M3 plan file written

- **Owner directed "continue with M3"** — read as the explicit gate decision CURRENT_STATUS/
  2.10 left pending: open M3 with the 3 open M2 lines (heading-in-info-card, `sim::advance_all`
  perf, antimeridian wrap) and the trail-buffer crash carried forward uncommitted-to-fix, same
  precedent as M1's token-refresh line carrying into M2.
- **Found a real gap, not silently worked around**: docs/07 states M3–M6 plan files are "written
  as part of the preceding milestone's gate" — M2's own gate (2.10) never produced
  `plans/M3_ENRICHMENT_AND_NON_ADSB.md`. Wrote it now, before any implementation, mirroring the
  M0/M1/M2 plan-file shape (Goal / constraining docs / Checklist), sourced from docs/07's M3
  paragraph and docs/11 §M3's 5 acceptance lines (6th checklist item is the gate itself, same
  pattern as every prior milestone). Linked back from docs/07 (it had no `→ plans/...` line yet,
  the only milestone missing one).
- **Recorded a cross-milestone tension explicitly in the new plan file** rather than paper over
  it: docs/11 §M3's first acceptance line ("airports visible at L1, runway outlines at L2")
  presumes LOD tier switching, which is M4's deliverable, not M3's — the renderer currently draws
  everything at one fixed tier (the same gap M2's own gate flagged for aircraft glyphs/trails).
  3.2 is scoped to the query/data half now (`Store::airports_in_bbox`'s existing `min_size` param)
  and defers true tier-gated visibility to M4, recorded as an open half at the M3 gate (3.6) the
  same way M1's token-refresh line and M2's three open lines were carried rather than silently
  marked done.
- Checklist order follows docs/07's own M3 sentence order (airports/runways import → rendering →
  METAR → adsbdb selection lookups → info card wiring → gate), not re-derived from scratch.

## 2026-07-20 — M3 item 3.1: OurAirports import lands

- **Extended `crates/import` rather than adding a new crate** — same reasoning 2.2a already
  established for Natural Earth: `store` has no network dependencies and must stay that way (M0
  acceptance line 3), so a static, public dataset needing a live fetch belongs in the one
  workspace crate that's allowed to touch the network at build/setup time and depended on by
  nothing at runtime.
- **One item, not split into 3.1a/3.1b** (unlike 2.2's basemap fetch): OurAirports is plain CSV,
  no shapefile/zip parsing, so fetch-and-bundle and migration-and-query stayed small enough to
  scope together without the session losing track of either half.
- **`AirportSize::from_ourairports_type` lives in `core::contracts`**, not duplicated into
  `import` and `store` separately — both need the identical `seaplane_base`/`balloonport`/
  `closed`-drop ladder, and `contracts.rs`'s own doc comment on `AirportSize` had already flagged
  "that mapping is an M3 decision, recorded when the importer lands" as the seam to close here.
- **New `StoreError::SeedAsset` variant**, distinct from the existing `Corrupt` — `Corrupt` is
  documented as data that already round-tripped through SQLite; a bundled-asset parse failure
  fires before any row reaches the database, a build-time defect in the shipped bundle rather
  than a runtime storage failure.
- **"Within 5% of source CSV row count" (docs/11 §M3 line 1) is measured against the *kept-type*
  source count (71,086 rows: large/medium/small/heliport), not the raw 85,776-row upstream
  total.** The raw total includes ~13,355 `closed` rows alone (plus balloonport/seaplane_base),
  which the type-drop mapping above already commits to excluding entirely — comparing the seeded
  DB against that undropped total would silently fail an acceptance line the project's own prior
  design decision (the `AirportSize` ladder) makes structurally impossible to hit literally.
  Recorded here as an explicit interpretation, the same honesty standard applied to every other
  acceptance-line reading at a gate (e.g. M1's literal-10-minute-scope call).
  Bundled result: 71,086 airports / 43,240 runways (after also dropping runways whose airport was
  cut), ~6.5 MB combined committed asset.
- **Did not implement `core::contracts::Store` for `Writer`** — only added
  `Writer::airports_in_bbox` through the existing `Command`-channel pattern. The full trait still
  can't be implemented honestly: `insert_positions`/`prune` need the `positions` table, which
  stays M5's deliverable, exactly as `crates/store/src/lib.rs`'s own pre-existing doc comment
  already explained before this item touched the file.
- Delegated to the storage-agent (its named remit covers "enrichment imports (OurAirports, FAA
  registry, METAR cache)"); independently re-verified — full diff read, fresh fmt/clippy/test
  (**539 passed, 5 ignored, 0 failed**, +24 over the M2 gate's 515), and the bundled CSV row

## 2026-07-20 — M3 item 3.2: airport/runway rendering lands

- **Split into two sequential, lane-scoped delegations** (store-side, then render-side), same
  shape as 2.3a: the store half needed a genuinely new contract (`Runway`, `runways_in_bbox`) the
  render half would otherwise have had to invent ad hoc, so it landed first and the render agent
  was briefed against the finished API rather than guessing its shape.
- **`core::contracts::Runway` omits the schema's `closed` column** — closed runways are filtered
  out in SQL at query time (`store::ourairports::runways_in_bbox`, `closed = 0`), so no caller
  ever needs to re-check a flag that query already enforced. A runway with a `None` `le_*`/`he_*`
  end (incomplete source rows, same as some bundled airports) is still returned — the *render*
  side decides what to draw for a partial runway (nothing), not the query.
- **`runways_in_bbox` added to `core::contracts::Store` and docs/09** even though M0's own
  forward-declared trait shape never anticipated it — recorded explicitly as a new contract
  addition in docs/09 rather than silently folded in as if it had always been there.
- **Real bug caught during implementation, not after**: a first-pass runway stroke width fixed in
  world-plane units (mirroring the coastline stroke's own constant) tessellated to *zero geometry*
  for realistic runway lengths — `lyon`'s stroke tessellator has a hardcoded internal point-merge
  floor (~1e-4 in path-space) that this crate's whole-Earth-spanning `[-1, 1]` normalized plane
  puts most real runways well under. Fixed two ways: the stroke width is screen-constant (like
  every other on-screen-fixed-size element in this renderer), and each runway is tessellated in a
  coordinate space rescaled `1e5×` before handing it to `lyon`, then scaled back down — both
  documented in `airport.rs`, with a regression test pinned to a realistic ~300 m runway.
- **Airport markers are a plain filled circle**, not a diamond (no heading to orient one against,
  unlike an aircraft glyph) or a single-pixel dot (harder to distinguish from AA noise at this
  size). Both the marker fill and the runway outline are deliberately dimmer than every altitude
  tint and the label/info-card white (color.rs tests pin this) — static map furniture, not live
  traffic, per docs/01's "aircraft stay brightest" rule.
- **Airport-marker draw-order placement is this item's own judgement call**: docs/01 only names
  "coastlines/borders/runways" as one "map lines" step; runway outlines fit that literally (drawn
  right after the coastline stroke), but airport markers aren't separately named. Placed in the
  same map-lines slot, right after the runway outlines and still before trails — recorded as a
  judgement call in `renderer.rs` rather than silently decided.
- **Fixed `AirportSize::Medium` threshold, no LOD-tier gating** — the checklist's own "markers for
  large/medium airports" wording, hardcoded rather than driven by the (nonexistent until M4) zoom
  tier, per the M3 plan's own cross-milestone tension note recorded when M3 opened. The query is
  piggybacked onto `app::window::App::maybe_retarget`'s existing camera-settle trigger (a cloned
  `store::Writer` handle, cheap since it's just a channel `Sender`) rather than inventing a second
  settle/debounce mechanism for the same event.
- **Both geometries rebuild every frame from whatever slice `Renderer::render` is handed**, rather
  than caching against "did the queried set change" — mirrors `trail.rs`'s own per-frame
  tessellation shape, simpler than tracking a diff, and cheap at this milestone's scale (tens to a
  few hundred medium/large airports in a viewport, nowhere near the 10,000-aircraft budget).
  Recorded as a judgement call in `airport.rs` rather than assumed obvious.
- Delegated to the storage-agent (`Runway`/`runways_in_bbox`, its stated remit) then the
  renderer-agent (rendering + `app::window` wiring, its stated remit), independently re-verified
  after each: every changed/new file read in full, fresh `cargo fmt --check`/`clippy --workspace
  --all-targets -D warnings`/`test --workspace` after each lane — **541 passed** (storage lane,
  +2 over 3.1's 539) then **553 passed, 5 ignored, 0 failed** (render lane, +12), both matching
  each agent's own reported count exactly.
- **Live verification was partial, recorded honestly rather than overclaimed.** A live window-mode
  run against the owner's real `credentials.json` confirmed the app builds, launches, polls, and
  renders the existing M2 picture (aircraft/labels/trails) with no regression, and a
  resize-triggered run exited cleanly with no panics. Attempting to script a precise camera
  pan/zoom to a specific busy airport (to visually confirm marker/runway-outline pixels on screen)
  via synthetic Win32 input (`PostMessage` drag + wheel events) proved unreliable in this
  environment within a reasonable time budget — the same category of scripting fragility 2.2b's
  DPI bug, 2.8b's `FindWindow` miss, and 2.10's DPI-virtualization bug already ran into, this time
  not resolved. The airport/runway pipeline is verified at the code level (diff read in full,
  logic traced against the store query's actual behavior, tests cover the tessellation edge cases
  including the caught `lyon` bug) but **not visually confirmed live on screen** — flagged here as
  an open verification gap for whoever next runs the app in window mode, not silently marked done.
  No credit spend beyond ordinary poll cycles during the launches above.
  counts re-derived independently via `wc -l` rather than trusted from the agent's own report.

## 2026-07-20 — token-management audit and handoff compaction

**Decision:** make bounded reads and zero-by-default delegation enforceable in the startup
instructions, not optional advice. Startup reads only `CURRENT_STATUS`'s Now section, the
current delivery slice, cited doc sections, and targeted code symbols. Files over 400 lines and
tool output require bounded reads. Normal work uses at most one non-nesting subagent; generic
second-opinion agents and full-file re-reviews are prohibited. The recovery skill is no longer
loaded for every implementation session, and mechanical agents should use the cheapest capable
model.

**Throughput amendment:** a session now targets one coherent usable delivery slice rather than
exactly one checklist row; adjacent low-risk rows may share context and one acceptance check.
Verification is risk-tiered: documentation gets diff/link checks, isolated leaf-crate changes
get crate-local checks, and cross-crate/privacy/network/migration/concurrency/renderer work and
gates retain the full workspace sequence. Live visual QA runs once only when visible behavior
changes. After one failed synthetic-navigation/capture attempt, record the harness gap and stop.
A deterministic airport/region preset is queued as reusable tooling before more scripted visual
QA. This keeps assurance concentrated where failure impact warrants it instead of applying the
most expensive workflow to every change.

**Rationale:** the usage report attributed 80% of spend to contexts above 150k, 42% to
subagent-heavy sessions, 14% to renderer-agent descendants, and 24% to the token workflow skill.
The repository amplified those costs: `CURRENT_STATUS` had reached 155 KB/1,666 lines with a
367-line Now section; `DECISION_LOG` was 218 KB; `renderer.rs` was 152 KB/3,105 lines; and
completed work was narrated in the status, milestone plan, and decision log. The status is now
a concise handoff card, completed M3 notes link to this archive, and detailed history remains
recoverable from this log and Git rather than recurring startup context.

## 2026-07-21 — M3 item 3.3 (METAR polling + flight-category badges)

- **No failover chain for the METAR source** — unlike the live-position sources, docs/09 lists
  exactly one authorized METAR provider (`aviationweather.gov`), so `ingest::metar` does not
  reuse `poller::Poller`'s failover/budget machinery; it is a purpose-built single-source loop.
  A fetch error is logged and retried next cycle rather than failed over, since there is nowhere
  else to fail over to; docs/08's own 2-per-station retention already tolerates a gap.
- **Retargeting the METAR poller's station list does not interrupt an in-flight sleep**, unlike
  the position poller's own retarget race — METARs are hourly, so there is no responsiveness
  case for polling early on a camera pan, and racing the sleep would risk shortening the
  documented ≥10-minute spacing under a rapid sequence of retargets (privacy/source-etiquette
  rule: "enforced in code, not just documented"). A retarget only changes what the *next* cycle
  reads.
- **A short (5 s) idle-recheck interval, separate from the ≥10-minute poll interval, added after
  live testing caught a real startup bug**: the very first cycle always sees the channel's
  initial empty station list (no camera has settled yet), and without a separate short recheck
  the loop would sleep the *full* 10-minute poll interval before ever looking again — delaying
  every fresh session's first badges by up to 10 minutes even though the camera typically settles
  within seconds of launch. An empty check costs nothing (no request leaves the process), so
  there is no spacing rule it could violate by being frequent. Caught by running the built app
  live and watching for a `metar poll cycle` log line that never appeared; fixed by splitting the
  loop's single sleep into `poll_interval` (after a fetch attempt) and `idle_recheck_interval`
  (while the station list is empty), both explicit parameters (`run_metar_poller`) rather than
  constants baked into the loop, and covered by a new regression test
  (`a_station_list_populated_after_starting_empty_is_picked_up_on_the_next_idle_recheck`).
- **Badge rendering reuses the airport-marker mesh and draws before the marker pass**, at a
  larger fixed screen-pixel radius, rather than offsetting the badge to a separate on-screen
  position: the marker's own dot then paints over the badge ring's center on top, so the pairing
  between a badge and its airport is visually unambiguous with no projection/offset math needed.
  Color is carried per-instance (`metar_badge::BadgeInstanceRaw`), unlike the airport marker's
  flat per-layer uniform color, since a badge's color depends on that airport's own flight
  category.
- **The airport ↔ METAR join happens once in `app::window`, not in `render`** — `core::contracts`
  gained a resolved `MetarBadge { lat_deg, lon_deg, category }` type precisely so `render` never
  needs to know about `Airport`/`Metar` idents at all, only positions and a category to color.
  The join (large airports only, stations with no resolved flight category excluded) is unit
  tested directly (`window::tests`) since it is pure logic with real edge cases (ident
  mismatches, unresolved categories) worth covering in isolation.
- **Badge freshness follows the same camera-settle cadence 3.2 already established** for
  airports/runways (`app::window::App::maybe_retarget`) rather than a second trigger: the newly
  queried large-airport set becomes the poller's next station list *and* the join is read back
  from the store at that same instant. Known limitation, not fixed this item: the very first
  settle's own join necessarily reads the store *before* that settle's own fetch has landed (the
  fetch is asynchronous), so a session's badges only appear once the camera settles again after
  the first background fetch completes — acceptable given docs' own "≤ 70 min" freshness bar and
  ordinary continuous panning/zooming in practice, but recorded here rather than silently assumed
  away; a future item could add a "new METARs landed" signal back to the render/event thread if a
  tighter single-settle guarantee is ever wanted.
- Colors are this project's own shades of docs/13's fixed VFR-green/MVFR-blue/IFR-red/LIFR-magenta
  convention (`color::flight_category_badge_color`), picked dim enough to stay under every
  altitude-bucket tint and label text (docs/01: aircraft stay the brightest things on screen) but
  distinguishable from the airport marker's own desaturated gray and from each other by hue.
- **`aviationweather.gov`'s wire shapes verified live, not guessed**: `fltCat` (not `flight_cat`
  or similar), `obsTime` already in Unix seconds (unlike airplanes.live's epoch-millisecond
  `now` — sources are not assumed to share a convention), and `visib` returned as either a plain
  number or a qualified string (`"10+"`, fractions) rather than always one shape. Verified via
  the sanctioned live-fetch path (`scripts/record_fixture.rs`, extended this item to support a
  bare-JSON-array response body alongside the existing object-wrapped ones) and recorded to
  `crates/ingest/tests/fixtures/aviationweather/`.
- Verification: full workspace `cargo fmt --check` / `clippy --workspace --all-targets -D
  warnings` / `test --workspace` — 590 passed, 6 ignored (live-only, unrun), 0 failed. Live
  window-mode run against the owner's real `credentials.json` confirmed the full path end to
  end: a real `aviationweather.gov` fetch, persistence, and — screenshotted — colored VFR/MVFR
  badge rings on screen at their actual airports, with airports lacking a cached observation
  correctly left as plain gray markers. The same screenshots also confirm 3.2's own airport
  markers at real pixel positions, closing the marker half of that item's carried gap; the
  viewport zoom level during this pass was too far out for a runway outline to register at all
  (correctly so — a runway is sub-pixel at that scale), so the runway-outline half stays open,
  needing the same closer-zoom-on-a-specific-airport pass 3.2 itself could not get scripted
  navigation to reach.

## 2026-07-21 — M3 item 3.4: `flights` table pulled forward from M5

- 3.4's checklist wording ("Upserts `AircraftMeta`/`flights`") and 3.5's ("sourced from 3.4's
  cached `AircraftMeta`/`flights` lookup") both name the `flights` table for on-selection route
  caching, and docs/13's info-card acceptance line expects a route or "—" — but docs/08 tagged
  `flights` M5, and the project's own established rule (migration 0001's own comment, this log's
  2026-07-20 M0 entry) is to never create a table ahead of the milestone that needs it, since
  migrations are append-only and cannot be walked back. Raised to the owner rather than guessed
  (CLAUDE.md's "stop on missing/ambiguous milestone scope" rule) — three options were: (a) defer
  route to M5, upsert only `AircraftMeta` now; (b) a new, narrower M3-scoped cache table separate
  from M5's session-tracking `flights`; (c) pull `flights` into M3 now, as originally specified.
  Owner chose (c).
- **Scope of the pull-forward is deliberately narrow**: the table is created verbatim from
  docs/08 (now re-tagged M3, with a note explaining the pull-forward), but 3.4 only ever
  *inserts* a new row per successfully resolved, non-cached adsbdb callsign→route lookup — never
  an upsert-by-(icao24, callsign) merge. `flights`' own shape (`first_seen`/`last_seen`, "callsign
  sessions") implies session-boundary merging (extending `last_seen`, detecting gaps) that is
  fundamentally driven by continuous `positions` history M5 hasn't built yet; faking that logic
  now would mean guessing at M5's own design. `Store::latest_flight` (highest `last_seen`) is
  what 3.5 reads, so this is invisible to the info card either way. Same "flag the tension, don't
  silently fake the future milestone" shape as 3.2's LOD-tier carve-out.
- **Two-layer cache design for the adsbdb lookups** (3.4's "LRU + 24h negative cache" line):
  the persistent layer is exactly what migration 0001 already ships for this — the `aircraft`
  table's `fetched_at`/`lookup_failed_at` columns, unused until now, are the 24h negative-cache
  `Store::aircraft_meta` reads before deciding whether a hex is worth fetching. An in-memory
  `lru::LruCache` in `ingest::adsbdb` sits in front of that (both the aircraft-hex and the
  callsign-route lookups) purely to skip a store round-trip on repeat selections within one
  process run — `flights` has no `lookup_failed_at`-equivalent column (docs/08 never gave it
  one), so route negative-caching is in-memory-only, bounded by the same LRU.

## 2026-07-21 — M3 item 3.4 lands: adsbdb selection lookups

- Two parallel subagent lanes (`data-source-agent` for `crates/ingest`, `storage-agent` for
  `crates/store`), each scoped to one crate with the `core::contracts` seam (`Flight`, the four
  new `Store` trait methods) written first so neither had to guess the other's shape. Both
  finished clean against their own crate's tests/clippy/fmt independently; the orchestration
  (`crates/app/src/enrichment.rs`, `App::maybe_select` wiring) and the final workspace-wide
  verification were done directly, not delegated — the two crates meeting is exactly the seam
  this session's own read of CLAUDE.md's delegation-budget rule says stays in the main session.
- **`ingest::adsbdb::AdsbdbSource`** (`crates/ingest/src/adsbdb.rs`): pure adapter, same shape as
  `MetarSource`. adsbdb's real response shapes were unknown going in (docs/09 only names the
  endpoints) — recorded live via an extended `scripts/record_fixture.rs` (`adsbdb aircraft
  <hex> <name>` / `adsbdb callsign <callsign> <name>`), against a real registered Cirrus SR22
  (`a4b213`) and a real United Airlines route (`UAL123`, ANC→ORD at record time). Found:
  `response.aircraft.{icao_type, registration, registered_owner, ...}` (icao_type is the short
  ICAO type designator; a sibling `type` field is a longer non-ICAO description, deliberately
  not read) and `response.flightroute.{airline, callsign, origin{icao_code,...},
  destination{...}}`. An unknown hex/callsign answers a plain HTTP 404 with no body — caught at
  the `SourceError::Request{status:404}` level and mapped to `Ok(None)`, distinct from every
  other error (which still propagates), so a transient failure can never poison the negative
  cache the way a confirmed miss should.
- **`store`**: migration 0004 creates `flights` verbatim from docs/08 (now re-tagged M3);
  `aircraft.category` needed a string round-trip it didn't have yet
  (`AircraftCategory::as_str`/`from_store_str`, added to `core::contracts` alongside the type
  it classifies, mirroring `FlightCategory`'s existing pair) — the one contract addition beyond
  what was scoped up front, flagged and added in the same pass rather than a follow-up. `Writer`
  gained `upsert_aircraft_meta`/`aircraft_meta`/`insert_flight`/`latest_flight`, same
  `Command`-enum-plus-reply-channel shape every existing method uses.
- **`app::enrichment::Enrichment`** is the only thing allowed to call either
  `AdsbdbSource::fetch_*` method (privacy rule 2.2's gate, `should_enrich`, is the literal first
  line of `on_selection`). Two cache layers: the persistent one is the `aircraft` table's own
  `fetched_at`/`lookup_failed_at` (shipped unused since M1's migration 0001, now finally read);
  an in-memory `lru::LruCache` (capacity 256) sits in front of it per lookup kind so a repeat
  selection within one process run never round-trips to the store thread. A route's persistent
  check has no negative half (`flights` has no `lookup_failed_at`-equivalent column) — instead
  `latest_flight(icao24)` counts as a hit only when its `callsign` still matches the current
  selection, so a genuine callsign change (a new flight segment) is always worth a fresh lookup
  while a re-select of the same still-current flight is not. `flights` writes are plain inserts,
  never upserts — one row per resolved, non-cached lookup; the session-boundary merge the
  table's own shape (`first_seen`/`last_seen`) implies is still M5's, once `positions` exists to
  drive it.
- **Testing `Enrichment` without wiremock**: `ingest::http::HttpClient::build` and
  `AdsbdbSource::build` (the allowlist-widening, mock-server-pointed constructors every other
  adapter's tests use) are deliberately `pub(crate)` — privacy rule 1.1's "the only way to a
  client outside `ingest` is `HttpClient::new`, which cannot be talked out of the allowlist."
  That meant `app`'s own tests could not build a mock-backed `AdsbdbSource` the way
  `crates/ingest`'s own tests do. Resolved with a small `EnrichmentSource` trait (`#[async_trait]`,
  same dyn-compatibility reason `core::contracts::LiveSource` already uses it) that `AdsbdbSource`
  implements for production and a call-counting `FakeSource` (behind `Arc`, so a test keeps its
  own handle to the same counters `Enrichment` calls into) implements for tests — `AdsbdbSource`'s
  own HTTP/parsing correctness stays `ingest`'s tested responsibility; `app`'s tests exercise only
  the gate/cache/persistence orchestration, transport-independent. The acceptance line itself
  ("selecting an anonymous aircraft fires zero enrichment HTTP requests") is
  `enrichment::tests::selecting_an_anonymous_aircraft_fires_zero_enrichment_requests`, asserting
  the fake's call counters directly rather than a wiremock `expect(0)`.
- Verification: full workspace `cargo fmt --check` / `cargo clippy --workspace --all-targets -D
  warnings` / `cargo test --workspace` — 625 passed, 8 ignored (live-only: 6 pre-existing plus
  the two new `live_adsbdb_*` tests, both separately confirmed passing against the real
  `api.adsbdb.com` during the `ingest` lane's own work), 0 failed. A real window-mode boot (real
  `credentials.json`, real OpenSky poll cycle, 12196 aircraft tracked) confirmed
  `App::start`'s new `AdsbdbSource`/`Enrichment` construction path does not panic or otherwise
  fail; an actual click-triggered selection was not exercised live — the same scripted-navigation
  reliability gap 3.2's and 3.3's own gate records already carry, not a new one.

## 2026-07-21 — M3 item 3.5 lands: selection info card enrichment fields

- **`UNKNOWN` instead of a dash for unresolved fields**: 3.5's checklist wording ("'—' for any
  unknown field") and docs/13's own acceptance line both write a dash, but
  `render::label_atlas::CHARSET` (the stroke-font glyph set every on-screen text line is checked
  against, `info_card`'s own charset test included) has no dash character of any kind — only
  `A`-`Z`, `0`-`9`, space, and the two lowercase letters `label`'s unit suffixes need. Widening
  the charset for one placeholder glyph was rejected the same way 2.8b rejected it for
  lat/lon punctuation: out of scope for what this item's own wording names. `UNKNOWN` reads the
  same, costs nothing in the atlas, and matches "UNIDENTIFIED"'s existing convention for "nothing
  to show here" one line up.
- **The enrichment fields (`TYPE`/`OPR`/`RTE`) are always shown, never omitted** — the opposite of
  2.8b's original callsign/altitude/speed convention (omitted when unknown). Both conventions now
  coexist in the same `format_lines` deliberately: 3.5's own checklist line ("'—' for any unknown
  field, *never an error state*") reads as "always show something," while 2.8b's fields were
  never given that requirement and changing their existing, tested behavior wasn't in scope here.
- **`app::window::App` reads the store synchronously in `maybe_select`, not per-frame** —
  `store::Writer`'s methods block on a channel round-trip to the writer thread (`Writer::call`),
  which is exactly the kind of I/O ADR-005 says must never sit in the render loop. Per-frame was
  never seriously considered; the design that shipped mirrors `maybe_retarget`'s own already-
  established shape (a synchronous store read at a debounced trigger — camera-settle there,
  click-to-select here — cached into a plain `App` field, read cheaply every frame after). Two
  new fields, `selected_meta`/`selected_flight`, reset on every selection change including a
  deselect. Consequence accepted, not hidden: a first-ever selection of a given aircraft shows
  `UNKNOWN` for type/operator/route even after 3.4's background adsbdb fetch later completes and
  persists — nothing re-triggers the card to refresh mid-selection, only a fresh reselect reads
  the now-populated store row. `app::enrichment`'s own doc comment already anticipated this split
  ("3.5 reads the persisted data when it builds the info card, not from this cache"); a live
  push-on-fetch-completion channel was considered and rejected as more machinery than this item's
  scope asked for.
- **Privacy rule 2.2, defense in depth**: the store read in `maybe_select` is itself gated on
  `!instance.anonymous`, so `selected_meta`/`selected_flight` are never even populated for an
  anonymized target — belt-and-suspenders alongside `format_lines`' own anonymous branch, which
  returns before reading those fields regardless of what they hold.
- Verification: full workspace `cargo fmt` / `cargo clippy --workspace --all-targets` / `cargo
  test --workspace` — 629 passed (4 net new: `unresolved_enrichment_fields_show_unknown_never_
  omitted_or_an_error`, `a_route_with_only_one_known_end_shows_unknown_in_the_other_slot`,
  `with_enrichment_fills_type_operator_and_route_from_a_meta_and_flight_lookup`,
  `with_enrichment_leaves_fields_unknown_when_meta_and_flight_are_none`), 8 ignored (live-only,
  unchanged), 0 failed. Not live-verified by an actual click-triggered selection showing real
  type/operator/route text on screen — the same scripted-navigation reliability gap 3.2/3.3/3.4's
  own gate records already carry, not a new one; the underlying GPU text path
  (`pack_overlay_instances`) is unchanged from 2.8b's own already-confirmed rendering.

## 2026-07-21 — M3 item 3.6 gate: acceptance lines, docs/13 QA, kill-switch test

- **Baseline re-verified before checking anything else**: full workspace `cargo fmt --check` /
  `cargo clippy --workspace --all-targets` / `cargo test --workspace` — 629 passed, 8 ignored,
  0 failed, identical to 3.5's own recorded count, confirming the working tree hadn't drifted
  since 3.5's last live verification (no code changed this session; 3.1–3.5 remain uncommitted).
- **Kill-switch test, live-verified.** The owner blocked `api.adsbdb.com` and
  `aviationweather.gov` via the Windows hosts file (an admin-only system edit outside this
  process's own privileges, so the owner applied and later reverted it; `Resolve-DnsName`
  confirmed both the block and the revert). `OurAirports` was not hosts-blocked: 3.1 already
  bundles it at build time (`store` has no network deps by design), so nothing at runtime ever
  contacts it — blocking a host nothing calls would prove nothing.
  - A live `look-above.exe` (release) window-mode run kept OpenSky live positions flowing
    normally throughout (10k+ aircraft tracked across several poll cycles, credits accruing
    normally) while the METAR poller's next cycle hit the blocked host and logged
    `metar poll cycle failed; retrying next cycle error=network error: error sending request` —
    a plain `tracing::warn!`, not a panic. No retry storm: `ingest::metar::run_metar_poller`
    structurally cannot produce one — on any error it just logs and sleeps the normal
    `MIN_POLL_INTERVAL` (≥10 min), the same wait as a successful cycle, regardless of error kind.
  - Getting the METAR poller to actually attempt a fetch required a real camera-settle event
    first (`app::window::App::maybe_retarget` only retargets the METAR station list — and only
    if the resulting query differs from `last_sent_region` — on a genuine camera change, which
    the app's default launch state never produces on its own). Synthetic Win32 `SendMessage`
    (`WM_MOUSEWHEEL`) reliably reached the running window and changed the camera zoom/region —
    more reliable than the drag-based automation 3.2's gate record found unreliable for precise
    targeting, since no precision was needed here, only *some* change. One side effect
    reproduced, live, the already-known carried renderer blocker (whole-world trails exceeding
    wgpu's 256 MiB buffer panic) when a first zoom pulse landed while the view was still at
    global scale with dense trails; the second attempt zoomed in immediately after launch,
    avoided lingering at global scale, and completed cleanly. This is confirmation the blocker
    is still live, not a new finding — already tracked in CURRENT_STATUS, unrelated to 3.6's own
    scope, and not fixed here (M4/renderer LOD work owns it).
  - adsbdb's own network-error handling was **not** exercised live — enrichment only fires from
    `App::maybe_select`, which needs an actual click, the same scripted-navigation automation gap
    3.2/3.4/3.5 already carry (re-attempting the identical approach that already failed three
    times was judged not worth another try this session; docs/13's own "one focused attempt, then
    record the gap" instruction was followed instead). Static/code-level evidence stands in:
    `app::enrichment`'s `Err(error) => tracing::warn!(...)` arms (both `fetch_aircraft` and
    `fetch_route`) never populate the LRU or persistent negative-cache on a network error — only
    a confirmed 404 does — so a blocked host cannot poison the cache, and retries are bounded by
    user click rate, not any machine-paced loop.
- **docs/13 §Selection & overlays QA — not a fresh full pass, evidence reused deliberately.**
  METAR badge colors were already live-verified in 3.3 and nothing render-relevant changed since
  (confirmed by the identical 629-test count above) — re-running that check would have been pure
  overhead. Click hit-testing and info-card content still carry 3.2/3.4/3.5's own recorded
  automation gap. **Emergency squawk styling is not carried-gap territory — it does not exist**:
  no `squawk` field anywhere in `core::contracts`, `ingest`, or `render`, and no M1–M4 checklist
  item scopes building it, despite docs/01 (line 66) and privacy rule 6.1 both documenting the
  intended behavior and docs/13 listing it under "required at M3/M4." Per CLAUDE.md's own
  instruction not to guess or hunt for missing milestones, this is recorded here as an open
  question for the owner (which milestone should own it) rather than assumed into M4's plan.
- **Acceptance record (docs/11 §M3), evidence per line:**
  1. OurAirports import/count — done, 3.1 (evidence: DECISION_LOG 2026-07-20, M3 3.1). Rendering
     half done, 3.2. **L1/L2 tier-switching half stays open into M4** — recorded when M3 opened
     (tension note at the top of the M3 plan) and reconfirmed here, not a new gap.
  2. METAR badges (age/cadence) — done and live-verified, 3.3.
  3. Selecting a normal aircraft shows type/operator/route — implemented and unit-tested, 3.4/3.5.
     **Live click-triggered confirmation stays open** — same carried gap as above.
  4. Zero enrichment requests for an anonymous selection — done, unit-tested, 3.4
     (`enrichment::tests::selecting_an_anonymous_aircraft_fires_zero_enrichment_requests`).
  5. Kill-switch test — done and live-verified, this item.
- Gate table updated in `plans/CURRENT_STATUS.md`: "5/5 acceptance lines evidenced; 2 carry open
  halves (L1/L2 tier-switching → M4; click-triggered live verification), both pre-existing." Per
  CLAUDE.md, M3 is now gated and M4 is not started unprompted.

## 2026-07-21 — M4 item 4.2: orthographic globe camera, cursor-anchored zoom scoped as a linear
approximation

- **Cursor-anchored zoom on `GlobeCamera` uses a per-frame first-order correction, not an exact
  spherical-rotation solve.** True anchoring (keeping the world point under a moving cursor fixed
  on screen through the whole zoom ease) is a 2-unknown/2-constraint nonlinear system (solve for
  the sub-observer lat/lon that puts a given world point at a given disk position) with no closed
  form found in the time this item warrants. Instead `correct_toward_anchor` nudges `center` each
  frame using the small-angle partials of `orthographic_forward` near the observer point
  (`∂x/∂λ₀ ≈ -cos(φ₀)`, `∂y/∂φ₀ ≈ -1`), which is exact for anchors near screen center and
  converges over the ease's remaining frames otherwise. Off-globe cursor clicks (no world point
  under the cursor) fall back to center-anchored scaling. Documented in the module doc and
  `correct_toward_anchor`'s doc comment; item 4.2's acceptance criteria only test the projection
  function's visibility/bounds/NaN properties, not zoom-anchor precision, so this was in scope to
  defer. Revisit only if 4.3's manual QA finds the drift objectionable at typical zoom speeds.
- **`GlobeCamera` has no drag-inertia (no `begin_drag`/`drag_to`/`end_drag`/coasting velocity)**,
  unlike `Camera`. The 4.2 checklist item's wording ("pan/rotate + cursor-anchored zoom
  analogues") only names those two capabilities; inertia is UX polish addressable during 4.3's
  renderer wiring if wanted, not required by this item's property-test acceptance criteria.
- **No upper zoom-in bound on `radius_px`** (only a `MIN_RADIUS_PX` floor against a degenerate
  non-positive radius). Unlike `Camera`'s letterbox ceiling (a real "whole world visible" cap),
  there is no equivalent meaningful constraint yet for the globe — L0 is a wide-framing tier by
  definition, and any tier-transition-driven cap belongs with 4.3/4.4's tier wiring, not this
  pure-math item.

## 2026-07-21 — M4 item 4.3: globe scope, camera input routing, a real bug found live, one gap carried to 4.4

- **Full spherical basemap, not a placeholder disk** — owner chose this explicitly (AskUserQuestion)
  over a plain ocean-colored circle or no globe surface at all, even though the checklist line only
  named the density-dot layer as new geometry. Cost: a second, independent tessellation path
  (`basemap::tessellate_globe`, raw lon/lat radians, no Mercator projection) and a new shader
  (`globe_basemap.wgsl`) doing the orthographic projection per-vertex with a per-fragment `cos_c`
  discard (not a per-vertex all-or-nothing test) so a triangle straddling the true horizon clips
  along the correct curve instead of popping/jagging.
- **Both cameras (`Camera`, `GlobeCamera`) receive every raw drag/wheel/resize input
  unconditionally**, independent of which one is currently visible. Rationale: `LodTier`/
  `mode_blend` must keep deriving from `Camera::viewport_span_km()` alone (no change to 4.1's
  contract, no invented Mercator-`meters_per_pixel`-to-globe-`radius_px` conversion), while
  `GlobeCamera` needs to stay live and controllable the instant it's visible. Known consequence,
  owner-accepted rather than silently shipped: because the two cameras' framings diverge freely
  (Mercator pan/zoom vs. globe rotate/zoom are unrelated degrees of freedom), the still-ungated
  aircraft-glyph/trail/label layers (drawn via the Mercator camera; gating them is explicitly 4.4's
  job) visibly float disconnected from whatever the globe is currently showing once a user drags/
  scrolls to explore the globe. Confirmed live in a windowed run against real OpenSky traffic
  (~7,000 aircraft) and left as-is rather than pulled forward into 4.3 — see the carried line below.
- **Bug found and fixed during the live visual pass, not by automated tests**: the first
  implementation faded the *new* globe basemap in correctly but never faded the *pre-existing* flat
  Mercator basemap out, so at full L0 the flat map stayed opaque and visible in the four corners
  outside the globe's inscribed disk — read, at a glance, as "the whole world is still flat," even
  though the disk's interior was already a correctly-clipped hemisphere. Root-caused by rendering a
  headless frame to PNG and inspecting it, plus an independent property test round-tripping the
  globe mesh through `core::geo::orthographic_forward` (53/47 near/far split — ruled out a
  degrees/radians or lat/lon mixup). Fixed by giving the Mercator land/coastline pipeline a
  rewritable alpha (`BasemapLayer::set_alpha_multiplier`, driven by `1.0 - mode_blend` every frame)
  instead of leaving it permanently opaque. Regression test
  (`globe_mode_fades_out_the_flat_mercator_map_outside_the_disk`) verified to fail against the
  pre-fix code and pass with the fix — this class of "renders something, but the wrong shape" bug is
  exactly what the existing headless smoke test's "some pixels are non-background" assertion doesn't
  catch, so the new test asserts pixel color *outside* the expected disk specifically.
- **Carried gap → explicitly scoped into 4.4, not reopened as new**: aircraft glyphs/trails/labels
  float independently of the globe's current rotation while a user is actively exploring L0 (see the
  input-routing point above). 4.4's own "gate `TrailLayer`/`LabelLayer` to `Regional` only... glyph
  vs. density-dot drawing to the correct tier" already covers the fix (once glyphs simply don't draw
  at `Global`, there's nothing to float disconnected). Owner confirmed accepting this as a temporary
  4.3→4.4 rough edge rather than pulling gating work forward.
