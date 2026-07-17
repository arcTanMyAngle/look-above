//! adsb.lol `/v2/point` — the second keyless fallback (failover position 2).
//!
//! An open-data project rebroadcasting the same readsb JSON as airplanes.live, so the whole
//! query mechanism is the shared [`PointSource`](crate::point::PointSource) and this adapter
//! is only what is adsb.lol's own: its host, its [`SourceId`], its spacing, its fixtures, and
//! the live check.
//!
//! The two services drift independently — that is exactly why docs/09 mandates a shared
//! *parser* but separate *adapters and fixtures*. The shape is identical today; a fixture set
//! and a live test that are adsb.lol's own are what will catch the day it stops being.
//!
//! **Spacing.** The skill documents no explicit rate limit for adsb.lol (unlike
//! airplanes.live's 1 req/s), only "be gentle, credit them". Absent a number, this adapter
//! mirrors airplanes.live's conservative ≥ 2 s spacing rather than inventing a looser one —
//! privacy rule 1.3 is "never exceed documented limits", and with none documented the safe
//! reading is the gentle one. Recorded in `DECISION_LOG` 1.6.

use std::time::Duration;

use async_trait::async_trait;
use look_above_core::contracts::{LiveSource, RegionQuery};
use look_above_core::error::SourceError;
use look_above_core::types::{SourceId, StateVector};

use crate::http::HttpClient;
use crate::point::PointSource;

/// adsb.lol's point query (authorized-aviation-sources skill); the path takes
/// `/{lat}/{lon}/{radius_nm}`.
pub const POINT_ENDPOINT: &str = "https://api.adsb.lol/v2/point";

/// The conservative spacing this adapter holds to — see the module note: adsb.lol documents
/// no rate limit, so it inherits airplanes.live's gentle ≥ 2 s rather than a looser guess.
pub const MIN_REQUEST_SPACING: Duration = Duration::from_secs(2);

/// The adsb.lol live-position source.
#[derive(Debug)]
pub struct AdsbLolSource {
    inner: PointSource,
}

impl AdsbLolSource {
    /// The source against the real endpoint. No credentials to hold: always enabled, which
    /// is what makes it the last-resort failover.
    pub fn new(client: HttpClient) -> Self {
        Self::build(client, POINT_ENDPOINT.to_owned())
    }

    /// The one real constructor, private for the same reason the other adapters' are: the
    /// endpoint override reaches a mock in tests, not a different source in production — and
    /// the allowlist would refuse a different source anyway.
    fn build(client: HttpClient, endpoint: String) -> Self {
        Self {
            inner: PointSource::new(client, endpoint, SourceId::AdsbLol, MIN_REQUEST_SPACING),
        }
    }
}

#[async_trait]
impl LiveSource for AdsbLolSource {
    fn id(&self) -> SourceId {
        SourceId::AdsbLol
    }

    /// Unmetered — no credits, no ledger. What adsb.lol asks is gentleness, paid in time by
    /// the shared [`PointSource`]'s pacer.
    fn cost(&self, _query: &RegionQuery) -> u32 {
        0
    }

