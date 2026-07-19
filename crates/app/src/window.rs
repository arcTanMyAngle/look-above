//! The application window and its event loop.
//!
//! From M2 item 2.3b this also drives the live ingest pipeline (the same [`Poller`]/
//! [`SessionTable`]/[`Writer`] pieces [`crate::headless`] runs), sourced from the camera's own
//! viewport instead of a fixed bbox and retargeted live as the user pans/zooms — see
//! [`App::start`] for construction and [`App::maybe_retarget`] for the retarget policy.
//!
//! From M2 item 2.4b the merge/interpolation side of that pipeline moves off the render thread
//! entirely, onto [`crate::simulation`]'s worker: ADR-002 keeps *all* simulation, interpolation,
//! and projection on workers, leaving the render thread to only swap the latest
//! [`RenderFeed`](look_above_core::sim::RenderFeed) (through [`crate::double_buffer`]) and draw.
//! Nothing visible is drawn from the feed until 2.5's glyph pipeline; for now the swapped feed's
//! instance count is logged (F3 frame stats), which is 2.4b's verification.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::unbounded;
use look_above_core::camera::Camera;
use look_above_core::contracts::RegionQuery;
use look_above_core::merge::SessionTable;
use look_above_core::sim::{RenderFeed, Simulator};
use look_above_core::types::SourceId;
use look_above_ingest::budget::CreditLedger;
use look_above_ingest::http::HttpClient;
use look_above_ingest::opensky::OpenSkyAuth;
use look_above_ingest::poller::{PRIMARY, Poller, SystemWallClock, WallClock};
use look_above_render::{FrameOutcome, Renderer, camera_view_proj};
use look_above_store::Writer;
use tokio::sync::watch;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::double_buffer::{self, Consumer};
use crate::frame_stats::FrameStats;
use crate::simulation;

const WINDOW_TITLE: &str = "Look Above";

/// Logical, so the window is the same apparent size at any display scaling.
const INITIAL_SIZE: LogicalSize<u32> = LogicalSize::new(1280, 800);

/// A touchpad/precision-scroll `PixelDelta` event has no natural "notch" of its own the way a
/// mouse wheel's `LineDelta` does — this is how many pixels of `PixelDelta` count as one notch
/// of [`Camera::zoom_by_notches`]. A judgement call, not a platform constant.
const SCROLL_PIXELS_PER_NOTCH: f64 = 100.0;

/// How long the camera must sit still (no real pan/zoom change frame-to-frame) before its
/// viewport is sent to the poller as a new region — see [`App::maybe_retarget`]. Debounced so a
/// continuous drag or zoom-ease does not retarget (and re-fetch) on every single frame.
const CAMERA_SETTLE_DEBOUNCE: Duration = Duration::from_secs(2);

/// Open the window and pump events until the user closes it.
pub fn run(config: &Config) -> Result<()> {
    let event_loop = EventLoop::new().context("create the event loop")?;

    // Poll rather than Wait: from M2 the map animates between polls (dead reckoning), so
    // frames are driven by the clock, not by input.
    event_loop.set_control_flow(ControlFlow::Poll);

    // Kept alive for the whole function: the ingest pipeline `App::start` spawns onto this
    // runtime runs alongside winit's own (blocking) event loop below, not instead of it.
    let runtime = tokio::runtime::Runtime::new().context("start the window mode async runtime")?;

    let mut app = App::new(config.clone(), runtime.handle().clone());
    event_loop.run_app(&mut app).context("run the event loop")?;

    // A callback cannot return an error, so it parks one here and stops the loop.
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[derive(Debug)]
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

    /// Needed in [`App::start`] to open the same `store::Writer` (same `db_path`) and build the
    /// same `HttpClient`/`OpenSkyAuth` headless mode does.
    config: Config,
    /// A handle onto the runtime `window::run` built and is keeping alive, so the poller can be
    /// spawned from inside a winit callback (`App::start`), which is not itself async.
    runtime_handle: tokio::runtime::Handle,
    /// The live handle used to retarget the running poller's region (see
    /// [`App::maybe_retarget`]). Must stay alive for as long as the poller should remain
    /// retargetable — every `Sender` dropping falls the poller back to a fixed cadence forever
    /// (see `ingest::poller`'s module doc).
    retarget_tx: Option<watch::Sender<RegionQuery>>,
    /// When the camera's state (`center_m`/`meters_per_pixel`) was last observed to actually
    /// change frame-to-frame. `None` until the camera has moved for the first time — see
    /// [`App::maybe_retarget`].
    last_camera_change_instant: Option<Instant>,
    /// The region most recently sent to the poller (or, before the camera has ever moved, the
    /// initial region it was constructed with) — so a settled camera is not resent every frame.
    last_sent_region: RegionQuery,

    /// The render-thread end of the double buffer the simulation worker publishes into. Swapped
    /// once at the start of every frame — nothing here computes the feed (ADR-002). `None` until
    /// [`App::start`] has spawned the worker.
    feed_consumer: Option<Consumer<RenderFeed>>,
    /// The most recent feed taken from `feed_consumer` — the "front" buffer, kept between frames
    /// so a frame in which the worker has published nothing new still draws the last picture
    /// instead of blanking. Its `aircraft.len()` is the instance count 2.4b logs (2.5 draws it).
    current_feed: RenderFeed,
    /// Set on exit to stop the simulation worker; it checks this once per iteration.
    sim_shutdown: Option<Arc<AtomicBool>>,
    /// The simulation worker's join handle, so [`App::exiting`] waits for its final DB writes
    /// before the store is torn down.
    sim_handle: Option<JoinHandle<()>>,
}

