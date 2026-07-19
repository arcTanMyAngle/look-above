// Label text + leader lines (M2 item 2.7b), drawn last in docs/01's draw order ("... aircraft
// glyphs → labels → UI overlay" — there is no UI overlay pass yet).
//
// Unlike every earlier pass, this one does not read the shared world view-proj matrix:
// `label.rs` computes each label's on-screen position directly in physical pixels (screen-space
// placement/collision is inherently render-side — see that module's doc comment), so the vertex
// shader here only needs the viewport size to convert pixels to clip space.
//
// Two tiny pipelines share this file and the same `@group(0)` screen-size uniform:
//   - `vs_text`/`fs_text` draws each label's characters as instanced SDF glyph quads — the same
//     `smoothstep`-around-0.5 antialiasing technique as `aircraft.wgsl`, sampling
//     `label_atlas.rs`'s stroke-font atlas instead of the aircraft silhouette atlas.
//   - `vs_leader`/`fs_leader` draws each displaced label's leader line as a plain `LineList`
//     pass-through — the same "CPU bakes the geometry, GPU just transforms it" shape as
//     `trail.wgsl`.

struct ScreenParams {
    // .xy = viewport width/height, physical pixels; .zw unused padding (uniform buffers want
    // 16-byte members).
    size: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> screen: ScreenParams;

fn pixel_to_clip(px: vec2<f32>) -> vec4<f32> {
    let ndc_x = px.x / screen.size.x * 2.0 - 1.0;
    let ndc_y = 1.0 - px.y / screen.size.y * 2.0;
    return vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
}

// ---- Text pass ---------------------------------------------------------------------------------

@group(1) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;

// Must match `label_atlas::CHAR_COUNT`.
const CHAR_COUNT: f32 = 39.0;

struct TextVertexInput {
    @location(0) local_pos: vec2<f32>,
    @location(1) local_uv: vec2<f32>,
};

struct TextInstanceInput {
    @location(2) cell_origin_px: vec2<f32>,
    @location(3) cell_size_px: vec2<f32>,
    @location(4) char_index: f32,
    @location(5) color: vec4<f32>,
};

struct TextVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_text(in: TextVertexInput, instance: TextInstanceInput) -> TextVertexOutput {
    var out: TextVertexOutput;
    let px = instance.cell_origin_px + in.local_pos * instance.cell_size_px;
    out.clip_position = pixel_to_clip(px);

    let tile_width = 1.0 / CHAR_COUNT;
    out.uv = vec2<f32>((instance.char_index + in.local_uv.x) * tile_width, in.local_uv.y);
    out.color = instance.color;
    return out;
}

@fragment
fn fs_text(in: TextVertexOutput) -> @location(0) vec4<f32> {
    let distance = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    let edge = 0.5;
    let aa = max(fwidth(distance), 0.0001);
    let alpha = smoothstep(edge - aa, edge + aa, distance);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}

// ---- Leader-line pass ---------------------------------------------------------------------------

struct LeaderVertexInput {
    @location(0) screen_px: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct LeaderVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_leader(in: LeaderVertexInput) -> LeaderVertexOutput {
    var out: LeaderVertexOutput;
    out.clip_position = pixel_to_clip(in.screen_px);
    out.color = in.color;
    return out;
}

@fragment
fn fs_leader(in: LeaderVertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
