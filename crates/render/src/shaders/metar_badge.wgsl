// METAR flight-category badge pass (M3 item 3.3) — a small flat-shaded, screen-constant-radius
// ring per large airport with a cached observation, instanced off the same shared unit-circle
// mesh `airport.wgsl` uses (`airport::marker_mesh`). Drawn before the airport-marker pass, at a
// larger radius, so the marker's own dot paints over this ring's center — see `metar_badge.rs`'s
// module doc comment for why that reads as "a ring around the marker" without any position
// offset.
//
// Unlike `airport.wgsl`, color is per-instance (`@location(2)`), not a flat `@group(1)`
// uniform: a badge's color depends on that airport's own flight category. `params.scale` is
// still the one per-frame uniform, playing the same screen-constant-size role
// `airport.wgsl`'s `MarkerParams.scale` does.

struct ViewProj {
    matrix: mat4x4<f32>,
};

struct BadgeParams {
    scale: f32,
};

@group(0) @binding(0)
var<uniform> view_proj: ViewProj;

@group(1) @binding(0)
var<uniform> params: BadgeParams;

struct VertexInput {
    @location(0) local_pos: vec2<f32>,
};

struct InstanceInput {
    @location(1) world_xy: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput, instance: InstanceInput) -> VertexOutput {
    var out: VertexOutput;
    let world = instance.world_xy + in.local_pos * params.scale;
    out.clip_position = view_proj.matrix * vec4<f32>(world, 0.0, 1.0);
    out.color = instance.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
