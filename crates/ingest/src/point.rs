//! The point-and-radius query the keyless readsb fallbacks share.
//!
//! airplanes.live (1.5) and adsb.lol (1.6) expose the *same* request shape —
//! `/v2/point/{lat}/{lon}/{radius_nm}`, radius ≤ 250 nm — over the same readsb JSON
//! ([`crate::readsb`]). The parser was shared from the first adapter; the second proves the
//! *request* side is identical too, so it lives here rather than being copied per adapter:
//! turning a [`RegionQuery`] bbox into the smallest covering circle, clamping it to the
//! documented ceiling, spacing the call, and trimming the circular reply back to the
//! rectangular contract 1.9's merge assumes ("every source answers the same question").
//!
//! What stays per-adapter is what actually differs: the host, the [`SourceId`] stamp, the
//! request spacing, the fixtures, and the live check — because docs/09 keeps one adapter per
//! source so their independent drift is caught independently. A [`PointSource`] is the shared
//! machinery those thin adapters delegate to.

use std::time::Duration;

use look_above_core::contracts::RegionQuery;
use look_above_core::error::SourceError;
use look_above_core::geo::{LatLon, haversine_distance_m};
use look_above_core::types::{BBox, SourceId, StateVector};

use crate::http::{HttpClient, send_json};
use crate::pacer::Pacer;
use crate::readsb::{METRES_PER_NAUTICAL_MILE, PointResponse};

/// The documented ceiling on the radius parameter, in nautical miles — the same 250 for
/// both fallbacks (authorized-aviation-sources skill).
pub const MAX_RADIUS_NM: u32 = 250;

/// A keyless readsb point-query source: everything both fallbacks do identically, given a
/// host to reach, an identity to stamp, and a spacing to honor.
#[derive(Debug)]
pub struct PointSource {
    client: HttpClient,
    pacer: Pacer,
    endpoint: String,
    source: SourceId,
}

impl PointSource {
    /// `endpoint` is the `/v2/point` base (the `/{lat}/{lon}/{radius}` tail is appended
    /// here); `spacing` is the minimum gap between requests the source asks for, paid in
    /// time by the [`Pacer`].
    pub fn new(client: HttpClient, endpoint: String, source: SourceId, spacing: Duration) -> Self {
        Self {
            client,
            pacer: Pacer::new(spacing),
            endpoint,
            source,
        }
    }

    /// The request spacing this source paces at — what an adapter's wiring test asserts.
    pub fn spacing(&self) -> Duration {
        self.pacer.interval()
    }

