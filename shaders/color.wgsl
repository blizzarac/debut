// Transfer functions shared by the color passes. Mirrors debut_render::color.

const T_LINEAR: u32 = 0u;
const T_SRGB: u32 = 1u;
const T_BT1886: u32 = 2u;
const T_PQ: u32 = 3u;
const T_HLG: u32 = 4u;
const T_SLOG3: u32 = 5u;
const T_LOGC3: u32 = 6u;
const T_VLOG: u32 = 7u;

fn decode1(t: u32, v: f32) -> f32 {
    switch t {
        case 1u: {
            if (v <= 0.04045) { return v / 12.92; }
            return pow((v + 0.055) / 1.055, 2.4);
        }
        case 2u: { return pow(max(v, 0.0), 2.4); }
        case 3u: {
            let p = pow(max(v, 0.0), 1.0 / 78.84375);
            let num = max(p - 0.8359375, 0.0);
            let den = 18.8515625 - 18.6875 * p;
            return pow(num / den, 1.0 / 0.1593017578125) * 100.0;
        }
        case 4u: {
            var e: f32;
            if (v <= 0.5) { e = v * v / 3.0; } else { e = (exp((v - 0.55991073) / 0.17883277) + 0.28466892) / 12.0; }
            return e * 10.0;
        }
        case 5u: {
            if (v >= 171.2103 / 1023.0) { return pow(10.0, (v * 1023.0 - 420.0) / 261.5) * 0.19 - 0.01; }
            return (v * 1023.0 - 95.0) * 0.01125 / (171.2103 - 95.0);
        }
        case 6u: {
            if (v > 5.367655 * 0.010591 + 0.092809) { return (pow(10.0, (v - 0.385537) / 0.247190) - 0.052272) / 5.555556; }
            return (v - 0.092809) / 5.367655;
        }
        case 7u: {
            if (v >= 0.181) { return pow(10.0, (v - 0.598206) / 0.241514) - 0.00873; }
            return (v - 0.125) / 5.6;
        }
        default: { return v; }
    }
}

fn encode1(t: u32, l: f32) -> f32 {
    switch t {
        case 1u: {
            if (l <= 0.0031308) { return l * 12.92; }
            return 1.055 * pow(max(l, 0.0), 1.0 / 2.4) - 0.055;
        }
        case 2u: { return pow(max(l, 0.0), 1.0 / 2.4); }
        case 3u: {
            let y = pow(max(l / 100.0, 0.0), 0.1593017578125);
            return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), 78.84375);
        }
        case 4u: {
            let e = max(l / 10.0, 0.0);
            if (e <= 1.0 / 12.0) { return sqrt(3.0 * e); }
            return 0.17883277 * log(12.0 * e - 0.28466892) + 0.55991073;
        }
        case 5u: {
            if (l >= 0.01125) { return (420.0 + log((l + 0.01) / 0.19) / log(10.0) * 261.5) / 1023.0; }
            return (l * (171.2103 - 95.0) / 0.01125 + 95.0) / 1023.0;
        }
        case 6u: {
            if (l > 0.010591) { return 0.247190 * log(5.555556 * l + 0.052272) / log(10.0) + 0.385537; }
            return 5.367655 * l + 0.092809;
        }
        case 7u: {
            if (l >= 0.01) { return 0.241514 * log(l + 0.00873) / log(10.0) + 0.598206; }
            return 5.6 * l + 0.125;
        }
        default: { return l; }
    }
}

fn decode3(t: u32, c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(decode1(t, c.x), decode1(t, c.y), decode1(t, c.z));
}

fn encode3(t: u32, c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(encode1(t, c.x), encode1(t, c.y), encode1(t, c.z));
}

// Un-premultiply, apply, re-premultiply; transparent stays transparent.
fn straight(p: vec4<f32>) -> vec3<f32> {
    if (p.a <= 0.0) { return vec3<f32>(0.0); }
    return p.rgb / p.a;
}
