struct Globals {
    screen: vec2<f32>,
    atlas: vec2<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_smp: sampler;
@group(0) @binding(3) var image_tex: texture_2d<f32>;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_pos: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) kind: u32,
    @location(5) uv_size: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
    @location(3) local: vec2<f32>,
    @location(4) @interpolate(flat) size: vec2<f32>,
    @location(5) @interpolate(flat) radius: f32,
    @location(6) @interpolate(flat) thickness: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let px = inst.pos + corner * inst.size;
    var out: VsOut;
    out.clip = vec4<f32>(px.x / globals.screen.x * 2.0 - 1.0, 1.0 - px.y / globals.screen.y * 2.0, 0.0, 1.0);
    let atlas = select(globals.atlas, vec2<f32>(globals.params.y, globals.params.y), inst.kind == 5u);
    out.uv = (inst.uv_pos + corner * inst.uv_size) / atlas;
    out.color = inst.color;
    out.kind = inst.kind;
    out.local = corner * inst.size;
    out.size = inst.size;
    out.radius = inst.uv_pos.x;
    out.thickness = inst.uv_pos.y;
    return out;
}

fn rounded_box(p: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let half = size * 0.5;
    let r = min(radius, min(half.x, half.y));
    let q = abs(p - half) - (half - vec2<f32>(r, r));
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn shade(in: VsOut) -> vec4<f32> {
    if in.kind == 0u {
        return in.color;
    }
    if in.kind == 3u {
        let a = clamp(0.5 - rounded_box(in.local, in.size, in.radius), 0.0, 1.0);
        return vec4<f32>(in.color.rgb, in.color.a * a);
    }
    if in.kind == 4u {
        let d = rounded_box(in.local, in.size, in.radius);
        let t = in.thickness;
        let a = clamp(0.5 - (abs(d + t * 0.5) - t * 0.5), 0.0, 1.0);
        return vec4<f32>(in.color.rgb, in.color.a * a);
    }
    if in.kind == 5u {
        let px = textureSampleLevel(image_tex, atlas_smp, in.uv, 0.0);
        return vec4<f32>(px.rgb, px.a * in.color.a);
    }
    let s = textureSampleLevel(atlas_tex, atlas_smp, in.uv, 0.0);
    if in.kind == 1u {
        return vec4<f32>(in.color.rgb, in.color.a * s.a);
    }
    return vec4<f32>(s.rgb, s.a * in.color.a);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var c = shade(in);
    let wr = globals.params.x;
    if wr > 0.0 {
        let mask = clamp(0.5 - rounded_box(in.clip.xy, globals.screen, wr), 0.0, 1.0);
        c.a = c.a * mask;
    }
    return c;
}
