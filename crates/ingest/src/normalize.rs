//! Cross-adapter helpers for values coming off the wire.
//!
//! Every adapter converts wire values into `StateVector` fields, and two of those
//! conversions are identical everywhere: refusing a coordinate that is not on the globe,
//! and narrowing a source-reported `f64` to the `f32` the pipeline stores. They live here
//! so `opensky::states` and [`readsb`](crate::readsb) share one implementation of each
//! rather than growing two that can drift.

/// Accepts a coordinate only if it is finite and within `±limit` degrees.
///
/// `BBox` validates its own corners, but these come off the wire: `Web Mercator` of latitude
/// 91 is not an error, it is a plausible-looking point in the wrong place, and NaN would
/// propagate through the projection into the vertex buffer. Checked here, once, where the
/// value enters.
pub(crate) fn coordinate(value: f64, limit: f64) -> Option<f64> {
    (value.is_finite() && value.abs() <= limit).then_some(value)
}

/// Narrows a source-reported `f64` to the `f32` the pipeline stores.
///
/// Lossy in the last decimal places and deliberately so: these are metres, metres per second,
/// and degrees derived from a radio broadcast, and `f32` carries about seven significant
/// digits — more than the measurement has. `StateVector` chose `f32` for the GPU's sake
/// (docs/09); this is where that choice is paid.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn narrow(value: f64) -> f32 {
    value as f32
}
