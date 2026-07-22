//! Orthographic globe camera state: rotate (drag) and cursor-anchored zoom, the L0/global-tier
//! analogue of [`crate::camera::Camera`]'s regional Web Mercator camera.
//!
//! Pure state and arithmetic only — no wgpu, no matrices, no winit; see `crate::camera`'s module
//! doc for the same scoping rationale. The projection itself lives in [`crate::geo`]
//! ([`orthographic_forward`]/[`orthographic_inverse`]); this module only tracks the sub-observer
//! point (the globe position facing the viewer, [`GlobeCamera::center`]) and the current zoom
//! (globe radius in physical pixels, [`GlobeCamera::radius_px`]) and how they are moving.
//!
//! M4 item 4.2: no renderer wiring yet (that lands in M4 item 4.3, which also owns the animated
//! globe↔Mercator transition).
//!
//! Rotation and zoom-anchor correction both linearize pixels-to-radians using the current
//! `radius_px` as though the globe were locally flat near the screen center — exact only for
//! content close to center, approximate elsewhere (a real sphere foreshortens away from center).
//! This matches common "drag to spin" globe UIs rather than implementing true trackball rotation,
//! and keeps this pure-math item's scope proportionate to what 4.3's renderer wiring needs.

use crate::geo::{
    LatLon, UnitDiskXy, normalize_lon_deg, orthographic_forward, orthographic_inverse,
};

/// 10% zoom change per wheel notch — same feel as [`crate::camera::Camera::zoom_by_notches`].
const ZOOM_STEP_FACTOR: f64 = 1.1;

/// Exponential time constant for the eased approach to `target_radius_px`.
const ZOOM_EASE_TAU_S: f64 = 0.12;

/// A floor guarding against a degenerate/non-positive globe radius; there is no meaningful lower
/// bound otherwise (unlike `Camera`'s `MIN_METERS_PER_PIXEL`, nothing yet constrains how far this
/// camera should zoom in, since L0 only needs the wide "whole globe visible" framing — a tighter
/// bound can be added once 4.3 wires this into the tier transition).
const MIN_RADIUS_PX: f64 = 1.0;

/// The "whole globe visible" framing: the globe's diameter fills the shorter viewport dimension.
fn default_radius_px(width_px: f64, height_px: f64) -> f64 {
    width_px.min(height_px) / 2.0
}

/// The world point and cursor position a wheel-zoom should keep fixed on screen while
/// `radius_px` eases toward `target_radius_px`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ZoomAnchor {
    world: LatLon,
    cursor_x_px: f64,
    cursor_y_px: f64,
}

/// One window's orthographic globe camera: rotate (drag) and zoom (mouse wheel, cursor-
/// anchored, approximately).
///
/// All screen/pixel arguments to this type's methods are physical pixels, origin top-left, y
/// increasing downward — the same convention [`crate::camera::Camera`] uses.
#[derive(Debug, Clone, Copy)]
pub struct GlobeCamera {
    /// The sub-observer point: the globe position directly facing the viewer, shown at the
    /// viewport center.
    center: LatLon,
    /// Current zoom level: the globe's radius, in physical pixels.
    radius_px: f64,
    /// What `radius_px` is easing toward; equal to `radius_px` when not zooming.
    target_radius_px: f64,
    /// Set by [`GlobeCamera::zoom_by_notches`] when the cursor was over the visible globe;
    /// cleared once `radius_px` converges.
    zoom_anchor: Option<ZoomAnchor>,
    width_px: f64,
    height_px: f64,
}

impl GlobeCamera {
    /// A new camera centered on the equator/prime meridian, framed so the whole globe fills the
    /// shorter viewport dimension (see [`default_radius_px`]).
    ///
    /// `width_px`/`height_px` are clamped to at least 1 before use, matching
    /// [`crate::camera::Camera::new`]'s zero-window guard.
    pub fn new(width_px: u32, height_px: u32) -> Self {
        let width_px = f64::from(width_px.max(1));
        let height_px = f64::from(height_px.max(1));
        let radius_px = default_radius_px(width_px, height_px);

        Self {
            center: LatLon::new(0.0, 0.0),
            radius_px,
            target_radius_px: radius_px,
            zoom_anchor: None,
            width_px,
            height_px,
        }
    }

