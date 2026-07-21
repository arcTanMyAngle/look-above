// Airport marker pass (M3 item 3.2) — a small flat-shaded, screen-constant-radius circle per
// queried airport, instanced off one shared unit-circle mesh built once in `airport.rs`
// (`marker_mesh`). No atlas, no rotation: an airport marker has no heading, unlike an aircraft
// glyph (`aircraft.wgsl`). `params.scale` is the one per-frame value that keeps the marker's
// on-screen size constant across zoom, the same role `aircraft.wgsl`'s glyph-scale uniform plays
// for aircraft glyphs — see `airport::airport_marker_scale_normalized`'s doc comment.
//
// Color is a flat per-layer uniform (`@group(1)`), like the base-map passes: every marker
// instance is the same color, so there is nothing per-instance to carry beyond position.

struct ViewProj {
    matrix: mat4x4<f32>,
};

struct MarkerParams {
    color: vec4<f32>,
    scale: f32,
};

@group(0) @binding(0)
var<uniform> view_proj: ViewProj;

@group(1) @binding(0)
var<uniform> params: MarkerParams;

struct VertexInput {
    @location(0) local_pos: vec2<f32>,
};

struct InstanceInput {
    @location(1) world_xy: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput, instance: InstanceInput) -> VertexOutput {
    var out: VertexOutput;
    let world = instance.world_xy + in.local_pos * params.scale;
    out.clip_position = view_proj.matrix * vec4<f32>(world, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return params.color;
}