    /// One bbox query → the covering circle → the parsed, bbox-trimmed batch.
    pub async fn fetch(&self, query: &RegionQuery) -> Result<Vec<StateVector>, SourceError> {
        // A point/radius endpoint cannot be asked for the world, and pretending — a max
        // radius circle around an arbitrary point — would be a confidently wrong answer.
        // Global polling is M4's problem; `Refused` is not transient, so the poller moves
        // on rather than retrying.
        let Some(bbox) = query.bbox else {
            return Err(SourceError::Refused {
                reason: format!(
                    "{} answers a point and radius, not a global query \
                     (global polling is deferred to M4)",
                    self.source
                ),
            });
        };

        let circle = covering_circle(bbox);
        if circle.truncated {
            // Partial coverage beats refusing to fail over, but it must not be silent:
            // aircraft near the box's corners will be missing until the region shrinks.
            tracing::warn!(
                source = %self.source,
                radius_nm = circle.radius_nm,
                "bbox needs a larger radius than the source's 250 nm ceiling; \
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
        let mut states = response.into_state_vectors(self.source);
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
    use look_above_core::types::Icao24;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use super::*;
    use crate::allowlist::HostPolicy;
    use crate::http::{REQUEST_TIMEOUT, USER_AGENT};

    fn bbox(lat_min: f64, lon_min: f64, lat_max: f64, lon_max: f64) -> BBox {
        BBox::new(lat_min, lon_min, lat_max, lon_max).expect("valid bbox in test")
    }

    /// Switzerland, the region every adapter's tests use: center (47, 8), radius 73 nm.
    fn a_region() -> RegionQuery {
        RegionQuery::region(bbox(46.0, 7.0, 48.0, 9.0))
    }

    /// The real client, widened to reach a loopback mock — the same escape hatch every
    /// adapter test uses, so the shipping User-Agent, timeout, and allowlist are what these
    /// assertions run through.
    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    fn source_against(server: &MockServer) -> PointSource {
        PointSource::new(
            client(),
            format!("{}/v2/point", server.uri()),
            SourceId::AirplanesLive,
            Duration::from_secs(2),
        )
    }

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

    // ---- The covering circle, as pure geometry ---------------------------------------------------

    /// The farthest corners of (46..48, 7..9) from center (47, 8) are the southern pair —
    /// haversine gives 134,988 m = 72.9 nm to (46, 7) — so the ceil is 73 and the center is
    /// the midpoint. A lat/lon swap in the center would show here first.
    #[test]
    fn covering_circle_uses_the_midpoint_and_the_farthest_corner() {
        let circle = covering_circle(bbox(46.0, 7.0, 48.0, 9.0));
        assert_eq!((circle.center.lat_deg, circle.center.lon_deg), (47.0, 8.0));
        assert_eq!(
            circle.radius_nm, 73,
            "ceil(72.9 nm), circumscribing the box"
        );
        assert!(!circle.truncated);
    }

    /// A box wider than 500 nm across is clamped to the documented ceiling, flagged so the
    /// caller can warn — not refused and not sent oversized.
    #[test]
    fn covering_circle_clamps_to_the_documented_maximum() {
        // (40..52, -2..16): the farthest corner is ~534 nm from center (46, 7).
        let circle = covering_circle(bbox(40.0, -2.0, 52.0, 16.0));
        assert_eq!(circle.radius_nm, MAX_RADIUS_NM);
        assert!(circle.truncated);
    }

    /// A degenerate (point) bbox still queries: radius floors at 1 nm, never 0.
    #[test]
    fn covering_circle_floors_a_degenerate_bbox_to_one_mile() {
        let circle = covering_circle(bbox(50.0, 8.0, 50.0, 8.0));
        assert_eq!((circle.center.lat_deg, circle.center.lon_deg), (50.0, 8.0));
        assert_eq!(circle.radius_nm, 1);
        assert!(!circle.truncated);
    }

    // ---- The request on the wire -----------------------------------------------------------------

    /// docs/10 §2: the covering circle reaches the URL intact, four decimals, whole-mile
    /// radius, through the shipping client (User-Agent asserted).
    #[tokio::test]
    async fn a_bbox_becomes_the_covering_point_and_radius_on_the_wire() {
        let server = MockServer::start().await;
        mock_point(&server, r#"{"ac": [], "now": 1721000000000}"#).await;

        source_against(&server)
            .fetch(&a_region())
            .await
            .expect("the fetch succeeds");

        let request = only_request(&server).await;
        let (lat, lon, radius) = point_segments(&request);
        assert_eq!(
            (lat.as_str(), lon.as_str(), radius.as_str()),
            ("47.0000", "8.0000", "73")
        );
        assert_eq!(
            request.headers["user-agent"], USER_AGENT,
            "the shared source must not bypass the client"
        );
    }

    /// The endpoint serves a circle; the contract is a bbox. Aircraft in the circle but
    /// outside the box must be trimmed, or 1.9's merge compares different regions per source.
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

        let states = source_against(&server)
            .fetch(&a_region())
            .await
            .expect("the fetch succeeds");
        assert_eq!(states.len(), 1);
        assert_eq!(
            states[0].icao24,
            Icao24::from_hex("3c6444").expect("valid hex"),
            "only the aircraft inside the bbox survives"
        );
    }

    /// No request leaves for a global query — not even to be told no — and the refusal is
    /// not transient, so the poller moves on rather than retrying.
    #[tokio::test]
    async fn a_global_query_is_refused_without_sending_anything() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let error = source_against(&server)
            .fetch(&RegionQuery::GLOBAL)
            .await
            .expect_err("a point endpoint cannot answer a global query");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(reason.contains("global"), "{reason}");
        assert!(
            reason.contains("airplaneslive"),
            "names the source: {reason}"
        );
        assert!(!error.is_transient());
        // expect(0): asserted on drop.
    }
}
