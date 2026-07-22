//! Device, swapchain, and the frame(s) M0/M2 know how to draw.

use std::collections::HashSet;
use std::mem::size_of;
use std::sync::Arc;

use look_above_core::camera::Camera;
use look_above_core::contracts::{Airport, MetarBadge, Runway};
use look_above_core::geo::{LatLon, WEB_MERCATOR_EXTENT_M};
use look_above_core::globe_camera::GlobeCamera;
use look_above_core::sim::{AircraftInstance, RenderFeed, TrailVertex};
use look_above_core::types::Icao24;
use lyon::tessellation::VertexBuffers;
use wgpu::util::DeviceExt as _;
use wgpu::{CurrentSurfaceTexture, DisplayAndWindowHandle};

use crate::aircraft::{self, InstanceRaw, QuadVertex};
use crate::airport::{self, AirportInstanceRaw};
use crate::basemap::{self, GlobeVertex, MeshData, Vertex};
use crate::color;
use crate::density::{self, DensityDotInstanceRaw, DensityQuadVertex};
use crate::error::RenderError;
use crate::glyph_atlas;
use crate::info_card::{self, InfoCardContent};
use crate::label::{self, LabelPlacement, LeaderVertexRaw, TextInstanceRaw, TextQuadVertex};
use crate::label_atlas;
use crate::metar_badge::{self, BadgeInstanceRaw};
use crate::stats_overlay::{self, StatsOverlay};
use crate::trail::{self, TrailVertexRaw};

/// docs/01's render-target sample count. Checked against the adapter's format features in
/// [`Renderer::new`] before the first MSAA texture is created — see
/// [`RenderError::UnsupportedMsaa`].
const SAMPLE_COUNT: u32 = 4;

/// The base-map shaders (M2 item 2.2b): one vertex entry point shared by both passes, one
/// fragment entry point whose output color comes from a per-pass `@group(1)` uniform rather
/// than being baked into the shader source (see `basemap.wgsl`'s module doc comment).
const BASEMAP_SHADER: &str = include_str!("shaders/basemap.wgsl");

/// The airport marker shader (M3 item 3.2): instanced circles, screen-constant radius, flat
/// per-layer color — see `airport.wgsl`'s module doc comment. The runway-outline pass draws no
/// new shader of its own: it reuses [`BASEMAP_SHADER`] (see [`build_runway_resources`]'s doc
/// comment for why that reuse is exact, not approximate).
const AIRPORT_SHADER: &str = include_str!("shaders/airport.wgsl");

/// The METAR badge shader (M3 item 3.3): instanced rings, screen-constant radius, per-instance
/// color — see `metar_badge.wgsl`'s module doc comment for how this differs from
/// [`AIRPORT_SHADER`]'s flat per-layer color.
const METAR_BADGE_SHADER: &str = include_str!("shaders/metar_badge.wgsl");

/// The aircraft glyph shader (M2 item 2.5): instanced quads, rotation from per-instance heading,
/// SDF atlas sampling — see `aircraft.wgsl`'s module doc comment.
const AIRCRAFT_SHADER: &str = include_str!("shaders/aircraft.wgsl");

/// The trail ribbon shader (M2 item 2.6b): pass-through vertices that `trail.rs` already offset
/// and colored on the CPU — see `trail.wgsl`'s module doc comment.
const TRAIL_SHADER: &str = include_str!("shaders/trail.wgsl");

/// The label shader (M2 item 2.7b): two tiny screen-space pipelines (instanced text-glyph quads,
/// a leader-line `LineList`) sharing one viewport-size uniform — see `label.wgsl`'s module doc
/// comment.
const LABEL_SHADER: &str = include_str!("shaders/label.wgsl");

/// The globe base-map shader (M4 item 4.3): per-vertex orthographic projection with a
/// per-fragment horizon test — see `globe_basemap.wgsl`'s module doc comment.
const GLOBE_BASEMAP_SHADER: &str = include_str!("shaders/globe_basemap.wgsl");

/// The L0 density-dot shader (M4 item 4.3): instanced screen-constant quads, additively blended
/// — see `density.wgsl`'s module doc comment.
const DENSITY_SHADER: &str = include_str!("shaders/density.wgsl");

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
///
/// M4 item 4.3: alpha-blended (not opaque, unlike its pre-4.3 shape) and carrying its own
/// rewritable `color_buffer`/`base_color`, so [`BasemapLayer::set_alpha_multiplier`] can fade
/// this layer *out* every frame as `mode_blend` rises toward the globe — the other half of the
/// plan's "cross-faded during transition" line that only [`GlobeBasemapLayer`]'s fade-in
/// originally covered (see [`Renderer::record_draw_passes`]'s own comment on this call site: the
/// bug that omission caused was the flat Mercator map staying fully opaque and visible in the
/// screen corners outside the globe's inscribed disk, even at full L0). At `multiplier = 1.0`
/// (every L1/L2 frame, `mode_blend` always `0.0` there) this reproduces the original always-
/// opaque behavior exactly — zero visible change outside L0's transition. **Not** shared with
/// [`RunwayLayer`], which keeps building its own pipeline from [`create_basemap_pipeline`]
/// (still opaque, `blend: None`) — see that struct's own doc comment for why runways must not be
/// entangled with globe-transition state.
#[derive(Debug)]
struct BasemapLayer {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    color_bind_group: wgpu::BindGroup,
    /// The buffer backing [`BasemapLayer::color_bind_group`] — kept here (not just inside the
    /// bind group) so [`BasemapLayer::set_alpha_multiplier`] can `queue.write_buffer` into it
    /// every frame, the same "layer keeps the buffer handle for per-frame rewrites" shape
    /// [`AirportMarkerLayer::params_buffer`]/[`DensityLayer::params_buffer`] already have.
    color_buffer: wgpu::Buffer,
    /// This layer's authored, fully-opaque color (`color::land_fill_color`/
    /// `color::coastline_stroke_color`) — [`BasemapLayer::set_alpha_multiplier`] scales only its
    /// alpha channel by this frame's multiplier, never mutating this stored baseline.
    base_color: [f32; 4],
    pipeline: wgpu::RenderPipeline,
}

impl BasemapLayer {
    /// Rewrites this layer's alpha as `base_color.a * multiplier` for this frame — see this
    /// struct's own doc comment for why (the L0 cross-fade). `multiplier = 1.0` is a no-op
    /// against the authored (already-opaque) color, so every L1/L2 frame (where the caller always
    /// passes `1.0 - mode_blend = 1.0`) draws pixel-identical to this layer's original opaque
    /// behavior.
    fn set_alpha_multiplier(&self, queue: &wgpu::Queue, multiplier: f32) {
        let color = [
            self.base_color[0],
            self.base_color[1],
            self.base_color[2],
            self.base_color[3] * multiplier,
        ];
        queue.write_buffer(&self.color_buffer, 0, bytemuck::bytes_of(&color));
    }

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

/// The globe base-map's own land/coastline layer (M4 item 4.3) — the globe analogue of
/// [`BasemapLayer`], drawn right after the Mercator basemap's own land/coastline (see
/// [`Renderer::record_draw_passes`]'s draw-order comment). Static geometry, built once in
/// [`Renderer::new`]/`new_headless` alongside [`BasemapLayer`]'s own — the globe camera can
/// rotate/zoom, but the tessellated sphere mesh itself never changes.
///
/// Unlike [`BasemapLayer`], `draw` takes this frame's `blend` directly: it both gates the whole
/// draw call (skipped entirely at `blend <= 0.0`, matching every other layer's
/// zero-work-when-invisible early return) and — inside the shared `globe_view_proj` uniform
/// [`Renderer`] rewrites once per frame — fades the fragment shader's output alpha, so the globe
/// basemap cross-fades in/out with the rest of the L0 transition rather than popping.
#[derive(Debug)]
struct GlobeBasemapLayer {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    color_bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl GlobeBasemapLayer {
    /// Binds this layer's pipeline/color and draws its whole static mesh in one indexed call —
    /// unless `blend` is fully transparent, in which case nothing is bound or drawn at all.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
        blend: f64,
    ) {
        if blend <= 0.0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_bind_group(1, &self.color_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// The runway-outline pass's GPU resources (M3 item 3.2) — one of the two "map lines" passes
/// this item adds (see [`AirportMarkerLayer`] for the other). Drawn right after
/// [`BasemapLayer`]'s coastline pass and before the trail pass (docs/01's draw order names
/// "coastlines/borders/runways" together as one "map lines" step) — see
/// [`Renderer::record_draw_passes`]'s own doc comment for that placement call.
///
/// Unlike [`BasemapLayer`]'s static, build-once mesh, this rebuilds every frame from whatever
/// `runways` slice the caller hands `Renderer::render` — see `airport.rs`'s module doc comment on
/// why (the queried set only actually changes on camera settle, but re-tessellating a few hundred
/// runways at most, every frame, from reused scratch buffers, is simpler than tracking "did it
/// change" and still costs nothing measurable against docs/01's 4 ms render-thread budget).
#[derive(Debug)]
struct RunwayLayer {
    pipeline: wgpu::RenderPipeline,
    /// This renderer's surface format's runway-outline color (`@group(1)`, fixed, like
    /// [`BasemapLayer::color_bind_group`] — see [`color::runway_outline_color`]).
    color_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    index_capacity: usize,
    /// Reused every frame so tessellating a frame's runways never allocates once capacity has
    /// warmed up (ADR-002) — cleared and refilled by [`airport::tessellate_runways`], not
    /// reallocated. `lyon`'s own `VertexBuffers` (not `basemap::MeshData`): the tessellator
    /// writes into it directly across the whole runway list in one pass (see that function's
    /// doc comment), the same shape `basemap::tessellate_coastline`'s per-feature loop uses
    /// internally, just kept alive across frames here instead of being built fresh once.
    scratch: VertexBuffers<Vertex, u32>,
}

impl RunwayLayer {
    /// Tessellates `runways` into this frame's outline geometry, growing the GPU vertex/index
    /// buffers first if the frame produced more than they currently hold. Returns the index
    /// count to draw. `meters_per_pixel` sizes the screen-constant stroke width — see
    /// [`airport::RUNWAY_STROKE_WIDTH_PX`]'s own doc comment.
    fn upload_runways(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        runways: &[Runway],
        meters_per_pixel: f64,
    ) -> u32 {
        airport::tessellate_runways(runways, meters_per_pixel, &mut self.scratch);

        if self.scratch.vertices.len() > self.vertex_capacity {
            let new_capacity = self
                .scratch
                .vertices
                .len()
                .max(self.vertex_capacity.saturating_mul(2))
                .max(airport::MIN_RUNWAY_VERTEX_CAPACITY);
            self.vertex_buffer = create_runway_vertex_buffer(device, new_capacity);
            self.vertex_capacity = new_capacity;
        }
        if self.scratch.indices.len() > self.index_capacity {
            let new_capacity = self
                .scratch
                .indices
                .len()
                .max(self.index_capacity.saturating_mul(2))
                .max(airport::MIN_RUNWAY_INDEX_CAPACITY);
            self.index_buffer = create_runway_index_buffer(device, new_capacity);
            self.index_capacity = new_capacity;
        }

        if !self.scratch.vertices.is_empty() {
            queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&self.scratch.vertices),
            );
        }
        if !self.scratch.indices.is_empty() {
            queue.write_buffer(
                &self.index_buffer,
                0,
                bytemuck::cast_slice(&self.scratch.indices),
            );
        }

        // Runway index counts are bounded by the queried viewport's runway rows (a few hundred
        // at most, nowhere near docs/01's 10,000-aircraft budget), far inside `u32`.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "runway index count is bounded by a viewport's queried runway rows, far \
                      inside u32::MAX"
        )]
        {
            self.scratch.indices.len() as u32
        }
    }

    /// Binds the runway pipeline/color and draws every uploaded index in one call.
    /// `view_proj_bind_group` is the same `@group(0)` every other pass uses.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
        index_count: u32,
    ) {
        if index_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_bind_group(1, &self.color_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..index_count, 0, 0..1);
    }
}

/// The airport-marker pass's GPU resources (M3 item 3.2) — the other "map lines" pass this item
/// adds (see [`RunwayLayer`] for the runway outlines). Drawn right after [`RunwayLayer`]'s own
/// draw, still before the trail pass — airport markers are not separately named in docs/01's
/// draw-order sentence, so this ordering (map-lines slot, after the runway outlines) is this
/// item's own judgement call, recorded at the call site in
/// [`Renderer::record_draw_passes`].
///
/// Instanced off one static unit-circle mesh ([`airport::marker_mesh`], built once — the same
/// "static shared geometry + per-instance offsets" shape [`AircraftLayer`]'s shared unit quad
/// has), with a per-frame screen-constant-size uniform ([`AirportMarkerLayer::set_params`]) the
/// same role [`AircraftLayer::set_glyph_scale`] plays for aircraft glyphs. The *instance* buffer
/// itself, like [`RunwayLayer`]'s geometry, rebuilds every frame from whatever `airports` slice
/// the caller hands `Renderer::render` — see `airport.rs`'s module doc comment on why.
#[derive(Debug)]
struct AirportMarkerLayer {
    pipeline: wgpu::RenderPipeline,
    /// The `@group(1)` uniform carrying both the fixed marker color and the per-frame
    /// screen-constant scale — one buffer/bind group rather than two (color never changes;
    /// writing both together each frame costs nothing extra over writing the scale alone).
    params_buffer: wgpu::Buffer,
    params_bind_group: wgpu::BindGroup,
    mesh_vertex_buffer: wgpu::Buffer,
    mesh_index_buffer: wgpu::Buffer,
    mesh_index_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// Reused every frame so packing a frame's airports never allocates once capacity has warmed
    /// up (ADR-002) — cleared and refilled by [`airport::pack_airport_instances`].
    instance_scratch: Vec<AirportInstanceRaw>,
    /// This renderer's surface format's airport-marker color (see
    /// [`color::airport_marker_color`]) — kept here (not just baked into the uniform once) so
    /// [`AirportMarkerLayer::set_params`] can rewrite the whole uniform (color + scale) together
    /// every frame without needing to re-derive the color from the format each time.
    color: [f32; 4],
}

impl AirportMarkerLayer {
    /// Rewrites the marker's screen-constant scale (and its unchanging color) for this frame —
    /// see [`airport::airport_marker_scale_normalized`]'s doc comment for why this must be
    /// recomputed every frame rather than once at startup (it depends on the camera's current
    /// zoom), the same reason [`AircraftLayer::set_glyph_scale`] does.
    fn set_params(&self, queue: &wgpu::Queue, scale: f32) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&marker_params_bytes(self.color, scale)),
        );
    }

    /// Packs `airports` into this frame's instance buffer (via
    /// [`airport::pack_airport_instances`]), growing the GPU buffer first if the queried set has
    /// more airports than it currently holds. Returns the instance count to draw.
    fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        airports: &[Airport],
    ) -> u32 {
        airport::pack_airport_instances(airports, &mut self.instance_scratch);

        if self.instance_scratch.len() > self.instance_capacity {
            let new_capacity = self
                .instance_scratch
                .len()
                .max(self.instance_capacity.saturating_mul(2))
                .max(airport::MIN_AIRPORT_INSTANCE_CAPACITY);
            self.instance_buffer = create_airport_instance_buffer(device, new_capacity);
            self.instance_capacity = new_capacity;
        }

        if !self.instance_scratch.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instance_scratch),
            );
        }

        // Bounded by a viewport's queried medium/large airport count — a handful to a few
        // hundred at most, far inside `u32`.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "airport instance count is bounded by a viewport's queried airport rows, \
                      far inside u32::MAX"
        )]
        {
            self.instance_scratch.len() as u32
        }
    }

    /// Binds the marker pipeline/resources and draws every uploaded instance in one call.
    /// `view_proj_bind_group` is the same `@group(0)` every other pass uses.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_bind_group(1, &self.params_bind_group, &[]);
        pass.set_vertex_buffer(0, self.mesh_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.mesh_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.mesh_index_count, 0, 0..instance_count);
    }
}

/// The METAR badge pass's GPU resources (M3 item 3.3) — a ring drawn around a large airport's
/// marker, colored by its cached flight category. Instanced off the same shared unit-circle
/// mesh [`AirportMarkerLayer`] uses (this layer builds its own independent copy of the GPU
/// buffers — see [`metar_badge`]'s module doc comment for why), at a larger screen-constant
/// radius so the marker's own dot paints over the ring's center once drawn after this pass (see
/// [`Renderer::record_draw_passes`]'s own doc comment for that ordering).
///
/// Unlike [`AirportMarkerLayer`], the `@group(1)` uniform here carries only the per-frame scale
/// — a badge's color is per-instance ([`metar_badge::BadgeInstanceRaw::color`]), not a flat
/// per-layer constant, so there is nothing color-shaped to keep fixed at startup.
#[derive(Debug)]
struct BadgeLayer {
    pipeline: wgpu::RenderPipeline,
    params_buffer: wgpu::Buffer,
    params_bind_group: wgpu::BindGroup,
    mesh_vertex_buffer: wgpu::Buffer,
    mesh_index_buffer: wgpu::Buffer,
    mesh_index_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// Reused every frame so packing a frame's badges never allocates once capacity has warmed
    /// up (ADR-002) — cleared and refilled by [`metar_badge::pack_badge_instances`].
    instance_scratch: Vec<BadgeInstanceRaw>,
}

