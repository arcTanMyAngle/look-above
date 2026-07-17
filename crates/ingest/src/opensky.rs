//! The `OpenSky` Network source — the primary live-position provider.
//!
//! `OpenSky` is the only authorized source that needs an account: a free one carries 4,000
//! credits/day, and a `/states/all` bbox query costs 1–4 of them by area. That budget is
//! what makes it the *primary* rather than the only source — the community fallbacks
//! (airplanes.live, adsb.lol) need no key and carry no ledger.
//!
//! [`auth`] is the `OAuth2` half (item 1.3); [`states`] is the `/states/all` adapter and the
//! credit cost function (item 1.4). They are separate modules because they fail differently:
//! a token endpoint blip is survivable inside the refresh slack, where a bad bbox is not.

pub mod auth;
pub mod states;

pub use auth::{Credentials, OpenSkyAuth};
pub use states::{OpenSkySource, credit_cost};
