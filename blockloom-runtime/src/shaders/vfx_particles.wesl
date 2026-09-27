// VFX particles and ribbons in 3D. Nothing about a particle is in the mesh:
// each vertex's position attribute names a slot, a corner and (for ribbons)
// a segment, and the vertex shader reads the particle the sim wrote. So one
// mesh draws every particle, whichever sim filled the buffer.

import bevy_pbr::render::{
    mesh_view_bindings::view,
    view_transformations::depth_ndc_to_view_z,
    pbr_types::pbr_input_new,
    pbr_functions::apply_pbr_lighting,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
};
import bevy_pbr::render::mesh_view_bindings as view_bindings;
import blockloom::vfx::{Particle, alive, pinned, life_t, lut_at, frame_rect};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;

struct ParticleLook {
    // Linear start and end tints.
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    // Size start, size end, stretch, intensity.
    size: vec4<f32>,
    // Flipbook columns, rows, fps, frames.
    flipbook: vec4<f32>,
    // Soft distance, ribbon width, ribbon intensity, translucency.
    extra: vec4<f32>,
    // Mode (0 billboards, 1 ribbons), facing, flags, capacity.
    modes: vec4<u32>,
    // Ribbon points, tessellation, 0, 0.
    ribbon: vec4<u32>,
    // Size, alpha, rotation and ribbon width over life (width along the ribbon).
    curves: array<vec4<f32>, 32>,
    colors: array<vec4<f32>, 32>,
    ribbon_colors: array<vec4<f32>, 32>,
}

const FLAG_LIT: u32 = 1u;
const FLAG_SOFT: u32 = 2u;
const FLAG_FLIPBOOK: u32 = 4u;
const FLAG_FRAME_BLEND: u32 = 8u;
const FLAG_PINNED_ONLY: u32 = 16u;
const FLAG_FACE_CAMERA: u32 = 32u;
const FLAG_ADDITIVE: u32 = 64u;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> look: ParticleLook;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<storage, read> particles: array<Particle>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<storage, read> trail: array<vec4<f32>>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var sheet: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var sheet_sampler: sampler;
// Fragments every particle draw shades this frame, for the overdraw meter.
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var<storage, read_write> overdraw: array<atomic<u32>>;

struct Vertex {
    // Slot, corner, ribbon segment.
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    // Flipbook frame, the next frame, the blend between them.
    @location(3) frames: vec3<f32>,
    @location(4) @interpolate(flat) kind: u32,
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

fn ribbon_width(s: f32) -> f32 {
    let at = lut_at(s);
    let i = u32(at.x);
    return mix(look.curves[i].w, look.curves[min(i + 1u, 31u)].w, at.y);
}

fn hidden() -> VertexOutput {
    var out: VertexOutput;
    // Every corner at one point outside the view: nothing rasterizes.
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.world_position = vec4<f32>(0.0);
    out.uv = vec2<f32>(0.0);
    out.color = vec4<f32>(0.0);
    out.frames = vec3<f32>(0.0);
    out.kind = 0u;
    return out;
}

fn frames_of(p: Particle, t: f32) -> vec3<f32> {
    let frames = max(look.flipbook.w, 1.0);
    var f = p.frame + t * frames;
    if (look.flipbook.z > 0.0) {
        f = p.frame + p.age * look.flipbook.z;
    }
    if (look.flipbook.z <= 0.0) {
        // Played once over the life: stop on the last frame.
        f = min(f, p.frame + frames - 1.0);
    }
    return vec3<f32>(floor(f), floor(f) + 1.0, fract(f));
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
    let angle = radians(p.rotation + c.z);
    let turned = vec2<f32>(q.x * cos(angle) - q.y * sin(angle), q.x * sin(angle) + q.y * cos(angle));
    var offset = vec3<f32>(0.0);
    switch look.modes.y {
        case 1u: {
            let speed = length(p.velocity);
            let axis = select(view.world_from_view[1].xyz, p.velocity / speed, speed > 1e-5);
            let to_eye = normalize(view.world_position - p.position);
            var side = cross(axis, to_eye);
            side = select(view.world_from_view[0].xyz, normalize(side), length(side) > 1e-5);
            let reach = size + speed * look.size.z;
            offset = side * q.x * size + axis * q.y * reach;
        }
        case 2u: {
            offset = vec3<f32>(turned.x, 0.0, turned.y) * size;
        }
        default: {
            offset = (view.world_from_view[0].xyz * turned.x + view.world_from_view[1].xyz * turned.y) * size;
        }
    }
    let world = vec4<f32>(p.position + offset, 1.0);
    var out: VertexOutput;
    out.position = view.clip_from_world * world;
    out.world_position = world;
    out.uv = vec2<f32>(q.x + 0.5, 0.5 - q.y);
    var color = life_color(t);
    color.a *= c.y;
    out.color = color;
    out.frames = frames_of(p, t);
    out.kind = 0u;
    return out;
}

// Ribbon point `k` back from the head, clamped to what the particle holds.
fn point(base: u32, head: u32, held: u32, k: i32) -> vec3<f32> {
    let points = look.ribbon.x;
    let back = u32(clamp(k, 0, i32(held) - 1));
    return trail[base + (head + points - back) % points].xyz;
}

// Catmull-Rom through the points, `x` points back from the head.
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
    var tangent = along(base, head, held, x + 0.05) - along(base, head, held, max(x - 0.05, 0.0));
    if (length(tangent) < 1e-6) {
        tangent = vec3<f32>(0.0, 1.0, 0.0);
    }
    var side: vec3<f32>;
    if (has(FLAG_FACE_CAMERA)) {
        side = cross(tangent, view.world_position - pos);
    } else {
        side = cross(tangent, vec3<f32>(0.0, 1.0, 0.0));
    }
    side = select(view.world_from_view[0].xyz, normalize(side), length(side) > 1e-6);
    let s = x / f32(held - 1u);
    let t = life_t(p);
    let width = look.extra.y * ribbon_width(s);
    let edge = select(-0.5, 0.5, (corner & 1u) != 0u);
    let world = vec4<f32>(pos + side * width * edge, 1.0);
    var out: VertexOutput;
    out.position = view.clip_from_world * world;
    out.world_position = world;
    out.uv = vec2<f32>(s, edge + 0.5);
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

@if(DEPTH_PREPASS)
fn soft_fade(position: vec4<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(view_bindings::depth_prepass_texture));
    let texel = vec2<i32>(clamp(position.xy, vec2<f32>(0.0), size - 1.0));
    let scene = textureLoad(view_bindings::depth_prepass_texture, texel, 0);
    if (scene <= 0.0) {
        return 1.0;
    }
    let gap = depth_ndc_to_view_z(position.z) - depth_ndc_to_view_z(scene);
    return clamp(gap / max(look.extra.x, 1e-4), 0.0, 1.0);
}

