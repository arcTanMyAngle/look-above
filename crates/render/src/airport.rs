//! CPU-side geometry for the airport-marker and runway-outline "map lines" passes (M3 item 3.2).
//!
//! Scoped exactly per the M3 plan's own cross-milestone tension note: no LOD-tier gating here
//! (M4's job) — `app::window` queries `store::Writer` at a fixed `AirportSize::Medium` threshold
//! on camera settle, and this module (and `renderer.rs`'s layers built on top of it) draws
//! whatever `Vec<Airport>`/`Vec<Runway>` it is handed, unconditionally, at the current single
//! render tier.
//!
//! Reuses `lyon` the same way `basemap.rs` does: `web_mercator_forward`/`WEB_MERCATOR_EXTENT_M`
//! project lon/lat into the same normalized `[-1, 1]` Mercator plane every other layer works in,
//! and `basemap::Vertex`/`basemap::MeshData` (position-only, flat-shaded) are reused directly
//! rather than duplicating that shape.
//!
//! Two pieces of geometry:
//! - **Airport markers**: a small filled circle per airport — see [`AIRPORT_MARKER_RADIUS_PX`]'s
//!   own doc comment for why a circle was picked over a diamond/dot — instanced off one static
//!   unit-circle mesh ([`marker_mesh`]) the same way `aircraft.rs` instances one static unit quad.
//!   [`pack_airport_instances`] is this pass's per-instance packer, mirroring
//!   `aircraft::pack_instances`'s shape.
//! - **Runway outlines**: one open `StrokeTessellator` line segment per runway with both ends
//!   present (a runway missing either end draws nothing — see [`tessellate_runways`]'s own doc
//!   comment), mirroring `basemap::tessellate_coastline`'s per-feature stroke loop but per-runway.
//!
//! Judgement call on rebuild cadence: unlike `basemap.rs`'s static, build-once tessellation, both
//! of these rebuild every frame from whatever slice `renderer.rs` is handed, mirroring
//! `trail.rs`'s per-publish reused-scratch shape rather than caching "did the queried set actually
//! change." Simpler than tracking that comparison, and cheap at this milestone's scale (a handful
//! to a few hundred medium/large airports in a viewport, nowhere near docs/01's 10,000-aircraft
//! budget) — the reused `Vec`/`VertexBuffers` scratch buffers below still satisfy ADR-002's
//! no-per-frame-*allocation* rule even though the geometry itself is recomputed every frame.

use std::mem::size_of;

use look_above_core::contracts::{Airport, Runway};
use look_above_core::geo::{LatLon, WEB_MERCATOR_EXTENT_M, web_mercator_forward};
use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};

use crate::basemap::{MeshData, Vertex};

/// On-screen radius of an airport marker, in physical pixels, screen-constant regardless of zoom
/// — the same "fixed pixel size, recomputed from the camera's `meters_per_pixel` every frame"
/// shape `aircraft::AIRCRAFT_GLYPH_PX`/`glyph_scale_normalized` use (see
/// [`airport_marker_scale_normalized`]). A judgement call, tuned by eye: small enough not to
/// compete with an aircraft glyph (`aircraft::AIRCRAFT_GLYPH_PX` is 20 px) but large enough to
/// register as a distinct point at typical L2 zoom.
///
/// Shape judgement call: a plain filled circle, not a diamond or a single-pixel dot. An airport
/// has no heading to orient a diamond against (unlike an aircraft glyph), so a circle reads as a
/// static point-of-interest marker without implying a direction; a dot (few-pixel square) would
/// be harder to distinguish from anti-aliased noise at this size under MSAA.
pub const AIRPORT_MARKER_RADIUS_PX: f64 = 4.0;

/// Straight segments approximating the marker's unit circle. A marker this small (a handful of
/// screen pixels — see [`AIRPORT_MARKER_RADIUS_PX`]) does not need `lyon`'s arc-flattening
/// machinery; enough facets that docs/01's 4x MSAA hides them is all that's needed, and this is
/// well past that point.
const MARKER_CIRCLE_SEGMENTS: usize = 24;

