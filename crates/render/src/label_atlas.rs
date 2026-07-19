//! Procedurally generated SDF stroke-font atlas for label text (M2 item 2.7b).
//!
//! Same technique as [`crate::glyph_atlas`] (no font/asset-loading crate exists in this
//! workspace, and `render` must stay self-contained/network-free — ADR-002), applied to a
//! **stroke font** instead of filled silhouettes: each character is a small set of hand-authored
//! line segments (not a closed polygon), rasterized by taking the distance from every atlas
//! texel to the *nearest stroke segment* (reusing [`glyph_atlas::distance_to_segment`]) rather
//! than a signed inside/outside polygon test — there is no "inside" for an open stroke, only "how
//! close is this texel to a line". A texel within [`STROKE_HALF_WIDTH`] of some stroke is "on"
//! the glyph; [`glyph_atlas::encode_signed_distance`] (this module's own [`SPREAD`]) gives the
//! same `0.5`-at-the-edge `R8Unorm` convention `label.wgsl`'s fragment shader `smoothstep`s
//! around, mirroring `aircraft.wgsl`'s SDF-derived antialiasing.
//!
//! Character set: exactly what the label content format (`CALLSIGN  FLnnn  nnnkt`, M2 item 2.7a)
//! needs — `A`–`Z` and `0`–`9` (callsigns, `FL`, altitude/speed digits), a space (the format's own
//! two-space separators collapse to nothing since a space glyph is intentionally blank, but the
//! character still needs a valid atlas slot so a label string can be packed without special-
//! casing it), and the two lowercase letters `k`/`t` (the literal unit suffix). No other ASCII —
//! this is a technical/UI font, not a general-purpose one.
//!
//! Letterforms are a compact "stick font": each character is 2–6 straight strokes over a 3×5
//! point grid spanning local glyph-space `[0, 1]` on both axes (`x = 0/0.5/1` left/center/right,
//! `y = 0/0.25/0.5/0.75/1` bottom/down/mid/up/top — `y = 0` is the baseline, `y = 1` the cap
//! height). Digits reuse the familiar seven-segment layout (top/mid/bottom rows only); letters use
//! the full grid, including the quarter-height rows, for the handful of shapes (`O`, `S`, `G`, the
//! lowercase pair) that need them. Evocative, not typographic — the same "distinguishable at a
//! glance, not literal" bar `glyph_atlas`'s own aircraft silhouettes are held to.

use crate::glyph_atlas;

/// One glyph tile's side, in texels. Smaller than [`glyph_atlas::TILE_PX`] (64): these are simple
/// line-stroke shapes at a small on-screen label-text size, not aircraft silhouettes, so less
/// raster resolution is needed to stay legible after the SDF's own antialiasing.
pub const TILE_PX: u32 = 32;

/// The full character set, in atlas tile order — [`char_index`] is this array's position lookup,
/// so this order is the single source of truth both [`build_atlas_bytes`] and every atlas
/// consumer (`label.rs`'s GPU packing) agree on.
const CHARSET: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ' ', 'k',
    't',
];

/// How many character tiles the atlas holds.
#[allow(
    clippy::cast_possible_truncation,
    reason = "CHARSET is a small fixed literal (39 entries), far inside u32's range"
)]
pub const CHAR_COUNT: u32 = CHARSET.len() as u32;

pub const ATLAS_WIDTH_PX: u32 = TILE_PX * CHAR_COUNT;
pub const ATLAS_HEIGHT_PX: u32 = TILE_PX;

/// Half the on-screen thickness of a stroke, in local glyph-space units (the grid spans
/// `[0, 1]`) — thick enough to stay legible after the SDF antialiasing band below, thin enough
/// that parallel strokes in the same tile (e.g. `E`'s three horizontal bars) stay visually
/// distinct rather than merging into a solid block.
const STROKE_HALF_WIDTH: f32 = 0.09;

/// Half the distance over which the encoded distance ramps from 0 to 1 around a stroke's edge —
/// this module's own (smaller-tile) analog of [`glyph_atlas`]'s `SPREAD`.
const SPREAD: f32 = 0.05;

/// A single straight stroke, as its two endpoints in local glyph-space `[0, 1]²`.
type Segment = ((f32, f32), (f32, f32));