    async fn fetch(&self, query: &RegionQuery) -> Result<Vec<StateVector>, SourceError> {
        self.inner.fetch(query).await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use look_above_core::types::{BBox, CallSign, UnixSeconds};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::allowlist::{HostPolicy, is_authorized_host};
    use crate::http::REQUEST_TIMEOUT;

    const NOMINAL: &str = include_str!("../tests/fixtures/adsblol/point_nominal.json");
    const EMPTY: &str = include_str!("../tests/fixtures/adsblol/point_empty.json");
    const NULLS: &str = include_str!("../tests/fixtures/adsblol/point_nulls.json");
    const MALFORMED: &str = include_str!("../tests/fixtures/adsblol/point_malformed.json");

    fn bbox(lat_min: f64, lon_min: f64, lat_max: f64, lon_max: f64) -> BBox {
        BBox::new(lat_min, lon_min, lat_max, lon_max).expect("valid bbox in test")
    }

    fn a_region() -> RegionQuery {
        RegionQuery::region(bbox(46.0, 7.0, 48.0, 9.0))
    }

    /// The real client, widened to reach a loopback mock — so the shipping User-Agent,
    /// timeout, and allowlist are what these assertions run through.
    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    fn source_against(server: &MockServer) -> AdsbLolSource {
        AdsbLolSource::build(client(), format!("{}/v2/point", server.uri()))
    }

    async fn mock_point(server: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .mount(server)
            .await;
    }

    // ---- The adapter over its own fixtures -------------------------------------------------------

    /// The full path: real client, shared parse, adsb.lol's own fixture body. Geometry and
    /// bbox trimming are proven in `point::tests`; here the point is that this adapter stamps
    /// its own id (`AdsbLol`, not `AirplanesLive`) and reads its own recorded shape.
    #[tokio::test]
    async fn a_fetch_returns_the_parsed_normalized_batch() {
        let server = MockServer::start().await;
        mock_point(&server, NOMINAL).await;

        // A box around the two Swiss-area aircraft in the fixture; the ground target is
        // outside it and filtered, which is itself part of the expectation.
        let states = source_against(&server)
            .fetch(&RegionQuery::region(bbox(46.0, 7.0, 51.0, 9.0)))
            .await
            .expect("the fetch succeeds");

        assert_eq!(states.len(), 2);
        assert_eq!(states[0].callsign, CallSign::new("SWR22H"));
        assert_eq!(states[0].ts, UnixSeconds(1_721_000_000));
        assert_eq!(states[0].baro_alt_m, Some(10972.8), "feet became metres");
        assert!(states[1].anonymous);
        assert!(
            states.iter().all(|s| s.source == SourceId::AdsbLol),
            "every record must be stamped adsb.lol, not the shared source's default"
        );
    }

    #[tokio::test]
    async fn a_fetch_over_a_malformed_batch_still_returns_the_good_records() {
        let server = MockServer::start().await;
        mock_point(&server, MALFORMED).await;

        let states = source_against(&server)
            .fetch(&RegionQuery::region(bbox(46.0, 7.0, 51.0, 9.0)))
            .await
            .expect("a malformed record must never fail the fetch");
        assert_eq!(states.len(), 2);
    }

    #[tokio::test]
    async fn an_empty_region_yields_no_aircraft_rather_than_an_error() {
        let server = MockServer::start().await;
        mock_point(&server, EMPTY).await;

        let states = source_against(&server)
            .fetch(&a_region())
            .await
            .expect("an empty region is not an error");
        assert!(states.is_empty());
    }

    /// docs/10 §2: nulls (and absent) in every optional field still yield a position, stamped
    /// with this source's id.
    #[tokio::test]
    async fn nulls_in_every_optional_field_still_yield_a_position() {
        let server = MockServer::start().await;
        mock_point(&server, NULLS).await;

        let states = source_against(&server)
            .fetch(&RegionQuery::region(bbox(46.0, 7.0, 51.0, 9.0)))
            .await
            .expect("nulls in optional fields are tolerated");
        assert_eq!(states.len(), 2);
        assert!(states.iter().all(|s| s.source == SourceId::AdsbLol));
        assert_eq!(
            states[0].baro_alt_m, None,
            "a null altitude is absent, not zero"
        );
        assert!(states[0].anonymous, "no flight field means anonymous");
    }

    // ---- Spacing ---------------------------------------------------------------------------------

    /// Wired to the conservative ≥ 2 s spacing the module note explains. The pacer's own
    /// behavior is proven under paused time in `pacer::tests`.
    #[test]
    fn the_source_paces_at_the_conservative_spacing() {
        let source = AdsbLolSource::new(HttpClient::new().expect("client builds"));
        assert_eq!(source.inner.spacing(), MIN_REQUEST_SPACING);
        assert_eq!(MIN_REQUEST_SPACING, Duration::from_secs(2));
    }

    // ---- Failure paths ---------------------------------------------------------------------------

    /// docs/10 §2 asks for a 5xx case; the mapping lives in `http` and must survive the
    /// adapter intact. (429 and parse mapping are exercised in `airplanes_live` over the same
    /// shared client and `PointSource` — no need to re-prove the mapping per source.)
    #[tokio::test]
    async fn an_upstream_failure_surfaces_as_server_and_is_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let error = source_against(&server)
            .fetch(&a_region())
            .await
            .expect_err("503 is an error");
        assert_eq!(error, SourceError::Server { status: 503 });
        assert!(error.is_transient());
    }

    // ---- The allowlist ---------------------------------------------------------------------------

