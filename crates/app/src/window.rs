//! The application window and its event loop.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use look_above_render::{FrameOutcome, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::frame_stats::FrameStats;

const WINDOW_TITLE: &str = "Look Above";

/// Logical, so the window is the same apparent size at any display scaling.
const INITIAL_SIZE: LogicalSize<u32> = LogicalSize::new(1280, 800);

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
        let renderer = Renderer::new(Arc::clone(&window), size.width, size.height)
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

        self.window = Some(window);
        self.renderer = Some(renderer);
        Ok(())
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        match renderer.render() {
            Ok(FrameOutcome::Presented) => {
                if let Some(summary) = self.stats.record(Instant::now()) {
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
