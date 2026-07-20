//! CPU-side geometry, per-instance packing, and pure rotation/scale math for the aircraft glyph
//! pass (M2 item 2.5). `renderer.rs` owns the GPU resources (pipeline, atlas texture, instance
//! buffer) built from what this module produces; nothing here touches `wgpu` devices/queues, so
//! all of it is plain, testable Rust.
//!
//! Draw order (docs/01): map base → map lines → trails → **aircraft glyphs** → labels → UI.
//! Trails land in 2.6b (drawn just before this pass); labels (2.7) don't exist yet.

use std::mem::size_of;

use look_above_core::geo::{MercatorXy, WEB_MERCATOR_EXTENT_M};
use look_above_core::sim::AircraftInstance;

use crate::{color, glyph_atlas};

/// The L2 tier's glyph size, in on-screen pixels (docs/01/skill: L2 glyphs are 16–24 px; 2.3a's
/// camera is regional-only and 2.5 draws every aircraft as one fixed-size L2-style glyph — LOD
/// tiers are out of scope here, see the M2 2.5 decision-log entry). A judgement call within that
/// range, not a spec'd exact value.
pub const AIRCRAFT_GLYPH_PX: f64 = 20.0;

/// The skill's white selection-outline thickness, in on-screen physical pixels ("Selected: white
/// outline (2 px)") — see [`SELECTION_OUTLINE_SCALE_MUL`]'s own doc comment for how this becomes
/// a per-instance scale rather than a shader-side offset.
pub const SELECTION_OUTLINE_WIDTH_PX: f64 = 2.0;

/// Per-instance scale multiplier for [`pack_selection_outline_instance`]'s outline copy of a
/// glyph. Drawing the *same* silhouette scaled up by this factor, solid white, behind the
/// normal-size glyph (see [`pack_instances`]'s draw-order doc comment) leaves roughly
/// [`SELECTION_OUTLINE_WIDTH_PX`] of it visible as a highlight, without a second shader/pipeline.
///
/// Not a true uniform-width offset (a non-circular silhouette scaled up shows more of a halo at
/// points further from its own center, e.g. a dart's nose vs. its waist) — a per-texel SDF
/// threshold shrink would be exact, but `glyph_atlas`'s `SPREAD` is deliberately tuned tight (so
/// adjacent atlas tiles never bleed under bilinear filtering) and has no distance gradient left
/// this far inside a silhouette to threshold against.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a small ratio just above 1.0 (AIRCRAFT_GLYPH_PX and SELECTION_OUTLINE_WIDTH_PX are \
              both small compile-time literals), far inside f32's precision"
)]
const SELECTION_OUTLINE_SCALE_MUL: f32 =
    ((AIRCRAFT_GLYPH_PX + 2.0 * SELECTION_OUTLINE_WIDTH_PX) / AIRCRAFT_GLYPH_PX) as f32;

/// Starting capacity (in instances) for the GPU instance buffer, before any frame has grown it.
/// Small enough to cost nothing at startup, large enough that a first busy region rarely forces
/// an immediate regrow.
pub const MIN_INSTANCE_CAPACITY: usize = 256;

/// One vertex of the shared unit quad every aircraft instance reuses. Local space spans
/// `[-0.5, 0.5]` on both axes, `+y` = the glyph's nose/front (`glyph_atlas`'s convention); `uv`
/// is this corner's atlas coordinate *within one tile* (`aircraft.wgsl` offsets it into the
/// instance's category tile).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct QuadVertex {
    pub local_pos: [f32; 2],
    pub local_uv: [f32; 2],
}

impl QuadVertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<QuadVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 1,
            },
        ],
    };
}

/// The shared quad's four corners, wound so [`QUAD_INDICES`] draws two triangles covering it.
/// `local_uv`'s `v` is flipped relative to `local_pos.y` (`y = +0.5` ⇒ `v = 0`) so the atlas's
/// row 0 (top of the rasterized image — see `glyph_atlas::texel_to_local`) lands at the glyph's
/// nose, matching `glyph_atlas`'s texel-to-local-space mapping.
pub fn quad_vertices() -> [QuadVertex; 4] {
    [
        QuadVertex {
            local_pos: [-0.5, -0.5],
            local_uv: [0.0, 1.0],
        },
        QuadVertex {
            local_pos: [0.5, -0.5],
            local_uv: [1.0, 1.0],
        },
        QuadVertex {
            local_pos: [0.5, 0.5],
            local_uv: [1.0, 0.0],
        },
        QuadVertex {
            local_pos: [-0.5, 0.5],
            local_uv: [0.0, 0.0],
        },
    ]
}

