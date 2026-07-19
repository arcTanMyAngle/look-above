//! The F3 debug frame-stats HUD (M2 item 2.1b) — a small, fixed top-left overlay block (fps,
//! p50/p95/worst frame time, aircraft instance count), the last piece of docs/01's draw order
//! ("map base → map lines → trails → aircraft glyphs → labels → **UI overlay**").
//!
//! Deliberately reuses 2.7b's stroke-font SDF text pipeline/atlas (`label.rs`/`label_atlas.rs`)
//! rather than a second text renderer: `renderer.rs` clones the existing pipeline/atlas
//! bind-group/quad-mesh handles (all cheap, `Arc`-backed `wgpu` types) into its own small
//! `StatsOverlayLayer`, so there is exactly one SDF text atlas texture and one text pipeline in
//! the whole crate.
//!
//! This module itself is pure, `wgpu`-free Rust (the same "layer owns GPU state, a plain module
//! owns the content/packing logic" split `label.rs` established) — its only `wgpu`-adjacent type
//! is [`crate::label::TextInstanceRaw`], a `#[repr(C)]` plain-old-data struct with no device
//! handle in it.
//!
//! **Character-set constraint.** [`label_atlas::CHARSET`](crate::label_atlas) is deliberately
//! *not* grown for this task (39 characters: `A`-`Z`, `0`-`9`, space, `k`, `t` — see that
//! module's doc comment). [`format_lines`] therefore stays entirely within that set: ALL CAPS,
//! whole numbers only, no `.`/`=`/`_`/lowercase (`k`/`t` exist in the atlas but are unused here —
//! this HUD spells `MS`, not `ms`). [`format_lines`]'s own unit test asserts every character of
//! every returned line resolves through [`label_atlas::char_index`].

use crate::label::{self, TextInstanceRaw};
use crate::label_atlas;

/// Starting capacity (in character instances) for the overlay's instance buffer — a handful of
/// short lines (docs/01's HUD, not a full label pass), so far smaller than
/// [`label::MIN_TEXT_INSTANCE_CAPACITY`].
pub const MIN_OVERLAY_INSTANCE_CAPACITY: usize = 64;

/// Extra vertical gap between HUD lines, in physical pixels, on top of
/// [`label::LABEL_CHAR_HEIGHT_PX`] — a small bit of leading so the block reads as separate lines
/// rather than a solid stack.
const LINE_LEADING_PX: f64 = 2.0;

/// Plain numeric input for the F3 HUD, filled by `app` from its own `FrameSummary` — `f64`s, not
/// `FrameSummary` itself: `render` must not depend on `app` (workspace dependency direction), and
/// `FrameSummary` lives in `app::frame_stats`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatsOverlay {
    pub fps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub worst_ms: f64,
}

/// Rounds `value` to the nearest non-negative whole number — every HUD number is a rate or a
/// duration, never meaningfully negative, and the character set has no `-` glyph to draw one
/// with anyway.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value is a rounded, non-negative fps/millisecond figure at frame-budget \
              magnitudes (docs/01's own numbers are all well under a million), far inside \
              i64's range"
)]
fn round_non_negative(value: f64) -> i64 {
    value.round().max(0.0) as i64
}

/// The HUD's lines for one report, in draw order (top to bottom). Every character in every
/// returned line is inside [`label_atlas::CHARSET`] — see [`format_lines`]'s own unit test.
pub fn format_lines(stats: &StatsOverlay, instances: usize) -> Vec<String> {
    let fps = round_non_negative(stats.fps);
    let p50 = round_non_negative(stats.p50_ms);
    let p95 = round_non_negative(stats.p95_ms);
    let worst = round_non_negative(stats.worst_ms);
    vec![
        format!("FPS {fps}"),
        format!("P50 {p50}MS  P95 {p95}MS"),
        format!("WORST {worst}MS"),
        format!("N {instances}"),
    ]
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "screen-pixel-space positions/sizes at ordinary window magnitudes stay well within \
              f32's precision"
)]
fn to_f32_pair(pair: (f64, f64)) -> [f32; 2] {
    [pair.0 as f32, pair.1 as f32]
}

