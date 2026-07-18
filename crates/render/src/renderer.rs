//! Device, swapchain, and the one frame M0 knows how to draw.

use std::sync::Arc;

use wgpu::{CurrentSurfaceTexture, DisplayAndWindowHandle};

use crate::color;
use crate::error::RenderError;

/// docs/01's render-target sample count. Checked against the adapter's format features in
/// [`Renderer::new`] before the first MSAA texture is created — see
/// [`RenderError::UnsupportedMsaa`].
const SAMPLE_COUNT: u32 = 4;

/// What [`Renderer::render`] did with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// A frame was drawn and handed to the compositor.
    Presented,
    /// Nothing was drawn: the surface was busy, hidden, or stale. Not an error — the caller
    /// should carry on and try the next frame.
    Skipped,
}

/// Owns the GPU device and a window's swapchain, and paints the map background.
///
/// M0 draws only the clear (the plan keeps `render` to surface-and-clear); M2 hangs the map,
/// aircraft, trail, and label passes off the same encoder.
#[derive(Debug)]
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    clear_color: wgpu::Color,
    adapter_info: wgpu::AdapterInfo,
    /// The 4x-multisampled color target every pass (from M2 2.2 on) renders into. `render`
    /// resolves it onto the swapchain view on submit. Recreated alongside the swapchain in
    /// [`Renderer::reconfigure`] — it must always match the surface size.
    msaa_view: wgpu::TextureView,
}

