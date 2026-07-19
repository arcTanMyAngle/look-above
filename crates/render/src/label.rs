//! CPU-side label content, screen-space placement, and collision culling for the label pass (M2
//! item 2.7b) — the render half of 2.7a's label *content* fields on `AircraftInstance`.
//! `renderer.rs` owns the GPU resources (two small pipelines: instanced text-glyph quads and a
//! leader-line `LineList`) built from what this module produces; nothing here touches `wgpu`
//! devices/queues, so all of it is plain, testable Rust — the same per-item split
//! `aircraft.rs`/`trail.rs` already established.
//!
//! **Documented deviation from docs/09** (recorded on `core::sim::RenderFeed` at 2.7a, repeated
//! here at the point it actually applies): docs/09 types a `labels: Vec<Label>` field directly on
//! `RenderFeed`, "pre-collision-culled". Collision culling and placement (right-of-glyph, flip
//! near the viewport edge, leader lines) are inherently screen-space and need the camera, which
//! `core` deliberately does not have (2.3a) — so none of that lives in `core`. This module is
//! where it actually happens, driven directly from `AircraftInstance`'s 2.7a content fields and
//! the camera's live screen-space parameters (`renderer.rs` passes `center_m`/`meters_per_pixel`/
//! `width_px`/`height_px`, not a `Camera` reference — this crate stays winit/camera-shape-free the
//! same way `aircraft.rs`/`trail.rs` only ever take `meters_per_pixel`).
//!
//! **No selection state exists yet.** The skill's collision priority is "selected > speed >
//! proximity to viewport center"; M2 item 2.8 (selection) has not landed, so there is no real
//! selection signal anywhere in the codebase. [`label_priority`] hardcodes `selected = false`
//! rather than fabricating a placeholder signal — see that function's doc comment. Wiring 2.8 in
//! later means computing a real `bool` there instead.
//!
//! **Re-evaluation cadence.** The skill requires collision/membership to be re-evaluated at
//! `≤ 5 Hz` (not per frame) with hysteresis, so a label doesn't flicker as priorities jitter
//! frame to frame. That throttling is *stateful* (which labels are currently held, and when they
//! were last re-evaluated) and therefore lives on `renderer.rs`'s `LabelLayer`, not in this pure
//! module — the same reason the collision *decision* ([`resolve_collisions`]) takes the
//! previously-held set as a plain argument rather than owning it. Between re-evaluations,
//! `LabelLayer` still re-projects each *currently held* label's screen position every frame (via
//! [`placement_geometry`] alone, without rebuilding text or re-running the collision sweep) so a
//! shown label visually tracks its aircraft smoothly at render cadence even though *which* labels
//! are shown only changes at the throttled rate.

use std::collections::HashSet;
use std::mem::size_of;

use look_above_core::geo::MercatorXy;
use look_above_core::sim::AircraftInstance;
use look_above_core::types::Icao24;

use crate::aircraft::AIRCRAFT_GLYPH_PX;
use crate::label_atlas;

/// A physical-pixel screen-space point `(x, y)` — origin top-left, y increasing downward, the
/// same convention `core::camera::Camera`'s own screen-pixel inputs use.
pub type ScreenPoint = (f64, f64);

/// A leader line's two endpoints, `(glyph-side point, label-side point)` — present only when a
/// label is displaced enough from its ideal anchor to need one (see [`placement_geometry`]).
pub type Leader = Option<(ScreenPoint, ScreenPoint)>;

/// Minimum interval between full label re-evaluations (candidate rebuild + collision sweep) —
/// the skill's "≤ 5 Hz".
pub const MIN_EVAL_INTERVAL_S: f64 = 0.2;

/// The skill's hysteresis margin: a held label is only displaced by a challenger whose priority
/// exceeds it by more than this fraction.
pub const HYSTERESIS_MARGIN: f64 = 0.10;

/// The skill's leader-line threshold, in physical pixels: a label displaced from its ideal
/// (unclamped, right-of-glyph) anchor by more than this draws a leader line back to the glyph.
pub const LEADER_DISPLACEMENT_THRESHOLD_PX: f64 = 24.0;

/// One character cell's on-screen width, in physical pixels — [`label_atlas`]'s stroke-font
/// tiles are authored on a `[0, 1]²` square, stretched onto this (narrower) cell so the rendered
/// text reads as an ordinary monospace label rather than a row of square blocks.
pub const LABEL_CHAR_WIDTH_PX: f64 = 7.0;

/// One character cell's on-screen height, in physical pixels.
pub const LABEL_CHAR_HEIGHT_PX: f64 = 12.0;

/// Gap between the aircraft glyph's edge and the start of its label, in physical pixels.
const LABEL_GLYPH_GAP_PX: f64 = 6.0;

/// Minimum distance a label is kept from the viewport's edge, in physical pixels.
const EDGE_MARGIN_PX: f64 = 4.0;

/// Priority weight for ground speed, priority-units per knot — large enough that any speed
/// difference (down to 1 kt) dominates the proximity-to-center term in [`label_priority`] at any
/// viewport size docs/01 supports (up to a handful of thousand physical pixels across), so the
/// skill's "speed, then proximity" ordering holds in practice even though both are folded into
/// one scalar (see that function's doc comment for why a scalar, not a lexicographic tuple).
const SPEED_PRIORITY_WEIGHT: f64 = 100_000.0;

