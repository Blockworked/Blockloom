// VFX particles and ribbons in 2D: `vfx_particles.wesl` flattened onto the
// screen plane, without the lit and soft paths a flat world has no use for.

import bevy_sprite_render::mesh2d::view_bindings::view;
import blockloom::vfx::{Particle, alive, pinned, life_t, lut_at, frame_rect};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
@if(SRGB_OUTPUT)
import bevy_render::color_operations::linear_to_srgb;
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;

// `ParticleLook` in `vfx_particles.wesl`, field for field.
struct ParticleLook {
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    size: vec4<f32>,
    flipbook: vec4<f32>,
    extra: vec4<f32>,
    modes: vec4<u32>,
    ribbon: vec4<u32>,
    curves: array<vec4<f32>, 32>,
    colors: array<vec4<f32>, 32>,
    ribbon_colors: array<vec4<f32>, 32>,
}

const FLAG_FLIPBOOK: u32 = 4u;
const FLAG_FRAME_BLEND: u32 = 8u;
const FLAG_PINNED_ONLY: u32 = 16u;
const FLAG_ADDITIVE: u32 = 64u;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> look: ParticleLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> particles: array<Particle>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<storage, read> trail: array<vec4<f32>>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var sheet: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var sheet_sampler: sampler;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) frames: vec3<f32>,
    @location(3) @interpolate(flat) kind: u32,
}

fn has(bit: u32) -> bool {
    return (look.modes.z & bit) != 0u;
}

fn curve(t: f32) -> vec4<f32> {
    let at = lut_at(t);
    let i = u32(at.x);
    return mix(look.curves[i], look.curves[min(i + 1u, 31u)], at.y);
}

fn life_color(t: f32) -> vec4<f32> {
    let at = lut_at(t);
    let i = u32(at.x);
    let over = mix(look.colors[i], look.colors[min(i + 1u, 31u)], at.y);
    return mix(look.color_start, look.color_end, t) * over;
}

fn ribbon_color(s: f32) -> vec4<f32> {
    let at = lut_at(s);
    let i = u32(at.x);
    return mix(look.ribbon_colors[i], look.ribbon_colors[min(i + 1u, 31u)], at.y);
}

fn hidden() -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.color = vec4<f32>(0.0);
    out.frames = vec3<f32>(0.0);
    out.kind = 0u;
    return out;
}

fn billboard(slot: u32, corner: u32) -> VertexOutput {
    let p = particles[slot];
    if (!alive(p) || pinned(p)) {
        return hidden();
    }
    let t = life_t(p);
    let c = curve(t);
    let size = mix(look.size.x, look.size.y, t) * c.x * p.size;
    let q = vec2<f32>(select(-0.5, 0.5, (corner & 1u) != 0u), select(-0.5, 0.5, (corner & 2u) != 0u));
    var offset: vec2<f32>;
    let speed = length(p.velocity.xy);
    if (look.modes.y == 1u && speed > 1e-5) {
        let axis = p.velocity.xy / speed;
        let side = vec2<f32>(-axis.y, axis.x);
        offset = side * q.x * size + axis * q.y * (size + speed * look.size.z);
    } else {
        let angle = radians(p.rotation + c.z);
        offset = vec2<f32>(q.x * cos(angle) - q.y * sin(angle), q.x * sin(angle) + q.y * cos(angle)) * size;
    }
    var out: VertexOutput;
    out.position = view.clip_from_world * vec4<f32>(p.position.xy + offset, p.position.z, 1.0);
    out.uv = vec2<f32>(q.x + 0.5, 0.5 - q.y);
    var color = life_color(t);
    color.a *= c.y;
    out.color = color;
    let frames = max(look.flipbook.w, 1.0);
    var f = p.frame + t * frames;
    if (look.flipbook.z > 0.0) {
        f = p.frame + p.age * look.flipbook.z;
    } else {
        f = min(f, p.frame + frames - 1.0);
    }
    out.frames = vec3<f32>(floor(f), floor(f) + 1.0, fract(f));
    out.kind = 0u;
    return out;
}

fn point(base: u32, head: u32, held: u32, k: i32) -> vec3<f32> {
    let points = look.ribbon.x;
    let back = u32(clamp(k, 0, i32(held) - 1));
    return trail[base + (head + points - back) % points].xyz;
}

