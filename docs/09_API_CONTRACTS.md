# 09 — API Contracts

Two halves: the **internal module contracts** (Rust traits between crates) and the
**external endpoint summaries** the ingest crate implements against.

## Internal contracts (`crates/core::contracts`)

These are the seams the crates agree on; changing one requires a decision-log entry.

```rust
/// Normalized live position report. The only shape the pipeline ever sees;
/// each source adapter converts into this and nothing downstream knows sources exist.
pub struct StateVector {
    pub icao24: Icao24,             // newtype over [u8; 3]
    pub callsign: Option<CallSign>, // trimmed, None if blank/anonymous
    pub ts: UnixSeconds,            // source-reported time of applicability
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub baro_alt_m: Option<f32>,
    pub velocity_ms: Option<f32>,
    pub heading_deg: Option<f32>,   // true track
    pub vert_rate_ms: Option<f32>,
    pub on_ground: bool,
    pub anonymous: bool,            // PIA/blocked: gates enrichment (privacy 2.2)
    pub source: SourceId,
}

/// Implemented once per external live-data source (crates/ingest).
/// Adapters own auth, pagination, and parsing; they must be side-effect free
/// beyond HTTP + token cache.
#[async_trait]
pub trait LiveSource: Send + Sync {
    fn id(&self) -> SourceId;
    /// Cost in this source's own budget units for the given query (0 if unmetered).
    fn cost(&self, query: &RegionQuery) -> u32;
    async fn fetch(&self, query: &RegionQuery) -> Result<Vec<StateVector>, SourceError>;
}

/// Bounding box or global. Poller picks region + cadence from camera + budget.
pub struct RegionQuery { pub bbox: Option<BBox>, }

/// Storage seam (crates/store). Sync API; called from the writer thread only.
pub trait Store {
    fn insert_positions(&mut self, batch: &[StateVector]) -> Result<(), StoreError>;
    fn upsert_aircraft_meta(&mut self, meta: &AircraftMeta) -> Result<(), StoreError>;
    fn airports_in_bbox(&self, bbox: BBox, min_size: AirportSize) -> Result<Vec<Airport>, StoreError>;
    fn prune(&mut self, keep_after: UnixSeconds) -> Result<u64, StoreError>;
}

/// What the CPU pipeline hands the renderer each frame (crates/core -> crates/render).
/// Flat, sorted by draw priority, built by the interpolation stage. Double-buffered;
/// the render thread swaps and never blocks on production.
pub struct RenderFeed {
    pub frame_ts: f64,
    pub aircraft: Vec<AircraftInstance>, // pos (projected), heading, alt_bucket, category, flags
    pub trails: Vec<TrailVertex>,
    pub labels: Vec<Label>,              // pre-collision-culled
}
```

Error taxonomy: `SourceError::{Auth, RateLimited{retry_after}, Network, Parse, Server}` —
the poller's backoff logic branches on these; `Parse` never kills the poller (log + skip record).

## External endpoints

Full auth/limits/attribution detail lives in the
[authorized-aviation-sources skill](../.claude/skills/authorized-aviation-sources/SKILL.md); this is the
contract summary the adapters implement.

### OpenSky Network (primary, free account)
- `GET https://opensky-network.org/api/states/all?lamin=&lomin=&lamax=&lomax=`
- Auth: OAuth2 client-credentials → Bearer token (~30 min TTL, refresh at 80%).
- Response: `{ time, states: [[icao24, callsign, origin_country, time_position, last_contact,
  lon, lat, baro_altitude, on_ground, velocity, true_track, vertical_rate, ...], ...] }` —
  positional arrays, nullable everywhere; adapter must tolerate nulls per field.
- Budget: bbox query costs 1–4 credits by area; 4,000 credits/day registered. Poller
  tracks spend in `source_status.credits_used_today`.

### airplanes.live (fallback, no key)
- `GET https://api.airplanes.live/v2/point/{lat}/{lon}/{radius_nm}` (radius ≤ 250 nm)
- Limit: 1 request/second — the adapter itself enforces ≥ 2 s spacing (`ingest::pacer`).
- Response: `{ ac: [{hex, flight, lat, lon, alt_baro, gs, track, baro_rate, seen_pos, ...}],
  now, ... }`; `alt_baro` may be the string `"ground"` — adapter maps to `on_ground=true`.
- **Units are aviation units** (adapter converts to SI): `alt_baro` in **feet**, `gs` in
  **knots**, `baro_rate` in **ft/min**. Position timestamp = `now − seen_pos` (`now` is epoch
  **milliseconds** here, seconds in raw readsb — verified live 2026-07-17). `~`-prefixed
  `hex` values are non-ICAO TIS-B/ADS-R synthetics and are skipped, not tracked.

### adsb.lol (second fallback, no key)
- `GET https://api.adsb.lol/v2/point/{lat}/{lon}/{radius_nm}` — same response family as
  airplanes.live (readsb JSON); shared parsing module *and* shared point-query implementation
  (`ingest::point`), separate adapter id + fixtures + live test (shapes can drift).
- No documented rate limit, so the adapter mirrors airplanes.live's conservative ≥ 2 s
  spacing (`ingest::pacer`) rather than a looser guess (DECISION_LOG 1.6).
- Verified live 2026-07-17: 46 aircraft over Switzerland, units/`now`-in-ms/field names all
  as believed, 0 credits.

### adsbdb (enrichment, on-selection only)
- `GET https://api.adsbdb.com/v0/aircraft/{hex}` / `v0/callsign/{callsign}`
- Called only from the selection path, only when `anonymous == false` (privacy 2.2),
  LRU + negative cache (24 h).

### aviationweather.gov (METAR)
- `GET https://aviationweather.gov/api/data/metar?ids={CSV}&format=json` — batch ≤ 100
  stations, poll ≥ 10 min interval.

## Cross-cutting adapter rules

- Every request: `User-Agent: look-above/<version> (github.com/arcTanMyAngle/look-above)`.
- Timeouts 10 s; retries: none on 4xx, exponential backoff w/ jitter on 429/5xx/network
  (base 5 s, cap 5 min, honor `Retry-After`).
- All parsing is fixture-tested ([10_TEST_PLAN.md](10_TEST_PLAN.md)); adapters never panic on
  malformed records.
