struct Params {
    domain_min: vec4<f32>, // xyz + size in w
    domain_max: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var lut: texture_3d<f32>;

fn lut_at(r: i32, g: i32, b: i32) -> vec3<f32> {
    return textureLoad(lut, vec3<i32>(r, g, b), 0).rgb;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    if (p.a <= 0.0) { return vec4<f32>(0.0); }
    let n = i32(params.domain_min.w);
    let s = f32(n - 1);
    let u = clamp((straight(p) - params.domain_min.xyz) / (params.domain_max.xyz - params.domain_min.xyz), vec3<f32>(0.0), vec3<f32>(1.0)) * s;
    let i0 = min(vec3<i32>(floor(u)), vec3<i32>(n - 2));
    let f = u - vec3<f32>(i0);
    let c00 = mix(lut_at(i0.x, i0.y, i0.z), lut_at(i0.x + 1, i0.y, i0.z), f.x);
    let c10 = mix(lut_at(i0.x, i0.y + 1, i0.z), lut_at(i0.x + 1, i0.y + 1, i0.z), f.x);
    let c01 = mix(lut_at(i0.x, i0.y, i0.z + 1), lut_at(i0.x + 1, i0.y, i0.z + 1), f.x);
    let c11 = mix(lut_at(i0.x, i0.y + 1, i0.z + 1), lut_at(i0.x + 1, i0.y + 1, i0.z + 1), f.x);
    let out = mix(mix(c00, c10, f.y), mix(c01, c11, f.y), f.z);
    return vec4<f32>(out * p.a, p.a);
}
