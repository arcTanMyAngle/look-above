//! CPU-side ribbon tessellation for the trail pass (M2 item 2.6b) — the render half of 2.6a's
//! `RenderFeed.trails`. `renderer.rs` owns the GPU resources (pipeline, the per-frame vertex
//! buffer); nothing here touches `wgpu` devices/queues, so all of it is plain, testable Rust.
//!
//! 2.6a produces a flat `Vec<TrailVertex>` of *centerline* samples, grouped contiguously per
//! aircraft in the feed's address-sorted order. It deliberately stops there: widening a centerline
//! into a ribbon needs the camera's current `meters_per_pixel` to keep the taper a constant
//! screen-space width, and `core` has no camera (2.3a keeps it in `app`) — the same reason 2.5's
//! zoom-dependent glyph sizing (`aircraft::glyph_scale_normalized`) lives here rather than in
//! `core`. This module is that render-side widening.
//!
//! Each aircraft's contiguous run of samples becomes one continuous ribbon: every centerline
//! vertex is offset perpendicular to the local direction of travel by ±half the ribbon width,
//! and consecutive samples' offset points are stitched into a triangle list. The offset points at
//! each joint are shared between the two adjacent segments (one vertex, one color, one alpha), so
//! the ribbon has no gap and — crucially for an alpha-blended pass — no double-blended overlap at
//! joints. Width and alpha both **taper** front-to-tail with each sample's age (the skill's
//! "3 px → 0.5 px, alpha 0.8 → 0"), and each vertex is altitude-ramp colored from 2.6a's
//! per-sample `altitude_bucket`.
//!
//! Draw order (docs/01): map base → map lines → **trails** → aircraft glyphs → labels → UI. The
//! trail pass runs *before* the aircraft pass so a glyph is never occluded by its own trail.

use std::mem::size_of;

use look_above_core::geo::WEB_MERCATOR_EXTENT_M;
use look_above_core::sim::{TRAIL_DURATION_S, TrailVertex};

use crate::color;

/// Ribbon width at the head (the aircraft, age 0), in on-screen pixels — the skill's "3 px".
pub const TRAIL_WIDTH_HEAD_PX: f64 = 3.0;

/// Ribbon width at the tail (the oldest retained sample, age [`TRAIL_DURATION_S`]), in on-screen
/// pixels — the skill's "0.5 px".
pub const TRAIL_WIDTH_TAIL_PX: f64 = 0.5;

/// Ribbon alpha at the head — the skill's "0.8".
pub const TRAIL_ALPHA_HEAD: f32 = 0.8;

/// Ribbon alpha at the tail — the skill's "0". A trail vertex fades fully out by the time it
/// ages to the tail, so the ribbon dissolves into the map rather than ending on a hard edge.
pub const TRAIL_ALPHA_TAIL: f32 = 0.0;

/// Starting capacity (in vertices) for the per-frame trail vertex buffer, before any frame has
/// grown it. Six vertices per centerline segment, so this holds ~170 segments' worth before the
/// first regrow — enough for a handful of full-length regional trails without an immediate resize.
pub const MIN_TRAIL_VERTEX_CAPACITY: usize = 1024;

/// Below this squared distance (in the normalized `[-1, 1]` plane) two consecutive samples are
/// treated as coincident and the newer one is dropped: a stationary aircraft (on the ground, or
/// holding) records repeated identical displayed positions, which would otherwise produce a
/// zero-length segment with an undefined travel direction. `1e-18` here is ~2 cm on the ground —
/// far below the metres a moving aircraft covers between 1 Hz samples, so no real motion is merged.
const MIN_SEGMENT_LEN_SQ: f64 = 1e-18;

