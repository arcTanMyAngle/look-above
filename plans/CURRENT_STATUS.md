# Current Status

> Startup handoff only. Read `Now`, then stop at the next `##` heading. Detailed history and
> rationale live in `plans/DECISION_LOG.md` and Git. Keep Now at no more than 10 bullets and
> retain only the 10 newest one-line session entries.

## Now (updated 2026-07-22)

- **M4 items 4.1–4.3, 4.5, 4.4a done.** 4.4a's camera-sync fix (`Camera::set_center_latlon`,
  `crates/core/src/camera.rs`; called once from `App::draw`) is now live-verified: this session's
  live-pass log shows a globe-driven zoom onto Sardinia/southern Italy landing a ~91 km × 132 km
  bbox (well under the 300 km Regional threshold) at that exact real location, matching
  screenshots of full Regional-tier rendering there (colored heading-glyphs, labels, leader
  lines, info card).
- **M4 item 4.4 still open — two of its three acceptance lines unresolved after two live passes:**
  the whole-world-trails-vs-256-MiB-buffer blocker is verified resolved (reached ~12,500 tracked
  at Global, no panic, twice); but (a) **pop-free cross-fade**: owner reports popping still occurs
  at both tier boundaries — no gating/tint logic bug found on inspection (every aircraft's color
  comes from real altitude regardless of tier), leading suspicion is `TIER_BLEND_EASE_TAU_S`'s
  front-loaded exponential ease reading as a snap rather than a fade; owner declined an
  experimental fix this session, so no code changed for this line — it needs either a follow-up
  session's tau experiment or a clearer description of what pops; (b) **8,000+-aircraft p95**: this
  session's attempt was invalid (OpenSky failed over to `airplaneslive`'s 250 nm-capped 3–5
  aircraft mid-run) — needs a fresh F3 reading once OpenSky is healthy.
- **M4 plan** ([plans/M4_DUAL_MODE_LOD_AND_INTERACTION.md](M4_DUAL_MODE_LOD_AND_INTERACTION.md)):
  8 items — 4.1–4.3, 4.5, 4.4a done; 4.4 (two acceptance lines open), 4.6 (emergency squawk), 4.7
  (gate) remain.
- **Backlog idea, not scoped into any plan (owner's own request to defer, not pivot mid-session):**
  instead of defaulting to whole-world density on launch, a location-search-driven view — type an
  address or pick a point on the globe, see live flights overhead there. Distinct feature
  (geocoding, new default-view UX) from M4's tier-gating scope; needs its own milestone/plan
  scoping conversation before any implementation.
- **M3 gate closed 2026-07-21 (item 3.6):** docs/11 §M3's 5 acceptance lines all evidenced (2
  carry an already-recorded open half — L1/L2 tier switching → M4, now this plan; click-triggered
  live verification, pre-existing scripted-navigation gap).
- **3.1–3.5 remain uncommitted** in the working tree — untouched this session; committing is the
  owner's call.
- **Carried 3.2 gap:** runway outlines still visually unconfirmed at close zoom.
- **Carried visual gap:** dense regional labels are algorithmically non-overlapping but remain
  visually cluttered after the 2.10 size increase.

## Gate record

| Milestone | State | Evidence |
|---|---|---|
| M0 | Gate run 2026-07-15 — 6/7; CI badge awaits first remote workflow run | DECISION_LOG M0 gate |
| M1 | Gate run 2026-07-18 — 6/7; token-refresh line owner-accepted open | M1 plan 1.13 |
| M2 | Gate run 2026-07-20 — 3/6 clean; 3 lines carried | M2 plan 2.10 |
| M3 | Gate run 2026-07-21 — 5/5 acceptance lines evidenced; 2 carry open halves (L1/L2 tier-switching → M4; click-triggered live verification, both pre-existing) | M3 plan 3.6, DECISION_LOG 2026-07-21 |
| M4 | In progress — 5/8 items done (4.1–4.3, 4.5, 4.4a) | M4 plan, DECISION_LOG 2026-07-21/2026-07-22 |
| M5–M6 | Not started | Plans not yet written |

## Session log (newest first; keep 10)

- 2026-07-22 — M4 4.4a live-verified (globe-zoom landed a ~91×132 km Regional bbox at the
  targeted location); 4.4's pop-free cross-fade and p95 lines remain open after a second live
  pass — no code changed (owner declined an experimental tau fix, and a p95 attempt hit an
  OpenSky failover with too few aircraft). Location-search/default-view idea noted as backlog,
  not scoped.
- 2026-07-22 — M4 4.4a camera-sync fix implemented (Mercator camera snaps to the globe's
  sub-observer point on leaving `Global` tier); not yet live-verified, no GUI-automation driver
  for this app exists.
- 2026-07-22 — M4 4.5 Oklab altitude ramp implemented; camera-sync gap from 4.4 added as plan item
  4.4a; fixed a real pre-existing lightness-monotonicity gap in the M2/M3 ramp colors (owner
  approved the nudge).
- 2026-07-22 — M4 4.3 globe transition + L0 density layer implemented and live-verified against
  real traffic; fixed a flat-map-not-fading-out bug found during that pass; disconnected-glyphs
  gap knowingly carried to 4.4.
- 2026-07-21 — Wrote plans/M4_DUAL_MODE_LOD_AND_INTERACTION.md (7 items) after confirming
  approach with the owner; no implementation started.
- 2026-07-21 — M3 3.6 gate: acceptance lines recorded, docs/13 QA pass, live hosts-file
  kill-switch test passed; found emergency-squawk styling unimplemented and unscoped.
- 2026-07-21 — M3 3.5 selection info card enrichment (type/operator/route) implemented.
- 2026-07-21 — M3 3.4 adsbdb selection lookups implemented; `flights` table pulled forward from
  M5 to back route caching (owner decision).
- 2026-07-21 — M3 3.3 METAR polling + flight-category badges implemented and live-verified;
  fixed an idle-poller startup delay caught during that verification.
- 2026-07-20 — Token/throughput audit: bounded context and agents, delivery slices,
  risk-tiered checks, one-attempt visual QA, and a deterministic-navigation follow-up.