impl App {
    /// `Config` and the runtime handle have no meaningful `Default`, so this replaces what
    /// `#[derive(Default)]` used to give `App` — everything else starts the same way it did.
    fn new(config: Config, runtime_handle: tokio::runtime::Handle) -> Self {
        Self {
            window: None,
            renderer: None,
            camera: None,
            last_cursor_pos: None,
            last_drag_instant: None,
            last_frame_instant: None,
            stats: FrameStats::default(),
            stats_visible: false,
            error: None,
            config,
            runtime_handle,
            retarget_tx: None,
            last_camera_change_instant: None,
            last_sent_region: RegionQuery::default(),
            feed_consumer: None,
            current_feed: RenderFeed::default(),
            sim_shutdown: None,
            sim_handle: None,
        }
    }

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

        // The same three ingest pieces `headless::run` builds — see that module's doc comment
        // for why each is required, not just the poller. A failure here is fatal, the same way
        // a renderer-init failure above is: a broken DB or client cannot degrade gracefully into
        // a render-only mode without silently losing the ledger-restore guarantee (see this
        // item's own notes on why that guarantee matters).
        let writer = Writer::open(&self.config.storage.db_path).with_context(|| {
            format!(
                "open the store at {}",
                self.config.storage.db_path.display()
            )
        })?;

        let client = HttpClient::new().context("build the shared HTTP client")?;
        let auth =
            OpenSkyAuth::from_optional(client.clone(), self.config.sources.opensky.credentials());

        // The initial region comes from the camera, not a fixed bbox — this is the seam that
        // makes window mode's ingest camera-driven rather than headless's fixed one.
        let initial_bbox = camera.viewport_bbox();
        let initial_query = RegionQuery::region(initial_bbox);
        let (retarget_tx, retarget_rx) = watch::channel(initial_query);

        let clock: Arc<dyn WallClock> = Arc::new(SystemWallClock);
        let (sender, receiver) = unbounded();
        let mut poller =
            Poller::with_default_chain(client, auth, retarget_rx, sender, Arc::clone(&clock));

        // Item 1.7's ledger seam, closed here exactly as it is in headless mode: seed the
        // primary's ledger from what was already spent today (privacy rule 1.3's daily cap is
        // a real-world quota, not a per-process one — see this item's own notes).
        match writer.source_status(SourceId::OpenSky) {
            Ok(Some(status)) => {
                let now = clock.now();
                poller.restore_ledger(
                    PRIMARY,
                    CreditLedger::restored(status.credits_used_today, now),
                );
                tracing::info!(
                    credits_used_today = status.credits_used_today,
                    "restored the OpenSky credit ledger from source_status"
                );
            }
            Ok(None) => {
                tracing::info!("no persisted OpenSky source_status; starting the ledger fresh");
            }
            Err(error) => tracing::warn!(
                %error,
                "could not read OpenSky's source_status; starting the ledger fresh"
            ),
        }

        tracing::info!(
            bbox = ?initial_bbox,
            opensky_credentials = if self.config.sources.opensky.is_configured() {
                "configured"
            } else {
                "absent"
            },
            "window mode: starting the poll loop"
        );
        self.runtime_handle.spawn(poller.run());

        // Hand the merge/interpolate/persist side to a worker thread (ADR-002): it owns the
        // `SessionTable`, the `Writer`, and the batch receiver, drains poll cycles, runs
        // `core::sim` at render cadence, and publishes each frame's feed into the double buffer
        // this thread swaps at frame start.
        let shutdown = Arc::new(AtomicBool::new(false));
        let (producer, consumer) = double_buffer::channel();
        let sim_handle = simulation::spawn(
            Simulator::new(),
            SessionTable::new(),
            writer,
            receiver,
            producer,
            Arc::clone(&shutdown),
        )
        .context("spawn the simulation worker")?;

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.camera = Some(camera);
        self.retarget_tx = Some(retarget_tx);
        self.last_sent_region = initial_query;
        self.feed_consumer = Some(consumer);
        self.sim_shutdown = Some(shutdown);
        self.sim_handle = Some(sim_handle);
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

