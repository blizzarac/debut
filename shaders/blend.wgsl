// Composite top over bottom. Modes match cpu::blend_px.

struct Params {
    mode: u32,     // 0 normal, 1 add, 2 multiply, 3 screen
    opacity: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var bottom: texture_2d<f32>;
@group(0) @binding(2) var top: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let b = textureLoad(bottom, p, 0);
    let t = textureLoad(top, p, 0) * params.opacity;
    switch params.mode {
        case 1u: {
            return min(t + b, vec4<f32>(1.0));
        }
        case 2u: {
            let rgb = t.rgb * b.rgb + t.rgb * (1.0 - b.a) + b.rgb * (1.0 - t.a);
            return vec4<f32>(rgb, t.a + b.a * (1.0 - t.a));
        }
        case 3u: {
            let rgb = t.rgb + b.rgb - t.rgb * b.rgb;
            return vec4<f32>(rgb, t.a + b.a * (1.0 - t.a));
        }
        default: {
            return t + b * (1.0 - t.a);
        }
    }
}
