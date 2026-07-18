# Current Status

> The single source of truth for "where are we". Every session reads this first and updates
> it last. Keep the Now section ≤ 10 lines; move history to the log below.

## Now (updated 2026-07-18)

- **Phase:** **M1 open** (owner call, M0 gate at 6/7 — see below). Items 1.1–1.12 done; 334
  tests green (5 live `#[ignore]`d). Plan:
  [M1_AUTHORIZED_DATA_INGESTION.md](M1_AUTHORIZED_DATA_INGESTION.md)
- **Next action:** **M1 item 1.13**, the gate — a 10-min supervised live run of
  `look-above --headless` (1.12) per acceptance §M1; record the numbers; human review. Last
  M1 checklist item.
- **1.12 headless mode landed:** `app::headless` + `--headless` run the poller, merge, and
  store writer together as one process for the first time, closing 1.7's ledger-restore seam
  and 1.11's writer-wiring gap. Verified live against real OpenSky auth: dedup and credit
  spend both carried correctly across a restart. `record_error` stays unwired (the poller's
  channel never carries a failure) — carried forward, not an oversight. DECISION_LOG 1.12.
- **Blockers:** the owner must rename the repo `look_above` → `look-above`, then push (no SSH
  key on this machine) — CI has never run; M0's one unmet gate line.
  [NEXT_ACTIONS.md](NEXT_ACTIONS.md) #1.
- **Credit spend to date: 7 of 4,000/day** (1 from 1.4's live test, 6 from verifying 1.12's
  headless pipeline live). Every automatic test stays a mock; only these two items have spent.
- **⚠ Carried to M3:** `anonymous` catches only the no-callsign half of privacy rule 2.2 — a
  PIA hex broadcasting a callsign needs FAA range data not yet available. DECISION_LOG 1.4.

## Gate record

| Milestone | Status | Evidence |
|---|---|---|
| M0 | **gate run 2026-07-15 — 6/7; owner opened M1 with the badge line outstanding** | per-line below |
| M1 | in progress — 1.1–1.12 done | — |
| M2 | not started | — |
| M3–M6 | not started (plan files written at preceding gates) | — |

### M0 acceptance §M0 — evidence (run 2026-07-15, Windows 11, rustc 1.96.0, Intel Arc / Vulkan)

| # | Line | Result | Evidence |
|---|---|---|---|
| 1 | `cargo build --workspace` on a clean clone | **pass** | fresh `git clone` to a scratch dir, cold build: **exit 0 in 66.2 s**. Not the warm tree — a clone is the only thing that can catch a needed-but-uncommitted file. |
| 2 | CI fmt + clippy + tests on push; badge green | **BLOCKED** | no git remote (`git remote -v` empty); `github.com/arcTanMyAngle/look-above` → **HTTP 404** (fetched). Workflow has never executed. Its three commands are green locally on Windows; the Linux job is unproven. |
| 3 | Five crates, direction core ← {ingest, store, render} ← app, no reverse deps | **pass** | full intra-workspace edge list from `cargo metadata`: `ingest`/`store`/`render` → `core`; `app` → all four; **nothing else**. Nothing depends on `app`; the three middle crates don't depend on each other. `core` externals: async-trait, rayon, serde, thiserror (no tokio/reqwest/rusqlite). `render`: no winit, no network, no DB. |
| 4 | `cargo run -p look-above` opens a window, resizes without panic, closes cleanly | **pass** | driven over Win32: window titled "Look Above" (hwnd confirmed), resized 800×600 / 1280×720 / 640×480 / 1024×768, minimized to 0×0 and restored, all alive; `WM_CLOSE` → "close requested" → "window closed"; **exit code 0**; zero panics on stdout/stderr. |
| 5 | Config from `config.toml` + env override; missing file → defaults, not error | **pass** | against the **binary**, not the tests. No file → "no configuration file; using defaults", `look_above.db`, 24 h, credentials "absent". With a file → `from_file.db`, 6 h. With `LOOK_ABOVE_*` → `from_env.db`, 3 h. Env > file > default, observed each time. |
| 6 | `config.toml` gitignored; repo contains `config.example.toml` | **pass** | `git check-ignore -v` hits for `config.toml`, `target/`, `qa/`, `*.db`; `config.toml` untracked and **absent from the clone**; `config.example.toml` tracked and present. |
| 7 | ADRs 001–005 accepted; DECISION_LOG updated | **pass** | docs/02: all five marked `Status: accepted`. DECISION_LOG has a dated entry per item 0.1–0.8. |

Suite at the gate: **87 tests** (51 core, 31 app, 5 render), `fmt`/`clippy --all-targets -D warnings`/`test` all green. No code changed at 0.8; working tree clean afterwards.

## Session log (newest first)

