//! Great-circle math and map projections.
//!
//! Everything here is pure, spherical, and in SI units. A sphere is accurate to
//! ~0.5% against the WGS84 ellipsoid — far below the position error of the feeds
//! themselves — and it keeps dead reckoning cheap enough to run over every
//! tracked aircraft each frame.
//!
//! Two radii appear below and they are not interchangeable: great-circle math
//! uses the *mean* Earth radius, while Web Mercator is *defined* on the WGS84
//! semi-major axis (docs/01: L1/L2 render in Web Mercator).

use std::f64::consts::PI;

/// Mean Earth radius (IUGG R₁ = (2a + b)/3 for WGS84), in metres.
///
/// Used for all great-circle work: distance, bearing, dead reckoning.
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// The WGS84 semi-major axis, in metres — the sphere radius `EPSG:3857` is
/// defined on.
///
/// Not the mean radius: Web Mercator's definition mandates this value, and using
/// the mean radius instead would shift every projected position by ~0.1%.
pub const WEB_MERCATOR_RADIUS_M: f64 = 6_378_137.0;

/// The latitude where Web Mercator's y equals its x extent, in degrees.
///
/// Beyond this the projection runs to infinity at the poles, so the standard
/// square domain cuts off here; [`web_mercator_forward`] clamps to it.
pub const MAX_MERCATOR_LAT_DEG: f64 = 85.051_128_779_806_59;

/// Half the width of the projected world, in metres: `WEB_MERCATOR_RADIUS_M * π`.
///
/// The familiar `20037508.34` — the projected square runs ±this on both axes.
pub const WEB_MERCATOR_EXTENT_M: f64 = WEB_MERCATOR_RADIUS_M * PI;

/// A geographic position in degrees.
///
/// A struct rather than an `(f64, f64)` pair because latitude/longitude
/// transposition is the classic silent bug in this kind of code: it produces a
/// plausible position somewhere else on Earth rather than an error.
///
/// Unvalidated by construction — feeds are the source of these and validation
/// belongs at the parse boundary (M1), not in the hot path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    pub lat_deg: f64,
    pub lon_deg: f64,
}

impl LatLon {
    pub const fn new(lat_deg: f64, lon_deg: f64) -> Self {
        Self { lat_deg, lon_deg }
    }
}

/// A projected position in Web Mercator metres (`EPSG:3857`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MercatorXy {
    pub x_m: f64,
    pub y_m: f64,
}

impl MercatorXy {
    pub const fn new(x_m: f64, y_m: f64) -> Self {
        Self { x_m, y_m }
    }
}

/// Wraps a longitude into `[-180, 180)`. Exactly 180° normalizes to -180°.
pub fn normalize_lon_deg(lon_deg: f64) -> f64 {
    (lon_deg + 180.0).rem_euclid(360.0) - 180.0
}

/// Wraps a bearing into `[0, 360)`.
pub fn normalize_bearing_deg(bearing_deg: f64) -> f64 {
    bearing_deg.rem_euclid(360.0)
}

/// Great-circle distance in metres (haversine).
///
/// Haversine rather than the spherical law of cosines: it stays accurate for the
/// small separations that dominate here (consecutive fixes of one aircraft are
/// often < 1 km apart), where the cosine formula loses precision badly.
pub fn haversine_distance_m(from: LatLon, to: LatLon) -> f64 {
    let phi1 = from.lat_deg.to_radians();
    let phi2 = to.lat_deg.to_radians();
    let delta_phi = (to.lat_deg - from.lat_deg).to_radians();
    let delta_lambda = (to.lon_deg - from.lon_deg).to_radians();

    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    // `a` is analytically within [0, 1]; rounding can push it a hair past 1 for
    // antipodal points, which would make the `1 - a` square root NaN.
    let a = a.clamp(0.0, 1.0);

    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    EARTH_RADIUS_M * c
}

/// Initial bearing (forward azimuth) in degrees clockwise from true north, `[0, 360)`.
///
/// "Initial" is the operative word: along a great circle the bearing changes
/// continuously, so this is the heading at `from` only, and the reverse trip's
/// bearing is not generally this ± 180°.
pub fn initial_bearing_deg(from: LatLon, to: LatLon) -> f64 {
    let phi1 = from.lat_deg.to_radians();
    let phi2 = to.lat_deg.to_radians();
    let delta_lambda = (to.lon_deg - from.lon_deg).to_radians();

    let y = delta_lambda.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lambda.cos();

    normalize_bearing_deg(y.atan2(x).to_degrees())
}

