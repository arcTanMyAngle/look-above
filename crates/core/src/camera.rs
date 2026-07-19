//! Regional (Web Mercator) camera state: pan, cursor-anchored zoom, and pan inertia.
//!
//! Pure state and arithmetic only — no wgpu, no matrices, no winit. A separate render-side
//! builder reads [`Camera`]'s accessors to construct the actual view/projection matrix; this
//! module only tracks where the viewport is and how it is moving.
//!
//! All screen/pixel inputs are **physical pixels**, origin top-left, y increasing downward —
//! matching winit's `PhysicalPosition` and the convention `crates/render/src/renderer.rs`
//! already uses for window width/height. World positions stay in Web Mercator metres
//! ([`MercatorXy`], produced by [`crate::geo::web_mercator_forward`]); this module never
//! touches latitude/longitude directly.
//!
//! Scope: the *regional* Web Mercator camera only (M2 item 2.3a). There is no global/
//! orthographic view yet, so [`max_meters_per_pixel`] — the "whole projected world visible,
//! letterboxed" scale — doubles as both the initial framing and the zoom-out ceiling; zooming
//! out further would show empty space with nothing to fill it.

use crate::geo::{MercatorXy, WEB_MERCATOR_EXTENT_M};

/// An arbitrary conservative close-zoom floor. There is no aircraft content to zoom in on yet
/// (M2 item 2.5+), so this only guards against a degenerate/zero scale. Revisit once glyphs
/// exist.
const MIN_METERS_PER_PIXEL: f64 = 0.5;

/// 10% zoom change per wheel notch.
const ZOOM_STEP_FACTOR: f64 = 1.1;

/// Exponential time constant for the eased approach to `target_meters_per_pixel`.
const ZOOM_EASE_TAU_S: f64 = 0.12;

/// Exponential decay time constant for post-drag pan-inertia velocity.
const PAN_FRICTION_TAU_S: f64 = 0.35;

/// Velocity magnitude below which coasting snaps to a full stop — avoids perpetual
/// imperceptible drift.
const STOP_SPEED_M_PER_S: f64 = 0.5;

/// The "whole projected world visible, letterboxed" scale: the camera must never zoom out past
/// this, because there is no globe/orthographic view yet (see module docs).
///
/// Derivation: the pre-camera placeholder (`fit_to_window_matrix` in
/// `crates/render/src/renderer.rs`) fit the ±1 normalized world plane into the window by
/// scaling the *wider* pixel-dimension's axis down — a "contain" fit keyed off
/// `min(width_px, height_px)`. Reproducing that framing as a `meters_per_pixel` value gives
/// `2 * WEB_MERCATOR_EXTENT_M / min(width_px, height_px)`.
fn max_meters_per_pixel(width_px: f64, height_px: f64) -> f64 {
    2.0 * WEB_MERCATOR_EXTENT_M / width_px.min(height_px)
}

/// The initial zoom level: the same "contain" fit as [`max_meters_per_pixel`], since the
/// starting view and the zoom-out ceiling are the same framing.
fn default_meters_per_pixel(width_px: f64, height_px: f64) -> f64 {
    max_meters_per_pixel(width_px, height_px)
}

/// One window's regional (Web Mercator) camera: pan (drag), zoom (mouse wheel, cursor-
/// anchored), and pan inertia (coast-and-decelerate after a drag release).
///
/// All screen/pixel arguments to this type's methods are physical pixels, origin top-left, y
/// increasing downward. World coordinates are Web Mercator metres.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// World position shown at the viewport center.
    center_m: MercatorXy,
    /// Current zoom level: world metres spanned by one physical pixel.
    meters_per_pixel: f64,
    /// What `meters_per_pixel` is easing toward; equal to `meters_per_pixel` when not zooming.
    target_meters_per_pixel: f64,
    /// Cursor's clip-space position from the most recent wheel event. Kept fixed in world
    /// space while the zoom ease runs; cleared once `meters_per_pixel` converges.
    zoom_anchor_clip: Option<(f64, f64)>,
    /// Pan-inertia coasting velocity, in world metres/second.
    velocity_m_per_s: (f64, f64),
    is_dragging: bool,
    width_px: f64,
    height_px: f64,
}