impl BadgeLayer {
    /// Rewrites the badge ring's screen-constant scale for this frame — see
    /// [`metar_badge::badge_scale_normalized`]'s doc comment for why this must be recomputed
    /// every frame rather than once at startup (it depends on the camera's current zoom), the
    /// same reason [`AirportMarkerLayer::set_params`] does for the marker's own scale.
    fn set_params(&self, queue: &wgpu::Queue, scale: f32) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&badge_params_bytes(scale)),
        );
    }

    /// Packs `badges` into this frame's instance buffer (via
    /// [`metar_badge::pack_badge_instances`], which also resolves each badge's category to this
    /// renderer's surface `format`), growing the GPU buffer first if the queried set has more
    /// badges than it currently holds. Returns the instance count to draw.
    fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        badges: &[MetarBadge],
        format: wgpu::TextureFormat,
    ) -> u32 {
        metar_badge::pack_badge_instances(badges, format, &mut self.instance_scratch);

        if self.instance_scratch.len() > self.instance_capacity {
            let new_capacity = self
                .instance_scratch
                .len()
                .max(self.instance_capacity.saturating_mul(2))
                .max(metar_badge::MIN_BADGE_INSTANCE_CAPACITY);
            self.instance_buffer = create_badge_instance_buffer(device, new_capacity);
            self.instance_capacity = new_capacity;
        }

        if !self.instance_scratch.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instance_scratch),
            );
        }

        // Bounded by a viewport's queried large-airport count (a subset of the airport-marker
        // pass's own bounded count), far inside `u32`.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "badge instance count is bounded by a viewport's queried large-airport \
                      rows, far inside u32::MAX"
        )]
        {
            self.instance_scratch.len() as u32
        }
    }

    /// Binds the badge pipeline/resources and draws every uploaded instance in one call.
    /// `view_proj_bind_group` is the same `@group(0)` every other pass uses.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_bind_group(1, &self.params_bind_group, &[]);
        pass.set_vertex_buffer(0, self.mesh_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.mesh_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.mesh_index_count, 0, 0..instance_count);
    }
}

/// The aircraft glyph pass's GPU resources (M2 item 2.5): a static unit-quad mesh shared by
/// every instance, a static SDF atlas texture (one tile per category), a small per-frame
/// uniform for the glyph's constant screen-space size, and a dynamically-grown instance buffer
/// uploaded fresh every frame from the current `RenderFeed`.
///
/// The view-proj bind group is *not* held here — like [`BasemapLayer`], it is shared with the
/// base-map passes and passed into [`AircraftLayer::draw`] by the caller (see
/// [`Renderer::render`]).
#[derive(Debug)]
struct AircraftLayer {
    pipeline: wgpu::RenderPipeline,
    glyph_params_buffer: wgpu::Buffer,
    glyph_params_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// The six altitude-bucket tints for this renderer's surface format, built once (`renderer`
    /// never changes format after `Renderer::new`) — see
    /// [`color::altitude_bucket_tint_table`].
    tint_table: [[f32; 4]; 6],
    /// Reused every frame so packing an instance list never allocates once capacity has warmed
    /// up (ADR-002: no per-frame allocation in the render loop) — cleared and refilled in
    /// [`AircraftLayer::upload_instances`], not reallocated.
    instance_scratch: Vec<InstanceRaw>,
}

impl AircraftLayer {
    /// Rewrites the glyph's constant screen-space size for this frame — see
    /// [`aircraft::glyph_scale_normalized`]'s doc comment for why this must be recomputed every
    /// frame rather than once at startup (it depends on the camera's current zoom).
    fn set_glyph_scale(&self, queue: &wgpu::Queue, glyph_scale: f32) {
        queue.write_buffer(
            &self.glyph_params_buffer,
            0,
            bytemuck::bytes_of(&[glyph_scale, 0.0_f32, 0.0, 0.0]),
        );
    }

    /// Packs `aircraft` into this frame's instance buffer (via [`aircraft::pack_instances`],
    /// which also prepends a selection-outline instance when one aircraft is selected — M2 item
    /// 2.8b), growing the GPU buffer first if the feed has more instances than it currently
    /// holds. Returns the instance count to draw.
    fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aircraft: &[AircraftInstance],
    ) -> u32 {
        aircraft::pack_instances(aircraft, &self.tint_table, &mut self.instance_scratch);

        if self.instance_scratch.len() > self.instance_capacity {
            let new_capacity = self
                .instance_scratch
                .len()
                .max(self.instance_capacity.saturating_mul(2))
                .max(aircraft::MIN_INSTANCE_CAPACITY);
            self.instance_buffer = create_instance_buffer(device, new_capacity);
            self.instance_capacity = new_capacity;
        }

        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&self.instance_scratch),
        );

        // `instance_scratch.len()` is bounded by the feed size; docs/01's own upper budget
        // (10,000 aircraft) is far inside `u32`.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the aircraft count is bounded by docs/01's own 10,000-aircraft budget, \
                      far inside u32::MAX"
        )]
        {
            self.instance_scratch.len() as u32
        }
    }

    /// Binds the aircraft pipeline and its resources and draws every uploaded instance in one
    /// call. `view_proj_bind_group` is the same `@group(0)` bind group the base-map passes use
    /// (see [`BasemapLayer::draw`]) — the aircraft pass reads the identical view-proj matrix.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_bind_group(1, &self.glyph_params_bind_group, &[]);
        pass.set_bind_group(2, &self.atlas_bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..instance_count);
    }
}

/// The L0 density-dot pass's GPU resources (M4 item 4.3) — a screen-constant, additively-blended
/// quad per visible aircraft, drawn right after the aircraft-glyph pass (see
/// [`Renderer::record_draw_passes`]'s draw-order comment). *Not* gated against
/// [`AircraftLayer`]'s own draw here — both draw unconditionally today; cross-fading glyphs vs.
/// density dots by LOD tier is 4.4's job, not this item's (see the M4 plan's own 4.3 entry).
///
/// Shares [`Renderer`]'s `globe_view_proj_bind_group` for its own `@group(0)` — the same shared-
/// bind-group shape [`GlobeBasemapLayer`] uses, since both are globe-space passes reading the
/// same sub-observer center/scale/blend uniform.
#[derive(Debug)]
struct DensityLayer {
    pipeline: wgpu::RenderPipeline,
    params_buffer: wgpu::Buffer,
    params_bind_group: wgpu::BindGroup,
    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// This renderer's surface format's density-dot color (see [`color::density_dot_color`]) —
    /// kept here, like [`AirportMarkerLayer::color`], so [`DensityLayer::set_params`] can
    /// rewrite the whole uniform (color + scale) together every frame.
    color: [f32; 4],
    /// Reused every frame so packing a frame's visible aircraft never allocates once capacity
    /// has warmed up (ADR-002) — cleared and refilled by [`density::pack_density_dots`].
    instance_scratch: Vec<DensityDotInstanceRaw>,
}

impl DensityLayer {
    /// Rewrites the dot's constant screen-space size (and its unchanging color) for this frame —
    /// see [`density::density_dot_scale_normalized`]'s doc comment for why the scale must be
    /// recomputed every frame (it depends on the globe camera's current zoom).
    fn set_params(&self, queue: &wgpu::Queue, scale: f32) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&density_params_bytes(self.color, scale)),
        );
    }

    /// Packs `aircraft` into this frame's instance buffer (via [`density::pack_density_dots`],
    /// projected around `globe_center`), growing the GPU buffer first if the frame produced more
    /// instances than it currently holds. Returns the instance count to draw.
    fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aircraft: &[AircraftInstance],
        globe_center: LatLon,
    ) -> u32 {
        density::pack_density_dots(aircraft, globe_center, &mut self.instance_scratch);

        if self.instance_scratch.len() > self.instance_capacity {
            let new_capacity = self
                .instance_scratch
                .len()
                .max(self.instance_capacity.saturating_mul(2))
                .max(density::MIN_DENSITY_INSTANCE_CAPACITY);
            self.instance_buffer = create_density_instance_buffer(device, new_capacity);
            self.instance_capacity = new_capacity;
        }

        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&self.instance_scratch),
        );

        // Bounded by the feed size; docs/01's own upper budget (10,000 aircraft) is far inside
        // `u32`, same reasoning as `AircraftLayer::upload_instances`.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the aircraft count is bounded by docs/01's own 10,000-aircraft budget, \
                      far inside u32::MAX"
        )]
        {
            self.instance_scratch.len() as u32
        }
    }

    /// Binds the density pipeline/resources and draws every uploaded instance in one call.
    /// `view_proj_bind_group` is [`Renderer`]'s shared globe `@group(0)` bind group.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_bind_group(1, &self.params_bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..instance_count);
    }
}

/// The trail ribbon pass's GPU resources (M2 item 2.6b): one dynamically-grown vertex buffer,
/// re-tessellated and re-uploaded from the frame's `RenderFeed.trails` every frame (the ribbon's
/// width and taper depend on the camera's live zoom, so unlike the base map this cannot be built
/// once). No atlas, no per-frame uniform: `trail.rs` bakes the geometry and color on the CPU, so
/// the pipeline reads only the shared view-proj `@group(0)`, passed into [`TrailLayer::draw`] like
/// the other passes.
#[derive(Debug)]
struct TrailLayer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    /// The six altitude-bucket tints for this renderer's surface format, built once (see
    /// [`color::altitude_bucket_tint_table`]) — the same table the aircraft pass uses.
    tint_table: [[f32; 4]; 6],
    /// Reused every frame so tessellating a feed's trails never allocates once capacity has warmed
    /// up (ADR-002) — cleared and refilled by [`trail::tessellate_trails`], not reallocated.
    vertex_scratch: Vec<TrailVertexRaw>,
}

impl TrailLayer {
    /// Tessellates `trails` into this frame's ribbon vertices, growing the GPU buffer first if the
    /// frame produced more than it currently holds. Returns the vertex count to draw.
    fn upload_trails(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        trails: &[TrailVertex],
        meters_per_pixel: f64,
    ) -> u32 {
        trail::tessellate_trails(
            trails,
            &self.tint_table,
            meters_per_pixel,
            &mut self.vertex_scratch,
        );

        if self.vertex_scratch.len() > self.vertex_capacity {
            let new_capacity = self
                .vertex_scratch
                .len()
                .max(self.vertex_capacity.saturating_mul(2))
                .max(trail::MIN_TRAIL_VERTEX_CAPACITY);
            self.vertex_buffer = create_trail_vertex_buffer(device, new_capacity);
            self.vertex_capacity = new_capacity;
        }

        if !self.vertex_scratch.is_empty() {
            queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&self.vertex_scratch),
            );
        }

        // `vertex_scratch.len()` is bounded by the feed's trail size (docs/01's 10,000-aircraft
        // budget × the 300-sample retention window × 6 vertices/segment) — large, but far inside
        // `u32`.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "trail vertex count is bounded by docs/01's aircraft budget and the trail \
                      retention window, far inside u32::MAX"
        )]
        {
            self.vertex_scratch.len() as u32
        }
    }

    /// Binds the trail pipeline and draws every uploaded ribbon vertex in one call.
    /// `view_proj_bind_group` is the same `@group(0)` the base-map and aircraft passes use.
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        view_proj_bind_group: &'pass wgpu::BindGroup,
        vertex_count: u32,
    ) {
        if vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, view_proj_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..vertex_count, 0..1);
    }
}

/// The label pass's GPU resources (M2 item 2.7b): two tiny pipelines (instanced text-glyph
/// quads, a leader-line `LineList`) sharing one screen-space `@group(0)` viewport-size uniform —
/// unlike every earlier pass, this one does **not** read the shared world view-proj matrix
/// (`basemap_view_proj_bind_group`): label placement/collision already happened in screen-pixel
/// space on the CPU (`label.rs`), so the vertex shader only needs to know the viewport size to
/// convert pixels to clip space.
///
/// Also owns the collision/hysteresis state `label::resolve_collisions` needs across frames
/// (`held`, `last_eval_s`, `cached_placements`) — the same "layer owns the state a pure CPU
/// function needs threaded frame to frame" shape [`AircraftLayer`]'s `instance_scratch` and
/// [`TrailLayer`]'s `vertex_scratch` already have, just with real cross-frame *decisions* (not
/// only scratch reuse) behind it here. See `label.rs`'s module doc comment for why the
/// re-evaluation itself is throttled to ≤ 5 Hz while the *positions* of already-shown labels are
/// still refreshed every frame.
#[derive(Debug)]
struct LabelLayer {
    text_pipeline: wgpu::RenderPipeline,
    leader_pipeline: wgpu::RenderPipeline,
    screen_params_buffer: wgpu::Buffer,
    screen_params_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,
    text_instance_buffer: wgpu::Buffer,
    text_instance_capacity: usize,
    text_instance_scratch: Vec<TextInstanceRaw>,
    leader_vertex_buffer: wgpu::Buffer,
    leader_vertex_capacity: usize,
    leader_vertex_scratch: Vec<LeaderVertexRaw>,
    /// This renderer's surface format's label colors, built once (see
    /// [`color::label_text_color`]/[`color::label_leader_color`]).
    text_color: [f32; 4],
    leader_color: [f32; 4],
    /// `icao24`s that held a slot as of the most recent re-evaluation — the hysteresis input
    /// `label::resolve_collisions` boosts against (see that function's doc comment).
    held: HashSet<Icao24>,
    /// Frame time (`RenderFeed::frame_ts`) of the most recent re-evaluation, or `None` before the
    /// first one — throttles re-evaluation to `label::MIN_EVAL_INTERVAL_S`.
    last_eval_s: Option<f64>,
    /// The labels actually shown as of the most recent re-evaluation, re-projected to this
    /// frame's live aircraft positions every frame regardless of whether this frame itself is an
    /// evaluation tick (see this struct's doc comment).
    cached_placements: Vec<LabelPlacement>,
}

impl LabelLayer {
    /// Rewrites the viewport-size uniform both label pipelines read to convert screen pixels to
    /// clip space — like [`AircraftLayer::set_glyph_scale`], this must be recomputed every frame
    /// (the viewport can resize) rather than once at startup.
    fn set_screen_params(&self, queue: &wgpu::Queue, width_px: f64, height_px: f64) {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "viewport dimensions in physical pixels stay well within f32's precision at \
                      any window size this app supports"
        )]
        let params = [width_px as f32, height_px as f32, 0.0_f32, 0.0_f32];
        queue.write_buffer(&self.screen_params_buffer, 0, bytemuck::bytes_of(&params));
    }

    /// Refreshes which labels are shown — re-evaluating candidates/collisions at ≤ 5 Hz with
    /// hysteresis, or (between evaluations) just re-projecting each currently shown label's
    /// screen position from this frame's live aircraft positions — then packs this frame's
    /// text-instance and leader-vertex GPU buffers, growing them first if needed. Returns
    /// `(text_instance_count, leader_vertex_count)` to draw.
    ///
    /// Split into [`LabelLayer::refresh_placements`] (the ≤ 5 Hz decision) and
    /// [`LabelLayer::upload`] (packing + GPU upload) purely to stay under clippy's line-count
    /// lint — the two are only ever called back to back, here.
    fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        feed: &RenderFeed,
        camera: &Camera,
    ) -> (u32, u32) {
        self.refresh_placements(feed, camera);
        self.upload(device, queue)
    }

    /// The ≤ 5 Hz-with-hysteresis re-evaluation, or (between evaluations) a cheap per-frame
    /// re-projection of the currently shown labels — see [`LabelLayer::update`]'s doc comment.
    fn refresh_placements(&mut self, feed: &RenderFeed, camera: &Camera) {
        let width_px = camera.width_px();
        let height_px = camera.height_px();
        let center_m = camera.center_m();
        let meters_per_pixel = camera.meters_per_pixel();
        let now_s = feed.frame_ts;

        let should_reevaluate = self
            .last_eval_s
            .is_none_or(|last| now_s - last >= label::MIN_EVAL_INTERVAL_S);

        if should_reevaluate {
            let candidates = label::build_candidates(
                &feed.aircraft,
                center_m,
                meters_per_pixel,
                width_px,
                height_px,
            );
            let placements =
                label::resolve_collisions(&candidates, &self.held, width_px, height_px);
            self.held = placements
                .iter()
                .map(|placement| placement.icao24)
                .collect();
            self.cached_placements = placements;
            self.last_eval_s = Some(now_s);
            return;
        }

        // Not a re-evaluation tick: the shown *set* doesn't change, but each shown label's
        // aircraft has kept moving at render cadence, so its on-screen position (and leader
        // line) still needs to track it. `label.rs`'s doc comment on why this calls
        // `placement_geometry` directly rather than rebuilding a whole candidate (no text
        // re-allocation on this, the common, path).
        self.cached_placements.retain_mut(|placement| {
            let Some(instance) = label::find_instance(&feed.aircraft, placement.icao24) else {
                // The aircraft left the feed (faded out) between evaluations — drop its label
                // rather than leaving it frozen in place.
                return false;
            };
            let glyph_px = label::world_to_screen_px(
                instance.position,
                center_m,
                meters_per_pixel,
                width_px,
                height_px,
            );
            let (anchor_px, leader) = label::placement_geometry(
                glyph_px,
                placement.width_px,
                placement.height_px,
                width_px,
                height_px,
            );
            placement.anchor_px = anchor_px;
            placement.leader = leader;
            true
        });
    }

    /// Packs [`LabelLayer::cached_placements`] into this frame's GPU buffers, growing them first
    /// if needed, and returns `(text_instance_count, leader_vertex_count)` to draw.
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> (u32, u32) {
        label::pack_text_instances(
            &self.cached_placements,
            self.text_color,
            &mut self.text_instance_scratch,
        );
        label::pack_leader_vertices(
            &self.cached_placements,
            self.leader_color,
            &mut self.leader_vertex_scratch,
        );

        if self.text_instance_scratch.len() > self.text_instance_capacity {
            let new_capacity = self
                .text_instance_scratch
                .len()
                .max(self.text_instance_capacity.saturating_mul(2))
                .max(label::MIN_TEXT_INSTANCE_CAPACITY);
            self.text_instance_buffer = create_text_instance_buffer(device, new_capacity);
            self.text_instance_capacity = new_capacity;
        }
        if !self.text_instance_scratch.is_empty() {
            queue.write_buffer(
                &self.text_instance_buffer,
                0,
                bytemuck::cast_slice(&self.text_instance_scratch),
            );
        }

        if self.leader_vertex_scratch.len() > self.leader_vertex_capacity {
            let new_capacity = self
                .leader_vertex_scratch
                .len()
                .max(self.leader_vertex_capacity.saturating_mul(2))
                .max(label::MIN_LEADER_VERTEX_CAPACITY);
            self.leader_vertex_buffer = create_leader_vertex_buffer(device, new_capacity);
            self.leader_vertex_capacity = new_capacity;
        }
        if !self.leader_vertex_scratch.is_empty() {
            queue.write_buffer(
                &self.leader_vertex_buffer,
                0,
                bytemuck::cast_slice(&self.leader_vertex_scratch),
            );
        }

        // Both counts are bounded by docs/01's own 10,000-aircraft budget (a handful of
        // characters/one leader line per label at most), far inside `u32`.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "label instance/vertex counts are bounded by docs/01's aircraft budget, far \
                      inside u32::MAX"
        )]
        {
            (
                self.text_instance_scratch.len() as u32,
                self.leader_vertex_scratch.len() as u32,
            )
        }
    }

    /// Binds each label pipeline in turn and draws its uploaded geometry. The leader lines are
    /// drawn *before* the text (so a line never overlaps its own label's characters — it already
    /// terminates at the label box's near edge, per `label::nearest_point_on_box`, but drawing
    /// order still matters for the alpha-blended edge pixels right at that seam).
    fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        text_count: u32,
        leader_vertex_count: u32,
    ) {
        if leader_vertex_count > 0 {
            pass.set_pipeline(&self.leader_pipeline);
            pass.set_bind_group(0, &self.screen_params_bind_group, &[]);
            pass.set_vertex_buffer(0, self.leader_vertex_buffer.slice(..));
            pass.draw(0..leader_vertex_count, 0..1);
        }
        if text_count > 0 {
            pass.set_pipeline(&self.text_pipeline);
            pass.set_bind_group(0, &self.screen_params_bind_group, &[]);
            pass.set_bind_group(1, &self.atlas_bind_group, &[]);
            pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.text_instance_buffer.slice(..));
            pass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..6, 0, 0..text_count);
        }
    }
}