/// The point reached by travelling `distance_m` from `from` along `bearing_deg`.
///
/// This is the dead-reckoning step: with `distance_m = ground_speed * elapsed`
/// it advances an aircraft along its track between sparse fixes.
pub fn destination_point(from: LatLon, bearing_deg: f64, distance_m: f64) -> LatLon {
    let phi1 = from.lat_deg.to_radians();
    let lambda1 = from.lon_deg.to_radians();
    let theta = bearing_deg.to_radians();
    // Angular distance travelled.
    let delta = distance_m / EARTH_RADIUS_M;

    let sin_phi2 = phi1.sin() * delta.cos() + phi1.cos() * delta.sin() * theta.cos();
    let phi2 = sin_phi2.clamp(-1.0, 1.0).asin();

    let y = theta.sin() * delta.sin() * phi1.cos();
    let x = delta.cos() - phi1.sin() * sin_phi2;
    let lambda2 = lambda1 + y.atan2(x);

    LatLon::new(phi2.to_degrees(), normalize_lon_deg(lambda2.to_degrees()))
}

/// Projects a position to Web Mercator metres (`EPSG:3857`).
///
/// Latitude is clamped to ±[`MAX_MERCATOR_LAT_DEG`] rather than returning an
/// error: the projection is only undefined at the poles, and a camera panned to
/// the top of the map should show the map's edge, not fail.
pub fn web_mercator_forward(position: LatLon) -> MercatorXy {
    let lat = position
        .lat_deg
        .clamp(-MAX_MERCATOR_LAT_DEG, MAX_MERCATOR_LAT_DEG);

    // y = R·ln(tan(π/4 + φ/2)), written as the equivalent R·artanh(sin φ) — the
    // inverse Gudermannian — which avoids tan() blowing up near the limit.
    MercatorXy::new(
        WEB_MERCATOR_RADIUS_M * position.lon_deg.to_radians(),
        WEB_MERCATOR_RADIUS_M * lat.to_radians().sin().atanh(),
    )
}

