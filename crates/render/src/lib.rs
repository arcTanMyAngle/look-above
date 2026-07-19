//! GPU rendering for Look Above: wgpu pipelines, WGSL shaders, camera.
//!
//! Kept minimal in M0 (surface + clear only); the real pipeline is M2.
//! No network and no DB access from this crate.

mod aircraft;
mod basemap;
mod color;
mod error;
mod glyph_atlas;
mod label;
mod label_atlas;
mod renderer;
mod trail;

pub use error::RenderError;
pub use renderer::{FrameOutcome, Renderer, camera_view_proj};
