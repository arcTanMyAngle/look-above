//! Maps `rusqlite::Error` onto the backend-agnostic [`StoreError`] taxonomy (docs/09): `core`
//! depends on neither `rusqlite` nor any other DB crate, so this is the one seam that
//! translates the library's own error type into ours — mirroring how `ingest` maps
//! `reqwest`/`SourceError` in `ingest::http`.

use look_above_core::error::StoreError;

/// A `rusqlite::Error` that reached an ordinary data-access path.
///
/// Takes `error` by reference — `Display` only needs `&self`, and every call site still holds
/// its `rusqlite::Result` at the point it maps the error, so there is nothing to consume.
/// Migration failures get their own `StoreError::Migration` variant (carrying the version
/// that failed), built at the call site inside `migrations::apply`; this is for every other
/// query the writer thread runs.
pub(crate) fn backend_error(error: &rusqlite::Error) -> StoreError {
    StoreError::Backend {
        message: error.to_string(),
    }
}
