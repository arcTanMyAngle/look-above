# M3 — Enrichment & Non-ADS-B Integration

**Goal:** static/enrichment data (airports, runways, METAR, aircraft metadata/routes) layered
onto the live picture without ever touching the live-position render/sim path's performance or
the privacy gate. Exit criteria: [../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md) §M3.
Constraining docs: 04 (rule 2.2 — the anonymity enrichment gate, and 4.3 — no private-owner
names), 08 (`airports`/`runways`/`airlines`/`metars` tables — schema already fully specified,
migration 0002+ implements it), 09 (`AircraftMeta`/`Airport`/`AirportSize`/`Store` contracts —
already defined in `core::contracts`, unused until this milestone), 13 (§Selection & overlays),
and the [authorized-aviation-sources skill](../.claude/skills/authorized-aviation-sources/SKILL.md)
(adsbdb / aviationweather.gov / OurAirports sections — read before any new HTTP call).

## Known cross-milestone tension (read before 3.2)

Acceptance line 1 says airports should be "visible at L1, runway outlines at L2" — but L0/L1/L2
**LOD tier switching doesn't exist yet**. M2's own gate (2.10, carried forward) flagged that the
renderer currently draws everything at one fixed L2-style tier; true tier cross-fade is M4's
deliverable (docs/07). 3.2 is scoped to build the *query-side* size filtering
(`Store::airports_in_bbox(bbox, min_size)` already takes `min_size` — M2 pattern was to plug
into what M4 will drive later) and render markers/runway outlines at whatever the current
single-tier view shows; it does **not** attempt to fake LOD-gating ahead of M4. This is the same
shape as M2's own LOD flag — recorded here rather than silently worked around, and the M3 gate
(3.6) checks the acceptance line's *data* half (right airports, right runways) honestly, noting
the tier-switching half stays open until M4 the same way M1's token-refresh line stayed open into
M2's gate record.

## Checklist

