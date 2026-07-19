//! The application window and its event loop.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use look_above_core::camera::Camera;
use look_above_render::{FrameOutcome, Renderer, camera_view_proj};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::frame_stats::FrameStats;

const WINDOW_TITLE: &str = "Look Above";

/// Logical, so the window is the same apparent size at any display scaling.
const INITIAL_SIZE: LogicalSize<u32> = LogicalSize::new(1280, 800);

/// A touchpad/precision-scroll `PixelDelta` event has no natural "notch" of its own the way a
/// mouse wheel's `LineDelta` does — this is how many pixels of `PixelDelta` count as one notch
/// of [`Camera::zoom_by_notches`]. A judgement call, not a platform constant.
const SCROLL_PIXELS_PER_NOTCH: f64 = 100.0;

/// Open the window and pump events until the user closes it.
pub fn run() -> Result<()> {
    let event_loop = EventLoop::new().context("create the event loop")?;

    // Poll rather than Wait: from M2 the map animates between polls (dead reckoning), so
    // frames are driven by the clock, not by input.
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::default();
    event_loop.run_app(&mut app).context("run the event loop")?;

    // A callback cannot return an error, so it parks one here and stops the loop.
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[derive(Debug, Default)]
struct App {
    /// `Arc` because the renderer's surface holds the window for as long as it draws to it.
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// The regional pan/zoom camera (M2 item 2.3a). Lives here, not in `render`, because it
    /// needs winit input events and `render` must stay winit-free (ADR-002). Built in
    /// [`App::start`] alongside the renderer, at the same physical size.
    camera: Option<Camera>,
    /// The most recent cursor position, in physical pixels. Needed for two things: computing
    /// per-move drag deltas, and anchoring wheel-zoom (a `MouseWheel` event carries no position
    /// of its own in winit — it always accompanies cursor movement, so the last tracked
    /// position is the correct anchor).
    last_cursor_pos: Option<(f64, f64)>,
    /// `Some` for exactly as long as the left button is held: doubles as the drag flag (no
    /// separate `is_dragging` bool needed) and as the "since when" clock `drag_to`'s `dt_s`
    /// is computed against, updated on every drag-tick.
    last_drag_instant: Option<Instant>,
    /// When the previous frame was drawn, for computing `Camera::update`'s `dt_s`. `None` on
    /// the very first frame, which is guarded to a zero `dt_s` rather than a garbage-huge one.
    last_frame_instant: Option<Instant>,
    stats: FrameStats,
    /// Toggled by F3. Widens the once-a-second frame-stats log from `debug` to `info` and
    /// adds p50/p95 — see [`FrameStats`]. No on-screen overlay yet (M2 2.1b, blocked on the
    /// glyph atlas in 2.5/2.7).
    stats_visible: bool,
    error: Option<anyhow::Error>,
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_inner_size(INITIAL_SIZE);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("create the window")?,
        );

        // Physical pixels: the surface is sized in real pixels, not logical ones.
        let size = window.inner_size();
        let mut renderer = Renderer::new(Arc::clone(&window), size.width, size.height)
            .context("initialise the GPU renderer")?;

        let adapter = renderer.adapter_info();
        tracing::info!(
            adapter = adapter.name,
            backend = %adapter.backend,
            format = ?renderer.format(),
            width = size.width,
            height = size.height,
            "window ready"
        );

        // Same `Camera::new(w, h)` call the renderer's own initial buffer contents were seeded
        // from (see `Renderer::new`'s doc comment) — this call is guaranteed to reproduce the
        // same matrix, so there is no visual jump, but it must still run so subsequent
        // input/frame updates have a real `Camera` to drive.
        let camera = Camera::new(size.width, size.height);
        renderer.set_view_proj(camera_view_proj(&camera));

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.camera = Some(camera);
        Ok(())
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(renderer), Some(camera)) = (self.renderer.as_mut(), self.camera.as_mut()) else {
            return;
        };

        let now = Instant::now();
        let dt_s = self
            .last_frame_instant
            .replace(now)
            .map_or(0.0, |previous| {
                now.saturating_duration_since(previous).as_secs_f64()
            });

        camera.update(dt_s);
        renderer.set_view_proj(camera_view_proj(camera));

        match renderer.render() {
            Ok(FrameOutcome::Presented) => {
                if let Some(summary) = self.stats.record(now) {
                    if self.stats_visible {
                        tracing::info!(
                            frames = summary.frames,
                            fps = format!("{:.1}", summary.fps()),
                            mean_ms = format!("{:.2}", summary.mean.as_secs_f64() * 1e3),
                            p50_ms = format!("{:.2}", summary.p50.as_secs_f64() * 1e3),
                            p95_ms = format!("{:.2}", summary.p95.as_secs_f64() * 1e3),
                            worst_ms = format!("{:.2}", summary.worst.as_secs_f64() * 1e3),
                            // Pinned to 0 until M2 2.5 gives the render loop aircraft glyph
                            // instances to count; this only wires the reporting path.
                            instances = 0,
                            "frame stats"
                        );
                    } else {
                        tracing::debug!(
                            frames = summary.frames,
                            fps = format!("{:.1}", summary.fps()),
                            mean_ms = format!("{:.2}", summary.mean.as_secs_f64() * 1e3),
                            worst_ms = format!("{:.2}", summary.worst.as_secs_f64() * 1e3),
                            "frame stats"
                        );
                    }
                }
            }
            // The surface had nothing to give us; the next frame is already queued.
            Ok(FrameOutcome::Skipped) => {}
            Err(error) => self.fail(
                event_loop,
                anyhow::Error::new(error).context("draw a frame"),
            ),
        }
    }

    /// Park a fatal error and stop the loop. The first one wins: later failures are usually
    /// fallout from it.
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        if self.error.is_none() {
            self.error = Some(error);
        }
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // `resumed` can fire more than once (a suspend/resume cycle on mobile); on desktop
        // the window outlives it, so build one only when there is none.
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.start(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("close requested");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                // The camera's zoom ceiling (and therefore possibly its `meters_per_pixel`)
                // can change on resize even though its center doesn't — see `Camera::resize` —
                // so the matrix must be rebuilt here, not left to the next frame.
                if let (Some(renderer), Some(camera)) =
                    (self.renderer.as_mut(), self.camera.as_mut())
                {
                    camera.resize(size.width, size.height);
                    renderer.set_view_proj(camera_view_proj(camera));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x, position.y);

                if let Some(last_instant) = self.last_drag_instant
                    && let Some(camera) = self.camera.as_mut()
                    && let Some((last_x, last_y)) = self.last_cursor_pos
                {
                    let now = Instant::now();
                    let dt_s = now.saturating_duration_since(last_instant).as_secs_f64();
                    camera.drag_to(x - last_x, y - last_y, dt_s);
                    self.last_drag_instant = Some(now);
                }

                self.last_cursor_pos = Some((x, y));
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(camera) = self.camera.as_mut() {
                    match state {
                        ElementState::Pressed => {
                            camera.begin_drag();
                            self.last_drag_instant = Some(Instant::now());
                        }
                        ElementState::Released => {
                            camera.end_drag();
                            self.last_drag_instant = None;
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(camera) = self.camera.as_mut() {
                    let notches = match delta {
                        MouseScrollDelta::LineDelta(_x, y) => f64::from(y),
                        MouseScrollDelta::PixelDelta(position) => {
                            position.y / SCROLL_PIXELS_PER_NOTCH
                        }
                    };
                    // A wheel event carries no cursor position of its own; fall back to the
                    // viewport center if one has never been tracked yet (e.g. the very first
                    // input event is a scroll, before any `CursorMoved`).
                    let (cursor_x, cursor_y) = self
                        .last_cursor_pos
                        .unwrap_or((camera.width_px() / 2.0, camera.height_px() / 2.0));
                    camera.zoom_by_notches(notches, cursor_x, cursor_y);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Only the press edge, not repeat (an OS auto-repeat while F3 is held) or
                // release — otherwise holding the key would flicker the mode.
                if !event.repeat
                    && event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::F3)
                {
                    self.stats_visible = !self.stats_visible;
                    tracing::info!(stats_visible = self.stats_visible, "F3 toggled");
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Under `ControlFlow::Poll` this is the frame clock: ask for the next redraw as
        // soon as the queue is drained.
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Drop the renderer before the window: the surface borrows it.
        self.renderer = None;
        self.window = None;
        tracing::info!("window closed");
    }
}
