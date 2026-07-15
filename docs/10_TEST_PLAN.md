# 10 — Test Plan

Principle: **tests never touch the network.** Everything external is exercised through
recorded, trimmed fixtures; live behavior is verified by running the app (visual QA, doc 13)
and by the M1 gate's supervised live run.

## Layers

### 1. Unit tests (`crates/core` mostly)

- **Geo math** (`core::geo`): haversine distance, initial/true bearing, destination-point
  (dead reckoning step), Web Mercator forward/inverse, orthographic globe projection.
  - Golden values from published test vectors (e.g., LAX→JFK great-circle distance/bearing).
  - Property tests (`proptest`): `inverse(forward(p)) ≈ p` within 1e-9°; bearing symmetry;
    antimeridian and polar edge cases explicitly pinned.
- **Interpolation / dead reckoning** (`core::sim`): position advances along track at ground
  speed; vertical rate integrates; correction blend converges to the new fix within the blend
  window and never moves backwards along track; stale-fade timing.
- **Dedup & staleness** (`core::merge`): same aircraft from two sources → newest `ts` wins;
  out-of-order updates dropped; anonymity flag is sticky for a session (privacy 2.2 —
  a later identified record for a previously-anonymous hex does not un-anonymize it).
- **Label collision culling**: no two output rects overlap; priority ordering respected.
- **Rate budgeting** (`ingest::budget`): OpenSky credit math per bbox area; poller cadence
  slows as remaining daily budget drops; 429 backoff schedule.

### 2. Adapter tests (fixtures) (`crates/ingest`)

- Fixtures in `tests/fixtures/<source>/*.json` — recorded by `scripts/record_fixture.rs`,
  trimmed to ≤ 20 records, credential-scrubbed (doc 04 §7.2).
- Required cases per source: nominal, empty region, nulls-in-every-optional-field,
  `"ground"` altitude string (airplanes.live), malformed record mid-array (parse must skip,
  not fail), 429 and 5xx response bodies.
- HTTP layer mocked with `wiremock`; assert request shape (params, auth header, User-Agent)
  as well as parse output.

### 3. Store tests (`crates/store`)

- Migrations apply cleanly on empty DB and are idempotent-by-version; `user_version` advances.
- Batched insert → bbox query round-trip; pruning deletes exactly the expired range in
  batches; retention/disk-cap policy (doc 08).
- In-memory SQLite (`:memory:`) for speed; one on-disk WAL smoke test.

### 4. Renderer smoke tests (`crates/render`)

- Headless wgpu (fallback adapter) render of a synthetic `RenderFeed` (1k aircraft, fixed
  seed) to an offscreen texture: asserts pipeline creation succeeds, draw calls submit, and
  non-background pixel count is within an expected band (catches "renders nothing" and
  "renders garbage everywhere" regressions without brittle image diffs).
- Skipped (not failed) with a warning if no adapter is available in CI.

### 5. Benchmarks (`criterion`, run at gates, not in CI-per-push)

- `sim::advance_all` for 10k aircraft — budget: < 2 ms on 8 cores (rayon).
- Projection batch 10k points — budget: < 0.5 ms.
- Store insert 10k positions — budget: < 50 ms.
- Regressions > 20% at a milestone gate block the gate.

### 6. Live verification (manual, gate-time only)

- M1 gate: 10-minute supervised live run within rate budget (doc 07).
- M2+ gates: visual QA checklist ([13_VISUAL_QA_CHECKLIST.md](13_VISUAL_QA_CHECKLIST.md)).

## CI (GitHub Actions, from M0)

`on: push` — `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
`cargo test --workspace` on `windows-latest` (primary target) and `ubuntu-latest` (portability
signal). No secrets in CI; no live tests.

## Privacy-rule regression tests

- Enrichment gate: constructing a selection lookup for an `anonymous` StateVector returns
  a compile-time-unreachable path or runtime refusal (test asserts the refusal).
- History replay of a fixture with anonymous aircraft never surfaces callsign/registration.
- Allowlist test: `ingest` has a single const list of permitted hosts; test walks all adapter
  base URLs and asserts membership (any new endpoint must be added deliberately).