/// One tessellated trail-ribbon vertex — `renderer.rs` uploads a `Vec` of these each frame as the
/// trail pipeline's sole (per-vertex) input. The perpendicular offset and the taper are already
/// baked in by [`tessellate_trails`]; the shader only applies the shared view-proj matrix and
/// passes the color through, so the trail WGSL carries no geometry logic of its own.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TrailVertexRaw {
    /// World position, Web Mercator metres divided by [`WEB_MERCATOR_EXTENT_M`] and already
    /// offset perpendicular to the trail — the same pre-normalized plane `camera_view_proj` and
    /// the aircraft/base-map meshes all operate on.
    pub world_xy: [f32; 2],
    /// Altitude-ramp tint (`.rgb`) with the front-to-tail taper alpha folded into `.a` — the
    /// alpha-blended trail pipeline uses it directly.
    pub color: [f32; 4],
}

impl TrailVertexRaw {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<TrailVertexRaw>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 1,
            },
        ],
    };
}

/// The `0..1` taper position of a sample: `0` at the head (the aircraft, age 0), `1` at the tail
/// ([`TRAIL_DURATION_S`] old). Clamped, so a sample somehow reported past the retention window
/// still maps to the tail rather than overshooting.
fn taper_fraction(age_s: f64) -> f64 {
    (age_s / TRAIL_DURATION_S).clamp(0.0, 1.0)
}

/// The ribbon's on-screen width, in pixels, for a sample of the given age — [`TRAIL_WIDTH_HEAD_PX`]
/// at the head, [`TRAIL_WIDTH_TAIL_PX`] at the tail, linear in between.
pub fn taper_width_px(age_s: f64) -> f64 {
    let t = taper_fraction(age_s);
    TRAIL_WIDTH_HEAD_PX + (TRAIL_WIDTH_TAIL_PX - TRAIL_WIDTH_HEAD_PX) * t
}

/// The ribbon's alpha for a sample of the given age — [`TRAIL_ALPHA_HEAD`] at the head,
/// [`TRAIL_ALPHA_TAIL`] at the tail, linear in between.
pub fn taper_alpha(age_s: f64) -> f32 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "taper_fraction is a clamped [0, 1] ratio; narrowing to f32 for a color alpha \
                  loses nothing meaningful"
    )]
    let t = taper_fraction(age_s) as f32;
    TRAIL_ALPHA_HEAD + (TRAIL_ALPHA_TAIL - TRAIL_ALPHA_HEAD) * t
}

/// The *half* width, in normalized-plane units, a screen-constant `width_px`-pixel ribbon must be
/// offset by at the camera's current `meters_per_pixel` — the same "pixels → world metres →
/// divide by the extent" narrowing `aircraft::glyph_scale_normalized` performs, halved because the
/// ribbon is offset symmetrically to either side of its centerline.
pub fn half_width_normalized(width_px: f64, meters_per_pixel: f64) -> f32 {
    let half_world_m = 0.5 * width_px * meters_per_pixel;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a screen-pixel-scale width in normalized-plane units is a tiny fraction, \
                  nowhere near f32's precision limits"
    )]
    {
        (half_world_m / WEB_MERCATOR_EXTENT_M) as f32
    }
}

/// Tessellates a whole frame's trail centerlines into ribbon triangles, appending them into `out`
/// (cleared first, its capacity reused frame to frame per ADR-002's no-per-frame-allocation rule).
///
/// `trails` is grouped contiguously per aircraft (2.6a's invariant); this walks those runs and
/// builds one continuous ribbon per aircraft. `tint_table` is the six altitude-bucket tints for
/// the surface format, indexed by [`color::altitude_bucket_index`] (built once in `renderer.rs`).
pub fn tessellate_trails(
    trails: &[TrailVertex],
    tint_table: &[[f32; 4]; 6],
    meters_per_pixel: f64,
    out: &mut Vec<TrailVertexRaw>,
) {
    out.clear();
    let mut start = 0;
    while start < trails.len() {
        let icao = trails[start].icao24;
        let mut end = start + 1;
        while end < trails.len() && trails[end].icao24 == icao {
            end += 1;
        }
        tessellate_run(&trails[start..end], tint_table, meters_per_pixel, out);
        start = end;
    }
}

