//! Procedurally generated SDF glyph atlas for the six aircraft categories (M2 item 2.5).
//!
//! docs/01 asks for an "SDF glyph atlas"; there is no image/font/asset-loading crate in this
//! workspace and `render` must stay self-contained (no bundled artwork, no network — ADR-002).
//! So the atlas is generated here, at startup, from small hand-authored 2D silhouettes (plain
//! `(f32, f32)` point lists in a local glyph space spanning roughly `[-0.5, 0.5]` on both axes,
//! nose/front at `+y` — the same "north is up" convention `aircraft.rs`'s rotation uses) rather
//! than rasterized from any external asset. Each category's silhouette is rasterized into a
//! single-channel signed-distance field (64×64, one tile per category, packed into one 384×64
//! `R8Unorm` strip) using the standard convention: `0.5` encodes the silhouette edge, `1.0` deep
//! inside, `0.0` far outside — `aircraft.wgsl`'s fragment shader `smoothstep`s around `0.5` for
//! the "SDF-derived AA" docs/01's quality bar asks for.
//!
//! These silhouettes are deliberately simple (a handful of line segments, or a circle) and not
//! meant to be literal aircraft artwork — "distinguishable at a glance" is the v1 bar (see the
//! M2 2.5 decision-log entry: no asset pipeline exists yet, so this is a judgement call, not a
//! placeholder to be embarrassed about).

use look_above_core::contracts::AircraftCategory;

/// One glyph tile's side, in texels. 64 is generous for a 16–24 px on-screen glyph (docs/01's
/// L2 tier) while keeping the whole atlas small (one `384x64` `R8Unorm` texture is 24 KiB).
pub const TILE_PX: u32 = 64;

/// How many category tiles the atlas holds — docs/01's six categories, one tile each, packed
/// left to right in [`category_index`]'s order.
pub const CATEGORY_COUNT: u32 = 6;

pub const ATLAS_WIDTH_PX: u32 = TILE_PX * CATEGORY_COUNT;
pub const ATLAS_HEIGHT_PX: u32 = TILE_PX;

/// Half the distance (in local glyph-space units, where a tile spans `[-0.5, 0.5]`) over which
/// the encoded distance ramps from 0 to 1 around a silhouette's edge. Small enough that every
/// silhouette below (whose points stay within `±0.4`) reaches full "outside" (`0.0`) well before
/// the tile edge, so adjacent tiles never bleed into each other under bilinear filtering.
const SPREAD: f32 = 0.06;

/// Maps a category to its tile index (and therefore its atlas UV offset) — matches the order
/// [`build_atlas_bytes`] rasterizes the atlas strip in.
pub fn category_index(category: AircraftCategory) -> u32 {
    match category {
        AircraftCategory::Jet => 0,
        AircraftCategory::Turboprop => 1,
        AircraftCategory::Piston => 2,
        AircraftCategory::Heli => 3,
        AircraftCategory::Glider => 4,
        AircraftCategory::Unknown => 5,
    }
}

/// The six categories in [`category_index`]'s tile order.
const CATEGORY_ORDER: [AircraftCategory; CATEGORY_COUNT as usize] = [
    AircraftCategory::Jet,
    AircraftCategory::Turboprop,
    AircraftCategory::Piston,
    AircraftCategory::Heli,
    AircraftCategory::Glider,
    AircraftCategory::Unknown,
];

/// `index`'s tile as a `(min_u, min_v, max_u, max_v)` rect in the full atlas's `[0, 1]` UV
/// space — every tile is the same height (one full `[0, 1]` `v` span) and `1 / CATEGORY_COUNT`
/// wide.
///
/// `aircraft.wgsl` computes this same rect itself from `category_index` and its own
/// `CATEGORY_COUNT` constant rather than calling into this function (WGSL can't call Rust); it
/// exists so that formula has a plain-Rust twin the UV-rect unit tests below can pin.
#[allow(
    dead_code,
    reason = "kept as a unit-testable mirror of aircraft.wgsl's UV-offset math, which cargo test \
              cannot exercise directly"
)]
#[allow(
    clippy::cast_precision_loss,
    reason = "index is one of 6 small integers (0..=5) and CATEGORY_COUNT is the constant 6, both far inside f32's exact-integer range"
)]
pub fn category_uv_rect(index: u32) -> (f32, f32, f32, f32) {
    let tile_width = 1.0 / CATEGORY_COUNT as f32;
    let min_u = index as f32 * tile_width;
    (min_u, 0.0, min_u + tile_width, 1.0)
}