/// Runway centerline on-screen stroke width, in physical pixels — screen-constant regardless of
/// zoom, tuned by eye the same judgement-call way [`basemap::COASTLINE_STROKE_WIDTH`] was.
///
/// **Not** a fixed world-plane constant the way the coastline stroke is, even though the M3
/// checklist's own wording ("reusing existing tessellation approach ... rather than a new one")
/// reads at first as "copy that shape exactly": a coastline linestring spans a whole landmass, so
/// a fixed-in-world-plane stroke width there is a legible, sensible physical-line convention. A
/// runway is a single two-point segment at a much smaller physical scale, and — separately from
/// this constant's own choice — needs [`RUNWAY_TESSELLATION_SCALE`]'s rescaling to tessellate at
/// all in this crate's whole-Earth-spanning normalized plane (see that constant's own doc comment
/// for the actual failure mode this module's tests caught). Making the *width* screen-constant
/// (via [`runway_stroke_width_normalized`], the exact "pixels → world metres → divide by the
/// extent" shape [`crate::trail::half_width_normalized`]/[`crate::aircraft::glyph_scale_normalized`]
/// already use) is a separate, purely visual judgement call: it keeps the outline a legible
/// on-screen width at whatever zoom the (LOD-gating-free, per the M3 tension note) camera happens
/// to be at, the same way a fixed world-plane width would stop being legible at extreme zoom.
pub const RUNWAY_STROKE_WIDTH_PX: f64 = 2.0;

/// Ceiling on the runway stroke tessellator's flattening tolerance, in the same pre-normalized
/// `[-1, 1]` plane [`basemap::TESSELLATION_TOLERANCE`] uses. The tolerance actually passed to
/// `lyon` (see [`tessellate_runways`]) is `min(stroke_width * 0.1, this ceiling)`, **not** this
/// constant alone: `lyon`'s stroke tessellator treats a stroke whose width is at or below its
/// tolerance as too thin to bother representing and silently produces empty geometry rather than
/// an error (this module's own tests caught exactly that when a fixed absolute tolerance this
/// size — copied from [`basemap::TESSELLATION_TOLERANCE`] — ended up *larger* than
/// [`RUNWAY_STROKE_WIDTH_PX`]'s own screen-constant width at ordinary zoom). Scaling the
/// tolerance down with the width keeps it reliably a fraction of the width at every zoom; this
/// ceiling only guards the opposite, harmless-but-wasteful extreme (an absurdly coarse tolerance
/// at an absurdly wide stroke).
const RUNWAY_TESSELLATION_TOLERANCE_CEILING: f32 = 0.000_5;

/// Factor [`tessellate_runways`] scales every coordinate (and the stroke width/tolerance) up by
/// before handing a runway's two-point path to `lyon`, then scales the resulting vertex positions
/// back down by afterward.
///
/// `lyon`'s stroke tessellator has its own hardcoded internal point-merge threshold (independent
/// of the `tolerance`/`line_width` this module configures — see `lyon_tessellation::stroke`'s own
/// `square_merge_threshold` computation, floored at `1e-8` squared, i.e. a ~`1e-4` absolute
/// distance) below which two path points are treated as coincident and the whole path collapses
/// to nothing. This crate's normalized Mercator plane spans the *entire Earth* in roughly
/// `[-1, 1]` (see this module's own doc comment), so even an ordinary few-kilometre runway often
/// projects to a segment shorter than that ~`1e-4` threshold — `lyon` would silently drop it,
/// not error (caught by this module's own regression test). `1e5` moves every runway's own
/// segment length comfortably above that floor (a 300 m runway, ~`1.5e-5` units at this crate's
/// native scale, becomes `~1.5` after scaling) without needing any camera/zoom information of its
/// own — a fixed geometric rescaling, not a screen-space one, so it composes cleanly with
/// [`runway_stroke_width_normalized`]'s separate (zoom-dependent) scaling of the width itself.
const RUNWAY_TESSELLATION_SCALE: f32 = 100_000.0;

/// The world-space (pre-normalized-plane) stroke width a screen-constant
/// [`RUNWAY_STROKE_WIDTH_PX`]-pixel runway outline must be tessellated at, given the camera's
/// current `meters_per_pixel` — see [`RUNWAY_STROKE_WIDTH_PX`]'s own doc comment for why this is
/// computed fresh at tessellation time rather than baked in as a fixed constant.
pub fn runway_stroke_width_normalized(meters_per_pixel: f64) -> f32 {
    let world_m = RUNWAY_STROKE_WIDTH_PX * meters_per_pixel;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a screen-pixel-scale width in normalized-plane units is a tiny fraction, \
                  nowhere near f32's precision limits"
    )]
    {
        (world_m / WEB_MERCATOR_EXTENT_M) as f32
    }
}