impl Camera {
    /// A new camera centered on the equator/prime meridian, framed to contain the whole
    /// projected world in the given window (see [`default_meters_per_pixel`]).
    ///
    /// `width_px`/`height_px` are clamped to at least 1 before use — a 0×0 minimized window
    /// must not divide by zero, the same guard `Renderer::new`/`resize` already apply.
    pub fn new(width_px: u32, height_px: u32) -> Self {
        let width_px = f64::from(width_px.max(1));
        let height_px = f64::from(height_px.max(1));
        let meters_per_pixel = default_meters_per_pixel(width_px, height_px);

        Self {
            center_m: MercatorXy::new(0.0, 0.0),
            meters_per_pixel,
            target_meters_per_pixel: meters_per_pixel,
            zoom_anchor_clip: None,
            velocity_m_per_s: (0.0, 0.0),
            is_dragging: false,
            width_px,
            height_px,
        }
    }

    /// Updates the viewport size, re-clamping the zoom to the new letterbox ceiling so a
    /// shrinking window cannot leave the camera zoomed out past it.
    pub fn resize(&mut self, width_px: u32, height_px: u32) {
        self.width_px = f64::from(width_px.max(1));
        self.height_px = f64::from(height_px.max(1));

        let max_mpp = max_meters_per_pixel(self.width_px, self.height_px);
        self.meters_per_pixel = self.meters_per_pixel.min(max_mpp);
        self.target_meters_per_pixel = self.target_meters_per_pixel.min(max_mpp);
    }

    /// Immediate 1:1 drag pan — no easing.
    ///
    /// The sign convention follows from requiring that the world point under the cursor at
    /// drag-start stays under the cursor as the mouse moves: screen and world x both increase
    /// rightward (no flip), but screen y increases downward while Mercator y increases
    /// northward (flip).
    // `dx_px`/`dy_px` are standard cartesian-delta vocabulary; the pedantic lint's
    // single-character-difference heuristic has no real confusion risk here.
    #[allow(clippy::similar_names)]
    pub fn pan_by_pixels(&mut self, dx_px: f64, dy_px: f64) {
        self.center_m.x_m -= dx_px * self.meters_per_pixel;
        self.center_m.y_m += dy_px * self.meters_per_pixel;
    }

    /// Starts a drag. Cancels any in-flight inertia coast — grabbing the map stops it.
    pub fn begin_drag(&mut self) {
        self.is_dragging = true;
        self.velocity_m_per_s = (0.0, 0.0);
    }

    /// Call on every pointer-move while dragging, with the incremental pixel delta *since the
    /// last call* and the elapsed time since the last call.
    ///
    /// Pans immediately by the delta, then folds the instantaneous pixel velocity (converted
    /// to world m/s with the same sign/scale convention as [`Camera::pan_by_pixels`]) into
    /// `velocity_m_per_s` as an exponential moving average, so the inertia coast started by
    /// [`Camera::end_drag`] reflects the recent drag motion rather than a single noisy sample.
    // See `pan_by_pixels` for why `dx_px`/`dy_px` are exempted from the similar-names lint.
    #[allow(clippy::similar_names)]
    pub fn drag_to(&mut self, dx_px: f64, dy_px: f64, dt_s: f64) {
        self.pan_by_pixels(dx_px, dy_px);

        if dt_s > 0.0 {
            // Weight of the new sample in the moving average.
            const SAMPLE_WEIGHT: f64 = 0.3;
            let sample_x = -dx_px / dt_s * self.meters_per_pixel;
            let sample_y = dy_px / dt_s * self.meters_per_pixel;

            self.velocity_m_per_s = (
                self.velocity_m_per_s
                    .0
                    .mul_add(1.0 - SAMPLE_WEIGHT, sample_x * SAMPLE_WEIGHT),
                self.velocity_m_per_s
                    .1
                    .mul_add(1.0 - SAMPLE_WEIGHT, sample_y * SAMPLE_WEIGHT),
            );
        }
    }

    /// Ends a drag. The velocity already accumulated by [`Camera::drag_to`] becomes the
    /// inertia-coast velocity that [`Camera::update`] applies from here on.
    pub fn end_drag(&mut self) {
        self.is_dragging = false;
    }

