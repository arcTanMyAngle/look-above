// Aircraft glyphs (M2 item 2.5): instanced quads, one per aircraft, drawn after the base map and
// before labels (docs/01's draw order — trails are 2.6, labels 2.7, neither exists yet).
//
// Per-instance data (world position, heading, category, altitude-bucket tint, stale-fade alpha)
// comes from `core::sim`'s `RenderFeed`, packed into `InstanceRaw` on the CPU side
// (`aircraft.rs::pack_instance`) each frame. The quad's four corners and their local UVs are the
// one static per-vertex buffer (`aircraft.rs::quad_vertices`), shared by every instance.
//
// Rotation mirrors `aircraft.rs::rotate_clockwise_from_north`'s exact formula: local glyph space
// is "north-up" (nose at local `+y`), and Mercator's own `+y` / clip space `+y` both point
// north/up too (see `renderer.rs::camera_view_proj`'s doc comment — no axis flip sits between
// world and screen here), so a heading clockwise from geographic north rotates the local offset
// the same way clockwise on screen.
//
// SDF sampling: `atlas_texture` encodes `0.5` at each silhouette's edge (`glyph_atlas.rs`); the
// fragment shader `smoothstep`s a `fwidth`-derived band around it for antialiased edges — the
// "SDF-derived AA" docs/01's quality bar asks for, without a second MSAA-style resolve of its own
// (this pass still renders into the shared 4x MSAA target like every other pass).
//
// Selection outline (M2 item 2.8b): `instance.scale_mul` multiplies the shared per-frame glyph
// scale for this one instance. `aircraft.rs::pack_instances` packs a selected aircraft's outline
// as an extra, earlier instance in the same buffer — same silhouette/position/heading, solid
// white, `scale_mul > 1` — so it draws first (no depth test; alpha-blended painter's-algorithm
// order) and the normal-size glyph drawn after it on top leaves a white halo peeking out from
// behind.

struct ViewProj {
    matrix: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> view_proj: ViewProj;

// .x = the glyph's on-screen size, in the same pre-normalized (Mercator-metres / extent) plane
// the view-proj matrix operates on — see `aircraft.rs::glyph_scale_normalized`. Recomputed and
// rewritten every frame (`Renderer::render`) so the glyph stays a constant screen-space pixel
// size across zoom levels; `.yzw` are unused padding (uniform buffers want 16-byte members).
@group(1) @binding(0)
var<uniform> glyph_params: vec4<f32>;

@group(2) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(2) @binding(1)
var atlas_sampler: sampler;

// Must match `glyph_atlas::CATEGORY_COUNT`.
const CATEGORY_COUNT: f32 = 6.0;

struct VertexInput {
    @location(0) local_pos: vec2<f32>,
    @location(1) local_uv: vec2<f32>,
};

struct InstanceInput {
    @location(2) world_xy: vec2<f32>,
    @location(3) heading_rad: f32,
    @location(4) category_index: f32,
    @location(5) tint: vec4<f32>,
    @location(6) scale_mul: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput, instance: InstanceInput) -> VertexOutput {
    var out: VertexOutput;

    let c = cos(instance.heading_rad);
    let s = sin(instance.heading_rad);
    // Clockwise-from-north rotation of the local corner offset — see this file's module doc.
    let rotated = vec2<f32>(
        in.local_pos.x * c + in.local_pos.y * s,
        -in.local_pos.x * s + in.local_pos.y * c,
    );

    let world = instance.world_xy + rotated * glyph_params.x * instance.scale_mul;
    out.clip_position = view_proj.matrix * vec4<f32>(world, 0.0, 1.0);

    let tile_width = 1.0 / CATEGORY_COUNT;
    out.uv = vec2<f32>((instance.category_index + in.local_uv.x) * tile_width, in.local_uv.y);
    out.tint = instance.tint;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    let edge = 0.5;
    let aa = max(fwidth(distance), 0.0001);
    let alpha = smoothstep(edge - aa, edge + aa, distance);
    return vec4<f32>(in.tint.rgb, in.tint.a * alpha);
}
