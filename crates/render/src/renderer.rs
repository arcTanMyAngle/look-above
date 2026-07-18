//! Device, swapchain, and the frame(s) M0/M2 know how to draw.

use std::sync::Arc;

use wgpu::util::DeviceExt as _;
use wgpu::{CurrentSurfaceTexture, DisplayAndWindowHandle};

use crate::basemap::{self, MeshData};
use crate::color;
use crate::error::RenderError;

/// docs/01's render-target sample count. Checked against the adapter's format features in
/// [`Renderer::new`] before the first MSAA texture is created — see
/// [`RenderError::UnsupportedMsaa`].
const SAMPLE_COUNT: u32 = 4;

/// The base-map shaders (M2 item 2.2b): one vertex entry point shared by both passes, one
/// fragment entry point whose output color comes from a per-pass `@group(1)` uniform rather
/// than being baked into the shader source (see `basemap.wgsl`'s module doc comment).
const BASEMAP_SHADER: &str = include_str!("shaders/basemap.wgsl");

/// What [`Renderer::render`] did with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// A frame was drawn and handed to the compositor.
    Presented,
    /// Nothing was drawn: the surface was busy, hidden, or stale. Not an error — the caller
    /// should carry on and try the next frame.
    Skipped,
}

/// One base-map layer's static GPU resources: its own geometry and its own flat color, drawn
/// by its own pipeline. The view-proj bind group (`@group(0)`) is shared across layers and
/// lives on [`Renderer`] directly — both layers share one camera.
#[derive(Debug)]
struct BasemapLayer {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    color_bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl BasemapLayer {
    /// Binds this layer's pipeline/color and draws its whole static mesh in one indexed call.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_bind_group(1, &self.color_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// Owns the GPU device and a window's swapchain, and paints the map.
///
/// M0 drew only the clear; M2 item 2.2b adds the base map (land fill, coastline stroke) as the
/// first two passes of docs/01's draw order ("map base → map lines → trails → aircraft →
/// labels → UI") — the rest of that order lands with 2.4+.
#[derive(Debug)]
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    clear_color: wgpu::Color,
    adapter_info: wgpu::AdapterInfo,
    /// The 4x-multisampled color target every pass renders into. `render` resolves it onto the
    /// swapchain view on submit. Recreated alongside the swapchain in [`Renderer::reconfigure`]
    /// — it must always match the surface size.
    msaa_view: wgpu::TextureView,
    /// The uniform both base-map pipelines read their `view_proj` matrix from (`@group(0)`).
    ///
    /// M2 2.2b's contents are a placeholder aspect-correcting fit-to-window matrix (see
    /// [`fit_to_window_matrix`]) — it exists only so a non-square window doesn't stretch the
    /// world, there is no pan/zoom yet. Rewritten in [`Renderer::reconfigure`] exactly the way
    /// `msaa_view` already is: same seam, same reason, it must always match the current surface
    /// size. M2 2.3 replaces the *contents* this buffer is written with (a real camera
    /// transform); the buffer and its bind group do not change.
    basemap_view_proj_buffer: wgpu::Buffer,
    basemap_view_proj_bind_group: wgpu::BindGroup,
    basemap_land: BasemapLayer,
    basemap_coastline: BasemapLayer,
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

        let basemap_resources =
            build_basemap_resources(&device, config.format, config.width, config.height);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            clear_color,
            adapter_info: adapter.get_info(),
            msaa_view,
            basemap_view_proj_buffer: basemap_resources.view_proj_buffer,
            basemap_view_proj_bind_group: basemap_resources.view_proj_bind_group,
            basemap_land: basemap_resources.land,
            basemap_coastline: basemap_resources.coastline,
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

    /// Draw and present one frame: the background clear, then the base map (land fill,
    /// coastline stroke) on top of it.
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
            // The pass renders into the 4x MSAA target and resolves onto the swapchain view on
            // submit — plumbing the aircraft, trail, and label passes 2.4+ hang off this same
            // attachment. The multisampled contents themselves are never read back, hence
            // `Discard`; only the resolved swapchain view needs to survive to present.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background + base map"),
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

