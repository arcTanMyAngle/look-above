# 11 — Acceptance Criteria (per milestone gate)

Binary, measurable checks. A gate passes when every box is checked and the result is
recorded (with numbers) in `plans/CURRENT_STATUS.md`. Roadmap context:
[07_MILESTONE_PLAN.md](07_MILESTONE_PLAN.md).

## M0 — Repo & Architecture

- [ ] `cargo build --workspace` succeeds on a clean clone (Windows, stable toolchain).
- [ ] CI runs fmt + clippy(-D warnings) + tests on push; badge green.
- [ ] Workspace has the five crates with the dependency direction core ← {ingest, store, render} ← app (verified: no reverse deps in `cargo tree`).
- [ ] `cargo run -p look-above` opens an empty window, resizes without panic, closes cleanly.
- [ ] Config loads from `config.toml` + env override; missing file yields defaults, not error.
- [ ] `config.toml` is gitignored; repo contains `config.example.toml`.
- [ ] ADRs 001–005 accepted; DECISION_LOG updated.

## M1 — Authorized Data Ingestion

- [ ] OpenSky adapter authenticates via OAuth2 client credentials; token auto-refreshes (observed across a > 30 min run).
- [ ] Credit budgeting: 10-min live run over a ~500×500 km bbox stays ≤ 80% of pro-rated daily budget; spend visible in `source_status`.
- [ ] Fallback adapter (airplanes.live or adsb.lol) returns normalized `StateVector`s for the same bbox; poller fails over automatically when the primary errors, and recovers.
- [ ] Dedup: with both sources active, no duplicate (icao24) in a merged snapshot; newest ts wins (unit-tested + observed in logs).
- [ ] All adapter fixture tests pass, including malformed-record and 429 cases.
- [ ] Anonymity flag populated and sticky (privacy 2.2 regression test passes).
- [ ] 10 continuous minutes of live polling with zero panics and zero rate-limit violations (no 429 received, or 429 honored with backoff and logged).

## M2 — High-Fidelity Renderer

- [ ] 60 fps sustained (frame time p95 < 16.6 ms, measured by the built-in frame stats overlay) with live regional traffic (≥ 200 aircraft).
- [ ] Aircraft glide: no visible teleport on update arrival; correction blend ≤ 2 s (visual QA L2-core + slow-motion capture check).
- [ ] Glyph heading matches true track (spot-check ≥ 10 aircraft against reported track values).
- [ ] Stale aircraft fade out after 60 s and are removed; none frozen on screen.
- [ ] Base map renders (coastlines/borders) with pan/zoom; no seams at antimeridian in the test region.
- [ ] Renderer smoke test passes headless; interpolation benchmarks within budget (doc 10 §5).

## M3 — Enrichment & Non-ADS-B

- [ ] OurAirports import completes; airport count in DB within 5% of source CSV row count; large/medium airports visible at L1, runway outlines at L2.
- [ ] METAR badges show current flight category for visible large airports; data age ≤ 70 min; API polled ≤ 1×/10 min (log-verified).
- [ ] Selecting a normal aircraft shows type/operator/route when adsbdb has them; unknown → "—", never an error state.
- [ ] Selecting an anonymous aircraft performs **zero** enrichment HTTP requests (log-verified) and displays "Unidentified".
- [ ] Kill-switch test: with all enrichment sources blocked (hosts file), the tracker runs indistinguishably minus enrichment.

## M4 — Dual-Mode LOD & Interaction

- [ ] Continuous zoom from globe to single-runway with no popping, label flicker, or mode seams (visual QA full pass).
- [ ] Global mode: ≥ 8,000 aircraft at 60 fps (frame stats overlay evidence).
- [ ] LOD hysteresis: zooming to a threshold and dithering ±5% does not flip tiers.
- [ ] Globe↔mercator camera transition ≤ 500 ms, interruptible mid-flight.
- [ ] No label overlaps at any zoom (automated collision test + visual pass).

## M5 — Persistence, History & Replay

- [ ] 24 h continuous run: process RSS < 1 GB, DB ≤ 1 GB, pruning observed in logs, zero panics.
- [ ] Trails render from stored history after app restart.
- [ ] Replay scrubber covers the retention window; playback speed ≥ 60× realtime.
- [ ] Anonymous-in-live stays anonymous-in-replay (privacy 5.2 regression test).
- [ ] Retention setting honored: lowering it prunes on next cycle.

## M6 — Polish & Packaging (v1)

- [ ] Clean-clone-to-tracking in ≤ 5 min following README only (timed, on a machine without the repo).
- [ ] No-credentials first run degrades to fallback source with a clear in-app explanation.
- [ ] Attribution screen lists OpenSky, community aggregators, NOAA, OurAirports, Natural Earth.
- [ ] Settings persist across restarts; light theme passes visual QA contrast checks.
- [ ] All M0–M5 test suites still green; all product-vision success criteria confirmed.
