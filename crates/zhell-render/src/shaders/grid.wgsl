fn background(px: vec2<f32>, uv: vec2<f32>) -> vec4<f32> {
    let horizon = 0.42;
    let sky = vec3<f32>(0.05, 0.03, 0.10) + vec3<f32>(0.25, 0.08, 0.35) * pow(max(1.0 - abs(uv.y - horizon) * 3.0, 0.0), 3.0);
    if uv.y < horizon {
        return vec4<f32>(sky, 1.0);
    }
    let depth = 0.35 / (uv.y - horizon + 0.001);
    let x = (uv.x - 0.5) * depth * (zhell.resolution.x / max(zhell.resolution.y, 1.0));
    let z = depth + zhell.time * 0.6;
    let gx = abs(fract(x) - 0.5);
    let gz = abs(fract(z) - 0.5);
    let w = 0.02 * depth;
    let line = max(1.0 - smoothstep(0.0, w, 0.5 - gx), 1.0 - smoothstep(0.0, w, 0.5 - gz));
    let fade = clamp(1.0 - depth / 18.0, 0.0, 1.0);
    let c = mix(sky * 0.6, vec3<f32>(0.75, 0.30, 0.95), line * fade);
    return vec4<f32>(c, 1.0);
}
