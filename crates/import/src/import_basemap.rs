//! `import-basemap` — the sanctioned one-off fetch for M2 item 2.2a: Natural Earth 1:50m land
//! and coastline geometry, bundled into `crates/render/assets/basemap/` as `GeoJSON` so `render`
//! never touches the network (ADR-002; docs/03 "Base map geometry" already calls for exactly
//! this: "bundle simplified `GeoJSON` with the app; no runtime fetch").
//!
//! docs/06's network rule sanctions two kinds of live fetch during implementation: running the
//! app, and a purpose-built recorder (`scripts/record_fixture.rs`, M1). This is the same idea
//! for a static, public-domain dataset instead of a live aviation feed — a separate crate
//! because a network+zip+shapefile dependency stack has no business anywhere near `render`'s
//! Cargo.toml, which the M0 gate checks stays network-free (see `DECISION_LOG` M1 item 1.2:
//! "[static-download hosts] are fetched by import tooling at setup time, not by `ingest`
//! ... that tooling extends the list on purpose when it lands" — this is that tooling).
//!
//! **The documented download host is dead.** docs/03 points at
//! `https://www.naturalearthdata.com/downloads/`, but that page's own direct file links
//! (`.../download/50m/physical/ne_50m_land.zip`) 404 — verified, not assumed. Natural Earth's
//! real file host is their CDN, `naciscdn.org` (linked from the same downloads page), confirmed
//! live: both files return `200` with a plausible size (~450 KB each). `ALLOWED_STATIC_HOSTS`
//! is scoped to exactly that host, exact-match, https-only, mirroring `ingest::allowlist`'s
//! rigor even though nothing here ships in the app.
//!
//! Usage: `cargo run -p look-above-import --bin import-basemap`. No arguments, no credentials.
//! Re-run whenever Natural Earth's 1:50m physical vectors are refreshed upstream.

use std::error::Error;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use shapefile::{Polygon, PolygonRing, Polyline, ShapeReader};

/// Natural Earth 1:50m land polygons (public domain, no attribution required).
const LAND_URL: &str = "https://naciscdn.org/naturalearth/50m/physical/ne_50m_land.zip";
/// Natural Earth 1:50m coastline (public domain, no attribution required).
const COASTLINE_URL: &str = "https://naciscdn.org/naturalearth/50m/physical/ne_50m_coastline.zip";

/// Hosts this tool is allowed to fetch from — exact match, https only, the same shape of gate
/// `ingest::allowlist` enforces for live aviation sources (docs/04 rule 1.1's spirit extended
/// to static downloads, which that allowlist deliberately excludes).
const ALLOWED_STATIC_HOSTS: &[&str] = &["naciscdn.org"];

/// Coordinates are rounded to this many places (1e-4 degree, ~11 m at the equator) before
/// writing — far finer than a base map ever needs, but it keeps the bundled `GeoJSON`'s text
/// representation compact by not carrying full `f64` precision noise through to the file.
const COORD_PRECISION: f64 = 10_000.0;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let land = shape_reader(fetch_shp(LAND_URL, "ne_50m_land.shp").await?)?.read_as::<Polygon>()?;
    let land_geojson = land_to_geojson(&land);

    let coastline = shape_reader(fetch_shp(COASTLINE_URL, "ne_50m_coastline.shp").await?)?
        .read_as::<Polyline>()?;
    let coastline_geojson = coastline_to_geojson(&coastline);

    let out_dir = basemap_dir();
    std::fs::create_dir_all(&out_dir)?;
    let land_path = write_geojson(&out_dir.join("land.geojson"), &land_geojson)?;
    let coastline_path = write_geojson(&out_dir.join("coastline.geojson"), &coastline_geojson)?;

    // Counts and paths only — never coordinates (docs/06's "no raw payload in context" rule
    // applies just as much to a 100k-point geometry as to an API response).
    println!(
        "land: {} shapefile record(s) -> {} polygon feature(s) -> {}",
        land.len(),
        feature_count(&land_geojson),
        land_path.display()
    );
    println!(
        "coastline: {} shapefile record(s) -> {} line feature(s) -> {}",
        coastline.len(),
        feature_count(&coastline_geojson),
        coastline_path.display()
    );
    Ok(())
}

/// `crates/render/assets/basemap`, anchored to this crate's manifest rather than the caller's
/// working directory.
fn basemap_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("render")
        .join("assets")
        .join("basemap")
}

/// Refuses anything not on [`ALLOWED_STATIC_HOSTS`] over plain https, before a request is sent.
fn assert_authorized(url: &str) -> Result<(), Box<dyn Error>> {
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" {
        return Err(format!("refusing non-https static download: {url}").into());
    }
    match parsed.host_str() {
        Some(host) if ALLOWED_STATIC_HOSTS.contains(&host) => Ok(()),
        Some(host) => {
            Err(format!("{host} is not on ALLOWED_STATIC_HOSTS — add it deliberately").into())
        }
        None => Err(format!("{url} has no host").into()),
    }
}

