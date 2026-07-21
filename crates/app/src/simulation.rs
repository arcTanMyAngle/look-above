//! The simulation worker thread: poll batches in, an interpolated [`RenderFeed`] out.
//!
//! ADR-002 and the high-fidelity-flight-visualization skill put *all* simulation,
//! interpolation, and projection on worker threads: "the render thread never computes any of
//! the above". M2 item 2.4a built the pure engine ([`core::sim`](look_above_core::sim)); this
//! is the thread that runs it in window mode. It owns the pieces that used to live on the render
//! thread's frame path (the [`SessionTable`], the [`Writer`], the poll-batch receiver) so that
//! path is left to do only what ADR-002 allows: swap the double buffer, and draw.
//!
//! Each iteration, at render cadence (~60 Hz):
//!
//! 1. Drain every poll batch that has arrived and [`record_cycle`] it (merge into the session
//!    picture, log the cycle, persist the source's success) — the exact step headless mode runs
//!    on its own blocking receiver loop.
//! 2. If any batch arrived, install the current picture into the [`Simulator`]. A fix not newer
//!    than the one already held is ignored by [`Simulator::ingest`], so re-feeding the whole
//!    table every cycle is safe: only the aircraft this cycle actually refreshed start a new
//!    correction blend.
//! 3. Apply `app::window`'s latest click selection ([`Simulator::set_selected`], M2 item 2.8a) —
//!    read fresh off a `watch` channel every iteration rather than edge-detected, since re-setting
//!    an unchanged `Option<Icao24>` is free and selection changes at click cadence.
//! 4. Drain every METAR batch that has arrived (M3 item 3.3) and persist it to the store —
//!    unlike position batches, there is nothing to merge into the [`Simulator`]: badges are read
//!    straight back out of the store on the camera-settle cadence `app::window` already drives
//!    `current_airports`/`current_runways` from, not carried through the render feed.
//! 5. Advance every track to now and publish the resulting feed to the render thread through the
//!    [`crate::double_buffer`].
//!
//! Stale tracks are evicted from the table (at [`DROP_AFTER_S`], the same horizon the simulator
//! forgets a track at) before each ingest, so the picture fed to the simulator stays bounded and
//! a long-gone aircraft is not re-created from a frozen entry. This eviction is window mode's
//! alone — headless deliberately keeps its stale count meaningful (item 1.12), so it is not
//! folded into the shared [`record_cycle`].

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use look_above_core::merge::{DROP_AFTER_S, SessionTable};
use look_above_core::sim::{RenderFeed, Simulator};
use look_above_core::types::{Icao24, StateVector, UnixSeconds};
use look_above_ingest::metar::MetarBatch;
use look_above_ingest::poller::PollBatch;
use look_above_store::Writer;
use tokio::sync::watch;

use crate::double_buffer::Producer;
use crate::pipeline::record_cycle;

/// Target wall-clock time per simulation iteration — docs/01's ~60 fps render cadence. The
/// producer paces itself to this so a quiet sky does not spin a core; the consumer (render
/// thread) reads whatever the latest published feed is, independent of this rate.
const FRAME_BUDGET: Duration = Duration::from_micros(16_667);

/// Spawns the simulation worker, moving the ingest-side pieces onto it. Returns the join handle
/// so [`crate::window`] can stop the thread cleanly (via `shutdown`) and join it on exit.
///
/// Fails only if the OS refuses a new thread (resource exhaustion), which [`crate::window`]
/// surfaces the same way a renderer-init failure is — fatal, not a silent degrade.
#[allow(
    clippy::too_many_arguments,
    reason = "this is the one place every piece the worker owns (simulator, table, writer, both \
              channel receivers, the feed producer, selection, shutdown) converges to be handed \
              off; a params struct would not reduce what the caller has to assemble, just move \
              it behind one more layer of indirection — the same reasoning `renderer::record_draw_passes` \
              gives for its own too_many_arguments"
)]
pub fn spawn(
    simulator: Simulator,
    table: SessionTable,
    writer: Writer,
    batch_rx: Receiver<PollBatch>,
    metar_rx: Receiver<MetarBatch>,
    producer: Producer<RenderFeed>,
    select_rx: watch::Receiver<Option<Icao24>>,
    shutdown: Arc<AtomicBool>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("look-above-sim".to_owned())
        // The closure owns `writer`, so the store's writer thread is torn down exactly when this
        // worker ends; `run` only needs to borrow it.
        .spawn(move || {
            run(
                simulator, table, &writer, &batch_rx, &metar_rx, &producer, &select_rx, &shutdown,
            );
        })
}

