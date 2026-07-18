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
- [x] 1.7 `ingest::budget`: daily credit ledger (persisted in `source_status`), pro-rated
      spend targets, cadence controller (poll interval widens as budget tightens; floor 5 s,
      ceiling 60 s).
      *(2026-07-17: done — `ingest::budget`: `CreditLedger` (per-UTC-day credit count that
      resets itself at the day boundary), the pure `poll_interval`/`can_afford`/
      `prorated_target`/`remaining_budget` functions, and `CreditLedger::decide` bundling them
      into a `BudgetDecision`. 25 tests, 267 total. **The seam decided first** (per
      CURRENT_STATUS): the ledger is a small owned struct, **in-memory for M1**, restored from
      `source_status.credits_used_today` at 1.11 via `CreditLedger::restored` — no reach into
      `store`, which does not exist yet. **The number defended is 3,200 = 80% of the 4,000/day
      allowance** (privacy rule 1.3's margin), not 4,000. **Cadence = even-spread of the
      *remaining* budget over the *remaining* seconds of the UTC day**, clamped to [5 s, 60 s]:
      this *is* the pro-rating — on the pro-rata line it gives the steady ~27 s/credit that just
      fills the day; under budget it shrinks toward the floor, over budget it widens toward the
      ceiling ("interval widens as budget tightens"). **Two separate protections**: the cadence
      (soft, within [5,60]) and `can_afford` (the hard stop that refuses the cycle crossing the
      cap — the ceiling alone can't bound a 4-credit query). **Wall-clock `UnixSeconds`, not the
      monotonic `Instant`** auth uses — the day boundary is a calendar fact. Pure functions
      only; the poller (1.8) drives them. Rationale in DECISION_LOG.)*
- [x] 1.8 Poller: drives the active source at the budgeted cadence for the current region;
      failover chain opensky → airplaneslive → adsblol on repeated `SourceError`s; recovery
      probe of the primary every 5 min; emits batches into `crossbeam` channel.
      *(2026-07-17: done — `ingest::poller`: `Poller` (the async loop), `PollBatch` (the
      channel payload, carrying per-cycle `credits_spent`/`spent_today` so 1.11/1.12 need not
      reach into the ledger), `WallClock`/`SystemWallClock` (the ledger's calendar clock,
      injected; the cadence sleeps + 5-min probe use tokio's monotonic clock). 18 new tests,
      284 total. **Failover branches on `is_transient` three ways**: transient
      (`RateLimited`/`Network`/`Server`) retries the same source with `http::backoff` and only
      fails over after `TRANSIENT_FAILOVER_THRESHOLD` (3) in a row; permanent-but-real
      (`Auth`/`Parse`/`Request`) fails over on the first (a disabled OpenSky returns `Auth` and
      drops straight to the keyless fallbacks); `Refused` (our own bug) **holds and idles** —
      the next source gets the same wrong question, so it is deliberately *not* a failover
      (error.rs already documents this). Chain advance wraps; the recovery probe is the
      separate faster path back to the primary. **Budget veto = skip, not failover**: a cycle
      `can_afford` refuses is not fetched and the poller idles at the ceiling until the UTC-day
      reset — a rationing primary is not a failed one (DECISION_LOG 1.8). **Verified live**
      (`#[ignore]`d, keyless, free): with OpenSky disabled the poller failed over and emitted a
      real fallback batch, 0 credits. Rationale in DECISION_LOG.)*
- [x] 1.9 `core::merge`: dedup across sources (newest ts per icao24 wins), out-of-order drop,
      staleness tracking, **sticky anonymity** (privacy 2.2) — with the unit tests from docs/10.
      *(2026-07-17: done — `core::merge`: `SessionTable` (one `StateVector` per `Icao24`, the
      freshest seen) and `MergeStats { new, updated, dropped }`. 20 tests, 304 total. **Dedup is
      strictly newest-`ts`-wins**; not-strictly-newer (out-of-order *or* equal-`ts` duplicate) is
      dropped — the same time-of-applicability reasoning as 1.4. **Sticky anonymity is a one-way
      latch honored independent of `ts`**: once any record marks a hex anonymous it stays so for
      the session and its callsign is pinned `None`, even against a newer identified record — and
      **the latch fires even for a record dropped as stale**, because an anonymity signal is a
      privacy fact, not a position (privacy 2.2 / 5.2). **Staleness tracked here, faded in M2**:
      `age`/`stale_count`/`evict_stale` with horizons `STALE_AFTER_S` = 60 s and `DROP_AFTER_S` =
      90 s pinned to the render skill's "begin fade" / "stop extrapolating" points; the visual
      fade stays render's job. `MergeStats` is the per-batch tally 1.12's new/updated/stale
      readout consumes. Clock-free for merging (dedup/stickiness test in isolation); only the
      staleness queries take a `now`. Rationale in DECISION_LOG.)*
- [x] 1.10 `scripts/record_fixture.rs`: fetch → trim to ≤ 20 records → scrub → write to
      tests/fixtures/ (never prints payloads; docs/06 network rule).
      *(2026-07-17: done — `scripts/record_fixture.rs`, wired as a `[[bin]]` of `ingest`
      (`path = "../../scripts/record_fixture.rs"`) so a recording goes out exactly as a poll
      would: it reuses the allowlist-enforcing `HttpClient`, the OpenSky `OAuth2` client, the
      source endpoint constants, and `point::MAX_RADIUS_NM` rather than reconstructing any. CLI
      speaks each source's native region shape (OpenSky bbox / readsb `point/{lat}/{lon}/{radius}`),
      which is what avoided a third copy of `point`'s covering-circle math — the recorded
      *response shape* is identical either way. Trims the record array to ≤ 20, then scrubs a
      denylist of credential/account-shaped keys (a tripwire — removes nothing from today's
      anonymous feeds), and writes pretty JSON to `crates/ingest/tests/fixtures/<source>/`,
      printing only a count and path (docs/06). OpenSky creds are env-only (`LOOK_ABOVE_OPENSKY_*`)
      — reaching `app`'s config loader would invert the crate direction. **Not a drop-in
      re-record**: the crafted `*_nominal` fixtures pin exact values the parser tests assert, and
      the `empty`/`nulls`/`malformed` cases stay hand-authored; the tool refreshes *shape*. 9
      offline tests (trim/scrub/naming/parse), plus the **live path exercised** (`adsblol 47 8 73`
      → 16 real aircraft, valid trimmed file, count-only output, checked structurally and deleted).
      313 tests, fmt/clippy/test green. `Box<dyn Error>` not `anyhow` (that stays in `app`). READMEs
      (root + three fixture) updated with the command and the re-record caveat. DECISION_LOG 1.10.)*
- [x] 1.11 `store`: migrations 0001 (aircraft, source_status) + writer thread skeleton;
      poller updates source_status (last_success/error, credits_used_today).
      *(2026-07-18: done — `crates/store`'s first real code. `migrations::apply` (numbered,
      `include_str!`-embedded SQL, `PRAGMA user_version`-tracked, idempotent-by-version) plus
      migration 0001, which creates **only** `aircraft` and `source_status` — the rest of
      docs/08's schema (`positions`/`flights`/`airports`/`runways`/`airlines`/`metars`) is each
      tagged with its own later milestone there, and migrations are append-only, so nothing
      landed ahead of the point it's used. `writer::Writer` is the single-writer-thread skeleton
      (docs/08): a cheap-to-clone channel handle over one `Command` enum
      (`RecordSuccess`/`RecordError`/`SourceStatus`) behind one `crossbeam` `Sender`, so a later
      item can add `positions`/`airports` commands without changing `Writer`'s shape. `Writer::open`
      runs migrations synchronously before spawning the thread, so a broken DB surfaces to the
      caller instead of silently killing an unwatched thread. **`core::contracts::Store` is
      deliberately not implemented yet** — `insert_positions`/`airports_in_bbox`/`prune` need
      tables that don't exist until M3/M5; `Writer`'s inherent API is scoped to exactly what
      migration 0001 backs. **Dependency direction held**: `store` depends on `core` only (verified
      against `Cargo.toml`, not `cargo tree` per CLAUDE.md) — no `look-above-ingest` edge, so
      `record_success`/`record_error` take plain `SourceId`/`UnixSeconds`/`u32`/`String`, never
      `ingest::poller::PollBatch`. `record_success` upserts only `last_success`/`credits_used_today`;
      `record_error` only `last_error`/`last_error_msg` — each verb owns its own columns, so an
      error after a success (or vice versa) never erases the other. `source_status`'s
      `credits_used_today` readback is exactly the `spent` argument `ingest::budget::CreditLedger::
      restored` takes; `store` carries no notion of UTC-day rollover itself since `restored`
      already discards a stale persisted day. Actually wiring the poller's channel into a running
      `Writer` inside `app` is out of scope here (no `PollBatch` consumer exists yet) — that's
      1.12 or later. 16 new tests (9 migrations + writer upsert/readback logic directly against a
      connection, 7 through the real channel/thread, one on-disk WAL smoke test confirming
      `journal_mode` is actually `wal` — `:memory:` can't be, so that's the one place it's
      checked). 329 tests (43 app, 71 core, 180 ingest, 9 `record_fixture` bin, 5 render, 16
      store), 5 live ignored; fmt/clippy/test green. Rationale in DECISION_LOG.)*
- [x] 1.12 Headless mode: `look-above --headless` logs per-cycle counts (new/updated/stale,
      credits spent) — the M1 gate evidence tool.
      *(2026-07-18: done — new `app::headless` and a `main.rs` CLI switch (`--headless`; any
      other argument is a hard error, the same "typo must not silently default" call
      `config` makes). This is the first item that runs M1's pieces together as one process:
      `Poller::with_default_chain` feeds a `crossbeam` channel, each `PollBatch` merges into a
      `core::merge::SessionTable`, and `store::Writer::record_success` persists the cycle
      against `source_status` — the wiring 1.11 left open on purpose. **Closes 1.7's ledger
      seam**: at startup, `Writer::source_status(OpenSky)`'s `credits_used_today` seeds the
      primary's ledger via `CreditLedger::restored`, so a restart mid-day resumes the day's
      spend instead of believing the budget is fresh again — verified live across two runs
      (spend carried 0→4→6 credits across a restart). Needed a new `Poller::restore_ledger`
      (ingest): `ledgers` is private, so nothing outside the module could seed it before this;
      a no-op on an out-of-range index rather than a panic. **The fixed region**: with no
      camera to drive `RegionQuery` yet (design note below), headless mode uses a constant
      ~530×555 km bbox centered on the Alps (44.5–49.5°N, 4.5–11.5°E) — sized to match
      acceptance §M1's "~500×500 km bbox" credit-budgeting line and landing OpenSky's area
      pricing in its middle (2-credit) tier rather than the cheapest or dearest, and it is the
      same airspace every adapter's own live test has flown since item 1.4. Per-cycle log
      carries `new`/`updated`/`dropped`/`stale`/`tracked`/`credits_spent`/`spent_today`/
      `source` — the checklist's "new/updated/stale, credits spent" plus the extra fields the
      dedup and credit-budget gate lines (acceptance §M1) need to be "observed in logs".
      **`record_error` is not wired**: the poller's channel only ever carries a successful
      `PollBatch` (errors are logged internally by the poller, per 1.8, and never reach the
      channel), so a consumer here has no error to hand `Writer::record_error` — extending the
      poller to surface failures over the channel is a real change, not this item's "smallest
      correct change" scope; carried forward, not an oversight. No graceful shutdown: the gate
      run (1.13) is operator-supervised and stopped with `Ctrl+C`, so adding a shutdown
      protocol here would be scope the checklist does not ask for. 5 new tests (3 CLI parsing,
      2 `restore_ledger`); 334 total, fmt/clippy/test green. **Verified live** (keyless-free
      fallback untouched — the owner's real `credentials.json` was configured, so this
      exercised the actual OpenSky OAuth2 path, not just the fallbacks): two short runs,
      `#[ignore]`-free because this is the binary itself, not a test — 249 aircraft on the
      first cycle, then 231 updated / 1 new / 18 dropped on the second (dedup visibly
      working), 2 credits/cycle, 6 of 3,200 spent total. `source_status` write confirmed by
      the *absence* of this module's own "could not record source_status" warning, which is
      what a failed write would have logged. Also found and fixed while wiring this: `Config::
      credentials()` was `#[allow(dead_code)]` since 1.3 with a comment saying the poller
      would reach it "in item 1.4" — it never did until now; the attribute and the stale
      comment are gone. DECISION_LOG 1.12.)*
- [ ] 1.13 Gate: 10-min supervised live run per acceptance §M1; record numbers; human review.

## Design notes

- The poller never knows about parsing; adapters never know about cadence. Budget decisions
  are unit-testable pure functions (`ledger + bbox + clock → next_poll_at`).
- Global-region polling (L0) is deferred to M4; M1 regions are bboxes ≤ ~1,000 km across.
- If both OpenSky credentials are absent *and* fallbacks are down, the app idles and retries;
  it never crashes and never widens its request behavior beyond documented limits.
