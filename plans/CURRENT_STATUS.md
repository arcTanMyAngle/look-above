# Current Status

> Startup handoff only. Read `Now`, then stop at the next `##` heading. Detailed history and
> rationale live in `plans/DECISION_LOG.md` and Git. Keep Now at no more than 10 bullets and
> retain only the 10 newest one-line session entries.

## Now (updated 2026-07-21)

- **Active milestone:** M3 — Enrichment & Non-ADS-B Integration.
- **Next action:** item 3.4, adsbdb selection lookups (anonymity-gated); see
  [M3_ENRICHMENT_AND_NON_ADSB.md](M3_ENRICHMENT_AND_NON_ADSB.md).
- **3.1–3.3 implemented but not yet committed:** OurAirports import, airport/runway rendering,
  and METAR polling + flight-category badges; 590 tests passed, 6 ignored, fmt/clippy clean.
- **3.3 visually confirmed live:** real `aviationweather.gov` fetch, colored VFR/MVFR badge
  rings seen on screen at their airports (plain gray where nothing is cached yet). Also fixed a
  bug this pass caught live: the METAR poller's empty-station-list case was sleeping the full
  10-minute interval before its first recheck, delaying every session's first badges by up to 10
  minutes — now a short `IDLE_RECHECK_INTERVAL` (5 s) applies while idle, `MIN_POLL_INTERVAL`
  only between actual fetches.
- **3.2 verification gap narrowed:** airport markers now visually confirmed (3.3's own live
  pass, same camera view); runway outlines still unconfirmed — that pass's zoom level was too
  far out for a runway to register at all.
- **Carried renderer blocker:** whole-world trails can exceed wgpu's 256 MiB buffer limit and
  panic; LOD/capping remains required before whole-world trail rendering is safe.
- **Carried visual gap:** dense regional labels are algorithmically non-overlapping but remain
  visually cluttered after the 2.10 size increase.

## Gate record

| Milestone | State | Evidence |
|---|---|---|
| M0 | Gate run 2026-07-15 — 6/7; CI badge awaits first remote workflow run | DECISION_LOG M0 gate |
| M1 | Gate run 2026-07-18 — 6/7; token-refresh line owner-accepted open | M1 plan 1.13 |
| M2 | Gate run 2026-07-20 — 3/6 clean; 3 lines carried | M2 plan 2.10 |
| M3 | Opened 2026-07-20 — 3.1–3.3 done; 3.4 next | M3 plan |
| M4–M6 | Not started | Plans written at preceding gates |

## Session log (newest first; keep 10)

- 2026-07-21 — M3 3.3 METAR polling + flight-category badges implemented and live-verified;
  fixed an idle-poller startup delay caught during that verification.
- 2026-07-20 — Token/throughput audit: bounded context and agents, delivery slices,
  risk-tiered checks, one-attempt visual QA, and a deterministic-navigation follow-up.
- 2026-07-20 — M3 3.2 airport/runway rendering implemented; visual confirmation gap recorded.
- 2026-07-20 — M3 3.1 OurAirports/runways import implemented.
- 2026-07-20 — M3 opened; plan written and M2 carry-overs retained.
- 2026-07-20 — M2 2.10 gate run; 3/6 acceptance lines clean, three carried.
- 2026-07-19 — M2 2.9 headless renderer smoke test added.
- 2026-07-19 — M2 2.8 selection outline and info card completed.
- 2026-07-19 — M2 2.7 labels and F3 overlay completed.
- 2026-07-19 — M2 2.6 trails completed; whole-world buffer risk identified.
