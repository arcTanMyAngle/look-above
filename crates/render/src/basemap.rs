//! Static tessellation of the bundled base-map `GeoJSON` (M2 item 2.2b).
//!
//! Runs once, at startup: [`tessellate`] turns `assets/basemap/{land,coastline}.geojson` —
//! embedded via `include_str!` so `render` never touches the filesystem or network at runtime
//! (ADR-002) — into flat CPU-side vertex/index buffers. `renderer.rs` uploads the result as
//! two pairs of static GPU buffers in [`crate::Renderer::new`] and never rebuilds them; nothing
//! in this module runs per frame.

use std::mem::size_of;

use look_above_core::geo::{LatLon, WEB_MERCATOR_EXTENT_M, web_mercator_forward};
use lyon::math::{Point, point};
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use serde_json::Value;

const LAND_GEOJSON: &str = include_str!("../assets/basemap/land.geojson");
const COASTLINE_GEOJSON: &str = include_str!("../assets/basemap/coastline.geojson");

/// Coastline stroke width, in the normalized coordinate space [`tessellate`] produces (the
/// world spans roughly `[-1, 1]` on each axis — see its doc comment).
///
/// Judgement call, tuned by eye against the M2 2.2b placeholder fit-to-window view (a
/// square-ish window shows close to the full `[-1, 1]` world): `0.0015` reads as a crisp
/// hairline without vanishing at ordinary desktop window sizes. 2.3's real camera introduces
/// zoom, at which point this may need revisiting — it is a screen-space judgement, not a
/// physical one, and there is no camera yet to make that call against.
pub const COASTLINE_STROKE_WIDTH: f32 = 0.0015;

/// Flattening tolerance for both tessellators, in the same normalized unit space as
/// [`COASTLINE_STROKE_WIDTH`]. `lyon`'s own default (`0.1`) is tuned for pixel-space paths;
/// against a world that spans only ~2 units total that would be a 5%-of-the-map error budget.
/// Every path tessellated here is already straight line segments (no curves), so this mostly
/// guards the tessellator's internal numerical robustness rather than visible flattening error.
const TESSELLATION_TOLERANCE: f32 = 0.000_5;

/// One tessellated vertex: position only. Both layers are flat-shaded (`color.rs` supplies the
/// color through a per-layer uniform — see `renderer.rs`), so no per-vertex color travels
/// through this buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
}

impl Vertex {
    /// The layout `renderer.rs`'s pipelines bind this struct at, matching `basemap.wgsl`'s
    /// `@location(0) position: vec2<f32>`.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }],
    };
}

/// One layer's CPU-side tessellation output — `renderer.rs` uploads `vertices`/`indices` as one
/// static vertex buffer and one static index buffer apiece.
#[derive(Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Both base-map layers, tessellated and ready for GPU upload.
#[derive(Debug, Default)]
pub struct BasemapGeometry {
    pub land: MeshData,
    pub coastline: MeshData,
}

/// Parses and tessellates the bundled base map. Pure and deterministic — same embedded input,
/// same output, every time — so [`crate::Renderer::new`] calls this once and never again.
///
/// # Panics
///
/// Panics if the bundled `GeoJSON` doesn't match the shape `import-basemap` produces
/// (`FeatureCollection` of `Polygon`/`LineString` features, coordinates as `[lon, lat]` number
/// pairs). That would mean the committed asset itself is corrupt — a build-time bug this
/// function is not meant to recover from at runtime, not a condition that can arise from live
/// data (this crate never touches live data).
pub fn tessellate() -> BasemapGeometry {
    BasemapGeometry {
        land: tessellate_land(LAND_GEOJSON),
        coastline: tessellate_coastline(COASTLINE_GEOJSON),
    }
}

