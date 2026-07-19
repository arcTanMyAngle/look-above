//! The one merge/log/persist step both ingest-driving modes share.
//!
//! [`crate::headless`] (a fixed region, driven by a blocking `for batch in receiver` loop) and
//! [`crate::window`] (a camera-driven region, draining non-blockingly once per frame) differ in
//! *how* a [`PollBatch`] arrives, but not in what happens to it once it does: merge into the
//! session's live picture, log the cycle's counts, and persist the source's success against
//! `source_status`. [`record_cycle`] is that shared body, extracted so M2 item 2.3b's window
//! mode does not duplicate item 1.12's headless one.

use look_above_core::merge::{STALE_AFTER_S, SessionTable};
use look_above_ingest::poller::PollBatch;
use look_above_store::Writer;

/// Merges one [`PollBatch`] into the session's live picture, logs the cycle's counts, and
/// persists the source's success against `source_status`.
///
/// A write failure is logged, not propagated — the pipeline itself is healthy (a batch just
/// arrived), and stopping the whole loop over one persistence hiccup would lose every cycle
/// after it, not just this one.
pub(crate) fn record_cycle(writer: &Writer, table: &mut SessionTable, batch: &PollBatch) {
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