/// One centerline vertex, normalized and reduced to what the ribbon build needs.
struct RibbonPoint {
    xy: [f64; 2],
    age_s: f64,
    bucket_index: usize,
}

/// Tessellates one aircraft's contiguous run of samples into a continuous ribbon.
fn tessellate_run(
    run: &[TrailVertex],
    tint_table: &[[f32; 4]; 6],
    meters_per_pixel: f64,
    out: &mut Vec<TrailVertexRaw>,
) {
    // Normalize to the [-1, 1] plane in f64 (direction math wants the precision), dropping
    // consecutive coincident samples so every surviving segment has a well-defined direction.
    let mut points: Vec<RibbonPoint> = Vec::with_capacity(run.len());
    for vertex in run {
        let xy = [
            vertex.position.x_m / WEB_MERCATOR_EXTENT_M,
            vertex.position.y_m / WEB_MERCATOR_EXTENT_M,
        ];
        if let Some(previous) = points.last() {
            let dx = xy[0] - previous.xy[0];
            let dy = xy[1] - previous.xy[1];
            if dx * dx + dy * dy < MIN_SEGMENT_LEN_SQ {
                continue;
            }
        }
        points.push(RibbonPoint {
            xy,
            age_s: vertex.age_s,
            bucket_index: color::altitude_bucket_index(vertex.altitude_bucket),
        });
    }
    // A single distinct point (or none) is a dot, not a ribbon — nothing to widen.
    if points.len() < 2 {
        return;
    }

    // Unit direction of each segment (guaranteed non-zero by the coincident-sample drop above).
    let segment_dirs: Vec<[f64; 2]> = points
        .windows(2)
        .map(|pair| {
            let dx = pair[1].xy[0] - pair[0].xy[0];
            let dy = pair[1].xy[1] - pair[0].xy[1];
            let len = (dx * dx + dy * dy).sqrt();
            [dx / len, dy / len]
        })
        .collect();

    // Per-vertex left/right offset points and color, sharing joint vertices between segments.
    let count = points.len();
    let mut left: Vec<[f32; 2]> = Vec::with_capacity(count);
    let mut right: Vec<[f32; 2]> = Vec::with_capacity(count);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(count);
    for (index, point) in points.iter().enumerate() {
        let tangent = vertex_tangent(index, count, &segment_dirs);
        // Rotate the unit tangent +90° for the offset normal.
        let normal = [-tangent[1], tangent[0]];
        let half_width = f64::from(half_width_normalized(
            taper_width_px(point.age_s),
            meters_per_pixel,
        ));

        #[allow(
            clippy::cast_possible_truncation,
            reason = "a normalized-plane position offset by a screen-pixel-scale half-width stays \
                      well within f32's precision at this magnitude"
        )]
        {
            left.push([
                (point.xy[0] + normal[0] * half_width) as f32,
                (point.xy[1] + normal[1] * half_width) as f32,
            ]);
            right.push([
                (point.xy[0] - normal[0] * half_width) as f32,
                (point.xy[1] - normal[1] * half_width) as f32,
            ]);
        }

        let mut color = tint_table[point.bucket_index];
        color[3] = taper_alpha(point.age_s);
        colors.push(color);
    }

    // Two triangles per segment. Winding is irrelevant — the trail pipeline (like every pass)
    // does not cull. Adjacent segments share the L/R vertices at their common joint exactly, so
    // there is no gap and no overlapping (double-blended) geometry.
    for i in 0..count - 1 {
        out.push(TrailVertexRaw {
            world_xy: left[i],
            color: colors[i],
        });
        out.push(TrailVertexRaw {
            world_xy: right[i],
            color: colors[i],
        });
        out.push(TrailVertexRaw {
            world_xy: right[i + 1],
            color: colors[i + 1],
        });
        out.push(TrailVertexRaw {
            world_xy: left[i],
            color: colors[i],
        });
        out.push(TrailVertexRaw {
            world_xy: right[i + 1],
            color: colors[i + 1],
        });
        out.push(TrailVertexRaw {
            world_xy: left[i + 1],
            color: colors[i + 1],
        });
    }
}

