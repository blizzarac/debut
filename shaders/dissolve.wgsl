// Cross-dissolve: a * (1 - p) + b * p.

struct Params {
    progress: f32,
    _p0: f32,
    _p1: f32,
    _p2: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var a: texture_2d<f32>;
@group(0) @binding(2) var b: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    return textureLoad(a, p, 0) * (1.0 - params.progress) + textureLoad(b, p, 0) * params.progress;
}