- [x] 3.1 OurAirports import: fetch `airports.csv`/`runways.csv` (URLs in the sources skill),
      convert/bundle the same no-runtime-fetch way 2.2a bundled Natural Earth (`crates/import`
      already exists, depended on by nothing) — `store` has no network deps and must stay that
      way (M0 acceptance line 3). New migration `0002_airports.sql` (verbatim from docs/08,
      append-only per its own convention), `Store::airports_in_bbox` real implementation, an
      import routine that seeds the bundled snapshot on first run. Map `OurAirports` `type` onto
      `AirportSize` (heliport/small/medium/large; drop `seaplane_base`/`balloonport`/`closed` —
      docs/09's own documented-at-import-time decision). Acceptance: airport count within 5% of
      source CSV row count.
      *(Done 2026-07-20: bundled OurAirports import, migration, size mapping, seed, and
      bbox query; 539 tests passed. The 5% check uses the kept-type source count. Evidence and
      rationale: DECISION_LOG 2026-07-20, M3 3.1.)*
- [x] 3.2 Airport + runway rendering: markers for large/medium airports, runway-outline
      polylines at close zoom, reusing existing tessellation approach (`lyon`, per 2.2b's
      basemap precedent) rather than a new one. Scoped per the tension noted above — no LOD-tier
      gating (M4's job), just correct data drawn at the current single render tier.
      *(Done 2026-07-20: runway contract/query plus airport-marker and runway render layers;
      553 tests passed. Live launch was healthy, but airport/runway pixels remain visually
      unconfirmed due unreliable scripted navigation. Evidence: DECISION_LOG 2026-07-20, 3.2.)*
- [x] 3.3 METAR polling + flight-category badges: new `ingest` adapter for
      `aviationweather.gov` (batch ≤ 100 stations, ≥ 10 min spacing — enforced in code, not just
      documented), `metars` table (keep latest 2/station per docs/08 retention), flight-category
      badge (VFR/MVFR/IFR/LIFR color per docs/13) drawn near visible large airports. Acceptance:
      badge data age ≤ 70 min; polling interval log-verified ≥ 10 min.
      *(Done 2026-07-21: `ingest::metar` (fetch + a dedicated single-source poller, no failover
      chain — docs/09 lists exactly one METAR provider), migration 0003 + `store::metar`
      (upsert/retention/query), core `Metar`/`FlightCategory`/`MetarBadge` contracts, a per-instance-
      colored badge ring layer in `render` (reuses `airport::marker_mesh`, drawn before the
      airport-marker pass so the marker paints over the ring's center), and `app::window` wiring
      (poller spawn, station retarget + badge join piggybacked on the existing camera-settle
      trigger, same as 3.2). 590 total passed, 6 ignored (live-only), 0 failed; fmt/clippy clean.
      Live-verified end to end: real `aviationweather.gov` fetch,
      colored VFR/MVFR badge rings visually confirmed on screen at their airports, plain gray for
      airports with no cached observation yet — see DECISION_LOG 2026-07-21, M3 3.3.)*
- [x] 3.4 adsbdb selection lookups: new `ingest` adapter for `GET /v0/aircraft/{hex}` and
      `GET /v0/callsign/{callsign}`, called **only** from the selection path and **only** when
      `anonymous == false` — this is a code gate (privacy rule 2.2), unit-tested as its own
      regression (mirrors 1.4's anonymity-sticky test). LRU + 24 h negative cache. Upserts
      `AircraftMeta`/`flights` (registration/type/operator/route). Acceptance: selecting an
      anonymous aircraft fires **zero** enrichment HTTP requests (log-verified).
      *(Done 2026-07-21: `flights` pulled forward from M5 to back route caching — owner decision,
      see DECISION_LOG; `ingest::adsbdb::AdsbdbSource` (pure adapter, live-verified against real
      `api.adsbdb.com`) + `store::Writer::upsert_aircraft_meta`/`aircraft_meta`/`insert_flight`/
      `latest_flight` (migration 0004) + `app::enrichment::Enrichment` (the gate, the two-layer
      LRU-then-persistent-store cache, and the only caller of either adsbdb fetch), wired into
      `App::maybe_select` as a spawned task off the render/event loop. 625 workspace tests
      passed, 8 ignored (live-only), 0 failed; fmt/clippy clean. The "zero enrichment HTTP
      requests for an anonymous selection" acceptance line is a direct unit test
      (`enrichment::tests::selecting_an_anonymous_aircraft_fires_zero_enrichment_requests`) against
      a call-counting fake source, not wiremock — `ingest`'s allowlist-widening constructors are
      `pub(crate)` on purpose, so a real mock-server-backed adapter can't be built from `app`.
      Live-verified: a real window-mode boot (real OpenSky credentials, real poll cycle, 12196
      aircraft tracked) confirmed the new `Enrichment`/`AdsbdbSource` construction path in
      `App::start` does not panic or fail; the click-triggered live path itself stays unconfirmed
      by an actual selection, the same scripted-navigation gap 3.2's and 3.3's own gates recorded
      — see DECISION_LOG 2026-07-21, M3 3.4.)*
- [x] 3.5 Selection info card enrichment data path: extend 2.8b's `render::info_card` with
      type/operator/route, sourced from 3.4's cached `AircraftMeta`/`flights` lookup keyed off
      the currently-selected `icao24` — "—" for any unknown field, never an error state on a 404
      or cache miss. Anonymous-selected keeps 2.8b's existing "Unidentified" + position/altitude
      path untouched (no route/type ever shown there, per rule 2.2 — this item must not touch
      that branch).
      *(Done 2026-07-21: `render::info_card::InfoCardContent` gained
      `type_code`/`operator`/`route_origin`/`route_destination` plus a `with_enrichment` builder;
      `format_lines` shows `TYPE`/`OPR`/`RTE` lines with `UNKNOWN` (not a dash — the label-atlas
      charset has none) for anything unresolved, appended before the existing `SRC` line, and the
      anonymous branch returns before ever reading them. `app::window::App` reads
      `Store::aircraft_meta`/`latest_flight` synchronously in `maybe_select` — the same
      "debounced trigger, not per-frame" shape `maybe_retarget` already uses for
      `current_airports` — gated on `!anonymous`, caching the result in two new `App` fields
      (`selected_meta`/`selected_flight`) folded into the per-frame `InfoCardContent` build. 629
      tests passed (4 new), 8 ignored (live-only), 0 failed; fmt/clippy clean. Not live-verified
      by an actual click-triggered selection — the same scripted-navigation gap 3.2/3.3/3.4
      already carry, not a new one; see DECISION_LOG 2026-07-21, M3 3.5.)*
- [x] 3.6 Gate: docs/11 §M3 acceptance lines recorded with evidence in CURRENT_STATUS
      (same format as M0/M1/M2's gate tables); docs/13 §Selection & overlays QA; the kill-switch
      test (block adsbdb/aviationweather.gov/OurAirports hosts via the hosts file, confirm the
      tracker runs indistinguishably minus enrichment — no panics, no retry storms). Records the
      L1/L2 tier-switching half of acceptance line 1 as open-into-M4 per the tension noted above,
      the same honest-carry pattern as M1's token-refresh line and M2's three open lines.
      *(Done 2026-07-21: full workspace fmt/clippy/`test --workspace` re-verified clean (629
      passed, 8 ignored, 0 failed) before the gate check. Kill-switch test live-verified: owner
      blocked `api.adsbdb.com`/`aviationweather.gov` via the hosts file (admin-only edit), a live
      window-mode run kept OpenSky live positions flowing (10k+ aircraft tracked) while the METAR
      poller hit the blocked host and logged a plain warning, retrying only at its normal
      ≥10-min cadence — no panic, no retry storm; OurAirports isn't fetched at runtime (bundled
      at build time) so wasn't hosts-blocked. adsbdb's own error handling was verified by reading
      `app::enrichment` (network errors warn-logged, never cached) rather than a live click — the
      selection path still needs an actual click, the same scripted-navigation gap 3.2/3.4/3.5
      already carry. docs/13 Selection & overlays: METAR badges already live-verified in 3.3
      (unchanged); click-dependent lines carry the same gap; a real gap found and recorded rather
      than silently skipped — emergency squawk styling (docs/01, privacy rule 6.1) has no
      implementation anywhere (no `squawk` field in any crate) and is unscoped by any M1–M4
      checklist item, flagged for the owner rather than guessed into a milestone. See
      DECISION_LOG 2026-07-21, M3 3.6 for full detail, including the reproduced (pre-existing,
      not new) whole-world-trail buffer panic hit once during setup.)*