    /// Updates the viewport size. Unlike [`crate::camera::Camera::resize`] this never re-clamps
    /// `radius_px`: there is no zoom-out ceiling yet (see [`MIN_RADIUS_PX`]'s doc).
    pub fn resize(&mut self, width_px: u32, height_px: u32) {
        self.width_px = f64::from(width_px.max(1));
        self.height_px = f64::from(height_px.max(1));
    }

    /// Immediate rotation of the sub-observer point, no easing — the globe analogue of
    /// [`crate::camera::Camera::pan_by_pixels`].
    ///
    /// The sign convention matches `pan_by_pixels`: dragging right/down reveals globe content
    /// from the west/north, i.e. the sub-observer point moves west/north under the cursor.
    // See `Camera::pan_by_pixels` for why `dx_px`/`dy_px` are exempted from the similar-names
    // lint.
    #[allow(clippy::similar_names)]
    pub fn rotate_by_pixels(&mut self, dx_px: f64, dy_px: f64) {
        let delta_lambda_deg = (dx_px / self.radius_px).to_degrees();
        let delta_phi_deg = (dy_px / self.radius_px).to_degrees();

        self.center = LatLon::new(
            (self.center.lat_deg + delta_phi_deg).clamp(-90.0, 90.0),
            normalize_lon_deg(self.center.lon_deg - delta_lambda_deg),
        );
    }

    /// Mouse-wheel zoom, anchored on the cursor when it falls over the visible globe: positive
    /// `notches` (scroll up/away) zooms in, negative zooms out. The zoom itself is applied by
    /// easing in [`GlobeCamera::update`]; this only sets the target and (re-)anchors the cursor.
    ///
    /// A cursor over empty space beyond the globe's edge has no world point to anchor to, so the
    /// zoom just scales around the current center in that case — the same graceful fallback
    /// [`crate::geo::orthographic_inverse`] already signals via `None`.
    // See `Camera::pan_by_pixels` for why `cursor_x_px`/`cursor_y_px` are exempted.
    #[allow(clippy::similar_names)]
    pub fn zoom_by_notches(&mut self, notches: f64, cursor_x_px: f64, cursor_y_px: f64) {
        self.target_radius_px =
            (self.target_radius_px * ZOOM_STEP_FACTOR.powf(notches)).max(MIN_RADIUS_PX);

        let disk = UnitDiskXy::new(
            (cursor_x_px - self.width_px / 2.0) / self.radius_px,
            -(cursor_y_px - self.height_px / 2.0) / self.radius_px,
        );
        // Unconditionally overwritten: repeated wheel events before the previous ease finishes
        // re-anchor to the latest cursor position, same as `Camera::zoom_by_notches`.
        self.zoom_anchor = orthographic_inverse(self.center, disk).map(|world| ZoomAnchor {
            world,
            cursor_x_px,
            cursor_y_px,
        });
    }

    /// Advances the camera by one frame: eases `radius_px` toward its target, nudging `center`
    /// each step so any active zoom anchor's world point drifts back toward its cursor position
    /// (see [`GlobeCamera::correct_toward_anchor`]).
    pub fn update(&mut self, dt_s: f64) {
        if self.radius_px == self.target_radius_px {
            return;
        }

        self.radius_px +=
            (self.target_radius_px - self.radius_px) * (1.0 - (-dt_s / ZOOM_EASE_TAU_S).exp());

        // Floating point should not leave this converging forever.
        if (self.radius_px - self.target_radius_px).abs() <= 1e-9 * self.target_radius_px.max(1.0) {
            self.radius_px = self.target_radius_px;
        }

        if let Some(anchor) = self.zoom_anchor {
            self.correct_toward_anchor(anchor);
        }

        if self.radius_px == self.target_radius_px {
            self.zoom_anchor = None;
        }
    }

