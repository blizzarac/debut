// Resample the source through an inverse affine map: src = M * out + t.

struct Params {
    m: vec4<f32>,  // m00, m01, m10, m11
    t: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let o = in.pos.xy; // pixel centre of the output fragment
    let sx = params.m.x * o.x + params.m.y * o.y + params.t.x;
    let sy = params.m.z * o.x + params.m.w * o.y + params.t.y;
    return sample_bilinear(src, vec2<f32>(sx, sy));
}