/// Starting capacity (in instances) for the airport-marker GPU instance buffer, before any frame
/// has grown it. A typical viewport's medium/large airports number in the tens; this comfortably
/// covers a first busy region without an immediate regrow.
pub const MIN_AIRPORT_INSTANCE_CAPACITY: usize = 64;

/// Starting capacity (in vertices) for the runway-outline GPU vertex buffer — matches
/// [`lyon::tessellation::VertexBuffers::new`]'s own default (512 vertices / 1024 indices), so the
/// GPU-side buffer and the CPU-side scratch buffer warm up to the same size together.
pub const MIN_RUNWAY_VERTEX_CAPACITY: usize = 512;

/// Starting capacity (in indices) for the runway-outline GPU index buffer — see
/// [`MIN_RUNWAY_VERTEX_CAPACITY`]'s own doc comment.
pub const MIN_RUNWAY_INDEX_CAPACITY: usize = 1024;

/// One airport marker's packed per-instance GPU attribute: position only — an airport marker has
/// no heading to carry, unlike [`crate::aircraft::InstanceRaw`]. `renderer.rs` uploads a `Vec` of
/// these, one per airport in the app's currently queried set, as the instance-stepped half of the
/// marker pipeline's vertex input (the vertex-stepped half is [`marker_mesh`]'s shared unit
/// circle).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AirportInstanceRaw {
    /// World position, Web Mercator metres divided by [`WEB_MERCATOR_EXTENT_M`] — the same
    /// pre-normalized plane every other layer's `world_xy` operates on.
    pub world_xy: [f32; 2],
}

impl AirportInstanceRaw {
    /// The layout `renderer.rs`'s marker pipeline binds this struct at (instance-stepped,
    /// `@location(1)` — `@location(0)` is [`marker_mesh`]'s shared [`Vertex`] geometry), matching
    /// `airport.wgsl`'s `InstanceInput`.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 1,
        }],
    };
}

/// Projects one lat/lon (an airport, a runway endpoint, or — `pub(crate)` since M3 item 3.3 —
/// a METAR badge's airport position) to the normalized Mercator plane this module works in —
/// identical math to `basemap::project_point`, operating on already-parsed `f64` degrees
/// (`core::contracts::Airport`/`Runway`/`MetarBadge` fields) instead of a raw `GeoJSON`
/// coordinate pair, since there is no `GeoJSON` here.
#[allow(
    clippy::cast_possible_truncation,
    reason = "lon/lat magnitudes here are at most a few times WEB_MERCATOR_EXTENT_M's scale (~2e7), \
              far inside f32's range; the narrowing is deliberate, same as basemap::project_point"
)]
pub(crate) fn project(lat_deg: f64, lon_deg: f64) -> [f32; 2] {
    let mercator = web_mercator_forward(LatLon::new(lat_deg, lon_deg));
    [
        (mercator.x_m / WEB_MERCATOR_EXTENT_M) as f32,
        (mercator.y_m / WEB_MERCATOR_EXTENT_M) as f32,
    ]
}

/// The airport marker's shared unit-circle mesh (radius 1 in local space, centered on the
/// origin), scaled per-instance in `airport.wgsl` — see [`airport_marker_scale_normalized`]. Built
/// once, in `renderer.rs`'s `Renderer::new`/`new_headless` (the same "static, build-once" shape
/// `basemap::tessellate`'s land/coastline meshes and `aircraft::quad_vertices`'s shared quad
/// already have): the circle's own shape never changes, only which airports get an instance of
/// it drawn.
///
/// A plain fan triangulation (center vertex + a ring of [`MARKER_CIRCLE_SEGMENTS`] points) rather
/// than `lyon`'s fill tessellator: a regular polygon fan needs no tessellation library at all, and
/// every vertex/index is already known in closed form.
pub fn marker_mesh() -> MeshData {
    let mut vertices = Vec::with_capacity(MARKER_CIRCLE_SEGMENTS + 1);
    let mut indices = Vec::with_capacity(MARKER_CIRCLE_SEGMENTS * 3);

    // Vertex 0 is the fan's center; the ring starts at index 1.
    vertices.push(Vertex {
        position: [0.0, 0.0],
    });
    for i in 0..MARKER_CIRCLE_SEGMENTS {
        #[allow(
            clippy::cast_precision_loss,
            reason = "i is bounded by MARKER_CIRCLE_SEGMENTS, a small compile-time constant, far \
                      inside f32's exact-integer range"
        )]
        let theta = (i as f32 / MARKER_CIRCLE_SEGMENTS as f32) * std::f32::consts::TAU;
        vertices.push(Vertex {
            position: [theta.cos(), theta.sin()],
        });
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "i is bounded by MARKER_CIRCLE_SEGMENTS, a small compile-time constant, far \
                  inside u32::MAX"
    )]
    for i in 0..MARKER_CIRCLE_SEGMENTS {
        let a = 1 + i as u32;
        let b = 1 + ((i + 1) % MARKER_CIRCLE_SEGMENTS) as u32;
        indices.push(0);
        indices.push(a);
        indices.push(b);
    }

    MeshData { vertices, indices }
}

