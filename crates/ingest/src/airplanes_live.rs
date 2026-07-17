//! airplanes.live `/v2/point` — the first keyless fallback (failover position 1).
//!
//! Community-run, no account, no credit ledger: the two things the operators ask are an
//! identifying User-Agent (the shared client's job) and gentleness — the documented limit
//! is 1 request/second, and the skill directs ≥ 2 s spacing, enforced here by a
//! [`Pacer`] rather than left to the poller's manners.
//!
//! The endpoint answers a **point and radius**, not a bounding box, so the adapter's own
//! geometry problem is turning a [`RegionQuery`] bbox into the smallest circle that
//! covers it ([`covering_circle`]), clamped to the documented 250 nm ceiling, and then
//! filtering the reply back down to the box — the circle necessarily sees past the
//! corners, and "the same bbox from every source" is the contract 1.9's merge assumes.
//!
//! Parsing itself lives in [`readsb`](crate::readsb): adsb.lol (1.6) speaks the same
//! shape, and the shapes drift independently, so the endpoint, spacing, and fixtures are
//! per-adapter while the field mapping is shared.

use std::time::Duration;

use async_trait::async_trait;
use look_above_core::contracts::{LiveSource, RegionQuery};
use look_above_core::error::SourceError;
use look_above_core::geo::{LatLon, haversine_distance_m};
use look_above_core::types::{BBox, SourceId, StateVector};

use crate::http::{HttpClient, send_json};
use crate::pacer::Pacer;
use crate::readsb::{METRES_PER_NAUTICAL_MILE, PointResponse};

/// airplanes.live's point query (authorized-aviation-sources skill); the path takes
/// `/{lat}/{lon}/{radius_nm}`.
pub const POINT_ENDPOINT: &str = "https://api.airplanes.live/v2/point";

/// The documented ceiling on the radius parameter, in nautical miles.
pub const MAX_RADIUS_NM: u32 = 250;

/// The spacing the skill directs: the documented limit is 1 request/second, and holding
/// to half that keeps a community-run source comfortable (privacy rule 1.3).
pub const MIN_REQUEST_SPACING: Duration = Duration::from_secs(2);

/// The airplanes.live live-position source.
#[derive(Debug)]
pub struct AirplanesLiveSource {
    client: HttpClient,
    pacer: Pacer,
    endpoint: String,
}

impl AirplanesLiveSource {
    /// The source against the real endpoint. No credentials to hold: the source is
    /// always enabled, which is exactly what makes it the failover.
    pub fn new(client: HttpClient) -> Self {
        Self::build(client, POINT_ENDPOINT.to_owned())
    }

    /// The one real constructor, private for the same reason `OpenSkySource::build` is:
    /// the endpoint override exists so tests can reach a mock, not so callers can
    /// retarget the adapter — and the allowlist would refuse them anyway.
    fn build(client: HttpClient, endpoint: String) -> Self {
        Self {
            client,
            pacer: Pacer::new(MIN_REQUEST_SPACING),
            endpoint,
        }
    }
}

#[async_trait]
impl LiveSource for AirplanesLiveSource {
    fn id(&self) -> SourceId {
        SourceId::AirplanesLive
    }

    /// Unmetered — no credits, no ledger, so the contract's "0 when unmetered". The cost
    /// airplanes.live *does* care about is spacing, which [`fetch`](Self::fetch) pays in
    /// time via the pacer instead.
    fn cost(&self, _query: &RegionQuery) -> u32 {
        0
    }

