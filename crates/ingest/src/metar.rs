//! `aviationweather.gov`'s METAR JSON API — the flight-category/wind/visibility enrichment
//! source for M3 item 3.3's airport badges.
//!
//! Unlike the live-position sources ([`crate::opensky`], [`crate::airplanes_live`],
//! [`crate::adsb_lol`]), there is no failover chain here — one authorized source, no keyless
//! fallback, because docs/09 lists exactly one METAR provider. [`MetarSource::fetch`] is the
//! stateless adapter half (one request in, `Vec<Metar>` out); [`run_metar_poller`] is the loop
//! half — see its own doc comment for why it does not reuse [`crate::poller::Poller`]'s shape.

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;
use look_above_core::contracts::{FlightCategory, Metar};
use look_above_core::error::SourceError;
use look_above_core::types::UnixSeconds;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::watch;
use tokio::time::sleep;

use crate::http::{HttpClient, send_json};
use crate::poller::WallClock;

/// `aviationweather.gov`'s METAR data endpoint (authorized-aviation-sources skill).
pub const METAR_ENDPOINT: &str = "https://aviationweather.gov/api/data/metar";

/// The skill's documented batch ceiling: at most this many station ids per request.
pub const MAX_STATIONS_PER_REQUEST: usize = 100;

/// The minimum spacing the skill directs between polls of this source ("poll ≥ 10 min apart" —
/// METARs are hourly, so more is waste). Enforced by [`run_metar_poller`]'s own loop shape (a
/// fixed sleep every cycle, regardless of outcome), not by per-request pacing like
/// [`crate::pacer::Pacer`] — there is only ever one caller here, so there is nothing concurrent
/// to space out.
pub const MIN_POLL_INTERVAL: Duration = Duration::from_mins(10);

/// The `aviationweather.gov` METAR source.
#[derive(Debug)]
pub struct MetarSource {
    client: HttpClient,
    endpoint: String,
}

impl MetarSource {
    /// The source against the real endpoint.
    pub fn new(client: HttpClient) -> Self {
        Self::build(client, METAR_ENDPOINT.to_owned())
    }

    /// `pub(crate)` so `record_fixture`/tests can point this at a mock; the allowlist refuses
    /// an unauthorized endpoint regardless (see `http::HttpClient::checked_url`).
    pub(crate) fn build(client: HttpClient, endpoint: String) -> Self {
        Self { client, endpoint }
    }

    /// Fetches the freshest observation for every station in `stations`, chunked into requests
    /// of at most [`MAX_STATIONS_PER_REQUEST`] ids each. All chunks are sent within this one
    /// call — the ≥10-minute spacing [`MIN_POLL_INTERVAL`] documents applies between *cycles*,
    /// not between one cycle's own chunks.
    ///
    /// A malformed individual record is skipped, not fatal (docs/09: "Parse never kills the
    /// poller"); an unreadable response body, or a request that never lands, is (docs/06:
    /// tests are offline by construction, so a live-shape mismatch here is a fixture bug, not
    /// something a caller should paper over).
    pub async fn fetch(&self, stations: &[String]) -> Result<Vec<Metar>, SourceError> {
        let mut metars = Vec::with_capacity(stations.len());
        for chunk in stations.chunks(MAX_STATIONS_PER_REQUEST) {
            metars.extend(self.fetch_chunk(chunk).await?);
        }
        Ok(metars)
    }

    async fn fetch_chunk(&self, stations: &[String]) -> Result<Vec<Metar>, SourceError> {
        let ids = stations.join(",");
        let request = self
            .client
            .get(&self.endpoint)?
            .query(&[("ids", ids.as_str()), ("format", "json")]);
        // Decoded as `Vec<Value>`, then each element parsed into `RawMetar` independently:
        // deserializing straight into `Vec<RawMetar>` would fail the *entire* response the
        // moment one element has the wrong shape (e.g. not an object at all), which is exactly
        // the "one malformed record kills the batch" outcome docs/09 forbids.
        let raw: Vec<Value> = send_json(request).await?;
        Ok(raw
            .into_iter()
            .filter_map(|value| serde_json::from_value::<RawMetar>(value).ok())
            .filter_map(RawMetar::into_metar)
            .collect())
    }
}