/// Priority weight for `selected` — larger than any plausible combined speed+proximity term, so
/// a selected aircraft's label always wins a collision against an unselected one, matching the
/// skill's "selected > speed > proximity" ordering. Unused today (see this module's doc comment)
/// but sized correctly for when 2.8 wires a real signal through.
const SELECTED_PRIORITY_WEIGHT: f64 = 1.0e12;

// ---- Content (M2 item 2.7a → 2.7b) -----------------------------------------------------------

/// Builds a label's text from `instance`'s 2.7a content fields, or `None` if it should not be
/// labeled at all: an anonymous (PIA/blocked) target is never labeled, no exception (privacy rule
/// 2.2 — there is no selection yet to except it into "Unidentified", that wiring is 2.8's job),
/// and an instance with nothing known (no callsign, altitude, or speed) has no content to show.
///
/// Present fields join with the skill's two-space separator (`"CALLSIGN  FL350  450kt"`);
/// unknown fields are omitted entirely, never printed as a placeholder. Altitude is still shown
/// while `on_ground` (`altitude_ft`'s own doc comment: `Some(0.0)` there is real data, not
/// unknown) — this is the one place that distinction actually matters for display.
pub fn format_label_text(instance: &AircraftInstance) -> Option<String> {
    if instance.anonymous {
        return None;
    }

    let mut pieces: Vec<String> = Vec::new();
    if let Some(callsign) = &instance.callsign {
        pieces.push(callsign.as_str().to_owned());
    }
    if let Some(altitude_ft) = instance.altitude_ft {
        // Flight level: hundreds of feet, rounded — the skill's "FL350" for 35,000 ft (and,
        // deliberately, "FL0" for 0 ft while on the ground, since that is real data too).
        let flight_level = (altitude_ft / 100.0).round().max(0.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "flight_level is a rounded, non-negative value derived from a real-world \
                      altitude (at most tens of thousands of feet); far inside i64's range"
        )]
        pieces.push(format!("FL{}", flight_level as i64));
    }
    if let Some(ground_speed_kt) = instance.ground_speed_kt {
        let rounded = ground_speed_kt.round().max(0.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "rounded is a non-negative, real-world ground speed in knots; far inside \
                      i64's range"
        )]
        pieces.push(format!("{}kt", rounded as i64));
    }

    if pieces.is_empty() {
        return None;
    }
    Some(pieces.join("  "))
}

// ---- Screen-space projection and priority ------------------------------------------------------