@if(!DEPTH_PREPASS)
fn soft_fade(position: vec4<f32>) -> f32 {
    return 1.0;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;
    if (in.kind == 1u) {
        // Across a ribbon: soft at both edges.
        let across = abs(in.uv.y * 2.0 - 1.0);
        color.a *= 1.0 - across * across;
    } else if (has(FLAG_FLIPBOOK)) {
        var tex = sample_frame(in.frames.x, in.uv);
        if (has(FLAG_FRAME_BLEND)) {
            tex = mix(tex, sample_frame(in.frames.y, in.uv), in.frames.z);
        }
        color *= tex;
    } else {
        // No sheet: a soft round dot.
        let r = length(in.uv - vec2<f32>(0.5)) * 2.0;
        color.a *= 1.0 - smoothstep(0.4, 1.0, r);
    }
    if (has(FLAG_SOFT)) {
        color.a *= soft_fade(in.position);
    }
    if (color.a <= 1e-4) {
        discard;
    }
    atomicAdd(&overdraw[0], 1u);
    var rgb = color.rgb;
    if (has(FLAG_LIT)) {
        // A billboard lit like the front of a ball, so light wraps round it.
        let to_eye = normalize(view.world_position - in.world_position.xyz);
        let right = view.world_from_view[0].xyz;
        let up = view.world_from_view[1].xyz;
        let d = (in.uv - vec2<f32>(0.5)) * vec2<f32>(2.0, -2.0);
        let bulge = sqrt(max(1.0 - dot(d, d), 0.0));
        let n = normalize(right * d.x * 0.7 + up * d.y * 0.7 + to_eye * max(bulge, 0.3));
        var pbr = pbr_input_new();
        pbr.material.base_color = vec4<f32>(rgb, 1.0);
        pbr.material.perceptual_roughness = 1.0;
        pbr.material.metallic = 0.0;
        pbr.material.reflectance = vec3<f32>(0.0);
        pbr.frag_coord = in.position;
        pbr.world_position = in.world_position;
        pbr.world_normal = n;
        pbr.N = n;
        pbr.V = to_eye;
        pbr.is_orthographic = view.clip_from_view[3].w == 1.0;
        pbr.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
        rgb = apply_pbr_lighting(pbr).rgb;
        // Thin smoke lets light through: the back half of the ball, lit
        // from behind, shadows and all.
        if (look.extra.w > 0.0) {
            pbr.world_normal = -n;
            pbr.N = -n;
            pbr.V = -to_eye;
            rgb += apply_pbr_lighting(pbr).rgb * look.extra.w;
        }
    }
    let intensity = select(look.size.w, look.extra.z, in.kind == 1u);
    rgb *= intensity;
    var out = vec4<f32>(rgb, color.a);
    if (has(FLAG_ADDITIVE)) {
        out = vec4<f32>(rgb * color.a, 0.0);
    }
    @if(TONEMAP_IN_SHADER)
    out = tonemapping::tone_mapping(out, view.color_grading);
    return out;
}