/// The world-space (pre-normalized-plane) radius a screen-constant [`AIRPORT_MARKER_RADIUS_PX`]
/// -pixel marker must be drawn at, given the camera's current `meters_per_pixel` — the exact same
/// "pixels → world metres → divide by the extent" narrowing `aircraft::glyph_scale_normalized`
/// performs. No halving (unlike `trail::half_width_normalized`): [`marker_mesh`]'s unit circle
/// already has radius 1, not a `[-0.5, 0.5]` quad half-extent, so this scale is already the full
/// desired radius.
pub fn airport_marker_scale_normalized(meters_per_pixel: f64) -> f32 {
    let world_m = AIRPORT_MARKER_RADIUS_PX * meters_per_pixel;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a screen-pixel-scale radius in normalized-plane units is a tiny fraction, \
                  nowhere near f32's precision limits"
    )]
    {
        (world_m / WEB_MERCATOR_EXTENT_M) as f32
    }
}

/// Packs every airport in `airports` into this frame's marker instance buffer, appending into
/// `out` (cleared first, capacity reused frame to frame — the same reused-scratch shape as
/// `aircraft::pack_instances`/`trail::tessellate_trails`; see this module's own doc comment on
/// why markers rebuild every frame rather than only when the queried set changes).
pub fn pack_airport_instances(airports: &[Airport], out: &mut Vec<AirportInstanceRaw>) {
    out.clear();
    out.extend(airports.iter().map(|airport| AirportInstanceRaw {
        world_xy: project(airport.lat_deg, airport.lon_deg),
    }));
}

/// Tessellates every runway in `runways` with both ends present into stroked line geometry,
/// appending into `out` (cleared first, capacity reused frame to frame — see this module's own
/// doc comment on rebuild cadence).
///
/// A runway missing either end's lat/lon draws nothing for that runway — `core::contracts::
/// Runway`'s own doc comment already documents some bundled rows as incomplete; this is not an
/// error condition; a runway whose two (present) ends project to the same point (zero-length —
/// no direction for the stroke tessellator to extrude) is likewise skipped, the same tolerance
/// `trail.rs` gives coincident samples.
///
/// `meters_per_pixel` is the camera's current zoom, threaded through to
/// [`runway_stroke_width_normalized`] — see [`RUNWAY_STROKE_WIDTH_PX`]'s own doc comment for why
/// the stroke width is computed fresh here rather than a fixed constant.
///
/// Tessellates each runway in a coordinate space scaled up by [`RUNWAY_TESSELLATION_SCALE`] from
/// this module's usual normalized `[-1, 1]` plane, then scales the resulting vertex positions
/// back down before appending them to `out` — see that constant's own doc comment for why: `lyon`
/// has a hardcoded internal minimum feature size, well above what a single runway (a couple of
/// points a few hundred metres to a few kilometres apart) occupies at this crate's normal
/// whole-Earth-spanning scale, so tessellating directly at that scale silently drops the runway
/// entirely rather than erroring.
pub fn tessellate_runways(
    runways: &[Runway],
    meters_per_pixel: f64,
    out: &mut VertexBuffers<Vertex, u32>,
) {
    out.vertices.clear();
    out.indices.clear();

    let stroke_width = runway_stroke_width_normalized(meters_per_pixel);
    // The flattening tolerance must stay well below the stroke width, or `lyon` treats the whole
    // stroke as too thin to bother representing — same reasoning as
    // `basemap::TESSELLATION_TOLERANCE`'s own doc comment, just scaled to this (now
    // screen-constant, not fixed) width rather than a fixed one.
    let tolerance = (stroke_width * 0.1).min(RUNWAY_TESSELLATION_TOLERANCE_CEILING);

    let scaled_width = stroke_width * RUNWAY_TESSELLATION_SCALE;
    let scaled_tolerance = tolerance * RUNWAY_TESSELLATION_SCALE;
    let mut tessellator = StrokeTessellator::new();
    let options = StrokeOptions::default()
        .with_line_width(scaled_width)
        .with_tolerance(scaled_tolerance);

    for runway in runways {
        let (Some(le_lat), Some(le_lon), Some(he_lat), Some(he_lon)) = (
            runway.le_lat_deg,
            runway.le_lon_deg,
            runway.he_lat_deg,
            runway.he_lon_deg,
        ) else {
            continue;
        };

        let le = project(le_lat, le_lon);
        let he = project(he_lat, he_lon);
        if le == he {
            continue;
        }

        let mut builder = Path::builder();
        builder.begin(point(
            le[0] * RUNWAY_TESSELLATION_SCALE,
            le[1] * RUNWAY_TESSELLATION_SCALE,
        ));
        builder.line_to(point(
            he[0] * RUNWAY_TESSELLATION_SCALE,
            he[1] * RUNWAY_TESSELLATION_SCALE,
        ));
        builder.end(false);
        let path = builder.build();

        tessellator
            .tessellate_path(
                &path,
                &options,
                &mut BuffersBuilder::new(out, |vertex: StrokeVertex| {
                    let scaled = vertex.position();
                    Vertex {
                        position: [
                            scaled.x / RUNWAY_TESSELLATION_SCALE,
                            scaled.y / RUNWAY_TESSELLATION_SCALE,
                        ],
                    }
                }),
            )
            .expect("a two-point open runway path tessellates");
    }
}

