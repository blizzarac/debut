// Shared by every debut-render pass. Images are rgba32float, linear light,
// premultiplied alpha. One full-screen triangle; `uv` is in output pixel space.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // Covers the viewport with one triangle: (-1,-1) (3,-1) (-1,3).
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var out: VsOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

// Bilinear sample at a continuous pixel-space position (pixel centres at +0.5),
// transparent outside. Mirrors CpuImage::sample so both backends agree exactly.
fn load_or_zero(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(t));
    if (x < 0 || y < 0 || x >= dims.x || y >= dims.y) {
        return vec4<f32>(0.0);
    }
    return textureLoad(t, vec2<i32>(x, y), 0);
}

fn sample_bilinear(t: texture_2d<f32>, p: vec2<f32>) -> vec4<f32> {
    let f = p - vec2<f32>(0.5);
    let fl = floor(f);
    let w = f - fl;
    let x0 = i32(fl.x);
    let y0 = i32(fl.y);
    let p00 = load_or_zero(t, x0, y0);
    let p10 = load_or_zero(t, x0 + 1, y0);
    let p01 = load_or_zero(t, x0, y0 + 1);
    let p11 = load_or_zero(t, x0 + 1, y0 + 1);
    let top = p00 * (1.0 - w.x) + p10 * w.x;
    let bot = p01 * (1.0 - w.x) + p11 * w.x;
    return top * (1.0 - w.y) + bot * w.y;
}
