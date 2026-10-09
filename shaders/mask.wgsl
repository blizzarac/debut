struct Params {
    center: vec2<f32>,
    half: vec2<f32>,
    feather: f32,
    shape: u32,    // 0 rectangle, 1 ellipse
    invert: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;

// Mirrors debut_render::nodes::Mask::coverage.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    let dx = abs(in.pos.x - params.center.x);
    let dy = abs(in.pos.y - params.center.y);
    let hw = max(params.half.x, 0.001);
    let hh = max(params.half.y, 0.001);
    var d: f32;
    if (params.shape == 1u) {
        let r = sqrt((dx / hw) * (dx / hw) + (dy / hh) * (dy / hh));
        d = (r - 1.0) * min(hw, hh);
    } else {
        d = max(dx - hw, dy - hh);
    }
    var m: f32;
    if (params.feather <= 0.0) {
        m = select(0.0, 1.0, d <= 0.0);
    } else {
        m = 1.0 - smoothstep(-0.5 * params.feather, 0.5 * params.feather, d);
    }
    if (params.invert != 0u) { m = 1.0 - m; }
    return p * m;
}