// ---- The 3×5 letterform grid ---------------------------------------------------------------
//
// Row prefixes (bottom to top): B(ottom, y=0), D(own, y=0.25), M(id, y=0.5), U(p, y=0.75),
// T(op, y=1). Column suffixes: L(eft, x=0), C(enter, x=0.5), R(ight, x=1). Digits only ever use
// the T/M/B rows (the familiar seven-segment layout); letters use the full grid where a shape
// needs the finer quarter-height rows (`O`, `S`, `G`, and the lowercase `k`/`t`).

const TL: (f32, f32) = (0.0, 1.0);
const TC: (f32, f32) = (0.5, 1.0);
const TR: (f32, f32) = (1.0, 1.0);
const UL: (f32, f32) = (0.0, 0.75);
const UC: (f32, f32) = (0.5, 0.75);
const UR: (f32, f32) = (1.0, 0.75);
const ML: (f32, f32) = (0.0, 0.5);
const MC: (f32, f32) = (0.5, 0.5);
const MR: (f32, f32) = (1.0, 0.5);
const DL: (f32, f32) = (0.0, 0.25);
const DR: (f32, f32) = (1.0, 0.25);
const BL: (f32, f32) = (0.0, 0.0);
const BC: (f32, f32) = (0.5, 0.0);
const BR: (f32, f32) = (1.0, 0.0);

/// `O`'s hexagonal ring — a "cut-corner rectangle" that reads as rounder than the plain
/// rectangles used elsewhere (`0`, `D`), so `O`/`0`/`D` all stay visually distinct from one
/// another. Shared with `Q` (an `O` plus a tail stroke) below.
fn o_strokes() -> Vec<Segment> {
    vec![(TC, UR), (UR, DR), (DR, BC), (BC, DL), (DL, UL), (UL, TC)]
}

/// One digit's strokes, standard seven-segment layout (well-known, not a judgement call): `top`,
/// `top_left`, `top_right`, `middle`, `bottom_left`, `bottom_right`, `bottom`.
fn digit_strokes(d: u8) -> Vec<Segment> {
    let top = (TL, TR);
    let top_left = (TL, ML);
    let top_right = (TR, MR);
    let middle = (ML, MR);
    let bottom_left = (ML, BL);
    let bottom_right = (MR, BR);
    let bottom = (BL, BR);
    match d {
        0 => vec![top, top_left, top_right, bottom_left, bottom_right, bottom],
        1 => vec![top_right, bottom_right],
        2 => vec![top, top_right, middle, bottom_left, bottom],
        3 => vec![top, top_right, middle, bottom_right, bottom],
        4 => vec![top_left, top_right, middle, bottom_right],
        5 => vec![top, top_left, middle, bottom_right, bottom],
        6 => vec![top, top_left, middle, bottom_left, bottom_right, bottom],
        7 => vec![top, top_right, bottom_right],
        8 => vec![
            top,
            top_left,
            top_right,
            middle,
            bottom_left,
            bottom_right,
            bottom,
        ],
        9 => vec![top, top_left, top_right, middle, bottom_right, bottom],
        _ => Vec::new(),
    }
}

