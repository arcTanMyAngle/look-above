//! The interpolation / dead-reckoning engine — the stage that makes sparse feeds *glide*.
//!
//! Authorized feeds update every 5–60 s, but aircraft must move continuously at 60 fps. This
//! module holds one [`Track`] per aircraft and, every frame, advances each to the current
//! render time and emits a flat [`RenderFeed`] the renderer consumes. The math is specified in
//! the high-fidelity-flight-visualization skill; this is its implementation, and the tests cite
//! the same formulas.
//!
//! Two entry points, driven at two different rates:
//!
//! - [`Simulator::ingest`] — called when a fresh poll cycle's [`StateVector`]s arrive (every
//!   5–60 s). A record whose source time of applicability (`ts`) is newer than the held fix
//!   installs a new fix and starts a **correction blend**; an older-or-equal record is ignored
//!   (the [`crate::merge::SessionTable`] already deduped, but a re-sent identical fix must not
//!   restart a blend).
//! - [`Simulator::advance_all`] — called once per presented frame. Dead-reckons every track
//!   forward to `now_s`, runs any in-progress blend and stale fade, projects to Web Mercator,
//!   and returns the [`RenderFeed`]. Parallel over the track table (`rayon`), since each track
//!   advances independently (ADR-002's "all layout/interpolation on workers").
//!
//! Time is wall-clock seconds as `f64`, the same epoch and units as [`StateVector::ts`] — the
//! caller passes `SystemTime::now()` as fractional seconds. Dead-reckoning `Δt` is clamped to
//! `[0, DROP_AFTER_S]` so a clock jump or a source-clock skew never advances an aircraft
//! wildly; the blend clamps its own progress to `[0, 1]`.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::collections::hash_map::Entry;

use rayon::prelude::*;

use crate::contracts::AircraftCategory;
use crate::geo::{
    LatLon, MercatorXy, destination_point, haversine_distance_m, initial_bearing_deg,
    normalize_bearing_deg, web_mercator_forward,
};
use crate::merge::{DROP_AFTER_S, STALE_AFTER_S};
use crate::types::{CallSign, Icao24, StateVector};

/// The correction-blend window, in seconds — the skill's `min(2 s, …)` cap. We do not know the
/// exact time to the next fix (it depends on the budget-driven poll cadence), so the cap is used
/// directly; the ease-out curve makes even a full 2 s feel immediate.
const BLEND_WINDOW_S: f64 = 2.0;

/// Position error, in metres, beyond which a new fix is treated as a teleport (data gap or a
/// wrong-aircraft merge) rather than a slide to correct — the skill's 10 km threshold.
const TELEPORT_M: f64 = 10_000.0;

/// The teleport fade-out/in window, in seconds — the skill's 300 ms. The glyph fades out, the
/// position snaps at the midpoint (while invisible), then it fades back in, so the eye never
/// sees a slide across the map.
const TELEPORT_FADE_S: f64 = 0.3;

/// How long, in seconds, a stale track fades from full to zero alpha once past
/// [`STALE_AFTER_S`] — the "+ 5 s" of the plan's "60 s + 5 s". Fully faded by
/// `STALE_AFTER_S + FADE_DURATION_S` (65 s); the track itself is retained (invisible) until
/// [`DROP_AFTER_S`] so a reacquisition inside that window blends rather than pops back in.
pub const FADE_DURATION_S: f64 = 5.0;

/// How long a trail's ring buffer retains displayed positions, in seconds — the skill's "last
/// 5 min of displayed positions" (M2 item 2.6a).
pub const TRAIL_DURATION_S: f64 = 300.0;

/// Minimum spacing between retained trail samples, in seconds — the skill's "sampled at ≥ 1 Hz".
/// [`Simulator::advance_all`] runs far faster than this (render cadence, tens of Hz); this
/// throttles how often a new point is actually pushed onto the ring buffer, bounding a full
/// track's trail to at most `TRAIL_DURATION_S / TRAIL_SAMPLE_INTERVAL_S` (300) points.
pub const TRAIL_SAMPLE_INTERVAL_S: f64 = 1.0;

/// Feet per metre, for the altitude-bucket thresholds (which the skill states in feet).
const FT_PER_M: f64 = 3.280_839_895;

/// Knots per m/s — exact, from the definition of a knot (1 nmi/h = 1,852 m / 3,600 s). Used to
/// convert [`Fix::speed_ms`] into [`AircraftInstance::ground_speed_kt`] for the label text (M2
/// item 2.7a: `core` carries the unit conversion once, the same way it already does for
/// [`AltitudeBucket`], rather than teaching every consumer feet/knots arithmetic).
const KT_PER_MS: f64 = 3600.0 / 1852.0;

/// Seconds-since-epoch (and the small staleness horizons) as `f64`.
///
/// Seconds-since-epoch is ~1.7 × 10⁹ and the horizons are tens of seconds — both are exact
/// integers in `f64` (whose exact-integer range is 2⁵³), so this widening loses nothing.
#[allow(clippy::cast_precision_loss)]
const fn as_seconds_f64(seconds: i64) -> f64 {
    seconds as f64
}

/// The altitude band an aircraft falls in — the tint attribute for glyphs and trails (the
/// skill's six-stop ramp; the actual ramp colors land in M4, the buckets are wired now).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AltitudeBucket {
    /// On the ground/taxiing, or airborne with no altitude reported (a neutral fallback — feeds
    /// almost always carry barometric altitude, so this is rare in the air).
    Ground,
    /// Below 2,000 ft.
    Below2000Ft,
    /// 2,000–10,000 ft.
    To10000Ft,
    /// 10,000–28,000 ft.
    To28000Ft,
    /// 28,000–40,000 ft (up to FL400).
    To40000Ft,
    /// Above FL400.
    Above40000Ft,
}

impl AltitudeBucket {
    /// Classifies an altitude (metres) into its band. On the ground, or with no known altitude,
    /// this is [`AltitudeBucket::Ground`].
    fn classify(on_ground: bool, alt_known: bool, alt_m: f64) -> Self {
        if on_ground || !alt_known {
            return Self::Ground;
        }
        let ft = alt_m * FT_PER_M;
        if ft < 2_000.0 {
            Self::Below2000Ft
        } else if ft < 10_000.0 {
            Self::To10000Ft
        } else if ft < 28_000.0 {
            Self::To28000Ft
        } else if ft < 40_000.0 {
            Self::To40000Ft
        } else {
            Self::Above40000Ft
        }
    }
}