    async fn fetch(&self, query: &RegionQuery) -> Result<Vec<StateVector>, SourceError> {
        // A point/radius endpoint cannot be asked for the world, and pretending — a max
        // radius circle around an arbitrary point — would be a confidently wrong answer.
        // Global polling is M4's problem; `Refused` is not transient, so the poller moves
        // on rather than retrying.
        let Some(bbox) = query.bbox else {
            return Err(SourceError::Refused {
                reason: "airplanes.live answers a point and radius, not a global query \
                         (global polling is deferred to M4)"
                    .to_owned(),
            });
        };

        let circle = covering_circle(bbox);
        if circle.truncated {
            // Partial coverage beats refusing to fail over, but it must not be silent:
            // aircraft near the box's corners will be missing until the region shrinks.
            tracing::warn!(
                radius_nm = circle.radius_nm,
                "bbox needs a larger radius than airplanes.live's 250 nm ceiling; \
                 coverage will be partial"
            );
        }

        // Four decimals ≈ 11 m — far below the 1 nm the radius is rounded up by, and it
        // keeps float noise from the midpoint arithmetic out of the URL.
        let url = format!(
            "{}/{:.4}/{:.4}/{}",
            self.endpoint, circle.center.lat_deg, circle.center.lon_deg, circle.radius_nm
        );
        let request = self.client.get(&url)?;

        // Pace *after* the allowlist could have refused: a request that never leaves
        // should not spend the interval.
        self.pacer.pause().await;

        let response: PointResponse = send_json(request).await?;
        let mut states = response.into_state_vectors(SourceId::AirplanesLive);
        // The circle circumscribes the box, so trim the overhang: every source must
        // answer the same question or 1.9's merge is comparing different regions.
        states.retain(|state| bbox.contains(state.lat_deg, state.lon_deg));
        Ok(states)
    }
}

/// The smallest point-and-radius query that covers `bbox`.
struct CoveringCircle {
    center: LatLon,
    radius_nm: u32,
    /// True when the box needed more than [`MAX_RADIUS_NM`] and was clamped.
    truncated: bool,
}