    /// Mouse-wheel zoom, anchored on the cursor: positive `notches` (scroll up/away) zooms in,
    /// negative zooms out. The zoom itself is applied by easing in [`Camera::update`]; this
    /// only sets the target and (re-)anchors the cursor position.
    // `cursor_x_px`/`cursor_y_px` are a cartesian pair; see `pan_by_pixels`.
    #[allow(clippy::similar_names)]
    pub fn zoom_by_notches(&mut self, notches: f64, cursor_x_px: f64, cursor_y_px: f64) {
        let clip_x = 2.0 * cursor_x_px / self.width_px - 1.0;
        let clip_y = 1.0 - 2.0 * cursor_y_px / self.height_px;

        let max_mpp = max_meters_per_pixel(self.width_px, self.height_px);
        self.target_meters_per_pixel = (self.target_meters_per_pixel
            * ZOOM_STEP_FACTOR.powf(-notches))
        .clamp(MIN_METERS_PER_PIXEL, max_mpp);

        // Unconditionally overwritten: repeated wheel events before the previous ease finishes
        // re-anchor to the latest cursor position, which is the expected behavior.
        self.zoom_anchor_clip = Some((clip_x, clip_y));
    }

    /// Advances the camera by one frame: eases the zoom toward its target (re-centering to
    /// keep any active cursor anchor fixed in world space), then — if not currently
    /// dragging — coasts and decays the pan-inertia velocity.
    pub fn update(&mut self, dt_s: f64) {
        if self.meters_per_pixel != self.target_meters_per_pixel {
            let mpp_before = self.meters_per_pixel;
            self.meters_per_pixel += (self.target_meters_per_pixel - mpp_before)
                * (1.0 - (-dt_s / ZOOM_EASE_TAU_S).exp());

            // Floating point should not leave this converging forever.
            if (self.meters_per_pixel - self.target_meters_per_pixel).abs()
                <= 1e-9 * self.target_meters_per_pixel.max(1.0)
            {
                self.meters_per_pixel = self.target_meters_per_pixel;
            }

            if let Some((clip_x, clip_y)) = self.zoom_anchor_clip {
                let mpp_delta = mpp_before - self.meters_per_pixel;
                self.center_m.x_m += clip_x * (self.width_px / 2.0) * mpp_delta;
                self.center_m.y_m += clip_y * (self.height_px / 2.0) * mpp_delta;
            }

            if self.meters_per_pixel == self.target_meters_per_pixel {
                self.zoom_anchor_clip = None;
            }
        }

        if !self.is_dragging {
            self.center_m.x_m += self.velocity_m_per_s.0 * dt_s;
            self.center_m.y_m += self.velocity_m_per_s.1 * dt_s;

            let decay = (-dt_s / PAN_FRICTION_TAU_S).exp();
            self.velocity_m_per_s = (
                self.velocity_m_per_s.0 * decay,
                self.velocity_m_per_s.1 * decay,
            );

            if self.velocity_m_per_s.0.hypot(self.velocity_m_per_s.1) < STOP_SPEED_M_PER_S {
                self.velocity_m_per_s = (0.0, 0.0);
            }
        }
    }

    /// World position shown at the viewport center.
    pub fn center_m(&self) -> MercatorXy {
        self.center_m
    }

    /// Current zoom level: world metres spanned by one physical pixel.
    pub fn meters_per_pixel(&self) -> f64 {
        self.meters_per_pixel
    }

    /// Viewport width, in physical pixels.
    pub fn width_px(&self) -> f64 {
        self.width_px
    }