/// The F3 debug frame-stats HUD's GPU resources (M2 item 2.1b) — docs/01's draw order's final
/// "UI overlay" pass.
///
/// Deliberately *not* a new pipeline or atlas: `wgpu::RenderPipeline`/`wgpu::Buffer`/
/// `wgpu::BindGroup` are cheap `Clone` (`Arc`-backed) handles, so [`build_stats_overlay_resources`]
/// clones [`LabelLayer`]'s already-built text pipeline, atlas bind group, shared text-quad mesh,
/// and screen-params bind group straight out of it rather than rasterizing a second copy of the
/// stroke-font atlas or compiling a second pipeline. Only the instance buffer/capacity/scratch
/// and the HUD's own text color are actually new state — the same "layer owns the GPU buffer +
/// reused scratch `Vec`" shape [`AircraftLayer`]/[`TrailLayer`]/[`LabelLayer`] already have.
///
/// No leader lines (this HUD never needs one) and no placement/collision logic (a fixed top-left
/// screen corner, not a world-anchored label) — see [`stats_overlay`]'s module doc comment.
#[derive(Debug)]
struct StatsOverlayLayer {
    text_pipeline: wgpu::RenderPipeline,
    screen_params_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// Reused every frame so packing the HUD's text never allocates once capacity has warmed up
    /// (ADR-002) — cleared and refilled by [`stats_overlay::pack_overlay_instances`].
    instance_scratch: Vec<TextInstanceRaw>,
    text_color: [f32; 4],
}

/// Fixed screen-pixel margin from the viewport's top-left corner where the HUD block starts —
/// a static corner overlay needs no viewport-edge clamping or collision the way a world-anchored
/// label does (`label.rs`'s own edge-margin constant, used only by that pass's placement, is a
/// different, smaller value).
const STATS_OVERLAY_ORIGIN_PX: (f64, f64) = (10.0, 10.0);

impl StatsOverlayLayer {
    /// Packs `stats`' HUD lines (`stats_overlay::format_lines`, fed `instances` for the `N`
    /// line) into this frame's instance buffer, growing it first if needed. Returns the
    /// instance count to draw.
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stats: &StatsOverlay,
        instances: usize,
    ) -> u32 {
        let lines = stats_overlay::format_lines(stats, instances);
        stats_overlay::pack_overlay_instances(
            &lines,
            STATS_OVERLAY_ORIGIN_PX,
            self.text_color,
            &mut self.instance_scratch,
        );

        if self.instance_scratch.len() > self.instance_capacity {
            let new_capacity = self
                .instance_scratch
                .len()
                .max(self.instance_capacity.saturating_mul(2))
                .max(stats_overlay::MIN_OVERLAY_INSTANCE_CAPACITY);
            self.instance_buffer = create_text_instance_buffer(device, new_capacity);
            self.instance_capacity = new_capacity;
        }
        if !self.instance_scratch.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instance_scratch),
            );
        }

        // The HUD is a handful of short lines — nowhere near u32::MAX.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the overlay's character count is a handful of short lines, far inside \
                      u32::MAX"
        )]
        {
            self.instance_scratch.len() as u32
        }
    }

    /// Binds the shared text pipeline/atlas and draws the uploaded HUD instances in one call —
    /// skipped entirely (0 instances) when F3 is off, so toggling it off costs nothing per frame
    /// beyond this no-op check.
    fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, instance_count: u32) {
        if instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.text_pipeline);
        pass.set_bind_group(0, &self.screen_params_bind_group, &[]);
        pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..instance_count);
    }
}

/// The selected-aircraft info card's GPU resources (M2 item 2.8b) — the other half of docs/01's
/// "Selection: white outline + info card" (the outline itself is
/// [`aircraft::pack_selection_outline_instance`], packed straight into [`AircraftLayer`]'s own
/// instance buffer, no separate GPU resources of its own).
///
/// Built the same way as [`StatsOverlayLayer`] (see that struct's own doc comment): *cloned* from
/// [`LabelLayer`]'s already-built text pipeline/atlas/mesh/screen-params bind group rather than a
/// second SDF atlas or pipeline — only the instance buffer/capacity/scratch and this card's own
/// (white) text color are new state. Fixed top-left origin below the F3 HUD's own block (see
/// [`INFO_CARD_ORIGIN_PX`]) so the two never overlap regardless of whether F3 is toggled on.
#[derive(Debug)]
struct InfoCardLayer {
    text_pipeline: wgpu::RenderPipeline,
    screen_params_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// Reused every frame so packing the card's text never allocates once capacity has warmed up
    /// (ADR-002) — cleared and refilled by [`stats_overlay::pack_overlay_instances`].
    instance_scratch: Vec<TextInstanceRaw>,
    text_color: [f32; 4],
}

/// Fixed screen-pixel origin for the info-card block — below [`STATS_OVERLAY_ORIGIN_PX`]'s own
/// 4-line HUD (`(10, 10)` plus 4 lines at `label::LABEL_CHAR_HEIGHT_PX + LINE_LEADING_PX` each) by
/// a comfortable margin, so the two never overlap whether or not F3 is on. Moved down from the
/// original `80.0` when `label::LABEL_CHAR_HEIGHT_PX` grew (M2 gate item 2.10's legibility fix) —
/// the HUD block now ends around `y=128` (`10 + 4 * (28 + 2)`), not `y=64`.
const INFO_CARD_ORIGIN_PX: (f64, f64) = (10.0, 145.0);

impl InfoCardLayer {
    /// Packs `content`'s lines (or nothing, when `content` is `None` — nothing selected) into
    /// this frame's instance buffer, growing it first if needed. Returns the instance count to
    /// draw.
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        content: Option<&InfoCardContent>,
    ) -> u32 {
        match content {
            Some(content) => {
                let lines = info_card::format_lines(content);
                stats_overlay::pack_overlay_instances(
                    &lines,
                    INFO_CARD_ORIGIN_PX,
                    self.text_color,
                    &mut self.instance_scratch,
                );
            }
            None => self.instance_scratch.clear(),
        }

        if self.instance_scratch.len() > self.instance_capacity {
            let new_capacity = self
                .instance_scratch
                .len()
                .max(self.instance_capacity.saturating_mul(2))
                .max(stats_overlay::MIN_OVERLAY_INSTANCE_CAPACITY);
            self.instance_buffer = create_text_instance_buffer(device, new_capacity);
            self.instance_capacity = new_capacity;
        }
        if !self.instance_scratch.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instance_scratch),
            );
        }

        // The card is a handful of short lines — nowhere near u32::MAX.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the card's character count is a handful of short lines, far inside u32::MAX"
        )]
        {
            self.instance_scratch.len() as u32
        }
    }

    /// Binds the shared text pipeline/atlas and draws the uploaded card instances in one call —
    /// skipped entirely (0 instances) when nothing is selected.
    fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, instance_count: u32) {
        if instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.text_pipeline);
        pass.set_bind_group(0, &self.screen_params_bind_group, &[]);
        pass.set_bind_group(1, &self.atlas_bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..instance_count);
    }
}

/// Where a frame's color output goes: a window's swapchain (the only path outside test code),
/// or — behind `#[cfg(test)]`, built only by [`Renderer::new_headless`] for the renderer smoke
/// test (docs/10 §4) — a plain offscreen texture with no presentation step at all.
///
/// Every `build_*_resources` free function below already takes only `&device`/`&queue`/a
/// `format`/a size, never the surface itself, so pulling the surface/config pair out into its
/// own type here is purely about `render`'s own frame-acquire/present step: the windowed path
/// keeps its exact current behavior (see [`Renderer::render`]), and the offscreen path (see
/// [`Renderer::render_headless`]) records the identical sequence of passes
/// ([`Renderer::record_draw_passes`]) into a texture instead.
#[derive(Debug)]
enum Target {
    Windowed {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// Test-only: see [`Renderer::new_headless`].
    #[cfg(test)]
    Offscreen(OffscreenTarget),
}

/// The renderer smoke test's offscreen color target and everything needed to read it back to
/// the CPU after a frame — see [`Renderer::new_headless`]/[`Renderer::render_headless`].
#[cfg(test)]
#[derive(Debug)]
struct OffscreenTarget {
    /// The single-sampled color target the MSAA target resolves onto — `render_headless` copies
    /// this into `readback_buffer` after the pass, instead of presenting it.
    texture: wgpu::Texture,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    /// Sized for one full `width` × `height` RGBA8 frame, its rows padded to
    /// [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`] (`copy_texture_to_buffer`'s own requirement) —
    /// see `padded_bytes_per_row`.
    readback_buffer: wgpu::Buffer,
    /// `width * 4` (RGBA8) rounded up to a multiple of
    /// [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`] — computed once at construction since `width`
    /// never changes for a headless renderer (there is no window to resize it from).
    padded_bytes_per_row: u32,
}

/// Owns the GPU device and a window's swapchain, and paints the map.
///
/// M0 drew only the clear; M2 item 2.2b added the base map (land fill, coastline stroke), 2.5
/// added the aircraft glyph pass, 2.6b added the trail ribbon pass between them, 2.7b added the
/// label pass (text + leader lines), 2.1b added the F3 debug HUD, and 2.8b adds the selected
/// aircraft's white glyph outline (packed into the aircraft pass itself) and its info card, drawn
/// last of all — every piece of docs/01's draw order ("map base → map lines → trails → aircraft →
/// labels → UI overlay") now exists. M3 item 3.2 fills in docs/01's "map lines" step itself
/// (runway outlines, airport markers), which had nothing in it before.
#[derive(Debug)]
pub struct Renderer {
    target: Target,
    device: wgpu::Device,
    queue: wgpu::Queue,
    clear_color: wgpu::Color,
    adapter_info: wgpu::AdapterInfo,
    /// The 4x-multisampled color target every pass renders into. `render` resolves it onto the
    /// swapchain view on submit. Recreated alongside the swapchain in [`Renderer::reconfigure`]
    /// — it must always match the surface size.
    msaa_view: wgpu::TextureView,
    /// The uniform both base-map pipelines read their `view_proj` matrix from (`@group(0)`).
    ///
    /// M2 2.3a: the camera itself lives in `app` (it needs winit input events, and `render`
    /// must stay winit-free — ADR-002/M0's dependency-direction check), so this crate no longer
    /// computes this buffer's contents on its own. [`Renderer::set_view_proj`] is the only
    /// writer now; the caller is expected to call it once per frame (after advancing its
    /// `Camera`) and again after every [`Renderer::resize`]. [`Renderer::new`] seeds it with
    /// [`camera_view_proj`] of a freshly-constructed default [`Camera`] so there is a correct
    /// matrix in place before the app ever calls `set_view_proj` — see that function's doc
    /// comment for why this exactly matches what the app's own first call produces.
    basemap_view_proj_buffer: wgpu::Buffer,
    basemap_view_proj_bind_group: wgpu::BindGroup,
    basemap_land: BasemapLayer,
    basemap_coastline: BasemapLayer,
    /// The uniform the globe basemap and density-dot passes (M4 item 4.3) share for their own
    /// `@group(0)` — sub-observer center/scale plus this frame's globe<->Mercator blend. Distinct
    /// from [`Renderer::basemap_view_proj_buffer`]: those passes compute their own per-vertex
    /// orthographic projection instead of applying a baked matrix, and need `blend` for their
    /// fragment-shader alpha fade too (see `globe_basemap.wgsl`'s own doc comment).
    /// [`Renderer::set_globe_params`] is the only writer, called once per frame by `app` right
    /// next to [`Renderer::set_view_proj`].
    globe_view_proj_buffer: wgpu::Buffer,
    globe_view_proj_bind_group: wgpu::BindGroup,
    globe_basemap_land: GlobeBasemapLayer,
    globe_basemap_coastline: GlobeBasemapLayer,
    /// The L0 density-dot pass (M4 item 4.3) — drawn right after [`Renderer::aircraft`] (see
    /// [`Renderer::record_draw_passes`]'s draw-order comment).
    density: DensityLayer,
    /// This frame's globe camera sub-observer center, stashed by [`Renderer::set_globe_params`]
    /// so [`Renderer::record_draw_passes`] can project aircraft onto the globe's disk
    /// ([`density::pack_density_dots`]) without `render` ever holding a
    /// [`look_above_core::globe_camera::GlobeCamera`] itself — `app` owns that (this crate stays
    /// winit-free, ADR-002), so only the two scalars this pass actually needs cross the
    /// boundary.
    globe_center: LatLon,
    /// This frame's globe<->Mercator blend, stashed alongside `globe_center` for the same
    /// reason — [`GlobeBasemapLayer::draw`]'s own early-return and the density pass's
    /// upload-gate (`record_draw_passes`) both read it.
    globe_blend: f64,
    /// The runway-outline pass (M3 item 3.2) — docs/01's "map lines" step, drawn right after the
    /// coastline stroke and before the trail pass (see [`RunwayLayer`]'s own doc comment).
    runway: RunwayLayer,
    /// The METAR badge pass (M3 item 3.3) — also docs/01's "map lines" step, drawn right after
    /// [`Renderer::runway`] and *before* [`Renderer::airport_marker`] so the marker's own dot
    /// paints over each ring's center (see [`BadgeLayer`]'s own doc comment).
    badge: BadgeLayer,
    /// The airport-marker pass (M3 item 3.2) — also docs/01's "map lines" step, drawn right
    /// after [`Renderer::badge`] and still before the trail pass; not separately ordered by
    /// docs/01's draw-order sentence, so this placement is this item's own judgement call (see
    /// [`AirportMarkerLayer`]'s own doc comment).
    airport_marker: AirportMarkerLayer,
    /// The trail ribbon pass (M2 item 2.6b) — drawn after the base map and *before* the aircraft
    /// glyphs, reusing [`Renderer::basemap_view_proj_bind_group`] for its own `@group(0)` (see
    /// [`TrailLayer::draw`]).
    trail: TrailLayer,
    /// The aircraft glyph pass (M2 item 2.5) — drawn after the trails, reusing
    /// [`Renderer::basemap_view_proj_bind_group`] for its own `@group(0)` (see
    /// [`AircraftLayer::draw`]).
    aircraft: AircraftLayer,
    /// The label pass (M2 item 2.7b) — drawn after the aircraft glyphs. Unlike every other
    /// pass, it does *not* share [`Renderer::basemap_view_proj_bind_group`]: label placement is
    /// already in screen-pixel space (see [`LabelLayer`]'s own doc comment).
    label: LabelLayer,
    /// The F3 debug frame-stats HUD (M2 item 2.1b) — drawn last, after everything else,
    /// docs/01's own final "UI overlay" draw-order step. Reuses [`LabelLayer`]'s pipeline/atlas
    /// (see [`StatsOverlayLayer`]'s own doc comment); like the label pass it is screen-space
    /// only, not the shared world view-proj matrix.
    stats_overlay: StatsOverlayLayer,
    /// The selected-aircraft info card (M2 item 2.8b) — drawn last of all, on top of the F3 HUD.
    /// Reuses [`LabelLayer`]'s pipeline/atlas the same way [`StatsOverlayLayer`] does (see
    /// [`InfoCardLayer`]'s own doc comment).
    info_card: InfoCardLayer,
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
        let msaa_view = create_msaa_view(&device, config.width, config.height, config.format);

        // Shared by both the base-map pipelines and the aircraft pipeline's own `@group(0)`:
        // built once here so every pipeline layout is created from the *same* `BindGroupLayout`
        // object, which is what lets `Renderer::render` pass the one
        // `basemap_view_proj_bind_group` into all three passes' draw calls without wgpu
        // rejecting the bind group as incompatible with a pipeline built from a merely
        // structurally-identical (but distinct) layout.
        let view_proj_layout = create_uniform_bind_group_layout(
            &device,
            wgpu::ShaderStages::VERTEX,
            "look-above shared view-proj bind group layout",
        );
        // Vertex *and* fragment: the globe passes' shared uniform carries both the vertex
        // shader's center/scale and the fragment shader's blend-fade alpha — see the field doc
        // on `Renderer::globe_view_proj_buffer`.
        let globe_view_proj_layout = create_uniform_bind_group_layout(
            &device,
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            "look-above shared globe view-proj bind group layout",
        );

        let basemap_resources = build_basemap_resources(
            &device,
            &view_proj_layout,
            config.format,
            config.width,
            config.height,
        );
        let globe_basemap_resources = build_globe_basemap_resources(
            &device,
            &globe_view_proj_layout,
            config.format,
            config.width,
            config.height,
        );
        let density = build_density_resources(&device, &globe_view_proj_layout, config.format);
        let runway = build_runway_resources(&device, &view_proj_layout, config.format);
        let badge = build_badge_resources(&device, &view_proj_layout, config.format);
        let airport_marker =
            build_airport_marker_resources(&device, &view_proj_layout, config.format);
        let trail = build_trail_resources(&device, &view_proj_layout, config.format);
        let aircraft = build_aircraft_resources(&device, &queue, &view_proj_layout, config.format);
        let label = build_label_resources(&device, &queue, config.format);
        let stats_overlay = build_stats_overlay_resources(&device, &label, config.format);
        let info_card = build_info_card_resources(&device, &label, config.format);