/// Projects a Web Mercator position to physical-pixel screen space, centered on the camera.
///
/// Mirrors `Camera::pan_by_pixels`'s sign convention (see that function's doc comment): world x
/// and screen x both increase rightward (no flip), but Mercator y increases north while screen y
/// increases downward (flip).
// `dx_m`/`dy_m` are a standard cartesian-delta pair — see `Camera::pan_by_pixels`'s own
// identical exemption for the same lint.
#[allow(clippy::similar_names)]
pub fn world_to_screen_px(
    position: MercatorXy,
    camera_center_m: MercatorXy,
    meters_per_pixel: f64,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> ScreenPoint {
    let dx_m = position.x_m - camera_center_m.x_m;
    let dy_m = position.y_m - camera_center_m.y_m;
    (
        viewport_width_px / 2.0 + dx_m / meters_per_pixel,
        viewport_height_px / 2.0 - dy_m / meters_per_pixel,
    )
}

/// A candidate label's collision priority: the skill's "selected > speed > proximity to viewport
/// center", folded into one scalar (rather than a lexicographic tuple) so the hysteresis
/// percentage math in [`resolve_collisions`] has a single number to compare — see
/// [`SPEED_PRIORITY_WEIGHT`]/[`SELECTED_PRIORITY_WEIGHT`] for why the weights keep the ordering
/// intent even as a sum. Higher is drawn in preference to lower.
///
/// `selected` is always `false`: there is no selection state anywhere in the codebase yet (M2
/// item 2.8 is what adds one). This is written so wiring 2.8 in later is a one-line change here
/// (pass a real `bool` instead of the hardcoded one) rather than a redesign.
pub fn label_priority(
    ground_speed_kt: Option<f64>,
    glyph_px: ScreenPoint,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> f64 {
    let selected = false; // No selection signal exists yet — see this function's doc comment.
    let selected_component = if selected {
        SELECTED_PRIORITY_WEIGHT
    } else {
        0.0
    };
    let speed_component = ground_speed_kt.unwrap_or(0.0) * SPEED_PRIORITY_WEIGHT;

    let center_px = (viewport_width_px / 2.0, viewport_height_px / 2.0);
    let distance_from_center_px = (glyph_px.0 - center_px.0).hypot(glyph_px.1 - center_px.1);

    // Closer to center is higher priority, so proximity contributes negatively with distance.
    selected_component + speed_component - distance_from_center_px
}

// ---- Candidates -----------------------------------------------------------------------------

/// One aircraft's would-be label before collision resolution: its text, its glyph's current
/// screen position (the placement anchor for [`placement_geometry`]), and its priority.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelCandidate {
    pub icao24: Icao24,
    pub text: String,
    pub glyph_px: ScreenPoint,
    pub priority: f64,
}

/// Whether a glyph at `glyph_px` is actually visible in a `viewport_width_px` ×
/// `viewport_height_px` viewport — the aircraft pass has no CPU-side viewport culling of its own
/// (an off-screen instance's quad simply never rasterizes, since `wgpu` clips it in clip space),
/// so this is the label pass's equivalent check: without it, an aircraft far outside the current
/// view (the feed can span a wider region than the viewport — e.g. right after a camera zoom,
/// before the poller has retargeted) would still get a candidate, and [`placement_geometry`]'s
/// edge clamp would then draw a label with no glyph anywhere near it. The margin is the glyph's
/// own half-width, so a glyph straddling the exact edge (and therefore still partly drawn) still
/// gets labeled.
fn glyph_is_visible(
    glyph_px: ScreenPoint,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> bool {
    let margin = AIRCRAFT_GLYPH_PX / 2.0;
    glyph_px.0 >= -margin
        && glyph_px.0 <= viewport_width_px + margin
        && glyph_px.1 >= -margin
        && glyph_px.1 <= viewport_height_px + margin
}

/// Builds one label candidate per labelable, on-screen aircraft in `aircraft` (anonymous or
/// content-less instances are simply absent — see [`format_label_text`] — and so is any aircraft
/// whose glyph isn't actually visible in the current viewport — see [`glyph_is_visible`]).
pub fn build_candidates(
    aircraft: &[AircraftInstance],
    camera_center_m: MercatorXy,
    meters_per_pixel: f64,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> Vec<LabelCandidate> {
    aircraft
        .iter()
        .filter_map(|instance| {
            let text = format_label_text(instance)?;
            let glyph_px = world_to_screen_px(
                instance.position,
                camera_center_m,
                meters_per_pixel,
                viewport_width_px,
                viewport_height_px,
            );
            if !glyph_is_visible(glyph_px, viewport_width_px, viewport_height_px) {
                return None;
            }
            let priority = label_priority(
                instance.ground_speed_kt,
                glyph_px,
                viewport_width_px,
                viewport_height_px,
            );
            Some(LabelCandidate {
                icao24: instance.icao24,
                text,
                glyph_px,
                priority,
            })
        })
        .collect()
}

/// Finds `icao24`'s current instance in `aircraft` — which `core::sim::RenderFeed` guarantees is
/// sorted by address (`Simulator::advance_all`'s own doc comment), so this is a binary search, not
/// a scan. Used by `renderer.rs`'s `LabelLayer` to re-project a *held* label's screen position
/// every frame without rebuilding the whole candidate list (see this module's doc comment on the
/// re-evaluation cadence).
pub fn find_instance(aircraft: &[AircraftInstance], icao24: Icao24) -> Option<&AircraftInstance> {
    aircraft
        .binary_search_by_key(&icao24.as_bytes(), |instance| instance.icao24.as_bytes())
        .ok()
        .map(|index| &aircraft[index])
}

// ---- Placement --------------------------------------------------------------------------------

/// A text box's on-screen size for `text` — [`LABEL_CHAR_WIDTH_PX`] per character (monospace),
/// one line tall.
fn text_box_size(text: &str) -> (f64, f64) {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a label's character count is a handful (docs/01's own content format is short), \
                  nowhere near usize's width where f64's 52-bit mantissa would actually lose a \
                  character"
    )]
    let char_count = text.chars().count() as f64;
    (char_count * LABEL_CHAR_WIDTH_PX, LABEL_CHAR_HEIGHT_PX)
}

/// The label's ideal anchor (top-left corner of its text box): immediately right of the glyph,
/// vertically centered on it. Unclamped — may fall (partly) off-screen; [`placement_geometry`]
/// adjusts from here.
fn ideal_anchor(glyph_px: ScreenPoint, text_height_px: f64) -> ScreenPoint {
    (
        glyph_px.0 + AIRCRAFT_GLYPH_PX / 2.0 + LABEL_GLYPH_GAP_PX,
        glyph_px.1 - text_height_px / 2.0,
    )
}

/// The nearest point on the axis-aligned box `[anchor, anchor + (w, h)]` to `from` — the leader
/// line's endpoint on the label side, so it terminates at the box's near edge rather than a
/// (possibly far) corner.
fn nearest_point_on_box(
    anchor: ScreenPoint,
    width_px: f64,
    height_px: f64,
    from: ScreenPoint,
) -> ScreenPoint {
    (
        from.0.clamp(anchor.0, anchor.0 + width_px),
        from.1.clamp(anchor.1, anchor.1 + height_px),
    )
}