- **2026-07-18** — M1 item 1.12: headless mode. New `app::headless` (`headless::run`) plus a
  `--headless` CLI switch in `main.rs` (`parse_args`/`parse_args_from`; any other argument is a
  hard error, matching `config`'s "a typo must not silently default" call). This is the first
  item that runs M1's pieces together as one live process rather than in isolation under a
  test: `Poller::with_default_chain` feeds a `crossbeam` channel; each `PollBatch` merges into
  a `core::merge::SessionTable`; `store::Writer::record_success` persists the cycle against
  `source_status` — the wiring 1.11 deliberately left open. **Closes 1.7's ledger-restore
  seam**: at startup, `Writer::source_status(OpenSky)`'s `credits_used_today` seeds the
  primary's ledger through `CreditLedger::restored`, so a restart mid-day resumes the day's
  spend rather than believing the budget is fresh — needed a new `Poller::restore_ledger`
  (`ledgers` is private, so nothing outside `ingest` could seed it before this; a no-op on an
  out-of-range index rather than a panic). **The fixed region**: with no camera to drive
  `RegionQuery` yet, headless mode polls a constant ~530×555 km bbox over the Alps
  (44.5–49.5°N, 4.5–11.5°E) — sized to match acceptance §M1's "~500×500 km bbox" credit-budget
  line and landing OpenSky's area pricing in its middle (2-credit) tier, the same airspace
  every adapter's own live test has flown since item 1.4. Per-cycle log carries
  `new`/`updated`/`dropped`/`stale`/`tracked`/`credits_spent`/`spent_today`/`source` — the
  checklist's "new/updated/stale, credits spent" plus what acceptance §M1's dedup and
  credit-budget lines need "observed in logs". **`record_error` is not wired**: the poller's
  channel only ever carries a successful `PollBatch` (1.8: failures are logged internally and
  never reach the channel), so there is no error here to hand `Writer::record_error` —
  extending the poller to surface failures over the channel is a real change, not this item's
  smallest-correct-change scope; carried forward. No graceful shutdown: the gate run (1.13) is
  operator-supervised and stopped with `Ctrl+C`, so a shutdown protocol would be scope the
  checklist does not ask for. 5 new tests (3 CLI parsing, 2 `restore_ledger` on the poller
  side); 334 total, fmt/clippy/test green. **Verified live** against the owner's real
  `credentials.json` (the actual OpenSky OAuth2 path, not just the keyless fallbacks): two
  short runs of the binary itself — 249 aircraft on the first cycle, then 231 updated / 1 new
  / 18 dropped on the second (dedup visibly correct), 2 credits/cycle, 6 of 3,200 spent; the
  second run's startup line read `credits_used_today=4`, confirming the restore round-tripped
  through a real restart. `source_status` writes confirmed by the *absence* of this module's
  own "could not record source_status" warning — what a failed write would have logged.
  Along the way: `Config::credentials()` had carried `#[allow(dead_code)]` and a stale "the
  poller reaches this in item 1.4" comment since 1.3; both are gone now that `headless::run`
  is the real caller. DECISION_LOG 1.12. Next: **1.13**, the M1 gate.

- **2026-07-18** — M1 item 1.11: `store` migrations + writer-thread skeleton. New
  `crates/store` code (the crate's first): `migrations::apply` (numbered, `include_str!`-embedded
  SQL, `PRAGMA user_version`-tracked, idempotent-by-version — each migration's DDL and version
  bump commit together in one `BEGIN IMMEDIATE … COMMIT`) plus migration 0001, which creates
  **only** `aircraft` and `source_status` — verbatim from docs/08, whose other tables
  (`positions`/`flights`/`airports`/`runways`/`airlines`/`metars`) are each tagged with a later
  milestone there and land as their own append-only migrations when needed, not ahead of time.
  `writer::Writer` is the single-writer-thread skeleton docs/08 calls for: a cheap-to-clone
  channel handle over one `Command` enum (`RecordSuccess`/`RecordError`/`SourceStatus`, each with
  its own one-shot reply channel) behind one `crossbeam` `Sender`, with a dedicated OS thread
  owning the sole `rusqlite::Connection` and draining commands until every `Writer` clone is
  dropped. `Writer::open` runs migrations synchronously before spawning the thread, so a broken
  database surfaces as an `Err` to the caller instead of silently killing an unwatched thread.
  **`core::contracts::Store` is deliberately not implemented yet**: its `insert_positions`/
  `airports_in_bbox`/`prune` each need a table (`positions`/`airports`) that doesn't exist until
  M3/M5 migrations land, so implementing the trait now would mean methods that can't work —
  `Writer`'s inherent API is scoped to exactly what 0001 backs, and wiring `Store` for real is a
  future item, noted so it isn't mistaken for an oversight. **Dependency direction verified from
  `Cargo.toml` directly** (not `cargo tree`, per CLAUDE.md): `store` depends on `core` only, so
  `record_success`/`record_error` take plain `SourceId`/`UnixSeconds`/`u32`/`String` — never
  `ingest::poller::PollBatch` — and `source_status` returns a `store`-local `SourceStatus`, never
  `ingest::budget::CreditLedger`. That readback's `credits_used_today` is exactly the `spent`
  argument `CreditLedger::restored(spent, now)` (item 1.7) takes; `restored` already discards a
  stale persisted day on its own, so `store` carries no UTC-day-rollover logic at all — the actual
  restore call is `ingest`/`app` wiring, still to come. **Each verb owns exactly its own
  columns**: `record_success` upserts only `last_success`/`credits_used_today`, `record_error`
  only `last_error`/`last_error_msg`, so a success after a prior error (or vice versa) never
  erases the other — proven both directions. `source` is the table's primary key, so a repeat
  write for one source overwrites rather than duplicating (tested). **Wiring an actual running
  `Writer` from the poller's channel inside `app` is out of scope here** — `app` doesn't consume
  `PollBatch` yet; that starts at 1.12. **The on-disk WAL smoke test is the one place WAL is
  actually checked**: `SQLite`'s `:memory:` can't use WAL at all, so `open_connection` requests it
  unconditionally without asserting it took in the in-memory tests; a dedicated on-disk test
  (temp file, `Drop`-guard cleanup that also removes `-wal`/`-shm`/`-journal` siblings) confirms
  `journal_mode` reads back `wal` for real. Work was delegated to the `storage-agent`; this
  session independently re-ran `cargo fmt --check`/`clippy --workspace --all-targets -D
  warnings`/`test --workspace` rather than taking the agent's word, and read every new file.
  16 new tests (4 on the migration runner + 1 trust-`user_version`-not-a-table-probe edge case, 6
  on the upsert semantics against a raw connection, 5 through the real channel/thread, 1 on-disk
  WAL smoke test). 329 tests total (43 app, 71 core, 180 ingest, 9 `record_fixture` bin, 5
  render, 16 store), 5 live ignored; fmt/clippy/test green. DECISION_LOG 1.11. Next: **1.12**,
  headless mode (`--headless` per-cycle counts — the M1 gate evidence tool, and the first item
  that actually wires a running poller loop to a live `Writer`).

- **2026-07-17** — M1 item 1.10: the fixture recorder. New `scripts/record_fixture.rs`, wired
  as a `[[bin]]` of `ingest` from the repo-root `scripts/` the docs name (out-of-package
  `path`, which Cargo accepts — probed first). It is the recorder docs/06 sanctions and the
  fixture READMEs have promised since 1.4: fetch from an authorized source → trim the record
  array to ≤ 20 → credential-scrub → write to `crates/ingest/tests/fixtures/<source>/`, printing
  only a count and path, **never the payload**. A bin of `ingest` (not a standalone crate)
  because a recording must go out exactly as a poll would — it reuses the allowlist-enforcing
  `HttpClient`, the OpenSky `OAuth2` client, `STATES_ENDPOINT`/the two `POINT_ENDPOINT`s, and
  `point::MAX_RADIUS_NM`. CLI speaks each source's native region shape (OpenSky bbox / readsb
  point+radius), which is what let it avoid a *third* copy of `point`'s covering-circle math —
  the recorded response *shape* is identical either way. OpenSky creds are env-only
  (`LOOK_ABOVE_OPENSKY_*`): reaching `app`'s `config.toml`/`credentials.json` loader would invert
  the crate direction. Scrub is a tripwire (denylist of account-shaped keys) that removes nothing
  from today's anonymous feeds but keeps the tool safe without reading the payload. **Not a
  drop-in re-record**: the crafted `*_nominal` fixtures pin exact values the parser tests assert,
  and `empty`/`nulls`/`malformed` stay hand-authored — the tool refreshes shape and resets after
  a documented source change. `Box<dyn Error>`, not `anyhow` (that stays in `app`). 9 offline
  unit tests (trim/scrub/naming/parse-order), and the **live path exercised** — `adsblol 47 8 73`
  fetched 16 real aircraft over Switzerland, wrote a valid trimmed `{ac, now, …}` file, printed
  only the count; checked structurally (never printing values) and deleted. 313 tests (the 9 in
  the new bin), fmt/clippy/test green. Root README's stale "51 tests / no API client" section and
  all three fixture READMEs updated. DECISION_LOG 1.10. Next: **1.11**, `store` migrations +
  writer thread.

