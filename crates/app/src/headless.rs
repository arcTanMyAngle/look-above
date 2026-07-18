//! Headless mode: `look-above --headless` runs the live ingest pipeline with no window,
//! logging each poll cycle's counts — the M1 gate's evidence tool (item 1.12; the gate run
//! itself is item 1.13).
//!
//! This is the first place the pieces built across M1 run together as one process rather than
//! in isolation under a test: [`Poller`] drives the failover chain into a `crossbeam` channel,
//! [`SessionTable`] dedups what arrives, and [`Writer`] persists each cycle's outcome to
//! `source_status` — closing the loop 1.11 left open (its own doc: "wiring the poller's
//! channel into a running `Writer` is out of scope here"). It also closes 1.7's ledger seam:
//! `source_status.credits_used_today`, read back through `Writer::source_status`, seeds the
//! primary's [`CreditLedger`] via [`CreditLedger::restored`] so a restart mid-day resumes the
//! day's spend instead of believing the whole budget is fresh.
//!
//! There is no graceful shutdown here — the gate run (item 1.13) is a supervised, timed
//! session ended by the operator, and the OS default `SIGINT`/`Ctrl+C` handling is what stops
//! it. Adding a shutdown protocol for a debug tool that is always operator-attended would be
//! scope the checklist item does not ask for.

use std::sync::Arc;

use anyhow::{Context, Result};
use crossbeam_channel::unbounded;
use look_above_core::contracts::RegionQuery;
use look_above_core::merge::{STALE_AFTER_S, SessionTable};
use look_above_core::types::{BBox, SourceId};
use look_above_ingest::budget::CreditLedger;
use look_above_ingest::http::HttpClient;
use look_above_ingest::opensky::OpenSkyAuth;
use look_above_ingest::poller::{PRIMARY, PollBatch, Poller, SystemWallClock, WallClock};
use look_above_store::Writer;

use crate::config::Config;

/// M1's fixed poll region, since regions are not yet camera-driven (M2/M4): a ~530×555 km box
/// centered on the Alps, big enough to match acceptance §M1's "~500×500 km bbox" credit-budget
/// check and to land `OpenSky`'s bbox pricing in its middle (25–100 deg²) tier rather than the
/// cheapest or the dearest — real area 35 deg². Also covers the exact area every adapter's own
/// live test has flown against since item 1.4, so a quiet sky here would be surprising.
const HEADLESS_REGION: (f64, f64, f64, f64) = (44.5, 4.5, 49.5, 11.5);

/// Runs the headless ingest loop until the process is killed. Never returns `Ok` on its own —
/// the poller runs forever, so the only way out is an error or the operator's `Ctrl+C`.
pub fn run(config: &Config) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("start the headless async runtime")?;

    let writer = Writer::open(&config.storage.db_path)
        .with_context(|| format!("open the store at {}", config.storage.db_path.display()))?;

    let client = HttpClient::new().context("build the shared HTTP client")?;
    let auth = OpenSkyAuth::from_optional(client.clone(), config.sources.opensky.credentials());
    let (lat_min, lon_min, lat_max, lon_max) = HEADLESS_REGION;
    let bbox = BBox::new(lat_min, lon_min, lat_max, lon_max)
        .expect("HEADLESS_REGION is a fixed, valid bbox");
    let query = RegionQuery::region(bbox);
    let clock: Arc<dyn WallClock> = Arc::new(SystemWallClock);

    let (sender, receiver) = unbounded();
    let mut poller = Poller::with_default_chain(client, auth, query, sender, Arc::clone(&clock));

    // Item 1.7's seam, closed here: seed the primary's ledger from what was already spent
    // today, so a restart does not get a full budget back for free. A read failure is not
    // fatal — it just means the ledger starts fresh, the same as a first-ever run.
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
        Ok(None) => tracing::info!("no persisted OpenSky source_status; starting the ledger fresh"),
        Err(error) => tracing::warn!(
            %error,
            "could not read OpenSky's source_status; starting the ledger fresh"
        ),
    }

    tracing::info!(
        bbox = ?HEADLESS_REGION,
        opensky_credentials = if config.sources.opensky.is_configured() {
            "configured"
        } else {
            "absent"
        },
        "headless mode: starting the poll loop"
    );
    runtime.spawn(poller.run());

    let mut table = SessionTable::new();
    for batch in receiver {
        record_cycle(&writer, &mut table, &batch);
    }

    tracing::warn!("poll loop stopped unexpectedly (channel closed); headless mode exiting");
    Ok(())
}

/// Merges one [`PollBatch`] into the session's live picture, logs the cycle's counts, and
/// persists the source's success against `source_status`.
///
/// A write failure is logged, not propagated — the pipeline itself is healthy (a batch just
/// arrived), and stopping the whole loop over one persistence hiccup would lose every cycle
/// after it, not just this one.
fn record_cycle(writer: &Writer, table: &mut SessionTable, batch: &PollBatch) {
    let stats = table.merge(&batch.states);
    let stale = table.stale_count(batch.fetched_at, STALE_AFTER_S);

    tracing::info!(
        source = %batch.source,
        new = stats.new,
        updated = stats.updated,
        dropped = stats.dropped,
        stale,
        tracked = table.len(),
        credits_spent = batch.credits_spent,
        spent_today = batch.spent_today,
        "poll cycle"
    );

    if let Err(error) = writer.record_success(batch.source, batch.fetched_at, batch.spent_today) {
        tracing::warn!(%error, source = %batch.source, "could not record source_status");
    }
}
