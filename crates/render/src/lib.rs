//! GPU rendering for Look Above: wgpu pipelines, WGSL shaders, camera.
//!
//! Kept minimal in M0 (surface + clear only); the real pipeline is M2.
//! No network and no DB access from this crate.

mod aircraft;
mod airport;
mod basemap;
mod color;
mod error;
mod glyph_atlas;
mod info_card;
mod label;
mod label_atlas;
mod metar_badge;
mod renderer;
mod selection;
mod stats_overlay;
mod trail;

pub use error::RenderError;
pub use info_card::InfoCardContent;
pub use renderer::{FrameOutcome, Renderer, camera_view_proj};
pub use selection::hit_test;
pub use stats_overlay::StatsOverlay;