/// One category's silhouette as one or more simple (non-self-intersecting) closed polygons,
/// unioned together (see [`signed_distance_to_shapes`]) — every category is one polygon except
/// the helicopter, whose rotor disc and tail boom are two separate shapes rather than one
/// hand-threaded outline.
///
/// Evocative, not literal (docs/01/skill: these "do not need to be literal or pretty, just
/// distinguishable"): jet is swept/delta, turboprop and piston/light are straight-winged
/// (piston's wing set further forward and narrower — a "high wing" read), glider is the widest
/// span with the thinnest fuselage, helicopter is a rotor disc plus a tail-boom stub, unknown is
/// a plain dart.
fn category_shapes(category: AircraftCategory) -> Vec<Vec<(f32, f32)>> {
    match category {
        AircraftCategory::Jet => vec![vec![
            (0.0, 0.40),
            (0.38, -0.05),
            (0.10, -0.15),
            (0.14, -0.40),
            (0.0, -0.28),
            (-0.14, -0.40),
            (-0.10, -0.15),
            (-0.38, -0.05),
        ]],
        AircraftCategory::Turboprop => vec![vec![
            (0.0, 0.38),
            (0.05, 0.05),
            (0.40, 0.02),
            (0.40, -0.06),
            (0.06, -0.10),
            (0.10, -0.38),
            (-0.10, -0.38),
            (-0.06, -0.10),
            (-0.40, -0.06),
            (-0.40, 0.02),
            (-0.05, 0.05),
        ]],
        AircraftCategory::Piston => vec![vec![
            (0.0, 0.38),
            (0.05, 0.20),
            (0.32, 0.16),
            (0.32, 0.10),
            (0.05, 0.12),
            (0.06, -0.10),
            (0.09, -0.38),
            (-0.09, -0.38),
            (-0.06, -0.10),
            (-0.05, 0.12),
            (-0.32, 0.10),
            (-0.32, 0.16),
            (-0.05, 0.20),
        ]],
        AircraftCategory::Heli => vec![
            circle_points((0.0, 0.05), 0.34, 32),
            vec![(0.06, -0.02), (0.06, -0.40), (-0.06, -0.40), (-0.06, -0.02)],
        ],
        AircraftCategory::Glider => vec![vec![
            (0.0, 0.38),
            (0.035, 0.10),
            (0.40, 0.06),
            (0.40, 0.0),
            (0.035, -0.05),
            (0.05, -0.38),
            (-0.05, -0.38),
            (-0.035, -0.05),
            (-0.40, 0.0),
            (-0.40, 0.06),
            (-0.035, 0.10),
        ]],
        AircraftCategory::Unknown => vec![vec![
            (0.0, 0.38),
            (0.30, -0.35),
            (0.0, -0.12),
            (-0.30, -0.35),
        ]],
    }
}

/// Evenly spaced points around a circle, for the helicopter's rotor disc.
fn circle_points(center: (f32, f32), radius: f32, count: usize) -> Vec<(f32, f32)> {
    (0..count)
        .map(|i| {
            #[allow(
                clippy::cast_precision_loss,
                reason = "count is a small fixed constant (32), far inside f32's exact-integer range"
            )]
            let t = i as f32 / count as f32;
            let angle = t * std::f32::consts::TAU;
            (
                center.0 + radius * angle.cos(),
                center.1 + radius * angle.sin(),
            )
        })
        .collect()
}

/// Whether `point` is inside the closed polygon `poly`, by the standard even-odd ray-casting
/// rule. `poly`'s winding direction does not matter for this test.
fn point_in_polygon(point: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if ((yi > point.1) != (yj > point.1))
            && (point.0 < (xj - xi) * (point.1 - yi) / (yj - yi) + xi)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The shortest distance from `point` to any edge of the closed polygon `poly`.
fn distance_to_polygon_edges(point: (f32, f32), poly: &[(f32, f32)]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| distance_to_segment(point, poly[i], poly[(i + 1) % n]))
        .fold(f32::MAX, f32::min)
}

