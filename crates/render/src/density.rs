//! CPU-side packing for the L0 density-dot pass (M4 item 4.3): projects each `RenderFeed`
//! aircraft onto the orthographic globe's unit disk around the globe camera's current
//! sub-observer point, culling anything on the far hemisphere. Pure, testable Rust —
//! `renderer.rs` owns the GPU resources (pipeline, instance buffer) built from what this module
//! produces, the same CPU/GPU split `aircraft.rs`/`airport.rs` already have.
//!
//! Skill/docs/01: L0 draws "additive-blended dots (2 px), brightness proportional to local
//! count" — this module supplies the *position* half of that (screen-constant 2 px quads); the
//! "brightness proportional to count" half falls out of `renderer.rs`'s additive `BlendState`
//! summing overlapping instances in the framebuffer, not any CPU-side density binning.

use std::mem::size_of;

use look_above_core::geo::{LatLon, orthographic_forward, web_mercator_inverse};
use look_above_core::sim::AircraftInstance;

/// On-screen size of one density dot, physical pixels, screen-constant regardless of the globe
/// camera's zoom — the skill's own "additive-blended dots (2 px)" spec.
pub const DENSITY_DOT_PX: f64 = 2.0;

/// Starting capacity (in instances) for the density-dot GPU instance buffer, before any frame has
/// grown it — matches [`crate::aircraft`]'s own `MIN_INSTANCE_CAPACITY` reasoning (small enough
/// to cost nothing at startup, large enough that a first busy region rarely forces an immediate
/// regrow).
pub const MIN_DENSITY_INSTANCE_CAPACITY: usize = 256;

/// One shared quad vertex (`@location(0)`) for the density pipeline, spanning `[-0.5, 0.5]` on
/// both axes — the density analogue of [`crate::aircraft::QuadVertex`], minus the UV attribute
/// (a density dot has no texture, just a flat additive color).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DensityQuadVertex {
    pub local_pos: [f32; 2],
}

impl DensityQuadVertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<DensityQuadVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }],
    };
}

/// The shared quad's four corners — same winding as [`crate::aircraft::quad_vertices`].
pub fn quad_vertices() -> [DensityQuadVertex; 4] {
    [
        DensityQuadVertex {
            local_pos: [-0.5, -0.5],
        },
        DensityQuadVertex {
            local_pos: [0.5, -0.5],
        },
        DensityQuadVertex {
            local_pos: [0.5, 0.5],
        },
        DensityQuadVertex {
            local_pos: [-0.5, 0.5],
        },
    ]
}

/// Two triangles covering [`quad_vertices`]'s square — identical shape to
/// [`crate::aircraft::QUAD_INDICES`].
pub const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

/// One density dot's packed per-instance GPU attribute: its position on the globe's unit disk
/// ([`look_above_core::geo::UnitDiskXy`], narrowed to `f32`) — no heading, no per-instance color
/// (flat, set once via the pass's own `@group(1)` params uniform, like
/// [`crate::airport::AirportInstanceRaw`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DensityDotInstanceRaw {
    pub disk_xy: [f32; 2],
}

impl DensityDotInstanceRaw {
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

/// The disk-space (pre-scale) size a screen-constant [`DENSITY_DOT_PX`]-pixel dot must be drawn
/// at, given the globe camera's current `radius_px` — the same "pixels -> local-plane units"
/// shape [`crate::aircraft::glyph_scale_normalized`]/`crate::airport::airport_marker_scale_normalized`
/// use, just against the globe's own `radius_px` directly (the unit disk's radius *is* the
/// pixels-per-plane-unit conversion — no separate `meters_per_pixel`/extent pair needed).
pub fn density_dot_scale_normalized(radius_px: f64) -> f32 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a screen-pixel-scale size divided by a physical pixel radius is a tiny \
                  fraction, nowhere near f32's precision limits"
    )]
    {
        (DENSITY_DOT_PX / radius_px) as f32
    }
}

