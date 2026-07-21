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
use look_above_core::contracts::{Airport, AirportSize, Metar, MetarBadge, RegionQuery, Runway};
use look_above_core::merge::SessionTable;
use look_above_core::sim::{RenderFeed, Simulator};
use look_above_core::types::{Icao24, SourceId};
use look_above_ingest::budget::CreditLedger;
use look_above_ingest::http::HttpClient;
use look_above_ingest::metar::{self, MetarSource, run_metar_poller};
use look_above_ingest::opensky::OpenSkyAuth;
use look_above_ingest::poller::{PRIMARY, Poller, SystemWallClock, WallClock};
use look_above_render::{
    FrameOutcome, InfoCardContent, Renderer, StatsOverlay, camera_view_proj, hit_test,
};
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
use crate::frame_stats::{FrameStats, FrameSummary};
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

/// A left-button press/release pair is a **click** (selection, M2 item 2.8a), not a drag, when
/// the cursor moved no more than this many physical pixels between the two — see
/// [`App::maybe_select`]. A few pixels of unavoidable hand tremor must not read as a drag.
const CLICK_MAX_MOVEMENT_PX: f64 = 5.0;

/// ... and the press/release pair spans no longer than this — see [`App::maybe_select`]. A slow
/// drag that happens to end near its start point (e.g. a hesitant pan) must not be read as a
/// click just because it didn't move far.
const CLICK_MAX_DURATION: Duration = Duration::from_millis(300);

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
    /// Cursor position and time at the most recent left-button press, kept until release — the
    /// baseline [`App::maybe_select`] compares the release against to tell a click from a drag
    /// (M2 item 2.8a). `None` whenever the button isn't currently held.
    press_pos: Option<(f64, f64)>,
    press_instant: Option<Instant>,
    /// When the previous frame was drawn, for computing `Camera::update`'s `dt_s`. `None` on
    /// the very first frame, which is guarded to a zero `dt_s` rather than a garbage-huge one.
    last_frame_instant: Option<Instant>,
    stats: FrameStats,
    /// Toggled by F3. Widens the once-a-second frame-stats log from `debug` to `info`, adds
    /// p50/p95 to it, and (M2 item 2.1b) shows the same numbers as an on-screen HUD block —
    /// [`Renderer::render`]'s `stats` parameter, built from [`App::last_stats_summary`] — reusing
    /// the stroke-font label text pipeline (M2 2.7b) rather than a second text renderer.
    stats_visible: bool,
    /// The most recent frame-stats report ([`FrameStats::record`] only fires once a second),
    /// kept so the F3 HUD shows the latest numbers on every frame in between rather than
    /// blanking or flickering while waiting for the next report — see [`App::draw`].
    last_stats_summary: Option<FrameSummary>,
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

    /// A clone of the same `store::Writer` [`App::start`] hands to the simulation worker (cheap:
    /// `Writer` is just a channel `Sender` — see its own doc comment), kept here so the render/
    /// event thread can query airports/runways directly (M3 item 3.2) without routing through
    /// the worker. `None` until [`App::start`] has opened the store.
    store: Option<Writer>,
    /// The airports [`App::maybe_retarget`] most recently queried for the camera's settled
    /// viewport (M3 item 3.2), at the fixed `AirportSize::Medium` threshold the checklist's own
    /// "large/medium airports" wording asks for (no LOD-tier gating yet — see the M3 plan's own
    /// cross-milestone tension note). Kept (not cleared) across a failed query — see
    /// `maybe_retarget`'s own doc comment.
    current_airports: Vec<Airport>,
    /// The runways [`App::maybe_retarget`] most recently queried alongside
    /// [`App::current_airports`] — same threshold, same tolerant-on-error behavior.
    current_runways: Vec<Runway>,
    /// The live handle used to retarget the running METAR poller's station list (M3 item 3.3) —
    /// mirrors [`App::retarget_tx`]'s shape, but carries the `AirportSize::Large` subset of
    /// [`App::current_airports`]' idents instead of a `RegionQuery`. `None` until
    /// [`App::start`] has spawned the poller.
    metar_retarget_tx: Option<watch::Sender<Vec<String>>>,
    /// The flight-category badges [`App::maybe_retarget`] most recently resolved — the join of
    /// [`App::current_airports`]' large airports against whatever METAR the store has cached
    /// for each (M3 item 3.3). Kept (not cleared) across a failed query, same as
    /// [`App::current_airports`] itself.
    current_metar_badges: Vec<MetarBadge>,

    /// The render-thread end of the double buffer the simulation worker publishes into. Swapped
    /// once at the start of every frame — nothing here computes the feed (ADR-002). `None` until
    /// [`App::start`] has spawned the worker.
    feed_consumer: Option<Consumer<RenderFeed>>,
    /// The most recent feed taken from `feed_consumer` — the "front" buffer, kept between frames
    /// so a frame in which the worker has published nothing new still draws the last picture
    /// instead of blanking. Its `aircraft.len()` is the instance count 2.4b logs (2.5 draws it).
    current_feed: RenderFeed,
    /// The aircraft the user last clicked on, or `None` (M2 item 2.8a) — set by
    /// [`App::maybe_select`], mirrored to the simulation worker via `select_tx` so
    /// `core::sim::Simulator` marks the matching instance's `selected` field. Not yet read by
    /// anything on the render side (no outline/info card until 2.8b); kept here so `app` has a
    /// single source of truth for "what's selected" rather than only living inside the worker.
    selected_icao24: Option<Icao24>,
    /// The live handle used to push a new selection to the simulation worker — mirrors
    /// `retarget_tx`'s shape exactly. `None` until [`App::start`] has spawned the worker.
    select_tx: Option<watch::Sender<Option<Icao24>>>,
    /// Set on exit to stop the simulation worker; it checks this once per iteration.
    sim_shutdown: Option<Arc<AtomicBool>>,
    /// The simulation worker's join handle, so [`App::exiting`] waits for its final DB writes
    /// before the store is torn down.
    sim_handle: Option<JoinHandle<()>>,
}