/// Unprojects Web Mercator metres back to a position.
///
/// Exact inverse of [`web_mercator_forward`] within the clamped latitude domain.
pub fn web_mercator_inverse(point: MercatorXy) -> LatLon {
    LatLon::new(
        (point.y_m / WEB_MERCATOR_RADIUS_M)
            .tanh()
            .asin()
            .to_degrees(),
        (point.x_m / WEB_MERCATOR_RADIUS_M).to_degrees(),
    )
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

    use super::*;

    /// Metres. Generous next to Earth's radius, tight next to any real position error.
    const DIST_EPS_M: f64 = 0.5;
    /// Degrees. docs/10 §1 requires round-trips within 1e-9°.
    const DEG_EPS: f64 = 1e-9;

    #[track_caller]
    fn assert_close(actual: f64, expected: f64, eps: f64) {
        assert!(
            (actual - expected).abs() <= eps,
            "expected {expected}, got {actual} (difference {}, tolerance {eps})",
            (actual - expected).abs()
        );
    }

    /// Compares bearings on the circle, so 359.9999° and 0.0001° are close.
    #[track_caller]
    fn assert_bearing_close(actual: f64, expected: f64, eps: f64) {
        let diff = (actual - expected + 180.0).rem_euclid(360.0) - 180.0;
        assert!(
            diff.abs() <= eps,
            "expected bearing {expected}°, got {actual}° (difference {diff}°, tolerance {eps}°)"
        );
    }

    const LAX: LatLon = LatLon::new(33.9425, -118.4081);
    const JFK: LatLon = LatLon::new(40.6398, -73.7789);

    // --- Distance: pinned against arcs whose length is known analytically ------

    #[test]
    fn distance_from_a_point_to_itself_is_zero() {
        assert_close(haversine_distance_m(LAX, LAX), 0.0, f64::EPSILON);
    }

    #[test]
    fn quarter_of_the_equator_is_a_quarter_circumference() {
        let d = haversine_distance_m(LatLon::new(0.0, 0.0), LatLon::new(0.0, 90.0));
        assert_close(d, EARTH_RADIUS_M * FRAC_PI_2, DIST_EPS_M);
    }

    #[test]
    fn pole_to_pole_is_half_a_circumference() {
        let d = haversine_distance_m(LatLon::new(-90.0, 0.0), LatLon::new(90.0, 0.0));
        assert_close(d, EARTH_RADIUS_M * PI, DIST_EPS_M);
    }

    #[test]
    fn antipodal_equator_points_are_half_a_circumference() {
        // The case that makes the law-of-cosines formula fall apart, and where
        // `a` can round above 1.0 — must not produce NaN.
        let d = haversine_distance_m(LatLon::new(0.0, 0.0), LatLon::new(0.0, 180.0));
        assert!(d.is_finite());
        assert_close(d, EARTH_RADIUS_M * PI, DIST_EPS_M);
    }

    #[test]
    fn one_degree_of_latitude_is_one_arcdegree_of_meridian() {
        let d = haversine_distance_m(LatLon::new(0.0, 0.0), LatLon::new(1.0, 0.0));
        assert_close(d, EARTH_RADIUS_M * PI / 180.0, DIST_EPS_M);
    }

    #[test]
    fn distance_is_symmetric() {
        assert_close(
            haversine_distance_m(LAX, JFK),
            haversine_distance_m(JFK, LAX),
            DIST_EPS_M,
        );
    }

    #[test]
    fn lax_to_jfk_matches_the_published_great_circle_distance() {
        // The Great Circle Mapper publishes 2,145 nm for this pair. Asserted in
        // nautical miles because that is the unit the figure is published in —
        // converting the reference rather than our answer keeps the comparison
        // honest. Tolerance covers its rounding to whole nm plus the ~100 m
        // spread in published airport reference points.
        //
        // Note this is a cross-check, not the proof: the analytic arcs above
        // (quarter equator, pole to pole, one meridian degree) are what actually
        // pin the formula, since their lengths follow from geometry rather than
        // from a table someone typed.
        let nm = haversine_distance_m(LAX, JFK) / 1_852.0;
        assert_close(nm, 2_145.0, 2.0);
    }

    // --- Bearing: cardinal directions are exactly known ------------------------

    #[test]
    fn bearings_along_the_cardinal_directions_are_exact() {
        let origin = LatLon::new(0.0, 0.0);
        assert_bearing_close(
            initial_bearing_deg(origin, LatLon::new(10.0, 0.0)),
            0.0,
            1e-9,
        );
        assert_bearing_close(
            initial_bearing_deg(origin, LatLon::new(0.0, 10.0)),
            90.0,
            1e-9,
        );
        assert_bearing_close(
            initial_bearing_deg(origin, LatLon::new(-10.0, 0.0)),
            180.0,
            1e-9,
        );
        assert_bearing_close(
            initial_bearing_deg(origin, LatLon::new(0.0, -10.0)),
            270.0,
            1e-9,
        );
    }

    #[test]
    fn bearing_is_always_normalized_into_zero_to_360() {
        for lat in [-80.0, -10.0, 0.0, 10.0, 80.0] {
            for lon in [-179.0, -90.0, 0.0, 90.0, 179.0] {
                let b = initial_bearing_deg(LatLon::new(5.0, 5.0), LatLon::new(lat, lon));
                assert!((0.0..360.0).contains(&b), "bearing {b} out of range");
            }
        }
    }

    #[test]
    fn every_bearing_points_north_from_anywhere_toward_the_pole() {
        for lon in [-180.0, -90.0, 0.0, 45.0, 179.0] {
            let from = LatLon::new(10.0, lon);
            assert_bearing_close(initial_bearing_deg(from, LatLon::new(90.0, 0.0)), 0.0, 1e-9);
            assert_bearing_close(
                initial_bearing_deg(from, LatLon::new(-90.0, 0.0)),
                180.0,
                1e-9,
            );
        }
    }

    #[test]
    fn reverse_bearing_flips_by_180_along_a_meridian() {
        // True on a meridian (and the equator) specifically — a great circle's
        // bearing is not generally reversible this way.
        let a = LatLon::new(10.0, 20.0);
        let b = LatLon::new(50.0, 20.0);
        assert_bearing_close(initial_bearing_deg(a, b), 0.0, 1e-9);
        assert_bearing_close(initial_bearing_deg(b, a), 180.0, 1e-9);
    }

    #[test]
    fn bearing_across_the_antimeridian_points_east_not_the_long_way_round() {
        // The wrap bug: 179°E → 179°W is 2° east, not 358° west.
        let b = initial_bearing_deg(LatLon::new(0.0, 179.0), LatLon::new(0.0, -179.0));
        assert_bearing_close(b, 90.0, 1e-9);
    }

    // --- Destination point ----------------------------------------------------

    #[test]
    fn destination_travels_the_requested_distance_on_the_requested_bearing() {
        let start = LatLon::new(51.4775, -0.4614);
        for bearing in [0.0, 45.0, 90.0, 137.5, 180.0, 271.3, 359.0] {
            for distance in [1.0, 1_000.0, 100_000.0, 5_000_000.0] {
                let end = destination_point(start, bearing, distance);
                assert_close(haversine_distance_m(start, end), distance, DIST_EPS_M);
                assert_bearing_close(initial_bearing_deg(start, end), bearing, 1e-6);
            }
        }
    }

    #[test]
    fn destination_due_north_only_changes_latitude() {
        let start = LatLon::new(0.0, 30.0);
        let end = destination_point(start, 0.0, EARTH_RADIUS_M * PI / 180.0);
        assert_close(end.lat_deg, 1.0, 1e-9);
        assert_close(end.lon_deg, 30.0, 1e-9);
    }

    #[test]
    fn destination_of_zero_distance_stays_put() {
        let start = LatLon::new(33.9425, -118.4081);
        let end = destination_point(start, 42.0, 0.0);
        assert_close(end.lat_deg, start.lat_deg, 1e-12);
        assert_close(end.lon_deg, start.lon_deg, 1e-12);
    }

    #[test]
    fn destination_wraps_across_the_antimeridian() {
        // Flying east from 179°E must land just past -180°, not at 181°.
        let start = LatLon::new(0.0, 179.0);
        let end = destination_point(start, 90.0, EARTH_RADIUS_M * 2.0 * PI / 180.0);
        assert!(
            (-180.0..180.0).contains(&end.lon_deg),
            "longitude {} escaped [-180, 180)",
            end.lon_deg
        );
        assert_close(end.lon_deg, -179.0, 1e-9);
    }

    #[test]
    fn destination_crossing_the_pole_stays_a_valid_position() {
        // Due north from 80°N for 2000 km passes over the pole and comes down
        // the far side — latitude must fold back, longitude must flip ~180°.
        let start = LatLon::new(80.0, 0.0);
        let end = destination_point(start, 0.0, 2_000_000.0);
        assert!(
            (-90.0..=90.0).contains(&end.lat_deg),
            "latitude {} escaped [-90, 90]",
            end.lat_deg
        );
        assert!(
            (-180.0..180.0).contains(&end.lon_deg),
            "longitude {} escaped [-180, 180)",
            end.lon_deg
        );
        assert_bearing_close(end.lon_deg, 180.0, 1e-6);
    }

    // --- Web Mercator ---------------------------------------------------------

    #[test]
    fn mercator_origin_projects_to_the_origin() {
        let p = web_mercator_forward(LatLon::new(0.0, 0.0));
        assert_close(p.x_m, 0.0, 1e-9);
        assert_close(p.y_m, 0.0, 1e-9);
    }

    #[test]
    fn mercator_antimeridian_projects_to_the_published_extent() {
        // The defining constant of EPSG:3857: R·π = 20037508.342789244.
        assert_close(WEB_MERCATOR_EXTENT_M, 20_037_508.342_789_244, 1e-6);
        let p = web_mercator_forward(LatLon::new(0.0, 180.0));
        assert_close(p.x_m, 20_037_508.342_789_244, 1e-6);
    }

    #[test]
    fn mercator_latitude_limit_squares_the_world() {
        // The whole reason 85.0511° is the cutoff: it makes y match the x extent.
        let p = web_mercator_forward(LatLon::new(MAX_MERCATOR_LAT_DEG, 180.0));
        assert_close(p.y_m, WEB_MERCATOR_EXTENT_M, 1e-3);
        assert_close(p.x_m, WEB_MERCATOR_EXTENT_M, 1e-6);
    }

    #[test]
    fn mercator_45_north_matches_its_published_value() {
        // R·ln(tan(π/4 + π/8)) — the standard check value for EPSG:3857.
        let p = web_mercator_forward(LatLon::new(45.0, 0.0));
        assert_close(p.y_m, 5_621_521.486_192_066, 1e-6);
    }

    #[test]
    fn mercator_is_symmetric_about_the_equator_and_prime_meridian() {
        let north = web_mercator_forward(LatLon::new(30.0, 40.0));
        let south = web_mercator_forward(LatLon::new(-30.0, -40.0));
        assert_close(north.y_m, -south.y_m, 1e-9);
        assert_close(north.x_m, -south.x_m, 1e-9);
    }

    #[test]
    fn mercator_clamps_the_poles_instead_of_running_to_infinity() {
        for lat in [90.0, -90.0, 89.9, -89.9] {
            let p = web_mercator_forward(LatLon::new(lat, 0.0));
            assert!(p.y_m.is_finite(), "latitude {lat} projected to {}", p.y_m);
            assert!(p.y_m.abs() <= WEB_MERCATOR_EXTENT_M + 1e-6);
        }
        assert_close(
            web_mercator_forward(LatLon::new(90.0, 0.0)).y_m,
            WEB_MERCATOR_EXTENT_M,
            1e-3,
        );
    }

    #[test]
    fn mercator_round_trips_within_a_nanodegree() {
        // docs/10 §1: inverse(forward(p)) ≈ p within 1e-9°. Deterministic sweep
        // over the projection's domain, including its corners.
        let mut checked = 0;
        let mut lat = -85.0;
        while lat <= 85.0 {
            let mut lon = -180.0;
            while lon < 180.0 {
                let original = LatLon::new(lat, lon);
                let round_tripped = web_mercator_inverse(web_mercator_forward(original));
                assert_close(round_tripped.lat_deg, lat, DEG_EPS);
                assert_close(round_tripped.lon_deg, lon, DEG_EPS);
                checked += 1;
                lon += 7.5;
            }
            lat += 5.0;
        }
        assert!(checked > 1_000, "sweep only covered {checked} points");
    }

    #[test]
    fn mercator_round_trips_at_the_latitude_limit() {
        for lat in [MAX_MERCATOR_LAT_DEG, -MAX_MERCATOR_LAT_DEG] {
            let round_tripped =
                web_mercator_inverse(web_mercator_forward(LatLon::new(lat, 179.999)));
            assert_close(round_tripped.lat_deg, lat, DEG_EPS);
            assert_close(round_tripped.lon_deg, 179.999, DEG_EPS);
        }
    }

    // --- Normalization --------------------------------------------------------

    #[test]
    fn longitude_normalization_wraps_into_a_half_open_range() {
        assert_close(normalize_lon_deg(0.0), 0.0, 1e-12);
        assert_close(normalize_lon_deg(181.0), -179.0, 1e-12);
        assert_close(normalize_lon_deg(-181.0), 179.0, 1e-12);
        assert_close(normalize_lon_deg(540.0), -180.0, 1e-12);
        // The half-open boundary: +180 folds onto -180, they are the same meridian.
        assert_close(normalize_lon_deg(180.0), -180.0, 1e-12);
        assert_close(normalize_lon_deg(-180.0), -180.0, 1e-12);
    }

    #[test]
    fn bearing_normalization_wraps_into_zero_to_360() {
        assert_close(normalize_bearing_deg(0.0), 0.0, 1e-12);
        assert_close(normalize_bearing_deg(360.0), 0.0, 1e-12);
        assert_close(normalize_bearing_deg(-90.0), 270.0, 1e-12);
        assert_close(normalize_bearing_deg(450.0), 90.0, 1e-12);
    }

    #[test]
    fn frac_pi_4_is_used_by_the_documented_mercator_identity() {
        // R·ln(tan(π/4 + φ/2)) is the textbook form; the implementation uses the
        // equivalent artanh(sin φ). Pin the equivalence so a future edit to
        // either form has to keep them agreeing.
        for lat in [-80.0, -45.0, -1.0, 0.0, 1.0, 45.0, 80.0] {
            let phi = f64::to_radians(lat);
            let textbook = WEB_MERCATOR_RADIUS_M * (FRAC_PI_4 + phi / 2.0).tan().ln();
            let actual = web_mercator_forward(LatLon::new(lat, 0.0)).y_m;
            assert_close(actual, textbook, 1e-6);
        }
    }
}