fn along(base: u32, head: u32, held: u32, x: f32) -> vec3<f32> {
    let k = i32(floor(x));
    let f = x - floor(x);
    let p0 = point(base, head, held, k - 1);
    let p1 = point(base, head, held, k);
    let p2 = point(base, head, held, k + 1);
    let p3 = point(base, head, held, k + 2);
    let f2 = f * f;
    let f3 = f2 * f;
    return 0.5 * ((2.0 * p1) + (-p0 + p2) * f + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * f2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * f3);
}

fn ribbon(slot: u32, corner: u32, segment: u32) -> VertexOutput {
    let p = particles[slot];
    if (!alive(p) || (has(FLAG_PINNED_ONLY) != pinned(p))) {
        return hidden();
    }
    let held = p.ribbon >> 16u;
    let head = p.ribbon & 0xFFFFu;
    let per = look.ribbon.y + 1u;
    if (held < 2u || segment >= (held - 1u) * per) {
        return hidden();
    }
    let base = slot * look.ribbon.x;
    let x = f32(segment + (corner >> 1u)) / f32(per);
    let pos = along(base, head, held, x);
    var tangent = (along(base, head, held, x + 0.05) - along(base, head, held, max(x - 0.05, 0.0))).xy;
    if (length(tangent) < 1e-6) {
        tangent = vec2<f32>(0.0, 1.0);
    }
    let side = normalize(vec2<f32>(-tangent.y, tangent.x));
    let s = x / f32(held - 1u);
    let at = lut_at(s);
    let i = u32(at.x);
    let width = look.extra.y * mix(look.curves[i].w, look.curves[min(i + 1u, 31u)].w, at.y);
    let edge = select(-0.5, 0.5, (corner & 1u) != 0u);
    var out: VertexOutput;
    out.position = view.clip_from_world * vec4<f32>(pos.xy + side * width * edge, pos.z, 1.0);
    out.uv = vec2<f32>(s, edge + 0.5);
    let t = life_t(p);
    var color = life_color(t) * ribbon_color(s);
    color.a *= curve(t).y;
    if (pinned(p)) {
        color = look.color_start * ribbon_color(s);
    }
    out.color = color;
    out.frames = vec3<f32>(0.0);
    out.kind = 1u;
    return out;
}

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    let slot = u32(v.position.x);
    let corner = u32(v.position.y);
    if (slot >= look.modes.w) {
        return hidden();
    }
    if (look.modes.x == 1u) {
        return ribbon(slot, corner, u32(v.position.z));
    }
    return billboard(slot, corner);
}

fn sample_frame(index: f32, uv: vec2<f32>) -> vec4<f32> {
    let rect = frame_rect(index, look.flipbook.xy);
    return textureSample(sheet, sheet_sampler, rect.xy + uv * rect.zw);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;
    if (in.kind == 1u) {
        let across = abs(in.uv.y * 2.0 - 1.0);
        color.a *= 1.0 - across * across;
    } else if (has(FLAG_FLIPBOOK)) {
        var tex = sample_frame(in.frames.x, in.uv);
        if (has(FLAG_FRAME_BLEND)) {
            tex = mix(tex, sample_frame(in.frames.y, in.uv), in.frames.z);
        }
        color *= tex;
    } else {
        let r = length(in.uv - vec2<f32>(0.5)) * 2.0;
        color.a *= 1.0 - smoothstep(0.4, 1.0, r);
    }
    if (color.a <= 1e-4) {
        discard;
    }
    let intensity = select(look.size.w, look.extra.z, in.kind == 1u);
    var out = vec4<f32>(color.rgb * intensity, color.a);
    if (has(FLAG_ADDITIVE)) {
        out = vec4<f32>(out.rgb * color.a, 0.0);
    }
    @if(TONEMAP_IN_SHADER)
    out = tonemapping::tone_mapping(out, view.color_grading);
    @if(SRGB_OUTPUT)
    out = vec4(linear_to_srgb(out.rgb), out.a);
    @if(OKLAB_OUTPUT)
    out = vec4(linear_rgb_to_oklab(out.rgb), out.a);
    return out;
}