/// One letter's (or the space/`k`/`t`) strokes. A simple "stick font" — see this module's doc
/// comment for the grid and the "distinguishable, not typographic" bar.
fn letter_strokes(ch: char) -> Vec<Segment> {
    match ch {
        'A' => vec![(BL, TC), (TC, BR), (ML, MR)],
        'B' => vec![(BL, TL), (TL, TR), (TR, MR), (ML, MR), (MR, BR), (BL, BR)],
        'C' => vec![(TR, TL), (TL, BL), (BL, BR)],
        'D' => vec![(BL, TL), (TL, TC), (TC, UR), (UR, DR), (DR, BC), (BC, BL)],
        'E' => vec![(TR, TL), (TL, BL), (BL, BR), (ML, MR)],
        'F' => vec![(TR, TL), (TL, BL), (ML, MR)],
        'G' => vec![(TR, TL), (TL, BL), (BL, BR), (BR, MR), (MR, MC)],
        'H' => vec![(BL, TL), (BR, TR), (ML, MR)],
        'I' => vec![(TL, TR), (TC, BC), (BL, BR)],
        'J' => vec![(TL, TR), (TR, BR), (BR, BL)],
        'K' => vec![(BL, TL), (TR, ML), (ML, BR)],
        'L' => vec![(TL, BL), (BL, BR)],
        'M' => vec![(BL, TL), (TL, MC), (MC, TR), (TR, BR)],
        'N' => vec![(BL, TL), (TL, BR), (BR, TR)],
        'O' => o_strokes(),
        'P' => vec![(BL, TL), (TL, TR), (TR, MR), (MR, ML)],
        'Q' => {
            let mut strokes = o_strokes();
            strokes.push((MC, BR));
            strokes
        }
        'R' => vec![(BL, TL), (TL, TR), (TR, MR), (MR, ML), (MC, BR)],
        'S' => vec![(TR, TL), (TL, ML), (ML, MR), (MR, BR), (BR, BL)],
        'T' => vec![(TL, TR), (TC, BC)],
        'U' => vec![(TL, BL), (BL, BR), (BR, TR)],
        'V' => vec![(TL, BC), (BC, TR)],
        'W' => vec![(TL, BL), (BL, MC), (MC, BR), (BR, TR)],
        'X' => vec![(TL, BR), (TR, BL)],
        'Y' => vec![(TL, MC), (TR, MC), (MC, BC)],
        'Z' => vec![(TL, TR), (TR, BL), (BL, BR)],
        // Lowercase `k`/`t` (the label's literal unit suffix): shorter ascenders than their
        // uppercase counterparts (`k`'s stem stops at `UL`, not `TL`; `t`'s crossbar sits at the
        // `D` row, not `M`) — a deliberately small nod to lowercase proportions without a whole
        // second x-height class for a two-character subset.
        'k' => vec![(BL, UL), (UR, MC), (MC, BR)],
        't' => vec![(UC, BC), (DL, DR)],
        _ => Vec::new(),
    }
}

/// `ch`'s strokes, or an empty (blank) tile for a space or any character outside [`CHARSET`].
fn char_strokes(ch: char) -> Vec<Segment> {
    if let Some(digit) = ch.to_digit(10) {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "char::to_digit(10) always yields 0..=9"
        )]
        return digit_strokes(digit as u8);
    }
    letter_strokes(ch)
}

/// `ch`'s tile index in the atlas, or `None` if `ch` is outside [`CHARSET`] — a label whose text
/// somehow contains an unsupported character (a malformed feed's callsign, say) skips that one
/// character rather than panicking (see `label.rs::pack_text_instances`).
pub fn char_index(ch: char) -> Option<u32> {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "CHARSET has 39 entries, far inside u32's range"
    )]
    CHARSET.iter().position(|&c| c == ch).map(|i| i as u32)
}

/// `index`'s tile as a `(min_u, min_v, max_u, max_v)` rect in the full atlas's `[0, 1]` UV
/// space — `label.wgsl` computes this same rect itself from `char_index` and its own
/// `CHAR_COUNT` constant (WGSL can't call Rust); this is the unit-testable Rust twin, mirroring
/// [`glyph_atlas::category_uv_rect`]'s own role.
#[allow(
    dead_code,
    reason = "kept as a unit-testable mirror of label.wgsl's UV-offset math, which cargo test \
              cannot exercise directly"
)]
#[allow(
    clippy::cast_precision_loss,
    reason = "index is one of CHAR_COUNT (39) small integers, far inside f32's exact-integer range"
)]
pub fn char_uv_rect(index: u32) -> (f32, f32, f32, f32) {
    let tile_width = 1.0 / CHAR_COUNT as f32;
    let min_u = index as f32 * tile_width;
    (min_u, 0.0, min_u + tile_width, 1.0)
}

/// The unsigned distance from `point` to the nearest of `strokes` — `f32::INFINITY` for an empty
/// stroke list (the space glyph), which [`build_atlas_bytes`] below turns into "outside
/// everywhere" (a blank tile) via the ordinary signed-distance encoding.
fn distance_to_nearest_stroke(point: (f32, f32), strokes: &[Segment]) -> f32 {
    strokes
        .iter()
        .map(|&(a, b)| glyph_atlas::distance_to_segment(point, a, b))
        .fold(f32::INFINITY, f32::min)
}

/// Maps one tile-local texel (row 0 = top = local `y = 1`) to the local glyph-space point
/// [`char_strokes`] is authored in — the character-grid analog of
/// `glyph_atlas::texel_to_local`, just over `[0, 1]²` instead of `[-0.5, 0.5]²` (a font's glyph
/// space is naturally baseline-relative, not centered).
fn texel_to_local(row: u32, col: u32) -> (f32, f32) {
    #[allow(
        clippy::cast_precision_loss,
        reason = "row/col are < TILE_PX (32), far inside f32's exact-integer range"
    )]
    {
        let u = (col as f32 + 0.5) / TILE_PX as f32;
        let v = (row as f32 + 0.5) / TILE_PX as f32;
        (u, 1.0 - v)
    }
}