    /// One first-order correction step nudging `center` so `anchor.world` lands back under
    /// `anchor`'s cursor position at the current `radius_px`.
    ///
    /// Derived from the small-angle partials of [`crate::geo::orthographic_forward`] near
    /// `center`: a longitude change of `d` shifts a near-center point's disk `x` by
    /// `-cos(center.lat) * d`, and a latitude change of `d` shifts disk `y` by `-d`. Solving
    /// each for the `d` that closes this frame's disk-space error gives an exact correction in
    /// the linear regime (small drags close to center) and converges over the ease's remaining
    /// frames otherwise — see the module doc for why this stays a linear approximation rather
    /// than an exact spherical-rotation solve.
    fn correct_toward_anchor(&mut self, anchor: ZoomAnchor) {
        let Some(actual) = orthographic_forward(self.center, anchor.world) else {
            // The anchor point rotated out of view (e.g. a very large drag mid-ease) — nothing
            // sane to correct toward.
            return;
        };
        let desired_x = (anchor.cursor_x_px - self.width_px / 2.0) / self.radius_px;
        let desired_y = -(anchor.cursor_y_px - self.height_px / 2.0) / self.radius_px;
        let error_x = desired_x - actual.x;
        let error_y = desired_y - actual.y;

        // Guards the division below near the poles, where cos(center.lat) approaches zero and
        // longitude becomes a degenerate coordinate anyway.
        let phi0_cos = self.center.lat_deg.to_radians().cos().abs().max(0.01);

        self.center = LatLon::new(
            (self.center.lat_deg - error_y.to_degrees()).clamp(-90.0, 90.0),
            normalize_lon_deg(self.center.lon_deg - (error_x / phi0_cos).to_degrees()),
        );
    }

    /// The sub-observer point: the globe position directly facing the viewer.
    pub fn center(&self) -> LatLon {
        self.center
    }