        let before = (camera.center_m(), camera.meters_per_pixel());
        camera.update(dt_s);
        let changed = before != (camera.center_m(), camera.meters_per_pixel());
        renderer.set_view_proj(camera_view_proj(camera));

        Self::maybe_retarget(
            camera,
            now,
            changed,
            &mut self.last_camera_change_instant,
            &mut self.last_sent_region,
            self.retarget_tx.as_ref(),
        );

        // Swap in the latest feed the simulation worker has published (ADR-002's atomic
        // frame-start swap). `None` means nothing new since last frame, so the held feed stays —
        // the picture never blanks between publishes.
        if let Some(consumer) = self.feed_consumer.as_ref()
            && let Some(feed) = consumer.take_latest()
        {
            self.current_feed = feed;
        }

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
                            // The live feed's drawable count (2.4b). Still nothing is *drawn*
                            // from it — the glyph pipeline is 2.5 — so this logged number is the
                            // item's verification that the feed reaches the render thread.
                            instances = self.current_feed.aircraft.len(),
                            "frame stats"
                        );
                    } else {
                        tracing::debug!(
                            frames = summary.frames,
                            fps = format!("{:.1}", summary.fps()),
                            mean_ms = format!("{:.2}", summary.mean.as_secs_f64() * 1e3),
                            worst_ms = format!("{:.2}", summary.worst.as_secs_f64() * 1e3),
                            instances = self.current_feed.aircraft.len(),
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

    /// Retargets the running poller once the camera has settled on a viewport whose bbox
    /// differs from whichever region was last sent — see [`CAMERA_SETTLE_DEBOUNCE`].
    ///
    /// `changed` is whether the camera's state (`center_m`/`meters_per_pixel`) actually moved
    /// this frame; only a real change (re-)arms `last_change`, so the debounce clock never
    /// starts — and nothing is ever sent — before the user has interacted with the camera for
    /// the first time. A free-standing function so it can be called from [`App::draw`] while
    /// `renderer`/`camera` (borrowed from other `self` fields) are still in scope.
    fn maybe_retarget(
        camera: &Camera,
        now: Instant,
        changed: bool,
        last_change: &mut Option<Instant>,
        last_sent_region: &mut RegionQuery,
        retarget_tx: Option<&watch::Sender<RegionQuery>>,
    ) {
        if changed {
            *last_change = Some(now);
        }
        let Some(changed_at) = *last_change else {
            return;
        };
        if now.saturating_duration_since(changed_at) < CAMERA_SETTLE_DEBOUNCE {
            return;
        }
        let Some(retarget_tx) = retarget_tx else {
            return;
        };

        let query = RegionQuery::region(camera.viewport_bbox());
        if query != *last_sent_region {
            // A closed channel means the poller task itself has ended; there is nothing more
            // this side can do about it, so a failed send only stops retargeting, not the app.
            let _ = retarget_tx.send(query);
            *last_sent_region = query;
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
                    // `maybe_retarget`'s `changed` signal only ever sees `center_m`/
                    // `meters_per_pixel` deltas taken around `camera.update` inside `draw` — a
                    // resize lands here, strictly before the next `draw`, so that comparison
                    // never observes it even though `viewport_bbox` genuinely changes with the
                    // window's aspect ratio. Arming the settle clock directly here is what lets
                    // a resize (with no accompanying pan/zoom) still eventually retarget.
                    self.last_camera_change_instant = Some(Instant::now());
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
        // Stop the simulation worker and wait for it to finish before the store is torn down:
        // it owns the only `Writer` clone in window mode, so joining it flushes the last cycle's
        // DB writes rather than racing them against process teardown. Signal-then-join — the
        // worker checks the flag once per iteration, so this returns within ~one frame.
        if let Some(shutdown) = &self.sim_shutdown {
            shutdown.store(true, Ordering::Relaxed);
        }
        if let Some(handle) = self.sim_handle.take() {
            // A panic inside the worker has already unwound and logged on its own thread;
            // nothing here can do better than shut the window down cleanly regardless.
            let _ = handle.join();
        }
        // Drop the renderer before the window: the surface borrows it.
        self.renderer = None;
        self.window = None;
        tracing::info!("window closed");
    }
}