/// The worker loop — see the module doc for the per-iteration steps. Returns when `shutdown` is
/// set (checked once per iteration, so the thread stops within one frame of being told to).
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors spawn's own too_many_arguments reason: this is `spawn`'s closure body, \
              taking by-reference exactly what it was handed by value"
)]
fn run(
    mut simulator: Simulator,
    mut table: SessionTable,
    writer: &Writer,
    batch_rx: &Receiver<PollBatch>,
    metar_rx: &Receiver<MetarBatch>,
    producer: &Producer<RenderFeed>,
    select_rx: &watch::Receiver<Option<Icao24>>,
    shutdown: &AtomicBool,
) {
    while !shutdown.load(Ordering::Relaxed) {
        let started = Instant::now();
        let (now_s, now_unix) = wall_clock_now();

        if drain_and_merge(batch_rx, writer, &mut table) {
            sync_simulator(&mut simulator, &mut table, now_s, now_unix);
        }
        drain_and_store_metars(metar_rx, writer);

        // Cheap (an `Option<Icao24>` `Copy`) to re-apply every iteration rather than only on
        // change — simpler than edge-detecting a watch channel, and selection changes at click
        // cadence, nowhere near a cost worth avoiding at ~60 Hz (M2 item 2.8a).
        simulator.set_selected(*select_rx.borrow());

        producer.publish(simulator.advance_all(now_s));

        if let Some(remaining) = FRAME_BUDGET.checked_sub(started.elapsed()) {
            thread::sleep(remaining);
        }
    }
}

/// Merges every poll batch waiting on the channel into the session picture, returning whether
/// at least one arrived (so the caller only re-syncs the simulator when there is new data).
fn drain_and_merge(
    batch_rx: &Receiver<PollBatch>,
    writer: &Writer,
    table: &mut SessionTable,
) -> bool {
    let mut merged = false;
    for batch in batch_rx.try_iter() {
        record_cycle(writer, table, &batch);
        merged = true;
    }
    merged
}

/// Persists every METAR batch waiting on the channel (M3 item 3.3) — there is no session table
/// to merge into (unlike [`drain_and_merge`]'s positions): a badge is read straight back out of
/// the store on `app::window`'s own settle cadence, so this only needs to get each batch into
/// `metars` before that next read. A write failure is logged, not propagated — the same
/// "the pipeline itself is healthy" reasoning `pipeline::record_cycle` already documents for
/// `source_status`.
fn drain_and_store_metars(metar_rx: &Receiver<MetarBatch>, writer: &Writer) {
    for batch in metar_rx.try_iter() {
        let count = batch.metars.len();
        match writer.upsert_metars(batch.metars) {
            Ok(()) => tracing::debug!(count, "metar batch persisted"),
            Err(error) => tracing::warn!(%error, "could not persist a metar batch"),
        }
    }
}

/// Bounds the table to still-live tracks, then installs the current picture into the simulator.
///
/// Split from the loop so it is testable without a store or a channel. Eviction uses
/// [`DROP_AFTER_S`] — past that the simulator would forget the track anyway, so a frozen entry
/// that old only risks re-creating a track that is immediately dropped again.
fn sync_simulator(
    simulator: &mut Simulator,
    table: &mut SessionTable,
    now_s: f64,
    now_unix: UnixSeconds,
) {
    table.evict_stale(now_unix, DROP_AFTER_S);
    let states: Vec<StateVector> = table.states().cloned().collect();
    simulator.ingest(&states, now_s);
}

