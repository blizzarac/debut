struct Params {
    lift: vec4<f32>,      // xyz + exposure (stops) in w
    gamma: vec4<f32>,     // xyz + contrast in w
    gain: vec4<f32>,      // xyz + saturation in w
    wb: vec4<f32>,        // temperature, tint, pad, pad
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    if (p.a <= 0.0) { return vec4<f32>(0.0); }
    var c = straight(p) * exp2(params.lift.w);
    c = c * vec3<f32>(1.0 + 0.3 * params.wb.x, 1.0 + 0.3 * params.wb.y, 1.0 - 0.3 * params.wb.x);
    c = c * params.gain.xyz + params.lift.xyz;
    c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0) / max(params.gamma.xyz, vec3<f32>(0.0001)));
    c = (c - vec3<f32>(0.18)) * params.gamma.w + vec3<f32>(0.18);
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = vec3<f32>(luma) + (c - vec3<f32>(luma)) * params.gain.w;
    return vec4<f32>(c * p.a, p.a);
}