/// The wire shape of one element of `aviationweather.gov`'s METAR JSON array — only the fields
/// `core::contracts::Metar`/the `metars` table (docs/08) actually keep. `#[serde(rename_all)]`
/// maps this struct's `snake_case` field names onto the source's camelCase ones; `wdir`/`wspd`/
/// `visib` are already single words and pass through unchanged.
///
/// Verified live 2026-07-21 (`ids=KJFK,KLAX,KORD,...`): `fltCat` is a plain `"VFR"`/`"MVFR"`/
/// `"IFR"`/`"LIFR"` string, `wdir` was a number in every observation seen but the documented
/// format also allows the string `"VRB"` for variable wind (hence [`Value`], not `i64`), and
/// `visib` is sometimes a plain number (`6`) and sometimes a qualified string (`"10+"`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMetar {
    icao_id: Option<String>,
    obs_time: Option<i64>,
    raw_ob: Option<String>,
    flt_cat: Option<String>,
    wdir: Option<Value>,
    wspd: Option<i64>,
    visib: Option<Value>,
}

impl RawMetar {
    /// `None` for a record missing any of the three fields the schema requires `NOT NULL`
    /// (`station`/`observed_at`/`raw`, docs/08) — skipped rather than failing the whole batch.
    fn into_metar(self) -> Option<Metar> {
        Some(Metar {
            station: self.icao_id?,
            observed_at: UnixSeconds(self.obs_time?),
            raw: self.raw_ob?,
            flight_category: self
                .flt_cat
                .as_deref()
                .and_then(FlightCategory::from_metar_str),
            wind_dir_deg: parse_wind_dir(self.wdir),
            wind_kt: self.wspd.and_then(|kt| i32::try_from(kt).ok()),
            visibility_sm: parse_visibility(self.visib),
        })
    }
}

/// `wdir` is a plain number in every observation this adapter has seen live, but the
/// documented format also allows the string `"VRB"` for variable wind — which has no single
/// heading to store, so it (and any other non-numeric value) maps to `None` rather than a
/// guessed degree.
fn parse_wind_dir(value: Option<Value>) -> Option<i32> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .and_then(|degrees| i32::try_from(degrees).ok()),
        _ => None,
    }
}

/// `visib` is either a plain number (`6`) or a qualified string: `"10+"` (at-least-ten, the
/// common clear-day case), a fraction (`"1/2"`, `"M1/4"` — `M` for "less than"), or a whole
/// number plus fraction (`"1 1/2"`). The qualifier is dropped: `metars.visibility_sm` is a
/// plain `REAL` (docs/08) with nowhere to carry "at least" or "less than", so this stores the
/// numeric value itself. Anything this does not recognize maps to `None`, never a panic or an
/// error that would fail the whole record.
fn parse_visibility(value: Option<Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_visibility_str(&text),
        _ => None,
    }
}

fn parse_visibility_str(text: &str) -> Option<f64> {
    let trimmed = text.trim().trim_start_matches('M');
    if let Ok(direct) = trimmed.trim_end_matches('+').parse::<f64>() {
        return Some(direct);
    }
    let (whole, fraction) = match trimmed.rsplit_once(' ') {
        Some((whole, fraction)) => (whole.parse::<f64>().ok()?, fraction),
        None => (0.0, trimmed),
    };
    let (numerator, denominator) = fraction.split_once('/')?;
    let denominator: f64 = denominator.parse().ok()?;
    if denominator == 0.0 {
        return None;
    }
    Some(whole + numerator.parse::<f64>().ok()? / denominator)
}

/// One METAR poll cycle's result — the same "batch struct, not a bare `Vec`" shape
/// [`crate::poller::PollBatch`] uses, delivered to `app`'s simulation worker over a
/// `crossbeam` channel.
#[derive(Debug, Clone, PartialEq)]
pub struct MetarBatch {
    pub fetched_at: UnixSeconds,
    pub metars: Vec<Metar>,
}