/// Computes a label's actual anchor and (if any) leader line for a text box of `text_width_px` ×
/// `text_height_px` anchored ideally at [`ideal_anchor`] of `glyph_px`.
///
/// Placement: right of the glyph by default; flips to the glyph's *left* when the ideal
/// (right-of-glyph) box would run past the viewport's right edge (the skill's "flip left near
/// viewport edge"). The anchor is then clamped to stay fully within the viewport on every side —
/// docs/01's "no visible... popping" quality bar extends to a label's own box, not just the
/// aircraft glyph. A leader line back to the glyph is drawn when the actual anchor ends up more
/// than [`LEADER_DISPLACEMENT_THRESHOLD_PX`] from the *ideal* one (in practice: whenever the flip
/// fires, since flipping to the opposite side of the glyph is always a large displacement; the
/// viewport-edge clamp alone, for a single-line label, never displaces this far on its own).
///
/// Pure geometry only — no text is touched or allocated, so `renderer.rs`'s `LabelLayer` can call
/// this every frame to re-track a held label's moving glyph without paying [`format_label_text`]'s
/// allocation cost outside the throttled re-evaluation tick (see this module's doc comment).
pub fn placement_geometry(
    glyph_px: ScreenPoint,
    text_width_px: f64,
    text_height_px: f64,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> (ScreenPoint, Leader) {
    let ideal = ideal_anchor(glyph_px, text_height_px);

    let flip = ideal.0 + text_width_px > viewport_width_px - EDGE_MARGIN_PX;
    let unclamped_x = if flip {
        glyph_px.0 - AIRCRAFT_GLYPH_PX / 2.0 - LABEL_GLYPH_GAP_PX - text_width_px
    } else {
        ideal.0
    };

    let max_x = (viewport_width_px - EDGE_MARGIN_PX - text_width_px).max(EDGE_MARGIN_PX);
    let max_y = (viewport_height_px - EDGE_MARGIN_PX - text_height_px).max(EDGE_MARGIN_PX);
    let anchor = (
        unclamped_x.clamp(EDGE_MARGIN_PX, max_x),
        ideal.1.clamp(EDGE_MARGIN_PX, max_y),
    );

    let displacement_px = (anchor.0 - ideal.0).hypot(anchor.1 - ideal.1);
    let leader = (displacement_px > LEADER_DISPLACEMENT_THRESHOLD_PX).then(|| {
        (
            glyph_px,
            nearest_point_on_box(anchor, text_width_px, text_height_px, glyph_px),
        )
    });

    (anchor, leader)
}

/// One label actually shown this evaluation: its text (owned, kept between re-evaluations so
/// `renderer.rs`'s per-frame refresh never re-allocates it — see this module's doc comment) and
/// its current placement.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelPlacement {
    pub icao24: Icao24,
    pub text: String,
    /// Top-left corner of the text box, physical pixels.
    pub anchor_px: ScreenPoint,
    pub width_px: f64,
    pub height_px: f64,
    /// `(glyph_px, label-side point)`, present only when displaced more than
    /// [`LEADER_DISPLACEMENT_THRESHOLD_PX`] from the ideal anchor.
    pub leader: Leader,
}

/// Places one candidate, producing its full [`LabelPlacement`] (including the owned text clone —
/// only used when actually accepting a *new* candidate at a throttled re-evaluation; the
/// steady-state per-frame refresh calls [`placement_geometry`] directly instead, see this
/// module's doc comment).
fn place_candidate(
    candidate: &LabelCandidate,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> LabelPlacement {
    let (width_px, height_px) = text_box_size(&candidate.text);
    let (anchor_px, leader) = placement_geometry(
        candidate.glyph_px,
        width_px,
        height_px,
        viewport_width_px,
        viewport_height_px,
    );
    LabelPlacement {
        icao24: candidate.icao24,
        text: candidate.text.clone(),
        anchor_px,
        width_px,
        height_px,
        leader,
    }
}

// ---- Collision sweep (M2 item 2.7b) ------------------------------------------------------------

fn aabb_overlap(a: &LabelPlacement, b: &LabelPlacement) -> bool {
    a.anchor_px.0 < b.anchor_px.0 + b.width_px
        && b.anchor_px.0 < a.anchor_px.0 + a.width_px
        && a.anchor_px.1 < b.anchor_px.1 + b.height_px
        && b.anchor_px.1 < a.anchor_px.1 + a.height_px
}

/// Runs the skill's collision sweep: candidates are placed in priority order (highest first); a
/// candidate whose box overlaps an already-accepted one is **culled entirely** — never shrunk,
/// never repositioned to avoid the overlap.
///
/// `held` is the set of `icao24`s that held a slot as of the *previous* evaluation. Hysteresis
/// (the skill's "a label keeps its slot until its priority is beaten by > 10%") is implemented as
/// a priority *boost* applied only to held candidates during sorting: a held candidate is ranked
/// as if its priority were `priority * (1 + HYSTERESIS_MARGIN)`, so a challenger only outranks
/// (and therefore, on overlap, culls) it once the challenger's own raw priority exceeds that
/// boosted value — exactly the skill's ">10%" margin. A non-held candidate is ranked at its own
/// raw priority.
///
/// Ties (and the boosted-priority comparison generally) break on `icao24` bytes for a
/// deterministic, reproducible sweep order.
pub fn resolve_collisions(
    candidates: &[LabelCandidate],
    held: &HashSet<Icao24>,
    viewport_width_px: f64,
    viewport_height_px: f64,
) -> Vec<LabelPlacement> {
    let ranked_key = |candidate: &LabelCandidate| {
        if held.contains(&candidate.icao24) {
            candidate.priority * (1.0 + HYSTERESIS_MARGIN)
        } else {
            candidate.priority
        }
    };

    let mut order: Vec<&LabelCandidate> = candidates.iter().collect();
    order.sort_by(|a, b| {
        ranked_key(b)
            .total_cmp(&ranked_key(a))
            .then_with(|| a.icao24.as_bytes().cmp(&b.icao24.as_bytes()))
    });

    let mut accepted: Vec<LabelPlacement> = Vec::with_capacity(order.len());
    for candidate in order {
        let placement = place_candidate(candidate, viewport_width_px, viewport_height_px);
        if accepted
            .iter()
            .any(|existing| aabb_overlap(existing, &placement))
        {
            continue;
        }
        accepted.push(placement);
    }
    accepted
}

// ---- GPU packing --------------------------------------------------------------------------------

/// Starting capacity (in character instances) for the text-glyph instance buffer.
pub const MIN_TEXT_INSTANCE_CAPACITY: usize = 256;

/// Starting capacity (in vertices; two per leader line) for the leader-line vertex buffer.
pub const MIN_LEADER_VERTEX_CAPACITY: usize = 64;

/// One vertex of the shared unit quad every character cell reuses. Unlike
/// [`crate::aircraft::QuadVertex`] (local space `[-0.5, 0.5]`, centered), this spans `[0, 1]` on
/// both axes with `+y` **down** — screen-pixel convention, since label text is a screen-space
/// overlay with no rotation, not a world-space rotated glyph.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextQuadVertex {
    pub local_pos: [f32; 2],
    pub local_uv: [f32; 2],
}

