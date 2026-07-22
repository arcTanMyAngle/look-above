// L0 density-dot pass (M4 item 4.3): a small screen-constant additive dot per visible aircraft.
// CPU-side (`density.rs::pack_density_dots`) already culled to the near hemisphere and projected
// each aircraft onto the globe's unit disk; this shader only positions/sizes the shared quad and
// lets the pipeline's additive `BlendState` (renderer.rs) sum overlapping instances in the
// framebuffer — "brightness proportional to local count" (docs/01) falls out of that
// accumulation, not any per-fragment density calculation here.
//
// Shares `@group(0)` with `globe_basemap.wgsl` (both are globe-space passes reading the same
// sub-observer center/scale/blend uniform) — see renderer.rs's own doc comment on why one shared
// bind group serves both.

struct GlobeViewProj {
    center_and_scale: vec4<f32>, // .z = scale_x, .w = scale_y; .xy (center) unused here.
    blend: vec4<f32>,            // .x = this frame's globe<->Mercator blend.
};

struct DotParams {
    // Flat per-dot color, alpha already tuned low for additive accumulation — see
    // `color::density_dot_color`'s own doc comment.
    color: vec4<f32>,
    // .x = this frame's screen-constant dot scale (disk-plane units) — see
    // `density::density_dot_scale_normalized`. .yzw unused padding.
    scale: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> globe: GlobeViewProj;

@group(1) @binding(0)
var<uniform> params: DotParams;

struct VertexInput {
    @location(0) local_pos: vec2<f32>,
};

struct InstanceInput {
    @location(1) disk_xy: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput, instance: InstanceInput) -> VertexOutput {
    var out: VertexOutput;
    let disk_pos = instance.disk_xy + in.local_pos * params.scale.x;
    out.clip_position = vec4<f32>(
        disk_pos.x * globe.center_and_scale.z,
        disk_pos.y * globe.center_and_scale.w,
        0.0,
        1.0
    );
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(params.color.rgb, params.color.a * globe.blend.x);
}