/// Joins `airports` against `metars` by `station == ident`, keeping only the large airports
/// with a cached observation that resolved to a flight category (M3 item 3.3) — a `None`
/// category has nothing to badge (see `core::contracts::Metar::flight_category`'s own doc
/// comment on when the source reports no computable one).
///
/// A free function, not a method: it is a pure join with no `App` state to read, and
/// [`App::maybe_retarget`] is already a free function itself for the same "called while other
/// `self` fields are borrowed" reason.
fn metar_badges_for(airports: &[Airport], metars: &[Metar]) -> Vec<MetarBadge> {
    airports
        .iter()
        .filter(|airport| airport.size == AirportSize::Large)
        .filter_map(|airport| {
            let metar = metars.iter().find(|metar| metar.station == airport.ident)?;
            let category = metar.flight_category?;
            Some(MetarBadge {
                lat_deg: airport.lat_deg,
                lon_deg: airport.lon_deg,
                category,
            })
        })
        .collect()
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
            press_pos: None,
            press_instant: None,
            last_frame_instant: None,
            stats: FrameStats::default(),
            stats_visible: false,
            last_stats_summary: None,
            error: None,
            config,
            runtime_handle,
            retarget_tx: None,
            last_camera_change_instant: None,
            last_sent_region: RegionQuery::default(),
            store: None,
            current_airports: Vec::new(),
            current_runways: Vec::new(),
            metar_retarget_tx: None,
            current_metar_badges: Vec::new(),
            feed_consumer: None,
            current_feed: RenderFeed::default(),
            selected_icao24: None,
            select_tx: None,
            sim_shutdown: None,
            sim_handle: None,
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one-time startup sequencing: window/renderer/camera, then the store, the \
                  position-poller chain, the metar poller (M3 item 3.3), and the simulation \
                  worker, each a few lines to construct and hand off — splitting it into \
                  sub-functions would mean passing most of these same locals through another \
                  layer of parameters rather than reducing what this method actually does"
    )]
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
        // Cheap (a channel `Sender` clone — see `Writer`'s own doc comment): kept on `App` so
        // this thread can query airports/runways (M3 item 3.2) directly, alongside the other
        // clone the simulation worker below takes ownership of.
        let store_handle = writer.clone();

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
        // Cloned before `client` moves into `with_default_chain` below — the METAR source is a
        // separate, single-source poller (M3 item 3.3, see `ingest::metar`'s module doc
        // comment), not part of the live-position failover chain, but it shares the same
        // allowlist-enforcing `HttpClient`.
        let metar_source = MetarSource::new(client.clone());
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

        // The METAR poller (M3 item 3.3): starts with an empty station list — nothing to poll
        // until the first camera settle populates it (see `App::maybe_retarget`) — at the fixed
        // ≥10-minute cadence `ingest::metar::MIN_POLL_INTERVAL` documents.
        let (metar_retarget_tx, metar_retarget_rx) = watch::channel(Vec::new());
        let (metar_sender, metar_receiver) = unbounded();
        self.runtime_handle.spawn(run_metar_poller(
            metar_source,
            metar_retarget_rx,
            metar_sender,
            Arc::clone(&clock),
            metar::MIN_POLL_INTERVAL,
            metar::IDLE_RECHECK_INTERVAL,
        ));

        // Hand the merge/interpolate/persist side to a worker thread (ADR-002): it owns the
        // `SessionTable`, the `Writer`, and the batch receiver, drains poll cycles, runs
        // `core::sim` at render cadence, and publishes each frame's feed into the double buffer
        // this thread swaps at frame start.
        let shutdown = Arc::new(AtomicBool::new(false));
        let (producer, consumer) = double_buffer::channel();
        let (select_tx, select_rx) = watch::channel(None);
        let sim_handle = simulation::spawn(
            Simulator::new(),
            SessionTable::new(),
            writer,
            receiver,
            metar_receiver,
            producer,
            select_rx,
            Arc::clone(&shutdown),
        )
        .context("spawn the simulation worker")?;

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.camera = Some(camera);
        self.retarget_tx = Some(retarget_tx);
        self.last_sent_region = initial_query;
        self.store = Some(store_handle);
        self.metar_retarget_tx = Some(metar_retarget_tx);
        self.feed_consumer = Some(consumer);
        self.select_tx = Some(select_tx);
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
            self.store.as_ref(),
            &mut self.current_airports,
            &mut self.current_runways,
            self.metar_retarget_tx.as_ref(),
            &mut self.current_metar_badges,
        );

        // Swap in the latest feed the simulation worker has published (ADR-002's atomic
        // frame-start swap). `None` means nothing new since last frame, so the held feed stays —
        // the picture never blanks between publishes.
        if let Some(consumer) = self.feed_consumer.as_ref()
            && let Some(feed) = consumer.take_latest()
        {
            self.current_feed = feed;
        }

        // Built from the *previous* report ([`FrameStats::record`] only fires once a second —
        // see `last_stats_summary`'s own doc comment), so the HUD's numbers this frame lag the
        // live frame time by at most that reporting interval, never by a blank/stale gap.
        let stats_overlay = self
            .stats_visible
            .then(|| {
                self.last_stats_summary.map(|summary| StatsOverlay {
                    fps: summary.fps(),
                    p50_ms: summary.p50.as_secs_f64() * 1e3,
                    p95_ms: summary.p95.as_secs_f64() * 1e3,
                    worst_ms: summary.worst.as_secs_f64() * 1e3,
                })
            })
            .flatten();

        // The selected aircraft's own live instance this frame, if it's still in the feed (M2
        // item 2.8b) — `None` both when nothing is selected and when the selected `icao24` has
        // faded out of the feed since, so the card/outline simply stop drawing rather than
        // showing stale content.
        let info_card = self.selected_icao24.and_then(|icao24| {
            self.current_feed
                .aircraft
                .iter()
                .find(|instance| instance.icao24 == icao24)
                .map(InfoCardContent::from_instance)
        });

        match renderer.render(
            &self.current_feed,
            camera,
            stats_overlay,
            info_card.as_ref(),
            &self.current_airports,
            &self.current_runways,
            &self.current_metar_badges,
        ) {
            Ok(FrameOutcome::Presented) => {
                if let Some(summary) = self.stats.record(now) {
                    self.last_stats_summary = Some(summary);
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
    ///
    /// M3 item 3.2 piggybacks the airport/runway "map lines" query on this same settle-and-send
    /// trigger point (`store`/`current_airports`/`current_runways`): the checklist's own
    /// "reusing existing tessellation approach" scoping reads naturally as "reuse the existing
    /// retarget trigger" too, rather than inventing a second debounce/settle mechanism for the
    /// same camera-settled event. `store` is `None` only before [`App::start`] has opened it (in
    /// practice never true while this runs — see this method's own early returns above); a query
    /// failure logs a `tracing::warn!` and leaves the previous set in place, the same
    /// don't-crash-the-app tolerance this method's own failed `retarget_tx.send` already has.
    ///
    /// M3 item 3.3 piggybacks the same way again: the freshly queried `AirportSize::Large`
    /// subset becomes the METAR poller's next station list (`metar_retarget_tx`), and whatever
    /// the store already has cached for those stations is joined into `current_metar_badges` —
    /// no separate fetch here, this is a read of `metars` as it stands, not a wait for a new
    /// poll cycle to land.
    #[allow(
        clippy::too_many_arguments,
        reason = "this bundles the camera-settle state (last_change/last_sent_region/retarget_tx) \
                  with M3 3.2/3.3's own store-query outputs (current_airports/current_runways/ \
                  current_metar_badges) at the one point they share a trigger; splitting any of \
                  it into its own function would duplicate the settle-debounce check itself, not \
                  reduce it"
    )]
    fn maybe_retarget(
        camera: &Camera,
        now: Instant,
        changed: bool,
        last_change: &mut Option<Instant>,
        last_sent_region: &mut RegionQuery,
        retarget_tx: Option<&watch::Sender<RegionQuery>>,
        store: Option<&Writer>,
        current_airports: &mut Vec<Airport>,
        current_runways: &mut Vec<Runway>,
        metar_retarget_tx: Option<&watch::Sender<Vec<String>>>,
        current_metar_badges: &mut Vec<MetarBadge>,
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

        let bbox = camera.viewport_bbox();
        let query = RegionQuery::region(bbox);
        if query != *last_sent_region {
            // A closed channel means the poller task itself has ended; there is nothing more
            // this side can do about it, so a failed send only stops retargeting, not the app.
            let _ = retarget_tx.send(query);
            *last_sent_region = query;

            // M3 item 3.2: refresh the "map lines" airport/runway data for the newly settled
            // viewport. Fixed `AirportSize::Medium` threshold per the checklist's own "large/
            // medium airports" wording — a hardcoded interpretation, not an LOD-driven one, the
            // same kind of explicit reading call the M3 plan's 3.1 entry already recorded for
            // its own acceptance-line wording.
            if let Some(store) = store {
                match store.airports_in_bbox(bbox, AirportSize::Medium) {
                    Ok(airports) => *current_airports = airports,
                    Err(error) => tracing::warn!(
                        %error,
                        "airports_in_bbox query failed; keeping the previous airport set"
                    ),
                }
                match store.runways_in_bbox(bbox, AirportSize::Medium) {
                    Ok(runways) => *current_runways = runways,
                    Err(error) => tracing::warn!(
                        %error,
                        "runways_in_bbox query failed; keeping the previous runway set"
                    ),
                }

                // M3 item 3.3: retarget the METAR poller at the newly settled viewport's large
                // airports, and read back whatever the store already has cached for them.
                let large_idents: Vec<String> = current_airports
                    .iter()
                    .filter(|airport| airport.size == AirportSize::Large)
                    .map(|airport| airport.ident.clone())
                    .collect();
                if let Some(metar_retarget_tx) = metar_retarget_tx {
                    // Same tolerance as `retarget_tx.send` above: a gone poller task only stops
                    // retargeting, not the app.
                    let _ = metar_retarget_tx.send(large_idents.clone());
                }
                match store.metars_for_stations(large_idents) {
                    Ok(metars) => {
                        *current_metar_badges = metar_badges_for(current_airports, &metars);
                    }
                    Err(error) => tracing::warn!(
                        %error,
                        "metars_for_stations query failed; keeping the previous badge set"
                    ),
                }
            }
        }
    }

    /// If the left-button press/release pair that just ended looks like a click rather than a
    /// drag ([`CLICK_MAX_MOVEMENT_PX`]/[`CLICK_MAX_DURATION`]), hit-tests `release_pos` against
    /// the currently drawn feed and updates the selection (M2 item 2.8a) — a click that hits no
    /// aircraft deselects, the same way a click that hits one selects it. Mirrored to the
    /// simulation worker over `select_tx` so `core::sim::Simulator` marks the right instance on
    /// its next `advance_all`; a closed channel (worker gone) only stops that mirroring, the same
    /// tolerance `maybe_retarget`'s own send has.
    fn maybe_select(&mut self, release_pos: (f64, f64), released_at: Instant) {
        let (Some(press_pos), Some(pressed_at)) = (self.press_pos, self.press_instant) else {
            return;
        };
        let moved_px = (release_pos.0 - press_pos.0).hypot(release_pos.1 - press_pos.1);
        if moved_px > CLICK_MAX_MOVEMENT_PX {
            return;
        }
        if released_at.saturating_duration_since(pressed_at) > CLICK_MAX_DURATION {
            return;
        }
        let Some(camera) = self.camera.as_ref() else {
            return;
        };

        let selected = hit_test(
            &self.current_feed.aircraft,
            release_pos,
            camera.center_m(),
            camera.meters_per_pixel(),
            camera.width_px(),
            camera.height_px(),
        );
        tracing::info!(?selected, "selection changed");
        self.selected_icao24 = selected;
        if let Some(select_tx) = &self.select_tx {
            let _ = select_tx.send(selected);
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
                        ElementState::Pressed => camera.begin_drag(),
                        ElementState::Released => camera.end_drag(),
                    }
                }
                match state {
                    ElementState::Pressed => {
                        let now = Instant::now();
                        self.last_drag_instant = Some(now);
                        self.press_pos = self.last_cursor_pos;
                        self.press_instant = Some(now);
                    }
                    ElementState::Released => {
                        self.last_drag_instant = None;
                        if let Some(release_pos) = self.last_cursor_pos {
                            self.maybe_select(release_pos, Instant::now());
                        }
                        self.press_pos = None;
                        self.press_instant = None;
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

#[cfg(test)]
mod tests {
    use look_above_core::contracts::FlightCategory;
    use look_above_core::types::UnixSeconds;

    use super::*;

    fn airport(ident: &str, size: AirportSize, lat_deg: f64, lon_deg: f64) -> Airport {
        Airport {
            ident: ident.to_owned(),
            name: format!("{ident} test airport"),
            size,
            lat_deg,
            lon_deg,
            elevation_ft: None,
            iso_country: None,
            iata: None,
        }
    }

    fn metar(station: &str, category: Option<FlightCategory>) -> Metar {
        Metar {
            station: station.to_owned(),
            observed_at: UnixSeconds(1_700_000_000),
            raw: format!("{station} RAW"),
            flight_category: category,
            wind_dir_deg: None,
            wind_kt: None,
            visibility_sm: None,
        }
    }

    // ---- `metar_badges_for` (M3 item 3.3) ----------------------------------------------------

    #[test]
    fn a_large_airport_with_a_categorized_metar_gets_a_badge_at_its_own_position() {
        let airports = vec![airport("KJFK", AirportSize::Large, 40.64, -73.78)];
        let metars = vec![metar("KJFK", Some(FlightCategory::Ifr))];

        let badges = metar_badges_for(&airports, &metars);
        assert_eq!(
            badges,
            vec![MetarBadge {
                lat_deg: 40.64,
                lon_deg: -73.78,
                category: FlightCategory::Ifr,
            }]
        );
    }

    #[test]
    fn a_medium_or_smaller_airport_never_gets_a_badge_even_with_a_categorized_metar() {
        let airports = vec![airport("KTEB", AirportSize::Medium, 40.85, -74.06)];
        let metars = vec![metar("KTEB", Some(FlightCategory::Vfr))];

        assert!(metar_badges_for(&airports, &metars).is_empty());
    }

    #[test]
    fn a_large_airport_with_no_cached_metar_gets_no_badge() {
        let airports = vec![airport("KJFK", AirportSize::Large, 40.64, -73.78)];
        assert!(metar_badges_for(&airports, &[]).is_empty());
    }

    #[test]
    fn a_metar_with_no_resolved_category_gets_no_badge() {
        let airports = vec![airport("KJFK", AirportSize::Large, 40.64, -73.78)];
        let metars = vec![metar("KJFK", None)];
        assert!(
            metar_badges_for(&airports, &metars).is_empty(),
            "a metar with an uncomputable flight category has nothing to draw"
        );
    }

    #[test]
    fn a_metar_for_a_different_station_does_not_cross_wire_onto_this_airport() {
        let airports = vec![airport("KJFK", AirportSize::Large, 40.64, -73.78)];
        let metars = vec![metar("KLAX", Some(FlightCategory::Lifr))];
        assert!(metar_badges_for(&airports, &metars).is_empty());
    }

    #[test]
    fn only_the_large_subset_is_badged_out_of_a_mixed_size_query() {
        let airports = vec![
            airport("KJFK", AirportSize::Large, 40.64, -73.78),
            airport("KTEB", AirportSize::Medium, 40.85, -74.06),
        ];
        let metars = vec![
            metar("KJFK", Some(FlightCategory::Mvfr)),
            metar("KTEB", Some(FlightCategory::Vfr)),
        ];

        let badges = metar_badges_for(&airports, &metars);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].category, FlightCategory::Mvfr);
    }
}