impl TextQuadVertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<TextQuadVertex>() as wgpu::BufferAddress,
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

/// The shared text quad's four corners: `local_pos`/`local_uv` coincide exactly (both go
/// top-left `(0,0)` to bottom-right `(1,1)`), since [`label_atlas`]'s texel-to-local mapping
/// already puts row 0 (top of the rasterized tile) at the top of screen-pixel space — no flip
/// needed, unlike `aircraft::quad_vertices`'s centered/rotated convention.
pub fn text_quad_vertices() -> [TextQuadVertex; 4] {
    [
        TextQuadVertex {
            local_pos: [0.0, 0.0],
            local_uv: [0.0, 0.0],
        },
        TextQuadVertex {
            local_pos: [1.0, 0.0],
            local_uv: [1.0, 0.0],
        },
        TextQuadVertex {
            local_pos: [1.0, 1.0],
            local_uv: [1.0, 1.0],
        },
        TextQuadVertex {
            local_pos: [0.0, 1.0],
            local_uv: [0.0, 1.0],
        },
    ]
}

/// Two triangles covering [`text_quad_vertices`]'s square.
pub const TEXT_QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

/// One character's packed per-instance GPU attributes.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextInstanceRaw {
    /// Top-left corner of this character's cell, physical screen pixels.
    pub cell_origin_px: [f32; 2],
    /// This cell's on-screen size, physical pixels — [`LABEL_CHAR_WIDTH_PX`] ×
    /// [`LABEL_CHAR_HEIGHT_PX`], carried per-instance (rather than a shared uniform) so a future
    /// per-label font-size change needs no shader change.
    pub cell_size_px: [f32; 2],
    /// [`label_atlas::char_index`]'s tile index.
    pub char_index: f32,
    pub color: [f32; 4],
}

impl TextInstanceRaw {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<TextInstanceRaw>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: (2 * size_of::<[f32; 2]>()) as wgpu::BufferAddress,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: (2 * size_of::<[f32; 2]>() + size_of::<f32>()) as wgpu::BufferAddress,
                shader_location: 5,
            },
        ],
    };
}

/// One leader-line vertex — `renderer.rs` draws two of these (start, end) per displaced label as
/// a `LineList`, the same "CPU bakes it, GPU just transforms it" shape as
/// [`crate::trail::TrailVertexRaw`].
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LeaderVertexRaw {
    pub screen_px: [f32; 2],
    pub color: [f32; 4],
}

impl LeaderVertexRaw {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<LeaderVertexRaw>() as wgpu::BufferAddress,
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

#[allow(
    clippy::cast_possible_truncation,
    reason = "screen-pixel-space positions/sizes at ordinary window magnitudes stay well within \
              f32's precision"
)]
fn to_f32_pair(pair: (f64, f64)) -> [f32; 2] {
    [pair.0 as f32, pair.1 as f32]
}

