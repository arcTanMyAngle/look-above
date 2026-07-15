# M1 — Authorized Data Ingestion

**Goal:** live, normalized, rate-budgeted aircraft state flowing from authorized sources into
the pipeline, fully fixture-tested. Exit criteria: [../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md) §M1.
Constraining docs: 09 (contracts), 04 (rules 1.x, 2.2, 7.x), 10 (§1 rate budgeting, §2 fixtures),
and the [authorized-aviation-sources skill](../.claude/skills/authorized-aviation-sources/SKILL.md).

## Prerequisite (user action)

> **OpenSky account needed before item 1.3:** create a free account at
> https://opensky-network.org, then in your account settings create an **API client** to get
> a client id + secret. Put them in `config.toml` (`[opensky] client_id/client_secret`).
> Items 1.1–1.2 and the fallback path (1.5+) proceed without it.

## Checklist

- [ ] 1.1 `ingest::http`: shared reqwest client — 10 s timeouts, User-Agent per docs/09,
      backoff helper (exponential + jitter, honors Retry-After), `SourceError` mapping.
- [ ] 1.2 Allowlist const + test (docs/10 §privacy): permitted hosts only.
- [ ] 1.3 OpenSky auth: OAuth2 client-credentials token fetch + cache + refresh at 80% TTL;
      credentials from config; graceful "no credentials" state (source disabled, not error).
- [ ] 1.4 OpenSky adapter: `/states/all` bbox query → `Vec<StateVector>`; positional-array
      parsing tolerant of nulls per field; credit cost function (bbox area → 1–4);
      fixture set per docs/10 §2.
- [ ] 1.5 airplanes.live adapter: `/v2/point` query, readsb-JSON parsing (shared module),
      `"ground"` altitude handling, ≥ 2 s request spacing; fixtures.
- [ ] 1.6 adsb.lol adapter reusing the readsb parsing module; fixtures.
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