/// The shortest distance from `point` to the segment `a`–`b`.
///
/// `pub(crate)`, not private: `label_atlas.rs` (M2 item 2.7b) reuses this exact primitive for its
/// stroke-font rasterization (distance to the nearest stroke segment) rather than duplicating it —
/// the same point-to-segment math, just fed a font's line strokes instead of a silhouette's edges.
pub(crate) fn distance_to_segment(point: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (ax, ay) = a;
    let (bx, by) = b;
    let (px, py) = point;
    let (abx, aby) = (bx - ax, by - ay);
    let (apx, apy) = (px - ax, py - ay);
    let ab_len_sq = abx * abx + aby * aby;
    let t = if ab_len_sq > 0.0 {
        ((apx * abx + apy * aby) / ab_len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (ax + abx * t, ay + aby * t);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Signed distance to one polygon: positive inside, negative outside.
fn signed_distance_to_polygon(point: (f32, f32), poly: &[(f32, f32)]) -> f32 {
    let distance = distance_to_polygon_edges(point, poly);
    if point_in_polygon(point, poly) {
        distance
    } else {
        -distance
    }
}

/// Signed distance to the union of `shapes` — inside the union iff inside any one shape, which
/// in this "positive inside" convention is the `max` of the individual signed distances (the
/// mirror of the usual "negative inside" SDF union, which is a `min`).
fn signed_distance_to_shapes(point: (f32, f32), shapes: &[Vec<(f32, f32)>]) -> f32 {
    shapes
        .iter()
        .map(|shape| signed_distance_to_polygon(point, shape))
        .fold(f32::MIN, f32::max)
}

/// Encodes a signed distance (positive inside, negative outside) as the atlas's `R8Unorm`
/// convention: `0.5` at the edge, ramping linearly to `1.0`/`0.0` over ±`spread`.
///
/// `pub(crate)` and parameterized on `spread` (rather than capturing [`SPREAD`] directly): shared
/// with `label_atlas.rs`, whose stroke-font tiles are smaller and want their own antialiasing
/// band width — see that module's own spread constant. [`encode_distance`] is this module's own
/// thin wrapper fixing `spread` to [`SPREAD`], kept so every existing call site/test here reads
/// exactly as before.
pub(crate) fn encode_signed_distance(signed_distance: f32, spread: f32) -> u8 {
    let normalized = (0.5 + signed_distance / (2.0 * spread)).clamp(0.0, 1.0);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "normalized is clamped to [0, 1] just above, so it is never negative and \
                  * 255 always lands in u8's range"
    )]
    {
        (normalized * 255.0).round() as u8
    }
}

/// This module's own [`encode_signed_distance`] call, fixed to its own [`SPREAD`].
fn encode_distance(signed_distance: f32) -> u8 {
    encode_signed_distance(signed_distance, SPREAD)
}

/// Maps one tile-local texel (row 0 = top = local `y = +0.5`, matching `aircraft.wgsl`'s
/// `uv.y = 0` corner) to the local glyph-space point [`category_shapes`] is authored in.
fn texel_to_local(row: u32, col: u32) -> (f32, f32) {
    #[allow(
        clippy::cast_precision_loss,
        reason = "row/col are < TILE_PX (64), far inside f32's exact-integer range"
    )]
    {
        let u = (col as f32 + 0.5) / TILE_PX as f32;
        let v = (row as f32 + 0.5) / TILE_PX as f32;
        (u - 0.5, 0.5 - v)
    }
}