/// Packs `placements`' text into this frame's character-instance buffer, appending into `out`
/// (cleared first, its capacity reused frame to frame per ADR-002's no-per-frame-allocation
/// rule — this is the same reused-scratch shape as `aircraft::pack_instance`'s caller and
/// `trail::tessellate_trails`).
///
/// A character outside [`label_atlas::CHARSET`] is silently skipped (see
/// [`label_atlas::char_index`]'s own doc comment) rather than panicking.
pub fn pack_text_instances(
    placements: &[LabelPlacement],
    text_color: [f32; 4],
    out: &mut Vec<TextInstanceRaw>,
) {
    out.clear();
    let cell_size_px = to_f32_pair((LABEL_CHAR_WIDTH_PX, LABEL_CHAR_HEIGHT_PX));
    for placement in placements {
        for (i, ch) in placement.text.chars().enumerate() {
            let Some(char_tile_index) = label_atlas::char_index(ch) else {
                continue;
            };
            #[allow(
                clippy::cast_precision_loss,
                reason = "i indexes a short label string (a handful of characters), far inside \
                          f64's exact-integer range"
            )]
            let cell_x_px = placement.anchor_px.0 + i as f64 * LABEL_CHAR_WIDTH_PX;
            #[allow(
                clippy::cast_precision_loss,
                reason = "char_tile_index is one of CHAR_COUNT (39) small integers, far inside \
                          f32's exact-integer range"
            )]
            let char_index = char_tile_index as f32;
            out.push(TextInstanceRaw {
                cell_origin_px: to_f32_pair((cell_x_px, placement.anchor_px.1)),
                cell_size_px,
                char_index,
                color: text_color,
            });
        }
    }
}