- **2026-07-17** — M1 item 1.9: the cross-source merge. New `core::merge`: `SessionTable` (the
  session's deduplicated live picture — one `StateVector` per `Icao24`, the freshest seen) and
  `MergeStats { new, updated, dropped }`. 20 tests, 304 total (71 core, 180 ingest, 43 app, 5
  render); fmt/clippy/test green. **Dedup is strictly newest-`ts`-wins**: a record replaces the
  held one only when `incoming.ts > stored.ts`; anything not strictly newer — an out-of-order
  late arrival *or* an equal-`ts` duplicate from a second source — is dropped, the same
  time-of-applicability reasoning as 1.4's `time_position` (a slower feed must not drag an
  aircraft back to an older fix). **Sticky anonymity is a one-way latch honored independent of
  `ts`** (privacy 2.2): once any record marks a hex anonymous it stays so for the session with
  `callsign` pinned `None`, even against a *newer, identified* record — and the subtle call is
  that **the latch fires even on a record dropped as stale**, because an anonymity signal is a
  privacy fact, not a position. Insertion enforces the same invariant defensively rather than
  trusting an adapter. **Staleness is tracked here but faded in M2**: `age(now)` (signed, so a
  source clock ahead of us reads negative rather than underflowing), `stale_count(now, max_age)`,
  and `evict_stale(now, max_age)`, with named horizons `STALE_AFTER_S`=60 s and `DROP_AFTER_S`=90
  s pinned to the render skill's "begin fade" / "stop extrapolating" points — the visual fade
  stays render's job. `MergeStats` is exactly the per-batch tally 1.12's new/updated/stale
  readout consumes. Clock-free for merging (dedup/stickiness test in isolation); only staleness
  queries take a `now`. DECISION_LOG 1.9. Next: **1.10**, `scripts/record_fixture.rs`.

