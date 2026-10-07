fn nebula_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn nebula_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = nebula_hash(i);
    let b = nebula_hash(i + vec2<f32>(1.0, 0.0));
    let c = nebula_hash(i + vec2<f32>(0.0, 1.0));
    let d = nebula_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn nebula_fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var v = 0.0;
    var a = 0.5;
    for (var i = 0; i < 5; i = i + 1) {
        v = v + a * nebula_noise(p);
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        a = a * 0.5;
    }
    return v;
}

fn background(px: vec2<f32>, uv: vec2<f32>) -> vec4<f32> {
    let t = zhell.time * 0.015;
    let aspect = zhell.resolution.x / max(zhell.resolution.y, 1.0);
    let p = vec2<f32>(uv.x * aspect, uv.y) * 3.0;
    let n = nebula_fbm(p + vec2<f32>(t, -t * 0.7) + nebula_fbm(p * 0.7 - t));
    var c = mix(vec3<f32>(0.02, 0.02, 0.05), vec3<f32>(0.35, 0.12, 0.55), smoothstep(0.35, 0.85, n));
    c = mix(c, vec3<f32>(0.10, 0.45, 0.70), smoothstep(0.6, 0.95, n) * 0.5);
    let cell = floor(px / 3.0);
    let s = nebula_hash(cell);
    let twinkle = 0.6 + 0.4 * sin(zhell.time * 1.5 + s * 40.0);
    c = c + vec3<f32>(step(0.9985, s) * twinkle);
    return vec4<f32>(c, 1.0);
}