fn tessellate_land(geojson: &str) -> MeshData {
    let value: Value = serde_json::from_str(geojson).expect("bundled land.geojson is valid JSON");
    let features = value["features"]
        .as_array()
        .expect("land.geojson has a top-level features array");

    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();
    // RFC 7946 (and `import-basemap`'s writer): outer ring CCW, hole rings CW. `NonZero` is the
    // fill rule that matches that convention — `EvenOdd` (lyon's default) happens to agree for
    // a single hole, but stops agreeing the moment two holes overlap, so it is picked
    // deliberately rather than left at the default.
    let options = FillOptions::default()
        .with_fill_rule(FillRule::NonZero)
        .with_tolerance(TESSELLATION_TOLERANCE);

    for feature in features {
        let rings = feature["geometry"]["coordinates"]
            .as_array()
            .expect("land polygon feature has a coordinates array of rings");
        let path = polygon_path(rings);

        tessellator
            .tessellate_path(
                &path,
                &options,
                &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| Vertex {
                    position: vertex.position().to_array(),
                }),
            )
            .expect("land polygon feature tessellates");
    }

    MeshData {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

fn tessellate_coastline(geojson: &str) -> MeshData {
    let value: Value =
        serde_json::from_str(geojson).expect("bundled coastline.geojson is valid JSON");
    let features = value["features"]
        .as_array()
        .expect("coastline.geojson has a top-level features array");

    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    let mut tessellator = StrokeTessellator::new();
    let options = StrokeOptions::default()
        .with_line_width(COASTLINE_STROKE_WIDTH)
        .with_tolerance(TESSELLATION_TOLERANCE);

    for feature in features {
        let points = feature["geometry"]["coordinates"]
            .as_array()
            .expect("coastline feature has a coordinates array of points");
        let path = line_path(points);

        tessellator
            .tessellate_path(
                &path,
                &options,
                &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex| Vertex {
                    position: vertex.position().to_array(),
                }),
            )
            .expect("coastline linestring feature tessellates");
    }

    MeshData {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

/// Builds one `Path` for a `Polygon` feature: `rings[0]` is the outer boundary, any further
/// rings are holes, each its own closed sub-path (RFC 7946 / this crate's doc comment on
/// `assets/basemap/`).
fn polygon_path(rings: &[Value]) -> Path {
    let mut builder = Path::builder();
    for ring in rings {
        let points = ring.as_array().expect("ring is a coordinate array");
        add_closed_ring(&mut builder, points);
    }
    builder.build()
}

/// Appends one closed sub-path (a polygon ring) to `builder`.
fn add_closed_ring(builder: &mut lyon::path::Builder, points: &[Value]) {
    let projected: Vec<Point> = points.iter().map(project_point).collect();
    // GeoJSON rings repeat their first point as their last (RFC 7946; also how
    // `import-basemap` writes them — see its "rings stay closed" test). `end(true)` below
    // already closes the sub-path back to `begin`, so the duplicate is dropped here rather
    // than fed through as a zero-length final segment.
    let ring: &[Point] = if projected.len() > 1 && projected.first() == projected.last() {
        &projected[..projected.len() - 1]
    } else {
        &projected[..]
    };

    let Some((&first, remainder)) = ring.split_first() else {
        return;
    };
    builder.begin(first);
    for &next in remainder {
        builder.line_to(next);
    }
    builder.end(true);
}

/// Builds one open `Path` for a `LineString` feature.
fn line_path(points: &[Value]) -> Path {
    let mut builder = Path::builder();
    let projected: Vec<Point> = points.iter().map(project_point).collect();

    if let Some((&first, remainder)) = projected.split_first() {
        builder.begin(first);
        for &next in remainder {
            builder.line_to(next);
        }
        builder.end(false);
    }

    builder.build()
}

/// Projects one `[lon, lat]` `GeoJSON` coordinate pair to the normalized Mercator plane this
/// module works in: Web Mercator metres ([`web_mercator_forward`]) divided by
/// [`WEB_MERCATOR_EXTENT_M`], landing roughly in `[-1, 1]` on both axes (exactly at the
/// antimeridian/latitude-clamp edges, inside it everywhere else). `f32` from here on is
/// deliberate — it is `lyon`'s and `wgpu`'s native type, and a base map has no use for `f64`'s
/// extra precision.
#[allow(
    clippy::cast_possible_truncation,
    reason = "lon/lat magnitudes here are at most a few times WEB_MERCATOR_EXTENT_M's scale (~2e7), \
              far inside f32's range; the narrowing is deliberate (see the doc comment above), not accidental"
)]
fn project_point(coordinate: &Value) -> Point {
    let pair = coordinate
        .as_array()
        .expect("coordinate is a [lon, lat] array");
    let lon_deg = pair[0].as_f64().expect("longitude is a JSON number");
    let lat_deg = pair[1].as_f64().expect("latitude is a JSON number");

    let mercator = web_mercator_forward(LatLon::new(lat_deg, lon_deg));
    point(
        (mercator.x_m / WEB_MERCATOR_EXTENT_M) as f32,
        (mercator.y_m / WEB_MERCATOR_EXTENT_M) as f32,
    )
}