- **2026-07-17** — M1 item 1.8: the poller. New `ingest::poller`: `Poller` (the async poll
  loop), `PollBatch` (the `crossbeam` payload — source, states, and the cycle's own
  `credits_spent`/`spent_today` so 1.11/1.12 read cost off the channel, not the private
  ledger), and `WallClock`/`SystemWallClock` (the ledger's *calendar* clock, injected; the
  cadence sleeps + the 5-min probe use tokio's *monotonic* clock — the two-clock split `budget`
  already argued). 18 tests, 284 total. **Failover branches on `is_transient` three ways**
  (a pure, unit-tested `error_response`): transient (`RateLimited`/`Network`/`Server`) retries
  the *same* source with `http::backoff`, failing over only after **3 in a row** (one hiccup
  isn't a dead source); permanent-but-real (`Auth`/`Parse`/`Request`) fails over on the
  *first* (a disabled OpenSky returns `Auth` with no network call and drops straight to the
  keyless fallbacks); **`Refused` holds and idles — never a failover**, because it is *our*
  bug and the next source gets the same wrong question (error.rs already says so). Chain
  advance wraps; the **5-min recovery probe of the primary is the separate, faster path back**.
  **Budget veto = skip, not failover**: a cycle `can_afford` refuses is not fetched (proven by
  an `Arc`-shared scripted source whose `fetch` is asserted *never called*) and the poller
  idles at the ceiling until the UTC-day reset — a rationing primary is not a failed one
  (candidate M4+ improvement noted: serve from free fallbacks when the primary is budget-capped).
  The loop never panics on a wild clock and never crashes on a dead chain (idles + retries);
  only a dropped receiver stops `run`. **Verified live** (`#[ignore]`d, keyless, free): OpenSky
  disabled → failed over → real fallback batch, 0 credits. Next: **1.9**, `core::merge`.

- **2026-07-17** — M1 item 1.7: the credit ledger + cadence controller. New `ingest::budget`:
  `CreditLedger` (an in-memory per-UTC-day credit count that resets itself at the day
  boundary), the pure `poll_interval` / `can_afford` / `prorated_target` / `remaining_budget`
  functions, and `CreditLedger::decide` that bundles them into a `BudgetDecision`. 25 tests,
  267 total; fmt/clippy/test green. **The seam was the first call** (CURRENT_STATUS flagged it):
  the ledger is a small **owned struct, in memory now**, rehydrated from
  `source_status.credits_used_today` at 1.11 via `CreditLedger::restored` — no reach into
  `store`, which does not exist yet. **The number defended is 3,200 = 80% of OpenSky's
  4,000/day** (privacy rule 1.3's margin), never 4,000. **The cadence is even-spread of the
  *remaining* budget over the *remaining* seconds of the UTC day, clamped [5 s, 60 s] — and
  that *is* the pro-rating**: on the pro-rata line it gives the steady ~27 s/credit that just
  fills the day, under budget it shrinks toward the floor, over budget it widens toward the
  ceiling, which is exactly "interval widens as budget tightens". Rejected floor-by-default:
  the 5 s floor at cost 1 is ~5× the daily budget, so it must be the exception (banked budget
  late in the day), not the norm. **Two protections kept separate**: the soft cadence (bounded
  [5,60]) and the hard `can_afford` cap — the ceiling alone can't bound a 4-credit query, so
  the cap is what guarantees rule 1.3, and an exhausted budget idles at the ceiling until the
  midnight reset. **Wall-clock `UnixSeconds`, not the monotonic `Instant`** auth uses: the day
  boundary is a calendar fact, and a clock correction that shifts the day *should* reset the
  ledger. All pure functions — the poller (1.8) drives them. Next: **1.8**, the poller +
  failover chain.

- **2026-07-17** — M1 item 1.6: the adsb.lol adapter. New `ingest::adsb_lol`
  (`AdsbLolSource`), plus `ingest::point` (`PointSource`) — because the second readsb fallback
  showed the shared thing is bigger than 1.5 thought: not just the parser but the whole
  *request* path (bbox → covering circle, 250 nm clamp, pacing, send, bbox-trim), byte-for-byte
  identical between the two services. It moved into `point`, and 1.5's `airplanes_live` was
  refactored to delegate; each adapter is now only its host, `SourceId`, spacing, fixtures, and
  live test. **The design call worth knowing** (DECISION_LOG 1.6): 1.5 wrote the geometry as
  "the adapter's own problem", and that framing did not survive the second adapter — rule of
  two, and two copies of ~65 lines + their tests would fight the same ethos that made
  `readsb`/`normalize`/`pacer` shared. **adsb.lol's spacing mirrors airplanes.live's ≥ 2 s
  though no limit is documented**: privacy 1.3 is "never exceed documented limits", so with
  none published the safe reading is the gentle one, not licence to go faster. Four own fixtures
  + README with identities deliberately distinct from airplanes.live's, so a test can't pass off
  the wrong file. Geometry/URL/trim/global-`Refused` are proven once in `point::tests`; each
  adapter keeps only its own end-to-end/error-mapping/allowlist/live tests. **Verified live**:
  46 aircraft over Switzerland, all inside the bbox, `ts` within the hour, SI ranges — the same
  three beliefs (ms `now`, feet/knots, field names) pinned against adsb.lol *independently*, 0
  credits, `#[ignore]`d. 242 tests (56 core, 138 ingest, 43 app, 5 render); fmt/clippy/test
  green. docs/09's adsb.lol entry gained the shared-`point`/spacing/live-verified detail. Next:
  **1.7**, the credit ledger + cadence controller — which first needs the `store`-vs-now seam
  decided, since `source_status` lands at 1.11.

- **2026-07-17** — M1 item 1.5: the airplanes.live adapter. Four new modules:
  `ingest::readsb` (the shared `{ac: [...]}` parser, parameterized by `SourceId` so 1.6
  drops in), `ingest::airplanes_live` (`AirplanesLiveSource`), `ingest::pacer` (≥ 2 s
  spacing), `ingest::normalize` (`coordinate`/`narrow` lifted out of `opensky::states`).
  37 new tests, 233 total; fmt/clippy/test green. **The headline risk was units, and it is
  the first adapter where that is true**: readsb sends feet/knots/ft-per-min where OpenSky
  sent SI, and a missed conversion produces plausible-looking numbers in the wrong unit —
  so conversion happens at the parse boundary through named constants, and the live test
  asserts ranges an unconverted value cannot pass. **Verified live, keyless, free**: 48
  aircraft over Switzerland (a 73 nm circle around 47°N 8°E), every one inside the
  requested bbox, every `ts` within the hour — which pins the other belief at risk, that
  the API's `now` is epoch *milliseconds* (raw readsb uses seconds; the parser normalizes
  by magnitude). Judgement calls, all in DECISION_LOG: **ts = `now − seen_pos`** (1.4's
  time-of-applicability reasoning); **`~`-hex TIS-B synthetics are skipped**, never minted
  an identity (0.3's `Icao24` strictness paying off); **bbox → covering circle** (midpoint
  center, farthest of the four corners — the sphere makes them unequal — ceil'd, clamped to
  the documented 250 nm with a warn) and **results filtered back to the bbox** so every
  source answers the same question for 1.9's merge; **a global query is `Refused`** rather
  than approximated (M4's problem); **`cost()` = 0** — what this source meters is rate,
  paid in time by the pacer, which lives in the *adapter* because the limit is the
  source's, not a scheduling choice. Pacing is proven under tokio's paused clock
  (`test-util`, dev-only); deliberately not re-proven over wiremock, where the
  auto-advancing clock can fire the 10 s timeout mid-reply. docs/09 and the skill gained
  the units/`seen_pos`/`~`-hex detail — the contract summary had field names but not
  units, and units are the trap. Next: **1.6**, the adsb.lol adapter over the same parser.

- **2026-07-15** — M1 item 1.4: the OpenSky `/states/all` adapter. `ingest::opensky::states` —
  `OpenSkySource` (implements `LiveSource`), positional-array parsing, `credit_cost`. 35 new
  tests, 196 total; fmt/clippy/test green. **The project made its first live *data* request,
  and it is the headline**: every fixture here is hand-written to OpenSky's documented shape,
  so the mocks prove only that we parse what we *believe* they send — and the belief is the
  risky part, because **OpenSky sends lon before lat**, backwards from every other source and
  invisible to the compiler. An `#[ignore]`d live test fetched **72 real aircraft over
  Switzerland and asserted every one falls inside the requested bbox** (swapped, they would be
  near 8°N 47°E — Somalia — and every one would have failed). 20 on the ground, **1 credit of
  4,000** spent, `#[ignore]`d so CI never repeats it. It also asserts *someone* has a callsign
  and *someone* a velocity: reading the wrong indices would otherwise call every optional field
  absent and pass. Field indices are named constants for the same reason. Parsing is per-field
  tolerant, per-record fallible — `states` elements stay `Value` so one non-array record cannot
  fail the batch (docs/10 §2), and losing *every* record logs a **warn**, since that is exactly
  what a shape change looks like and an empty sky does not explain itself. Four judgement calls
  worth knowing, all in DECISION_LOG: **`time_position`, not `last_contact`** (the newer one
  dates a stale fix to now, and M2's dead reckoning would then advance an aircraft from a place
  it had already left); **credit tiers round to the dearer band** (under-pricing overruns the
  allowance rule 1.3 caps, over-pricing only widens the poll interval); **a disabled source
  returns `Auth`** rather than silently dropping to OpenSky's 400-credit anonymous tier, which
  would turn a missing credential into a tenth of the budget with no clue why; and **a global
  query sends no bbox params**, since the endpoint's default *is* the world. **Both of 1.3's
  carry-overs are closed**: `retry_after` now reads a list — standard header, then
  `X-Rate-Limit-Retry-After-Seconds` — taking the first *usable* hint so a bad standard header
  cannot shadow a good vendor one; and `reqwest`'s `query` feature is on. **One gap found and
  carried to M3**: `anonymous` catches only the no-callsign half of privacy 2.2 — a PIA hex
  broadcasting a callsign needs FAA range data we do not have, and the enrichment gate is where
  it binds. Next: **1.5**, the airplanes.live adapter.

- **2026-07-15** — M1 item 1.3: OpenSky OAuth2. `ingest::opensky::auth` — `OpenSkyAuth`
  (token fetch, cache, refresh at 80% TTL, `Ok(None)` when disabled), `Credentials`, an
  injected `Clock`. 35 new tests, 161 total; fmt/clippy/test green. **The project made its
  first live API call**, and it is the headline: every other test here is a mock, which proves
  only that we parse what we *believe* OpenSky sends. An `#[ignore]`d live test proves the
  belief — the real endpoint **accepted the owner's credentials, TTL 1798 s, refresh scheduled
  at 1438 s = 79.98%**, confirming the documented ~30 min and validating the whole schedule
  against reality rather than against my own fixture. It costs no credits (the ledger meters
  `/states/*`, not the token endpoint) and stays `#[ignore]`d so CI never runs it. **The owner
  supplied `credentials.json`** rather than transcribing into `config.toml`; it is gitignored
  (checked untracked and absent from history *before* anything else — nothing leaked) and read
  as-issued, at a new precedence rung below `config.toml`. That file is **all-or-nothing**: if
  either half is configured elsewhere it is ignored entirely, because the two values are issued
  as a pair and mixing halves builds a credential that authenticates as nobody — a 401 that
  neither file explains. **`SecretString` moved to `core::secret`**: `ingest` must hold
  credentials and cannot depend on `app`, and the alternative was privacy rule 7.1 implemented
  twice. **`HttpClient::post_form` is new** — 1.1/1.2 gated `get` only, and the grant is a POST
  carrying the secret, so a bare client for it would have routed the credential straight around
  the allowlist. The 80% refresh is a **retry window, not just a deadline**: a failed refresh
  reuses the still-valid token with a warning, since refreshing early and then hard-failing buys
  nothing over refreshing at 100%. 1.2's tripwire **armed exactly as predicted** and was
  exercised, not assumed: a `flightradar24.com` host planted in `TOKEN_ENDPOINT` failed the scan
  with file, host and remedy named, then reverted. Two things handed to 1.4, both in
  DECISION_LOG: OpenSky's 429 carries **`X-Rate-Limit-Retry-After-Seconds`**, not the standard
  header 1.1 reads, so the backoff floor misses their hint; and reqwest 0.13 keeps **`query`
  behind a feature** (as it did `form`, added here) that the bbox params will need.
  Next: **1.4**, the `/states/all` adapter.

- **2026-07-15** — M1 item 1.2: the host allowlist. `ingest::allowlist` — `AUTHORIZED_HOSTS`
  (the skill's six runtime hosts), `is_authorized_host`, and `HostPolicy`. 19 new tests, 126
  total; fmt/clippy/test green. The item's real decision was that docs/10's spec for it —
  "a const list; test walks all adapter base URLs and asserts membership" — is weaker than it
  reads: there are no adapters until 1.3, so it would **pass over an empty set today**, and it
  could only ever see base URLs an adapter *declared*, not a URL built at a call site. So the
  list is enforced, not merely checked: `HttpClient::get` (1.1's choke point, which every
  adapter must pass through) checks the parsed `Url`, and **so does every redirect hop** —
  reqwest follows 10 by default, so a gate on the outbound URL alone is one `Location` header
  away from irrelevant. Matching is exact, never suffix (`ends_with("opensky-network.org")`
  welcomes `evil-opensky-network.org`; eight such lookalikes are pinned), and **https is part
  of the gate** — an `http://` typo on the token endpoint would send the OAuth2 secret in
  cleartext. **`SourceError::Refused` is new in `core`**, the second extension of docs/09's
  taxonomy after 1.1: `Network` is transient, so a refusal mapped there would retry an
  unauthorized host forever. Static-download hosts (OurAirports, FAA, Natural Earth) are
  deliberately *off* the list — import tooling, not this crate, and `raw.githubusercontent.com`
  serves anyone's repo. The test escape hatch is `#[cfg(test)]`, **not** a cargo feature, since
  feature unification could switch a privacy gate off in a shipped binary. Verified the way 0.8
  did: a `flightradar24.com` const planted in `http.rs` **failed** the scan test with file, host
  and remedy named, then reverted — a tripwire nobody has seen trip is a decoration. It is a
  tripwire, though: the crate has no request URL yet, so it arms itself at 1.3 (hence the
  extractor's own unit test and an assert that the walk visited ≥ 1 file). Two calls to revisit
  if they chafe: a blocked redirect surfaces as the 3xx status mapped to `Refused` (reqwest's
  policy API offers only follow/stop), and `Retry-After`-style HTTP-date parsing stays out.
  Next: **1.3**, which needs the OpenSky account — or 1.5–1.6 without it.

- **2026-07-15** — Repo identity settled. The owner supplied
  `git@github.com:arcTanMyAngle/look_above.git` — an **underscore**, where every doc says
  hyphen. Probed both: `look_above` exists (HTTP 200), `look-above` 404s. That gap is the
  User-Agent we send every aviation source (docs/09), so the URL a source operator would
  follow to identify us points at nothing. Owner chose to **rename the repo to `look-above`**
  over editing the identity in five files — the hyphen already matches the crate names, so a
  rename fixes it with zero code change. `origin` set to the hyphenated URL; **the rename must
  land before the first push**. The push is the owner's: no SSH key exists here
  (`Permission denied (publickey)`), and generating one was declined. Also flagged: the repo
  is **public** while inception recorded "private by default" — nothing sensitive is exposed,
  but the record and reality disagree (NEXT_ACTIONS #1).

- **2026-07-15** — **M1 opened at the owner's direction** with the M0 gate still at 6/7 (the
  badge line needs a push that hasn't happened; nothing about the blocker changed). Then M1
  item 1.1: `ingest::http` — the shared reqwest client (User-Agent + 10 s timeout per docs/09),
  `send_json`, the `SourceError` mapping, and `ingest::http::backoff` (pure `retry_delay`,
  base 5 s → cap 5 min). 20 new tests, 107 total; fmt/clippy/test green. Three calls worth
  knowing about, all in DECISION_LOG. **`SourceError::Request { status }` is new in `core`** —
  docs/09's taxonomy had no non-retryable home for a 400/404, so every existing variant either
  retried a permanent failure forever or swallowed it silently; this extends a doc rather than
  following one. **`Retry-After` is treated as a floor**, `max(header, backoff)`, and honored
  in full even past the 5-min cap — the header means "not before", so waiting longer honors it
  while honoring it *exactly* would drop escalation on repeated 429s. **Equal jitter, not full
  jitter**, because full jitter can retry milliseconds after a 429. Testing followed 0.8's
  habit of checking the artifact, not a proxy: wiremock (docs/10 §2 already required it, pulled
  in early) proves the User-Agent and the timeout on the wire rather than asserting constants
  against themselves. The privacy test caught its own flake before CI could — dropping a
  `MockServer` for a connection failure let a parallel test bind the freed port and answer 404;
  it targets `127.0.0.1:1` now. New deps: `fastrand` (jitter; `rand`'s defaults drag in a
  CSPRNG to smear a retry), `wiremock` (dev). Next: **1.2**, the host allowlist.

- **2026-07-15** — M0 item 0.8: the gate. Ran acceptance §M0 — **6 of 7 lines met**, per-line
  evidence in the table above; no code changed. The gate is recorded as *run*, not passed: the
  badge line needs a remote that doesn't exist (the repo 404s, verified rather than assumed),
  and a gate that certifies its own unverifiable line is worth nothing. Everything else was
  checked against the real artifact rather than a proxy — a fresh clone for the cold build
  (the warm tree cannot prove that line), the running binary for config precedence (the 31 app
  tests already assert the rules; the question was whether the shipped binary obeys them), and
  the live window over Win32 for resize/close (exit 0). Dependency direction came from
  `cargo metadata` edges instead of eyeballing `cargo tree`, which is precisely where a reverse
  edge would hide: the whole graph is seven lines and has none. Two scripting breadcrumbs for
  M2's visual QA, logged in DECISION_LOG: `FindWindow` returns 0 against this app from a
  non-interactive host though the window is real and correctly titled (use `Get-Process` →
  `MainWindowHandle`; this produced one false "no window" scare), and `cargo run` makes the app
  a child, so exit codes must come from a foreground `$LASTEXITCODE`. Next: **human review**;
  M1 does not start until the gate closes.

- **2026-07-15** — M0 item 0.7: CI. `.github/workflows/ci.yml` — one job per OS
  (windows-latest + ubuntu-latest, `fail-fast: false`), each running fmt → clippy → test, plus
  the README badge. The item's real decision was that CI must run *exactly* what CLAUDE.md
  tells a human to run: the two had drifted (0.6 verified with `--all-targets`, the doc didn't
  say so), and CI stricter than the documented check means green locally, red in CI, for
  someone who followed the docs — so `--all-targets` went into both, verified green first.
  Toolchain comes from `rust-toolchain.toml` via a bare `rustup toolchain install` rather than
  a setup action, so the pinned version lives in exactly one place (confirmed against local
  rustup 1.29.0). No apt step on Linux: winit defaults to `wayland-dlopen`, x11/xkbcommon load
  via `dlopen`, and x11-dl's build.rs treats a missing pkg-config entry as `None` — read, not
  assumed — so nothing links a system windowing lib at build time. That also settles the
  "watch at 0.7" note: no test opens a window or requests an adapter, so the GPU-less runner is
  a non-issue. `Swatinem/rust-cache@v2` is the one third-party action (bare `actions/cache` on
  `target/` is the problem it exists to solve); pinned by tag, not SHA — noted in DECISION_LOG
  as a choice. No Rust code changed; 87 tests, fmt/clippy/test green. **The workflow has never
  executed** — there is no remote (see Blockers). Next: 0.8, the gate.

- **2026-07-15** — M0 item 0.6: the window. `render::Renderer` (instance/surface/device +
  background clear) and `app::window` (winit `ApplicationHandler`) and `app::frame_stats`.
  The crate seam is a wgpu trait, not a winit type: `Renderer::new` takes
  `Arc<W: DisplayAndWindowHandle>`, so `render` never depends on winit and the surface can be
  `'static`. `render` stays sync per ADR-005, which is what `pollster` (new dep) buys — wgpu's
  setup calls are async but resolve without yielding on native. Background `#0A0E14` is
  linearized before use: `wgpu::Color` is linear and the surface is `Bgra8UnormSrgb`, so
  passing encoded values through would land near `#3A4351` — a washed-out grey that still
  looks "dark" in a screenshot and quietly breaks the contrast the altitude ramp assumes.
  Transient surface states (`Timeout`/`Occluded`/`Outdated`) are `Skipped`, not errors, and a
  0×0 (minimized) window is never configured — otherwise minimizing kills the app. Four wgpu
  30 API changes vs. every tutorial online (`CurrentSurfaceTexture` enum, `Queue::present`,
  no `InstanceDescriptor::default`, `multiview_mask`) were found by reading the vendored
  source; ADR-003 predicted exactly this churn. 87 tests; fmt/clippy/test green. The window
  has no unit test (needs a real GPU + event loop), so it was exercised by driving the live
  window over Win32: opened titled "Look Above" on Intel Arc/Vulkan, four resizes, minimize
  (0×0) + restore, `WM_CLOSE` → exit 0, stderr empty; a `PrintWindow` capture reads exactly
  `#0A0E14`, confirming the linearization rather than assuming it. Frame stats log at `debug`
  (a line/second would bury the startup lines at the default filter). Next: 0.7.

- **2026-07-15** — M0 item 0.5: `app::config` + `app::logging` — `config.toml` → serde struct,
  `LOOK_ABOVE_*` overrides, tracing init, `config.example.toml`. Precedence env > file >
  default. The item's real decision was the one the plan didn't answer: acceptance §M0 excuses
  a *missing* file, not a broken one, so absence → defaults but a present-but-unparseable file
  (or an unknown key, or retention past the 7-day cap) is a hard error — silent defaults hide
  a typo, and the app then looks fine while running unauthenticated or keeping the wrong
  history. Credentials are a redacted-`Debug` `SecretString` (rule 7.1) and the startup line
  logs only `configured|absent`. Env injected via an `EnvSource` trait because `set_var` is
  `unsafe` in edition 2024. No new deps (`toml` was pinned in 0.2 for exactly this; a small
  `TempDir` avoids `tempfile`). `.gitignore` already covered all four paths — verified, not
  recreated. 26 app tests, 77 workspace; fmt/clippy/test green. Binary exercised beyond the
  tests: no file → defaults, env beats file, broken file → exit 1 with line/column.
  Self-audit caught the environment path violating the very rule the file path enforced:
  `std::env::var(..).ok()` flattens "unset" and "set to non-Unicode" into one `None`, so a
  corrupt secret read as absent. `EnvSource::var` now returns `Result<Option<String>>`;
  verified by spawning the binary with an unpaired surrogate. Next: 0.6.

- **2026-07-15** — M0 item 0.4: `core::geo` — haversine, initial bearing, destination-point
  (the dead-reckoning step), Web Mercator fwd/inv in `EPSG:3857` metres, `LatLon`/`MercatorXy`
  types, lon/bearing normalization. Two radii kept distinct (mean 6371008.8 for great-circle,
  WGS84 6378137.0 for Mercator per its definition). Goldens are analytic arcs + published
  `EPSG:3857` constants, not recalled figures — a remembered LAX→JFK value was wrong and the
  code was right; test now pins the published 2,145 nm. Deferred: orthographic globe (M2, L0
  camera), proptest (deterministic sweep covers docs/10's 1e-9° round-trip), rayon batch
  helpers (M2, with the bench). 28 geo tests, 51 in `core`; fmt/clippy/test green.
  Also rewrote README for a human reader: explains ICAO24/ADS-B/TIS-B/ADS-R/dead reckoning,
  and states plainly that the project needs no receiver hardware. Next: 0.5.

- **2026-07-15** — M0 item 0.3: `core` types + contracts — `core::types` (StateVector, Icao24,
  CallSign, BBox, SourceId, UnixSeconds), `core::error` (SourceError/StoreError, backend-agnostic),
  `core::contracts` (LiveSource, Store, RegionQuery, AircraftMeta, Airport, AirportSize).
  Added `async-trait` 0.1.89 (proc-macro only; needed for a dyn-compatible `LiveSource`).
  Icao24 stores bytes (case-safe Eq/Hash) and rejects readsb `~`-prefixed non-ICAO addresses;
  BBox validates bounds and refuses antimeridian spans. Deferred: `RenderFeed` (M2 shapes),
  serde derives (no consumer yet). 23 unit tests; fmt/clippy/test green. Next: 0.4.

- **2026-07-15** — M0 item 0.2: workspace dependency pins (table + rationale in DECISION_LOG).
  Full `major.minor.patch` + committed Cargo.lock for reproducibility; `=` pins only on
  wgpu 30.0.0 / winit 0.30.13 per ADR-003. winit held at stable 0.30.13 (0.31 is beta).
  Verified wgpu+winit share raw-window-handle 0.6.2, tree is rustls-only (no OpenSSL), and
  SQLite is bundled. Deps wired into owning crates; build/fmt/clippy/test green. Next: 0.3.

- **2026-07-15** — M0 item 0.1: cargo workspace (resolver 3) + five crates
  (core/ingest/store/render/app), workspace lints (clippy all+pedantic, unwrap_used),
  rust-toolchain.toml pinned to 1.96.0, edition 2024 via workspace.package.
  fmt/clippy/test all green. Next: 0.2 (pin deps).

- **2026-07-14** — Repository scaffolded: README/CLAUDE/AGENTS, master prompt, docs 00–13,
  plans (M0–M2, status/decision/risk/next-actions), 7 agents, 3 skills. Initial commit.
  Next: M0 item 0.1.