/// One aircraft's drawable state for a single frame, ready for the renderer.
///
/// The position is already projected to Web Mercator metres (the projection is batched on the
/// same worker pass as the interpolation, per the skill's performance recipe). Fields kept as
/// `f64` here; the renderer narrows to `f32` when it packs the GPU instance buffer (2.5), so
/// `core` carries no render-specific numeric convention.
///
/// Not `Copy` (M2 item 2.7a, same reason [`Track`] itself dropped it at 2.6a): `callsign` owns a
/// heap allocation. Nothing in this crate or `render` ever needed to duplicate a whole instance
/// by value, only pass it by reference (`aircraft::pack_instance`) or move it out of the
/// `rayon` collection in [`Simulator::advance_all`].
#[derive(Debug, Clone, PartialEq)]
pub struct AircraftInstance {
    pub icao24: Icao24,
    /// Displayed position, projected to Web Mercator (`EPSG:3857`) metres.
    pub position: MercatorXy,
    /// Smoothed display heading, degrees clockwise from north — glyph rotation.
    pub heading_deg: f64,
    pub altitude_bucket: AltitudeBucket,
    /// Glyph category. Always [`AircraftCategory::Unknown`] until enrichment wires it (M3/2.5);
    /// carried now so the instance shape is complete for the glyph pipeline.
    pub category: AircraftCategory,
    /// Stale-fade alpha, `(0, 1]` — instances at or below zero are omitted from the feed.
    pub alpha: f64,
    pub on_ground: bool,
    /// PIA/blocked target (privacy rule 2.2): the renderer draws it but never labels it.
    pub anonymous: bool,
    /// Last known callsign (M2 item 2.7a — label content). Sticky: a fix that omits it (a
    /// protocol framing gap, not a real loss of identity) does not clear a previously known one.
    /// `None` before any fix has carried one — the label's "omit unknowns" case.
    pub callsign: Option<CallSign>,
    /// Displayed altitude in feet, for the label's `FLnnn` text — the same value
    /// [`AltitudeBucket::classify`] buckets, just carried at full precision. `None` when the fix
    /// has never reported an altitude (mirrors `altitude_bucket`'s `alt_known` gate); still
    /// `Some` while on the ground, since "0 ft" is real data, not an unknown — the label's own
    /// formatting (2.7b) decides whether to show it while `on_ground`.
    pub altitude_ft: Option<f64>,
    /// Ground speed in knots, for the label's `nnnkt` text. `None` when the fix has never
    /// reported a velocity (the skill's missing-field rule) — the label's "omit unknowns" case.
    pub ground_speed_kt: Option<f64>,
    /// Whether this is the one aircraft [`Simulator::set_selected`] currently names (M2 item
    /// 2.8a). Drives the render side's white outline (2.8b) and label priority ("selected >
    /// speed > proximity", docs/01) — `render::label::label_priority` hardcoded this to `false`
    /// with a doc comment pointing here until this field existed.
    pub selected: bool,
}

/// One point along an aircraft's trail (M2 item 2.6a) — a flat centerline sample, not yet
/// widened into a ribbon: that needs the camera's current `meters_per_pixel` to keep the taper a
/// constant screen-space width, and `core` has no camera (2.3a keeps it in `app`) — so the
/// perpendicular-offset/tessellation math is 2.6b's render-side problem, the same way 2.5 kept
/// the glyph's zoom-dependent on-screen sizing (`aircraft::glyph_scale_normalized`) out of
/// `core` entirely.
///
/// [`RenderFeed::trails`] groups these contiguously per aircraft in the same address-sorted
/// order as [`RenderFeed::aircraft`] (see [`Simulator::advance_all`]), so the render side can
/// build one ribbon per aircraft without needing an explicit run-length or index into the feed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailVertex {
    pub icao24: Icao24,
    /// Displayed position at the time this sample was recorded, projected to Web Mercator
    /// metres — same projection convention as [`AircraftInstance::position`].
    pub position: MercatorXy,
    /// This sample's *own* altitude band at the time it was recorded, not the track's current
    /// one — the skill's "colored by the altitude ramp" per vertex, so a climbing aircraft's
    /// trail shows its real historical bands rather than one repeated current color.
    pub altitude_bucket: AltitudeBucket,
    /// Seconds since this sample was recorded: 0 at the aircraft, up to [`TRAIL_DURATION_S`] at
    /// the tail. The render side derives width/alpha taper from this.
    pub age_s: f64,
}

/// What the CPU pipeline hands the renderer each frame (docs/09).
///
/// Flat and pre-sorted; the render thread swaps to it and never blocks on production (the
/// double-buffering itself is 2.4b's job). docs/09 types a `labels: Vec<Label>` field here,
/// "pre-collision-culled" — but collision culling and placement (right-of-glyph, flip near the
/// viewport edge, leader lines) are inherently screen-space and need the camera, which `core`
/// deliberately doesn't have (2.3a), the same reason 2.6a kept trail ribbon-widening out of this
/// struct. M2 item 2.7a instead carries the label *content* (callsign/altitude/speed) as new
/// fields directly on [`AircraftInstance`] — no new interpolation or blending needed, it's static
/// per-fix data — and leaves layout/culling entirely to `render` (2.7b), a documented deviation
/// from docs/09's literal shape rather than a silent one.
#[derive(Debug, Clone, Default)]
pub struct RenderFeed {
    /// Frame time, wall-clock seconds — the `now_s` the feed was advanced to.
    pub frame_ts: f64,
    /// Drawable aircraft, sorted deterministically by address (a real draw-priority order —
    /// altitude, then selection — arrives with the glyph/selection work in 2.5/2.8).
    pub aircraft: Vec<AircraftInstance>,
    /// Trail centerline samples (M2 item 2.6a) — flat, grouped contiguously per aircraft in the
    /// same address-sorted order as [`RenderFeed::aircraft`].
    pub trails: Vec<TrailVertex>,
}

/// The interpolation engine: one [`Track`] per aircraft, advanced to render time on demand.
#[derive(Debug, Clone, Default)]
pub struct Simulator {
    tracks: HashMap<Icao24, Track>,
    /// The one aircraft the user has clicked on (M2 item 2.8a), or `None`. Naming an `icao24`
    /// with no live track is harmless — [`Simulator::advance_all`] just never finds a match to
    /// mark, and it self-corrects the moment that aircraft (re)appears, so nothing here needs to
    /// clear this when a track expires.
    selected: Option<Icao24>,
}

impl Simulator {
    /// An empty simulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets (or clears, with `None`) the currently selected aircraft — `app::window`'s click
    /// hit-test calls this (via the simulation worker) each time the user clicks the map. Applied
    /// to the very next [`Simulator::advance_all`], no faster.
    pub fn set_selected(&mut self, icao24: Option<Icao24>) {
        self.selected = icao24;
    }

    /// Applies a poll cycle's deduplicated states: a record newer than the held fix installs a
    /// new fix and starts a correction blend (or a teleport snap); an older-or-equal record is
    /// ignored. `now_s` is the current wall-clock time, needed to snapshot where each updated
    /// aircraft is *shown* right now as the blend's origin.
    pub fn ingest(&mut self, states: &[StateVector], now_s: f64) {
        for state in states {
            match self.tracks.entry(state.icao24) {
                Entry::Vacant(slot) => {
                    slot.insert(Track::new(state));
                }
                Entry::Occupied(mut slot) => {
                    let track = slot.get_mut();
                    if as_seconds_f64(state.ts.0) > track.fix.t0 {
                        track.apply_fix(state, now_s);
                    }
                }
            }
        }
    }

