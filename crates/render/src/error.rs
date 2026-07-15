//! What can go wrong bringing up or driving the GPU surface.

use thiserror::Error;

/// A failure in renderer setup or in drawing a frame.
///
/// Transient surface conditions (a frame that timed out, an occluded window) are not errors
/// — they are [`FrameOutcome::Skipped`](crate::FrameOutcome::Skipped). What lands here is
/// either fatal or needs the caller to rebuild something.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RenderError {
    /// The window handle could not be turned into a drawable surface.
    #[error("could not create a GPU surface for the window")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),

    /// No adapter at all — no GPU driver, or none that can present to this window.
    #[error("no GPU adapter can present to this window")]
    NoAdapter(#[from] wgpu::RequestAdapterError),

    /// An adapter exists but would not yield a device.
    #[error("GPU adapter provided no usable device")]
    NoDevice(#[from] wgpu::RequestDeviceError),

    /// The adapter and surface are individually fine but share no format or present mode.
    #[error("GPU adapter '{adapter}' supports no usable configuration for this window")]
    UnsupportedSurface {
        /// The adapter that came up short, for the bug report.
        adapter: String,
    },

    /// The surface died under us — a GPU reset, or the display went away. Recovering means
    /// recreating the surface, which the renderer cannot do without the window.
    #[error("the GPU surface was lost")]
    SurfaceLost,

    /// `get_current_texture` raised a validation error, which is a bug on our side.
    #[error("the GPU surface reported a validation error")]
    SurfaceValidation,
}