/// The unit tangent at vertex `index` of a run of `count` points: an endpoint uses its single
/// adjacent segment's direction; an interior vertex averages the two it joins (a cheap miterless
/// join — trails are near-straight at 1 Hz, so the width pinch a real miter would correct is
/// negligible). A ~180° reversal (which the no-backward-along-track invariant in `core::sim`
/// already prevents) would sum to ~zero; the guard falls back to the outgoing segment there.
fn vertex_tangent(index: usize, count: usize, segment_dirs: &[[f64; 2]]) -> [f64; 2] {
    if index == 0 {
        segment_dirs[0]
    } else if index == count - 1 {
        segment_dirs[count - 2]
    } else {
        let sum = [
            segment_dirs[index - 1][0] + segment_dirs[index][0],
            segment_dirs[index - 1][1] + segment_dirs[index][1],
        ];
        let len = (sum[0] * sum[0] + sum[1] * sum[1]).sqrt();
        if len < 1e-9 {
            segment_dirs[index]
        } else {
            [sum[0] / len, sum[1] / len]
        }
    }
}

#[cfg(test)]
mod tests {
    use look_above_core::geo::MercatorXy;
    use look_above_core::sim::AltitudeBucket;
    use look_above_core::types::Icao24;

    use super::*;

    fn hex(s: &str) -> Icao24 {
        Icao24::from_hex(s).expect("valid ICAO24 in test")
    }