/// Center = the bbox midpoint; radius = the farthest corner, rounded **up** to whole
/// nautical miles so the circle circumscribes the box rather than clipping its corners.
///
/// All four corners are measured because the lat/lon midpoint is not equidistant from
/// them on a sphere — the pair farther from the pole is farther in metres. The floor of
/// 1 nm keeps a degenerate (point) bbox a valid query instead of a radius-zero request.
fn covering_circle(bbox: BBox) -> CoveringCircle {
    let center = LatLon::new(
        f64::midpoint(bbox.lat_min(), bbox.lat_max()),
        f64::midpoint(bbox.lon_min(), bbox.lon_max()),
    );
    let corners = [
        LatLon::new(bbox.lat_min(), bbox.lon_min()),
        LatLon::new(bbox.lat_min(), bbox.lon_max()),
        LatLon::new(bbox.lat_max(), bbox.lon_min()),
        LatLon::new(bbox.lat_max(), bbox.lon_max()),
    ];
    let farthest_corner_m = corners
        .iter()
        .map(|corner| haversine_distance_m(center, *corner))
        .fold(0.0_f64, f64::max);

    let radius_nm = (farthest_corner_m / METRES_PER_NAUTICAL_MILE).ceil();
    if radius_nm > f64::from(MAX_RADIUS_NM) {
        CoveringCircle {
            center,
            radius_nm: MAX_RADIUS_NM,
            truncated: true,
        }
    } else {
        // In range and non-negative by construction: `ceil` of a distance, ≤ 250.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let radius_nm = radius_nm.max(1.0) as u32;
        CoveringCircle {
            center,
            radius_nm,
            truncated: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use look_above_core::types::{CallSign, Icao24, UnixSeconds};
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use super::*;
    use crate::allowlist::{HostPolicy, is_authorized_host};
    use crate::http::{REQUEST_TIMEOUT, USER_AGENT};

    const NOMINAL: &str = include_str!("../tests/fixtures/airplaneslive/point_nominal.json");
    const EMPTY: &str = include_str!("../tests/fixtures/airplaneslive/point_empty.json");
    const MALFORMED: &str = include_str!("../tests/fixtures/airplaneslive/point_malformed.json");

    fn bbox(lat_min: f64, lon_min: f64, lat_max: f64, lon_max: f64) -> BBox {
        BBox::new(lat_min, lon_min, lat_max, lon_max).expect("valid bbox in test")
    }

    /// Switzerland, the same region 1.4's tests use: center (47, 8), radius 73 nm.
    fn a_region() -> RegionQuery {
        RegionQuery::region(bbox(46.0, 7.0, 48.0, 9.0))
    }

    /// The real client, widened to reach a loopback mock — the same escape hatch every
    /// adapter test uses, so the shipping User-Agent, timeout, and allowlist are what
    /// these assertions run through.
    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    fn source_against(server: &MockServer) -> AirplanesLiveSource {
        AirplanesLiveSource::build(client(), format!("{}/v2/point", server.uri()))
    }

    /// Mounts a catch-all GET responder: the interesting assertion is the path the
    /// adapter *built*, so matching on it would assume the answer.
    async fn mock_point(server: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"),
            )
            .mount(server)
            .await;
    }

    async fn only_request(server: &MockServer) -> Request {
        let mut requests = server
            .received_requests()
            .await
            .expect("requests are recorded");
        assert_eq!(requests.len(), 1, "expected exactly one request");
        requests.remove(0)
    }

    /// The `/{lat}/{lon}/{radius}` tail of the request path.
    fn point_segments(request: &Request) -> (String, String, String) {
        let segments: Vec<&str> = request
            .url
            .path_segments()
            .expect("the URL has a path")
            .collect();
        let [.., lat, lon, radius] = segments.as_slice() else {
            panic!("path {} has no point segments", request.url.path());
        };
        ((*lat).to_owned(), (*lon).to_owned(), (*radius).to_owned())
    }

    // ---- The request on the wire -----------------------------------------------------------------

    /// docs/10 §2: assert the request shape. The radius is pinned by hand-calculation:
    /// the farthest corners of (46..48, 7..9) from (47, 8) are the southern pair —
    /// haversine gives 134,988 m = 72.9 nm to (46, 7) — so the ceil is 73. A swap of the
    /// center's lat/lon would send 8.0/47.0 and fail the first two assertions.
    #[tokio::test]
    async fn a_bbox_becomes_the_covering_point_and_radius() {
        let server = MockServer::start().await;
        mock_point(&server, EMPTY).await;

        let source = source_against(&server);
        source.fetch(&a_region()).await.expect("the fetch succeeds");

        let request = only_request(&server).await;
        let (lat, lon, radius) = point_segments(&request);
        assert_eq!(lat, "47.0000", "center latitude");
        assert_eq!(lon, "8.0000", "center longitude");
        assert_eq!(radius, "73", "ceil(72.9 nm), circumscribing the box");
        assert_eq!(
            request.headers["user-agent"], USER_AGENT,
            "the adapter must not bypass the shared client"
        );
    }

    /// A box needing more than 250 nm is clamped to the documented ceiling, not refused
    /// and not sent oversized: partial coverage beats no failover.
    #[tokio::test]
    async fn an_oversized_bbox_is_clamped_to_the_documented_maximum_radius() {
        let server = MockServer::start().await;
        mock_point(&server, EMPTY).await;

        let source = source_against(&server);
        // (40..52, -2..16): the corner is ~534 nm from center (46, 7).
        source
            .fetch(&RegionQuery::region(bbox(40.0, -2.0, 52.0, 16.0)))
            .await
            .expect("the fetch succeeds");

        let (_, _, radius) = point_segments(&only_request(&server).await);
        assert_eq!(radius, "250");
    }

    /// A degenerate (point) bbox still queries: radius floors at 1 nm, never 0.
    #[tokio::test]
    async fn a_degenerate_bbox_queries_a_one_mile_circle() {
        let server = MockServer::start().await;
        mock_point(&server, EMPTY).await;

        let source = source_against(&server);
        source
            .fetch(&RegionQuery::region(bbox(50.0, 8.0, 50.0, 8.0)))
            .await
            .expect("the fetch succeeds");

        let (lat, lon, radius) = point_segments(&only_request(&server).await);
        assert_eq!((lat.as_str(), lon.as_str()), ("50.0000", "8.0000"));
        assert_eq!(radius, "1");
    }

    /// The endpoint serves a circle; the contract is a bbox. Aircraft in the circle but
    /// outside the box must be trimmed, or 1.9's merge compares different regions per
    /// source.
    #[tokio::test]
    async fn aircraft_outside_the_bbox_are_filtered_from_the_circle() {
        let server = MockServer::start().await;
        let body = json!({
            "ac": [
                // Inside (46..48, 7..9).
                { "hex": "3c6444", "flight": "DLH9LF  ", "lat": 47.5, "lon": 8.0, "seen_pos": 0.0 },
                // In the covering circle (just past the northern edge), outside the box.
                { "hex": "4ca7b6", "flight": "EIN45K  ", "lat": 48.3, "lon": 8.0, "seen_pos": 0.0 },
            ],
            "now": 1_721_000_000_000.0_f64,
        });
        mock_point(&server, &body.to_string()).await;

        let source = source_against(&server);
        let states = source.fetch(&a_region()).await.expect("the fetch succeeds");

        assert_eq!(states.len(), 1);
        assert_eq!(
            states[0].icao24,
            Icao24::from_hex("3c6444").expect("valid hex"),
            "only the aircraft inside the bbox survives"
        );
    }

    /// The full path: real client, real parse, fixture body.
    #[tokio::test]
    async fn a_fetch_returns_the_parsed_normalized_batch() {
        let server = MockServer::start().await;
        mock_point(&server, NOMINAL).await;

        let source = source_against(&server);
        // A box around the two Swiss-area aircraft in the fixture; the JFK ground target
        // is outside it and filtered, which is itself part of the expectation.
        let states = source
            .fetch(&RegionQuery::region(bbox(46.0, 7.0, 51.0, 9.0)))
            .await
            .expect("the fetch succeeds");

        assert_eq!(states.len(), 2);
        assert_eq!(states[0].callsign, CallSign::new("DLH9LF"));
        assert_eq!(states[0].ts, UnixSeconds(1_721_000_000));
        assert_eq!(states[0].baro_alt_m, Some(10972.8), "feet became metres");
        assert!(states[1].anonymous);
        assert!(states.iter().all(|s| s.source == SourceId::AirplanesLive));
    }

    #[tokio::test]
    async fn a_fetch_over_a_malformed_batch_still_returns_the_good_records() {
        let server = MockServer::start().await;
        mock_point(&server, MALFORMED).await;

        let source = source_against(&server);
        let states = source
            .fetch(&RegionQuery::region(bbox(46.0, 7.0, 51.0, 9.0)))
            .await
            .expect("a malformed record must never fail the fetch");
        assert_eq!(states.len(), 2);
    }

    // ---- The query the endpoint cannot answer ----------------------------------------------------

    /// No request leaves for a global query — not even to be told no.
    #[tokio::test]
    async fn a_global_query_is_refused_without_sending_anything() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(EMPTY, "application/json"))
            .expect(0)
            .mount(&server)
            .await;

        let source = source_against(&server);
        let error = source
            .fetch(&RegionQuery::GLOBAL)
            .await
            .expect_err("a point endpoint cannot answer a global query");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(reason.contains("global"), "{reason}");
        assert!(
            !error.is_transient(),
            "retrying will not make the endpoint global — the poller must move on"
        );
        // expect(0): asserted on drop.
    }

    // ---- Spacing ----------------------------------------------------------------------------------

    /// The adapter is wired to the skill's ≥ 2 s spacing. That the pacer actually spaces
    /// is proven under paused time in `pacer::tests` — re-proving it here over wiremock
    /// would mix the paused clock with real sockets, where auto-advance can fire the
    /// 10 s request timeout mid-reply.
    #[test]
    fn the_source_paces_at_the_documented_spacing() {
        let source = AirplanesLiveSource::new(HttpClient::new().expect("client builds"));
        assert_eq!(source.pacer.interval(), MIN_REQUEST_SPACING);
        assert_eq!(MIN_REQUEST_SPACING, Duration::from_secs(2));
    }

    // ---- Failure paths ----------------------------------------------------------------------------

    /// docs/10 §2 asks for a 429 case. airplanes.live speaks the standard header; the
    /// mapping lives in `http` and must survive the adapter intact.
    #[tokio::test]
    async fn a_rate_limited_fetch_surfaces_the_retry_hint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30"))
            .mount(&server)
            .await;

        let error = source_against(&server)
            .fetch(&a_region())
            .await
            .expect_err("429 is an error");
        assert_eq!(
            error,
            SourceError::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            }
        );
        assert!(error.is_transient());
    }

    /// docs/10 §2 asks for a 5xx case.
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

    /// A body we cannot read is a `Parse`, and by contract that never kills the poller.
    #[tokio::test]
    async fn an_unreadable_body_surfaces_as_parse() {
        let server = MockServer::start().await;
        mock_point(&server, "<html>maintenance</html>").await;

        let error = source_against(&server)
            .fetch(&a_region())
            .await
            .expect_err("html is not a point response");
        assert!(matches!(error, SourceError::Parse { .. }), "{error:?}");
        assert!(!error.is_transient());
    }

    // ---- The allowlist ----------------------------------------------------------------------------

    #[test]
    fn the_point_endpoint_is_the_documented_one_and_is_authorized() {
        assert_eq!(POINT_ENDPOINT, "https://api.airplanes.live/v2/point");
        let host = reqwest::Url::parse(POINT_ENDPOINT)
            .expect("the endpoint parses")
            .host_str()
            .expect("the endpoint has a host")
            .to_owned();
        assert!(is_authorized_host(&host), "{host} must be on the allowlist");
    }

    /// Privacy rule 1.1 on the shipping client: whatever endpoint a bug names, the
    /// request goes to airplanes.live or nowhere.
    #[tokio::test]
    async fn the_real_client_will_not_fetch_from_an_unauthorized_host() {
        let source = AirplanesLiveSource::build(
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

    // ---- The real airplanes.live ------------------------------------------------------------------

    /// The one test that fetches real aircraft, and the reason the fixtures can be
    /// trusted: they are hand-written to readsb's documented shape, so the mocks prove
    /// we parse what we *believe* airplanes.live sends. The beliefs at risk are exactly
    /// the ones asserted here — that `now` is milliseconds (a wrong scale puts `ts`
    /// thousands of years off), that `alt_baro`/`gs` are feet/knots (unconverted values
    /// land far outside the sane ranges below), and that the field names are what the
    /// docs say (misread names would drop every record and fail the non-empty check).
    ///
    /// Keyless and free, but community-run — run it once after changes here, not in a
    /// loop, and it stays `#[ignore]`d so CI never runs it:
    ///
    /// ```text
    /// cargo test -p look-above-ingest -- --ignored live_airplanes_live
    /// ```
    ///
    /// Nothing here prints a payload — only counts (docs/06).
    #[tokio::test]
    #[ignore = "hits the real airplanes.live API; keyless and free, but be gentle"]
    async fn live_airplanes_live_point_matches_the_documented_shape() {
        let source = AirplanesLiveSource::new(HttpClient::new().expect("client builds"));

        // Switzerland, as in 1.4's live test: reliably busy, and small enough that the
        // covering radius (73 nm) is modest.
        let region = bbox(46.0, 7.0, 48.0, 9.0);
        let states = source
            .fetch(&RegionQuery::region(region))
            .await
            .expect("airplanes.live answers");
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
            // The ms-vs-s belief: a `now` read at the wrong scale dates every position
            // to the year ~56000 or to 1970, both far outside this hour.
            assert!(
                (state.ts.0 - now).abs() < 3600,
                "{} is timestamped {} — {}s from the wall clock; is `now` really ms?",
                state.icao24,
                state.ts,
                (state.ts.0 - now).abs()
            );
            // The unit beliefs: cruise altitude is ~11,000 m and 36,000 *unconverted
            // feet* would fail; fast jets ground-speed ~250 m/s and 450 *unconverted
            // knots* would fail.
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
            "live airplanes.live /v2/point: {} aircraft in {:?}, {} anonymous, {} on the \
             ground, 0 credits (unmetered)",
            states.len(),
            region,
            states.iter().filter(|s| s.anonymous).count(),
            states.iter().filter(|s| s.on_ground).count(),
        );
    }
}
