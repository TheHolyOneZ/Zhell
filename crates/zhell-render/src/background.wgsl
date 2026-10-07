struct Zhell {
    resolution: vec2<f32>,
    time: f32,
    dim: f32,
    tint: vec4<f32>,
    image: vec4<f32>,
};

@group(0) @binding(0) var<uniform> zhell: Zhell;
@group(0) @binding(1) var zhell_image: texture_2d<f32>;
@group(0) @binding(2) var zhell_sampler: sampler;

struct FullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> FullOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: FullOut;
    out.clip = vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    out.uv = p;
    return out;
}

fn zhell_window_mask(px: vec2<f32>) -> f32 {
    let r = zhell.image.w;
    if r <= 0.0 {
        return 1.0;
    }
    let half = zhell.resolution * 0.5;
    let q = abs(px - half) - (half - vec2<f32>(r, r));
    let d = length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
    return clamp(0.5 - d, 0.0, 1.0);
}

fn zhell_finish(c: vec4<f32>, px: vec2<f32>) -> vec4<f32> {
    let under = mix(zhell.tint.rgb, c.rgb, c.a);
    let rgb = mix(under, zhell.tint.rgb, zhell.dim);
    return vec4<f32>(rgb, zhell.tint.a * zhell_window_mask(px));
}

fn zhell_image_uv(px: vec2<f32>) -> vec3<f32> {
    let win = zhell.resolution;
    let img = max(zhell.image.xy, vec2<f32>(1.0, 1.0));
    let mode = u32(zhell.image.z);
    if mode == 2u {
        return vec3<f32>(px / win, 1.0);
    }
    if mode == 3u {
        return vec3<f32>(fract(px / img), 1.0);
    }
    var scale = 1.0;
    if mode == 0u {
        scale = max(win.x / img.x, win.y / img.y);
    } else if mode == 1u {
        scale = min(win.x / img.x, win.y / img.y);
    }
    let size = img * scale;
    let uv = (px - (win - size) * 0.5) / size;
    let inside = select(0.0, 1.0, all(uv >= vec2<f32>(0.0, 0.0)) && all(uv <= vec2<f32>(1.0, 1.0)));
    return vec3<f32>(uv, inside);
}

@fragment
fn fs_image(in: FullOut) -> @location(0) vec4<f32> {
    let px = in.clip.xy;
    let t = zhell_image_uv(px);
    var c = textureSampleLevel(zhell_image, zhell_sampler, t.xy, 0.0);
    c.a = c.a * t.z;
    return zhell_finish(c, px);
}

@fragment
fn fs_crt(in: FullOut) -> @location(0) vec4<f32> {
    let px = in.clip.xy;
    let line = 0.5 + 0.5 * cos(px.y * 3.14159265 * 2.0 / 3.0);
    let d = in.uv - vec2<f32>(0.5, 0.5);
    let vignette = smoothstep(0.35, 0.95, length(d * vec2<f32>(1.0, 0.85)) * 1.25);
    let a = clamp(line * 0.10 + vignette * 0.45, 0.0, 0.6) * zhell_window_mask(px);
    return vec4<f32>(0.0, 0.0, 0.0, a);
}