/// Rasterizes every character's strokes into one `(TILE_PX * CHAR_COUNT) × TILE_PX` `R8Unorm`
/// strip, row-major, top row first — the byte layout `renderer.rs` uploads directly as the atlas
/// texture's initial contents. Pure and deterministic; runs once, in [`crate::Renderer::new`].
pub fn build_atlas_bytes() -> Vec<u8> {
    let mut bytes = vec![0_u8; (ATLAS_WIDTH_PX * ATLAS_HEIGHT_PX) as usize];
    for (index, &ch) in CHARSET.iter().enumerate() {
        let strokes = char_strokes(ch);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "index is < CHAR_COUNT (39), fits comfortably in u32"
        )]
        let index_u32 = index as u32;
        for row in 0..TILE_PX {
            for col in 0..TILE_PX {
                let local = texel_to_local(row, col);
                let signed_distance =
                    STROKE_HALF_WIDTH - distance_to_nearest_stroke(local, &strokes);
                let value = glyph_atlas::encode_signed_distance(signed_distance, SPREAD);
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

    #[test]
    fn char_index_covers_the_whole_charset_distinctly_and_in_range() {
        let mut seen = Vec::new();
        for &ch in CHARSET {
            let index = char_index(ch).unwrap_or_else(|| panic!("{ch:?} missing from CHARSET"));
            assert!(index < CHAR_COUNT);
            seen.push(index);
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            CHARSET.len(),
            "CHARSET contains a duplicate character"
        );
    }

    #[test]
    fn char_index_is_none_for_an_unsupported_character() {
        assert_eq!(char_index('!'), None);
        assert_eq!(
            char_index('a'),
            None,
            "lowercase outside k/t is not supported"
        );
        assert_eq!(char_index('Ω'), None);
    }

    #[test]
    fn every_characters_uv_rect_is_distinct_and_inside_the_unit_square() {
        let mut rects = Vec::new();
        for index in 0..CHAR_COUNT {
            let (min_u, min_v, max_u, max_v) = char_uv_rect(index);
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
    fn atlas_bytes_are_the_expected_size() {
        let bytes = build_atlas_bytes();
        assert_eq!(bytes.len(), (ATLAS_WIDTH_PX * ATLAS_HEIGHT_PX) as usize);
    }

    /// The space glyph must rasterize to an entirely blank tile — no stroke anywhere in it should
    /// read as "on" (`> 127`), since a run of spaces in a label must draw nothing.
    #[test]
    fn the_space_glyph_is_entirely_blank() {
        let bytes = build_atlas_bytes();
        let index = char_index(' ').expect("space is in CHARSET");
        for row in 0..TILE_PX {
            for col in 0..TILE_PX {
                let x = index * TILE_PX + col;
                let value = bytes[(row * ATLAS_WIDTH_PX + x) as usize];
                assert!(
                    value <= 127,
                    "space tile has an \"on\" texel at ({row}, {col})"
                );
            }
        }
    }

    /// Every non-space character's tile has at least one texel that reads solidly "on"
    /// (`> 127`) somewhere — a sanity check that each glyph actually rasterized real strokes,
    /// not an accidentally-empty tile.
    #[test]
    fn every_non_space_character_rasterizes_at_least_one_on_texel() {
        let bytes = build_atlas_bytes();
        for &ch in CHARSET {
            if ch == ' ' {
                continue;
            }
            let index = char_index(ch).expect("every CHARSET entry has an index");
            let has_on_texel = (0..TILE_PX).any(|row| {
                (0..TILE_PX).any(|col| {
                    let x = index * TILE_PX + col;
                    bytes[(row * ATLAS_WIDTH_PX + x) as usize] > 127
                })
            });
            assert!(has_on_texel, "{ch:?}'s tile has no \"on\" texel at all");
        }
    }

    #[test]
    fn digit_strokes_are_defined_for_every_digit_and_empty_otherwise() {
        for d in 0..=9 {
            assert!(!digit_strokes(d).is_empty(), "digit {d} has no strokes");
        }
        assert!(digit_strokes(10).is_empty());
    }
}