// ================================================================================================
// Globe (orthographic) tessellation — M4 item 4.3.
//
// A second, independent path alongside everything above: same bundled GeoJSON, same ring-
// extraction/tessellation shape, but skipping `web_mercator_forward` entirely. `renderer.rs`'s
// globe basemap pass needs raw per-vertex lon/lat (in radians) so it can compute the orthographic
// projection itself, per-vertex, in `globe_basemap.wgsl` — see that shader's own doc comment for
// why a per-fragment (not per-vertex) horizon test is what keeps a triangle straddling the true
// horizon clipping cleanly along the correct curve rather than popping.
//
// Deliberately *not* sharing `Vertex`/`MeshData`/`tessellate_land`/`tessellate_coastline` with the
// Mercator path above: `RunwayLayer` (`renderer.rs`) reuses those exactly (see its own doc comment
// on why that reuse must stay exact), and runways only ever matter at L2/Regional where the globe
// is never visible — entangling the two paths would risk that reuse silently drifting.

/// One tessellated globe vertex: raw lon/lat, in radians. The tessellation itself (below) still
/// runs in *degrees* — `lyon`'s flattening tolerance and this section's stroke-width constant are
/// tuned against that plane — and each output vertex is converted to radians only at the very
/// end, once, by [`globe_vertex_from_degrees`].
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlobeVertex {
    pub lonlat_rad: [f32; 2],
}

impl GlobeVertex {
    /// The layout `renderer.rs`'s globe pipelines bind this struct at, matching
    /// `globe_basemap.wgsl`'s `@location(0) lonlat_rad: vec2<f32>`.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<GlobeVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }],
    };
}

/// One globe layer's CPU-side tessellation output — the globe analogue of [`MeshData`].
#[derive(Debug, Default)]
pub struct GlobeMeshData {
    pub vertices: Vec<GlobeVertex>,
    pub indices: Vec<u32>,
}

/// Both globe base-map layers, tessellated and ready for GPU upload — the globe analogue of
/// [`BasemapGeometry`].
#[derive(Debug, Default)]
pub struct GlobeBasemapGeometry {
    pub land: GlobeMeshData,
    pub coastline: GlobeMeshData,
}

/// Coastline stroke width for the globe mesh, in the *degree* tessellation plane
/// [`tessellate_globe`] works in (see this section's own doc comment) — **not** the same numeric
/// scale as [`COASTLINE_STROKE_WIDTH`], which is calibrated against the Mercator path's
/// normalized `[-1, 1]` plane instead.
///
/// Judgement call, tuned by eye the same way [`COASTLINE_STROKE_WIDTH`]'s own doc comment frames
/// its constant: the globe's unit disk ([`orthographic_forward`]) has radius 1 near the
/// sub-observer point, and 1 radian of angular distance maps to roughly 1 disk unit there
/// (small-angle approximation) — so `0.08°` (`≈ 0.0014` rad) reads at roughly the same on-screen
/// weight [`COASTLINE_STROKE_WIDTH`]'s `0.0015` does on the Mercator path, before either camera's
/// own zoom is applied.
const GLOBE_COASTLINE_STROKE_WIDTH_DEG: f32 = 0.08;

/// Flattening tolerance for the globe tessellators, in the same degree plane — chosen well below
/// [`GLOBE_COASTLINE_STROKE_WIDTH_DEG`] for the same reason `airport.rs`'s own runway
/// tessellation-tolerance constant documents: `lyon`'s stroke tessellator treats a stroke at or
/// below its own tolerance as too thin to represent and silently produces empty geometry.
const GLOBE_TESSELLATION_TOLERANCE_DEG: f32 = 0.02;

/// Parses and tessellates the bundled base map for the globe pass — the globe analogue of
/// [`tessellate`]. Pure and deterministic, run once by `Renderer::new`/`new_headless` alongside
/// [`tessellate`] itself.
///
/// # Panics
///
/// Same panic contract as [`tessellate`] — malformed bundled `GeoJSON` is a build-time bug, not a
/// runtime condition this function recovers from.
pub fn tessellate_globe() -> GlobeBasemapGeometry {
    GlobeBasemapGeometry {
        land: tessellate_globe_land(LAND_GEOJSON),
        coastline: tessellate_globe_coastline(COASTLINE_GEOJSON),
    }
}