#[cfg(test)]
mod tests {
    use look_above_core::contracts::AirportSize;

    use super::*;

    /// A plausible L2-regional `meters_per_pixel` (docs/01: L2 is < 300 km viewport width) —
    /// comfortably keeps [`RUNWAY_STROKE_WIDTH_PX`]'s tessellated width well below every test
    /// fixture runway's own (non-degenerate) length below.
    const TEST_METERS_PER_PIXEL: f64 = 150.0;

    fn test_airport(lat_deg: f64, lon_deg: f64) -> Airport {
        Airport {
            ident: "TEST".to_string(),
            name: "Test Airport".to_string(),
            size: AirportSize::Medium,
            lat_deg,
            lon_deg,
            elevation_ft: None,
            iso_country: None,
            iata: None,
        }
    }

    fn full_runway(le_lat: f64, le_lon: f64, he_lat: f64, he_lon: f64) -> Runway {
        Runway {
            airport_ident: "TEST".to_string(),
            le_ident: Some("09".to_string()),
            le_lat_deg: Some(le_lat),
            le_lon_deg: Some(le_lon),
            le_heading_deg: Some(90.0),
            he_ident: Some("27".to_string()),
            he_lat_deg: Some(he_lat),
            he_lon_deg: Some(he_lon),
            he_heading_deg: Some(270.0),
            length_ft: Some(10_000),
            width_ft: Some(150),
            surface: Some("ASP".to_string()),
        }
    }

    // ---- Marker mesh --------------------------------------------------------------------------

    #[test]
    fn marker_mesh_is_a_fan_of_the_expected_size() {
        let mesh = marker_mesh();
        assert_eq!(mesh.vertices.len(), MARKER_CIRCLE_SEGMENTS + 1);
        assert_eq!(mesh.indices.len(), MARKER_CIRCLE_SEGMENTS * 3);
        assert_eq!(mesh.indices.len() % 3, 0);
    }

    #[test]
    fn marker_mesh_ring_points_sit_on_the_unit_circle() {
        let mesh = marker_mesh();
        // Vertex 0 is the fan's center.
        assert_eq!(mesh.vertices[0].position, [0.0, 0.0]);
        for vertex in &mesh.vertices[1..] {
            let [x, y] = vertex.position;
            let radius = (x * x + y * y).sqrt();
            assert!(
                (radius - 1.0).abs() < 1e-5,
                "ring point off the unit circle: {radius}"
            );
        }
    }

    // ---- Marker scale --------------------------------------------------------------------------

    #[test]
    fn marker_scale_is_positive_and_doubles_with_meters_per_pixel() {
        let a = airport_marker_scale_normalized(10.0);
        let b = airport_marker_scale_normalized(20.0);
        assert!(a > 0.0);
        assert!((b - 2.0 * a).abs() < 1e-9);
    }

    // ---- Instance packing ----------------------------------------------------------------------

