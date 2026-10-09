struct Params {
    points: array<vec4<f32>, 32>,   // two xy points per vec4 (POLY_MAX_POINTS / 2)
    count: u32,
    feather: f32,
    invert: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

fn point(i: u32) -> vec2<f32> {
    let v = params.points[i / 2u];
    if ((i & 1u) == 0u) { return v.xy; }
    return v.zw;
}

// Mirrors debut_render::nodes::PolyMask::coverage.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    let x = in.pos.x;
    let y = in.pos.y;
    let n = params.count;
    if (n < 3u) { return vec4<f32>(0.0); }
    var inside = false;
    var best = 1e30;
    for (var i = 0u; i < n; i = i + 1u) {
        let a = point(i);
        let b = point((i + 1u) % n);
        if ((a.y > y) != (b.y > y)) {
            let t = (y - a.y) / (b.y - a.y);
            if (x < a.x + t * (b.x - a.x)) { inside = !inside; }
        }
        let e = b - a;
        let len2 = max(dot(e, e), 1e-12);
        let t = clamp(((x - a.x) * e.x + (y - a.y) * e.y) / len2, 0.0, 1.0);
        let q = a + t * e - vec2<f32>(x, y);
        best = min(best, length(q));
    }
    var d = best;
    if (inside) { d = -best; }
    var m: f32;
    if (params.feather <= 0.0) {
        m = select(0.0, 1.0, d <= 0.0);
    } else {
        m = 1.0 - smoothstep(-0.5 * params.feather, 0.5 * params.feather, d);
    }
    if (params.invert != 0u) { m = 1.0 - m; }
    return p * m;
}