/// Packs `placements`' leader lines (where present) into this frame's leader vertex buffer,
/// appending into `out` (cleared first, reused the same way as [`pack_text_instances`]).
pub fn pack_leader_vertices(
    placements: &[LabelPlacement],
    leader_color: [f32; 4],
    out: &mut Vec<LeaderVertexRaw>,
) {
    out.clear();
    for placement in placements {
        if let Some((from, to)) = placement.leader {
            out.push(LeaderVertexRaw {
                screen_px: to_f32_pair(from),
                color: leader_color,
            });
            out.push(LeaderVertexRaw {
                screen_px: to_f32_pair(to),
                color: leader_color,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use look_above_core::contracts::AircraftCategory;
    use look_above_core::sim::AltitudeBucket;

    use super::*;

    fn icao(hex: &str) -> Icao24 {
        Icao24::from_hex(hex).expect("valid test ICAO24")
    }

    /// A minimal, fully-known instance for content tests — every optional field populated, so
    /// individual tests can strip just the one field they're probing.
    fn full_instance() -> AircraftInstance {
        AircraftInstance {
            icao24: icao("3c6444"),
            position: MercatorXy::new(0.0, 0.0),
            heading_deg: 0.0,
            altitude_bucket: AltitudeBucket::To28000Ft,
            category: AircraftCategory::Unknown,
            alpha: 1.0,
            on_ground: false,
            anonymous: false,
            callsign: look_above_core::types::CallSign::new("DLH9LF"),
            altitude_ft: Some(35_000.0),
            ground_speed_kt: Some(450.0),
        }
    }

    // ---- Content (M2 item 2.7a → 2.7b) -----------------------------------------------------

    #[test]
    fn formats_every_field_when_all_are_known() {
        let text = format_label_text(&full_instance()).expect("content present");
        assert_eq!(text, "DLH9LF  FL350  450kt");
    }

    #[test]
    fn omits_a_missing_callsign() {
        let mut instance = full_instance();
        instance.callsign = None;
        assert_eq!(
            format_label_text(&instance).as_deref(),
            Some("FL350  450kt")
        );
    }

    #[test]
    fn omits_a_missing_altitude() {
        let mut instance = full_instance();
        instance.altitude_ft = None;
        assert_eq!(
            format_label_text(&instance).as_deref(),
            Some("DLH9LF  450kt")
        );
    }

    #[test]
    fn omits_a_missing_speed() {
        let mut instance = full_instance();
        instance.ground_speed_kt = None;
        assert_eq!(
            format_label_text(&instance).as_deref(),
            Some("DLH9LF  FL350")
        );
    }

    #[test]
    fn ground_altitude_of_zero_feet_is_still_shown_as_real_data() {
        let mut instance = full_instance();
        instance.on_ground = true;
        instance.altitude_ft = Some(0.0);
        instance.ground_speed_kt = None;
        assert_eq!(format_label_text(&instance).as_deref(), Some("DLH9LF  FL0"));
    }

    #[test]
    fn nothing_known_at_all_yields_no_label() {
        let mut instance = full_instance();
        instance.callsign = None;
        instance.altitude_ft = None;
        instance.ground_speed_kt = None;
        assert_eq!(format_label_text(&instance), None);
    }

    #[test]
    fn an_anonymous_target_is_never_labeled_even_with_full_content() {
        let mut instance = full_instance();
        instance.anonymous = true;
        assert_eq!(format_label_text(&instance), None);
    }

    // ---- Projection -------------------------------------------------------------------------

    #[test]
    fn the_camera_center_projects_to_the_viewport_center() {
        let center = MercatorXy::new(1_000.0, 2_000.0);
        let (x, y) = world_to_screen_px(center, center, 10.0, 1000.0, 800.0);
        assert!((x - 500.0).abs() < 1e-9);
        assert!((y - 400.0).abs() < 1e-9);
    }

    #[test]
    fn moving_east_increases_screen_x_and_moving_north_decreases_screen_y() {
        let center = MercatorXy::new(0.0, 0.0);
        let mpp = 10.0;
        let east = world_to_screen_px(MercatorXy::new(100.0, 0.0), center, mpp, 1000.0, 800.0);
        let north = world_to_screen_px(MercatorXy::new(0.0, 100.0), center, mpp, 1000.0, 800.0);
        assert!(east.0 > 500.0);
        assert!(north.1 < 400.0);
    }

    // ---- Priority -----------------------------------------------------------------------------

    #[test]
    fn faster_aircraft_gets_higher_priority_at_the_same_position() {
        let px = (500.0, 400.0);
        let slow = label_priority(Some(100.0), px, 1000.0, 800.0);
        let fast = label_priority(Some(500.0), px, 1000.0, 800.0);
        assert!(fast > slow);
    }

    #[test]
    fn closer_to_center_gets_higher_priority_at_the_same_speed() {
        let center_px = (500.0, 400.0);
        let far_px = (0.0, 0.0);
        let near = label_priority(Some(200.0), center_px, 1000.0, 800.0);
        let far = label_priority(Some(200.0), far_px, 1000.0, 800.0);
        assert!(near > far);
    }

    // ---- Candidates / visibility --------------------------------------------------------------

    #[test]
    fn build_candidates_excludes_an_aircraft_whose_glyph_is_off_screen() {
        let mut instance = full_instance();
        // Far west of the camera center at this scale — projects well outside the viewport, the
        // way an aircraft outside the current view but still in the feed (e.g. right after a
        // camera zoom, before the poller retargets) would.
        instance.position = MercatorXy::new(-10_000_000.0, 0.0);
        let candidates =
            build_candidates(&[instance], MercatorXy::new(0.0, 0.0), 10.0, 1000.0, 800.0);
        assert!(
            candidates.is_empty(),
            "an off-screen aircraft must not produce a floating label"
        );
    }

    #[test]
    fn build_candidates_includes_an_aircraft_whose_glyph_is_on_screen() {
        let instance = full_instance(); // position (0, 0) == camera center -> screen center.
        let candidates =
            build_candidates(&[instance], MercatorXy::new(0.0, 0.0), 10.0, 1000.0, 800.0);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn glyph_is_visible_includes_a_glyph_straddling_the_edge_within_its_own_half_width() {
        assert!(glyph_is_visible((-5.0, 400.0), 1000.0, 800.0));
        assert!(!glyph_is_visible((-50.0, 400.0), 1000.0, 800.0));
    }

    // ---- Placement ------------------------------------------------------------------------------

    #[test]
    fn default_placement_is_right_of_the_glyph_and_vertically_centered() {
        let glyph_px = (500.0, 400.0);
        let (anchor, leader) = placement_geometry(glyph_px, 60.0, 12.0, 1000.0, 800.0);
        assert!(
            anchor.0 > glyph_px.0,
            "label must sit to the right by default"
        );
        assert!(
            (anchor.1 - (glyph_px.1 - 6.0)).abs() < 1e-9,
            "must be vertically centered"
        );
        assert_eq!(leader, None, "the default placement is not displaced");
    }

    #[test]
    fn placement_flips_to_the_left_near_the_right_edge() {
        // A glyph close enough to the right edge that a 60px-wide label to its right would run
        // off-screen.
        let glyph_px = (980.0, 400.0);
        let (anchor, leader) = placement_geometry(glyph_px, 60.0, 12.0, 1000.0, 800.0);
        assert!(
            anchor.0 < glyph_px.0,
            "must flip to the glyph's left near the right edge"
        );
        assert!(
            leader.is_some(),
            "flipping to the other side of the glyph is a real displacement"
        );
    }

    #[test]
    fn placement_stays_within_the_viewport_near_the_top_and_bottom_edges() {
        let (top_anchor, _) = placement_geometry((500.0, 1.0), 60.0, 12.0, 1000.0, 800.0);
        assert!(top_anchor.1 >= EDGE_MARGIN_PX - 1e-9);

        let (bottom_anchor, _) = placement_geometry((500.0, 799.0), 60.0, 12.0, 1000.0, 800.0);
        assert!(bottom_anchor.1 + 12.0 <= 800.0 - EDGE_MARGIN_PX + 1e-9);
    }

    #[test]
    fn no_leader_line_when_the_label_is_not_displaced() {
        let (_, leader) = placement_geometry((500.0, 400.0), 40.0, 12.0, 1000.0, 800.0);
        assert_eq!(leader, None);
    }

    // ---- Collision sweep --------------------------------------------------------------------

    fn candidate(icao_hex: &str, glyph_px: (f64, f64), priority: f64) -> LabelCandidate {
        LabelCandidate {
            icao24: icao(icao_hex),
            text: "AAA123".to_owned(),
            glyph_px,
            priority,
        }
    }

    #[test]
    fn a_lower_priority_overlapping_candidate_is_culled_entirely_not_shrunk() {
        // Two candidates whose glyphs (and therefore ideal label boxes) sit right on top of one
        // another — guaranteed overlap.
        let strong = candidate("3c6444", (500.0, 400.0), 10.0);
        let weak = candidate("4b1815", (505.0, 400.0), 1.0);
        let placements =
            resolve_collisions(&[strong.clone(), weak], &HashSet::new(), 1000.0, 800.0);

        assert_eq!(
            placements.len(),
            1,
            "the loser must be culled, not merged/shrunk"
        );
        assert_eq!(placements[0].icao24, strong.icao24);
        // Full text width survives — not shrunk to fit.
        let (expected_w, expected_h) = text_box_size(&strong.text);
        assert!((placements[0].width_px - expected_w).abs() < 1e-9);
        assert!((placements[0].height_px - expected_h).abs() < 1e-9);
    }

    #[test]
    fn non_overlapping_candidates_are_both_kept() {
        let a = candidate("3c6444", (100.0, 100.0), 5.0);
        let b = candidate("4b1815", (900.0, 700.0), 1.0);
        let placements = resolve_collisions(&[a, b], &HashSet::new(), 1000.0, 800.0);
        assert_eq!(placements.len(), 2);
    }

    #[test]
    fn a_held_label_keeps_its_slot_when_the_challenger_is_under_the_hysteresis_margin() {
        let holder = candidate("3c6444", (500.0, 400.0), 100.0);
        // 9% stronger — under the 10% margin, must not evict the holder.
        let challenger = candidate("4b1815", (505.0, 400.0), 108.0);
        let mut held = HashSet::new();
        held.insert(holder.icao24);

        let placements = resolve_collisions(&[holder.clone(), challenger], &held, 1000.0, 800.0);
        assert_eq!(placements.len(), 1);
        assert_eq!(
            placements[0].icao24, holder.icao24,
            "holder must keep its slot"
        );
    }

    #[test]
    fn a_held_label_is_displaced_once_the_challenger_beats_it_by_over_ten_percent() {
        let holder = candidate("3c6444", (500.0, 400.0), 100.0);
        // 11% stronger — over the margin, must evict the holder.
        let challenger = candidate("4b1815", (505.0, 400.0), 111.0);
        let mut held = HashSet::new();
        held.insert(holder.icao24);

        let placements = resolve_collisions(&[holder, challenger.clone()], &held, 1000.0, 800.0);
        assert_eq!(placements.len(), 1);
        assert_eq!(
            placements[0].icao24, challenger.icao24,
            "challenger must win the slot"
        );
    }

    // ---- find_instance ------------------------------------------------------------------------

    #[test]
    fn find_instance_locates_an_aircraft_by_address_in_a_sorted_slice() {
        let mut a = full_instance();
        a.icao24 = icao("3c6444");
        let mut b = full_instance();
        b.icao24 = icao("4b1815");
        let aircraft = vec![a.clone(), b.clone()];

        assert_eq!(find_instance(&aircraft, icao("4b1815")), Some(&b));
        assert_eq!(find_instance(&aircraft, icao("aa0011")), None);
    }

    // ---- GPU packing ----------------------------------------------------------------------------

    fn placement(text: &str) -> LabelPlacement {
        let (width_px, height_px) = text_box_size(text);
        LabelPlacement {
            icao24: icao("3c6444"),
            text: text.to_owned(),
            anchor_px: (10.0, 20.0),
            width_px,
            height_px,
            leader: None,
        }
    }

    #[test]
    fn pack_text_instances_emits_one_instance_per_character_at_advancing_cells() {
        let placements = vec![placement("AB")];
        let mut out = Vec::new();
        pack_text_instances(&placements, [1.0, 1.0, 1.0, 1.0], &mut out);

        assert_eq!(out.len(), 2);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "test-only expectation for a value pack_text_instances itself narrows the \
                      same way"
        )]
        let char_width_f32 = LABEL_CHAR_WIDTH_PX as f32;
        assert!((out[0].cell_origin_px[0] - 10.0).abs() < 1e-4);
        assert!((out[1].cell_origin_px[0] - (10.0 + char_width_f32)).abs() < 1e-4);
        assert_ne!(
            out[0].char_index, out[1].char_index,
            "'A' and 'B' are different tiles"
        );
    }

    #[test]
    fn pack_text_instances_skips_unsupported_characters_without_panicking() {
        let placements = vec![placement("A!B")];
        let mut out = Vec::new();
        pack_text_instances(&placements, [1.0, 1.0, 1.0, 1.0], &mut out);
        assert_eq!(
            out.len(),
            2,
            "the unsupported '!' must be skipped, not drawn or panicked on"
        );
    }

    #[test]
    fn pack_text_instances_reuses_the_output_buffer() {
        let mut out = Vec::new();
        pack_text_instances(&[placement("AB")], [1.0; 4], &mut out);
        assert_eq!(out.len(), 2);
        pack_text_instances(&[], [1.0; 4], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn pack_leader_vertices_emits_two_vertices_only_for_displaced_labels() {
        let mut with_leader = placement("AB");
        with_leader.leader = Some(((1.0, 2.0), (3.0, 4.0)));
        let without_leader = placement("CD");

        let mut out = Vec::new();
        pack_leader_vertices(&[with_leader, without_leader], [0.5; 4], &mut out);
        assert_eq!(out.len(), 2);
        assert!((out[0].screen_px[0] - 1.0).abs() < 1e-6);
        assert!((out[1].screen_px[0] - 3.0).abs() < 1e-6);
    }
}