    /// Advances every track to `now_s` and returns the drawable feed. Tracks whose fade has
    /// completed are omitted from the feed but kept until [`DROP_AFTER_S`] (so a reacquisition
    /// blends); tracks past that horizon are forgotten.
    ///
    /// Parallel over the track table: each track advances independently of the others.
    pub fn advance_all(&mut self, now_s: f64) -> RenderFeed {
        let selected = self.selected;
        let mut frames: Vec<(AircraftInstance, Vec<TrailVertex>)> = self
            .tracks
            .par_iter_mut()
            .filter_map(|(icao24, track)| track.advance(now_s, selected == Some(*icao24)))
            .collect();

        // Deterministic order for a stable feed and reproducible tests; true draw-priority
        // ordering is 2.5/2.8's concern. Trails are flattened in this same order below, which is
        // what keeps each aircraft's samples contiguous in the flat `trails` list (2.6b's
        // render-side ribbon build depends on that contiguity).
        frames.sort_unstable_by_key(|(instance, _)| instance.icao24.as_bytes());

        self.tracks.retain(|_, track| !track.expired(now_s));

        let mut aircraft = Vec::with_capacity(frames.len());
        let mut trails = Vec::new();
        for (instance, trail) in frames {
            aircraft.push(instance);
            trails.extend(trail);
        }

        RenderFeed {
            frame_ts: now_s,
            aircraft,
            trails,
        }
    }

    /// How many aircraft are being tracked (including invisibly-fading ones not yet dropped).
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether no aircraft are tracked.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

/// One aircraft's evolving state between fixes.
///
/// Not `Copy` (unlike its own fields' individual types): the trail ring buffer is a `VecDeque`,
/// which owns a heap allocation — nothing in this module ever needs to duplicate a whole
/// `Track` by value, only mutate it in place through the tracks table.
#[derive(Debug, Clone)]
struct Track {
    icao24: Icao24,
    /// The authoritative fix currently dead-reckoned from.
    fix: Fix,
    /// The last state actually shown — the origin for the next blend and the anchor the
    /// no-backward invariant clamps against.
    display: Display,
    /// A correction blend in progress toward `fix`, if any.
    blend: Option<Blend>,
    /// Source time of the current fix, in wall-clock seconds — drives the stale fade.
    last_fix_ts: f64,
    /// PIA/blocked (privacy rule 2.2), carried from the fix onto the instance.
    anonymous: bool,
    /// Last known callsign (M2 item 2.7a). Sticky across fixes that omit it — see
    /// [`Track::apply_fix`].
    callsign: Option<CallSign>,
    /// Ring buffer of displayed positions sampled at ≥ 1 Hz over the last [`TRAIL_DURATION_S`]
    /// (M2 item 2.6a) — the skill's "last 5 min of displayed positions" trail. Oldest first.
    trail: VecDeque<TrailSample>,
    /// Wall-clock time (seconds) the last trail sample was recorded, or `None` before the first
    /// one — throttles [`Track::record_trail_sample`] to [`TRAIL_SAMPLE_INTERVAL_S`] even though
    /// [`Track::advance`] itself runs at render cadence, far faster than 1 Hz.
    last_trail_sample_s: Option<f64>,
}

impl Track {
    /// A first sighting: shown exactly where the fix places it, no blend.
    fn new(state: &StateVector) -> Self {
        let fix = Fix::from_state(state);
        Self {
            icao24: state.icao24,
            display: Display {
                pos: fix.pos,
                heading_deg: fix.track_deg.unwrap_or(0.0),
                alt_m: fix.alt_m.unwrap_or(0.0),
            },
            fix,
            blend: None,
            last_fix_ts: fix.t0,
            anonymous: state.anonymous,
            callsign: state.callsign.clone(),
            trail: VecDeque::new(),
            last_trail_sample_s: None,
        }
    }

    /// Installs a newer fix and begins the transition to it from where the aircraft is shown now.
    fn apply_fix(&mut self, state: &StateVector, now_s: f64) {
        let from = self.display;
        let new_fix = Fix::from_state(state);
        let target = dead_reckon(&new_fix, now_s);
        let error_m = haversine_distance_m(from.pos, target.pos);

        self.fix = new_fix;
        self.last_fix_ts = new_fix.t0;
        self.anonymous = state.anonymous;
        // Sticky: a fix that omits the callsign (a protocol framing gap — identification
        // messages arrive separately from position ones) must not blank out a previously known
        // one, or the label would flicker in and out with every other poll cycle.
        if let Some(cs) = &state.callsign {
            self.callsign = Some(cs.clone());
        }

        let mode = if error_m > TELEPORT_M {
            BlendMode::Teleport
        } else {
            BlendMode::Slide
        };
        self.blend = Some(Blend {
            from_pos: from.pos,
            from_heading_deg: from.heading_deg,
            from_alt_m: from.alt_m,
            start_s: now_s,
            window_s: match mode {
                BlendMode::Slide => BLEND_WINDOW_S,
                BlendMode::Teleport => TELEPORT_FADE_S,
            },
            mode,
        });
    }

    /// Advances to `now_s`, updating [`Track::display`] and returning the drawable instance plus
    /// its current trail vertices — `None` once the stale fade has reached zero (the track
    /// lingers for reacquisition but is not drawn, and records no trail sample while invisible).
    /// `selected` is [`Simulator::advance_all`]'s per-track comparison against its own held
    /// `icao24` (M2 item 2.8a) — carried in as a plain `bool` rather than an `Icao24` so `Track`
    /// itself never needs to know its own address is being compared against anything.
    fn advance(
        &mut self,
        now_s: f64,
        selected: bool,
    ) -> Option<(AircraftInstance, Vec<TrailVertex>)> {
        let prev = self.display;
        let mut alpha_mul = 1.0;

        let advanced = match self.blend {
            None => {
                let dr = dead_reckon(&self.fix, now_s);
                Display {
                    pos: dr.pos,
                    heading_deg: dr.heading_deg.unwrap_or(prev.heading_deg),
                    alt_m: dr.alt_m,
                }
            }
            Some(blend) => {
                let u = ((now_s - blend.start_s) / blend.window_s).clamp(0.0, 1.0);
                let target = dead_reckon(&self.fix, now_s);
                let display = match blend.mode {
                    BlendMode::Slide => {
                        let e = ease_out(u);
                        let target_heading = target.heading_deg.unwrap_or(blend.from_heading_deg);
                        Display {
                            pos: geodesic_lerp(blend.from_pos, target.pos, e),
                            heading_deg: blend_heading_deg(
                                blend.from_heading_deg,
                                target_heading,
                                e,
                            ),
                            alt_m: blend.from_alt_m + (target.alt_m - blend.from_alt_m) * e,
                        }
                    }
                    BlendMode::Teleport => {
                        // Snap at the midpoint, while the fade has it invisible; a symmetric
                        // dip (1 → 0 → 1) hides the jump.
                        alpha_mul = (2.0 * u - 1.0).abs();
                        if u < 0.5 {
                            Display {
                                pos: blend.from_pos,
                                heading_deg: blend.from_heading_deg,
                                alt_m: blend.from_alt_m,
                            }
                        } else {
                            Display {
                                pos: target.pos,
                                heading_deg: target.heading_deg.unwrap_or(blend.from_heading_deg),
                                alt_m: target.alt_m,
                            }
                        }
                    }
                };
                if u >= 1.0 {
                    self.blend = None;
                }
                display
            }
        };

        let pos = self.clamp_no_backward(prev.pos, advanced.pos);
        self.display = Display { pos, ..advanced };

        let alpha = alpha_from_age(now_s - self.last_fix_ts) * alpha_mul;
        if alpha <= 0.0 {
            return None;
        }

        let alt_known = self.fix.alt_m.is_some();
        let instance = AircraftInstance {
            icao24: self.icao24,
            position: web_mercator_forward(self.display.pos),
            heading_deg: self.display.heading_deg,
            altitude_bucket: AltitudeBucket::classify(
                self.fix.on_ground,
                alt_known,
                self.display.alt_m,
            ),
            category: AircraftCategory::Unknown,
            alpha,
            on_ground: self.fix.on_ground,
            anonymous: self.anonymous,
            callsign: self.callsign.clone(),
            altitude_ft: alt_known.then_some(self.display.alt_m * FT_PER_M),
            ground_speed_kt: self.fix.speed_ms.map(|v| v * KT_PER_MS),
            selected,
        };

        // Only recorded while the instance is actually visible this frame — an aircraft that
        // isn't shown has no "displayed position" to sample, so a stale-faded gap leaves a real
        // hole in the trail rather than recording a phantom point (the skill's ring buffer is of
        // *displayed* positions).
        self.record_trail_sample(now_s, alt_known);
        let trail = self.trail_vertices(now_s);

        Some((instance, trail))
    }