fn tessellate_globe_land(geojson: &str) -> GlobeMeshData {
    let value: Value = serde_json::from_str(geojson).expect("bundled land.geojson is valid JSON");
    let features = value["features"]
        .as_array()
        .expect("land.geojson has a top-level features array");

    let mut buffers: VertexBuffers<GlobeVertex, u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();
    // Same NonZero-over-EvenOdd reasoning as `tessellate_land` — RFC 7946's outer-CCW/hole-CW
    // winding convention.
    let options = FillOptions::default()
        .with_fill_rule(FillRule::NonZero)
        .with_tolerance(GLOBE_TESSELLATION_TOLERANCE_DEG);

    for feature in features {
        let rings = feature["geometry"]["coordinates"]
            .as_array()
            .expect("land polygon feature has a coordinates array of rings");
        let path = polygon_path_globe(rings);

        tessellator
            .tessellate_path(
                &path,
                &options,
                &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
                    globe_vertex_from_degrees(vertex.position())
                }),
            )
            .expect("land polygon feature tessellates");
    }

    GlobeMeshData {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

fn tessellate_globe_coastline(geojson: &str) -> GlobeMeshData {
    let value: Value =
        serde_json::from_str(geojson).expect("bundled coastline.geojson is valid JSON");
    let features = value["features"]
        .as_array()
        .expect("coastline.geojson has a top-level features array");

    let mut buffers: VertexBuffers<GlobeVertex, u32> = VertexBuffers::new();
    let mut tessellator = StrokeTessellator::new();
    let options = StrokeOptions::default()
        .with_line_width(GLOBE_COASTLINE_STROKE_WIDTH_DEG)
        .with_tolerance(GLOBE_TESSELLATION_TOLERANCE_DEG);

    for feature in features {
        let points = feature["geometry"]["coordinates"]
            .as_array()
            .expect("coastline feature has a coordinates array of points");
        let path = line_path_globe(points);

        tessellator
            .tessellate_path(
                &path,
                &options,
                &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex| {
                    globe_vertex_from_degrees(vertex.position())
                }),
            )
            .expect("coastline linestring feature tessellates");
    }

    GlobeMeshData {
        vertices: buffers.vertices,
        indices: buffers.indices,
    }
}

/// Converts one tessellated point in the degree plane [`tessellate_globe_land`]/
/// [`tessellate_globe_coastline`] work in into a [`GlobeVertex`]'s radian output.
fn globe_vertex_from_degrees(position: Point) -> GlobeVertex {
    GlobeVertex {
        lonlat_rad: [position.x.to_radians(), position.y.to_radians()],
    }
}

/// [`polygon_path`]'s globe-plane twin: identical ring extraction, [`project_point_globe`]
/// instead of [`project_point`].
fn polygon_path_globe(rings: &[Value]) -> Path {
    let mut builder = Path::builder();
    for ring in rings {
        let points = ring.as_array().expect("ring is a coordinate array");
        add_closed_ring_globe(&mut builder, points);
    }
    builder.build()
}

/// [`add_closed_ring`]'s globe-plane twin — see that function's own doc comment for the
/// duplicate-closing-point handling, unchanged here.
fn add_closed_ring_globe(builder: &mut lyon::path::Builder, points: &[Value]) {
    let projected: Vec<Point> = points.iter().map(project_point_globe).collect();
    let ring: &[Point] = if projected.len() > 1 && projected.first() == projected.last() {
        &projected[..projected.len() - 1]
    } else {
        &projected[..]
    };

    let Some((&first, remainder)) = ring.split_first() else {
        return;
    };
    builder.begin(first);
    for &next in remainder {
        builder.line_to(next);
    }
    builder.end(true);
}

/// [`line_path`]'s globe-plane twin.
fn line_path_globe(points: &[Value]) -> Path {
    let mut builder = Path::builder();
    let projected: Vec<Point> = points.iter().map(project_point_globe).collect();

    if let Some((&first, remainder)) = projected.split_first() {
        builder.begin(first);
        for &next in remainder {
            builder.line_to(next);
        }
        builder.end(false);
    }

    builder.build()
}