    /// Viewport height, in physical pixels.
    pub fn height_px(&self) -> f64 {
        self.height_px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Metres. Generous next to the world's ~4e7 m extent, tight next to visible drift.
    const EPS_M: f64 = 1e-6;

    #[track_caller]
    fn assert_close(actual: f64, expected: f64, eps: f64) {
        assert!(
            (actual - expected).abs() <= eps,
            "expected {expected}, got {actual} (difference {}, tolerance {eps})",
            (actual - expected).abs()
        );
    }

    // --- new / resize: the letterbox "contain" fit -----------------------------

    #[test]
    fn new_on_a_wide_window_lets_the_narrower_axis_set_the_contain_fit_scale() {
        let cam = Camera::new(2000, 1000);
        let expected = 2.0 * WEB_MERCATOR_EXTENT_M / 1000.0;
        assert_close(cam.meters_per_pixel(), expected, EPS_M);
        assert_close(cam.target_meters_per_pixel, expected, EPS_M);
    }

    #[test]
    fn new_on_a_tall_window_lets_the_narrower_axis_set_the_contain_fit_scale() {
        let cam = Camera::new(800, 1600);
        let expected = 2.0 * WEB_MERCATOR_EXTENT_M / 800.0;
        assert_close(cam.meters_per_pixel(), expected, EPS_M);
        assert_close(cam.target_meters_per_pixel, expected, EPS_M);
    }

    #[test]
    fn new_starts_centered_on_the_origin_with_no_velocity_or_drag() {
        let cam = Camera::new(1000, 800);
        assert_close(cam.center_m().x_m, 0.0, EPS_M);
        assert_close(cam.center_m().y_m, 0.0, EPS_M);
        assert_eq!(cam.velocity_m_per_s, (0.0, 0.0));
        assert!(!cam.is_dragging);
        assert_eq!(cam.zoom_anchor_clip, None);
    }

    #[test]
    fn new_and_resize_clamp_zero_dimensions_to_avoid_division_by_zero() {
        let mut cam = Camera::new(0, 0);
        assert!(cam.meters_per_pixel().is_finite());
        assert_close(cam.width_px(), 1.0, f64::EPSILON);
        assert_close(cam.height_px(), 1.0, f64::EPSILON);

        cam.resize(0, 0);
        assert!(cam.meters_per_pixel().is_finite());
        assert_close(cam.width_px(), 1.0, f64::EPSILON);
        assert_close(cam.height_px(), 1.0, f64::EPSILON);
    }

    #[test]
    fn resize_reclamps_meters_per_pixel_down_when_the_new_ceiling_is_lower() {
        // The ceiling is `2 * EXTENT / min(width, height)`: a fixed-size world spread over
        // *more* pixels needs *fewer* metres per pixel, so growing the window (500x500 ->
        // 2000x2000) lowers the ceiling — this is the direction that actually forces a
        // downward re-clamp (shrinking a window only ever raises the ceiling).
        let mut cam = Camera::new(500, 500);
        let old_max = max_meters_per_pixel(500.0, 500.0);
        assert_close(cam.meters_per_pixel(), old_max, EPS_M);
        assert_close(cam.target_meters_per_pixel, old_max, EPS_M);

        cam.resize(2000, 2000);
        let new_max = max_meters_per_pixel(2000.0, 2000.0);
        assert!(new_max < old_max);
        assert_close(cam.meters_per_pixel(), new_max, EPS_M);
        assert_close(cam.target_meters_per_pixel, new_max, EPS_M);
    }

    // --- pan_by_pixels: sign convention -----------------------------------------

    #[test]
    fn dragging_right_moves_center_west_ie_x_decreases() {
        let mut cam = Camera::new(1000, 800);
        let mpp = cam.meters_per_pixel();
        cam.pan_by_pixels(10.0, 0.0);
        assert_close(cam.center_m().x_m, -10.0 * mpp, EPS_M);
        assert_close(cam.center_m().y_m, 0.0, EPS_M);
    }

    #[test]
    fn dragging_down_moves_center_north_ie_y_increases() {
        let mut cam = Camera::new(1000, 800);
        let mpp = cam.meters_per_pixel();
        cam.pan_by_pixels(0.0, 10.0);
        assert_close(cam.center_m().y_m, 10.0 * mpp, EPS_M);
        assert_close(cam.center_m().x_m, 0.0, EPS_M);
    }

    #[test]
    fn pan_by_pixels_matches_the_derived_formula_for_several_deltas() {
        let mut cam = Camera::new(1000, 800);
        let mpp = cam.meters_per_pixel();
        for (dx, dy) in [(3.0, -7.0), (-50.0, 25.0), (0.0, 0.0), (100.0, 100.0)] {
            let before = cam.center_m();
            cam.pan_by_pixels(dx, dy);
            let after = cam.center_m();
            assert_close(after.x_m - before.x_m, -dx * mpp, EPS_M);
            assert_close(after.y_m - before.y_m, dy * mpp, EPS_M);
        }
    }

    // --- zoom_by_notches + update: cursor anchoring -----------------------------

    // Cartesian x/y pairs throughout; see `pan_by_pixels` for why the lint is exempted.
    #[allow(clippy::similar_names)]
    #[test]
    fn zoom_keeps_the_cursor_anchored_world_point_fixed_through_convergence() {
        let mut cam = Camera::new(1000, 800);
        let cursor_x = 700.0;
        let cursor_y = 200.0;

        let clip_x = 2.0 * cursor_x / cam.width_px() - 1.0;
        let clip_y = 1.0 - 2.0 * cursor_y / cam.height_px();
        let half_w = cam.width_px() / 2.0;
        let half_h = cam.height_px() / 2.0;
        let world_x_before = cam.center_m().x_m + clip_x * half_w * cam.meters_per_pixel();
        let world_y_before = cam.center_m().y_m + clip_y * half_h * cam.meters_per_pixel();

        cam.zoom_by_notches(3.0, cursor_x, cursor_y);
        for _ in 0..500 {
            cam.update(1.0 / 60.0);
        }
        assert_eq!(
            cam.zoom_anchor_clip, None,
            "anchor should clear once converged"
        );

        let clip_x_after =
            (world_x_before - cam.center_m().x_m) / (half_w * cam.meters_per_pixel());
        let clip_y_after =
            (world_y_before - cam.center_m().y_m) / (half_h * cam.meters_per_pixel());

        assert_close(clip_x_after, clip_x, 1e-6);
        assert_close(clip_y_after, clip_y, 1e-6);
    }

    #[test]
    fn zooming_in_decreases_meters_per_pixel_after_convergence() {
        let mut cam = Camera::new(1000, 800);
        let initial = cam.meters_per_pixel();

        cam.zoom_by_notches(5.0, 500.0, 400.0);
        for _ in 0..200 {
            cam.update(1.0 / 60.0);
        }

        assert!(cam.meters_per_pixel() < initial);
    }

    #[test]
    fn zooming_out_increases_meters_per_pixel_clamped_at_the_letterbox_ceiling() {
        let mut cam = Camera::new(1000, 800);

        cam.zoom_by_notches(-100.0, 500.0, 400.0);
        for _ in 0..200 {
            cam.update(1.0 / 60.0);
        }

        let max_mpp = max_meters_per_pixel(1000.0, 800.0);
        assert_close(cam.meters_per_pixel(), max_mpp, EPS_M);
    }

    #[test]
    fn meters_per_pixel_never_drops_below_the_minimum_even_after_many_zoom_in_notches() {
        let mut cam = Camera::new(1000, 800);

        cam.zoom_by_notches(1000.0, 500.0, 400.0);
        for _ in 0..500 {
            cam.update(1.0 / 60.0);
        }

        assert_close(cam.meters_per_pixel(), MIN_METERS_PER_PIXEL, EPS_M);
    }

    // --- drag / inertia ----------------------------------------------------------

    #[test]
    fn begin_drag_cancels_any_existing_inertia_velocity() {
        let mut cam = Camera::new(1000, 800);
        cam.velocity_m_per_s = (123.0, -45.0);
        cam.begin_drag();
        assert_eq!(cam.velocity_m_per_s, (0.0, 0.0));
    }

    #[test]
    fn drag_and_release_leaves_a_decaying_velocity_that_never_reverses_or_overshoots() {
        let mut cam = Camera::new(1000, 800);
        cam.begin_drag();
        for _ in 0..30 {
            cam.drag_to(5.0, 0.0, 1.0 / 60.0);
        }
        cam.end_drag();

        // Dragging right (+dx) accumulates a negative (westward) x velocity, zero y.
        assert!(cam.velocity_m_per_s.0 < 0.0);
        assert_eq!(cam.velocity_m_per_s.1, 0.0);

        let mut previous_speed = cam.velocity_m_per_s.0.abs();
        let mut snapped_to_zero = false;
        for _ in 0..2_000 {
            cam.update(1.0 / 60.0);
            let speed = cam.velocity_m_per_s.0.abs();
            assert!(
                speed <= previous_speed + 1e-9,
                "velocity magnitude increased: {speed} > {previous_speed}"
            );
            assert!(
                cam.velocity_m_per_s.0 <= 0.0,
                "velocity reversed sign: {}",
                cam.velocity_m_per_s.0
            );
            previous_speed = speed;
            if cam.velocity_m_per_s == (0.0, 0.0) {
                snapped_to_zero = true;
                break;
            }
        }
        assert!(snapped_to_zero, "velocity never snapped to exactly zero");
    }
}
