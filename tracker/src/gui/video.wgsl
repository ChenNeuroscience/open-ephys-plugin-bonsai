// Draws a grayscale camera frame into the callback's viewport, with the crop mask and threshold highlight
// applied per pixel. See video_gpu.rs.

struct Uniforms {
    // Pixels whose 8-bit value is strictly greater than this are highlighted.
    threshold: f32,
    highlight: u32,
    // 1: show everything outside the crop as-is; 0: mask it to black.
    crop_visible: u32,
    // 1 if the render target re-encodes to sRGB, so we must output linear values.
    srgb_target: u32,
    // The threshold within the edge band (where the mask is 128).
    edge_threshold: f32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var frame_tex: texture_2d<f32>;
// Same size as the frame (255 inside the crop, 128 in its edge band, 0 outside), or 1x1 white when there's
// no crop.
@group(0) @binding(1) var mask_tex: texture_2d<f32>;
@group(0) @binding(2) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle that covers the whole viewport; uv is 0..1 across it.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn texel(tex: texture_2d<f32>, uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(tex));
    let p = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - 1);
    return textureLoad(tex, p, 0).r;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let v = texel(frame_tex, in.uv);
    let mask = texel(mask_tex, in.uv);
    let inside = mask > 0.25;
    let threshold = select(u.threshold, u.edge_threshold, mask < 0.75);

    var colour = vec3<f32>(v);
    if !inside {
        if u.crop_visible == 0u {
            colour = vec3<f32>(0.0);
        }
    } else if u.highlight != 0u && v * 255.0 > threshold + 0.5 {
        colour = vec3<f32>(1.0, 0.0, 1.0);
    }
    if u.srgb_target != 0u {
        colour = srgb_to_linear(colour);
    }
    return vec4<f32>(colour, 1.0);
}