    #[test]
    fn the_point_endpoint_is_the_documented_one_and_is_authorized() {
        assert_eq!(POINT_ENDPOINT, "https://api.adsb.lol/v2/point");
        let host = reqwest::Url::parse(POINT_ENDPOINT)
            .expect("the endpoint parses")
            .host_str()
            .expect("the endpoint has a host")
            .to_owned();
        assert!(is_authorized_host(&host), "{host} must be on the allowlist");
    }

    /// Privacy rule 1.1 on the shipping client: whatever endpoint a bug names, the request
    /// goes to adsb.lol or nowhere.
    #[tokio::test]
    async fn the_real_client_will_not_fetch_from_an_unauthorized_host() {
        let source = AdsbLolSource::build(
            HttpClient::new().expect("client builds"),
            "https://www.flightradar24.com/v2/point".to_owned(),
        );
        let error = source
            .fetch(&a_region())
            .await
            .expect_err("a prohibited host is refused");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(reason.contains("flightradar24"), "{reason}");
        assert!(!error.is_transient());
    }

    // ---- The real adsb.lol -----------------------------------------------------------------------

    /// The one test that fetches real aircraft, and the reason adsb.lol's fixtures can be
    /// trusted: they are hand-written to readsb's documented shape, so the mocks prove we
    /// parse what we *believe* adsb.lol sends. The beliefs at risk are the same three the
    /// airplanes.live live test pins — that `now` is milliseconds, that `alt_baro`/`gs` are
    /// feet/knots, and that the field names are what the docs say — checked here against
    /// adsb.lol independently, because the two services can drift apart.
    ///
    /// Keyless and free, but an open-data community project — run it once after changes here,
    /// not in a loop, and it stays `#[ignore]`d so CI never runs it:
    ///
    /// ```text
    /// cargo test -p look-above-ingest -- --ignored live_adsb_lol
    /// ```
    ///
    /// Nothing here prints a payload — only counts (docs/06).
    #[tokio::test]
    #[ignore = "hits the real adsb.lol API; keyless and free, but be gentle"]
    async fn live_adsb_lol_point_matches_the_documented_shape() {
        let source = AdsbLolSource::new(HttpClient::new().expect("client builds"));

        // Switzerland, as in the airplanes.live live test: reliably busy, modest radius.
        let region = bbox(46.0, 7.0, 48.0, 9.0);
        let states = source
            .fetch(&RegionQuery::region(region))
            .await
            .expect("adsb.lol answers");
        assert!(
            !states.is_empty(),
            "no aircraft over Switzerland — either the sky is empty or the field names \
             are not what we believe"
        );

        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is past 1970")
                .as_secs(),
        )
        .expect("fits in i64");

        for state in &states {
            assert!(
                region.contains(state.lat_deg, state.lon_deg),
                "{} escaped the bbox filter at lat {} lon {}",
                state.icao24,
                state.lat_deg,
                state.lon_deg
            );
            assert!(
                (state.ts.0 - now).abs() < 3600,
                "{} is timestamped {} — {}s from the wall clock; is `now` really ms?",
                state.icao24,
                state.ts,
                (state.ts.0 - now).abs()
            );
            if let Some(altitude) = state.baro_alt_m {
                assert!(
                    (-500.0..=20000.0).contains(&altitude),
                    "{} reports {altitude} m — that is not an altitude in metres",
                    state.icao24
                );
            }
            if let Some(velocity) = state.velocity_ms {
                assert!(
                    (0.0..400.0).contains(&velocity),
                    "{} reports {velocity} m/s — that is not a speed in m/s",
                    state.icao24
                );
            }
            if let Some(heading) = state.heading_deg {
                assert!(
                    (0.0..=360.0).contains(&heading),
                    "{} reports heading {heading}",
                    state.icao24
                );
            }
        }

        assert!(
            states.iter().any(|s| s.callsign.is_some()),
            "not one aircraft had a callsign — `flight` is probably not the callsign field"
        );
        assert!(
            states.iter().any(|s| s.velocity_ms.is_some()),
            "not one aircraft had a velocity — `gs` is probably not the speed field"
        );

        eprintln!(
            "live adsb.lol /v2/point: {} aircraft in {:?}, {} anonymous, {} on the ground, \
             0 credits (unmetered)",
            states.len(),
            region,
            states.iter().filter(|s| s.anonymous).count(),
            states.iter().filter(|s| s.on_ground).count(),
        );
    }
}