        Ok(Self {
            target: Target::Windowed { surface, config },
            device,
            queue,
            clear_color,
            adapter_info: adapter.get_info(),
            msaa_view,
            basemap_view_proj_buffer: basemap_resources.view_proj_buffer,
            basemap_view_proj_bind_group: basemap_resources.view_proj_bind_group,
            basemap_land: basemap_resources.land,
            basemap_coastline: basemap_resources.coastline,
            globe_view_proj_buffer: globe_basemap_resources.view_proj_buffer,
            globe_view_proj_bind_group: globe_basemap_resources.view_proj_bind_group,
            globe_basemap_land: globe_basemap_resources.land,
            globe_basemap_coastline: globe_basemap_resources.coastline,
            density,
            globe_center: LatLon::new(0.0, 0.0),
            globe_blend: 0.0,
            runway,
            badge,
            airport_marker,
            trail,
            aircraft,
            label,
            stats_overlay,
            info_card,
        })
    }

    /// Headless construction for the renderer smoke test (docs/10 §4) only — never called
    /// outside `#[cfg(test)]`. Requests a *fallback* adapter (`force_fallback_adapter: true`: a
    /// software/CPU implementation — WARP on the DX12 backend, an LLVMpipe-class ICD on Vulkan
    /// when one happens to be registered) with no compatible surface, so this never opens a
    /// window and never depends on a real GPU being present — exactly the "headless wgpu
    /// (fallback adapter)" docs/10 asks for. `Err(RenderError::NoAdapter(_))` is the *expected*
    /// outcome on a CI runner with no fallback adapter registered (there is no dependable
    /// software Vulkan ICD preinstalled on `ubuntu-latest`, and `force_fallback_adapter`'s WARP
    /// path on `windows-latest` is not guaranteed either — see this crate's own CI decision log
    /// entry); the smoke test itself treats exactly that error as "skip", per docs/10's own
    /// "skipped, not failed" wording, not any other error this can return.
    ///
    /// Deviation worth documenting: `force_fallback_adapter` is filtered in `wgpu-core` by
    /// `DeviceType::Cpu` (see that crate's `Instance::request_adapter`), which is honored
    /// identically by every backend wgpu-core drives (DX12/Vulkan/Metal) — there is no
    /// backend-specific divergence to work around here, just the practical point above that
    /// *whether a CPU-type adapter exists at all* varies by runner/OS.
    ///
    /// Renders into a plain offscreen texture (`RENDER_ATTACHMENT | COPY_SRC`, fixed at
    /// [`wgpu::TextureFormat::Rgba8Unorm`] — there is no surface to pick a format from, and a
    /// non-sRGB format keeps the smoke test's own pixel readback simple: `color.rs`'s
    /// `layer_color`/`clear_color` already pass values through unlinearized for a non-sRGB
    /// format, so the raw bytes read back match the authored hex colors directly, no transfer
    /// function to invert first) instead of a swapchain — see [`Renderer::render_headless`].
    #[cfg(test)]
    #[allow(
        clippy::too_many_lines,
        reason = "one-time headless construction sequencing (device/adapter, the offscreen \
                  target, then every pass's resources, M4 4.3's globe/density resources now \
                  among them) — the same shape `Renderer::new` itself already has, splitting it \
                  further would just move locals through another layer of parameters rather than \
                  reducing what this constructor actually does"
    )]
    fn new_headless(width: u32, height: u32) -> Result<Self, RenderError> {
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: true,
            compatible_surface: None,
            ..Default::default()
        }))?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("look-above headless device"),
                ..Default::default()
            }))?;

        // Same guard as `Renderer::new`'s own MSAA check, against the same docs/01 requirement
        // — a fallback adapter is exactly the kind that might genuinely lack it.
        let msaa_features = adapter.get_texture_format_features(FORMAT).flags;
        if !msaa_features.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
            || !msaa_features.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        {
            return Err(RenderError::UnsupportedMsaa {
                adapter: adapter.get_info().name.clone(),
                format: FORMAT,
            });
        }

        // Zero-sized textures are invalid, same floor `Renderer::new` holds for the surface.
        let width = width.max(1);
        let height = height.max(1);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("look-above headless color target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // `copy_texture_to_buffer` requires each row's buffer offset to land on
        // `COPY_BYTES_PER_ROW_ALIGNMENT` — round the true (4 bytes/pixel) row size up to it, and
        // over-allocate the buffer to match; `read_offscreen_pixels` strips the padding back out
        // per row when it reads this back.
        let unpadded_bytes_per_row = width * 4;
        let padded_bytes_per_row = unpadded_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("look-above headless readback buffer"),
            size: wgpu::BufferAddress::from(padded_bytes_per_row)
                * wgpu::BufferAddress::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let clear_color = color::clear_color(FORMAT);
        let msaa_view = create_msaa_view(&device, width, height, FORMAT);

        let view_proj_layout = create_uniform_bind_group_layout(
            &device,
            wgpu::ShaderStages::VERTEX,
            "look-above shared view-proj bind group layout",
        );
        let globe_view_proj_layout = create_uniform_bind_group_layout(
            &device,
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            "look-above shared globe view-proj bind group layout",
        );
        let basemap_resources =
            build_basemap_resources(&device, &view_proj_layout, FORMAT, width, height);
        let globe_basemap_resources =
            build_globe_basemap_resources(&device, &globe_view_proj_layout, FORMAT, width, height);
        let density = build_density_resources(&device, &globe_view_proj_layout, FORMAT);
        let runway = build_runway_resources(&device, &view_proj_layout, FORMAT);
        let badge = build_badge_resources(&device, &view_proj_layout, FORMAT);
        let airport_marker = build_airport_marker_resources(&device, &view_proj_layout, FORMAT);
        let trail = build_trail_resources(&device, &view_proj_layout, FORMAT);
        let aircraft = build_aircraft_resources(&device, &queue, &view_proj_layout, FORMAT);
        let label = build_label_resources(&device, &queue, FORMAT);
        let stats_overlay = build_stats_overlay_resources(&device, &label, FORMAT);
        let info_card = build_info_card_resources(&device, &label, FORMAT);

        Ok(Self {
            target: Target::Offscreen(OffscreenTarget {
                texture,
                format: FORMAT,
                width,
                height,
                readback_buffer,
                padded_bytes_per_row,
            }),
            device,
            queue,
            clear_color,
            adapter_info: adapter.get_info(),
            msaa_view,
            basemap_view_proj_buffer: basemap_resources.view_proj_buffer,
            basemap_view_proj_bind_group: basemap_resources.view_proj_bind_group,
            basemap_land: basemap_resources.land,
            basemap_coastline: basemap_resources.coastline,
            globe_view_proj_buffer: globe_basemap_resources.view_proj_buffer,
            globe_view_proj_bind_group: globe_basemap_resources.view_proj_bind_group,
            globe_basemap_land: globe_basemap_resources.land,
            globe_basemap_coastline: globe_basemap_resources.coastline,
            density,
            globe_center: LatLon::new(0.0, 0.0),
            globe_blend: 0.0,
            runway,
            badge,
            airport_marker,
            trail,
            aircraft,
            label,
            stats_overlay,
            info_card,
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
        match &self.target {
            Target::Windowed { config, .. } => config.format,
            #[cfg(test)]
            Target::Offscreen(offscreen) => offscreen.format,
        }
    }

    /// Uploads a freshly computed view-proj matrix for the base-map passes to read this frame.
    ///
    /// The camera itself lives in `app` (see the field doc on `basemap_view_proj_buffer`);
    /// build `matrix` with [`camera_view_proj`] and call this once per frame, and again after
    /// every [`Renderer::resize`] (a resize can change the camera's zoom ceiling, so the matrix
    /// must be rebuilt, not just left alone — see [`Renderer::reconfigure`]).
    pub fn set_view_proj(&mut self, matrix: [[f32; 4]; 4]) {
        self.queue.write_buffer(
            &self.basemap_view_proj_buffer,
            0,
            bytemuck::bytes_of(&matrix),
        );
    }

    /// Uploads this frame's globe-space uniform (sub-observer center, radius-to-clip scale, and
    /// the globe<->Mercator `blend`) for the globe basemap and density-dot passes to read, and
    /// refreshes the density pass's own screen-constant dot-size uniform (which depends on the
    /// same `globe_camera.radius_px()`). Also stashes `globe_camera`'s center and `blend` on
    /// `self` — see the field docs on [`Renderer::globe_center`]/[`Renderer::globe_blend`] for
    /// why `record_draw_passes` needs them cached here rather than reading `globe_camera`
    /// directly (this crate never holds one — `app` owns it, per this crate's own winit-free
    /// boundary, ADR-002).
    ///
    /// Call once per frame, right next to [`Renderer::set_view_proj`] — `app::window::App::draw`
    /// is the one caller.
    pub fn set_globe_params(&mut self, globe_camera: &GlobeCamera, blend: f64) {
        self.queue.write_buffer(
            &self.globe_view_proj_buffer,
            0,
            bytemuck::bytes_of(&globe_view_proj_bytes(globe_camera, blend)),
        );
        self.density.set_params(
            &self.queue,
            density::density_dot_scale_normalized(globe_camera.radius_px()),
        );
        self.globe_center = globe_camera.center();
        self.globe_blend = blend;
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
        // Only the windowed path is ever resized (a headless renderer, test-only, is never
        // handed to `app::window`'s resize handler). A `match` rather than a `let`-`else`: in a
        // non-test build `Target` has only the one `Windowed` variant, and clippy correctly
        // treats a `let`-`else` whose pattern cannot actually fail as a bug (an always-taken
        // `else` block) — a `match` naturally stays exhaustive (and lint-clean) whether `Target`
        // has one variant or two, so this needs no `#[cfg(test)]` of its own.
        let config = match &mut self.target {
            Target::Windowed { config, .. } => config,
            #[cfg(test)]
            Target::Offscreen(_) => return,
        };
        if (width, height) == (config.width, config.height) {
            return;
        }

        config.width = width;
        config.height = height;
        self.reconfigure();
    }

    /// Draw and present one frame: the background clear, the base map (land fill, coastline
    /// stroke), then `feed`'s trail ribbons, its aircraft glyphs (a selected one's own outline
    /// instance drawn first among them — M2 item 2.8b), its labels (text + leader lines), the F3
    /// debug HUD, and finally the selected-aircraft info card on top of everything — docs/01's
    /// full draw order, end to end.
    ///
    /// Takes the live `camera` itself (M2 item 2.7b), not just its `meters_per_pixel` scalar as
    /// before: the aircraft/trail passes still only need that scalar (to size glyphs/ribbons a
    /// constant number of screen pixels regardless of zoom — see
    /// [`aircraft::glyph_scale_normalized`]/[`trail::tessellate_trails`]), but the label pass
    /// additionally needs the camera's `center_m`/`width_px`/`height_px` to project aircraft
    /// positions into screen-pixel space for placement and collision (see [`label`]'s module doc
    /// comment on why that stays render-side rather than living in `core`).
    ///
    /// `stats` is `Some` only while F3 is on (`app::window::App::stats_visible`); when `None`,
    /// nothing is built or uploaded for the HUD pass at all — see [`StatsOverlayLayer::draw`].
    /// `info_card` is `Some` only while an aircraft is selected (`app::window::App` looks it up
    /// in `feed.aircraft` by its held `selected_icao24`); `None` (nothing selected, or the
    /// selected aircraft left the feed) likewise builds/uploads nothing for the card.
    ///
    /// `airports`/`runways` (M3 item 3.2) are whatever `app::window::App` most recently queried
    /// from `store::Writer` for the camera's settled viewport (fixed `AirportSize::Medium`
    /// threshold, no LOD-tier gating yet — see the M3 plan's own cross-milestone tension note);
    /// empty slices draw nothing for either pass, the same tolerant shape `stats`/`info_card`
    /// being `None` already has. `metar_badges` (M3 item 3.3) is the same shape again, one level
    /// narrower: `app::window::App`'s own join of that same queried airport set (filtered to
    /// `AirportSize::Large`) against whatever METAR the store has cached for it.
    #[allow(
        clippy::too_many_arguments,
        reason = "the public entry point every one of this frame's per-pass inputs converges \
                  through before record_draw_passes; see that method's own too_many_arguments \
                  reason for why splitting this into a params struct would not actually reduce \
                  the information this call carries"
    )]
    pub fn render(
        &mut self,
        feed: &RenderFeed,
        camera: &Camera,
        stats: Option<StatsOverlay>,
        info_card: Option<&InfoCardContent>,
        airports: &[Airport],
        runways: &[Runway],
        metar_badges: &[MetarBadge],
    ) -> Result<FrameOutcome, RenderError> {
        // `match`, not `let`-`else` — see `Renderer::resize`'s own comment on why: in a
        // non-test build `Target` has only the one `Windowed` variant, which would make a
        // `let`-`else` here an always-taken (and clippy-denied) `else` block.
        let surface = match &self.target {
            Target::Windowed { surface, .. } => surface,
            // Only `Renderer::new` (windowed) is ever reachable outside test code; the headless
            // constructor (`Renderer::new_headless`, `#[cfg(test)]`-only) draws through
            // `render_headless` instead, which never calls this method.
            #[cfg(test)]
            Target::Offscreen(_) => {
                unreachable!("Renderer::render called on a non-windowed renderer")
            }
        };
        let (frame, stale) = match surface.get_current_texture() {
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
        self.record_draw_passes(
            &mut encoder,
            &view,
            feed,
            camera,
            stats,
            info_card,
            airports,
            runways,
            metar_badges,
        );

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);

        if stale {
            self.reconfigure();
        }

        Ok(FrameOutcome::Presented)
    }

    /// Draws one frame into the offscreen target built by [`Renderer::new_headless`] and copies
    /// the resolved color attachment into its readback buffer — the offscreen analog of
    /// [`Renderer::render`]'s frame-acquire/submit/present, minus the "present" (there is no
    /// swapchain) and plus the copy (there is no compositor to hand the texture to instead).
    /// [`Renderer::read_offscreen_pixels`] maps and actually reads the copied bytes back on the
    /// CPU; kept as a separate call so a test can render, then read, without paying for a map
    /// it might not need every frame.
    ///
    /// Returns `FrameOutcome` directly, not `Result<FrameOutcome, RenderError>` like
    /// [`Renderer::render`]: unlike a swapchain frame, an offscreen texture has no
    /// lost/outdated/occluded states to report, so — this being the one caller — there is no
    /// error path here for a `Result` to carry.
    #[cfg(test)]
    fn render_headless(&mut self, feed: &RenderFeed, camera: &Camera) -> FrameOutcome {
        let Target::Offscreen(offscreen) = &self.target else {
            unreachable!("render_headless called on a windowed renderer");
        };
        // Cloning `Texture`/`Buffer` is cheap (both are `Arc`-backed handles — see e.g.
        // `build_stats_overlay_resources`'s own doc comment on the same point for
        // `RenderPipeline`/`BindGroup`), and doing it here ends the immutable borrow of
        // `self.target` before `record_draw_passes` below needs `&mut self`.
        let texture = offscreen.texture.clone();
        let view = offscreen
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (width, height, padded_bytes_per_row) = (
            offscreen.width,
            offscreen.height,
            offscreen.padded_bytes_per_row,
        );
        let readback_buffer = offscreen.readback_buffer.clone();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("look-above headless frame"),
            });
        // No airports/runways/badges in the smoke test — it exercises the aircraft/trail/label
        // path (docs/10 §4), not these items' own passes; empty slices draw nothing for any of
        // them, the same tolerant shape `Renderer::render`'s own doc comment describes.
        self.record_draw_passes(&mut encoder, &view, feed, camera, None, None, &[], &[], &[]);

        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        FrameOutcome::Presented
    }

    /// Maps [`OffscreenTarget::readback_buffer`] and copies it into a flat, row-major, per-pixel
    /// RGBA `Vec` (padding stripped) — call only after [`Renderer::render_headless`] has actually
    /// run the copy for the frame being read.
    ///
    /// Blocks on `self.device.poll` with an indefinite wait: this is test-only code with no
    /// event loop to drive the map callback otherwise (see [`wgpu::Buffer::map_async`]'s own doc
    /// comment on what has to poll it).
    #[cfg(test)]
    fn read_offscreen_pixels(&self) -> Vec<[u8; 4]> {
        let Target::Offscreen(offscreen) = &self.target else {
            unreachable!("read_offscreen_pixels called on a windowed renderer");
        };
        let (width, height, padded_bytes_per_row) = (
            offscreen.width,
            offscreen.height,
            offscreen.padded_bytes_per_row,
        );

        let slice = offscreen.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver always outlives this callback (it is not dropped until after
            // `device.poll` returns below), so a failed send would mean wgpu invoked the
            // callback twice — a wgpu bug, not something this test needs to handle gracefully.
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("polling the headless device for the readback map");
        rx.recv()
            .expect("map_async's callback ran before device.poll returned")
            .expect("the readback buffer mapped successfully");

        let mapped = slice
            .get_mapped_range()
            .expect("the buffer is mapped at this point");
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for row in 0..height {
            let row_start = (row * padded_bytes_per_row) as usize;
            for col in 0..width {
                let px = row_start + (col * 4) as usize;
                pixels.push([mapped[px], mapped[px + 1], mapped[px + 2], mapped[px + 3]]);
            }
        }
        drop(mapped);
        offscreen.readback_buffer.unmap();
        pixels
    }

    /// Uploads this frame's dynamic GPU state (glyph scale, runway/badge/marker/trail/aircraft/
    /// label geometry, the optional F3 HUD and info-card text) and records docs/01's full draw
    /// order — background clear, base map, map lines (runway outlines, METAR badges, airport
    /// markers — M3 items 3.2/3.3), trails, aircraft, labels, HUD, info card — into `encoder`,
    /// resolving onto `resolve_target`. Shared by [`Renderer::render`] (the windowed swapchain
    /// path) and [`Renderer::render_headless`] (the renderer smoke test's offscreen path,
    /// `#[cfg(test)]`) so the two draw exactly the same pass sequence with nothing duplicated to
    /// drift apart.
    ///
    /// Airport markers/badges are not separately named in docs/01's draw-order sentence ("map
    /// base → map lines (coastlines/borders/runways) → trails → ..."); placing them in the same
    /// map-lines slot — badges right after the runway-outline draw, markers right after that, so
    /// a marker's own dot paints over its badge's ring center (see [`BadgeLayer`]'s own doc
    /// comment) — is this item's own judgement call, the same kind of explicit interpretation
    /// call this project records at every gate rather than silently deciding (see
    /// `plans/M3_ENRICHMENT_AND_NON_ADSB.md`'s own 3.1 entry for the precedent).
    #[allow(
        clippy::too_many_arguments,
        reason = "this is the one place every pass's per-frame inputs converge (feed, camera, \
                  optional HUD/info-card content, and M3's airports/runways/metar_badges) so \
                  every pass draws from the exact same frame's data — splitting it into a params \
                  struct would not reduce the actual information this function needs, just move \
                  it behind one more layer of indirection"
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "every pass's per-frame upload plus the full draw-order recording (M4 4.3's \
                  globe basemap and density-dot passes now among them) — the same \
                  one-place-everything-converges shape the too_many_arguments reason above \
                  already documents; splitting the uploads from the draw calls would separate two \
                  halves that must stay in the exact order they run in"
    )]
    fn record_draw_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        resolve_target: &wgpu::TextureView,
        feed: &RenderFeed,
        camera: &Camera,
        stats: Option<StatsOverlay>,
        info_card: Option<&InfoCardContent>,
        airports: &[Airport],
        runways: &[Runway],
        metar_badges: &[MetarBadge],
    ) {
        let meters_per_pixel = camera.meters_per_pixel();

        // M4 item 4.3's cross-fade: the flat Mercator land/coastline layers fade *out* as
        // `globe_blend` rises toward the globe — see `BasemapLayer`'s own doc comment for why
        // this is the fix for the flat map staying fully opaque (and visible in the screen
        // corners outside the globe's inscribed disk) at full L0. `clamp` is defensive only —
        // `App::draw`'s `mode_blend` is already kept within `[0, 1]` by construction.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "an f64 already clamped to [0, 1] narrows to f32 losslessly at this \
                      precision"
        )]
        let mercator_fade_multiplier = (1.0 - self.globe_blend).clamp(0.0, 1.0) as f32;
        self.basemap_land
            .set_alpha_multiplier(&self.queue, mercator_fade_multiplier);
        self.basemap_coastline
            .set_alpha_multiplier(&self.queue, mercator_fade_multiplier);

        // Both writes below are queue uploads, not render-pass work — they must land before the
        // pass's draw call reads them, which the queue's own call-order guarantee satisfies as
        // long as they run before `queue.submit`, same as `set_view_proj`'s writes always have.
        let glyph_scale = aircraft::glyph_scale_normalized(meters_per_pixel);
        self.aircraft.set_glyph_scale(&self.queue, glyph_scale);
        let airport_marker_scale = airport::airport_marker_scale_normalized(meters_per_pixel);
        self.airport_marker
            .set_params(&self.queue, airport_marker_scale);
        let badge_scale = metar_badge::badge_scale_normalized(meters_per_pixel);
        self.badge.set_params(&self.queue, badge_scale);
        let runway_index_count =
            self.runway
                .upload_runways(&self.device, &self.queue, runways, meters_per_pixel);
        // Read before the call below: `self.format()` borrows all of `self` immutably (it is a
        // method call, not a direct field access the borrow checker can see is disjoint), which
        // would conflict with `self.badge`'s simultaneous `&mut` if inlined into the call.
        let format = self.format();
        let badge_instance_count =
            self.badge
                .upload_instances(&self.device, &self.queue, metar_badges, format);
        let airport_instance_count =
            self.airport_marker
                .upload_instances(&self.device, &self.queue, airports);
        let trail_vertex_count =
            self.trail
                .upload_trails(&self.device, &self.queue, &feed.trails, meters_per_pixel);
        let instance_count =
            self.aircraft
                .upload_instances(&self.device, &self.queue, &feed.aircraft);
        // M4 item 4.3: skip the CPU-side projection/packing entirely while the globe isn't even
        // partially visible (`globe_blend <= 0.0`, the common case at L1/L2) — the same
        // zero-work-when-invisible reasoning `GlobeBasemapLayer::draw`'s own early return has,
        // just applied a step earlier so a Mercator-mode frame with docs/01's own 10,000-aircraft
        // budget never pays for a projection nothing will draw.
        let density_instance_count = if self.globe_blend > 0.0 {
            self.density.upload_instances(
                &self.device,
                &self.queue,
                &feed.aircraft,
                self.globe_center,
            )
        } else {
            0
        };
        self.label
            .set_screen_params(&self.queue, camera.width_px(), camera.height_px());
        let (label_text_count, label_leader_count) =
            self.label.update(&self.device, &self.queue, feed, camera);
        // F3 off: build/upload nothing for the HUD pass, not even an empty buffer write — see
        // `Renderer::render`'s own doc comment on `stats`.
        let stats_overlay_count = match stats {
            Some(stats) => {
                self.stats_overlay
                    .upload(&self.device, &self.queue, &stats, feed.aircraft.len())
            }
            None => 0,
        };
        let info_card_count = self.info_card.upload(&self.device, &self.queue, info_card);

        // The pass renders into the 4x MSAA target and resolves onto `resolve_target` on submit
        // — plumbing the aircraft, trail, and label passes 2.4+ hang off this same attachment.
        // The multisampled contents themselves are never read back, hence `Discard`; only the
        // resolved view needs to survive (to present, in `render`; to be copied out, in
        // `render_headless`).
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("background + base map"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.msaa_view,
                depth_slice: None,
                resolve_target: Some(resolve_target),
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

        // docs/01 draw order: map base (Mercator, then the globe basemap cross-fading in/out on
        // top of it — M4 item 4.3), then map lines (runway outlines, then airport markers — M3
        // item 3.2; see `record_draw_passes`'s own doc comment on the marker-placement judgement
        // call), then trails (2.6b), then aircraft glyphs (2.5; a selected one's own outline
        // instance is packed first among them — 2.8b) with the L0 density dots layered on top of
        // them (M4 item 4.3 — see `DensityLayer`'s own doc comment on why that overlap with the
        // glyph pass is intentional here, not yet tier-gated), then labels (2.7b), then the F3
        // debug HUD (2.1b), then the selected-aircraft info card (2.8b) last of all, on top of
        // everything else.
        self.basemap_land
            .draw(&mut pass, &self.basemap_view_proj_bind_group);
        self.basemap_coastline
            .draw(&mut pass, &self.basemap_view_proj_bind_group);
        self.globe_basemap_land.draw(
            &mut pass,
            &self.globe_view_proj_bind_group,
            self.globe_blend,
        );
        self.globe_basemap_coastline.draw(
            &mut pass,
            &self.globe_view_proj_bind_group,
            self.globe_blend,
        );
        self.runway.draw(
            &mut pass,
            &self.basemap_view_proj_bind_group,
            runway_index_count,
        );
        self.badge.draw(
            &mut pass,
            &self.basemap_view_proj_bind_group,
            badge_instance_count,
        );
        self.airport_marker.draw(
            &mut pass,
            &self.basemap_view_proj_bind_group,
            airport_instance_count,
        );
        self.trail.draw(
            &mut pass,
            &self.basemap_view_proj_bind_group,
            trail_vertex_count,
        );
        self.aircraft.draw(
            &mut pass,
            &self.basemap_view_proj_bind_group,
            instance_count,
        );
        self.density.draw(
            &mut pass,
            &self.globe_view_proj_bind_group,
            density_instance_count,
        );
        self.label
            .draw(&mut pass, label_text_count, label_leader_count);
        self.stats_overlay.draw(&mut pass, stats_overlay_count);
        self.info_card.draw(&mut pass, info_card_count);
    }

    /// Rebuild the swapchain from the current config, and everything tied to the surface size
    /// alongside it — just the MSAA target now.
    ///
    /// M2 2.2b's placeholder camera lived here too (this function used to rewrite the
    /// view-proj buffer with a fresh aspect-correcting fit on every resize). Now that the
    /// camera lives in `app`, this crate has nothing to compute that matrix from on its own —
    /// `App::window_event`'s `Resized` handler calls `Camera::resize` and
    /// `Renderer::set_view_proj` back-to-back, synchronously, before the next `RedrawRequested`
    /// (winit delivers `Resized` and redraw as separate events; no frame is drawn in between),
    /// so leaving the buffer untouched here never presents a stale-but-wrong matrix — the last
    /// value written (either the seed in `build_basemap_resources` or the app's most recent
    /// `set_view_proj`) stays valid until the app's own resize handler overwrites it a moment
    /// later.
    ///
    /// Windowed-only, like `resize` itself — see that method's own comment on why this is a
    /// `match`, not a `let`-`else`.
    fn reconfigure(&mut self) {
        let (surface, config) = match &self.target {
            Target::Windowed { surface, config } => (surface, config),
            #[cfg(test)]
            Target::Offscreen(_) => return,
        };
        surface.configure(&self.device, config);
        self.msaa_view = create_msaa_view(&self.device, config.width, config.height, config.format);
    }
}