    /// A tint table whose entries are distinguishable by their red channel, so a vertex's color
    /// can be traced back to the bucket it was classified into.
    fn probe_table() -> [[f32; 4]; 6] {
        let mut table = [[0.0_f32; 4]; 6];
        for (index, row) in table.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "index is 0..=5, exact in f32")]
            let r = index as f32 / 10.0;
            *row = [r, 0.2, 0.3, 1.0];
        }
        table
    }

    /// A trail vertex on the equator `x_m` metres east, `age_s` old, in `bucket`.
    fn vertex(icao: &str, x_m: f64, age_s: f64, bucket: AltitudeBucket) -> TrailVertex {
        TrailVertex {
            icao24: hex(icao),
            position: MercatorXy::new(x_m, 0.0),
            altitude_bucket: bucket,
            age_s,
        }
    }

    // ---- Taper curves -------------------------------------------------------------------------

    #[test]
    fn width_tapers_from_head_to_tail_and_clamps() {
        assert!((taper_width_px(0.0) - TRAIL_WIDTH_HEAD_PX).abs() < 1e-12);
        assert!((taper_width_px(TRAIL_DURATION_S) - TRAIL_WIDTH_TAIL_PX).abs() < 1e-12);
        // Monotonically narrowing, and past the tail it stays pinned (does not go negative).
        assert!(taper_width_px(TRAIL_DURATION_S / 2.0) < TRAIL_WIDTH_HEAD_PX);
        assert!(taper_width_px(TRAIL_DURATION_S / 2.0) > TRAIL_WIDTH_TAIL_PX);
        assert!((taper_width_px(TRAIL_DURATION_S * 2.0) - TRAIL_WIDTH_TAIL_PX).abs() < 1e-12);
    }

    #[test]
    fn alpha_tapers_from_head_to_tail_and_clamps() {
        assert!((taper_alpha(0.0) - TRAIL_ALPHA_HEAD).abs() < 1e-6);
        assert!((taper_alpha(TRAIL_DURATION_S) - TRAIL_ALPHA_TAIL).abs() < 1e-6);
        assert!(taper_alpha(TRAIL_DURATION_S / 2.0) < TRAIL_ALPHA_HEAD);
        assert!(taper_alpha(TRAIL_DURATION_S / 2.0) > TRAIL_ALPHA_TAIL);
        // Never negative past the tail.
        assert!(taper_alpha(TRAIL_DURATION_S * 2.0) >= 0.0);
    }

    #[test]
    fn half_width_is_positive_and_scales_with_pixels_and_zoom() {
        let base = half_width_normalized(3.0, 100.0);
        assert!(base > 0.0);
        // Twice the pixels, or twice the metres-per-pixel, doubles the offset.
        assert!((half_width_normalized(6.0, 100.0) - 2.0 * base).abs() < 1e-9);
        assert!((half_width_normalized(3.0, 200.0) - 2.0 * base).abs() < 1e-9);
    }

    // ---- Ribbon tessellation ------------------------------------------------------------------

    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "test expectations narrow the same metres-to-normalized-plane value tessellate \
                  itself narrows"
    )]
    fn a_straight_run_widens_into_a_ribbon_offset_perpendicular_to_travel() {
        let mpp = 5_000.0;
        // Three samples moving due east; oldest first (the feed's order), ages decreasing to the
        // head at age 0.
        let trails = vec![
            vertex("3c6444", 0.0, 100.0, AltitudeBucket::To28000Ft),
            vertex("3c6444", 20_000.0, 50.0, AltitudeBucket::To28000Ft),
            vertex("3c6444", 40_000.0, 0.0, AltitudeBucket::To28000Ft),
        ];
        let mut out = Vec::new();
        tessellate_trails(&trails, &probe_table(), mpp, &mut out);

        // Two segments × two triangles × three vertices.
        assert_eq!(out.len(), 12);

        // Travel is +x, so every vertex is offset only in ±y, by that sample's own half-width.
        let head_half = half_width_normalized(taper_width_px(0.0), mpp);
        let tail_half = half_width_normalized(taper_width_px(100.0), mpp);
        assert!(
            head_half > tail_half,
            "the head must be wider than the tail"
        );

        let x_head = (40_000.0_f64 / WEB_MERCATOR_EXTENT_M) as f32;
        let x_tail = (0.0_f64 / WEB_MERCATOR_EXTENT_M) as f32;
        for v in &out {
            // Only the two distinct offsets (±) at each of the three sample x-positions appear.
            let expected_half = if (v.world_xy[0] - x_head).abs() < 1e-6 {
                head_half
            } else if (v.world_xy[0] - x_tail).abs() < 1e-6 {
                tail_half
            } else {
                // The middle sample (age 50): its own half-width.
                half_width_normalized(taper_width_px(50.0), mpp)
            };
            assert!(
                (v.world_xy[1].abs() - expected_half).abs() < 1e-6,
                "vertex {:?} not offset by its own half-width {expected_half}",
                v.world_xy
            );
        }

        // Both sides of the centerline are present (the ribbon has real width, not a zero-area
        // sliver): some vertices offset +y, some −y.
        assert!(out.iter().any(|v| v.world_xy[1] > 0.0));
        assert!(out.iter().any(|v| v.world_xy[1] < 0.0));
    }

    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "test expectations narrow the same metres-to-normalized-plane value tessellate \
                  itself narrows"
    )]
    fn head_vertices_are_more_opaque_and_colored_by_their_own_bucket() {
        let mpp = 5_000.0;
        let table = probe_table();
        // Head sample in a high band, tail sample on the ground — different buckets, so the
        // per-vertex coloring (not one repeated color) is observable.
        let trails = vec![
            vertex("3c6444", 0.0, 100.0, AltitudeBucket::Ground),
            vertex("3c6444", 40_000.0, 0.0, AltitudeBucket::Above40000Ft),
        ];
        let mut out = Vec::new();
        tessellate_trails(&trails, &table, mpp, &mut out);

        let ground_r = table[color::altitude_bucket_index(AltitudeBucket::Ground)][0];
        let high_r = table[color::altitude_bucket_index(AltitudeBucket::Above40000Ft)][0];

        let x_head = (40_000.0_f64 / WEB_MERCATOR_EXTENT_M) as f32;
        let head: Vec<_> = out
            .iter()
            .filter(|v| (v.world_xy[0] - x_head).abs() < 1e-6)
            .collect();
        let tail: Vec<_> = out.iter().filter(|v| v.world_xy[0].abs() < 1e-6).collect();
        assert!(!head.is_empty() && !tail.is_empty());

        for v in head {
            assert!(
                (v.color[0] - high_r).abs() < 1e-6,
                "head not its own bucket color"
            );
            assert!((v.color[3] - taper_alpha(0.0)).abs() < 1e-6);
        }
        for v in tail {
            assert!(
                (v.color[0] - ground_r).abs() < 1e-6,
                "tail not its own bucket color"
            );
            assert!((v.color[3] - taper_alpha(100.0)).abs() < 1e-6);
            assert!(
                v.color[3] < taper_alpha(0.0),
                "tail must be more transparent than head"
            );
        }
    }

    #[test]
    fn a_single_sample_run_produces_no_geometry() {
        let trails = vec![vertex("3c6444", 0.0, 0.0, AltitudeBucket::To10000Ft)];
        let mut out = Vec::new();
        tessellate_trails(&trails, &probe_table(), 5_000.0, &mut out);
        assert!(out.is_empty(), "a lone point is a dot, not a ribbon");
    }

    #[test]
    fn a_stationary_run_of_coincident_samples_produces_no_geometry() {
        // On-ground/holding: repeated identical displayed positions, distinct ages.
        let trails = vec![
            vertex("3c6444", 1_000.0, 20.0, AltitudeBucket::Ground),
            vertex("3c6444", 1_000.0, 10.0, AltitudeBucket::Ground),
            vertex("3c6444", 1_000.0, 0.0, AltitudeBucket::Ground),
        ];
        let mut out = Vec::new();
        tessellate_trails(&trails, &probe_table(), 5_000.0, &mut out);
        assert!(
            out.is_empty(),
            "coincident samples collapse to one distinct point, so no ribbon"
        );
    }

    #[test]
    fn each_aircraft_run_is_tessellated_independently() {
        let mpp = 5_000.0;
        // Two aircraft, each a straight two-sample run: two segments total → 12 vertices.
        let trails = vec![
            vertex("3c6444", 0.0, 10.0, AltitudeBucket::To10000Ft),
            vertex("3c6444", 20_000.0, 0.0, AltitudeBucket::To10000Ft),
            vertex("4b1815", 0.0, 10.0, AltitudeBucket::To28000Ft),
            vertex("4b1815", 20_000.0, 0.0, AltitudeBucket::To28000Ft),
        ];
        let mut both = Vec::new();
        tessellate_trails(&trails, &probe_table(), mpp, &mut both);
        assert_eq!(both.len(), 12);

        // The count is exactly the sum of the two runs tessellated alone — the run boundary is
        // respected (no phantom segment stitching one aircraft's tail to the next one's head).
        let mut first = Vec::new();
        tessellate_trails(&trails[..2], &probe_table(), mpp, &mut first);
        let mut second = Vec::new();
        tessellate_trails(&trails[2..], &probe_table(), mpp, &mut second);
        assert_eq!(both.len(), first.len() + second.len());
    }

    #[test]
    fn tessellate_reuses_the_output_buffer() {
        let mut out = Vec::new();
        let trails = vec![
            vertex("3c6444", 0.0, 10.0, AltitudeBucket::To10000Ft),
            vertex("3c6444", 20_000.0, 0.0, AltitudeBucket::To10000Ft),
        ];
        tessellate_trails(&trails, &probe_table(), 5_000.0, &mut out);
        assert_eq!(out.len(), 6);
        // A second call clears rather than appends — no per-frame growth (ADR-002).
        tessellate_trails(&[], &probe_table(), 5_000.0, &mut out);
        assert!(out.is_empty());
    }
}