/// Projects one `[lon, lat]` `GeoJSON` coordinate pair into the plain equirectangular
/// degree-plane [`tessellate_globe`] tessellates in — no Mercator/extent division, unlike
/// [`project_point`]: the sphere projection itself happens per-vertex on the GPU (see this
/// section's own doc comment), so this is a standard "tessellate flat in lon/lat degrees, project
/// on the sphere per-vertex" technique, not an attempt at a flat map projection of its own.
#[allow(
    clippy::cast_possible_truncation,
    reason = "lon/lat degrees are always in [-180, 180]/[-90, 90], far inside f32's range"
)]
fn project_point_globe(coordinate: &Value) -> Point {
    let pair = coordinate
        .as_array()
        .expect("coordinate is a [lon, lat] array");
    let lon_deg = pair[0].as_f64().expect("longitude is a JSON number");
    let lat_deg = pair[1].as_f64().expect("latitude is a JSON number");
    point(lon_deg as f32, lat_deg as f32)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // ---- Regression: the bundled assets haven't silently changed shape ---------------------

    #[test]
    fn land_geojson_matches_the_known_feature_and_point_counts() {
        let value: Value = serde_json::from_str(LAND_GEOJSON).expect("bundled land.geojson parses");
        let features = value["features"].as_array().expect("features array");
        assert_eq!(
            features.len(),
            1_421,
            "land.geojson feature count regressed"
        );

        let points: usize = features
            .iter()
            .map(|feature| {
                feature["geometry"]["coordinates"]
                    .as_array()
                    .expect("rings array")
                    .iter()
                    .map(|ring| ring.as_array().expect("ring points array").len())
                    .sum::<usize>()
            })
            .sum();
        assert_eq!(points, 60_669, "land.geojson point count regressed");
    }

    #[test]
    fn coastline_geojson_matches_the_known_feature_and_point_counts() {
        let value: Value =
            serde_json::from_str(COASTLINE_GEOJSON).expect("bundled coastline.geojson parses");
        let features = value["features"].as_array().expect("features array");
        assert_eq!(
            features.len(),
            1_429,
            "coastline.geojson feature count regressed"
        );

        let points: usize = features
            .iter()
            .map(|feature| {
                feature["geometry"]["coordinates"]
                    .as_array()
                    .expect("points array")
                    .len()
            })
            .sum();
        assert_eq!(points, 60_416, "coastline.geojson point count regressed");
    }

    // ---- Fill rule: holes are actually excluded, not just assumed to be --------------------

    /// A 20x20 (degree) square with a 4x4 square hole dead center, RFC 7946 winding (outer
    /// CCW, hole CW). Small enough around the equator/prime-meridian that Web Mercator's
    /// nonlinearity is not in question here — `core::geo`'s own tests already pin that; this
    /// test only cares whether the *hole* survives from `GeoJSON` through to triangles.
    fn square_with_hole_geojson() -> String {
        json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {},
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [
                        [[-10.0, -10.0], [-10.0, 10.0], [10.0, 10.0], [10.0, -10.0], [-10.0, -10.0]],
                        [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0], [-2.0, -2.0]]
                    ]
                }
            }]
        })
        .to_string()
    }

    #[test]
    fn non_zero_fill_rule_excludes_the_hole() {
        let mesh = tessellate_land(&square_with_hole_geojson());
        assert!(!mesh.indices.is_empty(), "the outer square must still fill");
        assert_eq!(mesh.indices.len() % 3, 0);

        let hole_corner_a = project_point(&json!([-2.0, -2.0]));
        let hole_corner_b = project_point(&json!([2.0, 2.0]));
        let hole_min_x = hole_corner_a.x.min(hole_corner_b.x);
        let hole_max_x = hole_corner_a.x.max(hole_corner_b.x);
        let hole_min_y = hole_corner_a.y.min(hole_corner_b.y);
        let hole_max_y = hole_corner_a.y.max(hole_corner_b.y);

        for triangle in mesh.indices.chunks_exact(3) {
            let (cx, cy) = centroid(&mesh.vertices, triangle);
            let inside_hole =
                (hole_min_x..hole_max_x).contains(&cx) && (hole_min_y..hole_max_y).contains(&cy);
            assert!(
                !inside_hole,
                "a triangle centroid ({cx}, {cy}) fell inside the hole \
                 ({hole_min_x}..{hole_max_x}, {hole_min_y}..{hole_max_y})"
            );
        }
    }

    fn centroid(vertices: &[Vertex], triangle: &[u32]) -> (f32, f32) {
        let (sum_x, sum_y) = triangle
            .iter()
            .fold((0.0_f32, 0.0_f32), |(sx, sy), &index| {
                let position = vertices[index as usize].position;
                (sx + position[0], sy + position[1])
            });
        (sum_x / 3.0, sum_y / 3.0)
    }

    // ---- Normalization bounds ---------------------------------------------------------------

    #[test]
    fn every_tessellated_vertex_lands_within_the_normalized_square() {
        let geometry = tessellate();
        let bound = 1.01_f32;

        for mesh in [&geometry.land, &geometry.coastline] {
            for vertex in &mesh.vertices {
                let [x, y] = vertex.position;
                assert!((-bound..=bound).contains(&x), "x {x} escaped [-1.01, 1.01]");
                assert!((-bound..=bound).contains(&y), "y {y} escaped [-1.01, 1.01]");
            }
        }
    }

    // ---- Buffer sanity -----------------------------------------------------------------------

    #[test]
    fn land_and_coastline_buffers_are_non_empty_triangle_lists() {
        let geometry = tessellate();

        assert!(!geometry.land.indices.is_empty());
        assert_eq!(geometry.land.indices.len() % 3, 0);

        assert!(!geometry.coastline.indices.is_empty());
        assert_eq!(geometry.coastline.indices.len() % 3, 0);
    }

    // ---- Globe (M4 item 4.3) ----------------------------------------------------------------

    #[test]
    fn every_globe_tessellated_vertex_lands_within_radian_bounds() {
        let geometry = tessellate_globe();
        // A little past exact ±π/±π/2 to tolerate a coastline stroke's own extruded width at
        // the antimeridian/poles, the same generous-margin shape
        // `every_tessellated_vertex_lands_within_the_normalized_square` uses.
        let lon_bound = std::f32::consts::PI + 0.01;
        let lat_bound = std::f32::consts::FRAC_PI_2 + 0.01;

        for mesh in [&geometry.land, &geometry.coastline] {
            for vertex in &mesh.vertices {
                let [lon_rad, lat_rad] = vertex.lonlat_rad;
                assert!(
                    (-lon_bound..=lon_bound).contains(&lon_rad),
                    "lon {lon_rad} escaped [-{lon_bound}, {lon_bound}]"
                );
                assert!(
                    (-lat_bound..=lat_bound).contains(&lat_rad),
                    "lat {lat_rad} escaped [-{lat_bound}, {lat_bound}]"
                );
            }
        }
    }

    #[test]
    fn globe_land_and_coastline_buffers_are_non_empty_triangle_lists() {
        let geometry = tessellate_globe();

        assert!(!geometry.land.indices.is_empty());
        assert_eq!(geometry.land.indices.len() % 3, 0);

        assert!(!geometry.coastline.indices.is_empty());
        assert_eq!(geometry.coastline.indices.len() % 3, 0);
    }

    /// Property test (M4 item 4.3 follow-up, prompted by a live visual bug — see
    /// `renderer.rs`'s own `globe_mode_fades_out_the_flat_mercator_map_outside_the_disk` test for
    /// the pixel-level regression this pairs with): running [`tessellate_globe`]'s *own* output
    /// vertices back through `core::geo::orthographic_forward` independently must split them into
    /// a plausible near/far mix, not something degenerate (e.g. every vertex on one side — the
    /// signature of a degrees/radians or lat/lon mixup in the tessellation itself). Centered at
    /// the equator/prime meridian, a real-world coastline/land dataset's near-hemisphere share
    /// should land close to half; `[0.35, 0.65]` is a generous band around that expectation, not
    /// a tight pin on the bundled dataset's exact land distribution.
    #[test]
    fn globe_tessellated_vertices_split_plausibly_between_near_and_far_hemisphere() {
        use look_above_core::geo::{LatLon, orthographic_forward};

        let geometry = tessellate_globe();
        let center = LatLon::new(0.0, 0.0);

        for (name, mesh) in [("land", &geometry.land), ("coastline", &geometry.coastline)] {
            let total = mesh.vertices.len();
            let visible = mesh
                .vertices
                .iter()
                .filter(|vertex| {
                    let lon_deg = f64::from(vertex.lonlat_rad[0].to_degrees());
                    let lat_deg = f64::from(vertex.lonlat_rad[1].to_degrees());
                    orthographic_forward(center, LatLon::new(lat_deg, lon_deg)).is_some()
                })
                .count();
            // Vertex counts here are in the tens of thousands at most — far inside f64's
            // 53-bit exact-integer range, so this narrowing loses nothing in practice.
            #[allow(
                clippy::cast_precision_loss,
                reason = "tessellated vertex counts are far inside f64's exact-integer range"
            )]
            let fraction = visible as f64 / total as f64;
            assert!(
                (0.35..=0.65).contains(&fraction),
                "{name}: {visible}/{total} = {fraction:.3} visible from center (0, 0) — expected \
                 roughly half, not a near-degenerate split"
            );
        }
    }
}