/// Build the multisampled color target `render`/`render_headless` draw into, for one target
/// size/format — a plain `(width, height, format)` triple rather than a whole
/// `&wgpu::SurfaceConfiguration` so [`Renderer::new_headless`] (which has no surface, and so no
/// `SurfaceConfiguration`) can build one too.
fn create_msaa_view(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("look-above msaa color target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format,
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
///
/// `view_proj_layout` is built by the caller, not here: it is shared with the aircraft
/// pipeline's own `@group(0)` (M2 item 2.5), and both pipeline layouts must be built from the
/// exact same `BindGroupLayout` object for `Renderer::render` to pass one bind group into every
/// pass's draw call — see [`Renderer::new`]'s doc comment on that field.
fn build_basemap_resources(
    device: &wgpu::Device,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> BasemapResources {
    let geometry = basemap::tessellate();

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above basemap shader"),
        source: wgpu::ShaderSource::Wgsl(BASEMAP_SHADER.into()),
    });

    let color_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::FRAGMENT,
        "look-above basemap layer-color bind group layout",
    );

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above basemap pipeline layout"),
        bind_group_layouts: &[Some(view_proj_layout), Some(&color_layout)],
        immediate_size: 0,
    });

    // Seed with a freshly-constructed default `Camera`'s matrix rather than leaving the buffer
    // uninitialized: there is a gap between `Renderer::new` returning and `App::start`
    // constructing its own `Camera` and calling `set_view_proj` for the first time, and this
    // avoids a one-frame flash of a wrong transform in that gap. It's guaranteed to match what
    // the app immediately overwrites it with — same `Camera::new(width, height)` call, same
    // result (see `camera_view_proj`'s doc comment).
    let view_proj_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above basemap view-proj uniform"),
        contents: bytemuck::bytes_of(&camera_view_proj(&Camera::new(width, height))),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let view_proj_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above basemap view-proj bind group"),
        layout: view_proj_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: view_proj_buffer.as_entire_binding(),
        }],
    });

    // Two pipeline objects, per docs/01's draw order treating land fill and coastline stroke
    // as separate passes: both are `TriangleList` today (lyon's stroke tessellator already
    // emits triangles, not a `LineList` primitive), so they are identical apart from label and
    // bound resources for now, but kept separate so either can gain its own primitive/blend
    // state later without disturbing the other. Alpha-blended (M4 item 4.3), not opaque — see
    // [`create_basemap_pipeline_blended`]'s own doc comment.
    let land_pipeline = create_basemap_pipeline_blended(
        device,
        &shader,
        &pipeline_layout,
        format,
        "look-above basemap land fill pipeline",
    );
    let coastline_pipeline = create_basemap_pipeline_blended(
        device,
        &shader,
        &pipeline_layout,
        format,
        "look-above basemap coastline stroke pipeline",
    );

    let land_color = color::land_fill_color(format);
    let (land_color_buffer, land_color_bind_group) = create_rewritable_color_bind_group(
        device,
        &color_layout,
        land_color,
        "look-above basemap land color",
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
        color_bind_group: land_color_bind_group,
        color_buffer: land_color_buffer,
        base_color: land_color,
        pipeline: land_pipeline,
    };
    let coastline_color = color::coastline_stroke_color(format);
    let (coastline_color_buffer, coastline_color_bind_group) = create_rewritable_color_bind_group(
        device,
        &color_layout,
        coastline_color,
        "look-above basemap coastline color",
    );
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
        color_bind_group: coastline_color_bind_group,
        color_buffer: coastline_color_buffer,
        base_color: coastline_color,
        pipeline: coastline_pipeline,
    };

    BasemapResources {
        view_proj_buffer,
        view_proj_bind_group,
        land,
        coastline,
    }
}

/// Everything [`Renderer::new`] needs out of globe base-map setup (M4 item 4.3), bundled the
/// same way [`BasemapResources`] is for the Mercator path.
struct GlobeBasemapResources {
    view_proj_buffer: wgpu::Buffer,
    view_proj_bind_group: wgpu::BindGroup,
    land: GlobeBasemapLayer,
    coastline: GlobeBasemapLayer,
}

/// Tessellates the globe base map and uploads it as static GPU buffers, builds the shared globe
/// view-proj uniform, and builds both globe layers' pipelines. Runs once, in [`Renderer::new`]/
/// `new_headless`, alongside [`build_basemap_resources`] — see that function's own doc comment
/// for the `view_proj_layout`-sharing contract this mirrors (here shared with
/// [`build_density_resources`]'s own pipeline instead of the aircraft pipeline).
fn build_globe_basemap_resources(
    device: &wgpu::Device,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> GlobeBasemapResources {
    let geometry = basemap::tessellate_globe();

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above globe basemap shader"),
        source: wgpu::ShaderSource::Wgsl(GLOBE_BASEMAP_SHADER.into()),
    });

    let color_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::FRAGMENT,
        "look-above globe basemap layer-color bind group layout",
    );

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above globe basemap pipeline layout"),
        bind_group_layouts: &[Some(view_proj_layout), Some(&color_layout)],
        immediate_size: 0,
    });

    // Seeded at `blend: 0.0` (fully transparent/invisible) rather than whatever the app will
    // actually want first: the same "avoid a wrong flash in the gap before the app's own first
    // call" reasoning `build_basemap_resources`'s own seed has, except here "wrong" would mean a
    // stray flash of the globe before the app has decided whether to show it at all, which
    // `blend: 0.0` avoids regardless of what that decision turns out to be.
    let view_proj_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above globe view-proj uniform"),
        contents: bytemuck::bytes_of(&globe_view_proj_bytes(
            &GlobeCamera::new(width, height),
            0.0,
        )),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let view_proj_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above globe view-proj bind group"),
        layout: view_proj_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: view_proj_buffer.as_entire_binding(),
        }],
    });

    let land_pipeline = create_globe_basemap_pipeline(
        device,
        &shader,
        &pipeline_layout,
        format,
        "look-above globe basemap land fill pipeline",
    );
    let coastline_pipeline = create_globe_basemap_pipeline(
        device,
        &shader,
        &pipeline_layout,
        format,
        "look-above globe basemap coastline stroke pipeline",
    );

    let land = GlobeBasemapLayer {
        vertex_buffer: create_globe_mesh_buffer(
            device,
            &geometry.land,
            "look-above globe basemap land",
            BufferKind::Vertex,
        ),
        index_buffer: create_globe_mesh_buffer(
            device,
            &geometry.land,
            "look-above globe basemap land",
            BufferKind::Index,
        ),
        index_count: globe_index_count(&geometry.land),
        color_bind_group: create_color_bind_group(
            device,
            &color_layout,
            color::land_fill_color(format),
            "look-above globe basemap land color",
        ),
        pipeline: land_pipeline,
    };
    let coastline = GlobeBasemapLayer {
        vertex_buffer: create_globe_mesh_buffer(
            device,
            &geometry.coastline,
            "look-above globe basemap coastline",
            BufferKind::Vertex,
        ),
        index_buffer: create_globe_mesh_buffer(
            device,
            &geometry.coastline,
            "look-above globe basemap coastline",
            BufferKind::Index,
        ),
        index_count: globe_index_count(&geometry.coastline),
        color_bind_group: create_color_bind_group(
            device,
            &color_layout,
            color::coastline_stroke_color(format),
            "look-above globe basemap coastline color",
        ),
        pipeline: coastline_pipeline,
    };

    GlobeBasemapResources {
        view_proj_buffer,
        view_proj_bind_group,
        land,
        coastline,
    }
}