/// Two triangles covering [`quad_vertices`]'s square. `u16` — six indices never need `u32`.
pub const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

/// One aircraft's packed per-instance GPU attributes — `renderer.rs` uploads a `Vec` of these,
/// one per [`AircraftInstance`] in the frame's `RenderFeed`, as the instance-stepped half of the
/// aircraft pipeline's vertex input. Built by [`pack_instance`].
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
    /// World position, Web Mercator metres divided by [`WEB_MERCATOR_EXTENT_M`] — the same
    /// pre-normalized plane `camera_view_proj` and `basemap::project_point` both operate on.
    pub world_xy: [f32; 2],
    /// Display heading, radians, clockwise from north (glyph rotation — see
    /// [`rotate_clockwise_from_north`]'s doc comment for the sign convention `aircraft.wgsl`
    /// mirrors).
    pub heading_rad: f32,
    /// [`glyph_atlas::category_index`]'s tile index, carried as an `f32` (matching every other
    /// instance attribute's format) — `aircraft.wgsl` recovers the atlas UV offset from it.
    pub category_index: f32,
    /// Altitude-bucket tint color, `.rgb`, with the stale-fade alpha already folded into `.a`
    /// (see [`pack_instance`]) — the fragment shader multiplies this by the SDF edge alpha, not
    /// the other way around.
    pub tint: [f32; 4],
    /// Multiplies [`crate::renderer`]'s per-frame `glyph_scale` uniform for *this* instance only
    /// (M2 item 2.8b). `1.0` for every ordinary glyph; [`SELECTION_OUTLINE_SCALE_MUL`] for a
    /// selected aircraft's outline copy — see [`pack_selection_outline_instance`].
    pub scale_mul: f32,
}

impl InstanceRaw {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<InstanceRaw>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: (size_of::<[f32; 2]>() + size_of::<f32>()) as wgpu::BufferAddress,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: (size_of::<[f32; 2]>() + 2 * size_of::<f32>()) as wgpu::BufferAddress,
                shader_location: 5,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: (size_of::<[f32; 2]>() + 2 * size_of::<f32>() + size_of::<[f32; 4]>())
                    as wgpu::BufferAddress,
                shader_location: 6,
            },
        ],
    };
}

/// Rotates a local glyph-space offset by `heading_deg` clockwise from north — the exact formula
/// `aircraft.wgsl`'s vertex shader mirrors on the GPU.
///
/// Both Web Mercator's `y` (north-positive) and clip space's `y` (up-positive) point the same
/// way (see `renderer.rs::camera_view_proj`'s doc comment — no axis flip sits between world and
/// screen here), so "clockwise from north" in geography and "clockwise on screen" coincide, and
/// a single rotation formula serves both: at `heading = 0`, local `(0, 1)` ("north" in glyph
/// space) stays put; at `heading = 90`, it rotates onto `(1, 0)` ("east"); at 180 onto `(0, -1)`
/// ("south"); at 270 onto `(-1, 0)` ("west").
///
/// The actual rotation happens on the GPU (WGSL can't be exercised by `cargo test`); this
/// function exists so that exact formula has a plain-Rust twin the rotation unit tests below can
/// pin — not to be called from this crate's own runtime path.
#[allow(
    dead_code,
    reason = "kept as a unit-testable mirror of aircraft.wgsl's vertex-shader rotation math, \
              which cargo test cannot exercise directly"
)]
pub fn rotate_clockwise_from_north(local: [f32; 2], heading_deg: f64) -> [f32; 2] {
    let theta = heading_deg.to_radians();
    let (sin_t, cos_t) = (theta.sin(), theta.cos());
    let x = f64::from(local[0]);
    let y = f64::from(local[1]);
    let rotated_x = x * cos_t + y * sin_t;
    let rotated_y = -x * sin_t + y * cos_t;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "rotating a unit-ish local offset by a bounded angle stays within f32's range; \
                  the reduced precision is invisible at glyph scale"
    )]
    [rotated_x as f32, rotated_y as f32]
}