/// Rasterizes all six categories' silhouettes into one `384×64` `R8Unorm` strip, row-major, top
/// row first — the byte layout `renderer.rs` uploads directly as the atlas texture's initial
/// contents. Pure and deterministic; runs once, in [`crate::Renderer::new`].
pub fn build_atlas_bytes() -> Vec<u8> {
    let mut bytes = vec![0_u8; (ATLAS_WIDTH_PX * ATLAS_HEIGHT_PX) as usize];
    for (index, category) in CATEGORY_ORDER.iter().enumerate() {
        let shapes = category_shapes(*category);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "index is < CATEGORY_COUNT (6), fits comfortably in u32"
        )]
        let index_u32 = index as u32;
        for row in 0..TILE_PX {
            for col in 0..TILE_PX {
                let local = texel_to_local(row, col);
                let value = encode_distance(signed_distance_to_shapes(local, &shapes));
                let x = index_u32 * TILE_PX + col;
                let offset = (row * ATLAS_WIDTH_PX + x) as usize;
                bytes[offset] = value;
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CATEGORIES: [AircraftCategory; CATEGORY_COUNT as usize] = CATEGORY_ORDER;

    #[test]
    fn category_index_is_distinct_for_all_six_categories_and_in_range() {
        let indices: Vec<u32> = ALL_CATEGORIES.iter().map(|c| category_index(*c)).collect();
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                assert_ne!(
                    indices[i], indices[j],
                    "{:?} and {:?} share a tile",
                    ALL_CATEGORIES[i], ALL_CATEGORIES[j]
                );
            }
        }
        assert!(indices.iter().all(|&i| i < CATEGORY_COUNT));
    }

    #[test]
    fn every_categorys_uv_rect_is_distinct_and_inside_the_unit_square() {
        let mut rects = Vec::new();
        for index in 0..CATEGORY_COUNT {
            let (min_u, min_v, max_u, max_v) = category_uv_rect(index);
            assert!((0.0..=1.0).contains(&min_u));
            assert!((0.0..=1.0).contains(&max_u));
            assert!((0.0..=1.0).contains(&min_v));
            assert!((0.0..=1.0).contains(&max_v));
            assert!(min_u < max_u);
            assert!(min_v < max_v);
            rects.push((min_u, max_u));
        }
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                let (a_min, a_max) = rects[i];
                let (b_min, b_max) = rects[j];
                assert!(
                    a_max <= b_min || b_max <= a_min,
                    "tiles {i} and {j} overlap"
                );
            }
        }
    }

    #[test]
    fn signed_distance_is_zero_at_a_squares_edge_and_moves_the_right_way_on_each_side() {
        // A plain unit square (not one of the real silhouettes) pins the sign convention and the
        // edge crossing without depending on any category's exact shape.
        let square = vec![(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];

        assert!((signed_distance_to_polygon((0.5, 0.0), &square)).abs() < 1e-6);
        assert!(
            signed_distance_to_polygon((0.0, 0.0), &square) > 0.0,
            "center must be inside (positive)"
        );
        assert!(
            signed_distance_to_polygon((0.9, 0.0), &square) < 0.0,
            "outside the square must be negative"
        );

        let far_outside = signed_distance_to_polygon((0.9, 0.0), &square);
        let near_outside = signed_distance_to_polygon((0.6, 0.0), &square);
        assert!(
            far_outside < near_outside,
            "distance must grow more negative further outside"
        );

        let center = signed_distance_to_polygon((0.0, 0.0), &square);
        let near_inside = signed_distance_to_polygon((0.4, 0.0), &square);
        assert!(
            center > near_inside,
            "distance must shrink approaching the edge from inside"
        );
    }

    #[test]
    fn encode_distance_is_half_at_the_edge_and_saturates_away_from_it() {
        assert!((f32::from(encode_distance(0.0)) - 127.5).abs() < 2.0);
        assert_eq!(encode_distance(SPREAD * 10.0), 255);
        assert_eq!(encode_distance(-SPREAD * 10.0), 0);
        assert!(encode_distance(-SPREAD) < encode_distance(0.0));
        assert!(encode_distance(0.0) < encode_distance(SPREAD));
    }

    #[test]
    fn atlas_bytes_are_the_expected_size_and_every_tile_has_a_recognizable_edge() {
        let bytes = build_atlas_bytes();
        assert_eq!(bytes.len(), (ATLAS_WIDTH_PX * ATLAS_HEIGHT_PX) as usize);

        // Every tile's center reads solidly "inside" (> 0.5) and a point near its corner reads
        // solidly "outside" (< 0.5) — a sanity check that each category actually rasterized a
        // real silhouette, not an empty or inverted tile.
        for index in 0..CATEGORY_COUNT {
            let center_col = index * TILE_PX + TILE_PX / 2;
            let center_row = TILE_PX / 2;
            let center_value = bytes[(center_row * ATLAS_WIDTH_PX + center_col) as usize];
            assert!(
                center_value > 127,
                "category {index}'s tile center is not inside its silhouette"
            );

            let corner_col = index * TILE_PX + 1;
            let corner_row = 1;
            let corner_value = bytes[(corner_row * ATLAS_WIDTH_PX + corner_col) as usize];
            assert!(
                corner_value < 127,
                "category {index}'s tile corner is not outside its silhouette"
            );
        }
    }
}
