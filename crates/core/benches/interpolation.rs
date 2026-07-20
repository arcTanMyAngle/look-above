//! Gate benchmarks for M2 item 2.10 (docs/10 §5, "interpolation benchmarks"): the two
//! `core::sim` budgets that scope to M2 itself — `Simulator::advance_all` for 10k aircraft
//! (< 2 ms on 8 cores) and a Web Mercator projection batch over 10k points (< 0.5 ms). Both are
//! the same hot loop in production (`Track::advance` projects as part of dead-reckoning, per
//! its own doc comment — "the projection is batched on the same worker pass as the
//! interpolation"), so the projection bench below exercises the pure primitive
//! (`web_mercator_forward`) standalone under the same `rayon` parallelism, rather than adding a
//! `project_batch` production API with no real caller (the exact trap `DECISION_LOG` flagged at
//! M0: "a parallel batch API with no caller is a guess at the call shape").
//!
//! docs/10 §5's third line, "store insert 10k positions", is **not** benched here: `core::sim`
//! has nothing to do with persistence, and `look-above-store` has no `positions` table yet — the
//! `source_status` schema is all M1 built. Position storage is M5's own deliverable (docs/07);
//! docs/11's M2 gate line reads "**interpolation** benchmarks", not "all of docs/10 §5", so that
//! third bullet is out of scope until M5 gives it a schema to insert into. Recorded in
//! `DECISION_LOG` rather than silently skipped.
//!
//! `cargo bench -p look-above-core` (gate-time only, per docs/10 §5 — not run in CI-per-push).
//! Report the wall-clock number at the gate; regressions > 20% at a milestone gate block it.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use look_above_core::geo::{LatLon, web_mercator_forward};
use look_above_core::sim::Simulator;
use look_above_core::types::{CallSign, Icao24, SourceId, StateVector, UnixSeconds};
use rayon::prelude::*;
use std::hint::black_box;

const AIRCRAFT_COUNT: usize = 10_000;

/// A tiny deterministic PRNG (splitmix64), same technique `render`'s own headless smoke-test
/// fixture (M2 item 2.9) uses — no `rand` dependency anywhere in this workspace, and a
/// benchmark fixture needs a reproducible spread, not statistically rigorous randomness.
struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "both operands are exact integers within f64's 53-bit mantissa range"
    )]
    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn next_range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.next_unit() * (hi - lo)
    }
}

/// Narrows a synthetic `f64` sample (already within `f32`'s representable range — altitudes,
/// speeds, headings) down to the `f32` `StateVector` optional fields want.
#[allow(
    clippy::cast_possible_truncation,
    reason = "synthetic benchmark data in plausible aviation ranges, not a real measurement"
)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

/// `AIRCRAFT_COUNT` synthetic, fixed-seed `StateVector`s spread over a plausible lat/lon range
/// (comfortably clear of the poles/antimeridian — those edge cases are `core::geo`'s own unit
/// tests' job, not this fixture's).
fn synthetic_states() -> Vec<StateVector> {
    let mut rng = Lcg(0xB10C_C0DE_5EED_0002);
    (0..AIRCRAFT_COUNT)
        .map(|i| {
            let icao24 = Icao24::from_hex(&format!("{i:06x}")).expect("valid synthetic ICAO24");
            StateVector {
                icao24,
                callsign: CallSign::new(&format!("BNC{i:04}")),
                ts: UnixSeconds(1_700_000_000),
                lat_deg: rng.next_range(-60.0, 60.0),
                lon_deg: rng.next_range(-170.0, 170.0),
                baro_alt_m: Some(to_f32(rng.next_range(0.0, 12_500.0))),
                velocity_ms: Some(to_f32(rng.next_range(20.0, 260.0))),
                heading_deg: Some(to_f32(rng.next_range(0.0, 360.0))),
                vert_rate_ms: Some(to_f32(rng.next_range(-15.0, 15.0))),
                on_ground: false,
                anonymous: false,
                source: SourceId::OpenSky,
            }
        })
        .collect()
}

/// docs/10 §5: "`sim::advance_all` for 10k aircraft — budget: < 2 ms on 8 cores (rayon)".
///
/// `iter_batched` rebuilds a fresh, freshly-ingested `Simulator` per sample (excluded from the
/// timed region) rather than reusing one across iterations: repeatedly advancing the same
/// `now_s` a fixed 1 s tick keeps every track's blend/fade state realistic frame-to-frame without
/// ever running a track past `DROP_AFTER_S` and dropping it mid-benchmark, which reusing a
/// single long-lived `Simulator` across thousands of samples risked.
fn bench_advance_all(c: &mut Criterion) {
    let states = synthetic_states();
    let now_s = 1_700_000_000.0_f64;

    c.bench_with_input(
        BenchmarkId::new("sim_advance_all", AIRCRAFT_COUNT),
        &now_s,
        |b, &now_s| {
            b.iter_batched(
                || {
                    let mut simulator = Simulator::new();
                    simulator.ingest(&states, now_s);
                    simulator
                },
                |mut simulator| black_box(simulator.advance_all(now_s + 1.0)),
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

/// docs/10 §5: "Projection batch 10k points — budget: < 0.5 ms". Benches the pure
/// `web_mercator_forward` primitive under the same `rayon` parallelism `Simulator::advance_all`
/// already applies it with in production, rather than a standalone `project_batch` API this
/// workspace has no other caller for (see this file's module doc).
fn bench_projection_batch(c: &mut Criterion) {
    let mut rng = Lcg(0xB10C_C0DE_5EED_0003);
    let points: Vec<LatLon> = (0..AIRCRAFT_COUNT)
        .map(|_| LatLon::new(rng.next_range(-60.0, 60.0), rng.next_range(-170.0, 170.0)))
        .collect();

    c.bench_with_input(
        BenchmarkId::new("web_mercator_forward_batch", AIRCRAFT_COUNT),
        &points,
        |b, points| {
            b.iter(|| {
                let projected: Vec<_> = points
                    .par_iter()
                    .map(|&point| web_mercator_forward(point))
                    .collect();
                black_box(projected)
            });
        },
    );
}

criterion_group!(benches, bench_advance_all, bench_projection_batch);
criterion_main!(benches);