/// Narrows a Web Mercator position to the pre-normalized `[-1, 1]`-ish plane the view-proj
/// matrix and the static base-map mesh both work in — the same `metres / EXTENT` division
/// `basemap::project_point` performs, applied here to the (per-frame, dynamic) aircraft
/// instance data instead of the (static, build-once) base-map mesh.
pub fn world_xy_normalized(position: MercatorXy) -> [f32; 2] {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Web Mercator metres divided by WEB_MERCATOR_EXTENT_M land in roughly [-1, 1]; \
                  f32 has ample precision left over at that magnitude for a screen-space glyph"
    )]
    [
        (position.x_m / WEB_MERCATOR_EXTENT_M) as f32,
        (position.y_m / WEB_MERCATOR_EXTENT_M) as f32,
    ]
}

/// The world-space (pre-normalized-plane) size a screen-constant [`AIRCRAFT_GLYPH_PX`]-pixel
/// glyph must be drawn at, given the camera's current `meters_per_pixel`.
///
/// This is what keeps the glyph a fixed screen-space size regardless of zoom (LOD tiers are out
/// of scope for 2.5 — see this module's doc comment): `AIRCRAFT_GLYPH_PX * meters_per_pixel` is
/// the glyph's world-metre size at the current zoom, and dividing by [`WEB_MERCATOR_EXTENT_M`]
/// puts it on the same normalized plane the local quad offsets (`aircraft.wgsl`) are scaled
/// into before the view-proj matrix is applied.
pub fn glyph_scale_normalized(meters_per_pixel: f64) -> f32 {
    let world_m = AIRCRAFT_GLYPH_PX * meters_per_pixel;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a screen-pixel-scale glyph size in normalized-plane units is a tiny fraction, \
                  nowhere near f32's precision limits"
    )]
    {
        (world_m / WEB_MERCATOR_EXTENT_M) as f32
    }
}

/// Packs one `RenderFeed` aircraft into its GPU instance attributes: position narrowed to the
/// normalized plane, heading converted to radians, category resolved to its atlas tile, and the
/// altitude-bucket tint's alpha multiplied by the instance's own stale-fade `alpha` — the one
/// place those two independent alphas combine before reaching the shader.
///
/// `tint_table` is indexed by [`color::altitude_bucket_index`] — built once per surface format
/// in `renderer.rs` (the six colors don't change frame to frame, only which one applies).
pub fn pack_instance(instance: &AircraftInstance, tint_table: &[[f32; 4]; 6]) -> InstanceRaw {
    let world_xy = world_xy_normalized(instance.position);

    #[allow(
        clippy::cast_possible_truncation,
        reason = "a display heading in degrees converts to a bounded radian value; f32 keeps \
                  ample precision for a glyph's on-screen rotation"
    )]
    let heading_rad = instance.heading_deg.to_radians() as f32;

    #[allow(
        clippy::cast_precision_loss,
        reason = "category index is one of 6 small integers (0..=5), far inside f32's \
                  exact-integer range"
    )]
    let category_index = glyph_atlas::category_index(instance.category) as f32;

    let mut tint = tint_table[color::altitude_bucket_index(instance.altitude_bucket)];
    #[allow(
        clippy::cast_possible_truncation,
        reason = "core::sim already clamps alpha to (0, 1] before an instance ever reaches the \
                  feed"
    )]
    {
        tint[3] *= instance.alpha as f32;
    }

    InstanceRaw {
        world_xy,
        heading_rad,
        category_index,
        tint,
        scale_mul: 1.0,
    }
}

/// Packs one selected `instance`'s outline copy (M2 item 2.8b): same position/heading/category
/// (so the silhouette matches exactly), solid white, scaled up by
/// [`SELECTION_OUTLINE_SCALE_MUL`] — see that constant's doc comment.
///
/// `instance.alpha` is folded into the outline's alpha too, the same way [`pack_instance`] folds
/// it into the tint: a selected aircraft mid-stale-fade should have its outline fade with it,
/// not stay solid after the glyph itself has faded.
fn pack_selection_outline_instance(instance: &AircraftInstance) -> InstanceRaw {
    let world_xy = world_xy_normalized(instance.position);

    #[allow(
        clippy::cast_possible_truncation,
        reason = "a display heading in degrees converts to a bounded radian value; f32 keeps \
                  ample precision for a glyph's on-screen rotation"
    )]
    let heading_rad = instance.heading_deg.to_radians() as f32;

    #[allow(
        clippy::cast_precision_loss,
        reason = "category index is one of 6 small integers (0..=5), far inside f32's \
                  exact-integer range"
    )]
    let category_index = glyph_atlas::category_index(instance.category) as f32;

    #[allow(
        clippy::cast_possible_truncation,
        reason = "core::sim already clamps alpha to (0, 1] before an instance ever reaches the \
                  feed"
    )]
    let alpha = instance.alpha as f32;

    InstanceRaw {
        world_xy,
        heading_rad,
        category_index,
        tint: [1.0, 1.0, 1.0, alpha],
        scale_mul: SELECTION_OUTLINE_SCALE_MUL,
    }
}