    /// Current zoom level: the globe's radius, in physical pixels.
    pub fn radius_px(&self) -> f64 {
        self.radius_px
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

    #[track_caller]
    fn assert_close(actual: f64, expected: f64, eps: f64) {
        assert!(
            (actual - expected).abs() <= eps,
            "expected {expected}, got {actual} (difference {}, tolerance {eps})",
            (actual - expected).abs()
        );
    }

    // --- new / resize: the "whole globe visible" framing ------------------------

    #[test]
    fn new_on_a_wide_window_lets_the_narrower_axis_set_the_radius() {
        let cam = GlobeCamera::new(2000, 1000);
        assert_close(cam.radius_px(), 500.0, 1e-9);
    }

    #[test]
    fn new_on_a_tall_window_lets_the_narrower_axis_set_the_radius() {
        let cam = GlobeCamera::new(800, 1600);
        assert_close(cam.radius_px(), 400.0, 1e-9);
    }

    #[test]
    fn new_starts_centered_on_the_equator_and_prime_meridian() {
        let cam = GlobeCamera::new(1000, 800);
        assert_close(cam.center().lat_deg, 0.0, 1e-9);
        assert_close(cam.center().lon_deg, 0.0, 1e-9);
    }

    #[test]
    fn new_and_resize_clamp_zero_dimensions_to_avoid_division_by_zero() {
        let mut cam = GlobeCamera::new(0, 0);
        assert!(cam.radius_px().is_finite());
        assert_close(cam.width_px(), 1.0, f64::EPSILON);
        assert_close(cam.height_px(), 1.0, f64::EPSILON);

        cam.resize(0, 0);
        assert!(cam.radius_px().is_finite());
        assert_close(cam.width_px(), 1.0, f64::EPSILON);
        assert_close(cam.height_px(), 1.0, f64::EPSILON);
    }

    // --- rotate_by_pixels: sign conventions and clamping ------------------------

    #[test]
    fn dragging_right_moves_the_center_west_ie_lon_decreases() {
        let mut cam = GlobeCamera::new(1000, 800);
        cam.rotate_by_pixels(10.0, 0.0);
        assert!(cam.center().lon_deg < 0.0);
        assert_close(cam.center().lat_deg, 0.0, 1e-9);
    }

    #[test]
    fn dragging_down_moves_the_center_north_ie_lat_increases() {
        let mut cam = GlobeCamera::new(1000, 800);
        cam.rotate_by_pixels(0.0, 10.0);
        assert!(cam.center().lat_deg > 0.0);
        assert_close(cam.center().lon_deg, 0.0, 1e-9);
    }

    #[test]
    fn rotating_past_a_pole_clamps_latitude_rather_than_overshooting() {
        let mut cam = GlobeCamera::new(1000, 800);
        cam.rotate_by_pixels(0.0, 1.0e9);
        assert_close(cam.center().lat_deg, 90.0, 1e-6);
        assert!(cam.center().lat_deg.is_finite());
    }

    #[test]
    fn rotating_across_the_antimeridian_wraps_rather_than_leaving_the_valid_range() {
        let mut cam = GlobeCamera::new(1000, 800);
        cam.rotate_by_pixels(-1.0e6, 0.0);
        let lon = cam.center().lon_deg;
        assert!((-180.0..180.0).contains(&lon), "lon = {lon}");
        assert!(lon.is_finite());
    }

    // --- zoom_by_notches + update: convergence and clamping ---------------------

    #[test]
    fn zooming_in_increases_radius_after_convergence() {
        let mut cam = GlobeCamera::new(1000, 800);
        let initial = cam.radius_px();

        cam.zoom_by_notches(5.0, 500.0, 400.0);
        for _ in 0..200 {
            cam.update(1.0 / 60.0);
        }

        assert!(cam.radius_px() > initial);
    }

    #[test]
    fn zooming_out_repeatedly_clamps_at_the_minimum_radius() {
        let mut cam = GlobeCamera::new(1000, 800);

        cam.zoom_by_notches(-1000.0, 500.0, 400.0);
        for _ in 0..500 {
            cam.update(1.0 / 60.0);
        }

        assert_close(cam.radius_px(), MIN_RADIUS_PX, 1e-6);
    }

    #[test]
    fn zoom_anchor_clears_once_converged() {
        let mut cam = GlobeCamera::new(1000, 800);
        cam.zoom_by_notches(3.0, 520.0, 380.0);
        for _ in 0..500 {
            cam.update(1.0 / 60.0);
        }
        assert_eq!(cam.zoom_anchor, None);
    }

    #[test]
    fn zooming_with_the_cursor_at_the_globe_center_leaves_the_center_unrotated() {
        // At the exact screen center the desired disk position is always (0, 0), which is also
        // the actual disk position of `cam.center()` at every radius — zero error every frame,
        // so the anchor correction should be a no-op regardless of how far the zoom travels.
        let mut cam = GlobeCamera::new(1000, 800);
        cam.zoom_by_notches(10.0, 500.0, 400.0);
        for _ in 0..500 {
            cam.update(1.0 / 60.0);
        }
        assert_close(cam.center().lat_deg, 0.0, 1e-6);
        assert_close(cam.center().lon_deg, 0.0, 1e-6);
    }

    #[test]
    fn zoom_anchored_near_center_keeps_the_cursor_point_close_to_its_disk_position() {
        // A modest offset from screen center, where the linear approximation the anchor
        // correction relies on (see `correct_toward_anchor`'s doc) is close to exact.
        let mut cam = GlobeCamera::new(1000, 800);
        let cursor_x = 550.0;
        let cursor_y = 420.0;

        let disk_before = UnitDiskXy::new(
            (cursor_x - cam.width_px() / 2.0) / cam.radius_px(),
            -(cursor_y - cam.height_px() / 2.0) / cam.radius_px(),
        );
        let anchor_world = orthographic_inverse(cam.center(), disk_before)
            .expect("cursor within the default full-globe framing sits on the visible globe");

        cam.zoom_by_notches(6.0, cursor_x, cursor_y);
        for _ in 0..500 {
            cam.update(1.0 / 60.0);
        }

        let disk_after = orthographic_forward(cam.center(), anchor_world)
            .expect("a small drag/zoom should not rotate the anchor point out of view");
        let desired_x = (cursor_x - cam.width_px() / 2.0) / cam.radius_px();
        let desired_y = -(cursor_y - cam.height_px() / 2.0) / cam.radius_px();
        assert_close(disk_after.x, desired_x, 1e-3);
        assert_close(disk_after.y, desired_y, 1e-3);
    }

    #[test]
    fn zooming_with_the_cursor_off_the_globe_does_not_panic_and_scales_around_center() {
        let mut cam = GlobeCamera::new(1000, 800);
        // Far outside the default framing's globe disk (radius 400 px at screen center).
        cam.zoom_by_notches(5.0, 990.0, 10.0);
        assert_eq!(cam.zoom_anchor, None);

        for _ in 0..200 {
            cam.update(1.0 / 60.0);
        }
        assert!(cam.radius_px().is_finite());
        assert_close(cam.center().lat_deg, 0.0, 1e-9);
        assert_close(cam.center().lon_deg, 0.0, 1e-9);
    }
}
