# M1 — Authorized Data Ingestion

**Goal:** live, normalized, rate-budgeted aircraft state flowing from authorized sources into
the pipeline, fully fixture-tested. Exit criteria: [../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md) §M1.
Constraining docs: 09 (contracts), 04 (rules 1.x, 2.2, 7.x), 10 (§1 rate budgeting, §2 fixtures),
and the [authorized-aviation-sources skill](../.claude/skills/authorized-aviation-sources/SKILL.md).

## Prerequisite (user action)

> **Done 2026-07-15.** The owner created the API client and supplied its `credentials.json`.
> Item 1.3 reads that file as-issued (gitignored); no transcription into `config.toml` is
> needed. Precedence: `LOOK_ABOVE_OPENSKY_*` > `config.toml` > `credentials.json`. Verified
> live against the token endpoint — accepted, TTL 1798 s.

## Checklist

- [x] 1.1 `ingest::http`: shared reqwest client — 10 s timeouts, User-Agent per docs/09,
      backoff helper (exponential + jitter, honors Retry-After), `SourceError` mapping.
      *(2026-07-15: done — `ingest::http` (client, `send_json`, status/transport mapping) +
      `ingest::http::backoff` (pure `retry_delay`). 20 tests, wiremock-backed per docs/10 §2:
      the User-Agent and the timeout are asserted on the wire, not just as constants.
      `SourceError::Request { status }` added to `core` for non-auth, non-429 4xx — docs/09's
      taxonomy had no non-retryable home for a 400/404. `Retry-After` is a floor
      (`max(header, backoff)`), honored past the 5-min cap; parsed as delta-seconds only.
      Equal jitter, not full jitter — a 429 must never be retried milliseconds later. New
      deps: `fastrand` (jitter), `wiremock` (dev). Rationale in DECISION_LOG.)*
- [x] 1.2 Allowlist const + test (docs/10 §privacy): permitted hosts only.
      *(2026-07-15: done — `ingest::allowlist`: `AUTHORIZED_HOSTS` (the skill's six runtime
      hosts), `is_authorized_host` (exact + case-insensitive, never a suffix match), and
      `HostPolicy`, enforced in `HttpClient::get` and on **every redirect hop** — docs/10
      asked only for a test over declared base URLs, which would pass vacuously today and
      never see a dynamically built URL. 19 tests, 126 total. `SourceError::Refused` added
      to `core` — the taxonomy had no home for "we declined to send this". Static-download
      hosts deliberately excluded (import tooling, not this crate). Rationale in
      DECISION_LOG.)*
- [x] 1.3 OpenSky auth: OAuth2 client-credentials token fetch + cache + refresh at 80% TTL;
      credentials from config; graceful "no credentials" state (source disabled, not error).
      *(2026-07-15: done — `ingest::opensky::auth`: `OpenSkyAuth` (token cache, 80% refresh,
      `Ok(None)` when disabled), `Credentials`, injected `Clock`. 35 new tests, 161 total.
      **First live API call in the project**: an `#[ignore]`d test hit the real token endpoint
      with the owner's credentials — accepted, TTL 1798 s, refresh at 79.98%, confirming the
      documented ~30 min on real data rather than on a mock. `credentials.json` is read
      as-issued (owner call) and gitignored; it is all-or-nothing so a pair is never assembled
      from two sources. `SecretString` moved to `core::secret` so `ingest` can hold credentials
      without a second copy of privacy rule 7.1. `HttpClient::post_form` added — the allowlist
      choke point covered GET only, and the grant is a POST carrying the secret. Rationale in
      DECISION_LOG.)*
- [x] 1.4 OpenSky adapter: `/states/all` bbox query → `Vec<StateVector>`; positional-array
      parsing tolerant of nulls per field; credit cost function (bbox area → 1–4);
      fixture set per docs/10 §2.
      *(2026-07-15: done — `ingest::opensky::states`: `OpenSkySource` (implements
      `LiveSource`), positional-array parsing, `credit_cost`. 35 new tests, 196 total; four
      fixtures per docs/10 §2 (nominal, empty, nulls, malformed) — hand-written to the
      documented shape, since 1.10's recorder does not exist yet (provenance +
      re-record note in `tests/fixtures/opensky/README.md`). **The project's first live data
      request**: an `#[ignore]`d test fetched **72 real aircraft over Switzerland, every one
      inside the requested bbox, 1 credit spent** — containment is what proves OpenSky's
      **lon-before-lat** order, which no compiler can catch and which swapped would have put
      them in Somalia. Field indices are named constants for the same reason. Parsing is
      per-field tolerant, per-record fallible: `states` elements stay `Value` so one bad
      record cannot fail the batch; losing *every* record warns, since that is what a shape
      change looks like. `time_position`, not `last_contact` — the newer one would date a
      stale fix to now and M2's dead reckoning would advance it from a place it had left.
      Credit tiers round to the **dearer** band (under-pricing overruns the allowance rule 1.3
      caps; over-pricing costs a wider poll interval). A disabled source returns `Auth` rather
      than silently dropping to OpenSky's 400-credit anonymous tier. **Both 1.3 carry-overs
      closed**: `retry_after` now reads `X-Rate-Limit-Retry-After-Seconds` after the standard
      header (first *usable* hint wins), and `reqwest`'s `query` feature is on. **⚠ Known gap
      for M3**: `anonymous` catches only the no-callsign half of privacy 2.2 — a PIA hex that
      broadcasts a callsign needs FAA range data we do not have; the enrichment gate is where
      it binds. Rationale in DECISION_LOG.)*