            // docs/01 draw order: map base, then map lines, before anything else that exists
            // yet (trails/aircraft/labels are 2.4+).
            self.basemap_land
                .draw(&mut pass, &self.basemap_view_proj_bind_group);
            self.basemap_coastline
                .draw(&mut pass, &self.basemap_view_proj_bind_group);
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);

        if stale {
            self.reconfigure();
        }

        Ok(FrameOutcome::Presented)
    }

    /// Rebuild the swapchain from the current config, and everything tied to the surface size
    /// alongside it — the MSAA target and the base map's aspect-correcting view-proj matrix.
    fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.config);
        self.msaa_view = create_msaa_view(&self.device, &self.config);
        self.queue.write_buffer(
            &self.basemap_view_proj_buffer,
            0,
            bytemuck::bytes_of(&fit_to_window_matrix(self.config.width, self.config.height)),
        );
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

/// Everything [`Renderer::new`] needs out of base-map setup, bundled so it can be built in one
/// call before `Renderer`'s fields are assigned.
struct BasemapResources {
    view_proj_buffer: wgpu::Buffer,
    view_proj_bind_group: wgpu::BindGroup,
    land: BasemapLayer,
    coastline: BasemapLayer,
}

/// Tessellates the bundled base map and uploads it as static GPU buffers, builds the shared
/// view-proj uniform, and builds both layers' pipelines. Runs once, in [`Renderer::new`].
fn build_basemap_resources(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> BasemapResources {
    let geometry = basemap::tessellate();

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above basemap shader"),
        source: wgpu::ShaderSource::Wgsl(BASEMAP_SHADER.into()),
    });

    let view_proj_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::VERTEX,
        "look-above basemap view-proj bind group layout",
    );
    let color_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::FRAGMENT,
        "look-above basemap layer-color bind group layout",
    );

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above basemap pipeline layout"),
        bind_group_layouts: &[Some(&view_proj_layout), Some(&color_layout)],
        immediate_size: 0,
    });

    let view_proj_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above basemap view-proj uniform"),
        contents: bytemuck::bytes_of(&fit_to_window_matrix(width, height)),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let view_proj_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above basemap view-proj bind group"),
        layout: &view_proj_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: view_proj_buffer.as_entire_binding(),
        }],
    });

    // Two pipeline objects, per docs/01's draw order treating land fill and coastline stroke
    // as separate passes: both are `TriangleList` today (lyon's stroke tessellator already
    // emits triangles, not a `LineList` primitive), so they are identical apart from label and
    // bound resources for now, but kept separate so either can gain its own primitive/blend
    // state later without disturbing the other.
    let land_pipeline = create_basemap_pipeline(
        device,
        &shader,
        &pipeline_layout,
        format,
        "look-above basemap land fill pipeline",
    );
    let coastline_pipeline = create_basemap_pipeline(
        device,
        &shader,
        &pipeline_layout,
        format,
        "look-above basemap coastline stroke pipeline",
    );

    let land = BasemapLayer {
        vertex_buffer: create_mesh_buffer(
            device,
            &geometry.land,
            "look-above basemap land",
            BufferKind::Vertex,
        ),
        index_buffer: create_mesh_buffer(
            device,
            &geometry.land,
            "look-above basemap land",
            BufferKind::Index,
        ),
        index_count: index_count(&geometry.land),
        color_bind_group: create_color_bind_group(
            device,
            &color_layout,
            color::land_fill_color(format),
            "look-above basemap land color",
        ),
        pipeline: land_pipeline,
    };
    let coastline = BasemapLayer {
        vertex_buffer: create_mesh_buffer(
            device,
            &geometry.coastline,
            "look-above basemap coastline",
            BufferKind::Vertex,
        ),
        index_buffer: create_mesh_buffer(
            device,
            &geometry.coastline,
            "look-above basemap coastline",
            BufferKind::Index,
        ),
        index_count: index_count(&geometry.coastline),
        color_bind_group: create_color_bind_group(
            device,
            &color_layout,
            color::coastline_stroke_color(format),
            "look-above basemap coastline color",
        ),
        pipeline: coastline_pipeline,
    };

    BasemapResources {
        view_proj_buffer,
        view_proj_bind_group,
        land,
        coastline,
    }
}