/// How often an *empty* station list is rechecked — deliberately much shorter than
/// [`MIN_POLL_INTERVAL`]: an empty check costs nothing (no request leaves the process), so there
/// is no spacing rule to honor, and a station list that arrives just after a check must not sit
/// unpolled for up to a full poll interval before the next look (see [`run_metar_poller`]'s own
/// doc comment for the startup case this fixes: the channel begins empty, and without a short
/// recheck here the *very first* camera settle's station list would wait out one full
/// [`MIN_POLL_INTERVAL`] before ever being read).
pub const IDLE_RECHECK_INTERVAL: Duration = Duration::from_secs(5);

/// Polls `source` for whatever station list `stations` currently holds, delivering each cycle's
/// result to `sender`. `poll_interval` is the spacing between *fetches* (production callers
/// pass [`MIN_POLL_INTERVAL`]) and `idle_recheck_interval` is the spacing between checks while
/// the station list is empty (production callers pass [`IDLE_RECHECK_INTERVAL`]) — both explicit
/// parameters rather than the constants baked in directly, so tests can exercise the cadence
/// logic itself against a real (loopback) HTTP mock at real, short intervals instead of trying
/// to mix a paused virtual clock with genuine socket I/O.
///
/// Unlike [`crate::poller::Poller::run`], retargeting the station list does **not** interrupt a
/// sleep already in progress *after a fetch*: METARs are hourly, so there is no responsiveness
/// case for polling early just because the camera panned, and racing that sleep the way the
/// position poller does would risk shortening the documented ≥10-minute spacing under a rapid
/// sequence of retargets (docs/09: "enforced in code, not just documented"). A retarget only
/// changes what the *next* cycle asks for; the current station list is read fresh
/// (`borrow_and_update`) right before each check. The idle recheck is a different case — see
/// [`IDLE_RECHECK_INTERVAL`]'s own doc comment for why it is short rather than sharing the same
/// reasoning.
///
/// An empty station list — no large airport currently in view, or before the first camera
/// settle — skips the fetch for that cycle entirely rather than sending a request for nothing.
/// A fetch error is logged and retried next cycle: there is no fallback source to fail over to,
/// and a transient miss just leaves the previous cached observation in place (docs/08's own
/// retention already tolerates gaps).
///
/// Exits once a cycle that produced a batch finds `sender`'s receiver gone; an eternally-empty
/// station list has nothing to detect that on, so it idles harmlessly forever — the task is
/// torn down with the runtime at process shutdown, the same as [`crate::poller::Poller::run`]'s
/// task is never explicitly joined either.
pub async fn run_metar_poller(
    source: MetarSource,
    mut stations: watch::Receiver<Vec<String>>,
    sender: Sender<MetarBatch>,
    clock: Arc<dyn WallClock>,
    poll_interval: Duration,
    idle_recheck_interval: Duration,
) {
    loop {
        let current = stations.borrow_and_update().clone();
        if current.is_empty() {
            sleep(idle_recheck_interval).await;
            continue;
        }

        match source.fetch(&current).await {
            Ok(metars) => {
                let fetched_at = clock.now();
                tracing::info!(
                    stations = current.len(),
                    metars = metars.len(),
                    %fetched_at,
                    "metar poll cycle"
                );
                if sender.send(MetarBatch { fetched_at, metars }).is_err() {
                    tracing::info!("pipeline receiver dropped; metar poller shutting down");
                    return;
                }
            }
            Err(error) => {
                tracing::warn!(%error, "metar poll cycle failed; retrying next cycle");
            }
        }
        sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossbeam_channel::unbounded;
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::allowlist::{HostPolicy, is_authorized_host};
    use crate::http::REQUEST_TIMEOUT;
    use crate::poller::SystemWallClock;

    const NOMINAL: &str = include_str!("../tests/fixtures/aviationweather/metar_nominal.json");
    const MALFORMED: &str = include_str!("../tests/fixtures/aviationweather/metar_malformed.json");

    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    fn source_against(server: &MockServer) -> MetarSource {
        MetarSource::build(client(), server.uri())
    }

    async fn mock_metar(server: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .mount(server)
            .await;
    }

    fn stations(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    // ---- Parsing ----------------------------------------------------------------------------

    #[tokio::test]
    async fn a_fetch_returns_the_parsed_batch_from_the_fixture() {
        let server = MockServer::start().await;
        mock_metar(&server, NOMINAL).await;

        let metars = source_against(&server)
            .fetch(&stations(&["KJFK", "KLAX"]))
            .await
            .expect("fetch succeeds");

        assert!(!metars.is_empty());
        assert!(metars.iter().all(|metar| !metar.station.is_empty()));
        assert!(metars.iter().all(|metar| !metar.raw.is_empty()));
    }

    #[tokio::test]
    async fn a_fetch_over_malformed_records_still_returns_the_good_ones() {
        let server = MockServer::start().await;
        mock_metar(&server, MALFORMED).await;

        let metars = source_against(&server)
            .fetch(&stations(&["KJFK"]))
            .await
            .expect("a malformed record must never fail the fetch");
        assert_eq!(metars.len(), 1, "only the well-formed record survives");
        assert_eq!(metars[0].station, "KJFK");
    }

    #[test]
    fn wind_direction_accepts_a_number_and_maps_variable_wind_to_none() {
        assert_eq!(parse_wind_dir(Some(Value::from(280))), Some(280));
        assert_eq!(parse_wind_dir(Some(Value::from("VRB"))), None);
        assert_eq!(parse_wind_dir(None), None);
    }

    #[test]
    fn visibility_accepts_plain_numbers_and_qualified_strings() {
        assert_eq!(parse_visibility(Some(Value::from(6))), Some(6.0));
        assert_eq!(parse_visibility(Some(Value::from("10+"))), Some(10.0));
        assert_eq!(parse_visibility(Some(Value::from("1/2"))), Some(0.5));
        assert_eq!(parse_visibility(Some(Value::from("M1/4"))), Some(0.25));
        assert_eq!(parse_visibility(Some(Value::from("1 1/2"))), Some(1.5));
        assert_eq!(parse_visibility(Some(Value::from("garbage"))), None);
        assert_eq!(parse_visibility(None), None);
    }

    // ---- Batching -----------------------------------------------------------------------------

    #[tokio::test]
    async fn more_than_the_batch_ceiling_is_split_into_multiple_requests() {
        let server = MockServer::start().await;
        // Two chunks of the ceiling: the first request's `ids` must carry exactly
        // MAX_STATIONS_PER_REQUEST ids, the second the remainder.
        let many: Vec<String> = (0..=MAX_STATIONS_PER_REQUEST)
            .map(|i| format!("K{i:03}"))
            .collect();
        let first_ids = many[..MAX_STATIONS_PER_REQUEST].join(",");
        let second_ids = many[MAX_STATIONS_PER_REQUEST..].join(",");

        Mock::given(method("GET"))
            .and(query_param("ids", first_ids.as_str()))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("[]".to_owned(), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("ids", second_ids.as_str()))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("[]".to_owned(), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let metars = source_against(&server)
            .fetch(&many)
            .await
            .expect("fetch succeeds");
        assert!(metars.is_empty());
    }

    // ---- The allowlist --------------------------------------------------------------------------

    #[test]
    fn the_metar_endpoint_is_the_documented_one_and_is_authorized() {
        assert_eq!(METAR_ENDPOINT, "https://aviationweather.gov/api/data/metar");
        let host = reqwest::Url::parse(METAR_ENDPOINT)
            .expect("the endpoint parses")
            .host_str()
            .expect("the endpoint has a host")
            .to_owned();
        assert!(is_authorized_host(&host), "{host} must be on the allowlist");
    }

    // ---- The poller loop --------------------------------------------------------------------
    //
    // These run over real (loopback) sockets, so they use real, short intervals and real time
    // rather than `start_paused` — mixing a paused virtual clock with genuine socket I/O is a
    // known trap (the reactor's real-time completion and the test clock's virtual advancement
    // can race), which is exactly why `poll_interval`/`idle_recheck_interval` are parameters and
    // not the `MIN_POLL_INTERVAL`/`IDLE_RECHECK_INTERVAL` constants baked into the loop (see
    // `run_metar_poller`'s own doc comment). `poller::tests`' own `start_paused` tests avoid the
    // same trap the other way: by never touching a real socket, only a scripted, synchronous
    // test double.

    const TEST_INTERVAL: Duration = Duration::from_millis(120);
    /// Short relative to [`TEST_INTERVAL`] — the idle recheck is meant to be much faster than
    /// the fetch cadence (see [`IDLE_RECHECK_INTERVAL`]'s own doc comment on why).
    const TEST_IDLE_RECHECK: Duration = Duration::from_millis(20);
    /// Generous relative to [`TEST_INTERVAL`] so a loaded CI runner cannot flake this into a
    /// false "arrived too early"/"never arrived" — real-time tests need slack real tests don't.
    const WAIT_CEILING: Duration = Duration::from_secs(5);

    /// Polls `rx` until a batch is available or [`WAIT_CEILING`] elapses.
    async fn recv_within_ceiling(
        rx: &crossbeam_channel::Receiver<MetarBatch>,
    ) -> Option<MetarBatch> {
        tokio::time::timeout(WAIT_CEILING, async {
            loop {
                if let Ok(batch) = rx.try_recv() {
                    return batch;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .ok()
    }

    #[tokio::test]
    async fn an_empty_station_list_never_fetches() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let (_retarget_tx, retarget_rx) = watch::channel(Vec::new());
        let (tx, _rx) = unbounded();
        let handle = tokio::spawn(run_metar_poller(
            source_against(&server),
            retarget_rx,
            tx,
            Arc::new(SystemWallClock),
            TEST_INTERVAL,
            TEST_IDLE_RECHECK,
        ));

        tokio::time::sleep(TEST_INTERVAL * 3).await;

        handle.abort();
        let _ = handle.await;
        // wiremock's own `expect(0)` (asserted on drop) is the real check here.
    }

    /// Regression test for the startup case [`IDLE_RECHECK_INTERVAL`]'s own doc comment
    /// describes: a station list that starts empty and is populated shortly after must be
    /// picked up on the next *idle* recheck, not wait out a full `poll_interval` — caught live
    /// (M3 item 3.3's own manual verification pass) before this parameter existed, when the
    /// loop's only sleep was the fetch cadence and a fresh camera settle's badges could be
    /// delayed up to a full 10 minutes.
    #[tokio::test]
    async fn a_station_list_populated_after_starting_empty_is_picked_up_on_the_next_idle_recheck() {
        let server = MockServer::start().await;
        mock_metar(&server, NOMINAL).await;

        let (retarget_tx, retarget_rx) = watch::channel(Vec::new());
        let (tx, rx) = unbounded();
        let handle = tokio::spawn(run_metar_poller(
            source_against(&server),
            retarget_rx,
            tx,
            Arc::new(SystemWallClock),
            TEST_INTERVAL,
            TEST_IDLE_RECHECK,
        ));

        // Give the loop a couple of idle cycles over the still-empty list before populating it.
        tokio::time::sleep(TEST_IDLE_RECHECK * 2).await;
        assert!(rx.try_recv().is_err(), "nothing to fetch yet");

        retarget_tx
            .send(stations(&["KJFK"]))
            .expect("the poller task still holds the receiver");

        recv_within_ceiling(&rx)
            .await
            .expect("picked up within a couple of idle rechecks, not a full poll_interval");

        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn a_populated_list_fetches_immediately_then_waits_the_full_interval() {
        let server = MockServer::start().await;
        mock_metar(&server, NOMINAL).await;

        let (_retarget_tx, retarget_rx) = watch::channel(stations(&["KJFK"]));
        let (tx, rx) = unbounded();
        let handle = tokio::spawn(run_metar_poller(
            source_against(&server),
            retarget_rx,
            tx,
            Arc::new(SystemWallClock),
            TEST_INTERVAL,
            TEST_IDLE_RECHECK,
        ));

        let first = recv_within_ceiling(&rx)
            .await
            .expect("the first cycle ran without waiting for an interval");
        assert!(!first.metars.is_empty());

        // Nothing more arrives well before a full interval elapses.
        tokio::time::sleep(TEST_INTERVAL / 3).await;
        assert!(rx.try_recv().is_err(), "must not poll before the interval");

        // The second cycle eventually runs once the interval has elapsed.
        recv_within_ceiling(&rx)
            .await
            .expect("the second cycle ran after the interval");

        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn retargeting_mid_sleep_does_not_shorten_the_interval() {
        let server = MockServer::start().await;
        mock_metar(&server, NOMINAL).await;

        let (retarget_tx, retarget_rx) = watch::channel(stations(&["KJFK"]));
        let (tx, rx) = unbounded();
        let handle = tokio::spawn(run_metar_poller(
            source_against(&server),
            retarget_rx,
            tx,
            Arc::new(SystemWallClock),
            TEST_INTERVAL,
            TEST_IDLE_RECHECK,
        ));

        recv_within_ceiling(&rx).await.expect("first cycle ran");

        // A retarget partway through the sleep must not wake the loop early.
        tokio::time::sleep(TEST_INTERVAL / 2).await;
        retarget_tx
            .send(stations(&["KLAX"]))
            .expect("the poller task still holds the receiver");
        tokio::time::sleep(TEST_INTERVAL / 4).await;
        assert!(
            rx.try_recv().is_err(),
            "a mid-sleep retarget must not trigger an early fetch"
        );

        // The next cycle still runs once the full interval elapses.
        recv_within_ceiling(&rx)
            .await
            .expect("the next cycle still ran after the full interval");

        handle.abort();
        let _ = handle.await;
    }

    #[tokio::test]
    async fn a_dropped_receiver_stops_the_loop_on_the_next_delivering_cycle() {
        let server = MockServer::start().await;
        mock_metar(&server, NOMINAL).await;

        let (_retarget_tx, retarget_rx) = watch::channel(stations(&["KJFK"]));
        let (tx, rx) = unbounded();
        drop(rx);

        // Runs to completion (returns) as soon as the one populated cycle tries to send.
        run_metar_poller(
            source_against(&server),
            retarget_rx,
            tx,
            Arc::new(SystemWallClock),
            TEST_INTERVAL,
            TEST_IDLE_RECHECK,
        )
        .await;
    }

    // ---- The real aviationweather.gov ------------------------------------------------------------

    /// The one test that fetches real METARs. Keyless and free (NOAA), but be gentle:
    ///
    /// ```text
    /// cargo test -p look-above-ingest -- --ignored live_metar
    /// ```
    ///
    /// Nothing here prints a payload — only counts (docs/06).
    #[tokio::test]
    #[ignore = "hits the real aviationweather.gov API; keyless and free, but be gentle"]
    async fn live_metar_matches_the_documented_shape() {
        let source = MetarSource::new(HttpClient::new().expect("client builds"));
        let metars = source
            .fetch(&stations(&["KJFK", "KLAX", "KORD"]))
            .await
            .expect("aviationweather.gov answers");
        assert!(!metars.is_empty(), "no METARs for three major US airports");

        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is past 1970")
                .as_secs(),
        )
        .expect("fits in i64");
        for metar in &metars {
            assert!(
                (metar.observed_at.0 - now).abs() < 3600 * 3,
                "{} observed at {} is not within a few hours of now — is obsTime really seconds?",
                metar.station,
                metar.observed_at
            );
            assert!(!metar.raw.is_empty());
        }
        eprintln!(
            "live aviationweather.gov: {} metar(s), {} with a flight category",
            metars.len(),
            metars
                .iter()
                .filter(|m| m.flight_category.is_some())
                .count()
        );
    }
}
