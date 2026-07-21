//! CPU-side geometry for the METAR flight-category badge pass (M3 item 3.3).
//!
//! Reuses [`airport::marker_mesh`]'s shared unit-circle mesh (renderer.rs builds this layer a
//! second, independent copy of the same GPU buffers, the same way [`RunwayLayer`]/
//! [`AirportMarkerLayer`] each own their own resources despite a similar shape) and
//! [`airport::project`] for lat/lon → normalized-plane projection — there is nothing badge-
//! specific about either.
//!
//! What *is* badge-specific: color is per-instance, not a flat per-layer uniform like the
//! airport marker's — four possible flight categories, docs/13's own VFR/MVFR/IFR/LIFR colors
//! ([`color::flight_category_badge_color`]) — so [`BadgeInstanceRaw`] carries a `color` field
//! `airport::AirportInstanceRaw` does not, and `metar_badge.wgsl` reads it instead of a
//! `@group(1)` uniform.
//!
//! Drawn as a ring around the airport marker rather than offset from it: [`BADGE_RADIUS_PX`] is
//! larger than [`airport::AIRPORT_MARKER_RADIUS_PX`], and `renderer.rs`'s draw order puts this
//! pass *before* the airport-marker pass, so the marker's own gray dot paints over the badge's
//! center and only a colored ring shows — no lat/lon offset math needed, and the pairing (which
//! badge belongs to which airport) stays visually unambiguous even for two airports close
//! together on screen.
//!
//! Rebuilds every frame from whatever `Vec<MetarBadge>` slice `renderer.rs` is handed, same
//! reused-scratch shape and same "simpler than tracking whether the queried set changed"
//! reasoning as `airport::pack_airport_instances`.

use std::mem::size_of;

use look_above_core::contracts::MetarBadge;
use look_above_core::geo::WEB_MERCATOR_EXTENT_M;

use crate::airport;
use crate::color;

/// On-screen radius of a badge ring, in physical pixels, screen-constant regardless of zoom —
/// larger than [`AIRPORT_MARKER_RADIUS_PX`] so the marker's own dot paints over the badge's
/// center and only a colored ring reads (see this module's own doc comment). Tuned by eye:
/// large enough to register as a distinct ring around the marker, not so large it starts
/// overlapping a neighboring airport's own badge at typical L2 zoom.
pub const BADGE_RADIUS_PX: f64 = 7.0;

// A ring shows only if the badge is drawn larger than the marker it surrounds — checked once,
// at compile time, rather than as a runtime test of two constants.
const _: () = assert!(BADGE_RADIUS_PX > airport::AIRPORT_MARKER_RADIUS_PX);

/// One badge's packed per-instance GPU attribute: position and color — unlike
/// [`airport::AirportInstanceRaw`], color is per-instance here because a badge's color depends
/// on that airport's own flight category, not a flat per-layer constant.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BadgeInstanceRaw {
    /// World position, Web Mercator metres divided by `WEB_MERCATOR_EXTENT_M` — the same
    /// pre-normalized plane every other layer's `world_xy` operates on.
    pub world_xy: [f32; 2],
    /// Opaque linear RGBA, already resolved for the render target's surface format
    /// (`color::flight_category_badge_color`) — the shader reads this directly, no per-frame
    /// uniform to bind alongside it.
    pub color: [f32; 4],
}

impl BadgeInstanceRaw {
    /// The layout `renderer.rs`'s badge pipeline binds this struct at (instance-stepped,
    /// `@location(1)`/`@location(2)` — `@location(0)` is [`airport::marker_mesh`]'s shared
    /// [`crate::basemap::Vertex`] geometry), matching `metar_badge.wgsl`'s `InstanceInput`.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 2,
            },
        ],
    };
}

/// Starting capacity for the badge GPU/CPU instance buffers — a viewport's large airports are a
/// subset of its medium+large ones, so this can start smaller than
/// [`airport::MIN_AIRPORT_INSTANCE_CAPACITY`] and still rarely need to grow.
pub const MIN_BADGE_INSTANCE_CAPACITY: usize = 32;

/// The world-space (pre-normalized-plane) radius a screen-constant [`BADGE_RADIUS_PX`]-pixel
/// badge ring must be drawn at, given the camera's current `meters_per_pixel` — same "pixels →
/// world metres → divide by the extent" shape as
/// [`airport::airport_marker_scale_normalized`].
pub fn badge_scale_normalized(meters_per_pixel: f64) -> f32 {
    let world_m = BADGE_RADIUS_PX * meters_per_pixel;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a screen-pixel-scale radius in normalized-plane units is a tiny fraction, \
                  nowhere near f32's precision limits"
    )]
    {
        (world_m / WEB_MERCATOR_EXTENT_M) as f32
    }
}

/// Packs every badge in `badges` into this frame's instance buffer, appending into `out`
/// (cleared first, capacity reused frame to frame — see this module's own doc comment on
/// rebuild cadence). `format` resolves each badge's category to this surface's own color
/// (`color::flight_category_badge_color`); with only a handful of badges in view at once,
/// resolving color per instance here is simpler than a separate per-format lookup table for no
/// measurable cost.
pub fn pack_badge_instances(
    badges: &[MetarBadge],
    format: wgpu::TextureFormat,
    out: &mut Vec<BadgeInstanceRaw>,
) {
    out.clear();
    out.extend(badges.iter().map(|badge| BadgeInstanceRaw {
        world_xy: airport::project(badge.lat_deg, badge.lon_deg),
        color: color::flight_category_badge_color(badge.category, format),
    }));
}

#[cfg(test)]
mod tests {
    use look_above_core::contracts::FlightCategory;

    use super::*;

    fn badge(lat_deg: f64, lon_deg: f64, category: FlightCategory) -> MetarBadge {
        MetarBadge {
            lat_deg,
            lon_deg,
            category,
        }
    }

    #[test]
    fn badge_scale_is_positive_and_doubles_with_meters_per_pixel() {
        let a = badge_scale_normalized(10.0);
        let b = badge_scale_normalized(20.0);
        assert!(a > 0.0);
        assert!((b - 2.0 * a).abs() < 1e-9);
    }

    #[test]
    fn pack_badge_instances_projects_and_colors_each_badge_and_reuses_the_output_buffer() {
        let badges = vec![
            badge(40.0, -74.0, FlightCategory::Vfr),
            badge(51.5, -0.1, FlightCategory::Lifr),
        ];
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let mut out = Vec::new();
        pack_badge_instances(&badges, format, &mut out);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].world_xy, airport::project(40.0, -74.0));
        assert_eq!(
            out[0].color,
            color::flight_category_badge_color(FlightCategory::Vfr, format)
        );
        assert_eq!(
            out[1].color,
            color::flight_category_badge_color(FlightCategory::Lifr, format)
        );
        assert_ne!(
            out[0].color, out[1].color,
            "VFR and LIFR must not pack the same color"
        );

        // A second call clears rather than appends (ADR-002: no per-frame growth).
        pack_badge_instances(&[], format, &mut out);
        assert!(out.is_empty());
    }
}