/// Packs every drawable instance for this frame's aircraft pass, appending into `out` (cleared
/// first, its capacity reused frame to frame per ADR-002 — the same reused-scratch shape as
/// [`crate::trail::tessellate_trails`]).
///
/// Selection outlines (M2 item 2.8b) are packed *first*, ordinary glyphs after: this pass has no
/// depth test (alpha-blended, painter's-algorithm draw order), so an outline instance must be
/// earlier in the buffer than the normal-size glyph drawn on top of it, or the larger white copy
/// would occlude rather than merely peek out from behind. In practice at most one aircraft is
/// selected at a time ([`look_above_core::sim::Simulator`]'s own `selected: Option<Icao24>`), so
/// this never packs more than one outline instance — but the loop handles zero or several
/// (defensively; nothing in this crate requires exactly one) identically.
pub fn pack_instances(
    aircraft: &[AircraftInstance],
    tint_table: &[[f32; 4]; 6],
    out: &mut Vec<InstanceRaw>,
) {
    out.clear();
    out.extend(
        aircraft
            .iter()
            .filter(|instance| instance.selected)
            .map(pack_selection_outline_instance),
    );
    out.extend(
        aircraft
            .iter()
            .map(|instance| pack_instance(instance, tint_table)),
    );
}

#[cfg(test)]
mod tests {
    use look_above_core::contracts::AircraftCategory;
    use look_above_core::sim::AltitudeBucket;
    use look_above_core::types::{Icao24, SourceId};

    use super::*;

    const EPS: f32 = 1e-5;

    #[track_caller]
    fn assert_close2(actual: [f32; 2], expected: [f32; 2]) {
        assert!(
            (actual[0] - expected[0]).abs() < EPS && (actual[1] - expected[1]).abs() < EPS,
            "expected {expected:?}, got {actual:?}"
        );
    }

    // ---- Rotation ---------------------------------------------------------------------------

    #[test]
    fn rotation_hits_the_four_cardinal_points() {
        let north = [0.0, 1.0];
        assert_close2(rotate_clockwise_from_north(north, 0.0), [0.0, 1.0]);
        assert_close2(rotate_clockwise_from_north(north, 90.0), [1.0, 0.0]);
        assert_close2(rotate_clockwise_from_north(north, 180.0), [0.0, -1.0]);
        assert_close2(rotate_clockwise_from_north(north, 270.0), [-1.0, 0.0]);
    }

    #[test]
    fn rotation_wraps_cleanly_at_a_full_turn() {
        let point = [0.3, 0.4];
        assert_close2(
            rotate_clockwise_from_north(point, 0.0),
            rotate_clockwise_from_north(point, 360.0),
        );
    }

    // ---- Normalized plane narrowing ----------------------------------------------------------

    #[test]
    fn world_xy_normalized_lands_at_the_expected_fraction_of_the_extent() {
        let [x, y] = world_xy_normalized(MercatorXy::new(
            WEB_MERCATOR_EXTENT_M / 2.0,
            -WEB_MERCATOR_EXTENT_M / 4.0,
        ));
        assert!((x - 0.5).abs() < 1e-6);
        assert!((y - (-0.25)).abs() < 1e-6);
    }