impl Renderer {
    /// Bring up a GPU device for `window` and configure its surface at `width`×`height`.
    ///
    /// Takes an [`Arc`] so the surface can borrow the window for as long as it lives, which
    /// is what lets this return an owning `Surface<'static>`. The bound is wgpu's, not
    /// winit's: this crate stays free of a windowing dependency.
    ///
    /// Blocking, despite wgpu's async setup calls — ADR-005 keeps async out of `core` and
    /// `render` entirely. On native the futures resolve without ever yielding, so driving
    /// them to completion here costs nothing and buys a runtime-free crate.
    pub fn new<W>(window: Arc<W>, width: u32, height: u32) -> Result<Self, RenderError>
    where
        W: DisplayAndWindowHandle + 'static,
    {
        let (_instance, surface, adapter) = Self::request_backend(window)?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("look-above device"),
                ..Default::default()
            }))?;

        // Zero-sized surfaces are invalid, and Windows reports (0, 0) for a minimized
        // window. Configure at least 1×1 and let `resize` hold that floor.
        let config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| RenderError::UnsupportedSurface {
                adapter: adapter.get_info().name.clone(),
            })?;

        // docs/01 requires 4x MSAA on every pass from 2.2 on. Fail here, with the adapter
        // name in hand, rather than let a software/CI adapter panic the first time a pass
        // tries to create the render target.
        let msaa_features = adapter.get_texture_format_features(config.format).flags;
        if !msaa_features.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
            || !msaa_features.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        {
            return Err(RenderError::UnsupportedMsaa {
                adapter: adapter.get_info().name.clone(),
                format: config.format,
            });
        }

        surface.configure(&device, &config);

        let clear_color = color::clear_color(config.format);
        let msaa_view = create_msaa_view(&device, &config);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            clear_color,
            adapter_info: adapter.get_info(),
            msaa_view,
        })
    }

    /// Build the instance/surface/adapter trio, preferring DX12 on Windows.
    ///
    /// `WGPU_BACKEND` (see [`wgpu::Backends::from_env`]) is the documented way to bisect a
    /// backend bug (M0 item 0.6's decision log entry) and always wins: the DX12 preference
    /// below only kicks in when the caller has left it unset. If DX12 itself yields no
    /// adapter, this falls back to wgpu's normal multi-backend selection — same as everywhere
    /// that isn't Windows.
    fn request_backend<W>(
        window: Arc<W>,
    ) -> Result<(wgpu::Instance, wgpu::Surface<'static>, wgpu::Adapter), RenderError>
    where
        W: DisplayAndWindowHandle + 'static,
    {
        let backend_pinned_by_env = wgpu::Backends::from_env().is_some();

        if cfg!(windows) && !backend_pinned_by_env {
            // `..from_env()` still picks up `WGPU_DEBUG`/`WGPU_VALIDATION`/etc.; only the
            // backend set itself is forced here, and only because we already checked above
            // that the env var didn't ask for one.
            let dx12_only = wgpu::InstanceDescriptor {
                backends: wgpu::Backends::DX12,
                ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
            };
            match Self::try_backend(Arc::clone(&window), dx12_only) {
                Ok(found) => return Ok(found),
                Err(error) => tracing::info!(
                    %error,
                    "DX12 adapter unavailable, falling back to wgpu's default backend selection"
                ),
            }
        }

        Self::try_backend(
            window,
            wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
        )
    }

    /// One instance/surface/adapter attempt for a given [`wgpu::InstanceDescriptor`].
    ///
    /// No display handle: it is unused on this project's backends (DX12/Vulkan), and leaving
    /// it `None` is what lets `create_surface` take the window's own.
    fn try_backend<W>(
        window: Arc<W>,
        descriptor: wgpu::InstanceDescriptor,
    ) -> Result<(wgpu::Instance, wgpu::Surface<'static>, wgpu::Adapter), RenderError>
    where
        W: DisplayAndWindowHandle + 'static,
    {
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance.create_surface(window)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // Prefer the discrete GPU where there is one. On integrated-only machines —
            // the frame budget in docs/01 assumes one — this falls back to what exists.
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        Ok((instance, surface, adapter))
    }

    /// Which GPU this renderer ended up on, and through which backend.
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    /// The surface texture format the swapchain settled on.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Follow the window to a new size.
    ///
    /// A zero-sized window (minimized, on Windows) is ignored rather than configured: the
    /// old swapchain stays valid and unused until the window comes back, and `render` skips
    /// the frames in between.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (width, height) == (self.config.width, self.config.height) {
            return;
        }

        self.config.width = width;
        self.config.height = height;
        self.reconfigure();
    }

    /// Draw and present one frame: the background clear, and nothing else yet.
    pub fn render(&mut self) -> Result<FrameOutcome, RenderError> {
        let (frame, stale) = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => (frame, false),
            // Usable, but the swapchain no longer matches the window. Draw it anyway —
            // one imperfect frame beats a dropped one — and reconfigure after presenting,
            // because `configure` panics while a surface texture is still alive.
            CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            // Nothing to draw into yet, or nobody would see it.
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return Ok(FrameOutcome::Skipped);
            }
            // Stale configuration, normally a resize we have not been told about.
            CurrentSurfaceTexture::Outdated => {
                self.reconfigure();
                return Ok(FrameOutcome::Skipped);
            }
            CurrentSurfaceTexture::Lost => return Err(RenderError::SurfaceLost),
            CurrentSurfaceTexture::Validation => return Err(RenderError::SurfaceValidation),
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("look-above frame"),
            });

        {
            // The clear is the whole frame in M0/2.1. The pass renders into the 4x MSAA
            // target and resolves onto the swapchain view on submit — plumbing for the map,
            // aircraft, trail, and label passes 2.2+ hangs off this same attachment. The
            // multisampled contents themselves are never read back, hence `Discard`; only the
            // resolved swapchain view needs to survive to present.
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);

        if stale {
            self.reconfigure();
        }

        Ok(FrameOutcome::Presented)
    }

    /// Rebuild the swapchain from the current config, and the MSAA target alongside it — it
    /// must always match the surface size.
    fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.config);
        self.msaa_view = create_msaa_view(&self.device, &self.config);
    }
}

/// Build the multisampled color target `render` draws into for one swapchain configuration.
fn create_msaa_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("look-above msaa color target"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