/// Packs every visible aircraft in `aircraft` into this frame's density-dot instance buffer,
/// appending into `out` (cleared first, capacity reused frame to frame per ADR-002 — the same
/// reused-scratch shape as [`crate::aircraft::pack_instances`]).
///
/// An aircraft whose position is on the far hemisphere from `globe_center` —
/// [`orthographic_forward`] returning `None` — contributes no instance; see that function's own
/// doc comment. `aircraft`'s positions are Web Mercator
/// ([`look_above_core::geo::MercatorXy`]), so each is first unprojected back to lat/lon via
/// [`web_mercator_inverse`] before the orthographic projection.
pub fn pack_density_dots(
    aircraft: &[AircraftInstance],
    globe_center: LatLon,
    out: &mut Vec<DensityDotInstanceRaw>,
) {
    out.clear();
    out.extend(aircraft.iter().filter_map(|instance| {
        let latlon = web_mercator_inverse(instance.position);
        orthographic_forward(globe_center, latlon).map(|disk| {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "UnitDiskXy components stay within [-1, 1] by construction, far inside \
                          f32's precision"
            )]
            {
                DensityDotInstanceRaw {
                    disk_xy: [disk.x as f32, disk.y as f32],
                }
            }
        })
    }));
}

#[cfg(test)]
mod tests {
    use look_above_core::contracts::AircraftCategory;
    use look_above_core::geo::{MercatorXy, web_mercator_forward};
    use look_above_core::sim::AltitudeBucket;
    use look_above_core::types::{Icao24, SourceId};

    use super::*;

    fn some_icao() -> Icao24 {
        Icao24::from_hex("3c6444").expect("valid test ICAO24")
    }

    fn aircraft_at(position: MercatorXy) -> AircraftInstance {
        AircraftInstance {
            icao24: some_icao(),
            position,
            heading_deg: 0.0,
            altitude_bucket: AltitudeBucket::To28000Ft,
            category: AircraftCategory::Unknown,
            alpha: 1.0,
            on_ground: false,
            anonymous: false,
            callsign: None,
            altitude_ft: None,
            ground_speed_kt: None,
            selected: false,
            source: SourceId::OpenSky,
        }
    }

    // ---- density_dot_scale_normalized ----------------------------------------------------------

    #[test]
    fn dot_scale_is_positive_and_halves_as_radius_doubles() {
        let a = density_dot_scale_normalized(100.0);
        let b = density_dot_scale_normalized(200.0);
        assert!(a > 0.0);
        assert!((b - a / 2.0).abs() < 1e-9);
    }

    // ---- pack_density_dots: far-hemisphere culling ---------------------------------------------

    #[test]
    fn an_aircraft_on_the_near_hemisphere_produces_one_instance() {
        let globe_center = LatLon::new(0.0, 0.0);
        // Directly under the sub-observer point.
        let aircraft = vec![aircraft_at(MercatorXy::new(0.0, 0.0))];

        let mut out = Vec::new();
        pack_density_dots(&aircraft, globe_center, &mut out);

        assert_eq!(out.len(), 1);
        assert!(out[0].disk_xy[0].abs() < 1e-6);
        assert!(out[0].disk_xy[1].abs() < 1e-6);
    }

    #[test]
    fn an_aircraft_on_the_far_hemisphere_produces_no_instance() {
        let globe_center = LatLon::new(0.0, 0.0);
        // Antipodal to the sub-observer point: squarely on the far hemisphere.
        let antipodal = web_mercator_forward(LatLon::new(0.0, 180.0));
        let aircraft = vec![aircraft_at(antipodal)];

        let mut out = Vec::new();
        pack_density_dots(&aircraft, globe_center, &mut out);

        assert!(
            out.is_empty(),
            "an antipodal aircraft must not produce a density dot"
        );
    }

    #[test]
    fn pack_density_dots_culls_only_the_far_hemisphere_aircraft_in_a_mixed_set() {
        let globe_center = LatLon::new(0.0, 0.0);
        let near = aircraft_at(MercatorXy::new(0.0, 0.0));
        let far = aircraft_at(web_mercator_forward(LatLon::new(0.0, 180.0)));

        let mut out = Vec::new();
        pack_density_dots(&[near, far], globe_center, &mut out);

        assert_eq!(
            out.len(),
            1,
            "only the near-hemisphere aircraft should produce a dot"
        );
    }

    #[test]
    fn pack_density_dots_reuses_the_output_buffer() {
        let globe_center = LatLon::new(0.0, 0.0);
        let mut out = Vec::new();
        pack_density_dots(
            &[aircraft_at(MercatorXy::new(0.0, 0.0))],
            globe_center,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        pack_density_dots(&[], globe_center, &mut out);
        assert!(out.is_empty());
    }

    // ---- Static geometry ------------------------------------------------------------------------

    #[test]
    fn quad_vertices_span_the_unit_square() {
        for vertex in quad_vertices() {
            assert!((-0.5..=0.5).contains(&vertex.local_pos[0]));
            assert!((-0.5..=0.5).contains(&vertex.local_pos[1]));
        }
        assert_eq!(QUAD_INDICES.len(), 6);
    }
}
