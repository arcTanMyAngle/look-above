// Trail ribbons (M2 item 2.6b): the widened, tapered, altitude-colored path behind each aircraft,
// drawn after the base map and before the aircraft glyphs (docs/01's draw order: map base → map
// lines → trails → aircraft → labels → UI — trails go under the glyphs so an aircraft is never
// occluded by its own trail).
//
// All the geometry work happens on the CPU (`trail.rs::tessellate_trails`): each vertex arrives
// already offset perpendicular to the local direction of travel by the taper half-width, and
// already colored (altitude-ramp tint with the front-to-tail taper alpha in `.a`). This shader
// therefore does the minimum — apply the shared view-proj matrix, pass the color through — and
// carries no ribbon-widening logic of its own. The pass is alpha-blended (the taper alpha needs
// it), like the aircraft pass and unlike the opaque base-map passes.

struct ViewProj {
    matrix: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> view_proj: ViewProj;

struct VertexInput {
    @location(0) world_xy: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = view_proj.matrix * vec4<f32>(in.world_xy, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
