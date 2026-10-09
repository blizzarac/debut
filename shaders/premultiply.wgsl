// Straight-alpha source (8-bit decoded video) -> premultiplied float image.

struct Params {
    _pad: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    return vec4<f32>(p.rgb * p.a, p.a) + params._pad * 0.0;
}