/// Downloads `zip_url` (checked against [`ALLOWED_STATIC_HOSTS`] first) and returns the bytes
/// of the single named entry — the `.shp` file, never the whole archive held any longer than
/// it takes to pull one entry out of it.
async fn fetch_shp(zip_url: &str, shp_entry: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    assert_authorized(zip_url)?;
    let zip_bytes = reqwest::get(zip_url)
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes))?;
    let mut entry = archive.by_name(shp_entry)?;
    let mut shp_bytes = Vec::new();
    entry.read_to_end(&mut shp_bytes)?;
    Ok(shp_bytes)
}

/// Wraps raw `.shp` bytes in the crate's reader. No `.shx`/`.dbf` needed: this tool reads every
/// shape once, sequentially, and wants none of the attribute columns.
fn shape_reader(shp_bytes: Vec<u8>) -> Result<ShapeReader<Cursor<Vec<u8>>>, Box<dyn Error>> {
    Ok(ShapeReader::new(Cursor::new(shp_bytes))?)
}

/// Rounds one coordinate to [`COORD_PRECISION`].
fn round_coord(value: f64) -> f64 {
    (value * COORD_PRECISION).round() / COORD_PRECISION
}

/// One ring's points as a `GeoJSON` coordinate array (`[[lon, lat], ...]`).
fn ring_coords(points: &[shapefile::Point]) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|p| [round_coord(p.x), round_coord(p.y)])
        .collect()
}

/// Land polygons -> a `GeoJSON` `FeatureCollection` of `Polygon` geometries.
///
/// A shapefile `Polygon` record may hold several unconnected outer rings (e.g. a continent
/// plus its islands, packed into one record) — `GeoJSON`'s `Polygon` type allows exactly one, so
/// each outer ring starts a new output feature, and inner (hole) rings attach to whichever
/// outer ring immediately precedes them. That is the shapefile-writing convention (rings for
/// one polygon are written together, hole immediately after its shell) rather than a guarantee
/// the format makes, but it is what every common shapefile writer, including Natural Earth's
/// own toolchain, actually produces.
fn land_to_geojson(polygons: &[Polygon]) -> Value {
    let mut features = Vec::new();
    let mut current: Option<Vec<Vec<[f64; 2]>>> = None;

    for polygon in polygons {
        for ring in polygon.rings() {
            match ring {
                PolygonRing::Outer(points) => {
                    if let Some(rings) = current.take() {
                        features.push(polygon_feature(&rings));
                    }
                    current = Some(vec![ring_coords(points)]);
                }
                PolygonRing::Inner(points) => {
                    // An inner ring with no preceding outer ring is orphaned data; there is no
                    // sensible feature to attach it to, so it is dropped rather than guessed at.
                    if let Some(rings) = current.as_mut() {
                        rings.push(ring_coords(points));
                    }
                }
            }
        }
        if let Some(rings) = current.take() {
            features.push(polygon_feature(&rings));
        }
    }

    feature_collection(&features)
}

/// Coastline polylines -> a `GeoJSON` `FeatureCollection` of `LineString` geometries, one per
/// shapefile part (coastlines have no outer/inner distinction, just disjoint chains).
fn coastline_to_geojson(polylines: &[Polyline]) -> Value {
    let features: Vec<Value> = polylines
        .iter()
        .flat_map(Polyline::parts)
        .map(|part| {
            json!({
                "type": "Feature",
                "properties": {},
                "geometry": { "type": "LineString", "coordinates": ring_coords(part) }
            })
        })
        .collect();

    feature_collection(&features)
}

fn polygon_feature(rings: &[Vec<[f64; 2]>]) -> Value {
    json!({
        "type": "Feature",
        "properties": {},
        "geometry": { "type": "Polygon", "coordinates": rings }
    })
}

fn feature_collection(features: &[Value]) -> Value {
    json!({ "type": "FeatureCollection", "features": features })
}

fn feature_count(collection: &Value) -> usize {
    collection["features"].as_array().map_or(0, Vec::len)
}

