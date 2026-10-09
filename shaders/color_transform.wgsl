struct Params {
    m0: vec4<f32>, // matrix row 0 (xyz) + decode id in w
    m1: vec4<f32>, // row 1 + encode id in w
    m2: vec4<f32>, // row 2 + pad
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    if (p.a <= 0.0) { return vec4<f32>(0.0); }
    let lin = decode3(u32(params.m0.w), straight(p));
    let m = mat3x3<f32>(
        vec3<f32>(params.m0.x, params.m1.x, params.m2.x),
        vec3<f32>(params.m0.y, params.m1.y, params.m2.y),
        vec3<f32>(params.m0.z, params.m1.z, params.m2.z),
    );
    let out = encode3(u32(params.m1.w), m * lin);
    return vec4<f32>(out * p.a, p.a);
}