/// The current wall-clock time, as both fractional seconds (for the simulator, whose motion is
/// sub-second) and whole [`UnixSeconds`] (for the table's staleness horizon, which is integer).
/// A clock set before the Unix epoch — not a real condition on a tracking machine — reads as the
/// epoch rather than panicking; everything then looks maximally stale and self-heals once the
/// clock is sane.
fn wall_clock_now() -> (f64, UnixSeconds) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => (
            elapsed.as_secs_f64(),
            UnixSeconds(i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)),
        ),
        Err(_) => (0.0, UnixSeconds(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use look_above_core::types::{Icao24, SourceId};

    fn hex(s: &str) -> Icao24 {
        Icao24::from_hex(s).expect("valid ICAO24 in test")
    }

    /// A live state for `icao` at time `ts`, flying east at 200 m/s over a benign position.
    fn state(icao: &str, ts: i64) -> StateVector {
        StateVector {
            icao24: hex(icao),
            callsign: None,
            ts: UnixSeconds(ts),
            lat_deg: 47.0,
            lon_deg: 8.0,
            baro_alt_m: Some(10_000.0),
            velocity_ms: Some(200.0),
            heading_deg: Some(90.0),
            vert_rate_ms: Some(0.0),
            on_ground: false,
            anonymous: false,
            source: SourceId::OpenSky,
        }
    }

    #[test]
    fn the_feed_instance_count_tracks_the_live_table() {
        // The item's verification, at unit level: the number of drawn instances equals the
        // number of live (fed) aircraft.
        let mut simulator = Simulator::new();
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", 1_000), state("4b1815", 1_000)]);

        sync_simulator(&mut simulator, &mut table, 1_000.0, UnixSeconds(1_000));
        let feed = simulator.advance_all(1_000.0);

        assert_eq!(feed.aircraft.len(), 2);
    }

    #[test]
    fn a_dropped_out_aircraft_is_evicted_and_leaves_the_feed() {
        let mut simulator = Simulator::new();
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", 1_000)]);

        // Now is 1,000 s on — far past `DROP_AFTER_S` (90 s) for that fix — so the table entry
        // is evicted and the simulator has nothing live to draw.
        let now_s = 2_000.0;
        let now_unix = UnixSeconds(2_000);
        sync_simulator(&mut simulator, &mut table, now_s, now_unix);

        assert!(
            table.is_empty(),
            "the stale entry was evicted from the table"
        );
        assert!(
            simulator.advance_all(now_s).aircraft.is_empty(),
            "and nothing stale is drawn"
        );
    }

    #[test]
    fn re_syncing_the_same_table_does_not_restart_a_blend() {
        // A poll cycle that re-sends an unchanged fix (the SessionTable holds the same `ts`)
        // must not perturb the shown position — the sim's own older-or-equal guard, exercised
        // through this wiring rather than directly.
        let mut simulator = Simulator::new();
        let mut table = SessionTable::new();
        table.merge(&[state("3c6444", 1_000)]);

        sync_simulator(&mut simulator, &mut table, 1_000.0, UnixSeconds(1_000));
        let first = simulator.advance_all(1_010.0).aircraft[0].position;

        // Same table, a cycle later: re-synced, then advanced to the same instant.
        sync_simulator(&mut simulator, &mut table, 1_010.0, UnixSeconds(1_010));
        let after = simulator.advance_all(1_010.0).aircraft[0].position;

        assert!((first.x_m - after.x_m).abs() < 1e-6 && (first.y_m - after.y_m).abs() < 1e-6);
    }

    // The `as i64` truncation is exactly what is under test (the two views must agree on the
    // whole second); the value is ~1.7e9, far inside i64, so the lint's overflow concern is moot.
    #[allow(clippy::cast_possible_truncation)]
    #[test]
    fn wall_clock_now_agrees_across_its_two_representations() {
        let (secs_f64, unix) = wall_clock_now();
        // The whole-second view is the floor of the fractional one (never a rounded-up second).
        assert_eq!(unix.0, secs_f64.trunc() as i64);
        assert!(unix.0 > 1_700_000_000, "a plausibly-current epoch second");
    }
}
