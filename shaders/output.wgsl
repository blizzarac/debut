// Output transform: premultiplied working-space float -> straight, display-encoded
// 8-bit (the viewer and the encoder both want this).

struct Params {
    encode: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    if (p.a <= 0.0) { return vec4<f32>(0.0); }
    let c = clamp(encode3(params.encode, straight(p)), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(c, clamp(p.a, 0.0, 1.0));
}