/// [`create_mesh_buffer`]'s globe-plane twin, operating on [`basemap::GlobeMeshData`]/
/// [`GlobeVertex`] instead of [`MeshData`]/[`Vertex`].
fn create_globe_mesh_buffer(
    device: &wgpu::Device,
    mesh: &basemap::GlobeMeshData,
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

/// [`index_count`]'s globe-plane twin.
fn globe_index_count(mesh: &basemap::GlobeMeshData) -> u32 {
    u32::try_from(mesh.indices.len()).expect("globe basemap mesh index count fits in u32")
}

/// Builds one of the two globe base-map render pipelines: `TriangleList`, `SAMPLE_COUNT`-
/// multisampled like every other pass, no depth/stencil, alpha-blended (unlike
/// [`create_basemap_pipeline`]'s opaque Mercator pipelines) — the globe basemap draws over
/// whatever the Mercator basemap already painted and must fade with `blend`, not replace it
/// outright.
fn create_globe_basemap_pipeline(
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
            buffers: &[Some(GlobeVertex::LAYOUT)],
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The raw bytes behind `globe_basemap.wgsl`/`density.wgsl`'s shared `GlobeViewProj` uniform:
/// sub-observer center (radians), the unit-disk-to-clip-space scale, and this frame's blend —
/// see [`Renderer::set_globe_params`]'s own doc comment for the derivation.
///
/// Mirrors [`camera_view_proj`]'s own scale derivation: a point at the globe's visible edge
/// (`UnitDiskXy` magnitude 1, i.e. `globe_camera.radius_px()` physical pixels from the viewport
/// center) must land exactly at the clip-space edge — `scale = 2 * radius_px / width_px` (the
/// same "world unit spans this many pixels, clip space spans `width_px / 2` pixels per unit"
/// shape `camera_view_proj`'s own `scale_x`/`scale_y` use, just with the globe's disk radius
/// standing in for `camera_view_proj`'s `meters_per_pixel`-derived world span).
fn globe_view_proj_bytes(globe_camera: &GlobeCamera, blend: f64) -> [f32; 8] {
    let center = globe_camera.center();
    let radius_px = globe_camera.radius_px();
    let scale_x = 2.0 * radius_px / globe_camera.width_px();
    let scale_y = 2.0 * radius_px / globe_camera.height_px();

    // `f64` results of division/multiplication narrowed to the uniform's `f32` storage type —
    // same reasoning `camera_view_proj`'s own final cast documents.
    #[allow(clippy::cast_possible_truncation)]
    [
        center.lat_deg.to_radians() as f32,
        center.lon_deg.to_radians() as f32,
        scale_x as f32,
        scale_y as f32,
        blend as f32,
        0.0,
        0.0,
        0.0,
    ]
}

/// Builds the L0 density-dot pass's GPU resources (M4 item 4.3): the shared unit-quad mesh, the
/// per-frame params uniform (color + screen-constant dot scale), an initial (empty,
/// [`density::MIN_DENSITY_INSTANCE_CAPACITY`]-sized) instance buffer, and the pipeline itself.
/// Runs once, in [`Renderer::new`]/`new_headless`, alongside [`build_globe_basemap_resources`].
///
/// `view_proj_layout` must be the exact same object [`build_globe_basemap_resources`] was given
/// — see [`build_basemap_resources`]'s own doc comment on why every pass sharing one bind group
/// must be built from the same `BindGroupLayout` object.
fn build_density_resources(
    device: &wgpu::Device,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> DensityLayer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above density shader"),
        source: wgpu::ShaderSource::Wgsl(DENSITY_SHADER.into()),
    });

    let params_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        "look-above density params bind group layout",
    );
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above density pipeline layout"),
        bind_group_layouts: &[Some(view_proj_layout), Some(&params_layout)],
        immediate_size: 0,
    });

    let pipeline = create_density_pipeline(device, &shader, &pipeline_layout, format);

    let quad_vertices = density::quad_vertices();
    let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above density quad vertices"),
        contents: bytemuck::cast_slice(&quad_vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let quad_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above density quad indices"),
        contents: bytemuck::cast_slice(&density::QUAD_INDICES),
        usage: wgpu::BufferUsages::INDEX,
    });

    let color = color::density_dot_color(format);
    let (params_buffer, params_bind_group) =
        build_density_params_resources(device, &params_layout, color);

    let instance_buffer =
        create_density_instance_buffer(device, density::MIN_DENSITY_INSTANCE_CAPACITY);

    DensityLayer {
        pipeline,
        params_buffer,
        params_bind_group,
        quad_vertex_buffer,
        quad_index_buffer,
        instance_buffer,
        instance_capacity: density::MIN_DENSITY_INSTANCE_CAPACITY,
        color,
        instance_scratch: Vec::new(),
    }
}

/// The `@group(1)` density params uniform (color + scale) and its bind group, seeded with
/// `color` and a zero scale — the same "nothing reads this before `Renderer::record_draw_passes`
/// has already rewritten the scale for the frame" shape [`build_marker_params_resources`] uses.
fn build_density_params_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    color: [f32; 4],
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above density params uniform"),
        contents: bytemuck::bytes_of(&density_params_bytes(color, 0.0)),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above density params bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (buffer, bind_group)
}

/// The raw bytes behind `density.wgsl`'s `DotParams` uniform: `color` (`vec4<f32>`) followed by
/// `scale` (`f32`), padded to WGSL's 16-byte uniform alignment — same idiom as
/// [`marker_params_bytes`].
fn density_params_bytes(color: [f32; 4], scale: f32) -> [f32; 8] {
    [color[0], color[1], color[2], color[3], scale, 0.0, 0.0, 0.0]
}

/// An empty density-dot instance buffer sized for `capacity` instances —
/// [`DensityLayer::upload_instances`] recreates this at a larger capacity if a frame's packed
/// aircraft outgrow it.
fn create_density_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above density instance buffer"),
        size: (capacity * size_of::<DensityDotInstanceRaw>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Builds the density pipeline: `TriangleList` over the shared unit quad ([`DensityQuadVertex`]
/// per-vertex, [`DensityDotInstanceRaw`] per-instance), `SAMPLE_COUNT`-multisampled like every
/// other pass, no depth/stencil, **additively** blended — unlike every other pass's
/// `ALPHA_BLENDING`, this is what makes overlapping dots over a busy region sum their brightness
/// in the framebuffer instead of merely occluding each other, the mechanism docs/01's "brightness
/// proportional to local count" depends on (see `density.rs`'s own module doc comment).
fn create_density_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("look-above density pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(DensityQuadVertex::LAYOUT),
                Some(DensityDotInstanceRaw::LAYOUT),
            ],
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
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
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
/// [`RunwayLayer`]/[`GlobeBasemapLayer`] both use this (their colors are genuinely fixed for the
/// pipeline's lifetime) — [`BasemapLayer`]'s own land/coastline color needs its alpha rewritten
/// every frame instead (M4 item 4.3's cross-fade), so it uses
/// [`create_rewritable_color_bind_group`] below rather than this function.
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

/// [`create_color_bind_group`]'s rewritable twin (M4 item 4.3): identical shape, except the
/// buffer carries `COPY_DST` and is handed back alongside the bind group so the caller
/// ([`build_basemap_resources`]) can stash it on [`BasemapLayer`] for
/// [`BasemapLayer::set_alpha_multiplier`] to rewrite every frame.
fn create_rewritable_color_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    color: [f32; 4],
    label: &str,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::bytes_of(&color),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (buffer, bind_group)
}

/// Builds [`RunwayLayer`]'s pipeline: `TriangleList`, `SAMPLE_COUNT`-multisampled to match every
/// other pass, no depth/stencil (there is none yet), opaque (`blend: None` — a runway outline
/// only ever matters at L2/Regional, where `mode_blend` is always `0.0`, so it has no transition
/// to participate in), drawing into a surface of `format`. Was also [`BasemapLayer`]'s own land/
/// coastline pipeline before M4 item 4.3, which moved those two to
/// [`create_basemap_pipeline_blended`] instead — see that function's own doc comment.
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

/// [`create_basemap_pipeline`]'s alpha-blended twin (M4 item 4.3), for [`BasemapLayer`]'s own
/// land/coastline pipelines only: identical shape (same shader, same vertex layout, same
/// multisample/no-depth setup), except `blend: Some(wgpu::BlendState::ALPHA_BLENDING)` so
/// [`BasemapLayer::set_alpha_multiplier`]'s per-frame alpha actually fades the layer instead of
/// unconditionally overwriting the framebuffer — the "cross-faded during transition" half of the
/// plan's own 4.3 line that this layer originally lacked (see [`BasemapLayer`]'s own doc
/// comment).
fn create_basemap_pipeline_blended(
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the aircraft glyph pass's GPU resources (M2 item 2.5): the procedurally-generated SDF
/// atlas texture, the shared unit-quad mesh, the per-frame glyph-scale uniform, and the
/// pipeline itself. Runs once, in [`Renderer::new`], alongside `build_basemap_resources`.
///
/// `view_proj_layout` must be the exact same object `build_basemap_resources` was given — see
/// that function's doc comment.
fn build_aircraft_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> AircraftLayer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above aircraft shader"),
        source: wgpu::ShaderSource::Wgsl(AIRCRAFT_SHADER.into()),
    });

    let glyph_params_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::VERTEX,
        "look-above aircraft glyph-params bind group layout",
    );
    let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("look-above aircraft atlas bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above aircraft pipeline layout"),
        bind_group_layouts: &[
            Some(view_proj_layout),
            Some(&glyph_params_layout),
            Some(&atlas_layout),
        ],
        immediate_size: 0,
    });

    let pipeline = create_aircraft_pipeline(device, &shader, &pipeline_layout, format);
    let (glyph_params_buffer, glyph_params_bind_group) =
        build_glyph_params_resources(device, &glyph_params_layout);
    let atlas_bind_group = build_atlas_bind_group(device, queue, &atlas_layout);
    let (quad_vertex_buffer, quad_index_buffer) = build_quad_mesh_buffers(device);
    let instance_buffer = create_instance_buffer(device, aircraft::MIN_INSTANCE_CAPACITY);

    AircraftLayer {
        pipeline,
        glyph_params_buffer,
        glyph_params_bind_group,
        atlas_bind_group,
        quad_vertex_buffer,
        quad_index_buffer,
        instance_buffer,
        instance_capacity: aircraft::MIN_INSTANCE_CAPACITY,
        tint_table: color::altitude_bucket_tint_table(format),
        instance_scratch: Vec::new(),
    }
}

/// The `@group(1)` glyph-scale uniform and its bind group. Seeded with zeros: unlike the
/// base-map view-proj buffer, nothing ever reads this before `Renderer::render` has already
/// rewritten it for the frame (see `AircraftLayer::set_glyph_scale`'s call site) — there is no
/// external caller that can draw a frame in the gap the base map's own seed comment describes.
fn build_glyph_params_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above aircraft glyph-params uniform"),
        contents: bytemuck::bytes_of(&[0.0_f32; 4]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above aircraft glyph-params bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (buffer, bind_group)
}

/// Rasterizes and uploads the SDF atlas texture (once, at startup) and builds its `@group(2)`
/// bind group (texture + sampler).
fn build_atlas_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::BindGroup {
    let atlas_bytes = glyph_atlas::build_atlas_bytes();
    let atlas_texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("look-above aircraft glyph atlas"),
            size: wgpu::Extent3d {
                width: glyph_atlas::ATLAS_WIDTH_PX,
                height: glyph_atlas::ATLAS_HEIGHT_PX,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &atlas_bytes,
    );
    let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
    // Linear filtering smooths the SDF's own edges further (on top of the shader's `smoothstep`
    // AA); `ClampToEdge` keeps a glyph's own tile from sampling past the atlas's outer border —
    // adjacent-tile bleed at a tile's *inner* seam is a separate, accepted tradeoff (see
    // `glyph_atlas`'s `SPREAD` doc comment).
    let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("look-above aircraft atlas sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above aircraft atlas bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&atlas_sampler),
            },
        ],
    })
}

/// The shared unit-quad mesh every aircraft instance reuses (vertex buffer, index buffer) —
/// static, built once, never rebuilt.
fn build_quad_mesh_buffers(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer) {
    let quad_vertices = aircraft::quad_vertices();
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above aircraft quad vertices"),
        contents: bytemuck::cast_slice(&quad_vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above aircraft quad indices"),
        contents: bytemuck::cast_slice(&aircraft::QUAD_INDICES),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vertex_buffer, index_buffer)
}

/// An empty instance buffer sized for `capacity` instances — [`AircraftLayer::upload_instances`]
/// recreates this at a larger capacity if a frame's feed outgrows it.
fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above aircraft instance buffer"),
        size: (capacity * size_of::<InstanceRaw>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Builds the trail ribbon pass's GPU resources (M2 item 2.6b): the pipeline and an initial
/// (empty) vertex buffer. Runs once, in [`Renderer::new`], alongside the base-map and aircraft
/// resource builders.
///
/// `view_proj_layout` must be the exact same object the other passes were built from — see
/// [`build_basemap_resources`]'s doc comment.
fn build_trail_resources(
    device: &wgpu::Device,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> TrailLayer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above trail shader"),
        source: wgpu::ShaderSource::Wgsl(TRAIL_SHADER.into()),
    });

    // Only `@group(0)` (the shared view-proj matrix): `trail.rs` bakes geometry and color on the
    // CPU, so there is no atlas or per-frame uniform for this pass to bind.
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above trail pipeline layout"),
        bind_group_layouts: &[Some(view_proj_layout)],
        immediate_size: 0,
    });

    let pipeline = create_trail_pipeline(device, &shader, &pipeline_layout, format);
    let vertex_buffer = create_trail_vertex_buffer(device, trail::MIN_TRAIL_VERTEX_CAPACITY);

    TrailLayer {
        pipeline,
        vertex_buffer,
        vertex_capacity: trail::MIN_TRAIL_VERTEX_CAPACITY,
        tint_table: color::altitude_bucket_tint_table(format),
        vertex_scratch: Vec::new(),
    }
}

/// An empty trail vertex buffer sized for `capacity` vertices — [`TrailLayer::upload_trails`]
/// recreates this at a larger capacity if a frame's tessellated trails outgrow it.
fn create_trail_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above trail vertex buffer"),
        size: (capacity * size_of::<TrailVertexRaw>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Builds the trail pipeline: `TriangleList` over the per-frame ribbon vertex buffer
/// (`trail::TrailVertexRaw` per-vertex), `SAMPLE_COUNT`-multisampled like every other pass, no
/// depth/stencil — and, like the aircraft pass and unlike the opaque base-map passes,
/// alpha-blended: the front-to-tail taper alpha needs it.
fn create_trail_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("look-above trail pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(TrailVertexRaw::LAYOUT)],
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the runway-outline pass's GPU resources (M3 item 3.2): the pipeline and an initial
/// (empty, [`airport::MIN_RUNWAY_VERTEX_CAPACITY`]/[`airport::MIN_RUNWAY_INDEX_CAPACITY`]-sized)
/// vertex/index buffer pair. Runs once, in [`Renderer::new`]/[`Renderer::new_headless`], alongside
/// the base-map resource builder.
///
/// Reuses [`BASEMAP_SHADER`] and [`create_basemap_pipeline`] verbatim rather than a new shader —
/// a runway outline is exactly another flat-shaded, per-layer-colored stroke mesh over
/// [`basemap::Vertex`], the same shape the coastline pass already is (see [`RunwayLayer`]'s own
/// doc comment for the one real difference: the buffers here are rebuilt every frame, not built
/// once). Its own shader module/pipeline layout are still built fresh here (not shared with
/// [`build_basemap_resources`]'s locals, which are not returned) — a second small one-time
/// compile at startup, not a per-frame cost.
///
/// `view_proj_layout` must be the exact same object every other pass was built from — see
/// [`build_basemap_resources`]'s doc comment.
fn build_runway_resources(
    device: &wgpu::Device,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> RunwayLayer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above runway outline shader"),
        source: wgpu::ShaderSource::Wgsl(BASEMAP_SHADER.into()),
    });

    let color_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::FRAGMENT,
        "look-above runway outline color bind group layout",
    );
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above runway outline pipeline layout"),
        bind_group_layouts: &[Some(view_proj_layout), Some(&color_layout)],
        immediate_size: 0,
    });

    let pipeline = create_basemap_pipeline(
        device,
        &shader,
        &pipeline_layout,
        format,
        "look-above runway outline pipeline",
    );
    let color_bind_group = create_color_bind_group(
        device,
        &color_layout,
        color::runway_outline_color(format),
        "look-above runway outline color",
    );

    let vertex_buffer = create_runway_vertex_buffer(device, airport::MIN_RUNWAY_VERTEX_CAPACITY);
    let index_buffer = create_runway_index_buffer(device, airport::MIN_RUNWAY_INDEX_CAPACITY);

    RunwayLayer {
        pipeline,
        color_bind_group,
        vertex_buffer,
        index_buffer,
        vertex_capacity: airport::MIN_RUNWAY_VERTEX_CAPACITY,
        index_capacity: airport::MIN_RUNWAY_INDEX_CAPACITY,
        scratch: VertexBuffers::new(),
    }
}

/// An empty runway-outline vertex buffer sized for `capacity` vertices —
/// [`RunwayLayer::upload_runways`] recreates this at a larger capacity if a frame's tessellated
/// runways outgrow it.
fn create_runway_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above runway outline vertex buffer"),
        size: (capacity * size_of::<Vertex>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// An empty runway-outline index buffer sized for `capacity` indices — see
/// [`create_runway_vertex_buffer`]'s own doc comment.
fn create_runway_index_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above runway outline index buffer"),
        size: (capacity * size_of::<u32>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Builds the airport-marker pass's GPU resources (M3 item 3.2): the shared unit-circle mesh
/// ([`airport::marker_mesh`]), the per-frame params uniform (color + screen-constant scale), an
/// initial (empty, [`airport::MIN_AIRPORT_INSTANCE_CAPACITY`]-sized) instance buffer, and the
/// pipeline itself. Runs once, in [`Renderer::new`]/[`Renderer::new_headless`], alongside the
/// runway-outline resource builder.
///
/// `view_proj_layout` must be the exact same object every other pass was built from — see
/// [`build_basemap_resources`]'s doc comment.
fn build_airport_marker_resources(
    device: &wgpu::Device,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> AirportMarkerLayer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above airport marker shader"),
        source: wgpu::ShaderSource::Wgsl(AIRPORT_SHADER.into()),
    });

    let params_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        "look-above airport marker params bind group layout",
    );
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above airport marker pipeline layout"),
        bind_group_layouts: &[Some(view_proj_layout), Some(&params_layout)],
        immediate_size: 0,
    });

    let pipeline = create_airport_marker_pipeline(device, &shader, &pipeline_layout, format);

    let mesh = airport::marker_mesh();
    let mesh_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above airport marker mesh vertices"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let mesh_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above airport marker mesh indices"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    let mesh_index_count = index_count(&mesh);

    let color = color::airport_marker_color(format);
    let (params_buffer, params_bind_group) =
        build_marker_params_resources(device, &params_layout, color);

    let instance_buffer =
        create_airport_instance_buffer(device, airport::MIN_AIRPORT_INSTANCE_CAPACITY);

    AirportMarkerLayer {
        pipeline,
        params_buffer,
        params_bind_group,
        mesh_vertex_buffer,
        mesh_index_buffer,
        mesh_index_count,
        instance_buffer,
        instance_capacity: airport::MIN_AIRPORT_INSTANCE_CAPACITY,
        instance_scratch: Vec::new(),
        color,
    }
}

/// The `@group(1)` marker-params uniform (color + scale) and its bind group, seeded with `color`
/// and a zero scale — like the aircraft pass's glyph-params uniform, nothing reads this before
/// `Renderer::render` has already rewritten the scale for the frame (see
/// [`AirportMarkerLayer::set_params`]'s call site in [`Renderer::record_draw_passes`]).
fn build_marker_params_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    color: [f32; 4],
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above airport marker params uniform"),
        contents: bytemuck::bytes_of(&marker_params_bytes(color, 0.0)),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above airport marker params bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (buffer, bind_group)
}

/// The raw bytes behind `airport.wgsl`'s `MarkerParams` uniform: `color` (`vec4<f32>`) followed
/// by `scale` (`f32`), padded to WGSL's 16-byte uniform alignment — the same "plain `f32` array,
/// no dedicated repr(C) struct" idiom [`AircraftLayer::set_glyph_scale`]'s own buffer write uses.
fn marker_params_bytes(color: [f32; 4], scale: f32) -> [f32; 8] {
    [color[0], color[1], color[2], color[3], scale, 0.0, 0.0, 0.0]
}

