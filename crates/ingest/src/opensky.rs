//! The `OpenSky` Network source — the primary live-position provider.
//!
//! `OpenSky` is the only authorized source that needs an account: a free one carries 4,000
//! credits/day, and a `/states/all` bbox query costs 1–4 of them by area. That budget is
//! what makes it the *primary* rather than the only source — the community fallbacks
//! (airplanes.live, adsb.lol) need no key and carry no ledger.
//!
//! [`auth`] is the `OAuth2` half (item 1.3). The `/states/all` adapter and its credit cost
//! function land in item 1.4.

pub mod auth;

pub use auth::{Credentials, OpenSkyAuth};