- [x] 1.5 airplanes.live adapter: `/v2/point` query, readsb-JSON parsing (shared module),
      `"ground"` altitude handling, ≥ 2 s request spacing; fixtures.
      *(2026-07-17: done — `ingest::readsb` (the shared `{ac: [...]}` parser 1.6 reuses,
      parameterized by `SourceId`), `ingest::airplanes_live` (`AirplanesLiveSource`
      implementing `LiveSource`), `ingest::pacer` (≥ 2 s spacing in the adapter — the limit
      is the source's, not a scheduling choice), `ingest::normalize` (`coordinate`/`narrow`
      lifted from `opensky::states`). 37 new tests, 233 total; four fixtures + README per
      docs/10 §2. The traps and their answers: **units are feet/knots/ft-per-min → SI at the
      parse boundary** through named constants; **`alt_baro: "ground"` → `on_ground`, no
      altitude**; **ts = `now − seen_pos`** (1.4's time-of-applicability call), with `now`
      normalized ms-vs-s by magnitude; **`~`-hex TIS-B synthetics skipped**, never minted an
      identity. bbox → covering circle (midpoint, farthest corner, ceil, clamp 250 nm with
      warn), results filtered back to the bbox; global query → `Refused` (M4's problem);
      `cost()` = 0. **Verified live**: 48 aircraft over Switzerland, all inside the bbox,
      `ts` within the hour (`now` confirmed ms), altitudes/speeds in SI ranges (conversions
      confirmed), 0 credits, `#[ignore]`d. Rationale in DECISION_LOG.)*
- [x] 1.6 adsb.lol adapter reusing the readsb parsing module; fixtures.
      *(2026-07-17: done — `ingest::adsb_lol` (`AdsbLolSource`), and the second readsb
      fallback surfaced a bigger reuse than the parser: the *request* side (bbox → covering
      circle, 250 nm clamp, pacing, send, bbox-trim) is byte-identical between the two, so it
      was lifted into `ingest::point` (`PointSource`) and 1.5's `airplanes_live` refactored to
      delegate — the "adapter's own geometry problem" framing didn't survive the second
      adapter (rule of two). Each adapter is now only its host, `SourceId`, spacing, fixtures,
      and live test. adsb.lol has no documented rate limit, so it mirrors airplanes.live's
      conservative ≥ 2 s spacing rather than a looser guess (privacy 1.3: with no limit
      documented, the safe reading is the gentle one). Four own fixtures + README (identities
      deliberately distinct from airplanes.live's so a test can't pass off the wrong file).
      **Verified live**: 46 aircraft over Switzerland, all inside the bbox, `ts` within the
      hour, SI ranges — the same three beliefs (ms `now`, feet/knots, field names) pinned
      against adsb.lol independently, 0 credits, `#[ignore]`d. Net 242 tests (138 ingest),
      fmt/clippy/test green. Rationale in DECISION_LOG.)*
- [ ] 1.7 `ingest::budget`: daily credit ledger (persisted in `source_status`), pro-rated
      spend targets, cadence controller (poll interval widens as budget tightens; floor 5 s,
      ceiling 60 s).
- [ ] 1.8 Poller: drives the active source at the budgeted cadence for the current region;
      failover chain opensky → airplaneslive → adsblol on repeated `SourceError`s; recovery
      probe of the primary every 5 min; emits batches into `crossbeam` channel.
- [ ] 1.9 `core::merge`: dedup across sources (newest ts per icao24 wins), out-of-order drop,
      staleness tracking, **sticky anonymity** (privacy 2.2) — with the unit tests from docs/10.
- [ ] 1.10 `scripts/record_fixture.rs`: fetch → trim to ≤ 20 records → scrub → write to
      tests/fixtures/ (never prints payloads; docs/06 network rule).
- [ ] 1.11 `store`: migrations 0001 (aircraft, source_status) + writer thread skeleton;
      poller updates source_status (last_success/error, credits_used_today).
- [ ] 1.12 Headless mode: `look-above --headless` logs per-cycle counts (new/updated/stale,
      credits spent) — the M1 gate evidence tool.
- [ ] 1.13 Gate: 10-min supervised live run per acceptance §M1; record numbers; human review.

## Design notes

- The poller never knows about parsing; adapters never know about cadence. Budget decisions
  are unit-testable pure functions (`ledger + bbox + clock → next_poll_at`).
- Global-region polling (L0) is deferred to M4; M1 regions are bboxes ≤ ~1,000 km across.
- If both OpenSky credentials are absent *and* fallbacks are down, the app idles and retries;
  it never crashes and never widens its request behavior beyond documented limits.