/// An empty airport-marker instance buffer sized for `capacity` instances —
/// [`AirportMarkerLayer::upload_instances`] recreates this at a larger capacity if a frame's
/// queried airport set outgrows it.
fn create_airport_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above airport marker instance buffer"),
        size: (capacity * size_of::<AirportInstanceRaw>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Builds the airport marker pipeline: `TriangleList` over the shared unit-circle mesh
/// (`basemap::Vertex` per-vertex, `airport::AirportInstanceRaw` per-instance),
/// `SAMPLE_COUNT`-multisampled like every other pass, no depth/stencil, alpha-blended (MSAA's own
/// edge antialiasing on a small circle benefits from it, the same reasoning the aircraft
/// pipeline's blend state already has).
fn create_airport_marker_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("look-above airport marker pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(Vertex::LAYOUT), Some(AirportInstanceRaw::LAYOUT)],
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the METAR badge pass's GPU resources (M3 item 3.3): a second, independent copy of the
/// shared unit-circle mesh ([`airport::marker_mesh`] — see [`metar_badge`]'s module doc comment
/// for why this layer does not just borrow [`AirportMarkerLayer`]'s own copy), the per-frame
/// scale-only params uniform, an initial (empty,
/// [`metar_badge::MIN_BADGE_INSTANCE_CAPACITY`]-sized) instance buffer, and the pipeline itself.
/// Runs once, in [`Renderer::new`]/[`Renderer::new_headless`], alongside the airport-marker
/// resource builder.
///
/// `view_proj_layout` must be the exact same object every other pass was built from — see
/// [`build_basemap_resources`]'s doc comment.
fn build_badge_resources(
    device: &wgpu::Device,
    view_proj_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> BadgeLayer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above metar badge shader"),
        source: wgpu::ShaderSource::Wgsl(METAR_BADGE_SHADER.into()),
    });

    let params_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::VERTEX,
        "look-above metar badge params bind group layout",
    );
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above metar badge pipeline layout"),
        bind_group_layouts: &[Some(view_proj_layout), Some(&params_layout)],
        immediate_size: 0,
    });

    let pipeline = create_badge_pipeline(device, &shader, &pipeline_layout, format);

    let mesh = airport::marker_mesh();
    let mesh_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above metar badge mesh vertices"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let mesh_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above metar badge mesh indices"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    let mesh_index_count = index_count(&mesh);

    let (params_buffer, params_bind_group) = build_badge_params_resources(device, &params_layout);

    let instance_buffer =
        create_badge_instance_buffer(device, metar_badge::MIN_BADGE_INSTANCE_CAPACITY);

    BadgeLayer {
        pipeline,
        params_buffer,
        params_bind_group,
        mesh_vertex_buffer,
        mesh_index_buffer,
        mesh_index_count,
        instance_buffer,
        instance_capacity: metar_badge::MIN_BADGE_INSTANCE_CAPACITY,
        instance_scratch: Vec::new(),
    }
}

/// The `@group(1)` badge-params uniform (scale only) and its bind group, seeded with a zero
/// scale — nothing reads this before `Renderer::render` has already rewritten it for the frame
/// (see [`BadgeLayer::set_params`]'s call site in [`Renderer::record_draw_passes`]), the same
/// "seeded at zero, rewritten before first read" shape [`build_marker_params_resources`] uses.
fn build_badge_params_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above metar badge params uniform"),
        contents: bytemuck::bytes_of(&badge_params_bytes(0.0)),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above metar badge params bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (buffer, bind_group)
}

/// The raw bytes behind `metar_badge.wgsl`'s `BadgeParams` uniform: `scale` (`f32`), padded to
/// WGSL's 16-byte uniform alignment — the same idiom [`marker_params_bytes`]/
/// [`AircraftLayer::set_glyph_scale`]'s own buffer writes use.
fn badge_params_bytes(scale: f32) -> [f32; 4] {
    [scale, 0.0, 0.0, 0.0]
}

/// An empty badge instance buffer sized for `capacity` instances — [`BadgeLayer::upload_instances`]
/// recreates this at a larger capacity if a frame's queried badge set outgrows it.
fn create_badge_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above metar badge instance buffer"),
        size: (capacity * size_of::<BadgeInstanceRaw>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Builds the badge pipeline: `TriangleList` over the shared unit-circle mesh
/// (`basemap::Vertex` per-vertex, `metar_badge::BadgeInstanceRaw` per-instance),
/// `SAMPLE_COUNT`-multisampled and alpha-blended like [`create_airport_marker_pipeline`] — the
/// same MSAA-edge-antialiasing reasoning applies to a badge ring's own small circular geometry.
fn create_badge_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("look-above metar badge pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(Vertex::LAYOUT), Some(BadgeInstanceRaw::LAYOUT)],
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the aircraft pipeline: `TriangleList` over the shared quad mesh (`aircraft::QuadVertex`
/// per-vertex, `aircraft::InstanceRaw` per-instance), `SAMPLE_COUNT`-multisampled like every
/// other pass, no depth/stencil — but unlike the (opaque) base-map pipelines, alpha-blended: the
/// SDF's own edge antialiasing and the stale-fade alpha both need it.
fn create_aircraft_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("look-above aircraft pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(QuadVertex::LAYOUT), Some(InstanceRaw::LAYOUT)],
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the label pass's GPU resources (M2 item 2.7b): the procedurally-generated stroke-font
/// SDF atlas, the shared unit text-quad mesh, the screen-size uniform, and both label pipelines.
/// Runs once, in [`Renderer::new`], alongside the base-map/trail/aircraft resource builders.
///
/// Unlike [`build_aircraft_resources`]/[`build_trail_resources`], this does *not* take the
/// shared `view_proj_layout`: the label pass reads a screen-size uniform instead of the world
/// view-proj matrix (see [`LabelLayer`]'s own doc comment), so it needs no bind-group-layout
/// compatibility with the other passes.
fn build_label_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
) -> LabelLayer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-above label shader"),
        source: wgpu::ShaderSource::Wgsl(LABEL_SHADER.into()),
    });

    let screen_params_layout = create_uniform_bind_group_layout(
        device,
        wgpu::ShaderStages::VERTEX,
        "look-above label screen-params bind group layout",
    );
    let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("look-above label atlas bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let text_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above label text pipeline layout"),
        bind_group_layouts: &[Some(&screen_params_layout), Some(&atlas_layout)],
        immediate_size: 0,
    });
    let leader_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-above label leader pipeline layout"),
        bind_group_layouts: &[Some(&screen_params_layout)],
        immediate_size: 0,
    });

    let text_pipeline = create_label_text_pipeline(device, &shader, &text_pipeline_layout, format);
    let leader_pipeline =
        create_label_leader_pipeline(device, &shader, &leader_pipeline_layout, format);

    // Seeded with zeros: like the aircraft pass's glyph-params uniform, nothing reads this
    // before `Renderer::render` has already rewritten it for the frame (`LabelLayer::draw` is
    // never called before `LabelLayer::set_screen_params`/`update` run first in `render`).
    let screen_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above label screen-params uniform"),
        contents: bytemuck::bytes_of(&[0.0_f32; 4]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let screen_params_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above label screen-params bind group"),
        layout: &screen_params_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: screen_params_buffer.as_entire_binding(),
        }],
    });

    let atlas_bind_group = build_label_atlas_bind_group(device, queue, &atlas_layout);
    let (quad_vertex_buffer, quad_index_buffer) = build_label_quad_mesh_buffers(device);
    let text_instance_buffer =
        create_text_instance_buffer(device, label::MIN_TEXT_INSTANCE_CAPACITY);
    let leader_vertex_buffer =
        create_leader_vertex_buffer(device, label::MIN_LEADER_VERTEX_CAPACITY);

    LabelLayer {
        text_pipeline,
        leader_pipeline,
        screen_params_buffer,
        screen_params_bind_group,
        atlas_bind_group,
        quad_vertex_buffer,
        quad_index_buffer,
        text_instance_buffer,
        text_instance_capacity: label::MIN_TEXT_INSTANCE_CAPACITY,
        text_instance_scratch: Vec::new(),
        leader_vertex_buffer,
        leader_vertex_capacity: label::MIN_LEADER_VERTEX_CAPACITY,
        leader_vertex_scratch: Vec::new(),
        text_color: color::label_text_color(format),
        leader_color: color::label_leader_color(format),
        held: HashSet::new(),
        last_eval_s: None,
        cached_placements: Vec::new(),
    }
}

/// Builds the F3 debug HUD's GPU resources (M2 item 2.1b) by *cloning* `label`'s already-built
/// text pipeline, atlas bind group, shared text-quad mesh, and screen-params bind group — all
/// cheap `Arc`-backed `wgpu` handles — rather than rasterizing a second stroke-font atlas or
/// compiling a second pipeline (see [`StatsOverlayLayer`]'s own doc comment). Runs once, in
/// [`Renderer::new`], after `label` itself has been built.
fn build_stats_overlay_resources(
    device: &wgpu::Device,
    label: &LabelLayer,
    format: wgpu::TextureFormat,
) -> StatsOverlayLayer {
    let instance_buffer =
        create_text_instance_buffer(device, stats_overlay::MIN_OVERLAY_INSTANCE_CAPACITY);

    StatsOverlayLayer {
        text_pipeline: label.text_pipeline.clone(),
        screen_params_bind_group: label.screen_params_bind_group.clone(),
        atlas_bind_group: label.atlas_bind_group.clone(),
        quad_vertex_buffer: label.quad_vertex_buffer.clone(),
        quad_index_buffer: label.quad_index_buffer.clone(),
        instance_buffer,
        instance_capacity: stats_overlay::MIN_OVERLAY_INSTANCE_CAPACITY,
        instance_scratch: Vec::new(),
        text_color: color::stats_overlay_text_color(format),
    }
}

/// Builds the selected-aircraft info card's GPU resources (M2 item 2.8b) the same way
/// [`build_stats_overlay_resources`] does: *cloning* `label`'s already-built text
/// pipeline/atlas/mesh/screen-params bind group rather than a second SDF atlas or pipeline (see
/// [`InfoCardLayer`]'s own doc comment). Runs once, in [`Renderer::new`], after `label` itself has
/// been built.
fn build_info_card_resources(
    device: &wgpu::Device,
    label: &LabelLayer,
    format: wgpu::TextureFormat,
) -> InfoCardLayer {
    let instance_buffer =
        create_text_instance_buffer(device, stats_overlay::MIN_OVERLAY_INSTANCE_CAPACITY);

    InfoCardLayer {
        text_pipeline: label.text_pipeline.clone(),
        screen_params_bind_group: label.screen_params_bind_group.clone(),
        atlas_bind_group: label.atlas_bind_group.clone(),
        quad_vertex_buffer: label.quad_vertex_buffer.clone(),
        quad_index_buffer: label.quad_index_buffer.clone(),
        instance_buffer,
        instance_capacity: stats_overlay::MIN_OVERLAY_INSTANCE_CAPACITY,
        instance_scratch: Vec::new(),
        text_color: color::info_card_text_color(format),
    }
}

/// Rasterizes and uploads the stroke-font SDF atlas texture (once, at startup) and builds its
/// `@group(1)` bind group (texture + sampler) — the label-pass analog of
/// [`build_atlas_bind_group`], reading [`label_atlas`] instead of [`glyph_atlas`].
fn build_label_atlas_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::BindGroup {
    let atlas_bytes = label_atlas::build_atlas_bytes();
    let atlas_texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("look-above label glyph atlas"),
            size: wgpu::Extent3d {
                width: label_atlas::ATLAS_WIDTH_PX,
                height: label_atlas::ATLAS_HEIGHT_PX,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &atlas_bytes,
    );
    let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("look-above label atlas sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("look-above label atlas bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&atlas_sampler),
            },
        ],
    })
}

/// The shared unit text-quad mesh every character cell reuses (vertex buffer, index buffer) —
/// static, built once, never rebuilt. The label-pass analog of [`build_quad_mesh_buffers`].
fn build_label_quad_mesh_buffers(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer) {
    let quad_vertices = label::text_quad_vertices();
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above label quad vertices"),
        contents: bytemuck::cast_slice(&quad_vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("look-above label quad indices"),
        contents: bytemuck::cast_slice(&label::TEXT_QUAD_INDICES),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vertex_buffer, index_buffer)
}