/// Compact (not pretty) JSON — this is a bundled asset read by the app at startup, not a
/// fixture meant for a human to read, and pretty-printing a few hundred thousand coordinates
/// would bloat the file many times over for no benefit.
fn write_geojson(path: &Path, value: &Value) -> Result<PathBuf, Box<dyn Error>> {
    std::fs::write(path, serde_json::to_vec(value)?)?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use shapefile::Point;

    use super::*;

    // ---- Host gate ----------------------------------------------------------------------

    #[test]
    fn the_real_download_host_is_authorized_over_https() {
        assert!(assert_authorized(LAND_URL).is_ok());
        assert!(assert_authorized(COASTLINE_URL).is_ok());
    }

    #[test]
    fn an_unlisted_host_is_refused() {
        let error = assert_authorized("https://evil-naciscdn.org/x.zip")
            .expect_err("lookalike host must be refused")
            .to_string();
        assert!(error.contains("evil-naciscdn.org"), "{error}");
    }

    #[test]
    fn plain_http_is_refused_even_for_an_authorized_host() {
        assert!(assert_authorized("http://naciscdn.org/x.zip").is_err());
    }

    // ---- Coordinate rounding --------------------------------------------------------------

    #[test]
    fn coordinates_round_to_the_configured_precision() {
        assert!((round_coord(12.345_678_9) - 12.3457).abs() < 1e-9);
        assert!((round_coord(-8.000_04) - (-8.0)).abs() < 1e-9);
    }

    // ---- Polygon conversion -----------------------------------------------------------------

    /// A single outer ring, no holes: one shapefile record becomes one `GeoJSON` `Polygon`
    /// feature with exactly one ring.
    #[test]
    fn a_simple_outer_ring_becomes_one_polygon_feature() {
        let square = Polygon::new(PolygonRing::Outer(vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0),
            Point::new(1.0, 0.0),
        ]));

        let geojson = land_to_geojson(&[square]);
        let features = geojson["features"].as_array().expect("features array");
        assert_eq!(features.len(), 1);
        assert_eq!(features[0]["geometry"]["type"], "Polygon");
        let rings = features[0]["geometry"]["coordinates"]
            .as_array()
            .expect("coordinates array");
        assert_eq!(rings.len(), 1, "no holes means a single ring");
    }

    /// An inner ring immediately after an outer one becomes a hole in *that* feature, not a
    /// feature of its own — this is the whole point of the grouping heuristic.
    #[test]
    fn an_inner_ring_attaches_to_the_preceding_outer_ring() {
        let with_hole = Polygon::with_rings(vec![
            PolygonRing::Outer(vec![
                Point::new(-10.0, -10.0),
                Point::new(-10.0, 10.0),
                Point::new(10.0, 10.0),
                Point::new(10.0, -10.0),
            ]),
            PolygonRing::Inner(vec![
                Point::new(-1.0, -1.0),
                Point::new(1.0, -1.0),
                Point::new(1.0, 1.0),
                Point::new(-1.0, 1.0),
            ]),
        ]);

        let geojson = land_to_geojson(&[with_hole]);
        let features = geojson["features"].as_array().expect("features array");
        assert_eq!(features.len(), 1, "outer + hole is one feature");
        let rings = features[0]["geometry"]["coordinates"]
            .as_array()
            .expect("coordinates array");
        assert_eq!(rings.len(), 2, "shell plus one hole");
    }

    /// A record with two disjoint outer rings (a continent plus a separate island) splits into
    /// two `GeoJSON` `Polygon` features — `GeoJSON`'s `Polygon` cannot hold two unconnected shells.
    #[test]
    fn two_outer_rings_in_one_record_become_two_features() {
        let mainland_and_island = Polygon::with_rings(vec![
            PolygonRing::Outer(vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, 0.0),
            ]),
            PolygonRing::Outer(vec![
                Point::new(20.0, 20.0),
                Point::new(20.0, 21.0),
                Point::new(21.0, 21.0),
                Point::new(21.0, 20.0),
            ]),
        ]);

        let geojson = land_to_geojson(&[mainland_and_island]);
        assert_eq!(feature_count(&geojson), 2);
    }

    /// Every ring the shapefile crate hands back is already closed (first point == last), and
    /// that closure must survive the conversion — a fill tessellator downstream depends on it.
    #[test]
    fn rings_stay_closed_after_conversion() {
        let square = Polygon::new(PolygonRing::Outer(vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0),
            Point::new(1.0, 0.0),
        ]));

        let geojson = land_to_geojson(&[square]);
        let ring = geojson["features"][0]["geometry"]["coordinates"][0]
            .as_array()
            .expect("ring coordinates");
        assert_eq!(
            ring.first(),
            ring.last(),
            "shapefile crate auto-closes rings"
        );
    }

    // ---- Polyline conversion ----------------------------------------------------------------

    /// One shapefile part -> one `LineString` feature, coordinates preserved in order.
    #[test]
    fn a_single_part_polyline_becomes_one_linestring_feature() {
        let coast = Polyline::new(vec![
            Point::new(-5.0, 40.0),
            Point::new(-4.5, 40.2),
            Point::new(-4.0, 40.5),
        ]);

        let geojson = coastline_to_geojson(&[coast]);
        let features = geojson["features"].as_array().expect("features array");
        assert_eq!(features.len(), 1);
        assert_eq!(features[0]["geometry"]["type"], "LineString");
        let coords = features[0]["geometry"]["coordinates"]
            .as_array()
            .expect("coordinates array");
        assert_eq!(coords.len(), 3);
    }

    /// A multi-part polyline (e.g. a mainland coast plus a nearby island's) becomes one
    /// `LineString` feature per part — each part is its own disjoint chain.
    #[test]
    fn each_polyline_part_becomes_its_own_feature() {
        let two_coasts = Polyline::with_parts(vec![
            vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)],
            vec![
                Point::new(10.0, 10.0),
                Point::new(11.0, 11.0),
                Point::new(12.0, 10.0),
            ],
        ]);

        let geojson = coastline_to_geojson(&[two_coasts]);
        assert_eq!(feature_count(&geojson), 2);
    }
}