    /// Appends a trail sample for the position just shown, throttled to at most one push per
    /// [`TRAIL_SAMPLE_INTERVAL_S`] even though [`Track::advance`] itself runs at render cadence,
    /// then evicts anything older than [`TRAIL_DURATION_S`] (run every call regardless of
    /// whether a new sample was pushed, so a track that stops moving still ages its trail out).
    fn record_trail_sample(&mut self, now_s: f64, alt_known: bool) {
        let should_sample = match self.last_trail_sample_s {
            None => true,
            Some(last) => now_s - last >= TRAIL_SAMPLE_INTERVAL_S,
        };
        if should_sample {
            self.trail.push_back(TrailSample {
                pos: self.display.pos,
                alt_m: self.display.alt_m,
                alt_known,
                on_ground: self.fix.on_ground,
                t_s: now_s,
            });
            self.last_trail_sample_s = Some(now_s);
        }
        while let Some(front) = self.trail.front() {
            if now_s - front.t_s > TRAIL_DURATION_S {
                self.trail.pop_front();
            } else {
                break;
            }
        }
    }

    /// This track's current trail, projected to Web Mercator and classified per-sample — the
    /// skill's "colored by the altitude ramp" (each vertex reflects *that sample's own*
    /// historical altitude, not the track's current one).
    fn trail_vertices(&self, now_s: f64) -> Vec<TrailVertex> {
        self.trail
            .iter()
            .map(|sample| TrailVertex {
                icao24: self.icao24,
                position: web_mercator_forward(sample.pos),
                altitude_bucket: AltitudeBucket::classify(
                    sample.on_ground,
                    sample.alt_known,
                    sample.alt_m,
                ),
                age_s: now_s - sample.t_s,
            })
            .collect()
    }

    /// Enforces the skill's invariant: a moving aircraft never slides *backwards* along its own
    /// track (a fix behind the shown position slows it to a stop, never reverses it). Returns
    /// the candidate position, or the previous one if the step would move backward along track.
    fn clamp_no_backward(&self, prev: LatLon, candidate: LatLon) -> LatLon {
        let Some(track_deg) = self.fix.track_deg else {
            return candidate;
        };
        if self.fix.speed_ms.is_none() || self.fix.on_ground {
            return candidate;
        }
        let moved_m = haversine_distance_m(prev, candidate);
        if moved_m <= 1e-6 {
            return candidate;
        }
        let step_bearing = initial_bearing_deg(prev, candidate);
        let along_track_m = moved_m * (step_bearing - track_deg).to_radians().cos();
        if along_track_m < 0.0 { prev } else { candidate }
    }

    /// Whether the track has aged past the drop horizon and should be forgotten.
    fn expired(&self, now_s: f64) -> bool {
        (now_s - self.last_fix_ts) >= as_seconds_f64(DROP_AFTER_S)
    }
}

/// The fields of a fix the dead-reckoner needs. Optional where the feed genuinely omits the
/// value: no speed or no track ⇒ hold position (the skill's missing-field rules).
#[derive(Debug, Clone, Copy)]
struct Fix {
    pos: LatLon,
    alt_m: Option<f64>,
    speed_ms: Option<f64>,
    /// True track, degrees clockwise from north.
    track_deg: Option<f64>,
    /// Vertical rate, m/s; absent ⇒ 0 (level).
    vert_rate_ms: f64,
    on_ground: bool,
    /// Source time of applicability, wall-clock seconds.
    t0: f64,
}

impl Fix {
    fn from_state(state: &StateVector) -> Self {
        Self {
            pos: LatLon::new(state.lat_deg, state.lon_deg),
            alt_m: state.baro_alt_m.map(f64::from),
            speed_ms: state.velocity_ms.map(f64::from),
            track_deg: state.heading_deg.map(f64::from),
            vert_rate_ms: state.vert_rate_ms.map_or(0.0, f64::from),
            on_ground: state.on_ground,
            t0: as_seconds_f64(state.ts.0),
        }
    }
}

/// The last displayed state, carried frame to frame.
#[derive(Debug, Clone, Copy)]
struct Display {
    pos: LatLon,
    heading_deg: f64,
    alt_m: f64,
}

/// One recorded trail point. Carries enough of the sample's own state (altitude, on-ground) to
/// reclassify its altitude bucket at emission time — the skill colors each vertex by *its own*
/// historical altitude, not the track's current one, so this can't just store the bucket and
/// throw the rest away.
#[derive(Debug, Clone, Copy)]
struct TrailSample {
    pos: LatLon,
    alt_m: f64,
    alt_known: bool,
    on_ground: bool,
    /// Wall-clock seconds this sample was recorded. [`TrailVertex::age_s`] is derived from this
    /// relative to the emitting frame's `now_s`, not stored directly, so a sample's reported age
    /// is always relative to *now*, not to whenever it happened to be pushed.
    t_s: f64,
}

/// A transition toward a freshly installed fix.
#[derive(Debug, Clone, Copy)]
struct Blend {
    from_pos: LatLon,
    from_heading_deg: f64,
    from_alt_m: f64,
    /// Render time the blend began, wall-clock seconds.
    start_s: f64,
    window_s: f64,
    mode: BlendMode,
}

/// How a new fix is reconciled with the shown position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlendMode {
    /// Ordinary error: ease the shown position onto the new fix over the blend window.
    Slide,
    /// Error too large to be real motion (> [`TELEPORT_M`]): fade out, snap, fade back in.
    Teleport,
}

/// The result of dead-reckoning a fix forward to a render time.
struct DeadReckoned {
    pos: LatLon,
    alt_m: f64,
    /// The heading to display: the fix's track, or `None` to keep the last shown heading.
    heading_deg: Option<f64>,
}

/// Advances a fix to `now_s` along its track (the skill's dead-reckoning step).
///
/// `Δt` is clamped to `[0, DROP_AFTER_S]`: never negative (a source clock ahead of ours must
/// not rewind the aircraft) and never beyond the drop horizon (a stale fix does not fling it
/// across the map). No speed or no track ⇒ position held; on the ground ⇒ never extrapolated.
fn dead_reckon(fix: &Fix, now_s: f64) -> DeadReckoned {
    let dt = (now_s - fix.t0).clamp(0.0, as_seconds_f64(DROP_AFTER_S));
    let base_alt = fix.alt_m.unwrap_or(0.0);

    match (fix.speed_ms, fix.track_deg) {
        (Some(speed), Some(track)) if !fix.on_ground => {
            let distance_m = speed * dt;
            DeadReckoned {
                pos: destination_point(fix.pos, track, distance_m),
                alt_m: (base_alt + fix.vert_rate_ms * dt).max(0.0),
                heading_deg: Some(track),
            }
        }
        // Held: on the ground, or missing the speed/track needed to advance.
        _ => DeadReckoned {
            pos: fix.pos,
            alt_m: base_alt,
            heading_deg: fix.track_deg,
        },
    }
}

