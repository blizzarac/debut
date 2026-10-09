struct Params {
    key: vec4<f32>,       // linear rgb, pad
    knobs: vec4<f32>,     // tolerance, softness, spill, pad
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

fn ycbcr(c: vec3<f32>) -> vec3<f32> {
    let y = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    return vec3<f32>(y, (c.b - y) / 1.8556, (c.r - y) / 1.5748);
}

fn rgb(y: f32, cb: f32, cr: f32) -> vec3<f32> {
    let b = y + cb * 1.8556;
    let r = y + cr * 1.5748;
    let g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
    return vec3<f32>(r, g, b);
}

// Mirrors debut_render::nodes::ChromaKey::apply.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    if (p.a <= 0.0) { return vec4<f32>(0.0); }
    let c = straight(p);
    let k = ycbcr(params.key.rgb);
    let v = ycbcr(c);
    let d = length(v.yz - k.yz);
    let alpha = smoothstep(params.knobs.x, params.knobs.x + max(params.knobs.y, 0.0001), d);
    let klen = length(k.yz);
    var out = c;
    if (klen >= 0.0001 && params.knobs.z > 0.0) {
        let u = k.yz / klen;
        let proj = max(dot(v.yz, u), 0.0) * params.knobs.z;
        out = max(rgb(v.x, v.y - u.x * proj, v.z - u.y * proj), vec3<f32>(0.0));
    }
    let a = p.a * alpha;
    return vec4<f32>(out * a, a);
}