/// Which of a mesh's two buffers [`create_mesh_buffer`] should build.
#[derive(Clone, Copy)]
enum BufferKind {
    Vertex,
    Index,
}

/// Uploads one half (vertex or index) of `mesh` as a static GPU buffer — created once here in
/// [`build_basemap_resources`], never rebuilt: base-map geometry does not change after startup.
fn create_mesh_buffer(
    device: &wgpu::Device,
    mesh: &MeshData,
    label: &str,
    kind: BufferKind,
) -> wgpu::Buffer {
    match kind {
        BufferKind::Vertex => device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        BufferKind::Index => device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
    }
}

/// `mesh`'s index count as a `u32` for `draw_indexed`'s range. `mesh.indices` is already
/// `Vec<u32>` (lyon's own output type — see `basemap::tessellate`), so this can only fail if a
/// mesh somehow held more than `u32::MAX` indices, many orders of magnitude past anything a
/// 1:50m-scale base map produces (2.2a's known counts are tens of thousands of points).
fn index_count(mesh: &MeshData) -> u32 {
    u32::try_from(mesh.indices.len()).expect("basemap mesh index count fits in u32")
}

/// One uniform-buffer bind group layout, parameterized by which shader stage reads it — the
/// view-proj matrix (`@group(0)`, vertex-only) and the per-layer color (`@group(1)`,
/// fragment-only) are otherwise the same shape.
fn create_uniform_bind_group_layout(
    device: &wgpu::Device,
    visibility: wgpu::ShaderStages,
    label: &str,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// One layer's fixed `@group(1)` color uniform and its bind group. Never rewritten after
/// creation — unlike the view-proj buffer, a layer's color does not change with the window.
fn create_color_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    color: [f32; 4],
    label: &str,
) -> wgpu::BindGroup {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::bytes_of(&color),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

/// Builds one of the two (currently identical) base-map render pipelines: `TriangleList`,
/// `SAMPLE_COUNT`-multisampled to match every other pass, no depth/stencil (there is none yet),
/// drawing into a surface of `format`.
fn create_basemap_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(basemap::Vertex::LAYOUT)],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: SAMPLE_COUNT,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// A placeholder aspect-correcting fit-to-window transform.
///
/// `basemap::tessellate`'s normalized coordinates are already close to `[-1, 1]` on both axes
/// *ignoring aspect* (see its doc comment) — this only needs to keep a non-square window from
/// stretching the world, by shrinking whichever axis has more pixels per world-unit. There is
/// no pan/zoom camera yet (M2 item 2.3); the buffer/bind-group plumbing this matrix travels
/// through does not change when 2.3 replaces what gets written here.
///
/// Column-major, matching WGSL's `mat4x4<f32>` convention — worth stating because this one
/// happens to be diagonal, where row- and column-major would look identical.
fn fit_to_window_matrix(width: u32, height: u32) -> [[f32; 4]; 4] {
    // Widths/heights are window dimensions in pixels — far below where `f32` would round an
    // integer of this magnitude (`2^24`), so the pedantic lint's concern does not apply here.
    #[allow(clippy::cast_precision_loss)]
    let aspect = width as f32 / (height.max(1) as f32);

    // Whichever axis has more pixels per world-unit gets scaled down so both axes end up with
    // the same pixels-per-world-unit — the classic "fit inside, do not stretch" correction.
    let (scale_x, scale_y) = if aspect >= 1.0 {
        (1.0 / aspect, 1.0)
    } else {
        (1.0, aspect)
    };

    [
        [scale_x, 0.0, 0.0, 0.0],
        [0.0, scale_y, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
