// Globe base map (land fill + coastline stroke) — M4 item 4.3.
//
// Geometry is pre-tessellated on the CPU (`basemap.rs`'s tessellate_globe) as raw per-vertex
// lon/lat in radians; the orthographic sphere projection itself happens here, in the vertex
// shader, mirroring `core::geo::orthographic_forward`'s math exactly so this pass agrees with
// every CPU-side use of that function (e.g. the density-dot pass's own aircraft positions).
//
// The horizon (far hemisphere) test is *not* done per-vertex: `cos_c` (the cosine of the angular
// distance from the sub-observer point — negative on the far hemisphere) is passed through as an
// interpolated varying and discarded per-fragment instead. A per-vertex all-or-nothing test would
// either draw or drop a whole triangle based on its vertices alone, popping/jagging right at the
// globe's edge; interpolating `cos_c` across the triangle and testing it per-fragment clips
// exactly along the true horizon curve instead — this is the fix for docs/13's "no horizon
// clipping artifacts" line.
//
// Color is per-pass, flat-shaded, the same `@group(1)` shape `basemap.wgsl` already uses.

struct GlobeViewProj {
    // .x = sub-observer center latitude (radians), .y = center longitude (radians),
    // .z = scale_x, .w = scale_y (unit-disk-to-clip-space scale — see renderer.rs's
    // globe_view_proj_bytes for the derivation).
    center_and_scale: vec4<f32>,
    // .x = this frame's globe<->Mercator blend (0 = fully Mercator, 1 = fully globe); .yzw
    // unused padding (uniform buffers want 16-byte members).
    blend: vec4<f32>,
};

struct LayerColor {
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> globe: GlobeViewProj;

@group(1) @binding(0)
var<uniform> layer_color: LayerColor;

struct VertexInput {
    @location(0) lonlat_rad: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) cos_c: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    let lambda = in.lonlat_rad.x;
    let phi = in.lonlat_rad.y;
    let phi1 = globe.center_and_scale.x;
    let lambda1 = globe.center_and_scale.y;
    let delta_lambda = lambda - lambda1;

    let cos_c = sin(phi1) * sin(phi) + cos(phi1) * cos(phi) * cos(delta_lambda);
    let x = cos(phi) * sin(delta_lambda);
    let y = cos(phi1) * sin(phi) - sin(phi1) * cos(phi) * cos(delta_lambda);

    out.clip_position = vec4<f32>(
        x * globe.center_and_scale.z,
        y * globe.center_and_scale.w,
        0.0,
        1.0
    );
    out.cos_c = cos_c;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Per-fragment (not per-vertex) horizon test — see this file's own doc comment.
    if (in.cos_c < 0.0) {
        discard;
    }
    return vec4<f32>(layer_color.color.rgb, layer_color.color.a * globe.blend.x);
}
