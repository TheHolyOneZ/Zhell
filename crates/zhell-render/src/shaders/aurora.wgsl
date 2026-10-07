fn aurora_noise(p: vec2<f32>) -> f32 {
    return sin(p.x) * cos(p.y * 0.8) + sin(p.x * 0.6 + p.y * 1.3) * 0.5;
}

fn background(px: vec2<f32>, uv: vec2<f32>) -> vec4<f32> {
    let t = zhell.time * 0.06;
    let aspect = zhell.resolution.x / max(zhell.resolution.y, 1.0);
    let p = vec2<f32>(uv.x * aspect, uv.y) * 2.2;
    var c = vec3<f32>(0.0);
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        let band = p.y - 0.9 - 0.35 * aurora_noise(vec2<f32>(p.x * 0.9 + t * (1.0 + fi * 0.4) + fi * 2.1, t + fi));
        let glow = exp(-band * band * (5.0 + fi * 3.0));
        let tone = mix(vec3<f32>(0.30, 0.20, 0.95), vec3<f32>(0.10, 0.85, 0.75), 0.5 + 0.5 * sin(fi * 1.7 + t * 2.0 + p.x * 0.4));
        c = c + tone * glow * (0.55 - fi * 0.12);
    }
    let base = vec3<f32>(0.03, 0.03, 0.07) * (1.2 - uv.y * 0.6);
    return vec4<f32>(base + c, 1.0);
}
