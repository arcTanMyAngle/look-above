//! Live data ingestion for Look Above: `LiveSource` adapters, pollers, and
//! rate/credit budgeting.
//!
//! Every host this crate may contact is fixed by the authorized-aviation-sources skill;
//! [`http`] carries the cross-cutting client rules (User-Agent, timeout, backoff, error
//! mapping) that docs/09 requires of all of them.

pub mod http;
