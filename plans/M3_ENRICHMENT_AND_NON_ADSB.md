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
      *(2026-07-20: implemented — new `import-ourairports` binary in `crates/import` (host
      allowlist + own unit tests, mirroring `import-basemap`'s exact shape), migration
      `0002_airports.sql` verbatim from docs/08, `AirportSize::from_ourairports_type` added to
      `core::contracts` (shared by both `import` and `store` so the type-drop ladder isn't
      duplicated), a new `crate::ourairports` module in `store` (idempotent bundled-CSV seed +
      `airports_in_bbox` query), and `Writer::airports_in_bbox` wired through the existing
      `Command`-channel pattern — **not** a full `core::contracts::Store` impl (still blocked on
      `positions`, M5's table, exactly as `lib.rs`'s own doc comment already explained). Did not
      split 3.1a/3.1b — scoping held as one item cleanly, unlike 2.2's basemap fetch (no
      shapefile/zip parsing here, just CSV). Delegated to the storage-agent (its stated remit:
      "enrichment imports (OurAirports, FAA registry, METAR cache)"), independently re-verified
      by this session: every changed/new file read in full, fresh `cargo fmt --check`/
      `clippy --workspace --all-targets -D warnings`/`test --workspace` — **539 passed, 5
      ignored, 0 failed** (+24 over 2.10's 515: 3 in `core::contracts`, 10 in the new
      `import-ourairports` binary, 2 net in `migrations.rs` after fixing the now-stale
      single-migration test to check each version's own table set honestly, 6 in
      `store::ourairports`, 3 in `store::writer`). Live run of the import tool against the real
      `davidmegginson.github.io` host confirmed the agent's reported counts exactly (bundled
      `airports.csv` 71,086 rows / `runways.csv` 43,240 rows, both re-derived independently via
      `wc -l` against the committed bundled assets — not just trusted from the report).
      **"Within 5%" interpreted against the *kept-type* source count (71,086: large/medium/
      small/heliport), not the raw 85,776-row upstream total** — the raw total includes ~13,355
      `closed` rows alone, which the M3 plan's own `AirportSize` mapping decision (and
      `crates/core/src/contracts.rs`'s pre-existing doc comment) already commits to dropping
      entirely, so comparing against the undropped raw count would be the wrong denominator, not
      a stricter reading of the acceptance line. Recorded here explicitly rather than left
      implicit, same as every other acceptance-line interpretation call this project has made at
      a gate. Runway *query* API stays out of scope (3.2's job); this item only needed runway
      rows to exist, seeded and orphan-filtered. DECISION_LOG 2026-07-20 (3.1).)*
- [ ] 3.2 Airport + runway rendering: markers for large/medium airports, runway-outline
      polylines at close zoom, reusing existing tessellation approach (`lyon`, per 2.2b's
      basemap precedent) rather than a new one. Scoped per the tension noted above — no LOD-tier
      gating (M4's job), just correct data drawn at the current single render tier.
- [ ] 3.3 METAR polling + flight-category badges: new `ingest` adapter for
      `aviationweather.gov` (batch ≤ 100 stations, ≥ 10 min spacing — enforced in code, not just
      documented), `metars` table (keep latest 2/station per docs/08 retention), flight-category
      badge (VFR/MVFR/IFR/LIFR color per docs/13) drawn near visible large airports. Acceptance:
      badge data age ≤ 70 min; polling interval log-verified ≥ 10 min.
- [ ] 3.4 adsbdb selection lookups: new `ingest` adapter for `GET /v0/aircraft/{hex}` and
      `GET /v0/callsign/{callsign}`, called **only** from the selection path and **only** when
      `anonymous == false` — this is a code gate (privacy rule 2.2), unit-tested as its own
      regression (mirrors 1.4's anonymity-sticky test). LRU + 24 h negative cache. Upserts
      `AircraftMeta`/`flights` (registration/type/operator/route). Acceptance: selecting an
      anonymous aircraft fires **zero** enrichment HTTP requests (log-verified).
- [ ] 3.5 Selection info card enrichment data path: extend 2.8b's `render::info_card` with
      type/operator/route, sourced from 3.4's cached `AircraftMeta`/`flights` lookup keyed off
      the currently-selected `icao24` — "—" for any unknown field, never an error state on a 404
      or cache miss. Anonymous-selected keeps 2.8b's existing "Unidentified" + position/altitude
      path untouched (no route/type ever shown there, per rule 2.2 — this item must not touch
      that branch).
- [ ] 3.6 Gate: docs/11 §M3 acceptance lines recorded with evidence in CURRENT_STATUS
      (same format as M0/M1/M2's gate tables); docs/13 §Selection & overlays QA; the kill-switch
      test (block adsbdb/aviationweather.gov/OurAirports hosts via the hosts file, confirm the
      tracker runs indistinguishably minus enrichment — no panics, no retry storms). Records the
      L1/L2 tier-switching half of acceptance line 1 as open-into-M4 per the tension noted above,
      the same honest-carry pattern as M1's token-refresh line and M2's three open lines.
