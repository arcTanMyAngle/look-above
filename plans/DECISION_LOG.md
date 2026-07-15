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