    #[test]
    fn world_xy_normalized_reaches_exactly_one_at_the_antimeridian() {
        let [x, _y] = world_xy_normalized(MercatorXy::new(WEB_MERCATOR_EXTENT_M, 0.0));
        assert!((x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn world_xy_normalized_round_trips_through_the_extent_division_sanely() {
        for (x_m, y_m) in [
            (0.0, 0.0),
            (12_345.0, -98_765.0),
            (WEB_MERCATOR_EXTENT_M, WEB_MERCATOR_EXTENT_M),
            (-WEB_MERCATOR_EXTENT_M, -WEB_MERCATOR_EXTENT_M),
        ] {
            let [x, y] = world_xy_normalized(MercatorXy::new(x_m, y_m));
            assert!(x.is_finite() && y.is_finite());
            assert!(
                (-1.000_1..=1.000_1).contains(&x),
                "x {x} escaped the normalized plane"
            );
            assert!(
                (-1.000_1..=1.000_1).contains(&y),
                "y {y} escaped the normalized plane"
            );
            // The division is exact enough that re-multiplying by the extent recovers the
            // original metres within f32's precision at this magnitude.
            let recovered_x = f64::from(x) * WEB_MERCATOR_EXTENT_M;
            assert!(
                (recovered_x - x_m).abs() < 10.0,
                "x round-trip drifted for {x_m}"
            );
        }
    }

    // ---- Glyph scale --------------------------------------------------------------------------

    #[test]
    fn glyph_scale_is_positive_and_doubles_with_meters_per_pixel() {
        let a = glyph_scale_normalized(10.0);
        let b = glyph_scale_normalized(20.0);
        assert!(a > 0.0);
        assert!((b - 2.0 * a).abs() < 1e-9);
    }

    // ---- Instance packing ---------------------------------------------------------------------

    fn some_icao() -> Icao24 {
        Icao24::from_hex("3c6444").expect("valid test ICAO24")
    }

    #[test]
    fn pack_instance_carries_position_heading_category_and_folds_in_the_stale_fade_alpha() {
        let opaque_white_table = [[1.0_f32, 1.0, 1.0, 1.0]; 6];
        let instance = AircraftInstance {
            icao24: some_icao(),
            position: MercatorXy::new(1_000.0, 2_000.0),
            heading_deg: 90.0,
            altitude_bucket: AltitudeBucket::To28000Ft,
            category: AircraftCategory::Heli,
            alpha: 0.5,
            on_ground: false,
            anonymous: false,
            callsign: None,
            altitude_ft: None,
            ground_speed_kt: None,
            selected: false,
            source: SourceId::OpenSky,
        };

        let raw = pack_instance(&instance, &opaque_white_table);

        #[allow(
            clippy::cast_possible_truncation,
            reason = "test-only expectations for a value pack_instance itself narrows the same way"
        )]
        let (expected_x, expected_y) = (
            (1_000.0 / WEB_MERCATOR_EXTENT_M) as f32,
            (2_000.0 / WEB_MERCATOR_EXTENT_M) as f32,
        );
        assert!((raw.world_xy[0] - expected_x).abs() < 1e-6);
        assert!((raw.world_xy[1] - expected_y).abs() < 1e-6);
        assert!((raw.heading_rad - 90.0_f32.to_radians()).abs() < 1e-5);

        #[allow(
            clippy::cast_precision_loss,
            reason = "category index is one of 6 small integers (0..=5), far inside f32's exact-integer range"
        )]
        let expected_category_index = glyph_atlas::category_index(AircraftCategory::Heli) as f32;
        assert!((raw.category_index - expected_category_index).abs() < 1e-6);
        assert!(
            (raw.tint[3] - 0.5).abs() < 1e-6,
            "the instance's stale-fade alpha must reach the packed tint"
        );
        assert!(
            (raw.tint[0] - 1.0).abs() < 1e-6,
            "rgb passes through the tint table untouched"
        );
    }

    #[test]
    fn pack_instance_selects_the_tint_table_entry_for_its_altitude_bucket() {
        let mut table = [[0.0_f32; 4]; 6];
        table[color::altitude_bucket_index(AltitudeBucket::Above40000Ft)] = [0.5, 0.6, 0.7, 1.0];
        let instance = AircraftInstance {
            icao24: some_icao(),
            position: MercatorXy::new(0.0, 0.0),
            heading_deg: 0.0,
            altitude_bucket: AltitudeBucket::Above40000Ft,
            category: AircraftCategory::Unknown,
            alpha: 1.0,
            on_ground: false,
            anonymous: false,
            callsign: None,
            altitude_ft: None,
            ground_speed_kt: None,
            selected: false,
            source: SourceId::OpenSky,
        };

        let raw = pack_instance(&instance, &table);
        assert!((raw.tint[0] - 0.5).abs() < 1e-6);
        assert!((raw.tint[1] - 0.6).abs() < 1e-6);
        assert!((raw.tint[2] - 0.7).abs() < 1e-6);
    }

    #[test]
    fn pack_instance_leaves_scale_mul_at_one() {
        let opaque_white_table = [[1.0_f32; 4]; 6];
        let instance = AircraftInstance {
            icao24: some_icao(),
            position: MercatorXy::new(0.0, 0.0),
            heading_deg: 0.0,
            altitude_bucket: AltitudeBucket::To28000Ft,
            category: AircraftCategory::Unknown,
            alpha: 1.0,
            on_ground: false,
            anonymous: false,
            callsign: None,
            altitude_ft: None,
            ground_speed_kt: None,
            selected: true,
            source: SourceId::OpenSky,
        };
        assert!((pack_instance(&instance, &opaque_white_table).scale_mul - 1.0).abs() < 1e-9);
    }

    // ---- Selection outline (M2 item 2.8b) --------------------------------------------------------

    fn selected_instance() -> AircraftInstance {
        AircraftInstance {
            icao24: some_icao(),
            position: MercatorXy::new(500.0, -250.0),
            heading_deg: 45.0,
            altitude_bucket: AltitudeBucket::To10000Ft,
            category: AircraftCategory::Jet,
            alpha: 0.7,
            on_ground: false,
            anonymous: false,
            callsign: None,
            altitude_ft: None,
            ground_speed_kt: None,
            selected: true,
            source: SourceId::OpenSky,
        }
    }

    #[test]
    fn pack_selection_outline_instance_is_solid_white_scaled_up_and_matches_position() {
        let instance = selected_instance();
        let raw = pack_selection_outline_instance(&instance);

        assert!(raw.tint[0..3].iter().all(|&c| (c - 1.0).abs() < 1e-6));
        #[allow(
            clippy::cast_possible_truncation,
            reason = "test-only expectation for a value pack_selection_outline_instance itself \
                      narrows the same way"
        )]
        let expected_alpha = instance.alpha as f32;
        assert!((raw.tint[3] - expected_alpha).abs() < 1e-6);
        assert!(
            raw.scale_mul > 1.0,
            "the outline must be larger than a normal glyph"
        );

        let normal = pack_instance(&instance, &[[0.0; 4]; 6]);
        assert_eq!(raw.world_xy, normal.world_xy);
        assert!((raw.heading_rad - normal.heading_rad).abs() < 1e-6);
        assert!((raw.category_index - normal.category_index).abs() < 1e-6);
    }

    #[test]
    fn pack_instances_packs_an_outline_before_the_normal_glyph_only_for_selected_aircraft() {
        let mut plain = selected_instance();
        plain.selected = false;
        plain.icao24 = Icao24::from_hex("111111").expect("valid test ICAO24");
        let selected = selected_instance();

        let table = [[0.2_f32; 4]; 6];
        let mut out = Vec::new();
        pack_instances(&[plain, selected], &table, &mut out);

        // One outline (the selected aircraft only) plus two normal glyphs (one per aircraft).
        assert_eq!(out.len(), 3);
        assert!(
            out[0].tint[0..3].iter().all(|&c| (c - 1.0).abs() < 1e-6),
            "the outline instance must come first"
        );
        assert!(out[0].scale_mul > 1.0);
        assert!(
            out[1..]
                .iter()
                .all(|raw| (raw.scale_mul - 1.0).abs() < 1e-9),
            "every ordinary glyph instance keeps scale_mul at 1.0"
        );
    }

    #[test]
    fn pack_instances_emits_no_outline_when_nothing_is_selected() {
        let mut instance = selected_instance();
        instance.selected = false;
        let mut out = Vec::new();
        pack_instances(&[instance], &[[0.0; 4]; 6], &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0].scale_mul - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pack_instances_reuses_the_output_buffer() {
        let mut out = Vec::new();
        pack_instances(&[selected_instance()], &[[0.0; 4]; 6], &mut out);
        assert_eq!(out.len(), 2);
        pack_instances(&[], &[[0.0; 4]; 6], &mut out);
        assert!(out.is_empty());
    }

    // ---- Static geometry ------------------------------------------------------------------------

    #[test]
    fn quad_vertices_span_the_unit_square_with_uv_matching_the_flip() {
        let vertices = quad_vertices();
        for vertex in vertices {
            assert!((-0.5..=0.5).contains(&vertex.local_pos[0]));
            assert!((-0.5..=0.5).contains(&vertex.local_pos[1]));
            assert!((0.0..=1.0).contains(&vertex.local_uv[0]));
            assert!((0.0..=1.0).contains(&vertex.local_uv[1]));
            // The nose (+y) corners map to v = 0 (row 0 / top of the rasterized atlas).
            if vertex.local_pos[1] > 0.0 {
                assert!((vertex.local_uv[1] - 0.0).abs() < 1e-6);
            } else {
                assert!((vertex.local_uv[1] - 1.0).abs() < 1e-6);
            }
        }
        assert_eq!(QUAD_INDICES.len(), 6);
    }
}