/// Packs `lines` into this frame's character-instance buffer, appending into `out` (cleared
/// first, its capacity reused frame to frame per ADR-002's no-per-frame-allocation rule — the
/// same reused-scratch shape as [`label::pack_text_instances`]).
///
/// `lines` stack vertically starting at `origin_px` ([`label::LABEL_CHAR_HEIGHT_PX`] +
/// [`LINE_LEADING_PX`] apart), each line's characters advancing left to right by
/// [`label::LABEL_CHAR_WIDTH_PX`] — a fixed monospace HUD block, no wrapping/clamping (this is a
/// static top-left corner overlay, not a world-anchored label). A character outside
/// [`label_atlas::CHARSET`] is silently skipped, mirroring
/// [`label::pack_text_instances`]'s own defensive behavior — though [`format_lines`]'s own
/// charset guarantee means this should never actually trigger for this module's own output.
pub fn pack_overlay_instances(
    lines: &[String],
    origin_px: (f64, f64),
    color: [f32; 4],
    out: &mut Vec<TextInstanceRaw>,
) {
    out.clear();
    let cell_size_px = to_f32_pair((label::LABEL_CHAR_WIDTH_PX, label::LABEL_CHAR_HEIGHT_PX));
    let line_height_px = label::LABEL_CHAR_HEIGHT_PX + LINE_LEADING_PX;

    for (row, line) in lines.iter().enumerate() {
        #[allow(
            clippy::cast_precision_loss,
            reason = "row indexes a handful of HUD lines, far inside f64's exact-integer range"
        )]
        let row_y_px = origin_px.1 + row as f64 * line_height_px;
        for (col, ch) in line.chars().enumerate() {
            let Some(char_tile_index) = label_atlas::char_index(ch) else {
                continue;
            };
            #[allow(
                clippy::cast_precision_loss,
                reason = "col indexes a short HUD line (a handful of characters), far inside \
                          f64's exact-integer range"
            )]
            let col_x_px = origin_px.0 + col as f64 * label::LABEL_CHAR_WIDTH_PX;
            #[allow(
                clippy::cast_precision_loss,
                reason = "char_tile_index is one of CHAR_COUNT (39) small integers, far inside \
                          f32's exact-integer range"
            )]
            let char_index = char_tile_index as f32;
            out.push(TextInstanceRaw {
                cell_origin_px: to_f32_pair((col_x_px, row_y_px)),
                cell_size_px,
                char_index,
                color,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stats() -> StatsOverlay {
        StatsOverlay {
            fps: 58.4,
            p50_ms: 16.2,
            p95_ms: 22.7,
            worst_ms: 41.0,
        }
    }

    // ---- format_lines -----------------------------------------------------------------------

    #[test]
    fn every_character_of_every_line_is_inside_the_label_atlas_charset() {
        let lines = format_lines(&sample_stats(), 187);
        assert!(!lines.is_empty());
        for line in &lines {
            for ch in line.chars() {
                assert!(
                    label_atlas::char_index(ch).is_some(),
                    "{ch:?} in {line:?} is outside label_atlas::CHARSET"
                );
            }
        }
    }

    #[test]
    fn format_lines_rounds_to_whole_numbers_and_includes_the_instance_count() {
        let lines = format_lines(&sample_stats(), 187);
        let joined = lines.join(" ");
        assert!(joined.contains("58"), "fps must round to 58: {joined}");
        assert!(joined.contains("16"), "p50 must round to 16: {joined}");
        assert!(joined.contains("23"), "p95 must round to 23: {joined}");
        assert!(joined.contains("41"), "worst must round to 41: {joined}");
        assert!(
            joined.contains("187"),
            "instance count must appear: {joined}"
        );
        assert!(
            !joined.contains('.'),
            "no decimal point may appear: {joined}"
        );
    }

    #[test]
    fn format_lines_never_emits_a_negative_sign() {
        let stats = StatsOverlay {
            fps: -1.0,
            p50_ms: -1.0,
            p95_ms: -1.0,
            worst_ms: -1.0,
        };
        let lines = format_lines(&stats, 0);
        for line in &lines {
            assert!(!line.contains('-'), "{line:?} must not contain '-'");
        }
    }

    // ---- pack_overlay_instances --------------------------------------------------------------

    #[test]
    fn empty_lines_produce_no_instances() {
        let mut out = vec![TextInstanceRaw {
            cell_origin_px: [1.0, 1.0],
            cell_size_px: [1.0, 1.0],
            char_index: 0.0,
            color: [1.0; 4],
        }];
        pack_overlay_instances(&[], (10.0, 10.0), [1.0; 4], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn each_line_becomes_a_new_row_at_the_expected_y() {
        let lines = vec!["FPS 58".to_owned(), "N 187".to_owned()];
        let mut out = Vec::new();
        pack_overlay_instances(&lines, (10.0, 20.0), [1.0; 4], &mut out);

        assert_eq!(
            out.len(),
            lines[0].chars().count() + lines[1].chars().count()
        );

        #[allow(
            clippy::cast_possible_truncation,
            reason = "test-only expectation for a value pack_overlay_instances itself narrows \
                      the same way"
        )]
        let expected_row_height = (label::LABEL_CHAR_HEIGHT_PX + LINE_LEADING_PX) as f32;

        // First character of line 0 sits at the origin row.
        assert!((out[0].cell_origin_px[1] - 20.0).abs() < 1e-4);
        // First character of line 1 sits one row height down.
        let first_of_line_1 = lines[0].chars().count();
        assert!(
            (out[first_of_line_1].cell_origin_px[1] - (20.0 + expected_row_height)).abs() < 1e-4
        );
    }

    #[test]
    fn columns_advance_by_the_label_char_width_within_a_line() {
        let lines = vec!["AB".to_owned()];
        let mut out = Vec::new();
        pack_overlay_instances(&lines, (10.0, 20.0), [1.0; 4], &mut out);

        assert_eq!(out.len(), 2);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "test-only expectation mirroring pack_overlay_instances's own narrowing"
        )]
        let char_width_f32 = label::LABEL_CHAR_WIDTH_PX as f32;
        assert!((out[0].cell_origin_px[0] - 10.0).abs() < 1e-4);
        assert!((out[1].cell_origin_px[0] - (10.0 + char_width_f32)).abs() < 1e-4);
        assert_ne!(
            out[0].char_index, out[1].char_index,
            "'A' and 'B' are different tiles"
        );
    }

    #[test]
    fn reuses_the_output_buffer_leaving_no_stale_entries() {
        let mut out = Vec::new();
        pack_overlay_instances(&["FPS 58".to_owned()], (0.0, 0.0), [1.0; 4], &mut out);
        assert_eq!(out.len(), 6);

        pack_overlay_instances(&["N 1".to_owned()], (0.0, 0.0), [1.0; 4], &mut out);
        assert_eq!(
            out.len(),
            3,
            "the second, smaller call must leave no stale entries"
        );
    }
}
