struct Globals {
    viewport: vec2<f32>,
    mask_size: vec2<f32>,
    color_size: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var mask_tex: texture_2d<f32>;
@group(0) @binding(2) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var nearest: sampler;
@group(0) @binding(4) var linear_sampler: sampler;

fn corner(i: u32) -> vec2<f32> {
    return vec2<f32>(f32(i & 1u), f32(i >> 1u));
}

fn clip(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(p.x / g.viewport.x * 2.0 - 1.0, 1.0 - p.y / g.viewport.y * 2.0, 0.0, 1.0);
}

struct RectIn {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: u32,
    @location(4) radius: f32,
    @location(5) thickness: f32,
}

struct RectOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) kind: u32,
    @location(4) radius: f32,
    @location(5) thickness: f32,
}

@vertex
fn vs_rect(@builtin(vertex_index) vi: u32, r: RectIn) -> RectOut {
    var out: RectOut;
    let local = corner(vi) * r.size;
    out.position = clip(r.pos + local);
    out.local = local;
    out.size = r.size;
    out.color = r.color;
    out.kind = r.kind;
    out.radius = r.radius;
    out.thickness = r.thickness;
    return out;
}

@fragment
fn fs_rect(in: RectOut) -> @location(0) vec4<f32> {
    var a = 1.0;
    let t = max(in.thickness, 1.0);
    switch in.kind {
        case 1u: {
            // undercurl: one wave per cell, centered in the rect
            let amp = max((in.size.y - t) * 0.5, 0.5);
            let period = max(in.size.y * 2.6, 6.0);
            let y = in.size.y * 0.5 - amp * sin(in.local.x / period * 6.2831853);
            let slope = amp * 6.2831853 / period * cos(in.local.x / period * 6.2831853);
            let d = abs(in.local.y - y) / sqrt(1.0 + slope * slope);
            a = clamp(t * 0.5 + 0.5 - d, 0.0, 1.0);
        }
        case 2u: {
            a = select(0.0, 1.0, fract(in.local.x / (t * 2.0)) < 0.5);
        }
        case 3u: {
            a = select(0.0, 1.0, fract(in.local.x / (t * 6.0)) < 0.6);
        }
        case 4u: {
            // hollow box
            let edge = min(min(in.local.x, in.size.x - in.local.x), min(in.local.y, in.size.y - in.local.y));
            a = select(0.0, 1.0, edge < t);
        }
        default: {
            if in.radius > 0.0 {
                let half = in.size * 0.5;
                let q = abs(in.local - half) - half + vec2<f32>(in.radius);
                let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
                a = clamp(0.5 - d, 0.0, 1.0);
            }
        }
    }
    let alpha = in.color.a * a;
    return vec4<f32>(in.color.rgb * alpha, alpha);
}

struct SpriteIn {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv: vec4<f32>,
    @location(3) color: vec4<f32>,
}

struct SpriteOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_mask(@builtin(vertex_index) vi: u32, s: SpriteIn) -> SpriteOut {
    var out: SpriteOut;
    let c = corner(vi);
    out.position = clip(s.pos + c * s.size);
    out.uv = (s.uv.xy + c * s.uv.zw) / g.mask_size;
    out.color = s.color;
    return out;
}

@vertex
fn vs_color(@builtin(vertex_index) vi: u32, s: SpriteIn) -> SpriteOut {
    var out: SpriteOut;
    let c = corner(vi);
    out.position = clip(s.pos + c * s.size);
    out.uv = (s.uv.xy + c * s.uv.zw) / g.color_size;
    out.color = s.color;
    return out;
}

@fragment
fn fs_mask(in: SpriteOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(mask_tex, nearest, in.uv).r;
    let alpha = in.color.a * coverage;
    return vec4<f32>(in.color.rgb * alpha, alpha);
}

@fragment
fn fs_color(in: SpriteOut) -> @location(0) vec4<f32> {
    let c = textureSample(color_tex, linear_sampler, in.uv);
    let alpha = c.a * in.color.a;
    return vec4<f32>(c.rgb * alpha, alpha);
}

@group(1) @binding(0) var image_tex: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

@vertex
fn vs_image(@builtin(vertex_index) vi: u32, s: SpriteIn) -> SpriteOut {
    var out: SpriteOut;
    let c = corner(vi);
    out.position = clip(s.pos + c * s.size);
    out.uv = s.uv.xy + c * s.uv.zw;
    out.color = s.color;
    return out;
}

@fragment
fn fs_image(in: SpriteOut) -> @location(0) vec4<f32> {
    let c = textureSample(image_tex, image_sampler, in.uv);
    let alpha = c.a * in.color.a;
    return vec4<f32>(c.rgb * alpha, alpha);
}