/// The skill's ease-out curve, `1 − (1 − u)²`, on a clamped `u`.
fn ease_out(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    1.0 - (1.0 - u) * (1.0 - u)
}

/// Interpolates a fraction `t` of the way from `a` to `b` along their great circle — a geodesic
/// lerp, so the blended path curves correctly rather than cutting a straight chord.
fn geodesic_lerp(a: LatLon, b: LatLon, t: f64) -> LatLon {
    let distance_m = haversine_distance_m(a, b);
    if distance_m < 1e-6 {
        return a;
    }
    destination_point(a, initial_bearing_deg(a, b), distance_m * t)
}

/// Interpolates a heading the short way round (the skill's shortest-arc rule): 350° → 10°
/// crosses through 0°, not backwards through 180°.
fn blend_heading_deg(from: f64, to: f64, t: f64) -> f64 {
    let delta = (to - from + 180.0).rem_euclid(360.0) - 180.0;
    normalize_bearing_deg(from + delta * t)
}

/// The stale-fade alpha for a fix of the given age (seconds): full until [`STALE_AFTER_S`], then
/// a linear ramp to zero over [`FADE_DURATION_S`].
fn alpha_from_age(age_s: f64) -> f64 {
    let stale = as_seconds_f64(STALE_AFTER_S);
    if age_s <= stale {
        return 1.0;
    }
    (1.0 - (age_s - stale) / FADE_DURATION_S).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::web_mercator_inverse;
    use crate::types::{SourceId, UnixSeconds};

    const EAST: f32 = 90.0;

    fn hex(s: &str) -> Icao24 {
        Icao24::from_hex(s).expect("valid ICAO24 in test")
    }

    /// A fix for `icao` at `(lat, lon)`, time `t0`, flying `speed` m/s on `track`, level at
    /// `alt`. The `with_*`/`grounded` helpers below drop individual fields.
    fn state(icao: &str, t0: i64, lat: f64, lon: f64, speed: f32, track: f32) -> StateVector {
        StateVector {
            icao24: hex(icao),
            callsign: None,
            ts: UnixSeconds(t0),
            lat_deg: lat,
            lon_deg: lon,
            baro_alt_m: Some(3_000.0),
            velocity_ms: Some(speed),
            heading_deg: Some(track),
            vert_rate_ms: Some(0.0),
            on_ground: false,
            anonymous: false,
            source: SourceId::OpenSky,
        }
    }

    /// The displayed position of `icao` in the feed, unprojected back to lat/lon. Going through
    /// the projected instance (rather than reading internal state) is deliberate: it exercises
    /// that `sim` projects the displayed position via Web Mercator, as the renderer expects.
    #[track_caller]
    fn shown(sim: &mut Simulator, icao: &str, now_s: f64) -> LatLon {
        let feed = sim.advance_all(now_s);
        let instance = feed
            .aircraft
            .iter()
            .find(|a| a.icao24 == hex(icao))
            .expect("aircraft present in feed");
        web_mercator_inverse(instance.position)
    }

    /// Along-track (eastward) progress as a longitude, for the monotonicity assertions.
    fn east_lon(sim: &mut Simulator, icao: &str, now_s: f64) -> f64 {
        shown(sim, icao, now_s).lon_deg
    }

    // ---- Dead reckoning -------------------------------------------------------------------------

    #[test]
    fn position_advances_along_track_at_ground_speed() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST)], 1_000.0);

        // 10 s at 200 m/s due east = 2 km along the equator.
        let p = shown(&mut sim, "3c6444", 1_010.0);
        let from = LatLon::new(0.0, 0.0);
        assert!((haversine_distance_m(from, p) - 2_000.0).abs() < 0.5);
        let bearing = initial_bearing_deg(from, p);
        assert!(
            (bearing - 90.0).abs() < 1e-6,
            "bearing {bearing} not due east"
        );
    }

    #[test]
    fn vertical_rate_integrates_into_altitude() {
        let mut sim = Simulator::new();
        // Level at 2,900 m (9,514 ft, in the To10000Ft band), climbing 5 m/s.
        let mut climb = state("3c6444", 1_000, 0.0, 0.0, 100.0, EAST);
        climb.baro_alt_m = Some(2_900.0);
        climb.vert_rate_ms = Some(5.0);
        sim.ingest(&[climb], 1_000.0);
        assert_eq!(
            sim.advance_all(1_000.0).aircraft[0].altitude_bucket,
            AltitudeBucket::To10000Ft
        );
        // +60 s → +300 m → 3,200 m (10,499 ft): climbed across the 10,000 ft boundary.
        assert_eq!(
            sim.advance_all(1_060.0).aircraft[0].altitude_bucket,
            AltitudeBucket::To28000Ft
        );

        // A negative rate integrates downward — a descending aircraft pins the sign.
        let mut descend = state("4b1815", 1_000, 0.0, 0.0, 100.0, EAST);
        descend.baro_alt_m = Some(3_100.0); // 10,170 ft, To28000Ft
        descend.vert_rate_ms = Some(-5.0);
        sim.ingest(&[descend], 1_000.0);
        // +60 s → 3,100 − 300 = 2,800 m (9,186 ft): descended into To10000Ft.
        let feed = sim.advance_all(1_060.0);
        let inst = feed
            .aircraft
            .iter()
            .find(|a| a.icao24 == hex("4b1815"))
            .expect("descending aircraft present");
        assert_eq!(inst.altitude_bucket, AltitudeBucket::To10000Ft);
    }

    #[test]
    fn dead_reckon_clamps_dt_to_the_drop_horizon_and_never_rewinds() {
        // The clamp is defensive (a visible aircraft never ages past ~65 s), so exercise the
        // dead-reckoner directly rather than through the fade-gated feed.
        let fix = Fix::from_state(&state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST));
        let origin = LatLon::new(0.0, 0.0);

        // 10,000 s on, Δt clamps at DROP_AFTER_S = 90 s → at most 200 * 90 = 18 km along track.
        let far = dead_reckon(&fix, 11_000.0);
        assert!(haversine_distance_m(origin, far.pos) <= 200.0 * 90.0 + 1.0);

        // A source clock ahead of ours (negative Δt) holds position rather than rewinding it.
        let ahead_clock = dead_reckon(&fix, 990.0);
        assert!(haversine_distance_m(origin, ahead_clock.pos) < 1e-6);
    }

    #[test]
    fn missing_speed_holds_position() {
        let mut sim = Simulator::new();
        let mut s = state("3c6444", 1_000, 10.0, 20.0, 200.0, EAST);
        s.velocity_ms = None;
        sim.ingest(&[s], 1_000.0);

        let p = shown(&mut sim, "3c6444", 1_060.0);
        assert!((p.lat_deg - 10.0).abs() < 1e-9 && (p.lon_deg - 20.0).abs() < 1e-9);
    }

    #[test]
    fn missing_track_holds_position() {
        let mut sim = Simulator::new();
        let mut s = state("3c6444", 1_000, 10.0, 20.0, 200.0, EAST);
        s.heading_deg = None;
        sim.ingest(&[s], 1_000.0);

        let p = shown(&mut sim, "3c6444", 1_060.0);
        assert!((p.lat_deg - 10.0).abs() < 1e-9 && (p.lon_deg - 20.0).abs() < 1e-9);
    }

    #[test]
    fn on_ground_never_extrapolates() {
        let mut sim = Simulator::new();
        let mut s = state("3c6444", 1_000, 10.0, 20.0, 200.0, EAST);
        s.on_ground = true;
        sim.ingest(&[s], 1_000.0);

        let feed = sim.advance_all(1_060.0);
        let inst = &feed.aircraft[0];
        let p = web_mercator_inverse(inst.position);
        assert!((p.lat_deg - 10.0).abs() < 1e-9 && (p.lon_deg - 20.0).abs() < 1e-9);
        assert_eq!(inst.altitude_bucket, AltitudeBucket::Ground);
    }

    // ---- Correction blend -----------------------------------------------------------------------

    #[test]
    fn blend_converges_to_the_new_fix_within_the_window() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST)], 1_000.0);
        let _ = sim.advance_all(1_002.0); // shown ~400 m east

        // A fix a few km ahead, well under the teleport threshold.
        let ahead = state("3c6444", 1_002, 0.0, 0.05, 200.0, EAST); // ~5.6 km east
        let ahead_fix = Fix::from_state(&ahead);
        sim.ingest(&[ahead], 1_002.0);

        // At u = 0 the shown position is still the pre-blend one, not the new fix.
        let at_start = shown(&mut sim, "3c6444", 1_002.0);
        assert!(at_start.lon_deg < 0.02, "blend jumped instead of easing in");

        // At u = 1 (start + window) it has caught the new fix, itself dead-reckoned to now.
        let at_end = shown(&mut sim, "3c6444", 1_002.0 + BLEND_WINDOW_S);
        let target = dead_reckon(&ahead_fix, 1_002.0 + BLEND_WINDOW_S);
        assert!(
            haversine_distance_m(at_end, target.pos) < 1.0,
            "blend did not converge to the fix"
        );
    }

    #[test]
    fn blend_never_moves_backwards_along_track() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST)], 1_000.0);
        let _ = sim.advance_all(1_005.0); // shown ~1 km east

        // A fix that places the aircraft *behind* where it is shown (200 m east vs ~1 km).
        let behind = state("3c6444", 1_005, 0.0, 0.0018, 200.0, EAST); // ~200 m east
        sim.ingest(&[behind], 1_005.0);

        let mut last = east_lon(&mut sim, "3c6444", 1_005.0);
        let mut t = 1_005.2;
        while t <= 1_010.0 {
            let lon = east_lon(&mut sim, "3c6444", t);
            assert!(
                lon >= last - 1e-9,
                "moved backwards along track: {lon} < {last} at t={t}"
            );
            last = lon;
            t += 0.2;
        }
    }

    #[test]
    fn a_large_error_teleports_rather_than_sliding_across_the_map() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        let _ = sim.advance_all(1_000.0);

        // ~55 km east — past the 10 km teleport threshold — and a newer fix (t0 = 1001).
        let far = state("3c6444", 1_001, 0.0, 0.5, 0.0, EAST);
        sim.ingest(&[far], 1_001.0);

        // Before the midpoint the aircraft is still near the origin and fading out.
        let feed_early = sim.advance_all(1_001.1); // u ≈ 0.33
        let early = &feed_early.aircraft[0];
        assert!(web_mercator_inverse(early.position).lon_deg < 0.1);
        assert!(early.alpha < 1.0, "teleport did not dip the alpha");

        // After the window it has snapped to the far fix at full alpha, no intermediate slide.
        let feed_end = sim.advance_all(1_001.0 + TELEPORT_FADE_S);
        let end = &feed_end.aircraft[0];
        assert!((web_mercator_inverse(end.position).lon_deg - 0.5).abs() < 1e-6);
        assert!((end.alpha - 1.0).abs() < 1e-9);
    }

    #[test]
    fn an_older_or_equal_fix_does_not_restart_a_blend() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST)], 1_000.0);
        let _ = sim.advance_all(1_010.0);
        let after_first = east_lon(&mut sim, "3c6444", 1_010.0);

        // Re-sending the same-ts fix (as SessionTable might on a repeat sighting) must not
        // start a new blend or move the aircraft.
        sim.ingest(&[state("3c6444", 1_000, 5.0, 5.0, 200.0, EAST)], 1_010.0);
        let after_repeat = east_lon(&mut sim, "3c6444", 1_010.0);
        assert!((after_first - after_repeat).abs() < 1e-9);
    }

    // ---- Staleness ------------------------------------------------------------------------------

    #[test]
    fn stale_fade_ramps_over_the_fade_window_then_drops() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);

        // Fresh, and right at the stale horizon: full alpha.
        assert!((sim.advance_all(1_060.0).aircraft[0].alpha - 1.0).abs() < 1e-9);
        // Halfway through the 5 s fade.
        assert!((sim.advance_all(1_062.5).aircraft[0].alpha - 0.5).abs() < 1e-9);
        // Fully faded: dropped from the feed, but still tracked (for reacquisition).
        assert!(sim.advance_all(1_065.0).aircraft.is_empty());
        assert_eq!(sim.len(), 1);
        // Past the drop horizon: forgotten.
        assert!(sim.advance_all(1_090.0).aircraft.is_empty());
        assert_eq!(sim.len(), 0);
    }

    #[test]
    fn a_reacquired_fix_before_drop_blends_back_in() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        let _ = sim.advance_all(1_070.0); // faded out but still tracked
        assert!(sim.advance_all(1_070.0).aircraft.is_empty());

        // A fresh fix arrives before the drop horizon; the track is reused, not recreated.
        sim.ingest(&[state("3c6444", 1_075, 0.0, 0.0, 0.0, EAST)], 1_075.0);
        assert_eq!(sim.len(), 1);
        assert!((sim.advance_all(1_075.0).aircraft[0].alpha - 1.0).abs() < 1e-9);
    }

    // ---- Feed shape -----------------------------------------------------------------------------

    #[test]
    fn a_first_sighting_shows_exactly_at_the_fix() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 12.0, 34.0, 200.0, EAST)], 1_000.0);
        let p = shown(&mut sim, "3c6444", 1_000.0);
        assert!((p.lat_deg - 12.0).abs() < 1e-9 && (p.lon_deg - 34.0).abs() < 1e-9);
    }

    #[test]
    fn the_feed_is_sorted_by_address_and_carries_the_frame_time() {
        let mut sim = Simulator::new();
        sim.ingest(
            &[
                state("4b1815", 1_000, 0.0, 0.0, 0.0, EAST),
                state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST),
                state("aa0011", 1_000, 0.0, 0.0, 0.0, EAST),
            ],
            1_000.0,
        );
        let feed = sim.advance_all(1_000.0);
        assert!((feed.frame_ts - 1_000.0).abs() < 1e-9);
        let order: Vec<_> = feed.aircraft.iter().map(|a| a.icao24).collect();
        assert_eq!(order, vec![hex("3c6444"), hex("4b1815"), hex("aa0011")]);
    }

    #[test]
    fn anonymity_is_carried_onto_the_instance() {
        let mut sim = Simulator::new();
        let mut s = state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST);
        s.anonymous = true;
        sim.ingest(&[s], 1_000.0);
        assert!(sim.advance_all(1_000.0).aircraft[0].anonymous);
    }

    // ---- Selection (M2 item 2.8a) -----------------------------------------------------------------

    #[test]
    fn nothing_is_selected_by_default() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        assert!(!sim.advance_all(1_000.0).aircraft[0].selected);
    }

    #[test]
    fn set_selected_marks_only_the_named_aircraft() {
        let mut sim = Simulator::new();
        sim.ingest(
            &[
                state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST),
                state("4b1815", 1_000, 0.0, 0.0, 0.0, EAST),
            ],
            1_000.0,
        );
        sim.set_selected(Some(hex("4b1815")));

        let feed = sim.advance_all(1_000.0);
        let selected = |icao: &str| {
            feed.aircraft
                .iter()
                .find(|a| a.icao24 == hex(icao))
                .expect("aircraft present")
                .selected
        };
        assert!(!selected("3c6444"));
        assert!(selected("4b1815"));
    }

    #[test]
    fn set_selected_none_clears_the_selection() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        sim.set_selected(Some(hex("3c6444")));
        assert!(sim.advance_all(1_000.0).aircraft[0].selected);

        sim.set_selected(None);
        assert!(!sim.advance_all(1_000.0).aircraft[0].selected);
    }

    #[test]
    fn selecting_an_icao24_with_no_live_track_is_harmless() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        sim.set_selected(Some(hex("aa0011"))); // never ingested
        assert!(!sim.advance_all(1_000.0).aircraft[0].selected);
    }

    // ---- Label content (M2 item 2.7a) ------------------------------------------------------------

    #[test]
    fn a_first_sighting_carries_its_callsign_altitude_and_speed_onto_the_instance() {
        let mut sim = Simulator::new();
        let mut s = state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST); // 200 m/s, 3,000 m
        s.callsign = CallSign::new("DLH9LF");
        sim.ingest(&[s], 1_000.0);

        let instance = &sim.advance_all(1_000.0).aircraft[0];
        assert_eq!(
            instance.callsign.as_ref().map(CallSign::as_str),
            Some("DLH9LF")
        );
        assert!((instance.altitude_ft.expect("altitude known") - 3_000.0 * FT_PER_M).abs() < 1e-6);
        assert!((instance.ground_speed_kt.expect("speed known") - 200.0 * KT_PER_MS).abs() < 1e-6);
    }

    #[test]
    fn missing_callsign_altitude_or_speed_leaves_the_field_none() {
        let mut sim = Simulator::new();
        let mut s = state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST);
        s.callsign = None;
        s.baro_alt_m = None;
        s.velocity_ms = None;
        sim.ingest(&[s], 1_000.0);

        let instance = &sim.advance_all(1_000.0).aircraft[0];
        assert_eq!(instance.callsign, None);
        assert_eq!(instance.altitude_ft, None);
        assert_eq!(instance.ground_speed_kt, None);
    }

    #[test]
    fn a_later_fix_omitting_the_callsign_does_not_clear_a_previously_known_one() {
        let mut sim = Simulator::new();
        let mut first = state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST);
        first.callsign = CallSign::new("UAL123");
        sim.ingest(&[first], 1_000.0);
        assert_eq!(
            sim.advance_all(1_000.0).aircraft[0]
                .callsign
                .as_ref()
                .map(CallSign::as_str),
            Some("UAL123")
        );

        // A later fix's own callsign field is blank (a protocol framing gap) — the label must
        // keep showing the last known identity, not flicker to "no callsign".
        let mut second = state("3c6444", 1_010, 0.0, 0.0, 200.0, EAST);
        second.callsign = None;
        sim.ingest(&[second], 1_010.0);
        assert_eq!(
            sim.advance_all(1_010.0).aircraft[0]
                .callsign
                .as_ref()
                .map(CallSign::as_str),
            Some("UAL123"),
            "a blank callsign on a later fix must not clear the previously known one"
        );
    }

    #[test]
    fn a_later_fix_with_a_new_callsign_replaces_the_old_one() {
        let mut sim = Simulator::new();
        let mut first = state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST);
        first.callsign = CallSign::new("UAL123");
        sim.ingest(&[first], 1_000.0);

        let mut second = state("3c6444", 1_010, 0.0, 0.0, 200.0, EAST);
        second.callsign = CallSign::new("UAL456");
        sim.ingest(&[second], 1_010.0);
        assert_eq!(
            sim.advance_all(1_010.0).aircraft[0]
                .callsign
                .as_ref()
                .map(CallSign::as_str),
            Some("UAL456")
        );
    }

    #[test]
    fn altitude_ft_is_still_reported_while_on_the_ground() {
        // On-ground is real data ("0 ft"), not an unknown — the label's own formatting (2.7b)
        // decides whether to actually show it, `core` doesn't gate it away here.
        let mut sim = Simulator::new();
        let mut s = state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST);
        s.on_ground = true;
        s.baro_alt_m = Some(0.0);
        sim.ingest(&[s], 1_000.0);
        assert_eq!(sim.advance_all(1_000.0).aircraft[0].altitude_ft, Some(0.0));
    }

    // ---- Trails (M2 item 2.6a) ------------------------------------------------------------------

    /// This aircraft's trail vertices in the feed, in whatever order `advance_all` emitted them.
    fn trail_of(feed: &RenderFeed, icao: &str) -> Vec<TrailVertex> {
        feed.trails
            .iter()
            .copied()
            .filter(|v| v.icao24 == hex(icao))
            .collect()
    }

    #[test]
    fn a_first_sighting_records_exactly_one_trail_sample_at_zero_age() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        let feed = sim.advance_all(1_000.0);
        let trail = trail_of(&feed, "3c6444");
        assert_eq!(trail.len(), 1);
        assert!((trail[0].age_s - 0.0).abs() < 1e-9);
    }

    #[test]
    fn trail_sampling_is_throttled_to_the_sample_interval() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 200.0, EAST)], 1_000.0);

        // Several advances within one second must not each push a new sample. Each `t` is
        // computed fresh from the same base rather than accumulated with `+=`, so this doesn't
        // depend on floating-point drift staying under the 1 Hz throttle threshold.
        let mut feed = sim.advance_all(1_000.0);
        for step in 1..=9 {
            let t = 1_000.0 + f64::from(step) * 0.1;
            feed = sim.advance_all(t);
        }
        assert_eq!(
            trail_of(&feed, "3c6444").len(),
            1,
            "sub-interval advances must not add extra samples"
        );

        // Once the interval has elapsed, the next advance adds a second sample.
        let feed = sim.advance_all(1_000.0 + TRAIL_SAMPLE_INTERVAL_S);
        assert_eq!(trail_of(&feed, "3c6444").len(), 2);
    }

    #[test]
    fn trail_evicts_samples_older_than_the_retention_window() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);

        // Sample once a second for a while, well past the 5-minute retention window.
        let mut t = 1_000.0;
        let mut feed = sim.advance_all(t);
        while t < 1_000.0 + TRAIL_DURATION_S + 10.0 {
            t += TRAIL_SAMPLE_INTERVAL_S;
            feed = sim.advance_all(t);
        }

        let trail = trail_of(&feed, "3c6444");
        assert!(
            trail.iter().all(|v| v.age_s <= TRAIL_DURATION_S + 1e-6),
            "a sample older than the retention window survived: {trail:?}"
        );
        // Bounded to roughly one sample per second over the 300 s retention window (301
        // literal, not derived from the consts, to sidestep an f64->usize cast for a bound this
        // test only needs a generous ceiling on), not growing unboundedly with how long the
        // track has existed.
        assert!(
            trail.len() <= 301,
            "trail grew past its retention bound: {} samples",
            trail.len()
        );
    }

    #[test]
    fn no_trail_sample_is_recorded_while_the_instance_is_invisible() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        let before_fade = trail_of(&sim.advance_all(1_060.0), "3c6444").len();

        // Fully faded (past STALE_AFTER_S + FADE_DURATION_S = 65 s): not in the feed at all, so
        // no trail either.
        let feed = sim.advance_all(1_065.0);
        assert!(feed.aircraft.is_empty());
        assert!(trail_of(&feed, "3c6444").is_empty());

        // Reacquire before the drop horizon: the trail resumes (a new sample at the reacquired
        // position), rather than having grown a phantom sample during the invisible gap.
        sim.ingest(&[state("3c6444", 1_075, 0.0, 0.0, 0.0, EAST)], 1_075.0);
        let feed = sim.advance_all(1_075.0);
        let after_reacquire = trail_of(&feed, "3c6444").len();
        assert_eq!(
            after_reacquire,
            before_fade + 1,
            "exactly one new sample on reacquisition, none during the invisible gap"
        );
    }

    #[test]
    fn a_track_past_the_drop_horizon_carries_no_trail_into_the_feed() {
        let mut sim = Simulator::new();
        sim.ingest(&[state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST)], 1_000.0);
        let _ = sim.advance_all(1_000.0);

        let feed = sim.advance_all(1_090.0); // past DROP_AFTER_S: the track is forgotten
        assert_eq!(sim.len(), 0);
        assert!(trail_of(&feed, "3c6444").is_empty());
    }

    #[test]
    fn trail_vertex_altitude_bucket_reflects_its_own_recorded_altitude() {
        let mut sim = Simulator::new();
        // Climbing through the 2,000 ft boundary: below at t0, above a bit later.
        let mut climb = state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST);
        climb.baro_alt_m = Some(300.0); // ~984 ft, Below2000Ft
        climb.vert_rate_ms = Some(10.0); // climbs fast enough to cross 2,000 ft within a minute
        sim.ingest(&[climb], 1_000.0);

        let _ = sim.advance_all(1_000.0); // oldest sample: still low
        let feed = sim.advance_all(1_000.0 + TRAIL_SAMPLE_INTERVAL_S * 60.0); // newest: climbed
        let trail = trail_of(&feed, "3c6444");

        let oldest = trail
            .iter()
            .max_by(|a, b| a.age_s.total_cmp(&b.age_s))
            .expect("a sample");
        let newest = trail
            .iter()
            .min_by(|a, b| a.age_s.total_cmp(&b.age_s))
            .expect("a sample");
        assert_eq!(oldest.altitude_bucket, AltitudeBucket::Below2000Ft);
        assert_eq!(newest.altitude_bucket, AltitudeBucket::To10000Ft);
    }

    #[test]
    fn trails_stay_grouped_per_aircraft_in_the_feeds_sorted_order() {
        let mut sim = Simulator::new();
        sim.ingest(
            &[
                state("4b1815", 1_000, 0.0, 0.0, 0.0, EAST),
                state("3c6444", 1_000, 0.0, 0.0, 0.0, EAST),
            ],
            1_000.0,
        );
        let feed = sim.advance_all(1_000.0);

        // The trail list's icao24 sequence must not interleave the two aircraft — each run is
        // contiguous, in the same address-sorted order as `aircraft` (2.6b's render-side ribbon
        // build depends on this).
        let order: Vec<_> = feed.trails.iter().map(|v| v.icao24).collect();
        let mut seen_change = false;
        let mut previous = order[0];
        for icao in &order[1..] {
            if *icao != previous {
                assert!(
                    !seen_change,
                    "an aircraft's trail samples were not contiguous"
                );
                seen_change = true;
                previous = *icao;
            }
        }
        let aircraft_order: Vec<_> = feed.aircraft.iter().map(|a| a.icao24).collect();
        assert_eq!(aircraft_order, vec![hex("3c6444"), hex("4b1815")]);
        assert_eq!(order, vec![hex("3c6444"), hex("4b1815")]);
    }

    // ---- Altitude buckets -----------------------------------------------------------------------

    #[test]
    fn altitude_buckets_cover_the_ramp_stops() {
        // Boundaries in feet → metres: 2k, 10k, 28k, 40k.
        let ft_to_m = |ft: f64| ft / FT_PER_M;
        assert_eq!(
            AltitudeBucket::classify(false, true, ft_to_m(1_000.0)),
            AltitudeBucket::Below2000Ft
        );
        assert_eq!(
            AltitudeBucket::classify(false, true, ft_to_m(5_000.0)),
            AltitudeBucket::To10000Ft
        );
        assert_eq!(
            AltitudeBucket::classify(false, true, ft_to_m(20_000.0)),
            AltitudeBucket::To28000Ft
        );
        assert_eq!(
            AltitudeBucket::classify(false, true, ft_to_m(35_000.0)),
            AltitudeBucket::To40000Ft
        );
        assert_eq!(
            AltitudeBucket::classify(false, true, ft_to_m(41_000.0)),
            AltitudeBucket::Above40000Ft
        );
        // On the ground, or airborne with no altitude, is the neutral bucket.
        assert_eq!(
            AltitudeBucket::classify(true, true, ft_to_m(35_000.0)),
            AltitudeBucket::Ground
        );
        assert_eq!(
            AltitudeBucket::classify(false, false, 0.0),
            AltitudeBucket::Ground
        );
    }

    // ---- Pure helpers ---------------------------------------------------------------------------

    #[test]
    fn ease_out_is_zero_at_start_one_at_end_and_decelerating() {
        assert!((ease_out(0.0)).abs() < 1e-12);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-12);
        assert!(ease_out(0.5) > 0.5, "ease-out is above the diagonal midway");
        assert!((ease_out(-1.0)).abs() < 1e-12, "clamps below zero");
        assert!((ease_out(2.0) - 1.0).abs() < 1e-12, "clamps above one");
    }

    #[test]
    fn heading_blend_takes_the_short_way_across_north() {
        // 350° → 10° is +20° through 0°, so a quarter of the way is 355°, not ~5°.
        let h = blend_heading_deg(350.0, 10.0, 0.25);
        assert!((h - 355.0).abs() < 1e-9, "got {h}");
        // Endpoints exact.
        assert!((blend_heading_deg(350.0, 10.0, 0.0) - 350.0).abs() < 1e-9);
        assert!((blend_heading_deg(350.0, 10.0, 1.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn geodesic_lerp_hits_both_ends_and_the_midpoint() {
        let a = LatLon::new(0.0, 0.0);
        let b = LatLon::new(0.0, 10.0);
        assert!(haversine_distance_m(geodesic_lerp(a, b, 0.0), a) < 1e-6);
        assert!(haversine_distance_m(geodesic_lerp(a, b, 1.0), b) < 1e-3);
        let mid = geodesic_lerp(a, b, 0.5);
        assert!((mid.lon_deg - 5.0).abs() < 1e-6 && mid.lat_deg.abs() < 1e-9);
    }
}