/// An empty text-instance buffer sized for `capacity` characters —
/// [`LabelLayer::update`] recreates this at a larger capacity if a frame's labels outgrow it.
fn create_text_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above label text instance buffer"),
        size: (capacity * size_of::<TextInstanceRaw>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// An empty leader-line vertex buffer sized for `capacity` vertices — [`LabelLayer::update`]
/// recreates this at a larger capacity if a frame's leader lines outgrow it.
fn create_leader_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("look-above label leader vertex buffer"),
        size: (capacity * size_of::<LeaderVertexRaw>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Builds the label text pipeline: `TriangleList` over the shared text-quad mesh
/// (`label::TextQuadVertex` per-vertex, `label::TextInstanceRaw` per-instance),
/// `SAMPLE_COUNT`-multisampled like every other pass, no depth/stencil, alpha-blended (the SDF's
/// own edge antialiasing needs it, same as the aircraft pipeline).
fn create_label_text_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("look-above label text pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_text"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(TextQuadVertex::LAYOUT), Some(TextInstanceRaw::LAYOUT)],
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
            entry_point: Some("fs_text"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the leader-line pipeline: a `LineList` over `label::LeaderVertexRaw` (two vertices per
/// line, no instancing — `renderer.rs` draws every displaced label's leader in one call),
/// `SAMPLE_COUNT`-multisampled, alpha-blended (the leader color's own reduced alpha needs it).
fn create_label_leader_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("look-above label leader pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_leader"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(LeaderVertexRaw::LAYOUT)],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: SAMPLE_COUNT,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_leader"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Builds the base map's `view_proj` uniform from a [`Camera`]'s current state.
///
/// `basemap::tessellate`'s static mesh is pre-normalized by dividing every Web Mercator metre
/// coordinate by [`WEB_MERCATOR_EXTENT_M`] (see that module's doc comment) — this matrix must
/// therefore operate on that already-divided plane, not on raw metres, which is why
/// `center_plane_*` below re-divides the camera's center by the same extent before use.
///
/// Derivation: a camera viewport spans `meters_per_pixel * width_px` world metres across, so a
/// point `EXTENT` metres from the camera center should land exactly at the clip-space edge
/// (`+1`/`-1`) when it is `width_px / 2` pixels from the viewport center — giving
/// `scale = EXTENT / (meters_per_pixel * width_px / 2)`. The translation recenters the plane
/// on the camera before that scale is applied, in the same pre-normalized units.
///
/// Continuity with the old placeholder: for a freshly-constructed `Camera::new(w, h)` (centered
/// on the origin, framed to contain the whole projected world — see [`Camera`]'s own doc
/// comment), this produces exactly the aspect-correcting "contain" fit the M2 2.2b placeholder
/// (`fit_to_window_matrix`, since removed) used to hardcode — pinned by this module's tests.
///
/// Column-major, matching WGSL's `mat4x4<f32>` convention.
pub fn camera_view_proj(camera: &Camera) -> [[f32; 4]; 4] {
    let center = camera.center_m();
    let mpp = camera.meters_per_pixel();
    let width_px = camera.width_px();
    let height_px = camera.height_px();

    let scale_x = WEB_MERCATOR_EXTENT_M / (mpp * width_px / 2.0);
    let scale_y = WEB_MERCATOR_EXTENT_M / (mpp * height_px / 2.0);
    let center_plane_x = center.x_m / WEB_MERCATOR_EXTENT_M;
    let center_plane_y = center.y_m / WEB_MERCATOR_EXTENT_M;
    let tx = -center_plane_x * scale_x;
    let ty = -center_plane_y * scale_y;

    // `scale_x`/`scale_y`/`tx`/`ty` are ordinary `f64` results of division/multiplication, not
    // integer casts, so `clippy::cast_precision_loss` (which only fires on int-to-float casts)
    // does not apply here; `f64 as f32` is exactly the intended narrowing to the uniform's
    // storage type; and the standard reduced `f32` precision at these magnitudes is well within
    // sub-pixel visual tolerance.
    #[allow(clippy::cast_possible_truncation)]
    [
        [scale_x as f32, 0.0, 0.0, 0.0],
        [0.0, scale_y as f32, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [tx as f32, ty as f32, 0.0, 1.0],
    ]
}

#[cfg(test)]
mod tests {
    use look_above_core::geo::MAX_MERCATOR_LAT_DEG;

    use super::*;

    /// Generous next to `1.0`, tight next to what an `f32` roundtrip through this matrix could
    /// visibly shift a point by at ordinary window sizes.
    const EPS: f32 = 1e-5;

    #[track_caller]
    fn assert_close(actual: f32, expected: f32, eps: f32) {
        assert!(
            (actual - expected).abs() <= eps,
            "expected {expected}, got {actual} (difference {}, tolerance {eps})",
            (actual - expected).abs()
        );
    }

    // --- Continuity with the old fit_to_window_matrix placeholder --------------

    #[test]
    fn default_camera_on_a_wide_window_matches_the_old_placeholder_fit() {
        let matrix = camera_view_proj(&Camera::new(2000, 1000));
        let aspect = 2000.0_f32 / 1000.0;
        assert_close(matrix[0][0], 1.0 / aspect, EPS);
        assert_close(matrix[1][1], 1.0, EPS);
        assert_close(matrix[3][0], 0.0, EPS);
        assert_close(matrix[3][1], 0.0, EPS);
    }

    #[test]
    fn default_camera_on_a_tall_window_matches_the_old_placeholder_fit() {
        let matrix = camera_view_proj(&Camera::new(800, 1600));
        let aspect = 800.0_f32 / 1600.0;
        assert_close(matrix[0][0], 1.0, EPS);
        assert_close(matrix[1][1], aspect, EPS);
        assert_close(matrix[3][0], 0.0, EPS);
        assert_close(matrix[3][1], 0.0, EPS);
    }

    #[test]
    fn default_camera_on_a_square_window_scales_both_axes_identically() {
        let matrix = camera_view_proj(&Camera::new(1000, 1000));
        assert_close(matrix[0][0], 1.0, EPS);
        assert_close(matrix[1][1], 1.0, EPS);
    }

    // --- Panned/zoomed camera: pin against the hand-derived formula -------------

    // Same narrowing as `camera_view_proj` itself: the hand-derived expectations below are
    // ordinary `f64` arithmetic results, not integer casts, so `cast_possible_truncation` is
    // the applicable (and expected) lint, exactly as clippy flags in the function under test —
    // allowed here for the same reason. `expected_translate_x`/`_y` are also long enough that
    // the similar-names heuristic's single-character-suffix trigger has no real confusion risk.
    #[allow(clippy::cast_possible_truncation, clippy::similar_names)]
    #[test]
    fn panned_camera_translates_the_plane_opposite_its_center() {
        let mut camera = Camera::new(1000, 800);
        camera.pan_by_pixels(100.0, 0.0);
        camera.pan_by_pixels(0.0, 50.0);

        let matrix = camera_view_proj(&camera);
        let center = camera.center_m();
        let mpp = camera.meters_per_pixel();

        let expected_scale_x = (WEB_MERCATOR_EXTENT_M / (mpp * camera.width_px() / 2.0)) as f32;
        let expected_scale_y = (WEB_MERCATOR_EXTENT_M / (mpp * camera.height_px() / 2.0)) as f32;
        let expected_translate_x =
            (-(center.x_m / WEB_MERCATOR_EXTENT_M) * f64::from(expected_scale_x)) as f32;
        let expected_translate_y =
            (-(center.y_m / WEB_MERCATOR_EXTENT_M) * f64::from(expected_scale_y)) as f32;

        assert_close(matrix[0][0], expected_scale_x, EPS);
        assert_close(matrix[1][1], expected_scale_y, EPS);
        assert_close(matrix[3][0], expected_translate_x, EPS);
        assert_close(matrix[3][1], expected_translate_y, EPS);

        // A pan away from the origin must move the translation off zero, or this test would
        // pass vacuously.
        assert!(matrix[3][0].abs() > EPS);
        assert!(matrix[3][1].abs() > EPS);
    }

    #[test]
    fn zoomed_in_camera_scales_up_relative_to_the_default_fit() {
        let default_matrix = camera_view_proj(&Camera::new(1000, 800));

        let mut camera = Camera::new(1000, 800);
        camera.zoom_by_notches(10.0, 500.0, 400.0);
        for _ in 0..500 {
            camera.update(1.0 / 60.0);
        }
        let zoomed_matrix = camera_view_proj(&camera);

        assert!(zoomed_matrix[0][0] > default_matrix[0][0]);
        assert!(zoomed_matrix[1][1] > default_matrix[1][1]);
    }

    /// A camera panned near the Mercator latitude limit still produces a finite, sane matrix —
    /// guards against the plane-normalization arithmetic blowing up at the projection's edges.
    #[test]
    fn camera_near_the_mercator_latitude_limit_stays_finite() {
        let mut camera = Camera::new(1000, 800);
        let far_north = look_above_core::geo::web_mercator_forward(
            look_above_core::geo::LatLon::new(MAX_MERCATOR_LAT_DEG - 1.0, 0.0),
        );
        camera.pan_by_pixels(0.0, -(far_north.y_m / camera.meters_per_pixel()));

        let matrix = camera_view_proj(&camera);
        for column in matrix {
            for value in column {
                assert!(value.is_finite());
            }
        }
    }

    // --- Renderer smoke test (docs/10 §4, M2 item 2.9) ------------------------------------------

    use look_above_core::contracts::AircraftCategory;
    use look_above_core::geo::MercatorXy;
    use look_above_core::sim::AltitudeBucket;
    use look_above_core::types::{CallSign, SourceId};

    const SMOKE_TEST_WIDTH: u32 = 800;
    const SMOKE_TEST_HEIGHT: u32 = 600;
    const SMOKE_TEST_AIRCRAFT_COUNT: usize = 1_000;

    /// A tiny deterministic PRNG (splitmix64) for the synthetic feed below. This workspace has
    /// no `rand` dependency (`CLAUDE.md`'s "no new dependency for something this small"), and a
    /// fixed-seed reproducible spread is all a smoke test's synthetic fixture needs — not
    /// statistically rigorous randomness.
    struct Lcg(u64);

    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// A pseudo-random value in `[0, 1)`, the top 53 bits of [`Lcg::next_u64`] as an exact
        /// `f64` fraction.
        fn next_unit(&mut self) -> f64 {
            // `next_u64() >> 11` is exactly 53 bits, and `1u64 << 53` is an exact power of two
            // — both fit `f64`'s 53-bit mantissa exactly, so this narrowing loses nothing.
            #[allow(
                clippy::cast_precision_loss,
                reason = "both operands are exact integers within f64's 53-bit mantissa range"
            )]
            {
                (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
            }
        }

        fn next_range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + self.next_unit() * (hi - lo)
        }
    }

    /// Builds a fixed-seed, deterministic 1,000-aircraft [`RenderFeed`] spread across a
    /// plausible Web Mercator range, with varied altitude buckets/headings/categories/sources and
    /// a handful of trail vertices per aircraft (so the trail pass isn't trivially empty) —
    /// docs/10 §4's synthetic fixture for the renderer smoke test below.
    fn synthetic_render_feed() -> RenderFeed {
        const ALTITUDE_BUCKETS: [AltitudeBucket; 6] = [
            AltitudeBucket::Ground,
            AltitudeBucket::Below2000Ft,
            AltitudeBucket::To10000Ft,
            AltitudeBucket::To28000Ft,
            AltitudeBucket::To40000Ft,
            AltitudeBucket::Above40000Ft,
        ];
        const CATEGORIES: [AircraftCategory; 6] = [
            AircraftCategory::Jet,
            AircraftCategory::Turboprop,
            AircraftCategory::Piston,
            AircraftCategory::Heli,
            AircraftCategory::Glider,
            AircraftCategory::Unknown,
        ];
        const SOURCES: [SourceId; 3] = [
            SourceId::OpenSky,
            SourceId::AirplanesLive,
            SourceId::AdsbLol,
        ];
        // A short synthetic trail history per aircraft, near its own current position — enough
        // real geometry for the ribbon pass to tessellate, not a literal replay of `sim`'s own
        // dead-reckoning (out of scope for a renderer-only fixture).
        const TRAIL_AGES_S: [f64; 5] = [0.0, 10.0, 20.0, 30.0, 40.0];

        // Comfortably inside the Mercator square, not right at its edge (where a "contain"-fit
        // default camera would clip) — a realistic spread, not an edge case.
        let position_extent_m = WEB_MERCATOR_EXTENT_M * 0.8;

        let mut rng = Lcg(0x5EED_1234_ABCD_0001);
        let mut aircraft = Vec::with_capacity(SMOKE_TEST_AIRCRAFT_COUNT);
        let mut trails = Vec::with_capacity(SMOKE_TEST_AIRCRAFT_COUNT * TRAIL_AGES_S.len());

        for i in 0..SMOKE_TEST_AIRCRAFT_COUNT {
            // Zero-padded hex, ascending with `i` — already in the address-sorted order
            // `core::sim::RenderFeed`'s own doc comment documents `aircraft`/`trails` as kept
            // in, so this needs no separate sort.
            let icao24 = Icao24::from_hex(&format!("{i:06x}")).expect("valid synthetic ICAO24");
            let bucket = ALTITUDE_BUCKETS[i % ALTITUDE_BUCKETS.len()];
            let category = CATEGORIES[i % CATEGORIES.len()];
            let source = SOURCES[i % SOURCES.len()];

            let x_m = rng.next_range(-position_extent_m, position_extent_m);
            let y_m = rng.next_range(-position_extent_m, position_extent_m);
            let heading_deg = rng.next_range(0.0, 360.0);
            let altitude_ft = rng.next_range(0.0, 45_000.0);
            let ground_speed_kt = rng.next_range(80.0, 550.0);

            aircraft.push(AircraftInstance {
                icao24,
                position: MercatorXy::new(x_m, y_m),
                heading_deg,
                altitude_bucket: bucket,
                category,
                alpha: 1.0,
                on_ground: bucket == AltitudeBucket::Ground,
                anonymous: false,
                callsign: CallSign::new(&format!("TST{i:04}")),
                altitude_ft: Some(altitude_ft),
                ground_speed_kt: Some(ground_speed_kt),
                selected: i == 0,
                source,
            });

            for &age_s in &TRAIL_AGES_S {
                let offset_m = (age_s / 10.0) * 400.0;
                trails.push(TrailVertex {
                    icao24,
                    position: MercatorXy::new(x_m - offset_m, y_m - offset_m * 0.5),
                    altitude_bucket: bucket,
                    age_s,
                });
            }
        }

        RenderFeed {
            frame_ts: 1_700_000_000.0,
            aircraft,
            trails,
        }
    }

    /// `color`'s channels as the `u8` bytes an `Rgba8Unorm` surface actually stores them as
    /// (the headless smoke test's fixed offscreen format — see [`Renderer::new_headless`]) —
    /// used to recognize "background" pixels in the readback below.
    fn color_bytes(color: wgpu::Color) -> [u8; 4] {
        // Every channel `color::clear_color` produces is already in `[0, 1]` by construction
        // (an authored sRGB byte divided by 255) — the same narrowing `color.rs`'s own
        // `layer_color` does down to `f32`, just one step further to the GPU's actual 8-bit
        // storage.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "each channel is clamped to [0, 1] before scaling by 255, so the rounded \
                      result always fits u8"
        )]
        let channel = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        [
            channel(color.r),
            channel(color.g),
            channel(color.b),
            channel(color.a),
        ]
    }

    fn count_non_background(pixels: &[[u8; 4]], background: [u8; 4]) -> usize {
        pixels.iter().filter(|&&pixel| pixel != background).count()
    }

    /// docs/10 §4's renderer smoke test: a headless (fallback-adapter) render of a synthetic
    /// 1,000-aircraft [`RenderFeed`] to an offscreen texture, asserting pipeline creation
    /// succeeds, `render_headless` reports a drawn frame (not an error), and the pixel count the
    /// aircraft/trails/labels add over the bare base map falls within an expected band — wide
    /// enough not to be flaky, tight enough to still catch "renders nothing" (≈0 added) and
    /// "renders garbage everywhere" (≈the whole frame added).
    ///
    /// Skips (with a warning, not a failure) when no fallback adapter is available — the expected
    /// outcome on most CI runners today (see [`Renderer::new_headless`]'s own doc comment and
    /// this crate's CI decision log entry).
    #[test]
    fn renderer_smoke_test_headless_1000_aircraft() {
        let mut renderer = match Renderer::new_headless(SMOKE_TEST_WIDTH, SMOKE_TEST_HEIGHT) {
            Ok(renderer) => renderer,
            Err(RenderError::NoAdapter(error)) => {
                eprintln!(
                    "SKIP renderer_smoke_test_headless_1000_aircraft: no fallback GPU adapter \
                     available ({error}) — see docs/10 §4's \"skipped, not failed\" wording"
                );
                return;
            }
            Err(error) => panic!("headless renderer setup failed: {error}"),
        };

        let camera = Camera::new(SMOKE_TEST_WIDTH, SMOKE_TEST_HEIGHT);
        let background = color_bytes(color::clear_color(renderer.format()));

        // Baseline: the base map alone (background + land + coastline), no aircraft — this
        // isolates what the 1,000-aircraft feed itself adds below, rather than needing to
        // predict the (static, real-world-coastline-derived) base map's own pixel footprint.
        let empty_feed = RenderFeed {
            frame_ts: 0.0,
            ..RenderFeed::default()
        };
        let baseline_outcome = renderer.render_headless(&empty_feed, &camera);
        assert_eq!(baseline_outcome, FrameOutcome::Presented);
        let baseline_pixels = renderer.read_offscreen_pixels();
        let baseline_non_background = count_non_background(&baseline_pixels, background);

        let feed = synthetic_render_feed();
        let outcome = renderer.render_headless(&feed, &camera);
        assert_eq!(outcome, FrameOutcome::Presented);
        let pixels = renderer.read_offscreen_pixels();
        let expected_pixel_count = (SMOKE_TEST_WIDTH * SMOKE_TEST_HEIGHT) as usize;
        assert_eq!(pixels.len(), expected_pixel_count);
        let non_background = count_non_background(&pixels, background);

        // Band chosen from an actual run of this test on the DX12 WARP fallback adapter
        // (`AdapterInfo { name: "Microsoft Basic Render Driver", device_type: Cpu, backend: \
        // Dx12, .. }`), which measured `aircraft_non_background = 86,817` (of 480,000 total,
        // `baseline_non_background = 146,868`) — see this crate's own CI decision log entry.
        // `[20_000, 250_000)` keeps roughly a 4x margin below that measurement and a 3x margin
        // above it: loose enough to absorb a different fallback adapter's own AA/rounding
        // behavior, tight enough that "renders nothing" (≈0) and "renders garbage everywhere"
        // (≈333,132, the frame's non-baseline pixels) both land clearly outside it.
        let aircraft_non_background = non_background.saturating_sub(baseline_non_background);
        assert!(
            aircraft_non_background > 20_000,
            "1,000 aircraft painted implausibly few pixels ({aircraft_non_background}) over the \
             base map alone ({baseline_non_background} of {expected_pixel_count}) — looks like \
             \"renders nothing\""
        );
        assert!(
            aircraft_non_background < 250_000,
            "1,000 aircraft painted implausibly many pixels ({aircraft_non_background}) over the \
             base map alone ({baseline_non_background} of {expected_pixel_count}) — looks like \
             \"renders garbage everywhere\""
        );
    }

    /// Regression test for a live visual bug found after M4 item 4.3 first landed: at full L0
    /// (`blend: 1.0`), the pre-existing flat Mercator `BasemapLayer` (land/coastline) kept
    /// drawing fully opaque — never gated by `blend` at all — so it stayed visible in the screen
    /// corners outside the globe's inscribed disk (most visibly the Mercator-stretched polar
    /// regions, e.g. Greenland, right where a circle inscribed in a square leaves its corners
    /// exposed), making the view read as "the whole flat world map" rather than a clipped
    /// hemisphere. The existing smoke test above never caught this: it never calls
    /// `set_globe_params`, so `globe_blend` stays at its default `0.0` and
    /// `GlobeBasemapLayer::draw`'s own early return meant the globe pass — and the bug in what
    /// sits *underneath* it — was never actually exercised.
    ///
    /// At a window wider than it is tall (mirroring `App::draw`'s real default framing, where the
    /// globe's inscribed-circle disk leaves the widest margin), asserts two things at `blend:
    /// 1.0`: every pixel outside the disk (with a small margin past `radius_px` to tolerate MSAA
    /// edge antialiasing right at the boundary) is *exactly* the background/clear color — proving
    /// the flat map is now fully faded out there, not just visually similar — and a meaningful
    /// share of pixels *inside* the disk are non-background, guarding against the opposite
    /// regression (the globe pass itself silently drawing nothing).
    #[test]
    fn globe_mode_fades_out_the_flat_mercator_map_outside_the_disk() {
        const WIDTH: u32 = 1_200;
        const HEIGHT: u32 = 600;

        let mut renderer = match Renderer::new_headless(WIDTH, HEIGHT) {
            Ok(renderer) => renderer,
            Err(RenderError::NoAdapter(error)) => {
                eprintln!(
                    "SKIP globe_mode_fades_out_the_flat_mercator_map_outside_the_disk: no \
                     fallback GPU adapter available ({error})"
                );
                return;
            }
            Err(error) => panic!("headless renderer setup failed: {error}"),
        };

        let camera = Camera::new(WIDTH, HEIGHT);
        let globe_camera = GlobeCamera::new(WIDTH, HEIGHT);
        renderer.set_globe_params(&globe_camera, 1.0);
        let background = color_bytes(color::clear_color(renderer.format()));

        let empty_feed = RenderFeed {
            frame_ts: 0.0,
            ..RenderFeed::default()
        };
        let outcome = renderer.render_headless(&empty_feed, &camera);
        assert_eq!(outcome, FrameOutcome::Presented);
        let pixels = renderer.read_offscreen_pixels();

        let width = WIDTH as usize;
        let height = HEIGHT as usize;
        let radius_px = globe_camera.radius_px();
        // A few pixels past the true edge: MSAA-resolved antialiasing at the disk boundary can
        // legitimately blend land/coastline color into pixels a fraction of a pixel outside the
        // mathematical radius; this test cares about the corners well beyond that, not the seam.
        let outside_margin_px = 4.0;

        let mut outside_non_background = 0usize;
        let mut inside_non_background = 0usize;
        let mut inside_total = 0usize;
        // Pixel coordinates here are at most a few thousand — far inside f64's exact-integer
        // range, so every `usize as f64` narrowing below loses nothing in practice.
        #[allow(
            clippy::cast_precision_loss,
            reason = "pixel coordinates are far inside f64's exact-integer range"
        )]
        for row in 0..height {
            for col in 0..width {
                let dx = col as f64 - width as f64 / 2.0;
                let dy = row as f64 - height as f64 / 2.0;
                let dist = dx.hypot(dy);
                let pixel = pixels[row * width + col];

                if dist > radius_px + outside_margin_px {
                    if pixel != background {
                        outside_non_background += 1;
                    }
                } else if dist < radius_px - outside_margin_px {
                    inside_total += 1;
                    if pixel != background {
                        inside_non_background += 1;
                    }
                }
            }
        }

        assert_eq!(
            outside_non_background, 0,
            "at blend: 1.0, {outside_non_background} pixel(s) outside the globe's disk (radius \
             {radius_px}px + {outside_margin_px}px margin) were not background — the flat \
             Mercator map is leaking through instead of fading out"
        );
        // Generous floor: the near hemisphere from (0, 0) is mostly ocean, but real bundled
        // coastline/land data should still paint a clearly non-trivial share of the disk's
        // interior — 1% is far below any plausible land coverage and well above "renders nothing".
        #[allow(
            clippy::cast_precision_loss,
            reason = "pixel counts here are far inside f64's exact-integer range"
        )]
        let inside_fraction = inside_non_background as f64 / inside_total as f64;
        assert!(
            inside_fraction > 0.01,
            "at blend: 1.0, only {inside_non_background}/{inside_total} \
             ({inside_fraction:.4}) pixels inside the globe's disk were non-background — looks \
             like the globe pass itself is drawing nothing"
        );
    }
}