    #[test]
    fn pack_airport_instances_projects_each_airport_and_reuses_the_output_buffer() {
        let airports = vec![test_airport(40.0, -74.0), test_airport(51.5, -0.1)];
        let mut out = Vec::new();
        pack_airport_instances(&airports, &mut out);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].world_xy, project(40.0, -74.0));
        assert_eq!(out[1].world_xy, project(51.5, -0.1));

        // A second call clears rather than appends (ADR-002: no per-frame growth).
        pack_airport_instances(&[], &mut out);
        assert!(out.is_empty());
    }

    // ---- Runway tessellation -------------------------------------------------------------------

    #[test]
    fn a_complete_runway_produces_geometry() {
        let runways = vec![full_runway(40.0, -74.0, 40.01, -74.0)];
        let mut out = VertexBuffers::new();
        tessellate_runways(&runways, TEST_METERS_PER_PIXEL, &mut out);

        assert!(!out.vertices.is_empty());
        assert!(!out.indices.is_empty());
        assert_eq!(out.indices.len() % 3, 0);
    }

    #[test]
    fn a_runway_missing_either_end_draws_nothing() {
        let mut runway_no_high_end = full_runway(40.0, -74.0, 40.01, -74.0);
        runway_no_high_end.he_lat_deg = None;
        let mut runway_no_low_end = full_runway(40.0, -74.0, 40.01, -74.0);
        runway_no_low_end.le_lon_deg = None;

        let mut out = VertexBuffers::new();
        tessellate_runways(
            &[runway_no_high_end, runway_no_low_end],
            TEST_METERS_PER_PIXEL,
            &mut out,
        );

        assert!(
            out.vertices.is_empty(),
            "an incomplete runway must draw nothing"
        );
        assert!(out.indices.is_empty());
    }

    #[test]
    fn a_zero_length_runway_draws_nothing() {
        let runways = vec![full_runway(40.0, -74.0, 40.0, -74.0)];
        let mut out = VertexBuffers::new();
        tessellate_runways(&runways, TEST_METERS_PER_PIXEL, &mut out);

        assert!(
            out.vertices.is_empty(),
            "a degenerate runway must draw nothing"
        );
        assert!(out.indices.is_empty());
    }

    #[test]
    fn tessellate_runways_reuses_the_output_buffer() {
        let runways = vec![full_runway(40.0, -74.0, 40.01, -74.0)];
        let mut out = VertexBuffers::new();
        tessellate_runways(&runways, TEST_METERS_PER_PIXEL, &mut out);
        assert!(!out.vertices.is_empty());

        // A second call with no runways clears rather than appends.
        tessellate_runways(&[], TEST_METERS_PER_PIXEL, &mut out);
        assert!(out.vertices.is_empty());
        assert!(out.indices.is_empty());
    }

    #[test]
    fn two_complete_runways_produce_more_geometry_than_one() {
        let one = vec![full_runway(40.0, -74.0, 40.01, -74.0)];
        let mut one_out = VertexBuffers::new();
        tessellate_runways(&one, TEST_METERS_PER_PIXEL, &mut one_out);

        let two = vec![
            full_runway(40.0, -74.0, 40.01, -74.0),
            full_runway(51.5, -0.1, 51.51, -0.1),
        ];
        let mut two_out = VertexBuffers::new();
        tessellate_runways(&two, TEST_METERS_PER_PIXEL, &mut two_out);

        assert!(two_out.indices.len() > one_out.indices.len());
    }

    /// Regression for the exact bug this module's own stroke-width judgement-call doc comment
    /// warns about: a stroke width fixed in world-plane units (like the coastline's) can exceed a
    /// short runway's own segment length and `lyon` silently tessellates to nothing rather than
    /// erroring — [`runway_stroke_width_normalized`]'s screen-constant scaling must keep the
    /// stroke comfortably narrower than a realistically short runway at ordinary L2 zoom.
    #[test]
    fn a_short_realistic_runway_still_produces_geometry_at_l2_zoom() {
        // ~300 m runway (a short GA strip): roughly 111,320 m per degree of latitude, so this
        // spans about 0.0027 degrees — short enough that a fixed-world-plane stroke width (like
        // the coastline's) would risk swallowing it entirely (see this module's own doc comment).
        const SHORT_RUNWAY_LAT_SPAN_DEG: f64 = 0.0027;
        let runways = vec![full_runway(
            40.0,
            -74.0,
            40.0 + SHORT_RUNWAY_LAT_SPAN_DEG,
            -74.0,
        )];
        let mut out = VertexBuffers::new();
        tessellate_runways(&runways, TEST_METERS_PER_PIXEL, &mut out);

        assert!(
            !out.vertices.is_empty(),
            "a realistic short runway must still produce geometry at L2 zoom"
        );
    }
}
