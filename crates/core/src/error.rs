//! Error taxonomies for the crate seams.
//!
//! Both enums are deliberately backend-agnostic: `core` depends on neither
//! `reqwest` nor `rusqlite`, so the implementing crates map their library errors
//! into these variants. The taxonomy exists to be *branched on* — variants are
//! distinguished by what the caller must do about them, not by where they arose.

use std::time::Duration;

use thiserror::Error;

/// A failure from a [`LiveSource`](crate::contracts::LiveSource) fetch.
///
/// The poller's backoff logic branches on these (docs/09): `Parse` never kills
/// the poller — it logs and skips the offending record — whereas `Auth` means
/// stop and refresh credentials.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceError {
    /// Credentials rejected or expired (HTTP 401/403). Refresh the token; do not retry blindly.
    #[error("authentication failed: {message}")]
    Auth { message: String },

    /// HTTP 429. Honor `retry_after` when the response carried a `Retry-After` header,
    /// otherwise fall back to exponential backoff with jitter.
    #[error("rate limited by source")]
    RateLimited { retry_after: Option<Duration> },

    /// Transport-level failure: connection refused, TLS failure, timeout.
    #[error("network error: {message}")]
    Network { message: String },

    /// The response was received but could not be understood. Non-fatal by contract.
    #[error("failed to parse response: {message}")]
    Parse { message: String },

    /// HTTP 5xx — upstream is unhealthy; back off and consider failing over.
    #[error("upstream server error: HTTP {status}")]
    Server { status: u16 },
}

impl SourceError {
    /// Whether retrying the same request later could plausibly succeed.
    ///
    /// `Auth` is excluded: retrying with the same rejected credentials only burns
    /// budget. `Parse` is excluded: the bytes will not change on a re-fetch.
    pub const fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Network { .. } | Self::Server { .. }
        )
    }
}

/// A failure from a [`Store`](crate::contracts::Store) operation.
///
/// `crates/store` maps `rusqlite::Error` into these; `core` stays SQLite-free.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StoreError {
    /// The database engine rejected the operation (I/O, locking, constraint).
    #[error("database error: {message}")]
    Backend { message: String },

    /// A numbered migration failed to apply — startup cannot continue (docs/08).
    #[error("migration {version} failed: {message}")]
    Migration { version: u32, message: String },

    /// A stored row could not be read back into its domain type.
    #[error("stored data is invalid: {message}")]
    Corrupt { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_retryable_failures_are_transient() {
        assert!(SourceError::RateLimited { retry_after: None }.is_transient());
        assert!(
            SourceError::Network {
                message: "timed out".to_owned()
            }
            .is_transient()
        );
        assert!(SourceError::Server { status: 503 }.is_transient());

        assert!(
            !SourceError::Auth {
                message: "token expired".to_owned()
            }
            .is_transient()
        );
        assert!(
            !SourceError::Parse {
                message: "states[3] was not an array".to_owned()
            }
            .is_transient()
        );
    }
}
