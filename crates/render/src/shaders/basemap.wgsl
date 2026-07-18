// Base map (land fill + coastline stroke) — M2 item 2.2b.
//
// Geometry is pre-tessellated on the CPU (`basemap.rs`) into a static, normalized Mercator
// plane, so both passes share this one vertex shader: the only per-frame GPU work is the
// `view_proj` transform. For M2 2.2b that transform is a placeholder aspect-correcting
// fit-to-window matrix (see `Renderer::fit_to_window_matrix`); M2 2.3 replaces its *contents*
// with a real pan/zoom camera without this shader changing.
//
// Color is per-pass, not per-vertex: both layers are flat-shaded, so `layer_color` is a small
// uniform bound in `@group(1)`, one bind group per layer (`renderer.rs`), rather than baked
// into the shader source or carried on every vertex.

struct ViewProj {
    matrix: mat4x4<f32>,
};

struct LayerColor {
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> view_proj: ViewProj;

@group(1) @binding(0)
var<uniform> layer_color: LayerColor;

struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = view_proj.matrix * vec4<f32>(in.position, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return layer_color.color;
}
